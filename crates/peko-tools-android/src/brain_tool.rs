//! Second-brain / knowledge-graph tool. Wraps `peko_core::BrainStore`
//! so the agent can save markdown notes, search them, follow
//! wikilink graph edges, and surface backlinks.
//!
//! Notes are durable in two places:
//!   - SQLite at `<data_dir>/brain.db` for fast FTS5 + graph queries
//!   - Markdown files at `<data_dir>/brain/<slug>.md` for human reading
//!     and git-versioning the agent's memory
//!
//! Ship-priority: keep this tool's surface small but cover the four
//! verbs that compose into everything else (save / search / get /
//! linked_to). Stats + list_recent are low-frequency but make the
//! agent's introspection of its own knowledge cheap.

use peko_core::tool::{Tool, ToolResult};
use peko_core::BrainStore;
use serde_json::json;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct BrainTool {
    store: Arc<Mutex<BrainStore>>,
}

impl BrainTool {
    pub fn new(store: Arc<Mutex<BrainStore>>) -> Self {
        Self { store }
    }
}

impl Tool for BrainTool {
    fn name(&self) -> &str { "brain" }

    fn description(&self) -> &str {
        "Second-brain / knowledge-graph store. Persistent markdown notes \
         with full-text search, [[wikilink]] graph, and backlinks. Stored \
         on disk at <data_dir>/brain/<slug>.md and indexed in SQLite. \
         \
         Actions: \
         save { title, content, kind?, tags?: [..] } — write a note (upsert by slug), \
         search { query, limit?, expand_links?: bool } — FTS + optional 1-hop graph expansion, \
         get { slug } — fetch a single note by slug or id, \
         linked_to { slug } — backlinks (notes that link TO this slug), \
         linked_from { slug } — outgoing links (notes this slug links TO), \
         list_recent { limit?, kind? } — most-recently-updated, \
         stats — counts of notes, links, dangling links, \
         delete { slug }. \
         \
         Use [[Other Note Title]] in `content` to create graph edges. \
         Pass `kind` to tag notes by purpose: \"note\" (default), \
         \"research\", \"plan\", \"journal\", \"wonder\" (open question)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["save", "search", "get", "linked_to", "linked_from",
                             "list_recent", "stats", "delete"]
                },
                "title": { "type": "string" },
                "content": { "type": "string" },
                "kind": { "type": "string" },
                "tags": { "type": "array", "items": { "type": "string" } },
                "query": { "type": "string" },
                "slug": { "type": "string" },
                "limit": { "type": "integer" },
                "expand_links": { "type": "boolean" },
            },
            "required": ["action"]
        })
    }

    fn execute(
        &self,
        args: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + '_>> {
        let store = self.store.clone();
        Box::pin(async move {
            let action = args["action"].as_str().unwrap_or("").to_string();
            tokio::task::spawn_blocking(move || {
                let store = store.blocking_lock();
                dispatch(&store, &action, args)
            })
            .await
            .map_err(|e| anyhow::anyhow!("brain task panicked: {e}"))?
        })
    }
}

