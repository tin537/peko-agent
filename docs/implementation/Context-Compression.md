# Context Compression

> Managing the conversation token window to prevent overflow.

---

## The Problem

LLMs have finite context windows. A long agent task can easily exceed them:

- Tool results (especially screenshots as base64) are large
- Each iteration adds assistant message + tool calls + tool results
- 50 iterations × ~2,000 tokens each = 100,000 tokens just for tool exchanges

Without compression, the agent hits the context limit and fails mid-task.

## The Solution

`ContextCompressor` in [[peko-core]] implements a **head-tail compaction** strategy:

```rust
pub struct ContextCompressor {
    max_context_tokens: usize,  // e.g., 200,000
    history_share: f32,         // fraction for history (default 0.7)
    token_counter: Box<dyn TokenCounter>,
}

impl ContextCompressor {
    pub fn check_and_compress(
        &self, conversation: &mut Vec<Message>
    ) -> Result<CompressionResult>;
}
```

## How It Works

```
Before compression:
┌──────────────────────────────────┐
│ Message 1: System prompt         │ ← Always preserved
│ Message 2: User input            │ ← Always preserved
│ Message 3: Assistant + tools     │ ┐
│ Message 4: Tool results          │ │
│ Message 5: Assistant + tools     │ │ Middle turns
│ Message 6: Tool results          │ │ (candidates for compression)
│ Message 7: Assistant + tools     │ │
│ Message 8: Tool results          │ ┘
│ Message 9: Assistant + tools     │ ← Recent context preserved
│ Message 10: Tool results         │ ← Recent context preserved
│ Message 11: Assistant response   │ ← Recent context preserved
└──────────────────────────────────┘

After compression:
┌──────────────────────────────────┐
│ Message 1: System prompt         │ ← Preserved
│ Message 2: User input            │ ← Preserved
│ SUMMARY: "Used screenshot tool   │ ← Compressed middle
│ 3 times, touch tool 2 times.    │
│ Navigated to Settings > WiFi.   │
│ Found network 'HomeNet'."       │
│ Message 9: Assistant + tools     │ ← Recent preserved
│ Message 10: Tool results         │ ← Recent preserved
│ Message 11: Assistant response   │ ← Recent preserved
└──────────────────────────────────┘
```

## Compression Trigger

Compression fires when:

```
total_tokens(conversation) > max_context_tokens × history_share
```

With defaults (200K context, 0.7 share), that's **140K tokens**. The remaining 60K is reserved for the system prompt, tool schemas, and the LLM's response.

## Compression Strategies

### Strategy 1: Heuristic (default, no LLM call)

Extract only the essentials from middle turns:
- Tool names that were called
- Final result status (success/error)
- Key data points (coordinates clicked, files read, etc.)

Produces a summary like:
```
Previous actions: screenshot (3x), touch at (540,1200), (320,800), (540,600),
text_input "password123", key_event HOME. Navigated through Settings → WiFi → HomeNet.
```

**Pros**: Fast, free, no extra API call
**Cons**: May lose nuanced context

### Strategy 2: LLM-assisted (optional)

Send the middle turns to the LLM with a compression prompt:
```
Summarize the following conversation history, preserving all
important decisions, observations, and state changes:
```

**Pros**: Better quality summaries
**Cons**: Extra API call, latency, cost. Only worth it for very long tasks.

## Token Counting

```rust
pub trait TokenCounter: Send + Sync {
    fn count(&self, text: &str) -> usize;
}
```

Implementations:
- **Approximate**: `text.len() / 4` — fast, ~80% accurate
- **Tiktoken-based**: Use a tokenizer matching the model — accurate but heavier dependency

For mobile, the approximate counter is usually sufficient. The 0.7 share factor provides enough margin for estimation errors.

## Configuration

In [[peko-config|config.toml]]:

```toml
[agent]
context_window = 200000   # Model's max context
history_share = 0.7       # 70% for history, 30% reserved
```

Different models need different values:
| Model | Context | Suggested history_share |
|---|---|---|
| Claude Sonnet/Opus | 200K | 0.7 |
| GPT-4o | 128K | 0.65 |
| local model (local) | 8K | 0.5 |

## Related

- [[ReAct-Loop]] — Compression happens at step 3
- [[peko-core]] — Where ContextCompressor lives
- [[LLM-Providers]] — Token limits per provider
- [[Session-Persistence]] — Full uncompressed history still saved to SQLite

---

#implementation #context #compression #tokens
