//! DSLOCAL_ARROW_TEST=1 evidence hook for arrow keys in the page's text field.
//!
//! The page is driven with real NSEvents posted through AppKit (or straight to
//! the webview), and the resulting value/caret is read back through
//! `document.title`. DSLOCAL_ARROW_STEPS decides the key sequence, e.g.
//! `"j left"` types j through the IME and then presses the left arrow.

use std::time::Duration;

use gpui::{AnyWindowHandle, App, AsyncApp, Window};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSEvent, NSEventMask, NSEventModifierFlags, NSEventType};
use objc2_foundation::{NSPoint, NSString};
use objc2_web_kit::WKWebView;

/// A textarea of our own on top of the site: the site's editor is a controlled
/// component that resets the value we set, so its caret cannot be measured.
const ARM_JS: &str = r#"
(() => {
  let el = document.getElementById('__dslocalArrowProbe');
  if (!el) {
    el = document.createElement('textarea');
    el.id = '__dslocalArrowProbe';
    el.style.cssText = 'position:fixed;left:8px;top:8px;width:320px;height:80px;z-index:2147483647' +
      ';background:#fff;color:#000;font-size:16px;border:2px solid red';
    document.documentElement.appendChild(el);
  }
  el.value = '';
  el.focus();
  window.__dsArrow = [];
  const log = (t, a, b) => window.__dsArrow.push(t + ':' + a + '/' + b);
  if (!window.__dsArrowWired) {
    window.__dsArrowWired = true;
    window.addEventListener('keydown', (e) => log('kd', e.key, e.keyCode), true);
    window.addEventListener('beforeinput', (e) => log('bi', e.inputType, JSON.stringify(e.data)), true);
    window.addEventListener('input', (e) => log('in', e.inputType, JSON.stringify(e.data)), true);
  }
  document.title = 'arrow:armed';
})();
"#;

/// Focuses the site's own editor, so real typing lands where the user types.
const ARM_SITE_JS: &str = r#"
(() => {
  const editors = Array.from(document.querySelectorAll('textarea'));
  const el = editors.find((node) => node.offsetParent !== null) || editors[editors.length - 1];
  if (!el) { document.title = 'site:no-textarea'; return; }
  el.focus();
  window.__dsArrow = [];
  if (!window.__dsArrowWired) {
    window.__dsArrowWired = true;
    window.addEventListener('keydown', (e) => window.__dsArrow.push('kd:' + e.key + '/' + e.keyCode), true);
    window.addEventListener('beforeinput', (e) => window.__dsArrow.push('bi:' + e.inputType + '/' + JSON.stringify(e.data)), true);
    window.addEventListener('input', (e) => window.__dsArrow.push('in:' + e.inputType + '/' + JSON.stringify(e.data)), true);
  }
  document.title = 'site:armed';
})();
"#;

const READ_SITE_JS: &str = r#"
(() => {
  const el = document.activeElement;
  const value = el && el.value !== undefined ? el.value : '';
  const codepoints = Array.from(value).map((c) => c.codePointAt(0).toString(16)).join(' ');
  document.title = 'site:value=' + encodeURIComponent(value)
    + '|caret=' + (el && el.selectionStart)
    + '|cp=' + encodeURIComponent(codepoints)
    + '|tag=' + encodeURIComponent(el ? el.tagName : 'none')
    + '|events=' + encodeURIComponent((window.__dsArrow || []).join(','))
    + '|guard=' + encodeURIComponent(JSON.stringify(window.__dslocalKeyGuardStats || null));
})();
"#;

const READ_JS: &str = r#"
(() => {
  const el = document.getElementById('__dslocalArrowProbe');
  const codepoints = Array.from(el.value).map((c) => c.codePointAt(0).toString(16)).join(' ');
  document.title = 'arrow:value=' + encodeURIComponent(el.value)
    + '|caret=' + el.selectionStart + '|cp=' + encodeURIComponent(codepoints)
    + '|focused=' + (document.activeElement === el)
    + '|events=' + encodeURIComponent((window.__dsArrow || []).join(','))
    + '|guard=' + encodeURIComponent(JSON.stringify(window.__dslocalKeyGuardStats || null));
})();
"#;

