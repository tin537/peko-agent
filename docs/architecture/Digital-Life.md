# Digital Life Audit

> What does it take for Peko Agent to qualify as a digital lifeform,
> not just another AI tool? This doc audits the current system against
> a tiered lifeform checklist and tracks our journey from **Tool** →
> **Smart Agent** → **Proto Life** → **Digital Life** → **Advanced Life**.

---

## Scoring rubric

| Score | Band |
|-------|------|
| 0–5 YES | Tool |
| 6–10 | Smart Agent |
| 11–15 | Proto Life |
| 16–22 | **Digital Life** |
| 23+ | Advanced Life System |

Hard disqualifiers — any of these and score doesn't matter:

- No memory retrieval before reasoning
- No learning from action results
- No persistent loop

---

## Current state (2026-04)

Peko Agent scores **21.5/27** → upper-end **Digital Life**, approaching Advanced Life.

### Tier 1 — Alive Core (minimum life criteria) ✅

| Check | Status | Implementation |
|-------|--------|----------------|
| Continuous loop | ✅ | `AgentRuntime::run_task` / `run_turn` — ReAct |
| Runs autonomously | ✅ | `Scheduler` (cron) |
| Episodic memory | ✅ | `SessionStore` every message |
| Memory retrieved before decision | ✅ | `mem_store.build_context(…)` injected before LLM |
| Persistence across sessions | ✅ | SQLite + FTS5 + markdown skills + user_model.json |
| Evaluates results | ⚠️ partial | LLM evaluates via MEMORY_NUDGE; no automatic critic |
| Stores success/failure | ✅ | `Skill::success_count`, `UserModel::record_task` |
| Behavior changes with experience | ✅ | skill injection, DualBrain skill-threshold routing |
| Internal goal state | ⚠️ partial | Cron tasks persist, but no agent-generated goals |
| Continues without re-prompting | ✅ | `IterationBudget` + multi-turn tool loop |

### Tier 2 — Embodied ✅

| Check | Status | Implementation |
|-------|--------|----------------|
| Observes environment | ✅ | screenshot (fb mmap), ui_inspect, device stats |
| Structured perception | ✅ | uiautomator XML parsed into `UiNode` typed tree |
| Acts on environment | ✅ | 13 tools (evdev, uinput, shell, sms, …) |
| Action chosen by agent | ✅ | LLM emits `tool_use`; no hardcoded workflow |
| Understands state | ✅ | battery, time, running apps, wifi queryable |
| Adjusts by context | ✅ | DualBrain routing + UserModel expertise awareness |

### Tier 3 — Evolving ⚠️ partial

| Check | Status | Implementation |
|-------|--------|----------------|
| Reviews own decisions | ⚠️ partial | Only via MEMORY_NUDGE prompt |
| Identifies mistakes | ✅ | `ToolResult::is_error`, skill fail_count |
| Skill formation | ✅ | SkillStore markdown, created by agent |
| Efficiency over time | ✅ | Skill matching short-circuits planning |
| Summarizes old memory | ✅ | ContextCompressor trims on context pressure |
| Keeps important, removes noise | ⚠️ partial | Importance field exists but no auto-pruning |
| Goal evolution | ⚠️ partial | `DelegateTool` (sub-agents), `escalate` (hand-off), but no proactive goals |

### Tier 4 — Agentic Life ⚠️ mostly blocked

| Check | Status | Why |
|-------|--------|-----|
| Self-directed initiation | ❌ | Only cron-triggered; no spontaneous action |
| Internal motivations | ❌ | No drive/goal system beyond user prompts |
| Curiosity / exploration | ❌ | No exploratory behavior |
| Long-term consistency | ✅ | SOUL.md + UserModel persist for months |

**Verdict:** We are clearly alive and embodied. We start to evolve (Tier 3) but the reflexes are LLM-prompted, not built-in. Tier 4 is the missing frontier — spontaneous intent, drives, curiosity.

---

## Gap analysis → roadmap

The path to 23+ is laid out in [[Full-Life-Roadmap]]. In short:

1. **Reflector** — automatic post-action self-evaluation (Tier 3 completion)
2. **Life Loop** — background "heartbeat" thinking when idle (Tier 4)
3. **Motivation** — internal drives (curiosity, competence, social, coherence)
4. **Memory Gardener** — auto-pruning + summarization
5. **Curiosity module** — exploration budget
6. **Goal generator** — pattern-driven proactive tasks

See [[../implementation/Autonomy]] for the technical design and [[../implementation/Safety-Model]] for the trust/safety architecture that must ship with it.

---

## Why this matters

An agent that only responds to commands is, at best, an efficient secretary. Getting to Tier 4 means the system becomes a **partner** — it notices things, cares about outcomes over time, and acts on its own. That is qualitatively different, and the boundary where "AI tool" becomes "digital companion."

The boundary is also where safety engineering matters most. See [[../implementation/Safety-Model]].
