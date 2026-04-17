//! Reflector — Phase A.
//!
//! After every completed task, a Reflector instance evaluates what happened
//! and writes a structured `Reflection` to the memory store. This is the
//! "automatic evaluator" that completes Tier 3's self-reflection criterion
//! (see docs/architecture/Digital-Life.md).
//!
//! Runs asynchronously — the user gets their response first; reflection
//! happens in the background.

use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, warn};

use peko_transport::LlmProvider;
use peko_transport::provider::{Message as TransportMessage, MessageContent};

use crate::memory::{MemoryStore, MemoryCategory};
use crate::message::Message;

/// Summary verdict for a completed task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReflectionOutcome {
    WentWell,
    Partial,
    Failed,
}

impl ReflectionOutcome {
    pub fn importance(&self) -> f64 {
        match self {
            Self::Failed   => 0.8,
            Self::Partial  => 0.5,
            Self::WentWell => 0.3,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Reflection {
    pub task_id:     String,
    pub user_input:  String,
    pub outcome:     ReflectionOutcome,
    pub what_worked: String,
    pub what_failed: String,
    pub lessons:     Vec<String>,
    pub tool_errors: Vec<String>,
    pub timestamp:   String,
}

/// Input to the reflector — everything known about a just-completed task.
pub struct CompletedTask {
    pub session_id:    String,
    pub user_input:    String,
    pub conversation:  Vec<Message>,
    pub iterations:    usize,
}

/// Reflector wraps an LLM provider (cheap side preferred) + the memory store.
pub struct Reflector {
    provider: Arc<dyn LlmProvider>,
    memory:   Arc<Mutex<MemoryStore>>,
}

impl Reflector {
    pub fn new(provider: Arc<dyn LlmProvider>, memory: Arc<Mutex<MemoryStore>>) -> Self {
        Self { provider, memory }
    }

    /// Reflect on a task and persist the result as a memory of category
    /// `Reflection`. Returns the Reflection for observability.
    pub async fn reflect(&self, task: &CompletedTask) -> anyhow::Result<Reflection> {
        let prompt = build_reflect_prompt(task);

        // Call LLM to produce structured JSON reflection
        let system = "You are a self-evaluation critic. Respond ONLY with the \
            requested JSON object — no prose, no markdown, no preamble.";

        let mut stream = self.provider
            .stream_completion(
                system,
                &[TransportMessage {
                    role: "user".to_string(),
                    content: MessageContent::Text(prompt),
                }],
                &[],
            )
            .await?;

        // Accumulate the full response text
        use futures::StreamExt;
        use peko_transport::StreamEvent;
        let mut full = String::new();
        while let Some(event) = stream.next().await {
            if let Ok(StreamEvent::TextDelta(t)) = event {
                full.push_str(&t);
            }
        }

        let reflection = parse_reflection(&full, task)?;

        // Persist into memory store
        let key = format!("reflection:{}", reflection.task_id);
        let content = serde_json::to_string_pretty(&reflection)?;
        {
            let store = self.memory.lock().await;
            store.save(
                &key,
                &content,
                &MemoryCategory::Reflection,
                reflection.outcome.importance(),
                Some(&reflection.task_id),
            )?;
        }
        info!(
            task = %reflection.task_id,
            outcome = ?reflection.outcome,
            lessons = reflection.lessons.len(),
            "reflection persisted"
        );

        Ok(reflection)
    }
}

/// Build the user-side prompt describing the task that just ran.
fn build_reflect_prompt(task: &CompletedTask) -> String {
    let mut out = String::with_capacity(1024);
    out.push_str("Evaluate this just-completed agent task. Return JSON only.\n\n");
    out.push_str(&format!("User asked: {}\n\n", task.user_input));
    out.push_str(&format!("Agent ran {} iterations.\n\n", task.iterations));

    // Summarize the conversation — last 20 messages
    out.push_str("Conversation transcript:\n");
    let last: Vec<&Message> = task.conversation.iter().rev().take(20).collect::<Vec<_>>().into_iter().rev().collect();
    for msg in last {
        match msg {
            Message::User(t) => {
                out.push_str("user: ");
                out.push_str(&truncate(t, 300));
                out.push('\n');
            }
            Message::Assistant { text, tool_calls } => {
                if let Some(t) = text {
                    out.push_str("assistant: ");
                    out.push_str(&truncate(t, 300));
                    out.push('\n');
                }
                for tc in tool_calls {
                    out.push_str(&format!("  → tool {} ({})\n", tc.name,
                        truncate(&tc.input.to_string(), 120)));
                }
            }
            Message::ToolResult { name, content, is_error, .. } => {
                let marker = if *is_error { "ERROR" } else { "ok" };
                out.push_str(&format!("  ← {} [{}]: {}\n", name, marker, truncate(content, 200)));
            }
            Message::System(_) => {}
        }
    }

    out.push_str(concat!(
        "\nEvaluate and return:\n",
        "{\n",
        "  \"outcome\":     \"went_well\" | \"partial\" | \"failed\",\n",
        "  \"what_worked\": \"<short sentence>\",\n",
        "  \"what_failed\": \"<short sentence>\",\n",
        "  \"lessons\":     [\"<short lesson>\", ...]\n",
        "}\n",
    ));

    out
}

/// Extract the first JSON object from the LLM output; tolerate surrounding text.
fn parse_reflection(llm_output: &str, task: &CompletedTask) -> anyhow::Result<Reflection> {
    let json_str = extract_first_json_object(llm_output)
        .ok_or_else(|| anyhow::anyhow!("no JSON object in reflector output: {}", truncate(llm_output, 200)))?;
    let raw: serde_json::Value = serde_json::from_str(&json_str)
        .map_err(|e| anyhow::anyhow!("reflection parse failed: {}: {}", e, truncate(&json_str, 200)))?;

    let outcome = match raw["outcome"].as_str().unwrap_or("partial") {
        "went_well" => ReflectionOutcome::WentWell,
        "failed"    => ReflectionOutcome::Failed,
        _           => ReflectionOutcome::Partial,
    };

    let lessons: Vec<String> = raw["lessons"].as_array()
        .map(|arr| arr.iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect())
        .unwrap_or_default();

    let tool_errors: Vec<String> = collect_tool_errors(&task.conversation);

    Ok(Reflection {
        task_id:     task.session_id.clone(),
        user_input:  task.user_input.clone(),
        outcome,
        what_worked: raw["what_worked"].as_str().unwrap_or("").to_string(),
        what_failed: raw["what_failed"].as_str().unwrap_or("").to_string(),
        lessons,
        tool_errors,
        timestamp:   chrono::Utc::now().to_rfc3339(),
    })
}

fn collect_tool_errors(conv: &[Message]) -> Vec<String> {
    conv.iter().filter_map(|m| match m {
        Message::ToolResult { name, is_error, .. } if *is_error => Some(name.clone()),
        _ => None,
    }).collect()
}

fn extract_first_json_object(s: &str) -> Option<String> {
    let start = s.find('{')?;
    let mut depth = 0;
    let mut in_str = false;
    let mut escape = false;
    for (i, ch) in s.char_indices().skip(start) {
        if escape { escape = false; continue; }
        if ch == '\\' && in_str { escape = true; continue; }
        if ch == '"' { in_str = !in_str; continue; }
        if in_str { continue; }
        if ch == '{' { depth += 1; }
        if ch == '}' {
            depth -= 1;
            if depth == 0 { return Some(s[start..=i].to_string()); }
        }
    }
    None
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n { s.to_string() } else {
        // Find char boundary near n
        let mut end = n;
        while end > 0 && !s.is_char_boundary(end) { end -= 1; }
        format!("{}…", &s[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_json_simple() {
        let s = "prefix {\"a\": 1} suffix";
        assert_eq!(extract_first_json_object(s).as_deref(), Some("{\"a\": 1}"));
    }

    #[test]
    fn extract_json_nested() {
        let s = "{\"a\": {\"b\": 2}}";
        assert_eq!(extract_first_json_object(s).as_deref(), Some(s));
    }

    #[test]
    fn extract_json_with_string_braces() {
        // Make sure { inside strings doesn't throw off brace counting
        let s = r#"{"a": "hello {world}", "b": 1}"#;
        assert_eq!(extract_first_json_object(s).as_deref(), Some(s));
    }

    #[test]
    fn outcome_importance_ordering() {
        assert!(ReflectionOutcome::Failed.importance() > ReflectionOutcome::Partial.importance());
        assert!(ReflectionOutcome::Partial.importance() > ReflectionOutcome::WentWell.importance());
    }

    #[test]
    fn parse_reflection_full() {
        let task = CompletedTask {
            session_id: "sid123".into(),
            user_input: "open youtube".into(),
            conversation: vec![],
            iterations: 3,
        };
        let llm = r#"Sure! Here's my evaluation:
        {
          "outcome": "went_well",
          "what_worked": "Tapped the correct icon",
          "what_failed": "",
          "lessons": ["coordinates for youtube icon work"]
        }"#;
        let r = parse_reflection(llm, &task).unwrap();
        assert_eq!(r.outcome, ReflectionOutcome::WentWell);
        assert_eq!(r.lessons.len(), 1);
        assert_eq!(r.task_id, "sid123");
    }
}
