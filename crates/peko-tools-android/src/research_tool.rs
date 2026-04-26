//! Research pipeline. End-to-end:
//!
//!   1. Web search the topic (uses `web_tool::search` internals).
//!   2. Fetch the top N results, strip HTML to readable text.
//!   3. Summarise each source into a brain note (kind: "source").
//!   4. Synthesise a topic-level overview that [[wikilinks]] every
//!      source note, plus relevant pre-existing brain notes (so new
//!      research stitches into prior knowledge).
//!   5. Save the overview as a brain note (kind: "research").
//!
//! Three trigger sources:
//!
//!   - User message ("research X for me") → agent calls
//!     `research do { topic: "X" }` directly, gets a saved
//!     brain note slug back.
//!   - Curiosity drive (autonomy) → the life loop already proposes
//!     "explore" actions; with a research backbone in place those
//!     proposals can target real topics drawn from kind="wonder"
//!     notes in the brain.
//!   - Schedule cron (config [[schedule]]) → user can wire a
//!     recurring research like "every Monday 8am, research what
//!     changed in agent-as-OS news this week" by setting
//!     `task = "research weekly: agent-as-OS landscape"`.
//!
//! Synthesis + summary use the same LLM provider the agent does —
//! we don't open a second client. The research tool builds prompts
//! and posts them to a localhost web API endpoint exposed by main.rs;
//! when that endpoint is unreachable (test environment, agent not
//! running web UI) the tool falls back to returning per-source raw
//! extracts so the calling agent can synthesise inline.

use peko_core::tool::{Tool, ToolResult};
use peko_core::BrainStore;
use serde_json::json;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

use crate::web_tool;

const DEFAULT_MAX_SOURCES: usize = 5;
const MAX_SOURCE_BYTES: usize = 6000; // chars per source after HTML strip
const SUMMARIZE_BUDGET: Duration = Duration::from_secs(60);

pub struct ResearchTool {
    brain: Arc<Mutex<BrainStore>>,
    /// HTTP endpoint for in-process synthesis. Points at the agent's
    /// own web UI (`http://127.0.0.1:8080/api/llm/synth` typically).
    /// `None` falls back to "no synthesis" mode where the tool returns
    /// raw extracts and the agent does its own summarisation.
    synth_endpoint: Option<String>,
}

impl ResearchTool {
    pub fn new(brain: Arc<Mutex<BrainStore>>, synth_endpoint: Option<String>) -> Self {
        Self { brain, synth_endpoint }
    }
}

impl Tool for ResearchTool {
    fn name(&self) -> &str { "research" }

    fn description(&self) -> &str {
        "Run a research pipeline on a topic. Searches the web, fetches \
         the top results, distills each into a source note, synthesises \
         a topic-level overview, and saves everything as cross-linked \
         markdown into the second brain. Use for \"research X\" / \
         \"what's the latest on Y\" tasks where you want durable, \
         linkable findings rather than a one-shot answer. \
         \
         Action: do { topic: string, max_sources?: int (default 5), \
         kind?: string (default 'research') }. \
         \
         Returns the slug of the saved overview note. Subsequent \
         queries via `brain search` or `brain linked_from <slug>` will \
         surface the sources."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["do"] },
                "topic": { "type": "string" },
                "max_sources": { "type": "integer" },
                "kind": { "type": "string" },
            },
            "required": ["action", "topic"]
        })
    }

    fn execute(
        &self,
        args: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + '_>> {
        let brain = self.brain.clone();
        let synth_endpoint = self.synth_endpoint.clone();
        Box::pin(async move {
            let action = args["action"].as_str().unwrap_or("");
            if action != "do" {
                return Ok(ToolResult::error(format!(
                    "unknown action '{action}'. valid: do"
                )));
            }
            let Some(topic) = args["topic"].as_str().map(String::from) else {
                return Ok(ToolResult::error("missing 'topic'".to_string()));
            };
            if topic.trim().is_empty() {
                return Ok(ToolResult::error("'topic' is empty".to_string()));
            }
            let max_sources = args["max_sources"]
                .as_u64()
                .map(|n| (n as usize).clamp(1, 12))
                .unwrap_or(DEFAULT_MAX_SOURCES);
            let kind = args["kind"].as_str().unwrap_or("research").to_string();

            run_research(brain, synth_endpoint, &topic, max_sources, &kind).await
        })
    }
}

