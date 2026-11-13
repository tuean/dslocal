//! AppKit local event monitor that routes Command-key strokes through the
//! gpui keymap. Without it a click into the WKWebView makes it first responder
//! and gpui's own keyDown never sees cmd-R / cmd-[ / cmd-] / cmd-0.

use std::cell::RefCell;
use std::ptr::NonNull;
use std::time::Duration;

use gpui::{AnyWindowHandle, App, AsyncApp, Keystroke, Modifiers, Window};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSEvent, NSEventMask, NSEventModifierFlags, NSEventType, NSWindow,
};
use objc2_foundation::{NSPoint, NSString};

struct State {
    app: *mut App,
    window: AnyWindowHandle,
    native: Retained<NSWindow>,
    window_number: isize,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    static MONITOR: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
}

/// Installs the monitor for the lifetime of the app. A failure is logged and
/// the app keeps running with the old (focus-dependent) shortcut behaviour.
pub fn install(app: &mut App, window: &mut Window) {
    let Some(native) = crate::window_ext::ns_window(window) else {
        eprintln!("[keys] local monitor unavailable");
        return;
    };
    if MainThreadMarker::new().is_none() {
        eprintln!("[keys] local monitor unavailable");
        return;
    }

    let state = State {
        app: app as *mut App,
        window: window.window_handle(),
        window_number: native.windowNumber(),
        native,
    };
    // A reopened window must not stack a second monitor: the old one would
    // fire for the same key and route it against a dead window handle.
    let already = MONITOR.with(|cell| cell.borrow().is_some());
    STATE.with(|cell| *cell.borrow_mut() = Some(state));
    if already {
        eprintln!("[keys] local monitor reused for the new window");
        return;
    }

    let block: block2::RcBlock<dyn Fn(NonNull<NSEvent>) -> *mut NSEvent> =
        block2::RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
            // SAFETY: AppKit owns the event for the duration of this callback.
            let native = unsafe { event.as_ref() };
            if route(native) {
                std::ptr::null_mut()
            } else {
                event.as_ptr()
            }
        });

    match unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &block)
    } {
        Some(monitor) => {
            MONITOR.with(|cell| *cell.borrow_mut() = Some(monitor));
            eprintln!("[keys] local monitor installed for keyDown");
        }
        None => eprintln!("[keys] local monitor unavailable"),
    }
}

/// Feeds one native keyDown to the keymap. Returns true when a binding consumed
/// it, in which case the caller must swallow the event.
pub fn route(native: &NSEvent) -> bool {
    if native.r#type() != NSEventType::KeyDown {
        return false;
    }
    let flags = native.modifierFlags();
    // Plain keys belong to the page; cmd-C / cmd-V / cmd-A reach the keymap and,
    // when nothing claims them, are forwarded to the page as editing commands.
    if !flags.contains(NSEventModifierFlags::Command) {
        return false;
    }
    // control/option combos stay with the system or the page.
    if flags.intersects(NSEventModifierFlags::Control | NSEventModifierFlags::Option) {
        return false;
    }
    let Some((app, window)) = STATE.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|state| (state.app, state.window))
    }) else {
        return false;
    };

    let keystroke = keystroke(native, flags);
    if keystroke.key.is_empty() {
        return false;
    }
    let label = describe(&keystroke);
    // SAFETY: the monitor fires from the run loop, outside any gpui borrow of App.
    let app = unsafe { &mut *app };
    let handled = window.update(app, |_, window, cx| {
        if window.dispatch_keystroke(keystroke.clone(), cx) {
            return true;
        }
        // Nothing in the app claims it: hand standard text editing back to the
        // page instead of letting the event fall into the void.
        forward_editing_command(window, &keystroke)
    });
    match handled {
        Ok(true) => {
            eprintln!("[keys] monitor consumed {label}");
            true
        }
        Ok(false) => false,
        Err(_) => false,
    }
}

/// Editing actions WebKit expects to receive as `NSResponder` commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Editing {
    Paste,
    Copy,
    Cut,
    SelectAll,
    Undo,
    Redo,
}

fn editing(keystroke: &Keystroke) -> Option<Editing> {
    match (keystroke.key.as_str(), keystroke.modifiers.shift) {
        ("v", false) => Some(Editing::Paste),
        ("c", false) => Some(Editing::Copy),
        ("x", false) => Some(Editing::Cut),
        ("a", false) => Some(Editing::SelectAll),
        ("z", false) => Some(Editing::Undo),
        ("z", true) => Some(Editing::Redo),
        _ => None,
    }
}

