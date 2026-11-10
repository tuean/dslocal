//! WKWebView wrapper embedded as an AppKit subview of the gpui window.
//!
//! Coordinate systems: AppKit NSView uses a bottom-left origin with y pointing
//! up, while gpui uses a top-left origin with y pointing down. The webview is
//! pinned to the bottom of the parent view and keeps a fixed margin of
//! toolbar_height at the top -- exactly the strip where gpui draws its toolbar.
//! So frame.y == 0 and frame.height == parent.height - toolbar_height, and the
//! top-left origin of the gpui toolbar maps 1:1 onto that fixed top margin.

use std::ffi::c_void;

use anyhow::{Context as _, Result, anyhow};
use gpui::Window;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{Encode, Encoding, MainThreadMarker, MainThreadOnly, msg_send};
use objc2_app_kit::{
    NSApplication, NSAutoresizingMaskOptions, NSModalPanelWindowLevel, NSModalResponseOK, NSOpenPanel,
    NSView,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::{NSArray, NSString, NSURL, NSURLRequest};
use objc2_web_kit::{
    WKFrameInfo, WKOpenPanelParameters, WKUIDelegate, WKUserContentController, WKUserScript,
    WKUserScriptInjectionTime, WKWebView, WKWebViewConfiguration,
};

// File upload panel for the page's file input.
//
// WKWebView refuses to open any picker on its own: a click on the page's attach
// button makes WebKit ask its UIDelegate through
// webView:runOpenPanelWithParameters:initiatedByFrame:completionHandler:, and
// if nothing answers it behaves as if the user pressed Cancel. Hosting the
// panel here is therefore the whole feature.
objc2::define_class!(
    // SAFETY: NSObject has no subclassing requirements and this class holds no
    // ivars; it is only ever touched from the main thread, which is also where
    // WebKit calls the delegate method.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DSLocalFilePickerDelegate"]
    #[ivars = ()]
    struct FilePickerDelegate;

    unsafe impl NSObjectProtocol for FilePickerDelegate {}

    unsafe impl WKUIDelegate for FilePickerDelegate {
        #[unsafe(method(webView:runOpenPanelWithParameters:initiatedByFrame:completionHandler:))]
        unsafe fn run_open_panel(
            &self,
            _web_view: &WKWebView,
            parameters: &WKOpenPanelParameters,
            _frame: &WKFrameInfo,
            completion_handler: &block2::DynBlock<dyn Fn(*mut NSArray<NSURL>)>,
        ) {
            // The page picks the shape: multi-select and directories are its
            // own attributes, not ours.
            let multiple = unsafe { parameters.allowsMultipleSelection() };
            let directories = unsafe { parameters.allowsDirectories() };
            eprintln!("[upload] runOpenPanel multiple={multiple} directories={directories}");
            // Test hook: a modal panel would block the automation that heads
            // this path, so DSLOCAL_UPLOAD_TEST=1 logs the call and reports
            // Cancel instead of showing one.
            if std::env::var_os("DSLOCAL_UPLOAD_TEST").is_some() {
                let _ = completion_handler.call((std::ptr::null_mut::<NSArray<NSURL>>(),));
                return;
            }
            let picked = MainThreadMarker::new().and_then(|mtm| open_panel(mtm, multiple, directories));
            match &picked {
                Some(urls) => {
                    // The handler copies the URLs during this call, so holding
                    // the autoreleased array for its address is enough.
                    let ptr = Retained::as_ptr(urls) as *mut NSArray<NSURL>;
                    let _ = completion_handler.call((ptr,));
                }
                None => {
                    let _ = completion_handler.call((std::ptr::null_mut::<NSArray<NSURL>>(),));
                }
            }
        }
    }
);

/// Runs the upload panel and returns the chosen URLs; None means cancelled.
fn open_panel(
    mtm: MainThreadMarker,
    allows_multiple: bool,
    allows_directories: bool,
) -> Option<Retained<NSArray<NSURL>>> {
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseFiles(true);
    panel.setCanChooseDirectories(allows_directories);
    panel.setAllowsMultipleSelection(allows_multiple);
    // The panel's own level is normal (0), so an always-on-top main window
    // (level 3) would draw over the picker and the picker would look dead.
    panel.setLevel(NSModalPanelWindowLevel);
    // Panels hide themselves when the app loses focus, and on a busy desktop
    // another app steals it while the picker is open -- the picker then looks
    // like it never opened.
    panel.setHidesOnDeactivate(false);
    // The page triggers this while the app may not be frontmost; an app that
    // is not active gets its modal panel behind everything else.
    let app = NSApplication::sharedApplication(mtm);
    // Deprecated, but plain activate() loses the race against floating helper
    // windows on this machine and leaves the panel behind them.
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
    eprintln!(
        "[upload] app active={} panel visible={}",
        app.isActive(),
        panel.isVisible()
    );
    let response = panel.runModal();
    eprintln!("[upload] panel closed, response={response}");
    (response == NSModalResponseOK).then(|| panel.URLs())
}

/// Evidence hook: shows the same panel without a page behind it, so the picker
/// can be verified even though WebKit's user-gesture gate ignores synthetic
/// clicks on a file input.
pub fn panel_selftest() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    eprintln!("[upload] selftest opening panel (multi, files only)");
    match open_panel(mtm, true, false) {
        Some(urls) => eprintln!("[upload] selftest picked {} file(s)", urls.count()),
        None => eprintln!("[upload] selftest cancelled"),
    }
}

