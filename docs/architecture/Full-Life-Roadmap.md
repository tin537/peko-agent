# Full Life Roadmap

> Path from Digital Life (Tier 2+3) to Advanced Life (Tier 4) — the
> "agent becomes a partner" transition. Six phases, ~20-25 engineering
> hours, each phase ships behind an autonomy config toggle (default off).

See [[Digital-Life]] for the audit that motivates this roadmap.
See [[../implementation/Autonomy]] for detailed module designs.
See [[../implementation/Safety-Model]] for the trust architecture.

---

## Phases at a glance

| Phase | Module | Value | Effort | Score gain |
|-------|--------|-------|--------|------------|
| **A** | `reflector.rs` — automatic post-action self-evaluation | Quality of all future memories | 2-3h | +1 |
| **B** | `life_loop.rs` — idle-time "heartbeat" thinker | Core of autonomous behavior | 4-6h | +2 |
| **C** | `memory.rs` — gardener (pruning + summarization) | Avoid unbounded growth | 2-3h | +1 |
| **D** | `motivation.rs` — drives (curiosity, competence, social, coherence) | Gives B a decision function | 2-3h | +1 |
| **E** | `curiosity.rs` — exploration strategy | Tier 4 curiosity criterion | 3-4h | +1 |
| **F** | `goal.rs` — pattern-driven proactive goals | Tier 4 self-direction | 4-5h | +2 |
| **G** | Web UI "Life" tab | Transparency; trust | 3-4h | - |

**Baseline score:** 21.5/27 (Digital Life)
**Post-A-F score estimate:** 26-27/27 (Advanced Life)

---

## Phase order justification

**A first** — reflection is the foundation everything else learns from. Builds
the proposal/execution plumbing with zero safety risk (only writes to memory).

**D before B** — the life loop's decision function depends on drives.

**B next** — the heartbeat. After this, the agent has its own "time."

**C parallel to B** — memory gardener can ship anytime; easiest if done
right before autonomous writing starts polluting memory.

**E and F last** — the proactive layers. Most safety-sensitive, most value.

**G continuous** — UI updates as each phase lands.

---

## Config surface

One new section controls everything autonomous:

```toml
[autonomy]
enabled = false                    # master switch, default off
tick_interval_secs = 300           # heartbeat period
max_internal_tasks_per_hour = 4    # rate limit
propose_only = true                # queue for user approval; false auto-executes
allowed_tools = [                  # whitelist for autonomous tasks
  "screenshot", "ui_inspect", "memory", "skills",
  "filesystem",                    # read-only ops only in v1
]
memory_gardener_cron = "0 6 * * *" # daily 06:00 local
reflection = true                  # Phase A
curiosity = 0.1                    # 0-1; probability per tick
goal_generation = true             # Phase F
```

With all off (default), Peko behaves exactly as today.

---

## Rollout strategy

Ship each phase behind both a feature flag AND propose-only mode. Users opt-in
gradually:

1. Install with `autonomy.enabled = false` — default, no behavior change
2. Turn on `autonomy.enabled = true` + `propose_only = true` — see what the
   agent would do without it actually doing anything
3. Approve individual proposals from the Life tab — train the user on what
   good proposals look like
4. Optionally flip `propose_only = false` when trust is established
5. Heavy users can widen `allowed_tools` beyond the read-only default

---

## What we explicitly don't ship

To keep scope manageable and safety tight:

- **No action during active user tasks.** Life loop pauses while the task queue
  is non-empty.
- **No destructive default tools.** `shell`, `sms`, `call`, `touch`, `text_input`,
  `package_manager` start outside the autonomous allowlist. User must opt-in per tool.
- **No cross-device coordination.** Every instance runs alone for now.
- **No LLM-choice of drives.** Drives are numeric state updated by code paths,
  not set by LLM output (avoids reward-hacking the motivation model).