/// cmd-C/V/X/A/Z never reach a WKWebView subview here: AppKit offers key
/// equivalents to the gpui content view first, that view reports them unhandled,
/// and WebKit's editing command is skipped, so the shortcut dies silently.
/// Sending the responder action straight to the page restores text editing.
fn forward_editing_command(window: &Window, keystroke: &Keystroke) -> bool {
    let Some(command) = editing(keystroke) else {
        return false;
    };
    // Send it to the page that owns the keyboard: with two tabs open, the
    // first webview in the tree may be the hidden one.
    let Some(webview) = crate::window_ext::focused_webview(window) else {
        return false;
    };
    // SAFETY: main thread, live webview, standard NSResponder editing actions.
    unsafe {
        let sender: *mut AnyObject = std::ptr::null_mut();
        match command {
            Editing::Paste => {
                let _: () = objc2::msg_send![&*webview, paste: sender];
            }
            Editing::Copy => {
                let _: () = objc2::msg_send![&*webview, copy: sender];
            }
            Editing::Cut => {
                let _: () = objc2::msg_send![&*webview, cut: sender];
            }
            Editing::SelectAll => {
                let _: () = objc2::msg_send![&*webview, selectAll: sender];
            }
            Editing::Undo => {
                let _: () = objc2::msg_send![&*webview, undo: sender];
            }
            Editing::Redo => {
                let _: () = objc2::msg_send![&*webview, redo: sender];
            }
        }
    }
    true
}

fn keystroke(native: &NSEvent, flags: NSEventModifierFlags) -> Keystroke {
    let chars = native.charactersIgnoringModifiers()
        .map(|chars| chars.to_string())
        .unwrap_or_default();
    let mut modifiers = Modifiers {
        control: flags.contains(NSEventModifierFlags::Control),
        alt: flags.contains(NSEventModifierFlags::Option),
        shift: flags.contains(NSEventModifierFlags::Shift),
        platform: flags.contains(NSEventModifierFlags::Command),
        function: flags.contains(NSEventModifierFlags::Function),
    };
    let key = key_text(&chars, &mut modifiers);
    Keystroke {
        modifiers,
        key,
        key_char: None,
    }
}

/// AppKit bakes shift into the character it reports (shift+= reports "+"), so
/// the modifier is dropped unless the pair is a plain letter (gpui's own rule).
fn key_text(chars: &str, modifiers: &mut Modifiers) -> String {
    let mut key = special_key(chars).unwrap_or_else(|| chars.to_string());
    if modifiers.shift {
        if chars.chars().all(|c| c.is_ascii_uppercase()) {
            key = chars.to_ascii_lowercase();
        } else if !chars.chars().all(|c| c.is_ascii_lowercase()) {
            modifiers.shift = false;
        }
    }
    key
}

fn special_key(chars: &str) -> Option<String> {
    let mut iter = chars.chars();
    let ch = iter.next()?;
    if iter.next().is_some() {
        return None;
    }
    let code = ch as u32;
    let name = match code {
        0x20 => "space".to_string(),
        0x09 => "tab".to_string(),
        0x0d | 0x03 => "enter".to_string(),
        0x08 | 0x7f => "backspace".to_string(),
        0x1b => "escape".to_string(),
        0xf700 => "up".to_string(),
        0xf701 => "down".to_string(),
        0xf702 => "left".to_string(),
        0xf703 => "right".to_string(),
        0xf704..=0xf726 => format!("f{}", code - 0xf703),
        0xf728 => "delete".to_string(),
        0xf729 => "home".to_string(),
        0xf72b => "end".to_string(),
        0xf72c => "pageup".to_string(),
        0xf72d => "pagedown".to_string(),
        _ => return None,
    };
    Some(name)
}

fn describe(keystroke: &Keystroke) -> String {
    let mut out = String::new();
    if keystroke.modifiers.platform {
        out.push_str("cmd-");
    }
    if keystroke.modifiers.control {
        out.push_str("ctrl-");
    }
    if keystroke.modifiers.alt {
        out.push_str("alt-");
    }
    if keystroke.modifiers.shift {
        out.push_str("shift-");
    }
    out.push_str(&keystroke.key);
    out
}

