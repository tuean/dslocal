//! Markdown export: the in-page scrape script, the pure Markdown assembly and
//! the file write. No AppKit / gpui types here so everything but the write is
//! unit-testable.

use std::path::{Path, PathBuf};

pub const TITLE: &str = "# DeepSeek 对话导出";
pub const FALLBACK_URL: &str = "https://chat.deepseek.com";

/// Runs inside chat.deepseek.com and returns a JSON string. The site ships
/// hashed class names that change between releases, so the script tries three
/// strategies and reports which one produced the messages.
pub const SCRAPE_SCRIPT: &str = r#"(async function () {
  'use strict';
  // One macrotask of delay keeps this a real Promise, which is what the
  // evaluateJavaScript wrapper has to unwrap.
  await new Promise(function (res) { setTimeout(res, 0); });
  var MD = '.ds-markdown,[class*="markdown"]';
  var SKIP = '[class*="sidebar"],[class*="history"],[class*="nav"],nav,aside,header,footer,[class*="menu"],[class*="drawer"]';
  var LF = String.fromCharCode(10);
  var CR = String.fromCharCode(13);
  var NBSP = String.fromCharCode(160);
  var FENCE = String.fromCharCode(96, 96, 96);

  function all(sel, root) {
    try { return Array.prototype.slice.call((root || document).querySelectorAll(sel)); }
    catch (e) { return []; }
  }
  function textOf(el) {
    var t = (el && (el.innerText || el.textContent)) || '';
    return t.split(CR + LF).join(LF).split(CR).join(LF).split(NBSP).join(' ').trim();
  }
  function tidy(s) {
    var end = s.length;
    while (end > 0) {
      var ch = s.charAt(end - 1);
      if (ch !== LF && ch !== CR && ch !== ' ' && ch !== String.fromCharCode(9)) break;
      end--;
    }
    return s.slice(0, end);
  }
  // Assistant bodies are rendered HTML, so innerText alone would flatten code
  // blocks. Cloning and swapping every pre for a fenced block keeps them.
  function bodyText(el) {
    var clone = el.cloneNode(true);
    var pres = clone.querySelectorAll('pre');
    for (var i = 0; i < pres.length; i++) {
      var pre = pres[i];
      var code = pre.querySelector('code') || pre;
      var lang = '';
      var m = String((code.getAttribute && code.getAttribute('class')) || '').match(/language-([A-Za-z0-9_+-]+)/);
      if (m) lang = m[1];
      var fenced = LF + FENCE + lang + LF + tidy(String(code.textContent || '')) + LF + FENCE + LF;
      if (pre.parentNode) pre.parentNode.replaceChild(document.createTextNode(fenced), pre);
    }
    return textOf(clone);
  }
  function skipped(el) {
    try { return !!el.closest(SKIP); } catch (e) { return false; }
  }
  function isMd(el) {
    try { return !!(el.matches && el.matches(MD)); } catch (e) { return false; }
  }
  function hasMd(el) {
    try { return !!(el.querySelector && el.querySelector(MD)); } catch (e) { return false; }
  }

  var ORDER = null;
  function docIndex(el) {
    if (!ORDER) {
      ORDER = new Map();
      var nodes = document.body.querySelectorAll('*');
      for (var i = 0; i < nodes.length; i++) ORDER.set(nodes[i], i);
    }
    return ORDER.has(el) ? ORDER.get(el) : -1;
  }
  // The user turn renders right-aligned and narrower than the transcript
  // column; the welcome banner and the suggestion chips do not.
  function alignedRight(el) {
    try {
      var r = el.getBoundingClientRect();
      var p = (el.parentElement || el).getBoundingClientRect();
      if (!r.width || !p.width) return false;
      return r.width <= p.width * 0.88 && (r.left - p.left) > p.width * 0.12;
    } catch (e) { return false; }
  }
  function userish(el) {
    if (alignedRight(el)) return true;
    var kids = el.children || [];
    for (var i = 0; i < kids.length && i < 4; i++) {
      if (alignedRight(kids[i])) return true;
    }
    return false;
  }
  // Only two things count as a speaker: the markdown renderer for the answer,
  // and a right-aligned bubble for the question. Anything else is dropped.
  function roleOf(el) {
    if (isMd(el) || hasMd(el)) return 'assistant';
    if (userish(el)) return 'user';
    return '';
  }
  function finish(strategy, found) {
    var out = [];
    for (var i = 0; i < found.length; i++) {
      var m = found[i];
      if (!m || !m.text) continue;
      var last = out[out.length - 1];
      if (last && last.role === m.role) last.text = last.text + LF + LF + m.text;
      else out.push({ role: m.role, text: m.text });
    }
    return { strategy: strategy, url: String(location.href || ''), messages: out };
  }
  function outermost(nodes) {
    return nodes.filter(function (el) {
      return !nodes.some(function (other) { return other !== el && other.contains(el); });
    });
  }
  // Climb out of single-child wrappers until the parent holds siblings: that
  // node is the row the renderer repeats for every turn.
  function rowOf(el) {
    var node = el;
    for (var i = 0; i < 6; i++) {
      var parent = node.parentElement;
      if (!parent || parent === document.body || parent.children.length > 1) break;
      node = parent;
    }
    return node;
  }

  // Strategy 1: assistant bodies carry the markdown renderer; the user turn is
  // the nearest preceding text row.
  function markdownBlocks() {
    var anchors = outermost(all(MD).filter(function (el) { return !skipped(el) && textOf(el); }));
    if (!anchors.length) return null;
    var found = [];
    anchors.forEach(function (el) {
      var row = rowOf(el);
      var prev = row.previousElementSibling;
      while (prev) {
        var t = textOf(prev);
        if (t && !hasMd(prev)) {
          if (userish(prev)) found.push({ role: 'user', text: t, at: docIndex(prev) });
          break;
        }
        prev = prev.previousElementSibling;
      }
      found.push({ role: 'assistant', text: bodyText(el), at: docIndex(el) });
    });
    found.sort(function (a, b) { return a.at - b.at; });
    var result = finish('markdown-blocks', found);
    return result.messages.length ? result : null;
  }

  // Strategy 2: anything whose class mentions "message"/"msg".
  function messageBlocks() {
    var nodes = outermost(all('[class*="message"],[class*="msg"]').filter(function (el) {
      return !skipped(el) && textOf(el);
    }));
    var found = [];
    nodes.forEach(function (el) {
      var role = roleOf(el);
      if (!role) return;
      found.push({ role: role, text: role === 'assistant' ? bodyText(el) : textOf(el), at: docIndex(el) });
    });
    if (!found.length) return null;
    found.sort(function (a, b) { return a.at - b.at; });
    return finish('message-blocks', found);
  }

  // Strategy 3: no recognisable message nodes, so cut the raw text of the
  // tallest scroll container into blocks and guess the speaker by shape.
  function textFallback() {
    var best = null;
    var bestHeight = 400;
    all('div,main,section').forEach(function (el) {
      if (skipped(el)) return;
      var h = el.scrollHeight || 0;
      if (h > bestHeight) { bestHeight = h; best = el; }
    });
    var raw = textOf(best || document.body);
    var blocks = raw.split(new RegExp(LF + '{2,}')).map(function (s) { return s.trim(); })
      .filter(function (s) { return s.length > 0; });
    // A lone block is the empty-state wording, not a transcript.
    if (blocks.length < 2) return finish('text-fallback', []);
    // Flat text cannot tell the speakers apart, so it is labelled 系统 instead
    // of being guessed as 我 / DeepSeek.
    return finish('text-fallback', [{ role: 'system', text: blocks.join(LF + LF), at: 0 }]);
  }

  return JSON.stringify(markdownBlocks() || messageBlocks() || textFallback());
})()"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
    System,
}

