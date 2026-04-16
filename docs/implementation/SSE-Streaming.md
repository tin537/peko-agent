# SSE Streaming

> Parsing Server-Sent Events from LLM API responses in real-time.

---

## What is SSE?

Server-Sent Events is an HTTP streaming protocol where the server sends a sequence of text events over a long-lived connection. Each event has this format:

```
event: event_type
data: {"json": "payload"}
id: optional_id

```

Events are separated by blank lines (`\n\n`). The `data:` field can span multiple lines.

LLM APIs use SSE to stream tokens as they're generated — the agent can start processing the response before the model finishes thinking.

## SseParser

The low-level parser in [[peko-transport]]:

```rust
pub struct SseParser {
    buffer: String,
}

impl SseParser {
    pub fn new() -> Self;

    /// Feed raw bytes from the HTTP response, get back parsed events
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<SseEvent>;
}

pub struct SseEvent {
    pub event: Option<String>,  // "message_start", "content_block_delta", etc.
    pub data: String,            // JSON payload
    pub id: Option<String>,      // Reconnection ID (rarely used)
}
```

### Parsing Challenges

1. **Chunk boundaries**: HTTP chunks don't align with SSE event boundaries. A single chunk might contain half an event, or three complete events.
2. **Multi-line data**: `data:` can span lines: `data: line1\ndata: line2` → `data = "line1\nline2"`
3. **Provider differences**: Anthropic sends `event:` + `data:`. OpenAI sends only `data:`.
4. **Stream termination**: OpenAI uses `data: [DONE]`. Anthropic uses `event: message_stop`.

The `buffer` field accumulates partial data across `feed()` calls, only emitting complete events.

## Stream Processing Pipeline

```
HTTP Response Body (raw bytes)
       │
       ▼
SseParser::feed(chunk)
       │
       ▼
Vec<SseEvent> (raw event + data strings)
       │
       ▼
Provider-specific deserializer
(Anthropic: match on event type
 OpenAI: parse data JSON, check delta)
       │
       ▼
Vec<StreamEvent> (unified typed events)
       │
       ▼
Agent loop accumulator
  ├── TextDelta → append to text buffer
  ├── ToolUseStart → create new tool call entry
  ├── ToolInputDelta → append to tool args buffer
  ├── ContentBlockStop → parse accumulated tool JSON
  ├── MessageDelta → record token usage + stop reason
  └── MessageStop → yield complete response
```

## Anthropic SSE Processing

```rust
fn process_anthropic_event(event: SseEvent) -> Option<StreamEvent> {
    match event.event.as_deref() {
        Some("message_start") => {
            let msg: MessageStartEvent = serde_json::from_str(&event.data)?;
            Some(StreamEvent::MessageStart {
                id: msg.message.id,
                input_tokens: msg.message.usage.input_tokens,
            })
        }
        Some("content_block_start") => {
            let block: ContentBlockStart = serde_json::from_str(&event.data)?;
            match block.content_block.type_.as_str() {
                "tool_use" => Some(StreamEvent::ToolUseStart {
                    id: block.content_block.id,
                    name: block.content_block.name,
                }),
                _ => Some(StreamEvent::ContentBlockStart { ... }),
            }
        }
        Some("content_block_delta") => {
            let delta: ContentBlockDelta = serde_json::from_str(&event.data)?;
            match delta.delta.type_.as_str() {
                "text_delta" => Some(StreamEvent::TextDelta(delta.delta.text)),
                "input_json_delta" => Some(StreamEvent::ToolInputDelta(delta.delta.partial_json)),
                "thinking_delta" => Some(StreamEvent::ThinkingDelta(delta.delta.thinking)),
                _ => None,
            }
        }
        Some("message_stop") => Some(StreamEvent::MessageStop),
        Some("ping") => Some(StreamEvent::Ping),
        _ => None,
    }
}
```

## Tool Argument Accumulation

Tool arguments arrive as partial JSON strings that must be reassembled:

```
ToolUseStart { id: "tu_1", name: "touch" }
ToolInputDelta("{\"acti")
ToolInputDelta("on\": \"")
ToolInputDelta("tap\", ")
ToolInputDelta("\"x\": 5")
ToolInputDelta("40, \"y")
ToolInputDelta("\": 1200}")
ContentBlockStop { index: 1 }
```

On `ContentBlockStop`, concatenate all deltas and parse:
```rust
let full_json = accumulated_deltas.join("");
let args: serde_json::Value = serde_json::from_str(&full_json)?;
// args = {"action": "tap", "x": 540, "y": 1200}
```

## Error Handling

| Scenario | Handling |
|---|---|
| Malformed JSON in event data | Log warning, skip event |
| Incomplete event at stream end | Buffer discarded, stream ends |
| Network disconnect mid-stream | reqwest returns error, provider failover triggers |
| `[DONE]` sentinel (OpenAI) | Treat as stream end |
| Unknown event type | Skip silently (forward compatibility) |

## Performance Notes

- The parser operates on raw byte chunks — no intermediate string allocation for incomplete events
- `serde_json::from_str` is called once per complete event, not per chunk
- Tool argument accumulation uses a `String` that grows incrementally (amortized O(1) appends)

## Related

- [[LLM-Providers]] — Provider-specific SSE formats
- [[peko-transport]] — Where SseParser lives
- [[ReAct-Loop]] — Consumes StreamEvents

---

#implementation #sse #streaming #parsing
