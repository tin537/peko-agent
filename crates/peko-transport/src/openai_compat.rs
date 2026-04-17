use crate::provider::{LlmProvider, Message};
use crate::sse::SseParser;
use crate::stream::{StopReason, StreamEvent};
use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use reqwest::Client;
use serde_json::json;
use std::collections::HashMap;

pub struct OpenAICompatProvider {
    client: Client,
    api_key: String,
    model: String,
    base_url: String,
    max_tokens: usize,
}

impl OpenAICompatProvider {
    pub fn new(
        api_key: String,
        model: String,
        base_url: String,
        max_tokens: usize,
    ) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .connect_timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("failed to build HTTP client");

        Self { client, api_key, model, base_url, max_tokens }
    }

    /// Convert a neutral TransportMessage to OpenAI wire format.
    /// Handles: assistant tool_calls, role:tool results, vision images.
    pub(crate) fn to_openai_message(msg: &Message) -> serde_json::Value {
        use crate::provider::{MessageContent, ContentBlock};

        match (&msg.role as &str, &msg.content) {
            // User with plain text
            ("user", MessageContent::Text(text)) => {
                json!({"role": "user", "content": text})
            }
            // User with content blocks (tool results or images)
            ("user", MessageContent::Blocks(blocks)) => {
                // Check if this is a tool result (Anthropic sends as role:user with tool_result blocks)
                let has_tool_result = blocks.iter().any(|b| matches!(b, ContentBlock::ToolResult { .. }));

                if has_tool_result {
                    // Convert to OpenAI tool result format
                    // Each tool_result block becomes a separate role:tool message
                    // For simplicity, take the first one
                    for block in blocks {
                        if let ContentBlock::ToolResult { tool_use_id, content, .. } = block {
                            return json!({
                                "role": "tool",
                                "tool_call_id": tool_use_id,
                                "content": content,
                            });
                        }
                    }
                    json!({"role": "user", "content": ""})
                } else {
                    // User message with mixed content (text + images)
                    let mut parts: Vec<serde_json::Value> = Vec::new();
                    for block in blocks {
                        match block {
                            ContentBlock::Text { text } => {
                                parts.push(json!({"type": "text", "text": text}));
                            }
                            ContentBlock::Image { source } => {
                                // MiMo/OpenAI vision format: image_url with data URI
                                let data_uri = format!("data:{};base64,{}", source.media_type, source.data);
                                parts.push(json!({
                                    "type": "image_url",
                                    "image_url": {"url": data_uri}
                                }));
                            }
                            _ => {}
                        }
                    }
                    json!({"role": "user", "content": parts})
                }
            }
            // Assistant with plain text
            ("assistant", MessageContent::Text(text)) => {
                json!({"role": "assistant", "content": text})
            }
            // Assistant with tool calls (Anthropic format blocks → OpenAI tool_calls)
            ("assistant", MessageContent::Blocks(blocks)) => {
                let mut text_content = String::new();
                let mut tool_calls: Vec<serde_json::Value> = Vec::new();

                for block in blocks {
                    match block {
                        ContentBlock::Text { text } => {
                            text_content.push_str(text);
                        }
                        ContentBlock::ToolUse { id, name, input } => {
                            tool_calls.push(json!({
                                "id": id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": input.to_string(),
                                }
                            }));
                        }
                        _ => {}
                    }
                }

                let mut msg = json!({
                    "role": "assistant",
                    "content": if text_content.is_empty() { serde_json::Value::Null } else { json!(text_content) },
                });
                if !tool_calls.is_empty() {
                    msg["tool_calls"] = json!(tool_calls);
                }
                msg
            }
            // Tool result (shouldn't reach here if converted from user blocks above, but handle it)
            ("tool", MessageContent::Text(content)) => {
                json!({"role": "tool", "content": content})
            }
            // Fallback
            (role, MessageContent::Text(text)) => {
                json!({"role": role, "content": text})
            }
            (role, _) => {
                json!({"role": role, "content": ""})
            }
        }
    }

    pub(crate) fn parse_openai_delta(
        data: &serde_json::Value,
        tool_buffers: &mut HashMap<usize, (String, String, String)>,
    ) -> Vec<StreamEvent> {
        let mut events = Vec::new();
        let Some(choice) = data["choices"].get(0) else { return events };
        let delta = &choice["delta"];

        if let Some(content) = delta["content"].as_str() {
            if !content.is_empty() {
                events.push(StreamEvent::TextDelta(content.to_string()));
            }
        }

        if let Some(tool_calls) = delta["tool_calls"].as_array() {
            for tc in tool_calls {
                let index = tc["index"].as_u64().unwrap_or(0) as usize;
                let func = &tc["function"];

                if let Some(name) = func["name"].as_str() {
                    let id = tc["id"].as_str().unwrap_or("").to_string();
                    tool_buffers.insert(index, (id.clone(), name.to_string(), String::new()));
                    events.push(StreamEvent::ToolUseStart {
                        id,
                        name: name.to_string(),
                    });
                }

                if let Some(args) = func["arguments"].as_str() {
                    if let Some(buf) = tool_buffers.get_mut(&index) {
                        buf.2.push_str(args);
                    }
                    events.push(StreamEvent::ToolInputDelta(args.to_string()));
                }
            }
        }

        if let Some(finish) = choice["finish_reason"].as_str() {
            let stop_reason = match finish {
                "stop" => StopReason::EndTurn,
                "tool_calls" => StopReason::ToolUse,
                "length" => StopReason::MaxTokens,
                _ => StopReason::Unknown,
            };
            let output_tokens = data["usage"]["completion_tokens"].as_u64().unwrap_or(0) as usize;
            events.push(StreamEvent::MessageDelta { output_tokens, stop_reason });
            events.push(StreamEvent::MessageStop);
        }

        events
    }
}

