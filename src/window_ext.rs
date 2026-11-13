//! AppKit bits gpui does not expose: NSWindow level (always-on-top) and the
//! application (Dock) icon.

use std::path::{Path, PathBuf};

use gpui::Window;
use objc2::rc::Retained;
use objc2::{AnyThread as _, ClassType, MainThreadMarker};
use objc2_app_kit::{
    NSApplication, NSFloatingWindowLevel, NSImage, NSNormalWindowLevel, NSResponder, NSView,
    NSWindow,
};
use objc2_core_foundation::CGPoint;
use objc2_foundation::{NSObjectProtocol, NSString};
use objc2_web_kit::WKWebView;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// The NSWindow backing a gpui window.
pub fn ns_window(window: &Window) -> Option<Retained<NSWindow>> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return None;
    };
    // SAFETY: gpui keeps the native view alive for the lifetime of Window.
    let view = unsafe { &*handle.ns_view.as_ptr().cast::<NSView>() };
    view.window()
}

/// Raises the window to NSFloatingWindowLevel or drops it back to normal.
/// Returns the level the window reports afterwards.
pub fn set_always_on_top(window: &Window, on_top: bool) -> Option<i64> {
    let native = ns_window(window)?;
    let target = if on_top {
        NSFloatingWindowLevel
    } else {
        NSNormalWindowLevel
    };
    native.setLevel(target);
    Some(native.level() as i64)
}

/// Moves the window by a gpui-space delta (x right, y down). AppKit's origin
/// is bottom-left, so the vertical delta is applied with the opposite sign.
pub fn nudge_window(window: &Window, dx: f32, dy: f32) -> bool {
    let Some(native) = ns_window(window) else {
        return false;
    };
    let frame = native.frame();
    let origin = CGPoint::new(
        frame.origin.x + f64::from(dx),
        frame.origin.y - f64::from(dy),
    );
    native.setFrameOrigin(origin);
    true
}

pub fn level(window: &Window) -> Option<i64> {
    let native = ns_window(window)?;
    Some(native.level() as i64)
}

/// Icon candidates in lookup order: the bundle Resources next to the running
/// executable first (a copied .app still finds its own icon), then the source
/// tree for `cargo run`.
pub fn icon_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(resources) = bundle_resources() {
        candidates.push(resources.join("icon.icns"));
        candidates.push(resources.join("icon.png"));
    }
    candidates.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/icon.png"));
    candidates
}

/// Contents/Resources of the running .app, when the executable lives in one.
fn bundle_resources() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let macos = exe.parent().filter(|dir| dir.ends_with("MacOS"))?;
    let contents = Path::parent(macos).filter(|dir| dir.ends_with("Contents"))?;
    Some(contents.join("Resources"))
}

/// One selectable icon: the bundled copy first, then the source tree so
/// `cargo run` can switch icons without a rebuild.
pub fn icon_variant_paths(name: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(resources) = bundle_resources() {
        candidates.push(resources.join("icons").join(format!("{name}.png")));
    }
    candidates.push(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets/icon-candidates")
            .join(format!("{name}.png")),
    );
    candidates
}

/// Applies the first icon that exists. No candidate is not an error: the app
/// keeps the default icon and logs one line.
pub fn apply_dock_icon(variant: &str) {
    let mut candidates = icon_variant_paths(variant);
    candidates.extend(icon_candidates());
    let Some(path) = candidates.iter().find(|path| path.exists()) else {
        let tried = candidates
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        eprintln!("[icon] no icon found, tried: {tried}");
        return;
    };
    let Some(mtm) = MainThreadMarker::new() else {
        eprintln!("[icon] not on the main thread, skipped");
        return;
    };
    let name = NSString::from_str(&path.to_string_lossy());
    let Some(image) = NSImage::initWithContentsOfFile(NSImage::alloc(), &name) else {
        eprintln!("[icon] NSImage could not read {}", path.display());
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    // SAFETY: the image is retained for the duration of the call.
    unsafe { app.setApplicationIconImage(Some(&image)) };
    eprintln!(
        "[icon] Dock icon set from {} variant={variant} ({:.0}x{:.0})",
        path.display(),
        image.size().width,
        image.size().height
    );
}

/// The WKWebView gpui does not know about: it is an AppKit subview added by
/// webview.rs, so it is found by walking the window's view tree. With two tabs
/// both webviews live in that tree, so hidden ones are skipped: the visible one
/// is the page the user is looking at.
pub fn find_webview(window: &Window) -> Option<Retained<WKWebView>> {
    let native = ns_window(window)?;
    let content = native.contentView()?;
    find_webview_in(&content)
}

fn find_webview_in(view: &NSView) -> Option<Retained<WKWebView>> {
    if view.isHidden() {
        return None;
    }
    if view.isKindOfClass(WKWebView::class()) {
        // SAFETY: the class check above guarantees the type; the window owns it.
        return unsafe { Retained::retain(view as *const NSView as *mut WKWebView) };
    }
    for subview in view.subviews().iter() {
        if let Some(found) = find_webview_in(&subview) {
            return Some(found);
        }
    }
    None
}

/// The webview that owns the keyboard right now. Editing commands (cmd-C/V/...)
/// must go to the focused page; with two tabs "the first WKWebView found" may be
/// the hidden one.
pub fn focused_webview(window: &Window) -> Option<Retained<WKWebView>> {
    let native = ns_window(window)?;
    let responder = native.firstResponder()?;
    if !responder.isKindOfClass(WKWebView::class()) {
        return None;
    }
    // SAFETY: the class check above guarantees the type; the window retains it.
    unsafe {
        let ptr = Retained::as_ptr(&responder) as *const NSView as *mut WKWebView;
        Retained::retain(ptr)
    }
}

/// Reproduces what a click into the page does: the webview takes first
/// responder, after which gpui's own keyDown never fires. Returns the class
/// name of the first responder the window reports afterwards.
pub fn focus_webview(window: &Window) -> Option<String> {
    let webview = find_webview(window)?;
    let native = ns_window(window)?;
    let responder: &NSResponder = &webview;
    let accepted = native.makeFirstResponder(Some(responder));
    let class = native
        .firstResponder()
        .map(|responder| responder.class().name().to_string_lossy().into_owned());
    eprintln!("[keys] makeFirstResponder(WKWebView) accepted = {accepted}, first responder = {class:?}");
    class
}

pub fn webview_is_loading(window: &Window) -> Option<bool> {
    find_webview(window).map(|webview| unsafe { webview.isLoading() })
}
