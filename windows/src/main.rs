//! DeepSeek chat in a native Windows window (WebView2).
//!
//! The macOS app in the repo root is gpui + WKWebView and cannot be built for
//! Windows; this is the Windows shell: one tao window that hosts a WebView2
//! pointed at chat.deepseek.com. It is built on Windows CI only.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::{Icon, WindowBuilder};
use wry::{NewWindowResponse, WebViewBuilder};

const HOME: &str = "https://chat.deepseek.com";
const MIN_ZOOM: f64 = 0.5;
const MAX_ZOOM: f64 = 3.0;
const ZOOM_STEP: f64 = 0.1;

/// Commands the page sends back through `window.ipc.postMessage`.
#[derive(Clone, Debug)]
enum Cmd {
    Reload,
    Back,
    Forward,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    DevTools,
}

/// Keyboard shortcuts, installed as a document-start script.
///
/// WebView2 keeps the keyboard focus inside the page, so the host window never
/// sees `KeyboardInput`; routing the chords through IPC is the reliable path.
const SHORTCUTS_JS: &str = r#"
(() => {
  if (window.__dslocalShortcuts) return;
  window.__dslocalShortcuts = true;
  const send = (m) => { try { window.ipc.postMessage(m); } catch (_) {} };
  window.addEventListener('keydown', (e) => {
    const mod = e.ctrlKey || e.metaKey;
    const key = (e.key || '').toLowerCase();
    if (mod && (key === '=' || key === '+')) { e.preventDefault(); send('zoom-in'); }
    else if (mod && key === '-') { e.preventDefault(); send('zoom-out'); }
    else if (mod && key === '0') { e.preventDefault(); send('zoom-reset'); }
    else if (mod && key === 'r') { e.preventDefault(); send('reload'); }
    else if (mod && e.shiftKey && key === 'i') { e.preventDefault(); send('devtools'); }
    else if (e.altKey && e.key === 'ArrowLeft') { e.preventDefault(); send('back'); }
    else if (e.altKey && e.key === 'ArrowRight') { e.preventDefault(); send('forward'); }
    else if (e.key === 'F5') { e.preventDefault(); send('reload'); }
    else if (e.key === 'F12') { e.preventDefault(); send('devtools'); }
  }, true);
})();
"#;

fn main() -> wry::Result<()> {
    let event_loop = EventLoopBuilder::<Cmd>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let icon = load_icon();
    let window = WindowBuilder::new()
        .with_title("DeepSeek Local")
        .with_window_icon(icon)
        .with_inner_size(LogicalSize::new(1280.0, 860.0))
        .with_min_inner_size(LogicalSize::new(720.0, 480.0))
        .build(&event_loop)
        .expect("failed to create the window");

    let webview = WebViewBuilder::new()
        .with_url(HOME)
        .with_devtools(true)
        .with_initialization_script(SHORTCUTS_JS)
        .with_ipc_handler(move |request: wry::http::Request<String>| {
            let cmd = match request.body().as_str() {
                "reload" => Cmd::Reload,
                "back" => Cmd::Back,
                "forward" => Cmd::Forward,
                "zoom-in" => Cmd::ZoomIn,
                "zoom-out" => Cmd::ZoomOut,
                "zoom-reset" => Cmd::ZoomReset,
                "devtools" => Cmd::DevTools,
                _ => return,
            };
            let _ = proxy.send_event(cmd);
        })
        // Only DeepSeek's own pages stay in the app; anything else is a link
        // the user meant to read in their browser.
        .with_navigation_handler(|url| {
            if is_internal(&url) {
                true
            } else {
                if is_web(&url) {
                    open_external(&url);
                }
                false
            }
        })
        .with_new_window_req_handler(|url, _features| {
            if is_web(&url) {
                open_external(&url);
            }
            NewWindowResponse::Deny
        })
        .build(&window)?;

    let mut zoom = 1.0f64;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => *control_flow = ControlFlow::Exit,
            Event::LoopDestroyed => *control_flow = ControlFlow::Exit,
            Event::UserEvent(cmd) => match cmd {
                Cmd::Reload => {
                    let _ = webview.reload();
                }
                Cmd::Back => {
                    let _ = webview.evaluate_script("history.back()");
                }
                Cmd::Forward => {
                    let _ = webview.evaluate_script("history.forward()");
                }
                Cmd::ZoomIn => {
                    zoom = (zoom + ZOOM_STEP).min(MAX_ZOOM);
                    let _ = webview.zoom(zoom);
                }
                Cmd::ZoomOut => {
                    zoom = (zoom - ZOOM_STEP).max(MIN_ZOOM);
                    let _ = webview.zoom(zoom);
                }
                Cmd::ZoomReset => {
                    zoom = 1.0;
                    let _ = webview.zoom(zoom);
                }
                Cmd::DevTools => webview.open_devtools(),
            },
            _ => {}
        }
    })
}

