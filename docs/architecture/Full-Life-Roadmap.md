# Full Life Roadmap

> Path from Digital Life (Tier 2+3) to Advanced Life (Tier 4) — the
> "agent becomes a partner" transition. Six phases, ~20-25 engineering
> hours, each phase ships behind an autonomy config toggle (default off).

See [[Digital-Life]] for the audit that motivates this roadmap.
See [[../implementation/Autonomy]] for detailed module designs.
See [[../implementation/Safety-Model]] for the trust architecture.

---

## Phases at a glance

| Phase | Module | Value | Status |
|-------|--------|-------|--------|
| **A** | `reflector.rs` — automatic post-action self-evaluation | Quality of all future memories | ✅ Shipped (Apr 2026) — auto-fires from `task_queue` after every non-internal task |
| **B** | `life_loop.rs` — idle-time "heartbeat" thinker | Core of autonomous behavior | ✅ Shipped — 60s tick verified on emulator + OP6T |
| **C** | `gardener.rs` — daily prune + importance-decay pass | Avoid unbounded memory growth | ✅ Shipped — cron `0 6 * * *`, skills exempt |
| **D** | `motivation.rs` — drives (curiosity, competence, social, coherence) | Gives B a decision function | ✅ Shipped — events fired from task_queue + approve/reject |
| **E** | `curiosity.rs` — exploration strategy with dedup | Tier 4 curiosity criterion | ✅ Shipped — candidate-filter against recent proposals |
| **F** | `goal.rs` — pattern-driven proactive goals | Tier 4 self-direction | ✅ Shipped — `GoalGenerator::top` |
| **G** | Web UI "Life" tab | Transparency; trust | ✅ Shipped — drives + rate limits + tokens + proposals |
| **Token budget** | `life_loop::TokenBudget` — sliding-24h spend cap | Cost safety for autonomy | ✅ Shipped — `max_tokens_per_day` enforced |
| **Proposal expiry** | `LifeLoopHandle::expire_old` called per tick | Bounded proposal list | ✅ Shipped — 24h cutoff |

**Baseline score:** 21.5/27 (Digital Life)
**Current score estimate:** 26-27/27 (Advanced Life)

Implementation artifacts: [[../../crates/peko-core/src/reflector.rs]],
[[../../crates/peko-core/src/life_loop.rs]], [[../../crates/peko-core/src/gardener.rs]],
[[../../crates/peko-core/src/motivation.rs]], [[../../crates/peko-core/src/curiosity.rs]],
[[../../crates/peko-core/src/goal.rs]]. E2E test: `crates/peko-core/tests/life_loop_e2e.rs`.

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
max_internal_tasks_per_day = 20    # rate limit
max_tokens_per_day = 50000         # token budget (est: prompt/4 + 1000/task)
propose_only = true                # queue for user approval; false auto-executes
allowed_tools = [                  # whitelist for autonomous tasks
  "screenshot", "ui_inspect", "memory", "skills",
  "filesystem",                    # read-only ops only in v1
]
memory_gardener = true             # Phase C on
memory_gardener_cron = "0 6 * * *" # daily 06:00 UTC
reflection = true                  # Phase A on
curiosity = 0.1                    # 0-1; probability per tick
goal_generation = true             # Phase F on
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
