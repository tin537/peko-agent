# LLM Providers

> Anthropic, seven OpenAI-compatible clouds, embedded GGUF, and a UDS
> local-daemon path — all behind a unified `LlmProvider` trait with
> comma-separated chain fallback for dual-brain escalation.

## Supported providers (as of Apr 2026)

| Name in config | Backend | Env var | Default base URL |
|---|---|---|---|
| `anthropic` | `AnthropicProvider` | `ANTHROPIC_API_KEY` | `https://api.anthropic.com` |
| `openrouter` | `OpenAICompatProvider` | `OPENROUTER_API_KEY` | `https://openrouter.ai/api/v1` |
| `openai` | `OpenAICompatProvider` | `OPENAI_API_KEY` | `https://api.openai.com/v1` |
| `groq` | `OpenAICompatProvider` | `GROQ_API_KEY` | `https://api.groq.com/openai/v1` |
| `deepseek` | `OpenAICompatProvider` | `DEEPSEEK_API_KEY` | `https://api.deepseek.com` |
| `mistral` | `OpenAICompatProvider` | `MISTRAL_API_KEY` | `https://api.mistral.ai/v1` |
| `together` | `OpenAICompatProvider` | `TOGETHER_API_KEY` | `https://api.together.xyz/v1` |
| `embedded` | `EmbeddedProvider` (candle, in-process GGUF) | — | filesystem path |
| `local` (UDS) | `UnixSocketProvider` → `peko-llm-daemon` | — | `unix://@peko-llm` |
| *any other name* | Generic `OpenAICompatProvider` | — | `http://localhost:11434/v1` (Ollama) |

Provider selection + routing is done by `build_dual_brain` in `peko-core/src/runtime.rs` (see [[Dual-Brain]]). For the brain-mode picker UI wired to this, see `src/web/ui.rs` — Settings → Brain Mode.

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

## UnixSocketProvider

OpenAI-compatible, but speaks HTTP/1.1 over a **Unix Domain Socket** instead of TCP. Designed for on-device inference where the LLM runs in a sibling process — typically [[../../crates/peko-llm-daemon/README|peko-llm-daemon]] with llama.cpp.

### Why UDS instead of localhost TCP?

- No network stack overhead (~5-10 μs per message vs ~50-100 μs for TCP)
- No port conflicts / TIME_WAIT / port allocation
- File permissions or abstract namespace give automatic access control
- SELinux-friendly on Android: `shell` user can't create AF_UNIX socket **files** in `/data/local/tmp/`, but **abstract namespace** sockets live in kernel space and bypass filesystem ACLs entirely.

### Base URL scheme

```toml
[provider.local]
base_url = "unix:///tmp/peko.sock"   # filesystem path
# or
base_url = "unix://@peko-llm"         # Linux abstract namespace (prefix '@' → \0 in kernel)
```

When `OpenAICompatProvider` / the brain builder sees a `unix://` prefix, it instantiates `UnixSocketProvider` instead of a TCP HTTP client.

### Connection

`UnixSocketProvider::connect()` uses libc directly to handle abstract namespace (`std::os::linux::net::SocketAddrExt` only exists under `target_os = "linux"`, not `"android"`):

```rust
// Abstract namespace: first byte of sun_path is NUL
let fd = libc::socket(AF_UNIX, SOCK_STREAM, 0);
let mut addr: sockaddr_un = zeroed();
addr.sun_family = AF_UNIX;
addr.sun_path[0] = 0;                 // the NUL prefix
for (i, b) in name.bytes().enumerate() {
    addr.sun_path[i+1] = b as _;
}
libc::connect(fd, &addr, addrlen);
UnixStream::from_std(UnixStream::from_raw_fd(fd))
```

### Wire protocol

Plain HTTP/1.1 POST to `/v1/chat/completions` with `Transfer-Encoding: chunked` response. Body is OpenAI-compatible Chat Completions format — identical to `OpenAICompatProvider` for TCP. Each chunk contains SSE `data: {...}\n\n` frames.

Response parsing reuses the same `SseParser` + `OpenAICompatProvider::parse_openai_delta` helpers, so tool-call reconstruction and delta handling match the TCP path exactly.

### Stream lifecycle gotcha

The provider splits the stream into `(read_half, write_half)` but **keeps the write half alive** in the unfold state. Dropping it would send FIN, which cpp-httplib on the other side interprets as client disconnect and aborts the response mid-generation.

### Related daemon

See [[../../crates/peko-llm-daemon/README|peko-llm-daemon]] — the C++ process that serves the other end of the socket and runs llama.cpp inference.

## ProviderChain (Failover)

```rust
pub struct ProviderChain {
    providers: Vec<Box<dyn LlmProvider>>,
}
```

Used in two places now:

### 1. Legacy `priority` list

```toml
[provider]
priority = ["anthropic", "openrouter", "local"]
```

Preserved for backward-compat; `build_provider_helper` still reads it.

### 2. Brain-level cloud chain (preferred — Apr 2026)

The `provider.brain` string now accepts **comma-separated chains** on either
side of the colon:

```toml
[provider]
brain = "local:anthropic,openrouter"
# tries anthropic for escalation; if it rate-limits / errors, falls back
# to openrouter. Missing api_keys are silently skipped at build time.
```

Examples:

| `brain` value | Behavior |
|---|---|
| `"local:anthropic"` | Dual, single cloud (legacy behaviour preserved) |
| `"local:anthropic,openrouter"` | Dual; anthropic primary, openrouter fallback |
| `"local:groq,deepseek,openrouter"` | Dual; 3-deep chain of cheap options |
| `"anthropic"` | Cloud-only, single |
| `"anthropic,openrouter"` | Cloud-only with fallback (no local) |
| `"local"` / `"embedded"` | Local-only, no cloud |

Parsing happens in `build_dual_brain`; the chain is wrapped in
`ProviderChain` when length ≥ 2, unwrapped to a bare `Box<dyn LlmProvider>`
when length == 1. See the 4 unit tests in
`crates/peko-core/src/runtime.rs` (`multi_provider_tests`).

### Failover triggers

- Connection timeout / network error
- HTTP 429 (rate limited)
- HTTP 500-599 (server error)

### Does NOT failover on

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
