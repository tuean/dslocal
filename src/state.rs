//! Window geometry + page zoom persisted to
//! ~/Library/Application Support/dslocal/state.json.
//!
//! Restored bounds are clamped into the visible frame of a display that is
//! currently attached, so unplugging the monitor a window was saved on cannot
//! strand it off screen.

use std::path::PathBuf;

use gpui::{App, Bounds, Pixels, Window, WindowBounds, point, px, size};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub window: Option<Rect>,
    pub zoom: f64,
    pub always_on_top: bool,
    /// Icon variant picked with the toolbar switch button ("a"/"b"/"c").
    #[serde(default = "default_icon")]
    pub icon: String,
}

fn default_icon() -> String {
    "a".to_string()
}

impl Default for State {
    fn default() -> Self {
        Self {
            window: None,
            zoom: 1.0,
            always_on_top: false,
            icon: default_icon(),
        }
    }
}

impl State {
    pub fn path() -> Option<PathBuf> {
        let home = std::env::var_os("HOME")?;
        Some(PathBuf::from(home).join("Library/Application Support/dslocal/state.json"))
    }

    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            eprintln!("[state] {} not found, using defaults", path.display());
            return Self::default();
        };
        match serde_json::from_str::<State>(&text) {
            Ok(state) => {
                eprintln!("[state] loaded {}", path.display());
                state
            }
            Err(err) => {
                eprintln!("[state] unreadable {}: {err}", path.display());
                Self::default()
            }
        }
    }

    pub fn save(&self) {
        let Some(path) = Self::path() else {
            return;
        };
        if let Some(dir) = path.parent()
            && let Err(err) = std::fs::create_dir_all(dir)
        {
            eprintln!("[state] create_dir_all {} failed: {err}", dir.display());
            return;
        }
        match serde_json::to_string_pretty(self) {
            Ok(json) => match std::fs::write(&path, json + "\n") {
                Ok(()) => eprintln!("[state] saved {}", path.display()),
                Err(err) => eprintln!("[state] write {} failed: {err}", path.display()),
            },
            Err(err) => eprintln!("[state] serialize failed: {err}"),
        }
    }

    /// Records the window's current frame. Window::bounds() reads the live
    /// NSWindow frame in gpui's top-left origin global coordinates.
    pub fn record_window(&mut self, window: &Window) {
        let bounds = window.bounds();
        self.window = Some(Rect {
            x: f32::from(bounds.origin.x),
            y: f32::from(bounds.origin.y),
            width: f32::from(bounds.size.width),
            height: f32::from(bounds.size.height),
        });
    }

    pub fn restored_bounds(&self, cx: &App) -> WindowBounds {
        match self.window {
            Some(rect) => WindowBounds::Windowed(clamp_to_visible_displays(rect, cx)),
            None => WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(1280.0), px(860.0)),
                cx,
            )),
        }
    }
}

/// Keeps the window on a screen that exists right now: pick the display that
/// contains the saved origin (else the first one) and clamp origin + size into
/// its visible frame.
fn clamp_to_visible_displays(rect: Rect, cx: &App) -> Bounds<Pixels> {
    let displays: Vec<Bounds<Pixels>> = cx
        .displays()
        .iter()
        .map(|display| display.visible_bounds())
        .collect();
    // App::displays() is empty until the first window exists, so the caller
    // rescues the origin again on the first render (ensure_on_screen).
    eprintln!("[state] {} display(s) visible at restore time", displays.len());
    clamp_rect(rect, &displays)
}

/// Re-clamps the live window once displays are known, moving it back with
/// AppKit when a saved position belongs to a display that is gone.
pub fn ensure_on_screen(window: &Window, cx: &App) {
    let displays: Vec<Bounds<Pixels>> = cx
        .displays()
        .iter()
        .map(|display| display.visible_bounds())
        .collect();
    if displays.is_empty() {
        eprintln!("[state] no displays reported, skipping on-screen check");
        return;
    }

    let current = window.bounds();
    let rect = Rect {
        x: f32::from(current.origin.x),
        y: f32::from(current.origin.y),
        width: f32::from(current.size.width),
        height: f32::from(current.size.height),
    };
    let target = clamp_rect(rect, &displays);
    let dx = f32::from(target.origin.x) - rect.x;
    let dy = f32::from(target.origin.y) - rect.y;
    if dx == 0.0 && dy == 0.0 {
        return;
    }
    if crate::window_ext::nudge_window(window, dx, dy) {
        eprintln!(
            "[state] moved window ({}, {}) -> ({}, {}) to stay on a visible display",
            rect.x,
            rect.y,
            f32::from(target.origin.x),
            f32::from(target.origin.y)
        );
    }
}

