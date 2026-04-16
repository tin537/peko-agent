use crate::provider::{LlmProvider, Message};
use crate::sse::SseParser;
use crate::stream::{ContentBlockType, StopReason, StreamEvent};
use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;

pub struct AnthropicProvider {
    client: Client,
    api_key: String,
    model: String,
    max_tokens: usize,
    base_url: String,
}

impl AnthropicProvider {
    pub fn new(api_key: String, model: String, max_tokens: usize, base_url: Option<String>) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .connect_timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("failed to build HTTP client");

        Self {
            client,
            api_key,
            model,
            max_tokens,
            base_url: base_url.unwrap_or_else(|| "https://api.anthropic.com".to_string()),
        }
    }

    fn parse_event(event: &crate::sse::SseEvent) -> Option<StreamEvent> {
        let event_type = event.event.as_deref()?;
        let data: serde_json::Value = serde_json::from_str(&event.data).ok()?;

        match event_type {
            "message_start" => {
                let id = data["message"]["id"].as_str()?.to_string();
                let input_tokens = data["message"]["usage"]["input_tokens"].as_u64()? as usize;
                Some(StreamEvent::MessageStart { id, input_tokens })
            }
            "content_block_start" => {
                let index = data["index"].as_u64()? as usize;
                let block = &data["content_block"];
                match block["type"].as_str()? {
                    "tool_use" => Some(StreamEvent::ToolUseStart {
                        id: block["id"].as_str()?.to_string(),
                        name: block["name"].as_str()?.to_string(),
                    }),
                    "thinking" => Some(StreamEvent::ContentBlockStart {
                        index,
                        block_type: ContentBlockType::Thinking,
                    }),
                    _ => Some(StreamEvent::ContentBlockStart {
                        index,
                        block_type: ContentBlockType::Text,
                    }),
                }
            }
            "content_block_delta" => {
                let delta = &data["delta"];
                match delta["type"].as_str()? {
                    "text_delta" => Some(StreamEvent::TextDelta(
                        delta["text"].as_str()?.to_string(),
                    )),
                    "input_json_delta" => Some(StreamEvent::ToolInputDelta(
                        delta["partial_json"].as_str()?.to_string(),
                    )),
                    "thinking_delta" => Some(StreamEvent::ThinkingDelta(
                        delta["thinking"].as_str()?.to_string(),
                    )),
                    _ => None,
                }
            }
            "content_block_stop" => {
                let index = data["index"].as_u64()? as usize;
                Some(StreamEvent::ContentBlockStop { index })
            }
            "message_delta" => {
                let output_tokens = data["usage"]["output_tokens"].as_u64().unwrap_or(0) as usize;
                let stop_reason_str = data["delta"]["stop_reason"].as_str().unwrap_or("end_turn");
                let stop_reason = match stop_reason_str {
                    "end_turn" => StopReason::EndTurn,
                    "tool_use" => StopReason::ToolUse,
                    "max_tokens" => StopReason::MaxTokens,
                    "stop_sequence" => StopReason::StopSequence,
                    _ => StopReason::Unknown,
                };
                Some(StreamEvent::MessageDelta { output_tokens, stop_reason })
            }
            "message_stop" => Some(StreamEvent::MessageStop),
            "ping" => Some(StreamEvent::Ping),
            _ => None,
        }
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    async fn stream_completion(
        &self,
        system_prompt: &str,
        messages: &[Message],
        tools: &[serde_json::Value],
    ) -> anyhow::Result<BoxStream<'static, anyhow::Result<StreamEvent>>> {
        let url = format!("{}/v1/messages", self.base_url);

        let mut body = json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "stream": true,
            "system": system_prompt,
            "messages": messages,
        });

        if !tools.is_empty() {
            body["tools"] = serde_json::Value::Array(tools.to_vec());
        }

        let response = self.client
            .post(&url)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .header("x-api-key", &self.api_key)
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Anthropic API error {}: {}", status, body);
        }

        let byte_stream = response.bytes_stream();

        let event_stream = stream::unfold(
            (byte_stream, SseParser::new()),
            |(mut byte_stream, mut parser)| async move {
                use futures::StreamExt;
                loop {
                    match byte_stream.next().await {
                        Some(Ok(chunk)) => {
                            let sse_events = parser.feed(&chunk);
                            let stream_events: Vec<anyhow::Result<StreamEvent>> = sse_events
                                .iter()
                                .filter_map(Self::parse_event)
                                .map(Ok)
                                .collect();
                            if !stream_events.is_empty() {
                                return Some((stream::iter(stream_events), (byte_stream, parser)));
                            }
                        }
                        Some(Err(e)) => {
                            return Some((
                                stream::iter(vec![Err(anyhow::anyhow!("stream error: {}", e))]),
                                (byte_stream, parser),
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
        200_000
    }
}
