//! Global option+space hotkey through Carbon's RegisterEventHotKey: the same
//! key summons the window and hides it again. Carbon is part of the macOS SDK,
//! so this needs no extra crate.

use std::cell::RefCell;
use std::ffi::c_void;
use std::time::Duration;

use gpui::{App, AsyncApp};
use objc2::rc::Retained;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSWindow};

const HOTKEY_SIGNATURE: u32 = u32::from_be_bytes(*b"dsLK");
const K_VK_SPACE: u32 = 49;
const OPTION_KEY: u32 = 0x0800;
const K_EVENT_CLASS_KEYBOARD: u32 = u32::from_be_bytes(*b"keyb");
const K_EVENT_HOT_KEY_PRESSED: u32 = 5;
const NO_ERR: i32 = 0;

#[repr(C)]
struct EventHotKeyID {
    signature: u32,
    id: u32,
}

#[repr(C)]
struct EventTypeSpec {
    event_class: u32,
    event_kind: u32,
}

type EventTargetRef = *mut c_void;
type EventHandlerRef = *mut c_void;
type EventHotKeyRef = *mut c_void;
type EventHandlerProc =
    Option<unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32>;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn GetApplicationEventTarget() -> EventTargetRef;
    fn InstallEventHandler(
        target: EventTargetRef,
        handler: EventHandlerProc,
        num_types: usize,
        types: *const EventTypeSpec,
        user_data: *mut c_void,
        out_ref: *mut EventHandlerRef,
    ) -> i32;
    fn RegisterEventHotKey(
        code: u32,
        modifiers: u32,
        id: EventHotKeyID,
        target: EventTargetRef,
        options: u32,
        out_ref: *mut EventHotKeyRef,
    ) -> i32;
}

thread_local! {
    static WINDOW: RefCell<Option<Retained<NSWindow>>> = const { RefCell::new(None) };
    static HANDLER: RefCell<EventHandlerRef> = const { RefCell::new(std::ptr::null_mut()) };
    static HOTKEY: RefCell<EventHotKeyRef> = const { RefCell::new(std::ptr::null_mut()) };
}

/// Registers option+space for the whole session. A refusal from Carbon is
/// logged with its OSStatus and the app keeps running.
pub fn install(native: Retained<NSWindow>) {
    let already = HOTKEY.with(|cell| !cell.borrow().is_null());
    WINDOW.with(|cell| *cell.borrow_mut() = Some(native));
    if already {
        eprintln!("[hotkey] option-space retargeted to the new window");
        return;
    }

    let spec = EventTypeSpec {
        event_class: K_EVENT_CLASS_KEYBOARD,
        event_kind: K_EVENT_HOT_KEY_PRESSED,
    };
    // SAFETY: Carbon keeps the handler for the lifetime of the app; the proc is
    // stateless and its refs are parked in thread-locals, never dropped.
    unsafe {
        let target = GetApplicationEventTarget();
        let mut handler: EventHandlerRef = std::ptr::null_mut();
        let install_status = InstallEventHandler(
            target,
            Some(on_hot_key),
            1,
            &spec,
            std::ptr::null_mut(),
            &mut handler,
        );
        if install_status != NO_ERR {
            eprintln!("[hotkey] handler install failed status={install_status}");
            return;
        }
        HANDLER.with(|cell| *cell.borrow_mut() = handler);

        let mut hotkey: EventHotKeyRef = std::ptr::null_mut();
        let status = RegisterEventHotKey(
            K_VK_SPACE,
            OPTION_KEY,
            EventHotKeyID {
                signature: HOTKEY_SIGNATURE,
                id: 1,
            },
            target,
            0,
            &mut hotkey,
        );
        if status != NO_ERR {
            eprintln!("[hotkey] register failed status={status}");
            return;
        }
        HOTKEY.with(|cell| *cell.borrow_mut() = hotkey);
        eprintln!("[hotkey] option-space registered");
    }
}

unsafe extern "C" fn on_hot_key(_call: *mut c_void, _event: *mut c_void, _user: *mut c_void) -> i32 {
    toggle("hotkey");
    NO_ERR
}

/// Front window -> hide; otherwise activate the app and bring the window back.
pub fn toggle(tag: &str) {
    let before = window_state();
    let Some((visible, key)) = before else {
        eprintln!("[hotkey] no window to toggle");
        return;
    };
    let Some(native) = WINDOW.with(|cell| cell.borrow().as_ref().cloned()) else {
        return;
    };

    if should_hide(visible, key) {
        native.orderOut(None);
    } else {
        if let Some(mtm) = MainThreadMarker::new() {
            let app = NSApplication::sharedApplication(mtm);
            // activate(_:) is macOS 14+; the older call is the only portable one.
            #[allow(deprecated)]
            app.activateIgnoringOtherApps(true);
        }
        native.makeKeyAndOrderFront(None);
    }

    match window_state() {
        Some((visible_after, key_after)) => eprintln!(
            "[hotkey] {tag}: visible {visible}->{visible_after}, key {key}->{key_after}"
        ),
        None => eprintln!("[hotkey] {tag}: window disappeared"),
    }
}

fn should_hide(visible: bool, key: bool) -> bool {
    visible && key
}

fn window_state() -> Option<(bool, bool)> {
    WINDOW.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|window| (window.isVisible(), window.isKeyWindow()))
    })
}

/// DSLOCAL_HOTKEY_TEST=1: toggle twice from the run loop so the logged
/// isVisible / isKeyWindow pairs show the real transition.
pub fn schedule_test(app: &mut App) {
    if std::env::var("DSLOCAL_HOTKEY_TEST").is_err() {
        return;
    }
    app.spawn(async move |cx: &mut AsyncApp| {
        cx.background_executor()
            .timer(Duration::from_millis(3000))
            .await;
        force_hide();
        toggle("test 1");
        cx.background_executor()
            .timer(Duration::from_millis(1200))
            .await;
        // The hide branch needs a key window; on a locked screen AppKit refuses
        // to hand it over, which the logged bool makes visible instead of silent.
        let key = make_key();
        eprintln!("[hotkey] test 2 precondition: makeKeyWindow -> key = {key:?}");
        toggle("test 2");
        cx.background_executor()
            .timer(Duration::from_millis(600))
            .await;
        cx.update(|app: &mut App| app.quit());
    })
    .detach();
}

fn force_hide() {
    if let Some(native) = WINDOW.with(|cell| cell.borrow().as_ref().cloned()) {
        native.orderOut(None);
        eprintln!("[hotkey] test baseline after orderOut = {:?}", window_state());
    }
}

fn make_key() -> Option<bool> {
    let native = WINDOW.with(|cell| cell.borrow().as_ref().cloned())?;
    native.makeKeyWindow();
    Some(native.isKeyWindow())
}

#[cfg(test)]
mod tests {
    use super::should_hide;

    #[test]
    fn front_window_hides_others_show() {
        assert!(should_hide(true, true));
        assert!(!should_hide(true, false));
        assert!(!should_hide(false, false));
    }
}