fn clamp_rect(rect: Rect, displays: &[Bounds<Pixels>]) -> Bounds<Pixels> {
    const MIN_WIDTH: f32 = 720.0;
    const MIN_HEIGHT: f32 = 480.0;

    let origin = point(px(rect.x), px(rect.y));
    let Some(screen) = displays
        .iter()
        .find(|bounds| bounds.contains(&origin))
        .or_else(|| displays.first())
        .copied()
    else {
        return Bounds::new(origin, size(px(rect.width.max(MIN_WIDTH)), px(rect.height.max(MIN_HEIGHT))));
    };

    let screen_x = f32::from(screen.origin.x);
    let screen_y = f32::from(screen.origin.y);
    let screen_w = f32::from(screen.size.width);
    let screen_h = f32::from(screen.size.height);

    let width = rect.width.max(MIN_WIDTH).min(screen_w);
    let height = rect.height.max(MIN_HEIGHT).min(screen_h);
    let x = rect.x.clamp(screen_x, (screen_x + screen_w - width).max(screen_x));
    let y = rect.y.clamp(screen_y, (screen_y + screen_h - height).max(screen_y));

    if x != rect.x || y != rect.y {
        eprintln!("[state] clamped window origin ({}, {}) -> ({x}, {y})", rect.x, rect.y);
    }
    Bounds::new(point(px(x), px(y)), size(px(width), px(height)))
}
#[cfg(test)]
mod tests {
    use super::*;

    fn display(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(w), px(h)))
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect {
            x,
            y,
            width: w,
            height: h,
        }
    }

    #[test]
    fn origin_outside_every_display_is_pulled_onto_the_first() {
        let displays = [display(0.0, 25.0, 1920.0, 1055.0)];
        let out = clamp_rect(rect(5000.0, 4000.0, 1280.0, 860.0), &displays);
        assert_eq!(f32::from(out.origin.x), 640.0);
        assert_eq!(f32::from(out.origin.y), 220.0);
        assert_eq!(f32::from(out.size.width), 1280.0);
        assert_eq!(f32::from(out.size.height), 860.0);
    }

    #[test]
    fn origin_inside_a_secondary_display_is_kept() {
        let displays = [
            display(0.0, 25.0, 1920.0, 1055.0),
            display(1920.0, 0.0, 3440.0, 1415.0),
        ];
        let out = clamp_rect(rect(2000.0, 100.0, 1280.0, 860.0), &displays);
        assert_eq!(f32::from(out.origin.x), 2000.0);
        assert_eq!(f32::from(out.origin.y), 100.0);
    }

    #[test]
    fn oversize_window_is_shrunk_to_the_visible_frame() {
        let displays = [display(0.0, 25.0, 1280.0, 720.0)];
        let out = clamp_rect(rect(0.0, 0.0, 2000.0, 1000.0), &displays);
        assert_eq!(f32::from(out.size.width), 1280.0);
        assert_eq!(f32::from(out.size.height), 720.0);
        assert_eq!(f32::from(out.origin.y), 25.0);
    }

    #[test]
    fn tiny_window_is_grown_to_the_minimum() {
        let displays = [display(0.0, 25.0, 1920.0, 1055.0)];
        let out = clamp_rect(rect(100.0, 100.0, 400.0, 300.0), &displays);
        assert_eq!(f32::from(out.size.width), 720.0);
        assert_eq!(f32::from(out.size.height), 480.0);
        assert_eq!(f32::from(out.origin.x), 100.0);
    }

    #[test]
    fn no_display_keeps_the_saved_rect() {
        let out = clamp_rect(rect(5000.0, 4000.0, 1280.0, 860.0), &[]);
        assert_eq!(f32::from(out.origin.x), 5000.0);
        assert_eq!(f32::from(out.origin.y), 4000.0);
    }

    #[test]
    fn state_round_trips_through_json() {
        let text = r#"{"window":{"x":100.0,"y":200.0,"width":1280.0,"height":860.0},"zoom":1.2,"always_on_top":true}"#;
        let state: State = serde_json::from_str(text).unwrap();
        assert_eq!(state.zoom, 1.2);
        assert!(state.always_on_top);
        assert_eq!(state.window, Some(rect(100.0, 200.0, 1280.0, 860.0)));
        let back = serde_json::to_string(&state).unwrap();
        assert_eq!(serde_json::from_str::<State>(&back).unwrap(), state);
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let state: State = serde_json::from_str("{}").unwrap();
        assert_eq!(state, State::default());
        // A state file written before the icon switch existed still loads.
        assert_eq!(state.icon, "a");
    }

    #[test]
    fn icon_variant_survives_a_round_trip() {
        let mut state = State::default();
        state.icon = "c".to_string();
        let text = serde_json::to_string(&state).unwrap();
        assert_eq!(serde_json::from_str::<State>(&text).unwrap().icon, "c");
    }
}