impl FilePickerDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

/// Runs in every page before its own scripts. A browser is supposed to mark the
/// Enter that commits an IME candidate as composing, but WebKit lets the site
/// see it as a plain Enter, so a chat box that sends on Enter fires mid-word.
/// The capture listener kills the event before the site's handler, and the
/// composition flags cover the case where `isComposing` was not set either.
const IME_GUARD_JS: &str = r#"
(() => {
  if (window.__dslocalImeGuard) return;
  window.__dslocalImeGuard = true;
  const stats = { imeEnter: 0, plainEnter: 0 };
  window.__dslocalImeStats = stats;
  let composing = false;
  const stop = () => { composing = false; };
  window.addEventListener('compositionstart', () => { composing = true; }, true);
  window.addEventListener('compositionend', stop, true);
  // Failsafe: a lost compositionend would otherwise kill Enter for good.
  window.addEventListener('blur', stop, true);
  window.addEventListener('compositionstart', () => setTimeout(stop, 30000), true);
  window.addEventListener('keydown', (event) => {
    if (event.key !== 'Enter') return;
    const fromIme = composing || event.isComposing || event.keyCode === 229;
    if (!fromIme) {
      stats.plainEnter += 1;
      return;
    }
    stats.imeEnter += 1;
    // Swallow only the site's handling: the candidate still commits, because
    // the input method works above the DOM.
    event.stopImmediatePropagation();
  }, true);
})();
"#;

pub struct WebView {
    view: Retained<WKWebView>,
    /// WKWebView holds its UIDelegate weakly, so the picker delegate has to be
    /// owned here or it would be deallocated the moment `new` returns.
    _picker: Retained<FilePickerDelegate>,
    /// Height (in points) reserved at the top of the parent view for gpui.
    toolbar_height: f64,
}

