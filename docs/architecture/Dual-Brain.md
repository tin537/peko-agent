# Dual-Brain Architecture

> Route cheap tasks to an on-device model, reserve cloud calls for hard reasoning.
> Local model can escalate to cloud mid-task when it knows it's over its head.

---

## Motivation

A single cloud-only provider (e.g. Claude) is:
- **Latency-bound** — every turn pays a network round-trip
- **Cost-bound** — "open youtube" shouldn't cost the same as "debug my production outage"
- **Offline-fragile** — agent is dead without connectivity

A single local-only model is:
- **Capability-bound** — a 1B model can't plan multi-hour workflows
- **Power-bound** — long prompts blow through battery

**Dual-brain** gets the best of both by routing per-task.

---

## Components

```
┌─ peko-agent (Rust) ─────────────────────────────────────────┐
│                                                             │
│   AgentRuntime                                              │
│     ├─ brain: Option<Arc<DualBrain>>                        │
│     └─ active_brain: Option<BrainChoice>     (per-task)     │
│                                                             │
│   DualBrain                                                 │
│     ├─ local:  Box<dyn LlmProvider>  (small, fast)          │
│     ├─ cloud:  Box<dyn LlmProvider>  (big, smart)           │
│     └─ classify(task, skills) → Local | Cloud               │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

- `peko-core::brain::DualBrain` — holds two providers + routing heuristic
- `peko-core::brain::BrainChoice` — enum { Local, Cloud }
- `peko-core::brain::escalate_tool_schema()` — the `escalate` tool injected when running on local

---

## Task classification

`DualBrain::classify(user_input, skills)` decides by:

1. **Matching skill** — if the user input triggers a skill with ≥60% success rate and ≥2 historical uses → **Local** (the skill provides the playbook)
2. **Simple patterns** — imperative commands like `open youtube`, `tap 100 200`, `take a screenshot`, `wifi on`, `call Alice`, etc. → **Local**
3. **Short + no complex markers** — under 50 chars, no keywords like "explain", "analyze", "write a", "step by step" → **Local**
4. **Default** — **Cloud**

Heuristic lives in `brain::DualBrain::is_simple_task`. Easy to tune — it's a pure function.

---

## Escalation tool

When the active brain is `Local`, the runtime injects an extra tool `escalate` into the schemas sent to the model:

```json
{
  "name": "escalate",
  "description": "Escalate this task to the more powerful cloud AI model. Use this when you realize the task is too complex…",
  "input_schema": {
    "properties": {
      "reason":   { "type": "string", "description": "Why escalating" },
      "analysis": { "type": "string", "description": "What you tried / next steps" }
    },
    "required": ["reason"]
  }
}
```

If the local model calls `escalate`:

1. Runtime intercepts the tool call (doesn't dispatch it to the tool registry)
2. Builds a fresh context message: `[Escalated from local model (qwen3-0.6b)] Reason: … Analysis: …`
3. Clears the conversation and replays it as a fresh user turn to the **cloud** provider
4. Resets the iteration budget so the cloud gets its full allowance
5. Marks `active_brain = Cloud` so subsequent turns stay on cloud

This means the local model can "phone home" without the runtime needing to detect "is this hard?" — the model tells us.

---

## Provider topology

| Provider | What it is | Example `base_url` |
|----------|------------|---------------------|
| `AnthropicProvider` | Anthropic native wire format | `https://api.anthropic.com` |
| `OpenAICompatProvider` | OpenAI Chat Completions over HTTPS/HTTP | `https://openrouter.ai/api/v1`, `http://localhost:11434/v1` |
| `UnixSocketProvider` | OpenAI Chat Completions over Unix Domain Socket | `unix://@peko-llm`, `unix:///tmp/peko.sock` |

The brain router doesn't care which concrete provider is on each side — it just holds two `Box<dyn LlmProvider>` and calls `stream_completion` on the one it picks.

**Typical production config** on Android:
- `local` → `UnixSocketProvider` talking to `peko-llm-daemon` (llama.cpp running Qwen3/Gemma on-device)
- `cloud` → `AnthropicProvider` with Claude Sonnet for escalation

---

## Lifecycle of a task

```
User types "open youtube" in the web UI
   │
   ▼
peko-agent: TaskQueue.submit()
   │
   ▼
AgentRuntime.run_turn(user_input)
   │
   ├─ DualBrain.classify("open youtube", skills)  →  BrainChoice::Local
   │
   ├─ active_brain = Some(Local)
   │
   ▼
Build system prompt + SOUL + memory + skills + compact tool schemas (+ escalate)
   │
   ▼
UnixSocketProvider.stream_completion(…)
   │
   ├─ connect to @peko-llm (libc::socket + AF_UNIX + abstract name)
   ├─ send HTTP POST /v1/chat/completions with body
   ├─ read chunked response, feed bytes through SseParser
   └─ parse_openai_delta → StreamEvent::{TextDelta, ToolUseStart, …}
   │
   ▼
peko-llm-daemon receives POST, calls LlmSession.generate()
   │
   ├─ llama.cpp tokenizes prompt
   ├─ llama_decode() prefill
   ├─ llama_sampler_sample() + llama_decode() loop
   └─ on_token callback → SSE chunk → back to agent
   │
   ▼
Agent executes the tool calls (screenshot, touch, …)
   │
   ▼
Response streamed to web UI via outer /api/run SSE endpoint
```

If at any point the local model calls `escalate`, the runtime restarts the loop
with `active_brain = Cloud` and the next `stream_completion` goes to Anthropic instead.

---

## Configuration

```toml
[provider]
brain = "local:anthropic"        # "<local_name>:<cloud_name>"

[provider.local]
model    = "qwen3"
base_url = "unix://@peko-llm"     # UDS path or abstract name
max_tokens = 512

[provider.anthropic]
model      = "claude-sonnet-4-20250514"
max_tokens = 4096
```

If the cloud side fails to build (e.g. no API key set), the runtime automatically
falls back to using the local side for both brains — the agent still works, escalation
is just a no-op until a key is provided.

---

## Tuning tips

- **Adjust `skill_threshold`** (default 0.6) — lower = more aggressive local routing
- **Adjust `simple_max_len`** (default 200) — shorter cutoff = more escalation
- **Observe logs** — look for `brain: routed to …` to see classifier decisions
- **Add prompt patterns** in `is_simple_task` for your specific task distribution

---

## Files

- `crates/peko-core/src/brain.rs` — DualBrain, BrainChoice, escalate tool schema
- `crates/peko-core/src/runtime.rs` — `run_task` / `run_turn` use brain to pick provider; intercept `escalate` tool calls
- `crates/peko-transport/src/unix_socket.rs` — UnixSocketProvider
- `crates/peko-llm-daemon/` — C++ daemon serving the `unix://` endpoint