/// DSLOCAL_SELFTEST=1: put the WKWebView in the responder chain, then feed a
/// real NSEvent cmd-R to the monitor handler (not dispatch_keystroke) and check
/// the page started loading.
pub fn schedule_selftest(app: &mut App) {
    if std::env::var("DSLOCAL_SELFTEST").is_err() {
        return;
    }
    let Some(window) = STATE.with(|cell| cell.borrow().as_ref().map(|state| state.window)) else {
        return;
    };
    app.spawn(async move |cx: &mut AsyncApp| {
        cx.background_executor()
            .timer(Duration::from_millis(1200))
            .await;
        let responder = cx.update(|app: &mut App| {
            window.update(app, |_, window, _cx| {
                crate::window_ext::focus_webview(window)
            })
        });
        eprintln!("[keys] selftest first responder = {responder:?}");

        let consumed = fire_synthetic("r", 15);
        eprintln!("[keys] selftest monitor route cmd-R consumed = {consumed}");

        let loading = cx.update(|app: &mut App| {
            window.update(app, |_, window, _cx| {
                crate::window_ext::webview_is_loading(window)
            })
        });
        eprintln!("[keys] selftest webview isLoading after route = {loading:?}");

        // cmd-C has no binding: the monitor must hand it back so the page can copy.
        let copied = fire_synthetic("c", 8);
        eprintln!("[keys] selftest monitor route cmd-C consumed = {copied} (false = falls through)");
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command() -> Modifiers {
        Modifiers {
            control: false,
            alt: false,
            shift: false,
            platform: true,
            function: false,
        }
    }

    fn command_keystroke(key: &str, shift: bool) -> Keystroke {
        Keystroke {
            key: key.to_string(),
            modifiers: Modifiers {
                shift,
                ..command()
            },
            ..Default::default()
        }
    }

    #[test]
    fn editing_chords_forward_to_the_page() {
        assert_eq!(editing(&command_keystroke("v", false)), Some(Editing::Paste));
        assert_eq!(editing(&command_keystroke("c", false)), Some(Editing::Copy));
        assert_eq!(editing(&command_keystroke("x", false)), Some(Editing::Cut));
        assert_eq!(
            editing(&command_keystroke("a", false)),
            Some(Editing::SelectAll)
        );
        assert_eq!(editing(&command_keystroke("z", false)), Some(Editing::Undo));
        assert_eq!(editing(&command_keystroke("z", true)), Some(Editing::Redo));
        // Anything the app binds, or that is not an editing command, stays out.
        assert_eq!(editing(&command_keystroke("r", false)), None);
        assert_eq!(editing(&command_keystroke("v", true)), None);
    }

    #[test]
    fn letters_keep_their_case_and_modifier() {
        let mut modifiers = Modifiers {
            shift: true,
            ..command()
        };
        assert_eq!(key_text("r", &mut modifiers), "r");
        assert!(modifiers.shift);
        assert_eq!(key_text("R", &mut modifiers), "r");
        assert!(modifiers.shift);
    }

    #[test]
    fn shifted_symbol_drops_shift() {
        let mut modifiers = Modifiers {
            shift: true,
            ..command()
        };
        assert_eq!(key_text("+", &mut modifiers), "+");
        assert!(!modifiers.shift);
    }

    #[test]
    fn named_keys_and_function_row() {
        assert_eq!(special_key(" ").as_deref(), Some("space"));
        assert_eq!(special_key("\u{f700}").as_deref(), Some("up"));
        assert_eq!(special_key("\u{f704}").as_deref(), Some("f1"));
        assert_eq!(special_key("\u{f726}").as_deref(), Some("f35"));
        assert_eq!(special_key("\u{8}").as_deref(), Some("backspace"));
        assert_eq!(special_key("0"), None);
    }

    #[test]
    fn describe_round_trip() {
        let keystroke = Keystroke {
            modifiers: Modifiers {
                shift: true,
                ..command()
            },
            key: "=".to_string(),
            key_char: None,
        };
        assert_eq!(describe(&keystroke), "cmd-shift-=");
    }
}

fn fire_synthetic(key: &str, key_code: u16) -> bool {
    let Some((number, _native)) = STATE.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|state| (state.window_number, state.native.clone()))
    }) else {
        return false;
    };
    let chars = NSString::from_str(key);
    let event =
        NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            NSEventType::KeyDown,
            NSPoint::new(0.0, 0.0),
            NSEventModifierFlags::Command,
            0.0,
            number,
            None,
            &chars,
            &chars,
            false,
            key_code,
        );
    let Some(event) = event else {
        eprintln!("[keys] selftest could not build NSEvent");
        return false;
    };
    route(&event)
}