impl WebView {
    /// Creates the WKWebView inside window and starts loading url.
    pub fn new(window: &Window, toolbar_height: f32, url: &str) -> Result<Self> {
        let mtm = MainThreadMarker::new().context("WKWebView must be created on the main thread")?;
        let parent = parent_view(window)?;

        let frame = frame_for(parent, f64::from(toolbar_height));
        let configuration = unsafe { WKWebViewConfiguration::new(mtm) };
        let controller = unsafe { WKUserContentController::new(mtm) };
        let script = unsafe {
            WKUserScript::initWithSource_injectionTime_forMainFrameOnly(
                WKUserScript::alloc(mtm),
                &NSString::from_str(IME_GUARD_JS),
                WKUserScriptInjectionTime::AtDocumentStart,
                false,
            )
        };
        unsafe {
            controller.addUserScript(&script);
            configuration.setUserContentController(&controller);
        }
        let view = unsafe {
            WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), frame, &configuration)
        };

        // Let AppKit resize the webview itself: flexible width/height with
        // zero bottom/left margins and a fixed top margin equal to the toolbar.
        view.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        let picker = FilePickerDelegate::new(mtm);
        unsafe { view.setUIDelegate(Some(ProtocolObject::from_ref(&*picker))) };
        parent.addSubview(&view);

        let webview = Self {
            view,
            _picker: picker,
            toolbar_height: f64::from(toolbar_height),
        };
        webview.load_url(url)?;
        Ok(webview)
    }

    pub fn load_url(&self, url: &str) -> Result<()> {
        let nsurl = NSURL::URLWithString(&NSString::from_str(url))
            .ok_or_else(|| anyhow!("invalid URL: {url}"))?;
        let request = NSURLRequest::requestWithURL(&nsurl);
        unsafe { self.view.loadRequest(&request) };
        Ok(())
    }

    pub fn reload(&self) {
        unsafe { self.view.reload() };
    }

    pub fn go_back(&self) {
        unsafe { self.view.goBack() };
    }

    pub fn go_forward(&self) {
        unsafe { self.view.goForward() };
    }

    /// Page zoom, 1.0 == 100%.
    pub fn set_zoom(&self, zoom: f64) {
        unsafe { self.view.setPageZoom(zoom) };
    }

    pub fn zoom(&self) -> f64 {
        unsafe { self.view.pageZoom() }
    }

    /// False once the current load has settled; callers that need the page
    /// rendered poll this instead of guessing a delay.
    pub fn is_loading(&self) -> bool {
        unsafe { self.view.isLoading() }
    }

    /// Runs script in the page and returns its string result to cb on the main
    /// thread. A script returning a Promise only hands completionHandler the
    /// promise, so the wrapper stashes its settled value in a page global and
    /// the completion handler polls it until it is no longer the sentinel.
    pub fn evaluate_javascript(
        &self,
        script: &str,
        cb: impl FnOnce(Result<String, String>) + 'static,
    ) {
        let mut wrapped = String::with_capacity(script.len() + 420);
        wrapped.push_str("(function () { try { var v = (");
        wrapped.push_str(script);
        wrapped.push_str(
            "); if (v && typeof v.then === 'function') { window.__dslocalEval = '__DSLOCAL_PENDING__'; v.then(function (x) { window.__dslocalEval = (x === undefined || x === null) ? '' : String(x); }, function (e) { window.__dslocalEval = '__DSLOCAL_ERROR__' + String(e); }); return '__DSLOCAL_PENDING__'; } return (v === undefined || v === null) ? '' : String(v); } catch (e) { return '__DSLOCAL_ERROR__' + String(e); } })()",
        );

        let state = Box::new(EvalState {
            view: self.view.clone(),
            cb: Some(Box::new(cb)),
            tries: 0,
        });
        run_script(Box::into_raw(state), &wrapped);
    }

    /// The native view, e.g. for evaluateJavaScript: or frame tweaks.
    #[allow(dead_code)]
    pub fn native_view(&self) -> &NSView {
        &self.view
    }

    /// Re-applies frame + toolbar margin. Autoresizing already handles window
    /// resizes; this also covers the first frames and toolbar height changes.
    pub fn sync_frame(&self) {
        // SAFETY: the superview outlives the webview (it is the gpui view).
        let Some(parent) = (unsafe { self.view.superview() }) else {
            return;
        };
        let frame = frame_for(&parent, self.toolbar_height);
        if self.view.frame() != frame {
            self.view.setFrame(frame);
        }
    }

    /// Sets the frame explicitly, in AppKit points with a bottom-left origin.
    #[allow(dead_code)]
    pub fn set_frame(&self, x: f64, y: f64, width: f64, height: f64) {
        let frame = CGRect::new(CGPoint::new(x, y), CGSize::new(width, height));
        self.view.setFrame(frame);
    }
}

