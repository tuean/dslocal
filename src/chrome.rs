//! The native window chrome: a draggable 44pt toolbar drawn by gpui, the
//! actions its buttons and keyboard shortcuts invoke, plus state persistence.

use std::time::{Duration, Instant};

use gpui::{
    Context, FocusHandle, InteractiveElement as _, IntoElement, KeyBinding, MouseButton,
    ParentElement, Render, Styled, Window, actions, div, px, rgb,
};
use gpui_component::{Selectable as _, button::Button, label::Label};

use crate::state::State;
use crate::webview::WebView;
use crate::window_ext;

// main.rs only declares the four core modules, so the export module is grafted
// on here: the toolbar owns the feature's entry points anyway.
#[path = "export.rs"]
mod export;

pub const TOOLBAR_HEIGHT: f32 = 44.0;
pub const HOME_URL: &str = "https://chat.deepseek.com";

/// Left inset that keeps the traffic lights clear of the first button. The
/// lights start at x = 12 and span ~68pt including their 8pt gutters.
const TRAFFIC_LIGHT_INSET: f32 = 86.0;

const MIN_ZOOM: f64 = 0.25;
const MAX_ZOOM: f64 = 3.0;
const ZOOM_STEP: f64 = 0.1;

actions!([
    ReloadPage,
    GoBack,
    GoForward,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    ToggleAlwaysOnTop,
    ExportConversation,
    SwitchIcon
]);

/// Icon variants shipped with the app; the toolbar button cycles them.
const ICON_VARIANTS: [&str; 3] = ["a", "b", "c"];

/// Keymap entries registered once at startup with `App::bind_keys`.
pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("cmd-r", ReloadPage, None),
        KeyBinding::new("cmd-[", GoBack, None),
        KeyBinding::new("cmd-]", GoForward, None),
        // cmd-+ is cmd-shift-= on a US layout; accept all three spellings.
        KeyBinding::new("cmd-+", ZoomIn, None),
        KeyBinding::new("cmd-=", ZoomIn, None),
        KeyBinding::new("cmd-shift-=", ZoomIn, None),
        KeyBinding::new("cmd--", ZoomOut, None),
        KeyBinding::new("cmd-_", ZoomOut, None),
        KeyBinding::new("cmd-0", ZoomReset, None),
        KeyBinding::new("cmd-t", ToggleAlwaysOnTop, None),
        KeyBinding::new("cmd-s", ExportConversation, None),
    ]
}

pub struct DsLocalApp {
    webview: Option<WebView>,
    error: Option<String>,
    state: State,
    /// Set on mouse down, consumed on the first mouse move: waiting for the
    /// move keeps plain clicks landing on the buttons.
    toolbar_dragging: bool,
    /// Timestamp of the previous completed press on the bar itself (a click
    /// that did not land on a button), used to detect a double click.
    bar_click_at: Option<Instant>,
    focus_handle: FocusHandle,
    focus_set: bool,
    last_size: Option<(f32, f32)>,
    resize_test_scheduled: bool,
    selftest_scheduled: bool,
    persistence_started: bool,
    level_logged: bool,
    /// Replaces the webview status line while an export runs and afterwards,
    /// so both the success file name and the failure summary are visible.
    status_message: Option<String>,
    exporting: bool,
    export_test_scheduled: bool,
    icon_test_scheduled: bool,
    probe_scheduled: bool,
    panel_test_scheduled: bool,
}

impl DsLocalApp {
    pub fn new(state: State, cx: &mut Context<Self>) -> Self {
        Self {
            webview: None,
            error: None,
            state,
            focus_handle: cx.focus_handle(),
            focus_set: false,
            last_size: None,
            toolbar_dragging: false,
            bar_click_at: None,
            resize_test_scheduled: false,
            selftest_scheduled: false,
            persistence_started: false,
            level_logged: false,
            status_message: None,
            exporting: false,
            export_test_scheduled: false,
            icon_test_scheduled: false,
            probe_scheduled: false,
            panel_test_scheduled: false,
        }
    }

