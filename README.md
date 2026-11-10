# dslocal - gpui + WKWebView spike (M1)

A native macOS window drawn by gpui (gpui-pre 0.3.9 + gpui-component 0.7.3) with an
AppKit WKWebView embedded below a fixed-height gpui toolbar.

## Run

```sh
cd /Users/zhongxiaotian/RustroverProjects/dslocal
cargo run
```

The window opens at 1280x860, shows the gpui toolbar (label, five buttons, status text)
and loads https://chat.deepseek.com in the WKWebView beneath it.

Optional evidence hook: `DSLOCAL_RESIZE_TEST=1 cargo run` resizes the window to
900x600 twenty seconds after startup, so a screenshot proves the webview follows the
window instead of keeping its creation-time rect.

## Layout

| File | Role |
| --- | --- |
| `src/main.rs` | gpui app: window options, toolbar (`div` + `Label` + `Button`), first-render webview attach |
| `src/webview.rs` | `WebView` wrapper: create from the gpui `Window`, load URL, back/forward/reload, zoom, frame sync |

## Two feature flags this machine needs

There is no Xcode and no metal compiler here, so two non-default features are enabled
through direct dependencies in `Cargo.toml` (feature unification makes the whole graph
build with them):

- `gpui-pre-apple` features `runtime_shaders` - compiles the metal shaders at runtime
  (`new_library_with_source`) instead of shelling out to `xcrun metal`.
- `gpui-pre-apple` + `gpui-pre-macos` features `font-kit` - without BOTH, the macOS
  platform selects `NoopTextSystem` and gpui renders no text at all (quads and paths
  still draw, which makes the symptom look like a shader bug).

## Coordinates

AppKit uses a bottom-left origin (y up), gpui a top-left origin (y down). The webview is
pinned to `y = 0` with `height = parent.height - toolbar_height`, so the fixed top margin
of 44pt is exactly the gpui toolbar strip. `NSViewWidthSizable | NSViewHeightSizable`
lets AppKit keep that up during live resizes.

## Assets

`artifacts/` holds evidence only (screenshots, run logs, `winlist.swift`, `resize.swift`).
No source code depends on it.