/// True for URLs that should stay inside the window.
fn is_internal(url: &str) -> bool {
    match scheme(url) {
        Some("about") | Some("data") | Some("blob") => true,
        Some("http") | Some("https") => match host(url) {
            Some(h) => h == "deepseek.com" || h.ends_with(".deepseek.com"),
            None => false,
        },
        // Everything else that is not a normal web page is left to the engine.
        _ => true,
    }
}

fn is_web(url: &str) -> bool {
    matches!(scheme(url), Some("http") | Some("https"))
}

fn scheme(url: &str) -> Option<&str> {
    url.split_once("://")
        .map(|(s, _)| s)
        .filter(|s| !s.is_empty() && !s.contains(['/', '?', '#']))
}

fn host(url: &str) -> Option<&str> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit('@').next()?;
    let host = authority.split(':').next()?;
    if host.is_empty() {
        None
    } else {
        Some(host)
    }
}

fn open_external(url: &str) {
    #[cfg(windows)]
    let spawned = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn();
    #[cfg(target_os = "macos")]
    let spawned = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let spawned = std::process::Command::new("xdg-open").arg(url).spawn();

    if let Err(err) = spawned {
        eprintln!("[dslocal] could not open {url} externally: {err}");
    }
}

fn load_icon() -> Option<Icon> {
    let data = include_bytes!("../assets/icon.png");
    let decoder = png::Decoder::new(&data[..]);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        eprintln!(
            "[dslocal] icon.png must be 8-bit RGBA (got {:?}/{:?})",
            info.color_type, info.bit_depth
        );
        return None;
    }
    let rgba = buf[..info.buffer_size()].to_vec();
    match Icon::from_rgba(rgba, info.width, info.height) {
        Ok(icon) => Some(icon),
        Err(err) => {
            eprintln!("[dslocal] icon rejected: {err}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deepseek_stays_inside() {
        assert!(is_internal("https://chat.deepseek.com/"));
        assert!(is_internal("https://account.deepseek.com/sign_in"));
        assert!(is_internal("https://deepseek.com"));
        assert!(is_internal("about:blank"));
        assert!(is_internal("data:text/html,hi"));
    }

    #[test]
    fn third_party_goes_to_the_browser() {
        assert!(!is_internal("https://github.com/tuean/dslocal"));
        assert!(!is_internal("https://evil-deepseek.com/"));
        assert!(!is_internal("https://deepseek.com.evil.test/"));
        assert!(!is_internal("http://127.0.0.1:8787/"));
    }

    #[test]
    fn host_and_scheme_parsing() {
        assert_eq!(host("https://user:pw@chat.deepseek.com:443/x?y#z"), Some("chat.deepseek.com"));
        assert_eq!(host("https://chat.deepseek.com"), Some("chat.deepseek.com"));
        assert_eq!(host("file:///tmp/x"), None);
        assert_eq!(scheme("https://a.b/c"), Some("https"));
        assert_eq!(scheme("file:///tmp/x"), Some("file"));
    }
}