/// Key table: name -> (characters, virtual key code).
fn key(name: &str) -> Option<(String, u16)> {
    let pair = match name {
        "left" => ("\u{f702}".to_string(), 123),
        "right" => ("\u{f703}".to_string(), 124),
        "up" => ("\u{f700}".to_string(), 126),
        "down" => ("\u{f701}".to_string(), 125),
        "space" => (" ".to_string(), 49),
        "enter" => ("\r".to_string(), 36),
        "escape" => ("\u{1b}".to_string(), 53),
        "backspace" => ("\u{8}".to_string(), 51),
        "tab" => ("\t".to_string(), 48),
        single if single.chars().count() == 1 => {
            let ch = single.chars().next()?;
            (ch.to_string(), letter_code(ch)?)
        }
        _ => return None,
    };
    Some(pair)
}

/// ANSI virtual key codes for a-z, enough for the probe's typing steps.
fn letter_code(ch: char) -> Option<u16> {
    let code = match ch {
        'a' => 0,
        's' => 1,
        'd' => 2,
        'f' => 3,
        'h' => 4,
        'g' => 5,
        'z' => 6,
        'x' => 7,
        'c' => 8,
        'v' => 9,
        'b' => 11,
        'q' => 12,
        'w' => 13,
        'e' => 14,
        'r' => 15,
        'y' => 16,
        't' => 17,
        'o' => 31,
        'u' => 32,
        'i' => 34,
        'p' => 35,
        'l' => 37,
        'j' => 38,
        'k' => 40,
        'n' => 45,
        'm' => 46,
        _ => return None,
    };
    Some(code)
}

fn eval(view: &WKWebView, js: &str) {
    let source = NSString::from_str(js);
    // SAFETY: main thread, and a missing completion handler is allowed.
    unsafe { view.evaluateJavaScript_completionHandler(&source, None) };
}

fn title(view: &WKWebView) -> String {
    // SAFETY: the webview is owned by the window and lives on the main thread.
    unsafe { view.title().map(|title| title.to_string()).unwrap_or_default() }
}

fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => match u8::from_str_radix(&text[index + 1..index + 3], 16) {
                Ok(byte) => {
                    out.push(byte);
                    index += 3;
                }
                Err(_) => {
                    out.push(bytes[index]);
                    index += 1;
                }
            },
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    format!("{:?}", String::from_utf8_lossy(&out))
}

fn read_probe(view: &WKWebView) -> String {
    let title = title(view);
    let rest = match title.strip_prefix("arrow:value=") {
        Some(rest) => rest,
        None => match title.strip_prefix("site:value=") {
            Some(rest) => rest,
            None => return format!("(no result, title = {title:?})"),
        },
    };
    let mut out = String::new();
    for part in rest.split('|') {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        out.push_str(&format!(" {key}={}", decode(value)));
    }
    out
}

fn with_webview<T>(cx: &mut AsyncApp, handle: AnyWindowHandle, f: impl FnOnce(&Window) -> T) -> Option<T> {
    cx.update(|app: &mut App| handle.update(app, |_, window, _| f(window)).ok())
}

