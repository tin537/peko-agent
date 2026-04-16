# peko-transport

> Async HTTP client and LLM API streaming layer.

---

## Purpose

`peko-transport` handles all network communication with LLM providers. It abstracts away provider-specific API differences behind a unified `LlmProvider` trait, so [[peko-core]] never knows whether it's talking to Anthropic, OpenRouter, or a local model.

## LlmProvider Trait

The core abstraction — see [[LLM-Providers]] for implementation details:

```rust
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn stream_completion(
        &self,
        system_prompt: &str,
        messages: &[Message],
        tools: &[serde_json::Value],
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>>;

    fn model_name(&self) -> &str;
    fn max_context_tokens(&self) -> usize;
}
```

Returns an async `Stream` of `StreamEvent`s — [[peko-core]] consumes these to build up the assistant's response in real-time.

## StreamEvent

Unified event type across all providers. See [[SSE-Streaming]] for parsing details:

```rust
pub enum StreamEvent {
    MessageStart { id: String, input_tokens: usize },
    ContentBlockStart { index: usize, block_type: ContentBlockType },
    TextDelta(String),
    ToolUseStart { id: String, name: String },
    ToolInputDelta(String),    // Partial JSON for tool args
    ThinkingDelta(String),     // Extended thinking content
    ContentBlockStop { index: usize },
    MessageDelta { output_tokens: usize, stop_reason: StopReason },
    MessageStop,
    Ping,
}
```

## Provider Implementations

### AnthropicProvider

```rust
pub struct AnthropicProvider {
    client: reqwest::Client,
    api_key: String,
    model: String,
    max_tokens: usize,
    base_url: String,  // default: https://api.anthropic.com
}
```

- Endpoint: `POST /v1/messages` with `stream: true`
- Headers: `anthropic-version: 2023-06-01`, `x-api-key`
- Supports prompt caching via `cache_control` breakpoints
- Supports extended thinking (`thinking` content blocks)
- SSE event sequence: `message_start` → `content_block_start` → `content_block_delta`* → `content_block_stop` → `message_delta` → `message_stop`

### OpenAICompatProvider

```rust
pub struct OpenAICompatProvider {
    client: reqwest::Client,
    api_key: String,
    model: String,
    base_url: String,
    max_tokens: usize,
    extra_headers: HeaderMap,
}
```

- Works with OpenRouter, Nous Portal, any OpenAI-compatible endpoint
- Endpoint: `POST /chat/completions` with `stream: true`
- Tool calls arrive as `delta.tool_calls[i].function.arguments` fragments
- Must accumulate per tool call index

### PekoLocalProvider

- For local local models models via vLLM or llama.cpp
- Uses ChatML format (`<|im_start|>` / `<|im_end|>`)
- Tool definitions in `<tools>` XML tags
- Tool calls parsed from `<tool_call>` XML in output
- Connects via OpenAI-compatible API to local server

### ProviderChain

Wraps multiple providers for automatic failover:

```rust
pub struct ProviderChain {
    providers: Vec<Box<dyn LlmProvider>>,
}
```

Tries providers in priority order. Falls back on:
- Connection errors
- HTTP 429 (rate limit)
- HTTP 5xx (server errors)

## SSE Parser

Low-level Server-Sent Events parser. See [[SSE-Streaming]] for details:

```rust
pub struct SseParser {
    buffer: String,
}

impl SseParser {
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<SseEvent>;
}
```

Handles multi-line data fields, event type discrimination, `[DONE]` sentinel.

## HTTP Client Configuration

The `reqwest::Client` is built once at startup:

| Setting | Value | Rationale |
|---|---|---|
| Connection pooling | Enabled (keep-alive) | Reuse connections across requests |
| TLS | rustls (no OpenSSL) | Avoid heavy OpenSSL dependency |
| Connect timeout | 30s | Mobile networks can be slow |
| Read timeout | 300s | Long generations on large models |
| Retry | Exponential backoff on 5xx | Resilience to transient failures |
| HTTP version | HTTP/2 | Multiplexing for efficiency |

## Dependencies

```toml
[dependencies]
reqwest = { version = "0.12", features = ["rustls-tls", "stream"] }
tokio = { version = "1", features = ["rt", "net", "time"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
async-trait = "0.1"
futures-core = "0.3"  # Stream trait
tracing = "0.1"
```

## Related

- [[LLM-Providers]] — Provider implementation deep dive
- [[SSE-Streaming]] — SSE parsing deep dive
- [[peko-core]] — Consumes the stream
- [[../architecture/Crate-Map]] — Dependency position

---

#implementation #transport #network