impl Role {
    pub fn heading(self) -> &'static str {
        match self {
            Role::User => "我",
            Role::Assistant => "DeepSeek",
            Role::System => "系统",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Msg {
    pub role: Role,
    pub text: String,
}

#[derive(Debug, Clone, Default)]
pub struct Meta {
    pub source_url: String,
    pub exported_at: String,
    pub strategy: String,
}

/// What the in-page script produced, already parsed.
#[derive(Debug, Clone)]
pub struct Scrape {
    pub strategy: String,
    pub url: String,
    pub messages: Vec<Msg>,
}

#[derive(Debug, Clone)]
pub struct Exported {
    pub path: PathBuf,
    pub file_name: String,
    pub messages: usize,
    pub strategy: String,
}

pub fn to_markdown(msgs: &[Msg], meta: &Meta) -> String {
    let mut out = String::new();
    out.push_str(TITLE);
    out.push_str("\n\n");
    if !meta.source_url.is_empty() {
        out.push_str("- 来源: ");
        out.push_str(&meta.source_url);
        out.push('\n');
    }
    out.push_str("- 导出时间: ");
    out.push_str(&meta.exported_at);
    out.push('\n');
    if !meta.strategy.is_empty() {
        out.push_str("- 抓取策略: ");
        out.push_str(&meta.strategy);
        out.push('\n');
    }
    out.push_str("- 消息数: ");
    out.push_str(&msgs.len().to_string());
    out.push('\n');

    if msgs.is_empty() {
        out.push_str("\n> 空会话：没有在当前页面抓到任何消息");
        if !meta.strategy.is_empty() {
            out.push_str("（strategy=");
            out.push_str(&meta.strategy);
            out.push('）');
        }
        out.push_str("。请先在 chat.deepseek.com 打开一个已有对话再导出。\n");
        return out;
    }

    if meta.strategy == "text-fallback" {
        out.push_str("\n> 注意：没能定位到消息节点，以下内容由页面纯文本粗切，发言人可能与实际不符。\n");
    }

    for msg in msgs {
        out.push_str("\n## ");
        out.push_str(msg.role.heading());
        out.push_str("\n\n");
        out.push_str(msg.text.trim_matches('\n'));
        out.push('\n');
    }
    out
}

/// Parses the JSON the page script returns. Unknown roles are dropped rather
/// than guessed: a mislabelled speaker is worse than a missing line.
pub fn parse_scrape(json: &str) -> Result<Scrape, String> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|err| format!("bad scrape JSON: {err}"))?;
    let strategy = value
        .get("strategy")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let url = value
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let mut messages = Vec::new();
    if let Some(list) = value.get("messages").and_then(|v| v.as_array()) {
        for item in list {
            let role = match item.get("role").and_then(|v| v.as_str()) {
                Some("user") => Role::User,
                Some("assistant") => Role::Assistant,
                Some("system") => Role::System,
                _ => continue,
            };
            let text = item
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if text.is_empty() {
                continue;
            }
            messages.push(Msg { role, text });
        }
    }
    Ok(Scrape {
        strategy,
        url,
        messages,
    })
}