#[async_trait]
impl LlmProvider for OpenAICompatProvider {
    async fn stream_completion(
        &self,
        system_prompt: &str,
        messages: &[Message],
        tools: &[serde_json::Value],
    ) -> anyhow::Result<BoxStream<'static, anyhow::Result<StreamEvent>>> {
        let url = format!("{}/chat/completions", self.base_url);

        let mut oai_messages = vec![json!({
            "role": "system",
            "content": system_prompt,
        })];
        for msg in messages {
            oai_messages.push(Self::to_openai_message(msg));
        }

        let mut body = json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "stream": true,
            "messages": oai_messages,
        });

        if !tools.is_empty() {
            let oai_tools: Vec<serde_json::Value> = tools.iter().map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t["name"],
                        "description": t["description"],
                        "parameters": t["input_schema"],
                    }
                })
            }).collect();
            body["tools"] = serde_json::Value::Array(oai_tools);
        }

        let response = self.client
            .post(&url)
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("OpenAI-compatible API error {}: {}", status, body);
        }

        let byte_stream = response.bytes_stream();

        let event_stream = stream::unfold(
            (byte_stream, SseParser::new(), HashMap::<usize, (String, String, String)>::new()),
            |(mut byte_stream, mut parser, mut tool_buffers)| async move {
                use futures::StreamExt;
                loop {
                    match byte_stream.next().await {
                        Some(Ok(chunk)) => {
                            let sse_events = parser.feed(&chunk);
                            let mut stream_events: Vec<anyhow::Result<StreamEvent>> = Vec::new();
                            for sse in &sse_events {
                                if let Ok(data) = serde_json::from_str::<serde_json::Value>(&sse.data) {
                                    for ev in Self::parse_openai_delta(&data, &mut tool_buffers) {
                                        stream_events.push(Ok(ev));
                                    }
                                }
                            }
                            if !stream_events.is_empty() {
                                return Some((
                                    stream::iter(stream_events),
                                    (byte_stream, parser, tool_buffers),
                                ));
                            }
                        }
                        Some(Err(e)) => {
                            return Some((
                                stream::iter(vec![Err(anyhow::anyhow!("stream error: {}", e))]),
                                (byte_stream, parser, tool_buffers),
                            ));
                        }
                        None => return None,
                    }
                }
            },
        )
        .flatten();

        Ok(Box::pin(event_stream))
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    fn max_context_tokens(&self) -> usize {
        128_000
    }
}
