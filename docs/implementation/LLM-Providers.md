# LLM Providers

> Anthropic, OpenAI-compatible, and local model implementations.

---

## Provider Architecture

All providers implement the `LlmProvider` trait from [[peko-transport]]:

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

The [[ReAct-Loop|agent loop]] doesn't know or care which provider it's using. Swap providers at runtime via [[peko-config|config.toml]] without touching any code.

## AnthropicProvider

The primary cloud provider for high-capability reasoning.

### API Details

| Field | Value |
|---|---|
| Endpoint | `POST https://api.anthropic.com/v1/messages` |
| Auth header | `x-api-key: {api_key}` |
| Version header | `anthropic-version: 2023-06-01` |
| Streaming | `stream: true` in request body |

### Request Body Structure

```json
{
  "model": "claude-sonnet-4-20250514",
  "max_tokens": 8192,
  "stream": true,
  "system": "You are Peko, an autonomous agent...",
  "messages": [
    {"role": "user", "content": "Send an SMS..."},
    {"role": "assistant", "content": [
      {"type": "tool_use", "id": "tu_1", "name": "sms", "input": {...}}
    ]},
    {"role": "user", "content": [
      {"type": "tool_result", "tool_use_id": "tu_1", "content": "Sent"}
    ]}
  ],
  "tools": [
    {"name": "screenshot", "description": "...", "input_schema": {...}}
  ]
}
```

### SSE Event Sequence

```
event: message_start
data: {"type":"message_start","message":{"id":"msg_...","usage":{"input_tokens":1523}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Let me "}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"take a screenshot."}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: content_block_start
data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"tu_1","name":"screenshot","input":{}}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{}"}}

event: content_block_stop
data: {"type":"content_block_stop","index":1}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":42}}

event: message_stop
```

### Special Features

- **Prompt caching**: Add `cache_control: {"type": "ephemeral"}` to system prompt blocks. Reduces input token costs on repeated calls within 5 minutes.
- **Extended thinking**: `thinking` content blocks with `signature_delta` for chain-of-thought reasoning.
- **Vision**: Tool results can include `{"type": "image", "source": {"type": "base64", ...}}` for screenshot analysis.

## OpenAICompatProvider

Works with OpenRouter, Nous Portal, vLLM, llama.cpp, and any OpenAI-compatible API.

### API Details

| Field | Value |
|---|---|
| Endpoint | `POST {base_url}/chat/completions` |
| Auth header | `Authorization: Bearer {api_key}` |
| Streaming | `stream: true` |

### Key Differences from Anthropic

| Aspect | Anthropic | OpenAI-compatible |
|---|---|---|
| System prompt | Separate `system` field | First message with `role: "system"` |
| Tool results | `role: "user"` with `tool_result` type | `role: "tool"` with `tool_call_id` |
| SSE format | `event:` + `data:` | `data:` only |
| Stream end | `event: message_stop` | `data: [DONE]` |
| Tool call deltas | `input_json_delta` | `tool_calls[i].function.arguments` |

### Tool Call Accumulation

OpenAI-format streams tool calls as fragments indexed by position:

```
delta: {"tool_calls": [{"index": 0, "function": {"arguments": "{\"ac"}}]}
delta: {"tool_calls": [{"index": 0, "function": {"arguments": "tion\":"}}]}
delta: {"tool_calls": [{"index": 0, "function": {"arguments": "\"tap\"}"}}]}
```

Must accumulate `arguments` per `index` and parse as JSON when content_block stops.

## PekoLocalProvider

For running local models models locally.

### ChatML Format

```
<|im_start|>system
You are Peko, an autonomous agent...

<tools>
[{"name": "screenshot", "description": "...", "parameters": {...}}]
</tools>
<|im_end|>
<|im_start|>user
Send an SMS to +1234567890
<|im_end|>
<|im_start|>assistant
I'll send that SMS for you.
<tool_call>
{"name": "sms", "arguments": {"to": "+1234567890", "message": "hello"}}
</tool_call>
<|im_end|>
<|im_start|>tool
{"result": "SMS sent successfully"}
<|im_end|>
```

Tool calls are parsed from `<tool_call>` XML tags in the model output. Tool definitions go in `<tools>` tags in the system prompt.

Connects to local inference server (vLLM, llama.cpp) via the same OpenAI-compatible API, but with custom prompt formatting.

## ProviderChain (Failover)

```rust
pub struct ProviderChain {
    providers: Vec<Box<dyn LlmProvider>>,
}
```

Tries providers in priority order defined in [[peko-config|config.toml]]:

```toml
[provider]
priority = ["anthropic", "openrouter", "local"]
```

Failover triggers:
- Connection timeout / network error
- HTTP 429 (rate limited)
- HTTP 500-599 (server error)

Does NOT failover on:
- HTTP 400 (bad request — our fault, not transient)
- HTTP 401/403 (auth error — won't fix itself)

## Related

- [[SSE-Streaming]] — How raw bytes become StreamEvents
- [[peko-transport]] — Where providers live
- [[ReAct-Loop]] — How the loop consumes the stream
- [[Context-Compression]] — Managing token limits per provider
- [[../peko-config]] — Provider configuration

---

#implementation #providers #llm #anthropic #openai
