mod arrowprobe;
mod chrome;
mod hotkey;
mod keyroute;
mod pasteprobe;
mod state;
mod webview;
mod window_ext;

use std::time::Duration;

use gpui::{App, AppContext as _, AsyncApp, Window, WindowOptions, point, px};
use gpui_component::{Root, Theme, ThemeMode};

use chrome::DsLocalApp;
use state::State;

fn main() {
    let application = gpui_platform::application();

    // Registered before the run loop starts: macOS asks the app to reopen a
    // window only when none is visible, and the process outlives its last
    // window, so a Dock click has to build a fresh window instead of waking a
    // dead icon.
    application.on_reopen(|cx: &mut App| match cx.windows().first().copied() {
        Some(handle) => {
            let raised = handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok();
            eprintln!("[reopen] raised existing window = {raised}");
        }
        None => {
            open_main_window(cx);
            eprintln!("[reopen] opened a new window");
        }
    });

    application.run(|cx: &mut App| {
        gpui_component::init(cx);
        // gpui-component defaults to the light theme; the toolbar is dark, so
        // pick dark explicitly instead of following the system appearance.
        Theme::change(ThemeMode::Dark, None, cx);

        // Shortcuts are global: the keymap resolves them wherever focus sits.
        cx.bind_keys(chrome::key_bindings());

        open_main_window(cx);
    });
}

fn open_main_window(cx: &mut App) {
    let state = State::load();
    // Needs NSApplication, so this runs after the platform app exists. A
    // missing image is logged and never fatal.
    window_ext::apply_dock_icon(&state.icon);
    let bounds = state.restored_bounds(cx);
    eprintln!("[window] opening at {bounds:?}");

    cx.open_window(
        WindowOptions {
            window_bounds: Some(bounds),
            titlebar: Some(gpui::TitlebarOptions {
                title: Some("DeepSeek Local".into()),
                // Hide the system title text and extend the content view
                // under the traffic lights: our 44pt toolbar is the bar.
                appears_transparent: true,
                traffic_light_position: Some(point(px(12.0), px(15.0))),
            }),
            // The app draws and drags its own titlebar (start_window_move).
            app_owns_titlebar_drag: true,
            ..Default::default()
        },
        move |window, cx| {
            let view = cx.new(|cx| DsLocalApp::new(state, cx));
            // Routes cmd-combos through the keymap even when the WKWebView
            // owns first responder; needs the live window and App pointers.
            keyroute::install(cx, window);
            keyroute::schedule_selftest(cx);
            pasteprobe::schedule(cx, window);
            arrowprobe::schedule(cx, window);
            if let Some(native) = window_ext::ns_window(window) {
                hotkey::install(native);
            } else {
                eprintln!("[hotkey] register failed status=no-window");
            }
            hotkey::schedule_test(cx);
            schedule_reopen_test(cx, window);
            cx.new(|cx| Root::new(view, window, cx))
        },
    )
    .expect("failed to open window");

    cx.activate(true);
}

/// DSLOCAL_REOPEN_TEST=1: closes the window through the red-button path, so an
/// external `open -a` afterwards exercises the real reopen callback.
fn schedule_reopen_test(cx: &mut App, window: &mut Window) {
    static CLOSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if std::env::var("DSLOCAL_REOPEN_TEST").is_err()
        || CLOSED.swap(true, std::sync::atomic::Ordering::SeqCst)
    {
        return;
    }
    let Some(native) = window_ext::ns_window(window) else {
        eprintln!("[reopen-test] no native window to close");
        return;
    };
    cx.spawn(async move |cx: &mut AsyncApp| {
        cx.background_executor()
            .timer(Duration::from_millis(3000))
            .await;
        eprintln!(
            "[reopen-test] before close: registered = {}, visible = {}",
            cx.update(|app: &mut App| app.windows().len()),
            native.isVisible()
        );
        native.performClose(None);
        cx.background_executor()
            .timer(Duration::from_millis(600))
            .await;
        eprintln!(
            "[reopen-test] after close: registered = {}, visible = {}",
            cx.update(|app: &mut App| app.windows().len()),
            native.isVisible()
        );
    })
    .detach();
}