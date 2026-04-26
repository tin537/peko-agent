//! Web tool — direct HTTP fetch + browser-launch helper.
//!
//! The motivating use case: reading a web page. Without this tool, the
//! agent's only option is to drive a browser by screenshot + touch,
//! which on phones is brittle:
//!
//!   - Chrome renders web content as an opaque WebView; uiautomator
//!     dump only sees the URL bar + tab strip, not page content.
//!   - Screenshots are downscaled to 720p before reaching the LLM,
//!     so links shrink to ~7 px and the model misclicks.
//!   - Scrolling requires "screenshot, reason, swipe, repeat" loops
//!     that burn tokens and time.
//!
//! `web fetch` skips all of that: pull the URL with reqwest, strip
//! HTML to readable text, return up to N chars to the LLM. Works for
//! ~95% of "read this article / what does this page say" tasks.
//!
//! For genuinely interactive flows (login, multi-step forms),
//! `web open_in_browser` shells `am start -a VIEW -d <url>`, which
//! lands the user (or the agent's next screenshot tool) directly on
//! the page without typing into the address bar.

use peko_core::tool::{Tool, ToolResult};
use serde_json::json;
use std::future::Future;
use std::pin::Pin;
use std::process::{Command, Stdio};
use std::time::Duration;

const DEFAULT_USER_AGENT: &str =
    "Mozilla/5.0 (Linux; Android 13; OnePlus 6T) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/120.0.0.0 Mobile Safari/537.36";

const DEFAULT_MAX_CHARS: usize = 8000;
const HARD_MAX_BYTES: usize = 8 * 1024 * 1024; // 8 MB; refuse anything larger
const FETCH_TIMEOUT: Duration = Duration::from_secs(15);

pub struct WebTool;

impl WebTool {
    pub fn new() -> Self { Self }
}

impl Default for WebTool {
    fn default() -> Self { Self::new() }
}

impl Tool for WebTool {
    fn name(&self) -> &str { "web" }

    fn description(&self) -> &str {
        "Read or open a web page without driving the browser by hand. \
         Actions: fetch (HTTPS GET → strip HTML → return readable text; \
         pass max_chars to control truncation, default 8000), \
         open_in_browser (am start ACTION_VIEW <url> → lands the device \
         on the page directly, avoids tap-the-address-bar dance). \
         Prefer `fetch` for read-only research; only escalate to \
         interactive browser navigation when the page requires login \
         or multi-step JS."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["fetch", "open_in_browser"],
                    "description": "fetch = HTTP GET + text extract; open_in_browser = am start VIEW"
                },
                "url": {
                    "type": "string",
                    "description": "Absolute URL (http or https)."
                },
                "max_chars": {
                    "type": "integer",
                    "description": "Max characters of extracted text to return (fetch only). Default 8000."
                }
            },
            "required": ["action", "url"]
        })
    }

    fn execute(
        &self,
        args: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + '_>> {
        let action = args["action"].as_str().unwrap_or("").to_string();
        let url = args["url"].as_str().unwrap_or("").to_string();
        let max_chars = args["max_chars"]
            .as_u64()
            .map(|n| n as usize)
            .unwrap_or(DEFAULT_MAX_CHARS);
        Box::pin(async move {
            match action.as_str() {
                "fetch" => fetch(&url, max_chars).await,
                "open_in_browser" => open_in_browser(&url),
                "" => Ok(ToolResult::error("missing 'action' parameter".to_string())),
                other => Ok(ToolResult::error(format!(
                    "unknown action '{other}'. valid: fetch, open_in_browser"
                ))),
            }
        })
    }
}

fn validate_url(url: &str) -> Result<(), String> {
    if url.is_empty() {
        return Err("url is empty".into());
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(format!("only http/https URLs are accepted, got: {url}"));
    }
    if url.len() > 4096 {
        return Err("url exceeds 4096 chars".into());
    }
    Ok(())
}

