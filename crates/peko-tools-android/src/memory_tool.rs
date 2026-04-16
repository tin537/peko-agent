use peko_core::tool::{Tool, ToolResult};
use peko_core::memory::{MemoryStore, MemoryCategory};
use serde_json::json;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct MemoryTool {
    store: Arc<Mutex<MemoryStore>>,
}

impl MemoryTool {
    pub fn new(store: Arc<Mutex<MemoryStore>>) -> Self {
        Self { store }
    }
}

impl Tool for MemoryTool {
    fn name(&self) -> &str { "memory" }

    fn description(&self) -> &str {
        "Manage persistent memory across sessions. Use this to remember important facts, \
         user preferences, procedures you've learned, and observations about the device. \
         Memories persist across conversations and help you provide better assistance.\n\n\
         Actions:\n\
         - save: Store a memory (key, content, category, importance 0.0-1.0)\n\
         - search: Find relevant memories by query\n\
         - list: Show all memories (optional category filter)\n\
         - delete: Remove a memory by key or id\n\n\
         Categories: fact, preference, procedure, observation, skill\n\n\
         Guidelines:\n\
         - Save user preferences when they express them\n\
         - Save procedures for tasks you complete successfully\n\
         - Save facts about the device configuration\n\
         - Use importance 0.8-1.0 for critical info, 0.3-0.5 for nice-to-know"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["save", "search", "list", "delete"],
                    "description": "Memory operation to perform"
                },
                "key": {
                    "type": "string",
                    "description": "Short label for the memory (for save/delete)"
                },
                "content": {
                    "type": "string",
                    "description": "The memory content to store (for save)"
                },
                "category": {
                    "type": "string",
                    "enum": ["fact", "preference", "procedure", "observation", "skill"],
                    "description": "Memory category (for save, default: fact)"
                },
                "importance": {
                    "type": "number",
                    "description": "Importance 0.0-1.0 (for save, default: 0.5)"
                },
                "query": {
                    "type": "string",
                    "description": "Search query (for search)"
                }
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
            let action = args["action"].as_str()
                .ok_or_else(|| anyhow::anyhow!("missing 'action'"))?;

            let s = store.lock().await;

            match action {
                "save" => {
                    let key = args["key"].as_str()
                        .ok_or_else(|| anyhow::anyhow!("missing 'key' for save"))?;
                    let content = args["content"].as_str()
                        .ok_or_else(|| anyhow::anyhow!("missing 'content' for save"))?;
                    let category = args["category"].as_str()
                        .map(MemoryCategory::from_str)
                        .unwrap_or(MemoryCategory::Fact);
                    let importance = args["importance"].as_f64().unwrap_or(0.5);

                    let id = s.save(key, content, &category, importance, None)?;
                    let count = s.count()?;

                    Ok(ToolResult::success(format!(
                        "Memory saved: \"{}\" [{}] (importance: {:.1}, total memories: {})",
                        key, category, importance, count
                    )))
                }

                "search" => {
                    let query = args["query"].as_str()
                        .ok_or_else(|| anyhow::anyhow!("missing 'query' for search"))?;

                    let results = s.search(query, 10)?;

                    if results.is_empty() {
                        Ok(ToolResult::success(format!("No memories found for '{}'", query)))
                    } else {
                        let mut output = format!("Found {} memories:\n\n", results.len());
                        for (i, mem) in results.iter().enumerate() {
                            output.push_str(&format!(
                                "{}. [{}] **{}** (importance: {:.1}, accessed: {}x)\n   {}\n\n",
                                i + 1, mem.category, mem.key, mem.importance,
                                mem.access_count, mem.content
                            ));
                        }
                        Ok(ToolResult::success(output))
                    }
                }

                "list" => {
                    let category = args["category"].as_str();
                    let memories = s.list(50, category)?;
                    let count = s.count()?;

                    if memories.is_empty() {
                        Ok(ToolResult::success("No memories stored yet.".to_string()))
                    } else {
                        let mut output = format!("{} memories total", count);
                        if let Some(cat) = category {
                            output.push_str(&format!(" (showing: {})", cat));
                        }
                        output.push_str(":\n\n");
                        for mem in &memories {
                            output.push_str(&format!(
                                "- [{}] **{}**: {} (importance: {:.1})\n",
                                mem.category, mem.key, mem.content, mem.importance
                            ));
                        }
                        Ok(ToolResult::success(output))
                    }
                }

                "delete" => {
                    let key = args["key"].as_str()
                        .ok_or_else(|| anyhow::anyhow!("missing 'key' for delete"))?;

                    if s.delete(key)? {
                        Ok(ToolResult::success(format!("Memory '{}' deleted", key)))
                    } else {
                        Ok(ToolResult::error(format!("Memory '{}' not found", key)))
                    }
                }

                _ => Ok(ToolResult::error(format!("Unknown action: {}", action))),
            }
        })
    }
}