pub fn downloads_dir() -> PathBuf {
    match std::env::var_os("HOME") {
        Some(home) => PathBuf::from(home).join("Downloads"),
        None => PathBuf::from("."),
    }
}

/// Why an export produced nothing. "No conversation" is not an app failure, so
/// it gets its own status wording and never touches the filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportError {
    NoConversation { strategy: String, candidates: usize },
    Other(String),
}

impl ExportError {
    pub fn log_line(&self) -> String {
        match self {
            ExportError::NoConversation {
                strategy,
                candidates,
            } => format!("no conversation on page (strategy={strategy}, candidates={candidates})"),
            ExportError::Other(message) => format!("failed: {message}"),
        }
    }

    pub fn status(&self) -> String {
        match self {
            ExportError::NoConversation { .. } => "当前没有会话可导出".to_string(),
            ExportError::Other(message) => format!("导出失败: {message}"),
        }
    }
}

/// Conversation URLs carry the chat id; the landing page has none.
fn conversation_url(url: &str) -> bool {
    url.contains("/a/chat/") || url.contains("/chat/s/")
}

/// Decides whether a scrape is a conversation at all. The landing page ships a
/// welcome line and two chips, which look exactly like messages once the text
/// fallback has flattened them, so structure has to be proven before writing.
pub fn classify(strategy: &str, url: &str, candidates: &[Msg]) -> Option<Vec<Msg>> {
    if candidates.is_empty() {
        return None;
    }
    if conversation_url(url) {
        return Some(candidates.to_vec());
    }
    let has_assistant = candidates.iter().any(|m| m.role == Role::Assistant);
    // A fresh chat keeps the root URL, so fall back to structure: at least a
    // question and an answer. Raw text never qualifies, it has no roles.
    if strategy != "text-fallback" && candidates.len() >= 2 && has_assistant {
        return Some(candidates.to_vec());
    }
    None
}

