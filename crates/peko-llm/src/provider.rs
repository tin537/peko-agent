use std::sync::Arc;
use tokio::sync::Mutex;
use async_trait::async_trait;
use futures::stream::{BoxStream, StreamExt};
use tracing::error;

use peko_transport::provider::{LlmProvider, Message, MessageContent, ContentBlock};
use peko_transport::{StreamEvent, StopReason};

use crate::engine::LlmEngine;

/// LlmProvider that runs inference in-process via the embedded engine.
/// No HTTP, no serialization — tokens flow directly from llama.cpp to the runtime.
pub struct EmbeddedProvider {
    engine: Arc<Mutex<LlmEngine>>,
    model_name: String,
    max_context: usize,
}

impl EmbeddedProvider {
    pub fn new(engine: Arc<Mutex<LlmEngine>>) -> Self {
        let (name, ctx) = {
            // We can't lock async here, so store config at creation time
            // The engine config is set at construction and doesn't change
            ("embedded".to_string(), 4096)
        };
        Self {
            engine,
            model_name: name,
            max_context: ctx,
        }
    }

    pub fn with_model_name(mut self, name: String) -> Self {
        self.model_name = name;
        self
    }

    pub fn with_max_context(mut self, ctx: usize) -> Self {
        self.max_context = ctx;
        self
    }
}

/// Build a chat prompt string from the conversation messages + tools.
/// Formats as a simple chat template that works with most instruction-tuned models.
fn build_chat_prompt(
    system_prompt: &str,
    messages: &[Message],
    tools: &[serde_json::Value],
) -> String {
    let mut prompt = String::with_capacity(system_prompt.len() * 2);

    // Qwen / ChatML template: <|im_start|>role\n...<|im_end|>\n
    // This format is understood by Qwen 2/2.5/3, Yi, ChatGLM, Hermes, and many others.
    prompt.push_str("<|im_start|>system\n");
    prompt.push_str(system_prompt);

    if !tools.is_empty() {
        prompt.push_str("\n\nTools (respond with ```tool_call\\n{\"name\":..,\"arguments\":..}\\n```):\n");
        for tool in tools {
            if let Some(name) = tool["name"].as_str() {
                let desc_line = tool["description"].as_str()
                    .map(|d| d.split('.').next().unwrap_or(d).trim())
                    .unwrap_or("");
                let args = compact_args(tool.get("input_schema"));
                prompt.push_str(&format!("- {}({}): {}\n", name, args, desc_line));
            }
        }
    }
    prompt.push_str("<|im_end|>\n");

    // Conversation messages
    for msg in messages {
        match (&msg.role as &str, &msg.content) {
            ("user", MessageContent::Text(text)) => {
                prompt.push_str("<|im_start|>user\n");
                prompt.push_str(text);
                prompt.push_str("<|im_end|>\n");
            }
            ("user", MessageContent::Blocks(blocks)) => {
                prompt.push_str("<|im_start|>user\n");
                for block in blocks {
                    match block {
                        ContentBlock::Text { text } => prompt.push_str(text),
                        ContentBlock::ToolResult { content, is_error, .. } => {
                            if *is_error {
                                prompt.push_str(&format!("[Tool error: {}]\n", content));
                            } else {
                                prompt.push_str(&format!("[Tool result: {}]\n", content));
                            }
                        }
                        ContentBlock::Image { .. } => {
                            prompt.push_str("[Image attached]\n");
                        }
                        _ => {}
                    }
                }
                prompt.push_str("<|im_end|>\n");
            }
            ("assistant", MessageContent::Text(text)) => {
                prompt.push_str("<|im_start|>assistant\n");
                prompt.push_str(text);
                prompt.push_str("<|im_end|>\n");
            }
            ("assistant", MessageContent::Blocks(blocks)) => {
                prompt.push_str("<|im_start|>assistant\n");
                for block in blocks {
                    match block {
                        ContentBlock::Text { text } => prompt.push_str(text),
                        ContentBlock::ToolUse { name, input, .. } => {
                            prompt.push_str(&format!(
                                "\n```tool_call\n{}\n```\n",
                                serde_json::json!({"name": name, "arguments": input})
                            ));
                        }
                        _ => {}
                    }
                }
                prompt.push_str("<|im_end|>\n");
            }
            _ => {}
        }
    }

    // Prompt for assistant response
    prompt.push_str("<|im_start|>assistant\n");
    prompt
}