async fn fetch(url: &str, max_chars: usize) -> anyhow::Result<ToolResult> {
    if let Err(e) = validate_url(url) {
        return Ok(ToolResult::error(e));
    }

    let client = match reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .user_agent(DEFAULT_USER_AGENT)
        .build()
    {
        Ok(c) => c,
        Err(e) => return Ok(ToolResult::error(format!("client build failed: {e}"))),
    };

    let resp = match client.get(url).send().await {
        Ok(r) => r,
        Err(e) => return Ok(ToolResult::error(format!("GET {url} failed: {e}"))),
    };
    let status = resp.status();
    let final_url = resp.url().to_string();
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    if !status.is_success() {
        return Ok(ToolResult::error(format!(
            "GET {final_url} → HTTP {status}"
        )));
    }
    let bytes = match resp.bytes().await {
        Ok(b) => b,
        Err(e) => return Ok(ToolResult::error(format!("body read failed: {e}"))),
    };
    if bytes.len() > HARD_MAX_BYTES {
        return Ok(ToolResult::error(format!(
            "response is {} bytes; refusing to process anything over {} bytes",
            bytes.len(),
            HARD_MAX_BYTES
        )));
    }
    let body = String::from_utf8_lossy(&bytes);

    let extracted = if content_type.contains("text/html")
        || (content_type.is_empty() && body.contains("<html"))
    {
        html_to_text(&body)
    } else if content_type.contains("application/json") {
        body.to_string()
    } else if content_type.starts_with("text/") {
        body.to_string()
    } else {
        return Ok(ToolResult::error(format!(
            "content-type '{}' is not text-like; can't extract content",
            content_type
        )));
    };
    let truncated = truncate_chars(&extracted, max_chars);
    let truncated_note = if truncated.len() < extracted.len() {
        format!(" (truncated; {}/{} chars)", truncated.len(), extracted.len())
    } else {
        String::new()
    };
    Ok(ToolResult::success(format!(
        "GET {final_url} → HTTP {status} ({content_type}){}\n---\n{}",
        truncated_note, truncated
    )))
}

fn open_in_browser(url: &str) -> anyhow::Result<ToolResult> {
    if let Err(e) = validate_url(url) {
        return Ok(ToolResult::error(e));
    }
    // `am start -a android.intent.action.VIEW -d <url>` opens the
    // user's default browser/handler at that URL. Avoids the
    // tap-the-address-bar / type-text / press-enter dance entirely.
    let out = Command::new("am")
        .args(["start", "-a", "android.intent.action.VIEW", "-d", url])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();
    match out {
        Ok(o) if o.status.success() => Ok(ToolResult::success(format!(
            "Opened {url} in default browser via ACTION_VIEW intent."
        ))),
        Ok(o) => Ok(ToolResult::error(format!(
            "am start failed: {}",
            String::from_utf8_lossy(&o.stderr)
        ))),
        Err(e) => Ok(ToolResult::error(format!("am not available: {e}"))),
    }
}

/// Cheap HTML → readable text. Drops `<script>` + `<style>` blocks,
/// strips remaining tags, decodes common entities, collapses
/// whitespace. Not a full HTML parser — but good enough for ~95% of
/// "what does this page say" research tasks. Keeping pure Rust + zero
/// extra deps is intentional: the project's "no bug + easy maintain"
/// rule beats the marginal value of a proper parser here.
fn html_to_text(html: &str) -> String {
    let mut s = strip_block(html, "script");
    s = strip_block(&s, "style");
    s = strip_block(&s, "noscript");
    s = strip_block(&s, "svg");

    // Replace block-level tags with newlines so paragraphs survive.
    let block_tags = [
        "br", "p", "div", "li", "ul", "ol", "h1", "h2", "h3", "h4", "h5", "h6",
        "tr", "section", "article", "header", "footer", "nav", "blockquote", "pre",
    ];
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    let mut in_tag = false;
    let mut tag_buf = String::new();
    while let Some(c) = chars.next() {
        if in_tag {
            if c == '>' {
                in_tag = false;
                let lower = tag_buf.trim_start_matches('/').to_lowercase();
                let name: String = lower
                    .chars()
                    .take_while(|ch| ch.is_ascii_alphanumeric())
                    .collect();
                if block_tags.contains(&name.as_str()) {
                    out.push('\n');
                } else {
                    out.push(' ');
                }
                tag_buf.clear();
            } else {
                tag_buf.push(c);
            }
        } else if c == '<' {
            in_tag = true;
        } else {
            out.push(c);
        }
    }

    let decoded = decode_entities(&out);
    collapse_whitespace(&decoded)
}