pub fn export_json(json: &str, fallback_url: &str, dir: &Path) -> Result<Exported, ExportError> {
    let scrape = parse_scrape(json).map_err(ExportError::Other)?;
    let candidates = scrape.messages.len();
    eprintln!(
        "[export] strategy={} messages={candidates}",
        scrape.strategy
    );
    let Some(messages) = classify(&scrape.strategy, &scrape.url, &scrape.messages) else {
        return Err(ExportError::NoConversation {
            strategy: scrape.strategy,
            candidates,
        });
    };

    let (stamp, pretty) = now_stamps();
    let source_url = if scrape.url.is_empty() {
        fallback_url.to_string()
    } else {
        scrape.url.clone()
    };
    let meta = Meta {
        source_url,
        exported_at: pretty,
        strategy: scrape.strategy.clone(),
    };
    let markdown = to_markdown(&messages, &meta);

    std::fs::create_dir_all(dir)
        .map_err(|err| ExportError::Other(format!("mkdir {}: {err}", dir.display())))?;
    let file_name = format!("dslocal-{stamp}.md");
    let path = dir.join(&file_name);
    std::fs::write(&path, markdown.as_bytes())
        .map_err(|err| ExportError::Other(format!("write {}: {err}", path.display())))?;

    eprintln!("[export] wrote {} messages", messages.len());
    eprintln!("[export] file {}", path.display());
    Ok(Exported {
        path,
        file_name,
        messages: messages.len(),
        strategy: meta.strategy,
    })
}

/// `struct tm` from <time.h>: field order is C layout, do not reorder.
#[allow(dead_code)]
#[repr(C)]
struct Tm {
    tm_sec: i32,
    tm_min: i32,
    tm_hour: i32,
    tm_mday: i32,
    tm_mon: i32,
    tm_year: i32,
    tm_wday: i32,
    tm_yday: i32,
    tm_isdst: i32,
    tm_gmtoff: i64,
    tm_zone: *const i8,
}

unsafe extern "C" {
    fn time(buffer: *mut i64) -> i64;
    fn localtime_r(clock: *const i64, out: *mut Tm) -> *mut Tm;
}