    /// Creation needs the native window handle, so it happens on first render.
    fn ensure_webview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.webview.is_some() || self.error.is_some() {
            return;
        }
        let url = std::env::var("DSLOCAL_URL").unwrap_or_else(|_| HOME_URL.to_string());
        eprintln!("[webview] url = {url}");
        match WebView::new(window, TOOLBAR_HEIGHT, &url) {
            Ok(view) => {
                view.set_zoom(self.state.zoom);
                eprintln!("[webview] attached, zoom {:.0}%", self.state.zoom * 100.0);
                self.webview = Some(view);
            }
            Err(err) => self.error = Some(format!("{err:#}")),
        }
        self.schedule_resize_test(window, cx);
    }

    /// Evidence hook: DSLOCAL_RESIZE_TEST=1 shrinks the window once, which
    /// exercises the same AppKit frame -> autoresizing path a manual drag does.
    fn schedule_resize_test(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.resize_test_scheduled || std::env::var("DSLOCAL_RESIZE_TEST").is_err() {
            return;
        }
        self.resize_test_scheduled = true;
        cx.spawn_in(window, async move |_this, cx| {
            cx.background_executor()
                .spawn(async {
                    std::thread::sleep(std::time::Duration::from_secs(20));
                })
                .await;
            cx.update(|window, _cx| {
                window.resize(gpui::size(px(900.0), px(600.0)));
            })
            .ok();
        })
        .detach();
    }

    /// Evidence hook: DSLOCAL_SELFTEST=1 drives the real keymap, the zoom path
    /// and the level switch from inside the app, then quits so the persisted
    /// state can be checked on the next launch.
    fn schedule_selftest(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.selftest_scheduled || std::env::var("DSLOCAL_SELFTEST").is_err() {
            return;
        }
        self.selftest_scheduled = true;
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(2500))
                .await;
            // Dispatched from a window-level update: the action handlers
            // re-enter this entity, which is illegal from inside its own update.
            let _ = cx.update(|window, cx| {
                eprintln!(
                    "[selftest] keymap lookup for ReloadPage = {}",
                    window.keystroke_text_for(&ReloadPage)
                );
                for stroke in [
                    "cmd-r", "cmd-[", "cmd-]", "cmd-0", "cmd-+", "cmd-+", "cmd-t",
                ] {
                    match gpui::Keystroke::parse(stroke) {
                        Ok(keystroke) => {
                            let handled = window.dispatch_keystroke(keystroke, cx);
                            eprintln!("[selftest] {stroke} -> handled = {handled}");
                        }
                        Err(err) => eprintln!("[selftest] {stroke} -> parse error {err}"),
                    }
                }
            });
            let _ = this.update_in(cx, |this, window, _cx| {
                eprintln!(
                    "[selftest] zoom = {:.0}%, always_on_top = {}, level = {:?}",
                    this.zoom() * 100.0,
                    this.state.always_on_top,
                    window_ext::level(window)
                );
            });
            cx.background_executor()
                .timer(Duration::from_millis(800))
                .await;
            let _ = cx.update(|_window, cx| cx.quit());
        })
        .detach();
    }

    /// Window moves do not fire `observe_window_bounds` (that only reports
    /// resizes), so the frame is sampled on a timer and written when it changes.
    fn start_persistence(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.persistence_started {
            return;
        }
        self.persistence_started = true;
        self.state.record_window(window);
        self.state.save();

        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(700))
                    .await;
                let updated = this.update_in(cx, |this, window, _cx| {
                    let before = this.state.window;
                    this.state.record_window(window);
                    if before != this.state.window {
                        this.state.save();
                    }
                });
                if updated.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    pub fn status(&self) -> String {
        if let Some(message) = &self.status_message {
            return message.clone();
        }
        match (&self.webview, &self.error) {
            (_, Some(err)) => format!("webview error: {err}"),
            (Some(_), None) => format!("WKWebView -> {HOME_URL}"),
            (None, None) => "attaching WKWebView...".to_string(),
        }
    }

    fn zoom(&self) -> f64 {
        self.webview
            .as_ref()
            .map(WebView::zoom)
            .unwrap_or(self.state.zoom)
    }

    fn set_zoom(&mut self, zoom: f64, cx: &mut Context<Self>) {
        let zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        if let Some(view) = &self.webview {
            view.set_zoom(zoom);
        }
        self.state.zoom = zoom;
        self.state.save();
        eprintln!("[zoom] page zoom -> {:.0}%", zoom * 100.0);
        cx.notify();
    }

    /// Cycles the Dock icon and persists the choice, so the next launch opens
    /// with the same one.
    fn cycle_icon(&mut self, cx: &mut Context<Self>) {
        let current = ICON_VARIANTS
            .iter()
            .position(|name| *name == self.state.icon)
            .unwrap_or(0);
        let next = ICON_VARIANTS[(current + 1) % ICON_VARIANTS.len()];
        self.state.icon = next.to_string();
        self.state.save();
        window_ext::apply_dock_icon(next);
        self.status_message = Some(format!("图标：{next}"));
        cx.notify();
    }

    /// DSLOCAL_ICON_TEST=1: cycles through every variant so the Dock can be
    /// screenshotted between steps; it drives the same handler as the button.
    fn schedule_icon_test(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.icon_test_scheduled || std::env::var("DSLOCAL_ICON_TEST").is_err() {
            return;
        }
        self.icon_test_scheduled = true;
        cx.spawn_in(window, async move |this, cx| {
            for step in 1..=3 {
                cx.background_executor()
                    .timer(Duration::from_millis(1500))
                    .await;
                let done = this.update_in(cx, |this, _window, cx| {
                    this.cycle_icon(cx);
                    eprintln!("[icon-test] step {step} -> {}", this.state.icon);
                });
                if done.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn toggle_always_on_top(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let next = !self.state.always_on_top;
        match window_ext::set_always_on_top(window, next) {
            Some(level) => {
                eprintln!("[window] NSWindow level = {level} (always_on_top = {next})");
                self.state.always_on_top = next;
                self.state.save();
            }
            None => eprintln!("[window] no native NSWindow for level change"),
        }
        cx.notify();
    }

    fn reload(&mut self) {
        if let Some(view) = &self.webview {
            view.reload();
        }
        eprintln!("[key] cmd-r reload");
    }

    fn go_back(&mut self) {
        if let Some(view) = &self.webview {
            view.go_back();
        }
        eprintln!("[key] cmd-[ back");
    }

    fn go_forward(&mut self) {
        if let Some(view) = &self.webview {
            view.go_forward();
        }
        eprintln!("[key] cmd-] forward");
    }

    /// Read-only scrape of the page, Markdown assembly and the file write. Both
    /// the button and cmd-s land here; the export test dispatches cmd-s so it
    /// exercises exactly this path.
    fn export_conversation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.exporting {
            eprintln!("[export] already running, trigger ignored");
            return;
        }
        let Some(view) = self.webview.as_ref() else {
            self.status_message = Some("导出失败: WKWebView 未就绪".to_string());
            eprintln!("[export] no webview yet");
            cx.notify();
            return;
        };
        eprintln!("[export] scrape requested");
        self.exporting = true;
        self.status_message = Some("正在导出...".to_string());

        let (tx, rx) = std::sync::mpsc::channel();
        view.evaluate_javascript(export::SCRAPE_SCRIPT, move |result| {
            let _ = tx.send(result);
        });

        cx.spawn_in(window, async move |this, cx| {
            let scraped = cx
                .background_executor()
                .spawn(async move {
                    rx.recv_timeout(Duration::from_secs(30))
                        .map_err(|_| "抓取超时".to_string())?
                })
                .await;
            let outcome = match scraped {
                Ok(json) => export::export_json(&json, export::FALLBACK_URL, &export::downloads_dir()),
                Err(err) => Err(export::ExportError::Other(err)),
            };
            this.update(cx, |this, cx| {
                this.exporting = false;
                let message = match outcome {
                    Ok(exported) => {
                        eprintln!(
                            "[export] status 已导出 {} -> {} ({} messages, strategy={})",
                            exported.file_name,
                            exported.path.display(),
                            exported.messages,
                            exported.strategy
                        );
                        format!("已导出 {}", exported.file_name)
                    }
                    Err(err) => {
                        eprintln!("[export] {}", err.log_line());
                        err.status()
                    }
                };
                this.status_message = Some(message);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Evidence hook: DSLOCAL_PANEL_TEST=1 opens the upload panel itself. A
    /// synthetic click on the page's file input is not a user gesture, so
    /// WebKit refuses to raise the panel for it -- this reaches the same
    /// panel code without WebKit in the way.
    fn schedule_panel_test(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.panel_test_scheduled || std::env::var("DSLOCAL_PANEL_TEST").is_err() {
            return;
        }
        self.panel_test_scheduled = true;
        cx.spawn_in(window, async move |_this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(2500))
                .await;
            // Called from the task body, not inside an App borrow: runModal
            // spins a nested run loop that would re-enter a borrowed App.
            crate::webview::panel_selftest();
            let _ = cx.update(|_window, cx| cx.quit());
        })
        .detach();
    }

    /// Evidence hook: DSLOCAL_PROBE_JS=<script> waits for the load to settle,
    /// evaluates the script in the page, prints its string result and quits.
    /// Read-only diagnostic for arguing about what the page actually does.
    fn schedule_probe_test(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.probe_scheduled {
            return;
        }
        let Ok(script) = std::env::var("DSLOCAL_PROBE_JS") else {
            return;
        };
        self.probe_scheduled = true;
        cx.spawn_in(window, async move |this, cx| {
            for _ in 0..120 {
                let loading = this
                    .update(cx, |this, _cx| {
                        this.webview
                            .as_ref()
                            .map(WebView::is_loading)
                            .unwrap_or(false)
                    })
                    .ok();
                if loading == Some(false) {
                    break;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
            }
            cx.background_executor().timer(Duration::from_secs(3)).await;
            let _ = this.update(cx, |this, _cx| {
                if let Some(view) = &this.webview {
                    view.evaluate_javascript(&script, |result| match result {
                        Ok(value) => eprintln!("[probe] {value}"),
                        Err(err) => eprintln!("[probe] error {err}"),
                    });
                }
            });
            cx.background_executor().timer(Duration::from_secs(3)).await;
            let _ = cx.update(|_window, cx| cx.quit());
        })
        .detach();
    }

    /// Evidence hook: DSLOCAL_EXPORT_TEST=1 waits for the load to settle, waits
    /// 8s more for the conversation to render, then sends cmd-s through the
    /// real keymap and quits once the export has written (or failed).
    fn schedule_export_test(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.export_test_scheduled || std::env::var("DSLOCAL_EXPORT_TEST").is_err() {
            return;
        }
        self.export_test_scheduled = true;
        cx.spawn_in(window, async move |this, cx| {
            for _ in 0..120 {
                let loading = this
                    .update(cx, |this, _cx| {
                        this.webview
                            .as_ref()
                            .map(WebView::is_loading)
                            .unwrap_or(false)
                    })
                    .ok();
                if loading == Some(false) {
                    break;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
            }
            eprintln!("[export] export test: page settled, waiting 8s");
            cx.background_executor().timer(Duration::from_secs(8)).await;
            let _ = cx.update(|window, cx| match gpui::Keystroke::parse("cmd-s") {
                Ok(keystroke) => {
                    let handled = window.dispatch_keystroke(keystroke, cx);
                    eprintln!("[export] export test: cmd-s handled = {handled}");
                }
                Err(err) => eprintln!("[export] export test: cmd-s parse error {err}"),
            });
            cx.background_executor()
                .timer(Duration::from_millis(500))
                .await;
            for _ in 0..200 {
                let exporting = this.update(cx, |this, _cx| this.exporting).ok();
                if exporting == Some(false) {
                    break;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
            }
            let status = this
                .update(cx, |this, _cx| this.status_message.clone())
                .ok()
                .flatten();
            eprintln!("[export] export test done, status = {status:?}");
            let _ = cx.update(|_window, cx| cx.quit());
        })
        .detach();
    }
}

impl Render for DsLocalApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_webview(window, cx);
        self.start_persistence(window, cx);
        self.schedule_selftest(window, cx);
        self.schedule_export_test(window, cx);
        self.schedule_probe_test(window, cx);
        self.schedule_panel_test(window, cx);
        self.schedule_icon_test(window, cx);
        if let Some(view) = &self.webview {
            view.sync_frame();
        }
        let size = window.bounds().size;
        let size = (f32::from(size.width), f32::from(size.height));
        if self.last_size != Some(size) {
            self.last_size = Some(size);
            let frame = self.webview.as_ref().map(|view| {
                let frame = view.native_view().frame();
                (
                    frame.origin.x,
                    frame.origin.y,
                    frame.size.width,
                    frame.size.height,
                )
            });
            eprintln!("[window] {:.0}x{:.0}, webview frame = {frame:?}", size.0, size.1);
        }
        if !self.level_logged {
            self.level_logged = true;
            crate::state::ensure_on_screen(window, cx);
            if self.state.always_on_top {
                let level = window_ext::set_always_on_top(window, true);
                eprintln!("[window] restored always_on_top, level = {level:?}");
            }
            eprintln!(
                "[window] initial NSWindow level = {:?}, bounds = {:?}",
                window_ext::level(window),
                window.bounds()
            );
        }

        if !self.focus_set {
            self.focus_set = true;
            window.focus(&self.focus_handle, cx);
        }

        let on_top = self.state.always_on_top;
        let zoom_pct = format!("{:.0}%", self.zoom() * 100.0);
        let toolbar = div()
            .id("toolbar")
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .h(px(TOOLBAR_HEIGHT))
            .w_full()
            .pl(px(TRAFFIC_LIGHT_INSET))
            .pr_3()
            .bg(rgb(0x1b1b20))
            .text_color(rgb(0xe6e6e6))
            // Drag pattern from gpui-component's TitleBar: record the press,
            // start the native move on the first motion so clicks still work,
            // and let AppKit handle double clicks per the system preference.
            .on_mouse_down_out(cx.listener(|this, _, _, _| this.toolbar_dragging = false))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.toolbar_dragging = true),
            )
            // Buttons stop propagation on mouse down, so a press that reaches
            // the bar is a click on blank space: two of them in a row are a
            // double click and follow the system preference.
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, window, _| {
                    if !this.toolbar_dragging {
                        return;
                    }
                    this.toolbar_dragging = false;
                    let now = Instant::now();
                    match this.bar_click_at.take() {
                        Some(previous) if now.duration_since(previous) < Duration::from_millis(400) => {
                            window.titlebar_double_click();
                        }
                        _ => this.bar_click_at = Some(now),
                    }
                }),
            )
            .on_mouse_move(cx.listener(|this, _, window, _| {
                if this.toolbar_dragging {
                    this.toolbar_dragging = false;
                    window.start_window_move();
                }
            }))
            .child(
                Button::new("back".to_string())
                    .label("后退")
                    .on_click(cx.listener(|this, _, _, _| this.go_back())),
            )
            .child(
                Button::new("forward".to_string())
                    .label("前进")
                    .on_click(cx.listener(|this, _, _, _| this.go_forward())),
            )
            .child(
                Button::new("reload".to_string())
                    .label("刷新")
                    .on_click(cx.listener(|this, _, _, _| this.reload())),
            )
            .child(div().w(px(8.0)))
            .child(
                Button::new("zoom_out".to_string())
                    .label("缩小")
                    .on_click(cx.listener(|this, _, _, cx| {
                        let zoom = this.zoom();
                        this.set_zoom(zoom - ZOOM_STEP, cx);
                    })),
            )
            .child(
                Button::new("zoom_reset".to_string())
                    .label(zoom_pct)
                    .on_click(cx.listener(|this, _, _, cx| this.set_zoom(1.0, cx))),
            )
            .child(
                Button::new("zoom_in".to_string())
                    .label("放大")
                    .on_click(cx.listener(|this, _, _, cx| {
                        let zoom = this.zoom();
                        this.set_zoom(zoom + ZOOM_STEP, cx);
                    })),
            )
            .child(div().w(px(8.0)))
            .child(
                Button::new("pin".to_string())
                    .label(if on_top { "取消置顶" } else { "置顶" })
                    .selected(on_top)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_always_on_top(window, cx)
                    })),
            )
            .child(
                Button::new("export".to_string())
                    .label("导出")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.export_conversation(window, cx)
                    })),
            )
            .child(
                Button::new("icon".to_string())
                    .label("换图标")
                    .on_click(cx.listener(|this, _, _, cx| this.cycle_icon(cx))),
            )
            .child(div().flex_1())
            .child(Label::new(self.status()));

        // The WKWebView is an AppKit subview sitting on top of this area.
        // Actions are handled here, at the root, so they do not depend on
        // which child holds focus.
        div()
            .size_full()
            .flex()
            .flex_col()
            .track_focus(&self.focus_handle)
            .bg(rgb(0x101014))
            .on_action(cx.listener(|this, _: &ReloadPage, _, _| this.reload()))
            .on_action(cx.listener(|this, _: &GoBack, _, _| this.go_back()))
            .on_action(cx.listener(|this, _: &GoForward, _, _| this.go_forward()))
            .on_action(cx.listener(|this, _: &ZoomIn, _, cx| {
                let zoom = this.zoom();
                this.set_zoom(zoom + ZOOM_STEP, cx);
            }))
            .on_action(cx.listener(|this, _: &ZoomOut, _, cx| {
                let zoom = this.zoom();
                this.set_zoom(zoom - ZOOM_STEP, cx);
            }))
            .on_action(cx.listener(|this, _: &ZoomReset, _, cx| this.set_zoom(1.0, cx)))
            .on_action(cx.listener(|this, _: &ToggleAlwaysOnTop, window, cx| {
                this.toggle_always_on_top(window, cx)
            }))
            .on_action(cx.listener(|this, _: &ExportConversation, window, cx| {
                this.export_conversation(window, cx)
            }))
            .child(toolbar)
            .child(div().flex_1().w_full())
    }
}