pub fn schedule(cx: &mut App, window: &mut Window) {
    if std::env::var_os("DSLOCAL_ARROW_TEST").is_none() {
        return;
    }
    let handle = window.window_handle();
    cx.spawn(async move |cx: &mut AsyncApp| {
        for _ in 0..60 {
            let loading = with_webview(cx, handle, crate::window_ext::webview_is_loading).flatten();
            if loading == Some(false) {
                break;
            }
            cx.background_executor().timer(Duration::from_millis(200)).await;
        }
        settle(cx).await;

        let Some(mtm) = MainThreadMarker::new() else {
            eprintln!("[arrow] probe needs the main thread");
            return;
        };
        let Some(view) = with_webview(cx, handle, crate::window_ext::find_webview).flatten() else {
            eprintln!("[arrow] probe found no webview");
            return;
        };
        let responder = with_webview(cx, handle, crate::window_ext::focus_webview).flatten();
        eprintln!("[arrow] first responder = {responder:?}");

        let number = with_webview(cx, handle, |window| {
            crate::window_ext::ns_window(window).map(|native| native.windowNumber())
        })
        .flatten()
        .unwrap_or(0);
        let stamp = objc2_foundation::NSProcessInfo::processInfo().systemUptime();

        let app = NSApplication::sharedApplication(mtm);
        app.activate();
        settle(cx).await;
        eprintln!("[arrow] app active = {}", app.isActive());

        {
            let block: block2::RcBlock<dyn Fn(std::ptr::NonNull<NSEvent>) -> *mut NSEvent> =
                block2::RcBlock::new(|event: std::ptr::NonNull<NSEvent>| -> *mut NSEvent {
                    // SAFETY: AppKit owns the event for the duration of this callback.
                    let native = unsafe { event.as_ref() };
                    let chars = native.characters().map(|chars| chars.to_string()).unwrap_or_default();
                    let codepoints = chars
                        .chars()
                        .map(|ch| format!("{:x}", ch as u32))
                        .collect::<Vec<_>>()
                        .join(" ");
                    eprintln!(
                        "[arrow] app keydown code={} chars_len={} cp=[{}] window={}",
                        native.keyCode(),
                        chars.encode_utf16().count(),
                        codepoints,
                        native.windowNumber()
                    );
                    event.as_ptr()
                });
            let monitor = unsafe {
                NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &block)
            };
            match monitor {
                // Both live for the whole run; leaking them is fine in a probe.
                Some(monitor) => std::mem::forget((block, monitor)),
                None => eprintln!("[arrow] app monitor unavailable"),
            }
        }

        let target = std::env::var("DSLOCAL_ARROW_TARGET").unwrap_or_else(|_| "own".to_string());
        let steps = std::env::var("DSLOCAL_ARROW_STEPS").unwrap_or_else(|_| "left".to_string());
        let via = std::env::var("DSLOCAL_ARROW_VIA").unwrap_or_else(|_| "queue".to_string());
        eval(&view, if target == "site" { ARM_SITE_JS } else { ARM_JS });
        settle(cx).await;
        eprintln!("[arrow] target = {target}, title = {:?}", title(&view));
        eval(&view, "window.__dsArrow && (window.__dsArrow.length = 0);");
        settle_small(cx).await;
        eprintln!("[arrow] steps = {steps:?} via {via}");

        for step in steps.split_whitespace() {
            if step == "wait" {
                settle(cx).await;
                eprintln!("[arrow] waited");
                continue;
            }
            let Some((chars, key_code)) = key(step) else {
                eprintln!("[arrow] unknown step {step:?}");
                continue;
            };
            let chars = NSString::from_str(&chars);
            let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
                NSEventType::KeyDown,
                NSPoint::new(0.0, 0.0),
                NSEventModifierFlags::empty(),
                stamp,
                number,
                None,
                &chars,
                &chars,
                false,
                key_code,
            );
            match event {
                Some(event) => {
                    if via == "direct" {
                        let _: () = unsafe { objc2::msg_send![&*view, keyDown: &*event] };
                    } else {
                        app.postEvent_atStart(&event, false);
                    }
                    eprintln!("[arrow] sent {step}");
                }
                None => eprintln!("[arrow] could not build the {step} event"),
            }
            settle_small(cx).await;
        }

        settle(cx).await;
        // Optional: drive real keyboard input from outside the process. The
        // script runs while the page holds focus, then the result is read back.
        if let Ok(script) = std::env::var("DSLOCAL_ARROW_SCRIPT") {
            eprintln!("[arrow] running {script}");
            match std::process::Command::new("/bin/sh").arg("-c").arg(&script).status() {
                Ok(status) => eprintln!("[arrow] script exited {status}"),
                Err(error) => eprintln!("[arrow] script failed: {error}"),
            }
            settle(cx).await;
            settle(cx).await;
        }

        if let Ok(js) = std::env::var("DSLOCAL_ARROW_JS") {
            eprintln!("[arrow] eval js = {js}");
            eval(&view, &js);
            settle(cx).await;
        }

        eval(&view, if target == "site" { READ_SITE_JS } else { READ_JS });
        settle(cx).await;
        eprintln!("[arrow] RESULT steps={steps:?} = {}", read_probe(&view));
    })
    .detach();
}

async fn settle_small(cx: &mut AsyncApp) {
    cx.background_executor().timer(Duration::from_millis(350)).await;
}

async fn settle(cx: &mut AsyncApp) {
    cx.background_executor().timer(Duration::from_millis(700)).await;
}