const POLL_MS: u64 = 200;
const MAX_POLLS: u32 = 60;
const PENDING: &str = "__DSLOCAL_PENDING__";
const ERROR_PREFIX: &str = "__DSLOCAL_ERROR__";

/// One in-flight evaluateJavaScript. The page callback owns it and frees it
/// when it settles, so a dropped callback cannot leave a dangling pointer.
struct EvalState {
    view: Retained<WKWebView>,
    cb: Option<Box<dyn FnOnce(Result<String, String>)>>,
    tries: u32,
}

/// libclosure layout for a block capturing one pointer: the captured context
/// follows the descriptor, and size in the descriptor must match this struct.
#[repr(C)]
struct EvalBlock {
    isa: *mut c_void,
    flags: i32,
    reserved: i32,
    invoke: *const c_void,
    descriptor: *const BlockDescriptor,
    context: *mut EvalState,
}

#[repr(C)]
struct BlockDescriptor {
    reserved: usize,
    size: usize,
}

static BLOCK_DESCRIPTOR: BlockDescriptor = BlockDescriptor {
    reserved: 0,
    size: std::mem::size_of::<EvalBlock>(),
};

/// BLOCK_IS_GLOBAL: the runtime never copies or frees such a literal, which is
/// what makes it safe to hand WebKit a block built outside a stack frame.
const BLOCK_IS_GLOBAL: i32 = 1 << 28;

/// block2 is only a transitive dependency, so the completion handler is a
/// hand-built stack block rather than a copied Block<..>. The runtime checks
/// the argument encoding in debug builds, hence the explicit Block encoding.
#[repr(transparent)]
struct BlockArg(*const EvalBlock);

// SAFETY: repr(transparent) over a pointer, passed where the selector expects
// a @? argument; Encoding::Block is what completionHandler: declares.
unsafe impl Encode for BlockArg {
    const ENCODING: Encoding = Encoding::Block;
}

/// Opaque dispatch object; only its address is used.
#[repr(C)]
struct DispatchQueue {
    _private: [u8; 0],
}

unsafe extern "C" {
    static _NSConcreteGlobalBlock: *mut c_void;
    // dispatch_get_main_queue() is an SDK inline over this global, so the
    // symbol the linker actually has is _dispatch_main_q.
    static _dispatch_main_q: DispatchQueue;
    fn dispatch_time(when: u64, delta: i64) -> u64;
    fn dispatch_after_f(
        when: u64,
        queue: *mut DispatchQueue,
        context: *mut c_void,
        work: extern "C" fn(*mut c_void),
    );
}

fn run_script(state: *mut EvalState, script: &str) {
    let view: &WKWebView = unsafe { &(*state).view };
    let script = NSString::from_str(script);
    // A stack block is only valid until this frame returns, and WebKit invokes
    // the handler after an IPC round trip. A global block is immortal instead:
    // the literal is leaked on purpose (48 bytes per evaluation) and stays
    // valid for as long as WebKit may call it.
    let block = Box::new(EvalBlock {
        isa: unsafe { _NSConcreteGlobalBlock },
        flags: BLOCK_IS_GLOBAL,
        reserved: 0,
        invoke: eval_trampoline as extern "C" fn(*mut EvalBlock, *mut c_void, *mut c_void)
            as *const c_void,
        descriptor: &BLOCK_DESCRIPTOR,
        context: state,
    });
    let block: *const EvalBlock = Box::into_raw(block);
    // SAFETY: both pointers stay valid for the lifetime of the call.
    unsafe {
        let _: () = msg_send![view, evaluateJavaScript: &*script, completionHandler: BlockArg(block)];
    }
}