/// ("20261109-115203", "2026-11-09 11:52:03") in the machine's local time.
fn now_stamps() -> (String, String) {
    let mut secs: i64 = 0;
    let mut tm: Tm = unsafe { std::mem::zeroed() };
    // SAFETY: plain C library calls with valid out-pointers.
    unsafe {
        time(&mut secs);
        localtime_r(&secs, &mut tm);
    }
    let (year, month, day) = (tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday);
    let stamp = format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        tm.tm_hour, tm.tm_min, tm.tm_sec
    );
    let pretty = format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
        tm.tm_hour, tm.tm_min, tm.tm_sec
    );
    (stamp, pretty)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> Meta {
        Meta {
            source_url: "https://chat.deepseek.com/a/chat/s/abc".to_string(),
            exported_at: "2026-11-09 11:52:03".to_string(),
            strategy: "markdown-blocks".to_string(),
        }
    }

    fn user(text: &str) -> Msg {
        Msg {
            role: Role::User,
            text: text.to_string(),
        }
    }

    fn assistant(text: &str) -> Msg {
        Msg {
            role: Role::Assistant,
            text: text.to_string(),
        }
    }

    #[test]
    fn landing_page_candidates_are_rejected() {
        let home = vec![
            user("欢迎回来，想从哪里开始？"),
            user("深度思考"),
            user("智能搜索"),
        ];
        assert_eq!(classify("text-fallback", "https://chat.deepseek.com/", &home), None);
        assert_eq!(classify("markdown-blocks", "https://chat.deepseek.com/", &home), None);
        assert_eq!(classify("message-blocks", "https://chat.deepseek.com/", &home), None);
    }

    #[test]
    fn conversation_url_accepts_its_messages() {
        let msgs = vec![user("随便写个快排")];
        assert_eq!(
            classify("markdown-blocks", "https://chat.deepseek.com/a/chat/s/9f2c1", &msgs),
            Some(msgs.clone())
        );
    }

    #[test]
    fn structured_pair_is_accepted_on_a_fresh_chat() {
        let msgs = vec![user("问题"), assistant("回答")];
        assert_eq!(
            classify("message-blocks", "https://chat.deepseek.com/", &msgs),
            Some(msgs.clone())
        );
    }

    #[test]
    fn lone_block_is_never_a_conversation() {
        let one = vec![assistant("欢迎回来，想从哪里开始？")];
        assert_eq!(classify("markdown-blocks", "https://chat.deepseek.com/", &one), None);
        assert_eq!(classify("text-fallback", "https://chat.deepseek.com/", &one), None);
        assert_eq!(classify("markdown-blocks", "https://chat.deepseek.com/", &[]), None);
    }

    #[test]
    fn text_fallback_is_labelled_system_not_user() {
        let json = r#"{"strategy":"text-fallback","url":"https://chat.deepseek.com/a/chat/s/1",
            "messages":[{"role":"system","text":"页面全文"}]}"#;
        let scrape = parse_scrape(json).expect("parse");
        assert_eq!(scrape.messages[0].role, Role::System);
        let fallback_meta = Meta {
            strategy: scrape.strategy.clone(),
            ..meta()
        };
        let md = to_markdown(&scrape.messages, &fallback_meta);
        assert!(md.contains("## 系统"), "{md}");
        assert!(md.contains("纯文本粗切"), "{md}");
        assert!(!md.contains("## 我"), "{md}");
    }

    #[test]
    fn empty_conversation_states_it_explicitly() {
        let md = to_markdown(&[], &meta());
        assert!(md.starts_with(TITLE), "{md}");
        assert!(md.contains("https://chat.deepseek.com/a/chat/s/abc"), "{md}");
        assert!(md.contains("2026-11-09 11:52:03"), "{md}");
        assert!(md.contains("空会话"), "{md}");
        assert!(md.contains("strategy=markdown-blocks"), "{md}");
        assert!(!md.contains("## 我一"), "{md}");
        assert!(!md.contains("## DeepSeek"), "{md}");
    }

    #[test]
    fn single_message_renders_one_section() {
        let msgs = vec![Msg {
            role: Role::User,
            text: "你好".to_string(),
        }];
        let md = to_markdown(&msgs, &meta());
        assert!(md.contains("\n## 我\n\n你好\n"), "{md}");
        assert!(!md.contains("## DeepSeek"), "{md}");
        assert!(md.contains("- 消息数: 1"), "{md}");
    }

    #[test]
    fn multiple_roles_use_their_own_headings() {
        let msgs = vec![
            Msg { role: Role::User, text: "Q".to_string() },
            Msg { role: Role::Assistant, text: "A".to_string() },
            Msg { role: Role::System, text: "S".to_string() },
        ];
        let md = to_markdown(&msgs, &meta());
        assert_eq!(md.matches("## 我\n").count(), 1, "{md}");
        assert_eq!(md.matches("## DeepSeek\n").count(), 1, "{md}");
        assert_eq!(md.matches("## 系统\n").count(), 1, "{md}");
    }

    #[test]
    fn fenced_code_block_survives_verbatim() {
        let code = format!("前:\n```rust\nfn main() {{ println!(\"{{}}\", 1); }}\n```\n后");
        let msgs = vec![Msg {
            role: Role::Assistant,
            text: code.clone(),
        }];
        let md = to_markdown(&msgs, &meta());
        let expected = format!("## DeepSeek\n\n{code}\n");
        assert!(md.contains(&expected), "{md}");
        assert!(!md.contains('\\'), "{md}");
    }

    #[test]
    fn order_is_preserved() {
        let msgs = vec![
            Msg { role: Role::User, text: "first".to_string() },
            Msg { role: Role::Assistant, text: "second".to_string() },
            Msg { role: Role::User, text: "third".to_string() },
            Msg { role: Role::Assistant, text: "fourth".to_string() },
        ];
        let md = to_markdown(&msgs, &meta());
        let positions: Vec<usize> = ["first", "second", "third", "fourth"]
            .iter()
            .map(|needle| md.find(needle).unwrap_or_else(|| panic!("missing {needle}")))
            .collect();
        assert!(positions.windows(2).all(|w| w[0] < w[1]), "{positions:?}");
    }

    #[test]
    fn parse_drops_unknown_roles_and_empty_text() {
        let json = r#"{"strategy":"markdown-blocks","messages":[
            {"role":"user","text":"hi"},
            {"role":"weird","text":"x"},
            {"role":"assistant","text":"   "},
            {"role":"assistant","text":"there"}
        ]}"#;
        let scrape = parse_scrape(json).expect("parse");
        assert_eq!(scrape.strategy, "markdown-blocks");
        assert_eq!(scrape.messages.len(), 2, "{:?}", scrape.messages);
        assert_eq!(scrape.messages[1].text, "there");
    }
}
