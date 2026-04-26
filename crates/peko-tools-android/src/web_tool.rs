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
const DEFAULT_MAX_SEARCH_RESULTS: usize = 8;
const HARD_MAX_BYTES: usize = 8 * 1024 * 1024; // 8 MB; refuse anything larger
const FETCH_TIMEOUT: Duration = Duration::from_secs(15);
const SEARCH_TIMEOUT: Duration = Duration::from_secs(10);

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
        "Read, search, or open a web page without driving the browser by hand. \
         Actions: \
         search (web search via Brave or DuckDuckGo; returns top results with title + snippet + URL — use for \"what is X\" / \"find pages about Y\"), \
         fetch (HTTPS GET → strip HTML → return readable text; pass max_chars to control truncation, default 8000), \
         open_in_browser (am start ACTION_VIEW <url> → lands the device on the page directly). \
         Typical flow: search → pick a result → fetch its url. Only \
         escalate to screenshot+touch when the page requires login or \
         multi-step JS."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["search", "fetch", "open_in_browser"],
                    "description": "search = web search; fetch = HTTP GET + text extract; open_in_browser = am start VIEW"
                },
                "url": {
                    "type": "string",
                    "description": "Absolute URL (http or https). Required for fetch + open_in_browser."
                },
                "query": {
                    "type": "string",
                    "description": "Search query. Required for search."
                },
                "max_chars": {
                    "type": "integer",
                    "description": "Max characters of extracted text to return (fetch only). Default 8000."
                },
                "max_results": {
                    "type": "integer",
                    "description": "Max search results to return. Default 8."
                }
            },
            "required": ["action"]
        })
    }

    fn execute(
        &self,
        args: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + '_>> {
        let action = args["action"].as_str().unwrap_or("").to_string();
        let url = args["url"].as_str().unwrap_or("").to_string();
        let query = args["query"].as_str().unwrap_or("").to_string();
        let max_chars = args["max_chars"]
            .as_u64()
            .map(|n| n as usize)
            .unwrap_or(DEFAULT_MAX_CHARS);
        let max_results = args["max_results"]
            .as_u64()
            .map(|n| n.min(20) as usize)
            .unwrap_or(DEFAULT_MAX_SEARCH_RESULTS);
        Box::pin(async move {
            match action.as_str() {
                "fetch" => fetch(&url, max_chars).await,
                "open_in_browser" => open_in_browser(&url),
                "search" => search(&query, max_results).await,
                "" => Ok(ToolResult::error("missing 'action' parameter".to_string())),
                other => Ok(ToolResult::error(format!(
                    "unknown action '{other}'. valid: search, fetch, open_in_browser"
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

// -----------------------------------------------------------------------------
// Search
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SearchHit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Web search. Picks Brave when `PEKO_BRAVE_API_KEY` env var is set
/// (recommended; brave.com/search/api, free 2k queries/month). Falls
/// back to DuckDuckGo HTML scraping when no key is available — fragile
/// but works without auth.
async fn search(query: &str, max_results: usize) -> anyhow::Result<ToolResult> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(ToolResult::error("missing 'query' parameter".to_string()));
    }
    if q.len() > 512 {
        return Ok(ToolResult::error(
            "query too long (max 512 chars)".to_string(),
        ));
    }

    let brave_key = std::env::var("PEKO_BRAVE_API_KEY").ok().filter(|s| !s.is_empty());
    let result = match brave_key {
        Some(key) => search_brave(q, &key, max_results).await,
        None => search_ddg(q, max_results).await,
    };
    match result {
        Ok((backend, hits)) if hits.is_empty() => Ok(ToolResult::success(format!(
            "Search ({backend}) for '{q}' returned no results."
        ))),
        Ok((backend, hits)) => {
            let mut out = format!("Search ({backend}) — {} results for '{q}':\n\n", hits.len());
            for (i, h) in hits.iter().enumerate() {
                out.push_str(&format!(
                    "{}. {}\n   {}\n   {}\n\n",
                    i + 1,
                    h.title,
                    h.url,
                    if h.snippet.is_empty() { "(no snippet)" } else { &h.snippet }
                ));
            }
            Ok(ToolResult::success(out))
        }
        Err(e) => Ok(ToolResult::error(format!("search failed: {e}"))),
    }
}

async fn search_brave(
    query: &str,
    api_key: &str,
    max_results: usize,
) -> anyhow::Result<(&'static str, Vec<SearchHit>)> {
    let client = reqwest::Client::builder()
        .timeout(SEARCH_TIMEOUT)
        .user_agent(DEFAULT_USER_AGENT)
        .build()?;
    let count = max_results.min(20);
    let resp = client
        .get("https://api.search.brave.com/res/v1/web/search")
        .query(&[("q", query), ("count", &count.to_string())])
        .header("Accept", "application/json")
        .header("X-Subscription-Token", api_key)
        .send()
        .await?;
    let status = resp.status();
    let body = resp.text().await?;
    if !status.is_success() {
        anyhow::bail!("Brave Search HTTP {status}: {}", truncate_chars(&body, 200));
    }
    let v: serde_json::Value = serde_json::from_str(&body)?;
    let hits = parse_brave_hits(&v, max_results);
    Ok(("brave", hits))
}

pub(crate) fn parse_brave_hits(v: &serde_json::Value, max_results: usize) -> Vec<SearchHit> {
    let mut out = Vec::new();
    let Some(results) = v["web"]["results"].as_array() else { return out };
    for r in results.iter().take(max_results) {
        let title = r["title"].as_str().unwrap_or("").to_string();
        let url = r["url"].as_str().unwrap_or("").to_string();
        let snippet = r["description"]
            .as_str()
            .map(|s| html_to_text(s))
            .unwrap_or_default();
        if !title.is_empty() && !url.is_empty() {
            out.push(SearchHit { title, url, snippet });
        }
    }
    out
}

async fn search_ddg(
    query: &str,
    max_results: usize,
) -> anyhow::Result<(&'static str, Vec<SearchHit>)> {
    // DuckDuckGo HTML endpoint — no API key, no JS rendering.
    // Scraping is fragile by definition; treat as a best-effort fallback
    // for users who haven't set PEKO_BRAVE_API_KEY.
    let client = reqwest::Client::builder()
        .timeout(SEARCH_TIMEOUT)
        .user_agent(DEFAULT_USER_AGENT)
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()?;
    let resp = client
        .get("https://html.duckduckgo.com/html/")
        .query(&[("q", query), ("kl", "wt-wt")])
        .send()
        .await?;
    let status = resp.status();
    let body = resp.text().await?;
    if !status.is_success() {
        anyhow::bail!("DDG HTML HTTP {status}");
    }
    let hits = parse_ddg_hits(&body, max_results);
    Ok(("duckduckgo", hits))
}

pub(crate) fn parse_ddg_hits(html: &str, max_results: usize) -> Vec<SearchHit> {
    // The DDG HTML page renders each result inside
    //   <div class="result results_links results_links_deep web-result ">
    //     <h2 class="result__title">
    //       <a class="result__a" href="...">Title</a>
    //     </h2>
    //     <a class="result__snippet" href="...">snippet text</a>
    //   </div>
    //
    // Plus a redirect prefix on hrefs like /l/?uddg=<encoded> we have
    // to strip. The parser is line-oriented + regex-free to avoid the
    // dep tree. Fragile to layout changes; the brave path is preferred.
    let mut out = Vec::new();
    let mut chunks: Vec<&str> = html.split("class=\"result__title\"").collect();
    chunks.remove(0); // drop pre-first chunk
    for chunk in chunks.iter().take(max_results) {
        let url_raw = extract_after(chunk, "class=\"result__a\" href=\"", "\"")
            .unwrap_or_default();
        let url = decode_ddg_redirect(&url_raw);
        let title_html = extract_after(chunk, "\">", "</a>").unwrap_or_default();
        let title = html_to_text(&title_html);
        let snippet_html = extract_after(chunk, "class=\"result__snippet\"", "</a>")
            .unwrap_or_default();
        let snippet_html = match snippet_html.find('>') {
            Some(i) => &snippet_html[i + 1..],
            None => &snippet_html,
        };
        let snippet = html_to_text(snippet_html);
        if !title.is_empty() && !url.is_empty() {
            out.push(SearchHit { title, url, snippet });
        }
    }
    out
}

fn extract_after<'a>(haystack: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let i = haystack.find(start)? + start.len();
    let rest = &haystack[i..];
    let j = rest.find(end)?;
    Some(&rest[..j])
}

fn decode_ddg_redirect(href: &str) -> String {
    // DDG wraps results in /l/?uddg=<percent-encoded-url>&...
    if let Some(start) = href.find("uddg=") {
        let after = &href[start + 5..];
        let end = after.find('&').unwrap_or(after.len());
        return percent_decode(&after[..end]);
    }
    href.to_string()
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = hex_digit(bytes[i + 1]);
            let lo = hex_digit(bytes[i + 2]);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
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
        assert!(actions.contains(&"search"));
    }

    #[test]
    fn parses_brave_search_response() {
        let v: serde_json::Value = serde_json::json!({
            "web": {
                "results": [
                    {
                        "title": "Rust",
                        "url": "https://rust-lang.org",
                        "description": "A language empowering everyone <strong>to build</strong> reliable software."
                    },
                    {
                        "title": "Wikipedia: Rust",
                        "url": "https://en.wikipedia.org/wiki/Rust",
                        "description": "Encyclopedia article."
                    }
                ]
            }
        });
        let hits = parse_brave_hits(&v, 5);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].title, "Rust");
        assert_eq!(hits[0].url, "https://rust-lang.org");
        assert!(hits[0].snippet.contains("to build"));
        assert!(!hits[0].snippet.contains("<strong>"));
    }

    #[test]
    fn brave_parser_returns_empty_when_no_results() {
        let v = serde_json::json!({"web": {"results": []}});
        assert!(parse_brave_hits(&v, 5).is_empty());
        let v = serde_json::json!({"meta": "no web key"});
        assert!(parse_brave_hits(&v, 5).is_empty());
    }

    #[test]
    fn parses_ddg_html_results() {
        let html = r#"
        <div>
          <h2 class="result__title">
            <a class="result__a" href="/l/?uddg=https%3A%2F%2Fexample.com%2Fa&rut=abc">Example A</a>
          </h2>
          <a class="result__snippet" href="/l/?uddg=...">Snippet for A</a>
        </div>
        <div>
          <h2 class="result__title">
            <a class="result__a" href="/l/?uddg=https%3A%2F%2Fexample.com%2Fb">Example B</a>
          </h2>
          <a class="result__snippet" href="/l/?uddg=...">Snippet for B</a>
        </div>
        "#;
        let hits = parse_ddg_hits(html, 5);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].title, "Example A");
        assert_eq!(hits[0].url, "https://example.com/a");
        assert!(hits[0].snippet.contains("Snippet for A"));
        assert_eq!(hits[1].url, "https://example.com/b");
    }

    #[test]
    fn percent_decode_handles_basic_chars() {
        assert_eq!(percent_decode("https%3A%2F%2Fa.com"), "https://a.com");
        assert_eq!(percent_decode("hello%20world"), "hello world");
        // Invalid percent escape passes through.
        assert_eq!(percent_decode("a%zz%2Fb"), "a%zz/b");
    }
}
