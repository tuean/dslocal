# DeepSeek Local (dslocal)

DeepSeek 网页版 (chat.deepseek.com) 的本地桌面外壳，不逆向接口、不碰 token。

- **macOS**：gpui (gpui-pre 0.3.9 + gpui-component 0.7.3) 画的 44pt 工具条 + AppKit WKWebView。
- **Windows**：WebView2 外壳（`windows/`），同一个网页、原生窗口，无 gpui 工具条。

## 下载

最新安装包：<https://github.com/tuean/dslocal/releases/latest>

| 平台 | 文件 | 说明 |
| --- | --- | --- |
| macOS 13+ | `DSLocal-<version>-macos-universal.dmg`（或 `.zip`） | 通用二进制，Apple Silicon + Intel 通用；拖进「应用程序」 |
| Windows 10/11 x64 | `DSLocal-<version>-windows-x64-setup.exe`（或 `.zip` 免安装） | NSIS 安装包，装到 `%LOCALAPPDATA%\Programs\DeepSeek Local` |

两个包都没买签名证书：

- macOS 首次打开被 Gatekeeper 拦住时，右键 App → 打开，或
  `xattr -dr com.apple.quarantine /Applications/DSLocal.app`。
- Windows SmartScreen 提示时点「更多信息 → 仍要运行」；需要 WebView2 运行时
  （Win10 1803+ / Win11 通常已自带，否则装 Microsoft 官方的 Evergreen Runtime）。

## 构建安装包

```sh
# macOS：arm64 + x86_64 分别编译后 lipo 成通用包，输出 dist/*.dmg 与 dist/*.zip
./scripts/package-macos.sh --universal

# Windows（在 Windows 上，需要 rustup 目标 x86_64-pc-windows-msvc 和 NSIS）
pwsh -File windows/package.ps1 -Version 0.1.0
```

打 tag 推送即触发 `.github/workflows/release.yml`，两个平台并行构建并把产物挂到
GitHub Release：

```sh
git tag v0.1.0 && git push origin v0.1.0
```

## Run (macOS, from source)

```sh
cd /Users/zhongxiaotian/RustroverProjects/dslocal
cargo run
```

The window opens at 1280x860, shows the gpui toolbar (navigation, tabs, zoom, pin,
export, icon, status text) and loads https://chat.deepseek.com in the WKWebView
beneath it.

## Tabs

The toolbar switches between two live pages:

| Tab | Page | Source |
| --- | --- | --- |
| 对话 | https://chat.deepseek.com (`DSLOCAL_URL` overrides) | DeepSeek chat, the app's main page |
| 搜索 | http://127.0.0.1:8787 (`DSLOCAL_SEARCH_URL` overrides) | the local tdsearch service (project `~/RustroverProjects/tdsearch`) |

Both webviews stay in the view tree; the inactive one is hidden with `setHidden:`, so a
chat draft and a result list both survive a switch. The search page is created on the
first switch, so a launch that never opens it costs no extra WebKit process.

`cmd-1` / `cmd-2` select 对话 / 搜索. Export (`cmd-s`, 导出) only works on the chat tab,
since the scrapers only understand the transcript.

Optional evidence hooks: `DSLOCAL_TAB=search cargo run` starts on the search tab,
`DSLOCAL_TAB_TEST=1` sends `cmd-2` then `cmd-1` through the real keymap and logs which
page is visible and what title it reports, `DSLOCAL_PROBE_JS=<script>` evaluates a
script in the active page.

Optional evidence hook: `DSLOCAL_RESIZE_TEST=1 cargo run` resizes the window to
900x600 twenty seconds after startup, so a screenshot proves the webview follows the
window instead of keeping its creation-time rect.

## Global summon key

`option+space` is **not** registered by default: it belongs to uTools on this
machine, and a Carbon hot key (`RegisterEventHotKey`) consumes the chord before
uTools or any other app ever sees it. Summon the window by clicking the Dock
icon (or `open -a DSLocal`) instead.

To bind one, launch with `DSLOCAL_HOTKEY` set, e.g.
`DSLOCAL_HOTKEY="control+option+space"`; modifiers are `option`/`command`/`shift`/
`control` and the key is `space`, `tab`, `escape`, a letter or a digit. An unset
or unparsable value means no global hot key.

## Layout

| File | Role |
| --- | --- |
| `src/main.rs` | gpui app: window options, toolbar (`div` + `Label` + `Button`), first-render webview attach |
| `src/webview.rs` | `WebView` wrapper: create from the gpui `Window`, load URL, back/forward/reload, zoom, frame sync |
| `windows/src/main.rs` | Windows shell: one tao window hosting a WebView2 pointed at chat.deepseek.com |
| `windows/installer.nsi` | NSIS installer (per-user, Start Menu + Desktop shortcuts, uninstaller) |
| `scripts/bundle.sh` | `.app` assembly; `DSLOCAL_TARGET` / `DSLOCAL_APP_DIR` let CI cross-build |
| `scripts/package-macos.sh` | universal build → `.dmg` + `.zip` |

## Windows shell

`windows/` is a separate crate (not a workspace member), built on Windows only:

```sh
cd windows && cargo build --release     # needs the WebView2 evergreen runtime
```

| 快捷键 | 动作 |
| --- | --- |
| `Ctrl+R` / `F5` | 刷新 |
| `Alt+←` / `Alt+→` | 后退 / 前进 |
| `Ctrl+=` / `Ctrl+-` / `Ctrl+0` | 放大 / 缩小 / 复位 |
| `Ctrl+Shift+I` / `F12` | WebView2 开发者工具 |

WebView2 keeps the keyboard focus inside the page, so the window never receives
`KeyboardInput`; the chords are installed as a document-start script and routed to the
host through `window.ipc.postMessage`. Only `*.deepseek.com` stays inside the window —
every other link opens in the default browser.

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
