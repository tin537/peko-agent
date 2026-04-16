# ReAct Loop

> The observe-think-act cycle that drives the agent.

---

## What is ReAct?

**ReAct** (Reasoning + Acting) is the paradigm from [[../research/ReAct-Paper|Yao et al., ICLR 2023]] where an LLM alternates between:

1. **Observe** — receive input (user message, tool results, screenshots)
2. **Think** — generate reasoning traces (natural language)
3. **Act** — invoke tools to affect the environment

This interleaving outperforms both reasoning-only (chain-of-thought) and action-only approaches. It's the de facto architecture for all production agent systems.

## Peko Agent's Implementation

The loop lives in [[peko-core]]'s `AgentRuntime::run_task()`:

```
┌─────────────────────────────────────────────┐
│ run_task("Send SMS to +1234567890")         │
└──────────────────┬──────────────────────────┘
                   │
    ┌──────────────▼──────────────────┐
    │ 1. Append Message::User         │
    │    to conversation              │
    └──────────────┬──────────────────┘
                   │
    ┌──────────────▼──────────────────┐
    │ 2. Build system prompt          │
    │    SOUL + MEMORY + tool schemas │
    └──────────────┬──────────────────┘
                   │
    ┌──────────────▼──────────────────┐◄─────────────────────┐
    │ 3. Check context budget         │                      │
    │    Compress if needed           │                      │
    └──────────────┬──────────────────┘                      │
                   │                                         │
    ┌──────────────▼──────────────────┐                      │
    │ 4. Stream LLM completion        │                      │
    │    provider.stream_completion() │                      │
    └──────────────┬──────────────────┘                      │
                   │                                         │
    ┌──────────────▼──────────────────┐                      │
    │ 5. Process SSE stream           │                      │
    │    Accumulate text + tool calls │                      │
    └──────────────┬──────────────────┘                      │
                   │                                         │
              ┌────┴────┐                                    │
              │ Tools?  │                                    │
              └────┬────┘                                    │
            Yes    │    No                                   │
         ┌─────────┴──────────┐                              │
         │                    │                              │
    ┌────▼────────────┐  ┌────▼──────────────┐              │
    │ 6a. Execute     │  │ 6b. Return        │              │
    │ each tool call  │  │ AgentResponse     │              │
    │ via registry    │  │ Persist session   │              │
    └────┬────────────┘  └───────────────────┘              │
         │                                                   │
    ┌────▼────────────┐                                      │
    │ 7. Append tool  │                                      │
    │ results to conv │                                      │
    │ budget.decr()   ├──────────────────────────────────────┘
    └─────────────────┘
```

## Iteration Budget

The loop is bounded by `IterationBudget` to prevent runaway execution:

```rust
pub struct IterationBudget {
    remaining: Arc<AtomicUsize>,
    max: usize,
    interrupted: Arc<AtomicBool>,
}
```

- **Default**: 50 iterations per task (configurable in [[peko-config|config.toml]])
- **Thread-safe**: Uses `AtomicUsize` / `AtomicBool` — no locks needed
- **External interrupt**: The control socket handler can call `budget.interrupt()` from a different tokio task
- **Decrement check**: Each tool execution cycle decrements. If exhausted, the loop exits with whatever partial response exists

Why atomics instead of a mutex? The budget is shared between:
- The agent loop task (decrements on each iteration)
- The socket listener task (can set interrupted flag)
- The signal handler (SIGTERM sets interrupted)

`Arc<AtomicUsize>` is lock-free and safe across all of these.

## Tool Dispatch Within a Single Iteration

When the LLM returns multiple tool calls in one response, they execute sequentially:

```
LLM response:
  tool_call[0]: screenshot {}
  tool_call[1]: touch {"x": 540, "y": 1200}

Execution:
  1. screenshot.execute({}) → base64 PNG
  2. touch.execute({"x": 540, "y": 1200}) → "tapped (540, 1200)"

Both results appended as Message::ToolResult
```

Sequential execution is intentional — tool calls often have implicit ordering (take screenshot, then tap what you see). Parallel execution could introduce race conditions with hardware state.

## Dangerous Tool Confirmation

When a tool has `is_dangerous() == true` (e.g., [[peko-tools-android|SmsTool, CallTool, ShellTool]]):

```
1. Agent requests: sms.execute({"to": "+1234567890", "message": "hello"})
2. Runtime checks: sms_tool.is_dangerous() → true
3. Runtime sends confirmation request to control socket
4. External controller approves or denies
5. If approved: execute. If denied: return error to LLM as tool result
```

The LLM sees denied actions as tool errors and can reason about alternatives.

## Loop Termination Conditions

The loop exits when any of these are true:

| Condition | Behavior |
|---|---|
| LLM returns text with no tool calls | Normal completion — return response |
| Budget exhausted (remaining = 0) | Force stop — return partial response |
| `interrupted` flag set | Immediate stop — return partial response |
| LLM returns `stop_reason: end_turn` | Normal completion |
| Provider error (after failover exhausted) | Error response |

## Example Trace

```
Task: "What time is it in Tokyo?"

Iteration 1:
  → LLM thinks: "I need to check the current time. Let me use shell."
  → tool_call: shell {"command": "date -u"}
  → tool_result: "Tue Apr 15 03:42:17 UTC 2026"

Iteration 2:
  → LLM thinks: "UTC is +0, Tokyo is UTC+9, so 12:42 PM"
  → text: "It's currently 12:42 PM in Tokyo (JST, UTC+9)."
  → No tool calls → loop exits

Result: AgentResponse { text: "It's currently 12:42 PM...", iterations: 2 }
```

## Related

- [[../research/ReAct-Paper]] — The academic foundation
- [[Tool-System]] — How tools are registered and dispatched
- [[Context-Compression]] — What happens at step 3
- [[SSE-Streaming]] — What happens at step 5
- [[peko-core]] — Where this loop lives

---

#implementation #react #agent-loop #core
