# Safety Model for Autonomous Behavior

> Autonomy without guardrails is how agents embarrass their owners.
> Every autonomous capability ships behind layered safeguards.

---

## The four layers

```
┌─────────────────────────────────────────────────────────────┐
│ Layer 4: Audit & observability                              │
│   - Append-only autonomy.log                                │
│   - Web UI Life tab shows every decision + reasoning        │
│   - Drives + recent actions visible to user                 │
├─────────────────────────────────────────────────────────────┤
│ Layer 3: Kill switch                                        │
│   - POST /api/autonomy/pause                                │
│   - Global flag, checked before every life-loop tick        │
│   - User can also disable via config + restart              │
├─────────────────────────────────────────────────────────────┤
│ Layer 2: Proposal queue (default ON)                        │
│   - autonomy.propose_only = true                            │
│   - No internal task executes without user click            │
│   - Proposals auto-expire after 6 hours                     │
├─────────────────────────────────────────────────────────────┤
│ Layer 1: Master toggle (default OFF)                        │
│   - autonomy.enabled = false in shipped config              │
│   - Without this, none of the above exists                  │
└─────────────────────────────────────────────────────────────┘
```

---

## Tool allowlist for autonomous tasks

When `TaskSource::Internal`, the runtime applies a **whitelist** instead of
the full tool registry:

```toml
[autonomy]
allowed_tools = [
  # Read-only observation
  "screenshot", "ui_inspect",
  # Memory & skills (mutate internal state only)
  "memory", "skills",
  # Filesystem: read-only for autonomous (enforced in filesystem.rs)
  "filesystem",
]
```

Explicitly **blocked by default** for autonomous tasks:
- `shell` — arbitrary command execution
- `sms`, `call` — costs money, sends to real humans
- `touch`, `key_event`, `text_input` — modifies device state
- `package_manager` — installs/uninstalls apps
- `delegate` — would recurse into more autonomous behavior

Users can opt-in per tool via `autonomy.allowed_tools`, but it's a deliberate choice.

---

## Rate limiting

Sliding-window limiter keeps total autonomous output bounded:

```rust
pub struct RateLimiter {
    pub max_per_hour:  u32,
    pub max_per_day:   u32,
    pub window:        VecDeque<Instant>,
}
```

Defaults:
```toml
max_internal_tasks_per_hour = 4
max_internal_tasks_per_day  = 20
```

If the limit is hit, the life loop logs `"rate limited, skipping tick"` and
waits for the window to clear. User sees this in the Life tab with a
cooldown timer.

Token budgets (for LLM cost control):
```toml
max_autonomous_tokens_per_day = 50000
```

Once exceeded, autonomy pauses automatically for the rest of the UTC day.

---

## Audit log

Every autonomous action appends one JSON-per-line record to `autonomy.log`:

```json
{"t":"2026-04-17T09:31:02Z","tick":412,"drives":{"c":0.72,"q":0.61,"s":0.55,"h":0.80},
 "action":"Explore","decision":"tool 'modem' never used","outcome":"proposed",
 "proposal_id":"p_abc123"}
```

Two important properties:
- **Append-only** — the agent cannot rewrite the log (file opened O_APPEND in code)
- **Not deletable by the agent** — outside of `filesystem` tool's allowed paths

Used for:
- UI display
- Forensics if something unexpected happens
- Training signal for `coherence` drive ("are actions consistent?")

---

## Propose vs. execute

Two autonomy modes:

### `propose_only = true` (DEFAULT)

Life loop generates a `Proposal`, stores it in the proposal queue, notifies
the user via Life tab. User clicks Approve → task executes.

Pros:
- Zero surprise actions
- User sees what would happen before it happens
- Trains the user on what good proposals look like

Cons:
- Proposals pile up if user is away
- Not truly "autonomous" in the strictest sense

### `propose_only = false` (advanced users)

Internal tasks execute immediately, subject only to the tool allowlist and
rate limits. All execution is still logged and reversible actions preferred.

Recommended only after running propose-only for a while and confirming the
agent makes good decisions.

---

## Automatic pausing conditions

Life loop auto-pauses itself if any of:

- User actively using the device (recent task queue activity) — just skip ticks
- Battery < 15% — preserve user's battery budget
- Rate limit exceeded — wait for window
- Three consecutive failed proposals — human intervention needed
- SSE connection error rate spikes — something wrong upstream

Resumes automatically when conditions clear, except the "3 failures" case
which requires explicit unpause.

---

## UI affordances

The Life tab (Phase G) must surface:

- **Master on/off switch** (mirror of config)
- **Current drives** (4 progress bars)
- **Pending proposals** (with approve/reject)
- **Recent autonomous actions** (last 20, from audit log)
- **Rate-limit status** (x/4 this hour, y/20 today, tokens z/50000)
- **Pause button** (big, red)

Without these, the user can't know what the agent is doing on their behalf,
and that's the whole point of the safety model.

---

## Things explicitly NOT done

- **No reinforcement learning from user approval/rejection beyond drive tuning.**
  We don't fine-tune the underlying model, just nudge numeric drives. Keeps
  behavior predictable and auditable.

- **No LLM-written drive values.** The motivation model is pure code.
  If the LLM could write its own drives it could "decide" it's very curious
  right now — classic reward hacking.

- **No cross-device coordination.** Each Peko instance is alone. No syncing
  of motivation/drives/goals between your phone and another device yet.

- **No modification of `autonomy.*` config by the agent itself.** Even if
  `filesystem` writes are enabled, the config path is blocked.

- **No silent outbound network activity.** All LLM calls go through the
  standard provider chain and are visible in `/api/brain` + logs.