fn strip_block(s: &str, tag: &str) -> String {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.to_lowercase().find(&open) {
        out.push_str(&rest[..start]);
        let after_start = &rest[start..];
        if let Some(end) = after_start.to_lowercase().find(&close) {
            rest = &after_start[end + close.len()..];
        } else {
            // No closing tag — drop the rest of the input from `start`
            // onwards. Safer than leaving a dangling <script> open.
            return out;
        }
    }
    out.push_str(rest);
    out
}

fn decode_entities(s: &str) -> String {
    s.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&mdash;", "—")
        .replace("&ndash;", "–")
        .replace("&hellip;", "…")
        .replace("&copy;", "©")
        .replace("&reg;", "®")
}

fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_was_blank = false;
    for line in s.lines() {
        let trimmed = line.trim();
        // Collapse runs of internal spaces.
        let inner: String = {
            let mut buf = String::new();
            let mut last_space = false;
            for c in trimmed.chars() {
                if c.is_whitespace() {
                    if !last_space {
                        buf.push(' ');
                        last_space = true;
                    }
                } else {
                    buf.push(c);
                    last_space = false;
                }
            }
            buf
        };
        if inner.is_empty() {
            if !last_was_blank {
                out.push('\n');
                last_was_blank = true;
            }
        } else {
            out.push_str(&inner);
            out.push('\n');
            last_was_blank = false;
        }
    }
    out.trim().to_string()
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    s.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_urls() {
        assert!(validate_url("https://example.com").is_ok());
        assert!(validate_url("http://example.com/path?q=1").is_ok());
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_url("").is_err());
        assert!(validate_url("ftp://x.y").is_err());
    }

    #[test]
    fn strip_block_removes_scripts() {
        let html = "<html><body>Hello<script>evil()</script>World</body></html>";
        let stripped = strip_block(html, "script");
        assert!(!stripped.contains("evil()"));
        assert!(stripped.contains("Hello"));
        assert!(stripped.contains("World"));
    }

    #[test]
    fn strip_block_handles_unclosed_tag() {
        // Defensive: malformed HTML with no </script> shouldn't leak.
        let html = "before<script>alert(1)";
        let stripped = strip_block(html, "script");
        assert_eq!(stripped, "before");
    }

    #[test]
    fn html_to_text_extracts_readable_content() {
        let html = "<html><head><title>T</title><script>x=1</script></head>\
                    <body><h1>Headline</h1><p>Para one.</p>\
                    <p>Para <a href='/x'>two</a>.</p></body></html>";
        let text = html_to_text(html);
        assert!(text.contains("Headline"));
        assert!(text.contains("Para one"));
        assert!(text.contains("Para"));
        assert!(text.contains("two"));
        assert!(!text.contains("x=1"));
        assert!(!text.contains("<"));
    }

    #[test]
    fn decode_entities_basic() {
        assert_eq!(decode_entities("a&amp;b"), "a&b");
        assert_eq!(decode_entities("&lt;hi&gt;"), "<hi>");
        assert_eq!(decode_entities("don&#39;t"), "don't");
        assert_eq!(decode_entities("a&nbsp;b"), "a b");
    }

    #[test]
    fn collapse_whitespace_dedupes_runs_and_blank_lines() {
        let s = "hello   world\n\n\n\nnext   line";
        let collapsed = collapse_whitespace(s);
        assert_eq!(collapsed, "hello world\n\nnext line");
    }

    #[test]
    fn truncate_respects_char_count_not_byte_count() {
        // Multi-byte chars must not be cut mid-codepoint.
        let s = "日本語のテスト";
        let truncated = truncate_chars(s, 3);
        assert_eq!(truncated, "日本語");
        assert!(truncated.is_char_boundary(truncated.len()));
    }

    #[test]
    fn schema_lists_valid_actions() {
        let schema = WebTool::new().parameters_schema();
        let actions: Vec<&str> = schema["properties"]["action"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(actions.contains(&"fetch"));
        assert!(actions.contains(&"open_in_browser"));
    }
}
