//! DSLOCAL_PASTE_TEST=1 evidence hook: decides where cmd-V dies.
//!
//! Two independent paths are exercised against the live page:
//!   1. a real NSEvent cmd-V pushed through this app's own event queue, so the
//!      keyroute monitor and AppKit's responder chain both run exactly as they
//!      do for a keystroke;
//!   2. a direct `paste:` message to the WKWebView, which skips key routing and
//!      exercises WebKit's own clipboard handling.
//! If (1) inserts nothing and (2) inserts the marker, the page and clipboard are
//! fine and the keyboard route is at fault. If both fail, WebKit never reads the
//! clipboard and the fix has to live on the WebKit side.
//!
//! The page has no debugger channel, so results come back through
//! `document.title`, which the probe polls synchronously with `-[WKWebView title]`.

use std::time::Duration;

use gpui::{AnyWindowHandle, App, AsyncApp, Window};
use objc2::runtime::AnyObject;
use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSApplication, NSEvent, NSEventModifierFlags, NSEventType, NSPasteboard, NSPasteboardTypeString,
};
use objc2_foundation::{NSPoint, NSString};
use objc2_web_kit::WKWebView;

const MARKER: &str = "DSLOCAL-PASTE-PROBE";

/// Bare `#js` keeps the evaluate call readable: the probe never needs a value
/// back from JavaScript itself, only the title it leaves behind.
fn eval(view: &WKWebView, js: &str) {
    let source = NSString::from_str(js);
    // SAFETY: main thread, and a missing completion handler is allowed.
    unsafe { view.evaluateJavaScript_completionHandler(&source, None) };
}

fn title(view: &WKWebView) -> String {
    // SAFETY: the webview is owned by the window and lives on the main thread.
    unsafe { view.title().map(|title| title.to_string()).unwrap_or_default() }
}

const ARM_JS: &str = r#"
(() => {
  const el = document.querySelector('textarea, [contenteditable="true"], input[type=text]');
  if (!el) { document.title = 'probe:no-input'; return; }
  el.focus();
  if ('value' in el) { el.value = ''; } else { el.textContent = ''; }
  document.title = 'probe:armed';
})();
"#;

const LOG_INSTALL_JS: &str = r#"
(() => {
  window.__dsKeys = [];
  document.addEventListener('keydown', (event) => {
    window.__dsKeys.push((event.metaKey ? 'cmd+' : '') + event.key);
  }, true);
})();
"#;

const LOG_READ_JS: &str = r#"
(() => {
  const el = document.activeElement;
  const value = !el ? '' : (('value' in el) ? el.value : (el.innerText || el.textContent || ''));
  const keys = (window.__dsKeys || []).join(',');
  document.title = 'probe:value=' + encodeURIComponent(String(value)).slice(0, 80)
    + '|keys=' + encodeURIComponent(keys).slice(0, 80);
})();
"#;

/// Reads the probe result out of the title, converting the percent encoding the
/// page applied so the log line is readable.
fn read_probe(view: &WKWebView) -> String {
    let title = title(view);
    let Some(rest) = title.strip_prefix("probe:value=") else {
        return format!("(no result, title = {title:?})");
    };
    let (value, keys) = rest.split_once("|keys=").unwrap_or((rest, ""));
    format!("value={} keys={}", decode(value), decode(keys))
}

fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                match u8::from_str_radix(&text[index + 1..index + 3], 16) {
                    Ok(byte) => {
                        out.push(byte);
                        index += 3;
                    }
                    Err(_) => {
                        out.push(bytes[index]);
                        index += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    format!("{:?}", String::from_utf8_lossy(&out))
}

fn with_webview<T>(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    f: impl FnOnce(&Window) -> T,
) -> Option<T> {
    cx.update(|app: &mut App| handle.update(app, |_, window, _| f(window)).ok())
}

pub fn schedule(cx: &mut App, window: &mut Window) {
    if std::env::var_os("DSLOCAL_PASTE_TEST").is_none() {
        return;
    }
    let handle = window.window_handle();
    cx.spawn(async move |cx: &mut AsyncApp| {
        // The page has to be real before the input can be focused.
        for _ in 0..60 {
            let loading = with_webview(cx, handle, crate::window_ext::webview_is_loading).flatten();
            if loading == Some(false) {
                break;
            }
            cx.background_executor()
                .timer(Duration::from_millis(200))
                .await;
        }
        settle(cx).await;

        let Some(mtm) = MainThreadMarker::new() else {
            eprintln!("[paste] probe needs the main thread");
            return;
        };
        let pasteboard = NSPasteboard::generalPasteboard();
        let saved = unsafe { pasteboard.stringForType(NSPasteboardTypeString) };
        let saved_title = with_webview(cx, handle, |window| {
            crate::window_ext::find_webview(window).map(|view| title(&view))
        })
        .flatten()
        .unwrap_or_default();

        let Some(view) = with_webview(cx, handle, crate::window_ext::find_webview).flatten() else {
            eprintln!("[paste] probe found no webview");
            return;
        };
        unsafe {
            pasteboard.clearContents();
            pasteboard.setString_forType(&NSString::from_str(MARKER), NSPasteboardTypeString);
        }
        eprintln!("[paste] clipboard prepared, marker = {MARKER:?}");

        let responder = with_webview(cx, handle, crate::window_ext::focus_webview).flatten();
        eprintln!("[paste] first responder before arm = {responder:?}");
        eval(&view, ARM_JS);
        settle(cx).await;
        eprintln!("[paste] page armed, title = {:?}", title(&view));

        let number = with_webview(cx, handle, |window| {
            crate::window_ext::ns_window(window).map(|native| native.windowNumber())
        })
        .flatten()
        .unwrap_or(0);
        let chars = NSString::from_str("v");
        // A synthetic event with timestamp 0 is rejected by some WebKit paths.
        let stamp = objc2_foundation::NSProcessInfo::processInfo().systemUptime();
        let make_event = || {
            NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
                NSEventType::KeyDown,
                NSPoint::new(0.0, 0.0),
                NSEventModifierFlags::Command,
                stamp,
                number,
                None,
                &chars,
                &chars,
                false,
                9, // kVK_ANSI_V
            )
        };

        let app = NSApplication::sharedApplication(mtm);
        app.activate();
        settle(cx).await;
        eprintln!(
            "[paste] app active = {}, window key = {}, main = {}",
            app.isActive(),
            with_webview(cx, handle, |window| {
                crate::window_ext::ns_window(window).map(|native| native.isKeyWindow())
            })
            .flatten()
            .unwrap_or(false),
            with_webview(cx, handle, |window| {
                crate::window_ext::ns_window(window).map(|native| native.isMainWindow())
            })
            .flatten()
            .unwrap_or(false),
        );

        // ---- path 1: a real key event through the app's event queue ----
        eval(&view, LOG_INSTALL_JS);
        eval(&view, ARM_JS);
        settle(cx).await;
        match make_event() {
            Some(event) => {
                NSApplication::sharedApplication(mtm).postEvent_atStart(&event, false);
                eprintln!("[paste] posted cmd-V to the app event queue");
            }
            None => eprintln!("[paste] could not build the cmd-V event"),
        }
        settle(cx).await;
        eval(&view, LOG_READ_JS);
        settle(cx).await;
        eprintln!("[paste] RESULT event-queue-path = {}", read_probe(&view));

        // ---- path 2: performKeyEquivalent: straight on the WKWebView ----
        eval(&view, ARM_JS);
        settle(cx).await;
        match make_event() {
            Some(event) => {
                let handled: bool = unsafe {
                    objc2::msg_send![&*view, performKeyEquivalent: &*event]
                };
                eprintln!("[paste] performKeyEquivalent(WKWebView) handled = {handled}");
            }
            None => {}
        }
        settle(cx).await;
        eval(&view, LOG_READ_JS);
        settle(cx).await;
        eprintln!("[paste] RESULT perform-key-equivalent = {}", read_probe(&view));

        // ---- path 3: paste: straight on the WKWebView ----
        eval(&view, ARM_JS);
        settle(cx).await;
        let _: () = unsafe { objc2::msg_send![&*view, paste: std::ptr::null_mut::<AnyObject>()] };
        settle(cx).await;
        eval(&view, LOG_READ_JS);
        settle(cx).await;
        eprintln!("[paste] RESULT direct-paste = {}", read_probe(&view));

        // Leave the user's clipboard and page title as they were.
        unsafe {
            pasteboard.clearContents();
            match &saved {
                Some(text) => {
                    pasteboard.setString_forType(text, NSPasteboardTypeString);
                }
                None => {}
            }
        }
        let restore = format!(
            "document.title = {};",
            serde_json::to_string(&saved_title).unwrap_or_else(|_| "\"\"".to_string())
        );
        eval(&view, &restore);
        eprintln!("[paste] clipboard restored, saved title = {saved_title:?}");
    })
    .detach();
}

async fn settle(cx: &mut AsyncApp) {
    cx.background_executor()
        .timer(Duration::from_millis(700))
        .await;
}