/// Compact one-line representation of a tool's input schema.
/// Turns `{type: object, properties: {x: {type: integer, description: ...}, ...}}`
/// into `x:int, y:int, action:str` — much smaller than full JSON schema.
fn compact_args(schema: Option<&serde_json::Value>) -> String {
    let Some(schema) = schema else { return String::new(); };
    let Some(props) = schema.get("properties").and_then(|p| p.as_object()) else {
        return String::new();
    };
    let required: std::collections::HashSet<String> = schema.get("required")
        .and_then(|r| r.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();

    let mut args = Vec::with_capacity(props.len());
    for (name, info) in props {
        let ty = info.get("type").and_then(|t| t.as_str()).unwrap_or("any");
        let short_ty = match ty {
            "integer" => "int",
            "number" => "num",
            "string" => "str",
            "boolean" => "bool",
            "array" => "arr",
            "object" => "obj",
            other => other,
        };
        let marker = if required.contains(name) { "" } else { "?" };
        args.push(format!("{}{}:{}", name, marker, short_ty));
    }
    args.join(",")
}

/// Parse tool calls from model output text.
/// Looks for ```tool_call\n{...}\n``` blocks.
fn parse_tool_calls(text: &str) -> Vec<(String, serde_json::Value)> {
    let mut calls = Vec::new();
    let marker = "```tool_call";
    let mut search = text;

    while let Some(start) = search.find(marker) {
        let after_marker = &search[start + marker.len()..];
        // Skip optional newline after marker
        let json_start = after_marker.find('\n').map(|i| i + 1).unwrap_or(0);
        let json_text = &after_marker[json_start..];

        if let Some(end) = json_text.find("```") {
            let block = json_text[..end].trim();
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(block) {
                let name = parsed["name"].as_str().unwrap_or("").to_string();
                let args = parsed["arguments"].clone();
                if !name.is_empty() {
                    calls.push((name, args));
                }
            }
            search = &json_text[end..];
        } else {
            break;
        }
    }

    calls
}

#[async_trait]
impl LlmProvider for EmbeddedProvider {
    async fn stream_completion(
        &self,
        system_prompt: &str,
        messages: &[Message],
        tools: &[serde_json::Value],
    ) -> anyhow::Result<BoxStream<'static, anyhow::Result<StreamEvent>>> {
        let prompt = build_chat_prompt(system_prompt, messages, tools);
        let engine = self.engine.clone();

        // Spawn blocking inference on a dedicated thread
        let (tx, rx) = tokio::sync::mpsc::channel::<StreamEvent>(128);

        tokio::task::spawn_blocking(move || {
            let engine_guard = engine.blocking_lock();
            let backend = engine_guard.backend();
            let max_tokens = engine_guard.config().max_tokens;

            let mut full_text = String::new();
            let mut tool_call_buffer = String::new();
            let mut in_tool_call = false;

            // Send message start
            let _ = tx.blocking_send(StreamEvent::MessageStart {
                id: uuid::Uuid::new_v4().to_string(),
                input_tokens: backend.token_count(&prompt),
            });

            // Generate with streaming
            let result = backend.generate(
                &prompt,
                max_tokens,
                &["<|im_end|>", "<|im_start|>", "<|endoftext|>"],
                &mut |token: &str| {
                    full_text.push_str(token);

                    // Detect tool call blocks
                    if full_text.contains("```tool_call") && !in_tool_call {
                        in_tool_call = true;
                    }

                    if in_tool_call {
                        // Buffer tool call tokens, don't stream as text
                        tool_call_buffer.push_str(token);

                        // Check if tool call block is complete
                        if tool_call_buffer.contains("```tool_call") {
                            let after_start = tool_call_buffer.find("```tool_call").unwrap();
                            let rest = &tool_call_buffer[after_start..];
                            // Count closing backticks (need the second ```)
                            let ticks: Vec<_> = rest.match_indices("```").collect();
                            if ticks.len() >= 2 {
                                // Tool call complete — parse and emit
                                let calls = parse_tool_calls(&full_text);
                                for (i, (name, args)) in calls.iter().enumerate() {
                                    let id = format!("emb_{}", uuid::Uuid::new_v4().to_string().split('-').next().unwrap_or("0"));
                                    let _ = tx.blocking_send(StreamEvent::ToolUseStart {
                                        id: id.clone(),
                                        name: name.clone(),
                                    });
                                    let _ = tx.blocking_send(StreamEvent::ToolInputDelta(
                                        serde_json::to_string(&args).unwrap_or_default()
                                    ));
                                    let _ = tx.blocking_send(StreamEvent::ContentBlockStop { index: i + 1 });
                                }
                                in_tool_call = false;
                                tool_call_buffer.clear();
                            }
                        }
                    } else {
                        // Stream text delta
                        let _ = tx.blocking_send(StreamEvent::TextDelta(token.to_string()));
                    }

                    true // continue generating
                },
            );

            // Determine stop reason
            let stop_reason = if !parse_tool_calls(&full_text).is_empty() {
                StopReason::ToolUse
            } else {
                StopReason::EndTurn
            };

            let _ = tx.blocking_send(StreamEvent::MessageDelta {
                output_tokens: backend.token_count(&full_text),
                stop_reason,
            });
            let _ = tx.blocking_send(StreamEvent::MessageStop);

            if let Err(e) = result {
                error!(error = %e, "embedded LLM inference failed");
            }
        });

        // Convert mpsc receiver to a stream
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx)
            .map(|event| Ok(event));

        Ok(Box::pin(stream))
    }

    fn model_name(&self) -> &str {
        &self.model_name
    }

    fn max_context_tokens(&self) -> usize {
        self.max_context
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_tool_calls_single() {
        let text = r#"I'll open YouTube for you.
```tool_call
{"name": "shell", "arguments": {"command": "am start com.google.android.youtube"}}
```
"#;
        let calls = parse_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "shell");
        assert!(calls[0].1["command"].as_str().unwrap().contains("youtube"));
    }

    #[test]
    fn test_parse_tool_calls_multiple() {
        let text = r#"Let me take a screenshot first, then tap.
```tool_call
{"name": "screenshot", "arguments": {}}
```
Now I'll tap:
```tool_call
{"name": "touch", "arguments": {"action": "tap", "x": 100, "y": 200}}
```
"#;
        let calls = parse_tool_calls(text);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "screenshot");
        assert_eq!(calls[1].0, "touch");
    }

    #[test]
    fn test_parse_tool_calls_none() {
        let text = "Just a normal response without any tool calls.";
        let calls = parse_tool_calls(text);
        assert_eq!(calls.len(), 0);
    }

    #[test]
    fn test_build_chat_prompt() {
        let system = "You are Peko, an AI agent.";
        let messages = vec![
            Message {
                role: "user".to_string(),
                content: MessageContent::Text("open youtube".to_string()),
            },
        ];
        let tools = vec![serde_json::json!({
            "name": "shell",
            "description": "Run a shell command",
            "input_schema": {"type": "object", "properties": {"command": {"type": "string"}}}
        })];

        let prompt = build_chat_prompt(system, &messages, &tools);

        // ChatML template (Qwen / Hermes / many modern models)
        assert!(prompt.contains("<|im_start|>system"));
        assert!(prompt.contains("Peko"));
        assert!(prompt.contains("shell"));
        assert!(prompt.contains("<|im_start|>user"));
        assert!(prompt.contains("open youtube"));
        assert!(prompt.contains("<|im_end|>"));
        assert!(prompt.ends_with("<|im_start|>assistant\n"));
    }
}
