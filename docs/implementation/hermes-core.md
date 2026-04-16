# peko-core

> The agent's brain. Platform-agnostic orchestration engine.

---

## Purpose

`peko-core` contains the entire agent reasoning loop, tool dispatch, context management, and session persistence. It has **zero platform-specific dependencies** — you can compile and test it on macOS or desktop Linux.

## Key Components

| Module | Struct | Purpose |
|---|---|---|
| `runtime.rs` | `AgentRuntime` | Top-level orchestrator, owns the [[ReAct-Loop\|ReAct loop]] |
| `tool.rs` | `Tool` trait + `ToolRegistry` | [[Tool-System\|Tool registration and dispatch]] |
| `budget.rs` | `IterationBudget` | Thread-safe iteration limiting |
| `compressor.rs` | `ContextCompressor` | [[Context-Compression\|Token window management]] |
| `session.rs` | `SessionStore` | [[Session-Persistence\|SQLite + FTS5 persistence]] |
| `prompt.rs` | `SystemPrompt` | System prompt assembly |
| `message.rs` | `Message` enum | Strongly-typed conversation messages |

## AgentRuntime

The top-level struct that owns everything:

```rust
pub struct AgentRuntime {
    config: AgentConfig,
    tools: Arc<ToolRegistry>,
    budget: IterationBudget,
    compressor: ContextCompressor,
    session: SessionStore,
    provider: Box<dyn LlmProvider>,
    conversation: Vec<Message>,
    system_prompt: SystemPrompt,
}
```

### Public API

```rust
impl AgentRuntime {
    /// Create a new runtime with config, tools, and LLM provider
    pub async fn new(
        config: AgentConfig,
        tools: Arc<ToolRegistry>,
        provider: Box<dyn LlmProvider>,
    ) -> Result<Self>;

    /// Execute a single task (user input → agent response)
    pub async fn run_task(&mut self, user_input: &str) -> Result<AgentResponse>;

    /// Continuous mode — keep running until interrupted
    pub async fn run_loop(&mut self) -> Result<()>;

    /// Signal the loop to stop (callable from any thread)
    pub fn interrupt(&self);
}
```

### run_task Flow

See [[ReAct-Loop]] for the full deep dive. Summary:

```
user_input
  │
  ▼
1. Append Message::User to conversation
  │
  ▼
2. system_prompt.build() → assemble full prompt
  │
  ▼
3. compressor.check_and_compress(&mut conversation)
  │
  ▼
4. provider.stream_completion(prompt, conversation, tool_schemas)
  │
  ▼
5. Process SSE stream → accumulate text + detect tool calls
  │
  ├─► Tool calls found:
  │     tools.execute(name, args) for each
  │     Append tool results to conversation
  │     budget.decrement()
  │     Loop back to step 3
  │
  └─► Text response only (or budget exhausted):
        Return AgentResponse
        Persist to session store
```

## Message Types

```rust
pub enum Message {
    System(String),
    User(String),
    Assistant {
        text: Option<String>,
        tool_calls: Vec<ToolCall>,
    },
    ToolResult {
        tool_use_id: String,
        name: String,
        content: String,
        is_error: bool,
    },
}
```

These map directly to the LLM API message formats. The [[LLM-Providers|provider layer]] handles serialization to provider-specific JSON.

## SystemPrompt Builder

Assembles the system prompt from components:

1. **SOUL.md** — Agent personality and core instructions
2. **MEMORY.md** — Persistent facts (capped at ~2,200 chars)
3. **Tool schemas** — JSON Schema for all registered tools
4. **Dynamic context** — Files or state injected per-task

The system prompt is **stable within a single task** — it never changes mid-conversation. This follows the Peko Agent design principle of prompt stability.

## Dependencies

```toml
[dependencies]
peko-transport = { path = "../peko-transport" }
peko-config = { path = "../peko-config" }
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
rusqlite = { version = "0.31", features = ["bundled", "fts5"] }
tracing = "0.1"
```

Note: no `nix`, no `libc`, no `image` — those belong in [[peko-hal]] and [[peko-tools-android]].

## Testing on Desktop

Because `peko-core` has no platform dependencies, you can:

1. Create mock `Tool` implementations that return canned results
2. Create a mock `LlmProvider` that returns pre-scripted responses
3. Test the full ReAct loop on macOS/Linux CI
4. Verify context compression without needing a real LLM
5. Test session persistence with a local SQLite file

See [[../roadmap/Testing-Strategy]] for the full approach.

## Related

- [[ReAct-Loop]] — How the agent loop works internally
- [[Tool-System]] — The trait-based tool architecture
- [[Context-Compression]] — Token budget management
- [[Session-Persistence]] — SQLite storage layer
- [[../architecture/Crate-Map]] — Where this crate sits in the workspace

---

#implementation #core #agent