extern "C" fn eval_trampoline(block: *mut EvalBlock, result: *mut c_void, error: *mut c_void) {
    if block.is_null() {
        return;
    }
    // SAFETY: the runtime hands back the literal this call built.
    let context = unsafe { (*block).context };
    if context.is_null() {
        return;
    }
    let value = read_result(result, error);
    // SAFETY: context is a Box::into_raw allocation owned by this callback.
    unsafe { handle_result(context, value) };
}

fn read_result(result: *mut c_void, error: *mut c_void) -> Result<String, String> {
    // SAFETY: both pointers are what completionHandler received for this call.
    unsafe {
        if !error.is_null() {
            let err = &*error.cast::<AnyObject>();
            let description: *mut NSString = msg_send![err, localizedDescription];
            return Err(if description.is_null() {
                "javascript error".to_string()
            } else {
                (*description).to_string()
            });
        }
        if result.is_null() {
            return Ok(String::new());
        }
        let object = &*result.cast::<AnyObject>();
        let description: *mut NSString = msg_send![object, description];
        Ok(if description.is_null() {
            String::new()
        } else {
            (*description).to_string()
        })
    }
}

unsafe fn handle_result(context: *mut EvalState, value: Result<String, String>) {
    // SAFETY: ownership of the Box was handed over at creation time.
    let mut state = unsafe { Box::from_raw(context) };
    match value {
        Err(err) => {
            if let Some(cb) = state.cb.take() {
                cb(Err(err));
            }
        }
        Ok(text) if text == PENDING => {
            if state.tries >= MAX_POLLS {
                if let Some(cb) = state.cb.take() {
                    cb(Err("javascript promise did not settle".to_string()));
                }
            } else {
                state.tries += 1;
                let next = Box::into_raw(state);
                // SAFETY: the pointer is live and now owned by the timer.
                unsafe { schedule_poll(next) };
            }
        }
        Ok(text) => {
            let outcome = match text.strip_prefix(ERROR_PREFIX) {
                Some(tail) => Err(tail.to_string()),
                None => Ok(text),
            };
            if let Some(cb) = state.cb.take() {
                cb(outcome);
            }
        }
    }
}

unsafe fn schedule_poll(context: *mut EvalState) {
    let when = unsafe { dispatch_time(0, (POLL_MS as i64) * 1_000_000) };
    let queue = std::ptr::addr_of!(_dispatch_main_q) as *mut DispatchQueue;
    // SAFETY: dispatched to the main queue, which is where blocks belong.
    unsafe { dispatch_after_f(when, queue, context.cast(), poll_work) };
}

extern "C" fn poll_work(context: *mut c_void) {
    let state = context.cast::<EvalState>();
    if state.is_null() {
        return;
    }
    run_script(state, "window.__dslocalEval");
}

/// The NSView gpui renders into; the webview is added on top of it.
fn parent_view(window: &Window) -> Result<&NSView> {
    // Window has an inherent window_handle() too, so call the trait method.
    let handle = HasWindowHandle::window_handle(window)
        .map_err(|err| anyhow!("window has no native handle: {err}"))?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return Err(anyhow!("unsupported raw window handle for this platform"));
    };
    // SAFETY: gpui keeps the native view alive for the lifetime of Window.
    Ok(unsafe { &*handle.ns_view.as_ptr().cast::<NSView>() })
}

/// AppKit rect: y == 0 is the bottom edge, so a fixed top margin for the
/// toolbar means height = parent height - toolbar height.
fn frame_for(parent: &NSView, toolbar_height: f64) -> CGRect {
    let bounds = parent.bounds();
    let height = (bounds.size.height - toolbar_height).max(0.0);
    CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(bounds.size.width, height))
}