fn dispatch(
    store: &BrainStore,
    action: &str,
    args: serde_json::Value,
) -> anyhow::Result<ToolResult> {
    match action {
        "save" => {
            let Some(title) = args["title"].as_str() else {
                return Ok(ToolResult::error("missing 'title'".to_string()));
            };
            let content = args["content"].as_str().unwrap_or("");
            let kind = args["kind"].as_str();
            let tags: Vec<String> = args["tags"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
            match store.save(title, content, kind, &tags) {
                Ok(n) => Ok(ToolResult::success(format!(
                    "Saved note '{}' (slug: {}, kind: {}, {} tag(s)). Wikilinks parsed.",
                    n.title, n.slug, n.kind, n.tags.len()
                ))),
                Err(e) => Ok(ToolResult::error(format!("save failed: {e}"))),
            }
        }
        "search" => {
            let Some(query) = args["query"].as_str() else {
                return Ok(ToolResult::error("missing 'query'".to_string()));
            };
            let limit = args["limit"].as_u64().unwrap_or(8) as usize;
            let expand = args["expand_links"].as_bool().unwrap_or(false);
            match store.search(query, limit, expand) {
                Ok(notes) if notes.is_empty() => Ok(ToolResult::success(format!(
                    "No notes match '{query}'."
                ))),
                Ok(notes) => Ok(ToolResult::success(format_note_list(
                    &format!("Search '{query}' ({} hits, expand_links={expand}):", notes.len()),
                    &notes,
                ))),
                Err(e) => Ok(ToolResult::error(format!("search failed: {e}"))),
            }
        }
        "get" => {
            let Some(slug) = args["slug"].as_str() else {
                return Ok(ToolResult::error("missing 'slug'".to_string()));
            };
            match store.get(slug) {
                Ok(Some(n)) => Ok(ToolResult::success(format!(
                    "# {}\n\nslug: {}\nkind: {}\ntags: {}\nupdated: {}\n\n{}",
                    n.title, n.slug, n.kind,
                    n.tags.join(", "),
                    n.updated_at,
                    n.content
                ))),
                Ok(None) => Ok(ToolResult::success(format!("No note with slug '{slug}'."))),
                Err(e) => Ok(ToolResult::error(format!("get failed: {e}"))),
            }
        }
        "linked_to" => {
            let Some(slug) = args["slug"].as_str() else {
                return Ok(ToolResult::error("missing 'slug'".to_string()));
            };
            match store.linked_to(slug) {
                Ok(notes) => Ok(ToolResult::success(format_note_list(
                    &format!("Backlinks to '{slug}' ({} notes):", notes.len()),
                    &notes,
                ))),
                Err(e) => Ok(ToolResult::error(format!("linked_to failed: {e}"))),
            }
        }
        "linked_from" => {
            let Some(slug) = args["slug"].as_str() else {
                return Ok(ToolResult::error("missing 'slug'".to_string()));
            };
            match store.linked_from(slug) {
                Ok(notes) => Ok(ToolResult::success(format_note_list(
                    &format!("Outgoing links from '{slug}' ({} notes):", notes.len()),
                    &notes,
                ))),
                Err(e) => Ok(ToolResult::error(format!("linked_from failed: {e}"))),
            }
        }
        "list_recent" => {
            let limit = args["limit"].as_u64().unwrap_or(20) as usize;
            let kind = args["kind"].as_str();
            match store.list_recent(limit, kind) {
                Ok(notes) => Ok(ToolResult::success(format_note_list(
                    &format!(
                        "Recent notes ({} found{}):",
                        notes.len(),
                        kind.map(|k| format!(", kind={k}")).unwrap_or_default()
                    ),
                    &notes,
                ))),
                Err(e) => Ok(ToolResult::error(format!("list_recent failed: {e}"))),
            }
        }
        "stats" => match store.stats() {
            Ok(s) => Ok(ToolResult::success(format!(
                "Brain stats:\n  notes: {}\n  links: {}\n  dangling links: {}",
                s.note_count, s.link_count, s.dangling_links
            ))),
            Err(e) => Ok(ToolResult::error(format!("stats failed: {e}"))),
        },
        "delete" => {
            let Some(slug) = args["slug"].as_str() else {
                return Ok(ToolResult::error("missing 'slug'".to_string()));
            };
            match store.delete(slug) {
                Ok(true) => Ok(ToolResult::success(format!("Deleted '{slug}'."))),
                Ok(false) => Ok(ToolResult::success(format!("No note '{slug}' to delete."))),
                Err(e) => Ok(ToolResult::error(format!("delete failed: {e}"))),
            }
        }
        "" => Ok(ToolResult::error("missing 'action' parameter".to_string())),
        other => Ok(ToolResult::error(format!(
            "unknown action '{other}'. valid: save, search, get, linked_to, linked_from, list_recent, stats, delete"
        ))),
    }
}

fn format_note_list(header: &str, notes: &[peko_core::Note]) -> String {
    let mut out = String::from(header);
    out.push('\n');
    for n in notes {
        let preview: String = n.content.chars().take(120).collect();
        out.push_str(&format!(
            "\n  • [{}] {} — {} ({})\n    {}\n",
            n.kind,
            n.title,
            n.slug,
            n.updated_at.split('T').next().unwrap_or(&n.updated_at),
            preview.replace('\n', " ")
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_lists_actions() {
        let s = BrainTool::new(Arc::new(Mutex::new(BrainStore::open_in_memory().unwrap())))
            .parameters_schema();
        let actions: Vec<&str> = s["properties"]["action"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        for a in [
            "save", "search", "get", "linked_to", "linked_from",
            "list_recent", "stats", "delete",
        ] {
            assert!(actions.contains(&a), "missing {a}");
        }
    }
}