async fn run_research(
    brain: Arc<Mutex<BrainStore>>,
    synth_endpoint: Option<String>,
    topic: &str,
    max_sources: usize,
    kind: &str,
) -> anyhow::Result<ToolResult> {
    // Step 1: search.
    let search_hits = match web_tool::search_internal(topic, max_sources).await {
        Ok(hits) if !hits.is_empty() => hits,
        Ok(_) => {
            return Ok(ToolResult::error(format!(
                "research '{topic}': no search hits returned"
            )));
        }
        Err(e) => return Ok(ToolResult::error(format!("search failed: {e}"))),
    };
    tracing::info!(topic, hits = search_hits.len(), "research: search complete");

    // Step 2 + 3: fetch + per-source notes.
    let mut source_slugs: Vec<String> = Vec::new();
    let mut source_summaries: Vec<String> = Vec::new();
    for (i, hit) in search_hits.iter().enumerate() {
        let extract = match web_tool::fetch_extract_internal(&hit.url, MAX_SOURCE_BYTES).await {
            Ok(text) => text,
            Err(e) => {
                tracing::warn!(url = %hit.url, error = %e, "fetch failed, skipping");
                continue;
            }
        };
        if extract.trim().is_empty() {
            continue;
        }
        let snippet: String = extract.chars().take(800).collect();
        let title = format!("{}", hit.title);
        let body = format!(
            "Source {} of {} for [[{topic}]].\n\nURL: {}\n\n{}\n\n---\n*Fetched extract (first ~800 chars):*\n\n{}",
            i + 1, search_hits.len(), hit.url, hit.snippet, snippet
        );
        let saved = {
            let store = brain.lock().await;
            store.save(&title, &body, Some("source"), &["research-source".to_string()])
        };
        match saved {
            Ok(n) => {
                source_slugs.push(n.slug.clone());
                source_summaries.push(format!(
                    "- [[{}]] — {} ({})\n  {}",
                    n.title, hit.url, n.slug, snippet.lines().next().unwrap_or("")
                ));
            }
            Err(e) => tracing::warn!(error = %e, "brain save failed for source"),
        }
    }
    if source_slugs.is_empty() {
        return Ok(ToolResult::error(format!(
            "research '{topic}': no sources successfully fetched + saved"
        )));
    }

    // Step 4: cross-link to existing related brain notes (FTS5 ranked).
    let related: Vec<String> = {
        let store = brain.lock().await;
        store
            .search(topic, 5, false)
            .map(|notes| {
                notes
                    .into_iter()
                    .filter(|n| !source_slugs.contains(&n.slug))
                    .map(|n| format!("- [[{}]] — kind={} ({})", n.title, n.kind, n.slug))
                    .collect()
            })
            .unwrap_or_default()
    };

    // Step 5: synthesise the overview. Try the agent's own LLM via the
    // localhost synth endpoint; on any failure, fall back to a
    // deterministic structured note that the agent can read or
    // post-process itself.
    let synthesis = match synth_endpoint.as_ref() {
        Some(ep) => synthesise(ep, topic, &source_summaries).await.ok(),
        None => None,
    };

    let mut content = format!(
        "# Research: {topic}\n\n*Generated {} via the research pipeline. \
         {} source(s) fetched + saved.*\n\n",
        chrono::Utc::now().format("%Y-%m-%d %H:%M UTC"),
        source_slugs.len()
    );
    if let Some(s) = synthesis {
        content.push_str("## Synthesis\n\n");
        content.push_str(&s);
        content.push_str("\n\n");
    } else {
        content.push_str("## Sources\n\n*(synthesis unavailable; raw source extracts below — agent can summarise downstream)*\n\n");
    }
    content.push_str("## Source notes\n\n");
    for s in &source_summaries {
        content.push_str(s);
        content.push('\n');
    }
    if !related.is_empty() {
        content.push_str("\n## Related prior notes\n\n");
        for r in &related {
            content.push_str(r);
            content.push('\n');
        }
    }

    // The overview note itself.
    let overview_title = format!("Research: {topic}");
    let saved = {
        let store = brain.lock().await;
        store.save(
            &overview_title,
            &content,
            Some(kind),
            &["research".to_string()],
        )?
    };

    Ok(ToolResult::success(format!(
        "Research saved as [[{}]] (slug: {}, {} sources).\n\n{}",
        saved.title,
        saved.slug,
        source_slugs.len(),
        content.chars().take(600).collect::<String>()
    )))
}

/// POST a synth request to the agent's own web API. The endpoint
/// receives `{topic, sources: [..]}` and returns `{summary}`. When
/// missing or failing we fall back to no-synthesis mode.
async fn synthesise(
    endpoint: &str,
    topic: &str,
    source_summaries: &[String],
) -> anyhow::Result<String> {
    let client = reqwest::Client::builder()
        .timeout(SUMMARIZE_BUDGET)
        .build()?;
    let body = json!({
        "topic": topic,
        "sources": source_summaries,
    });
    let resp = client.post(endpoint).json(&body).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("synth endpoint returned HTTP {}", resp.status());
    }
    let v: serde_json::Value = resp.json().await?;
    let s = v["summary"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("synth response missing 'summary'"))?;
    Ok(s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_lists_action_do() {
        let store = Arc::new(Mutex::new(BrainStore::open_in_memory().unwrap()));
        let s = ResearchTool::new(store, None).parameters_schema();
        let actions: Vec<&str> = s["properties"]["action"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(actions.contains(&"do"));
    }
}
