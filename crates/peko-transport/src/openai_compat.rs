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

    fn parse_openai_delta(
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
            oai_messages.push(serde_json::to_value(msg)?);
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
