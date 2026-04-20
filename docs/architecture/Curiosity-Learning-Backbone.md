# Curiosity Learning Backbone

> **Status:** design; parts (1)–(3) cheap to land first, (4)–(6) build on
> them. Supersedes the manual-threshold exploration in `life_loop.rs` while
> keeping its rate/token guardrails intact.

## Problem

Today peko "explores" by walking a hard-coded candidate list
(`curiosity.rs` proposes unused tools, unusual-hour prompts) and gating
execution on token + rate budgets (`life_loop.rs::438`). It has the
*governance* of an autonomous agent but none of the *learning* — no notion
of novelty, surprise, prediction error, or expected information gain, so
repeated exploration produces no better choices over time. This doc
specifies a small learning backbone that plugs into the existing Life Loop
without rearchitecting it.

The six pieces, in dependency order:

1. **Surprise metric** — prediction error between expected and observed outcome
2. **Novelty metric** — rarity of tool + state-signature
3. **Risk score** — reversibility / cost / blast radius per action
4. **Exploration policy** — softmax over `gain·novelty − risk`, temperature tied to drives
5. **World model** — cheap LLM-cached `(state, action) → predicted next state`
6. **Intrinsic reward** — `r = α·surprise + β·novelty + γ·info_gain`, replaces hard-coded `motivation.record` deltas

## Non-goals

- No per-token neural world model. Keep predictions at the *outcome-class +
  observation-bullet* level; anything finer is too expensive on-device.
- No reinforcement-learning training loop. Policy is bandit-style
  (softmax-weighted selection with updated statistics), not PPO / DQN.
- No model of other humans. World = device state + peko's effect on it.

## Data model

One new table, one column on `memories`. Everything else is additive.

```sql
-- Stored under calls.db would be wrong scope; lives in state.db next to
-- sessions. Rationale: these events are agent-side, not user-facing logs,
-- and already live next to the runtime that writes them.
CREATE TABLE learning_events (
    id              TEXT PRIMARY KEY,
    ts              TEXT NOT NULL,
    action_kind     TEXT NOT NULL,        -- tool name or "curiosity_proposal"
    action_args_hash TEXT,                 -- stable hash of tool input for dedup
    state_sig       TEXT,                  -- hash of (pkg, activity, screen_sig)
    predicted       TEXT,                  -- JSON: {outcome, observations, cost}
    observed        TEXT,                  -- JSON: actual {outcome, observations, cost}
    surprise        REAL NOT NULL DEFAULT 0,  -- [0, 1]
    novelty         REAL NOT NULL DEFAULT 0,  -- [0, 1]
    info_gain       REAL NOT NULL DEFAULT 0,  -- [0, 1]
    risk            REAL NOT NULL DEFAULT 0,  -- [0, 1]
    intrinsic_r     REAL NOT NULL DEFAULT 0   -- α·surprise + β·novelty + γ·gain
);
CREATE INDEX idx_lev_ts ON learning_events(ts DESC);
CREATE INDEX idx_lev_action ON learning_events(action_kind);
CREATE INDEX idx_lev_state  ON learning_events(state_sig);

-- Extend existing memories table (migration in memory.rs::init_schema)
ALTER TABLE memories ADD COLUMN curiosity_metrics TEXT;   -- optional JSON
```

The `learning_events` table is the per-event scorecard called for in
checklist item 9. It's append-only and gardener-pruneable.

`curiosity_metrics` on memories is optional context on Observation /
Reflection rows so the policy can ask "show me memories tagged by high
surprise" at proposal time.

## Components

### 1. Surprise

**Purpose:** give every executed action a prediction-error score.

**Signal:** compare a *pre-execution prediction* to the *post-execution
observation*. Both coarse.

```rust
// crates/peko-core/src/world_model.rs
pub struct Prediction {
    pub outcome: Outcome,         // went_well | partial | failed
    pub observations: Vec<String>,// 3 short bullets of expected end-state
    pub cost_tokens: u32,          // expected LLM token cost
}
pub enum Outcome { WentWell, Partial, Failed }
```

The Reflector already classifies post-hoc outcome; we add a *pre-hoc*
prediction via the cheap brain side of `DualBrain`. For non-LLM tools
(screenshot, shell), use the empirical distribution from past
`learning_events` of the same `action_kind + args_hash` — no LLM call needed.

**Scoring:**

```
surprise = 0.4 * outcome_mismatch            // 1 if predicted wrong bucket
         + 0.4 * (1 - embedding_cos(predicted_obs, observed_obs))
         + 0.2 * min(1, |cost_err| / expected_cost)
```

Clamp to `[0, 1]`. If we skipped prediction (cold-start, no history),
surprise is `0.5` (maximum prior uncertainty).

**Cost note.** Pre-hoc prediction adds 1 cheap-brain call per executed
action. For autonomy-internal actions that's fine; for user-triggered
tasks we only predict when the Life Loop is driving the task (not every
chat turn) so we don't double LLM spend.

### 2. Novelty

**Purpose:** quantify rarity; already half-done via `UserPatterns.tools_used`.

**Signal:** two counters, combined.

```rust
// crates/peko-core/src/curiosity.rs (extend)
fn novelty(tool: &str, state_sig: &str, stats: &LearningStats) -> f32 {
    let t = stats.tool_count(tool).max(1);
    let s = stats.state_count(state_sig).max(1);
    let tool_novelty  = 1.0 / (1.0 + (t as f32).ln());
    let state_novelty = 1.0 / (1.0 + (s as f32).ln());
    0.6 * tool_novelty + 0.4 * state_novelty
}
```

`state_sig` is `sha1(package + top_activity + visible_text_bucket)`
(already exposed by `ui_inspect`). Cached per tick so one tick's proposals
all see the same state.

### 3. Risk

**Purpose:** make the policy cost-aware.

**Signal:** three tool-level attributes combined at rank time.

```rust
// crates/peko-core/src/tool.rs (extend Tool trait)
pub struct ActionCost {
    pub reversible: bool,
    pub blast_radius: Blast,   // Self | Device | External
    pub estimated_tokens: u32,
}
pub enum Blast { Self_, Device, External }

pub trait Tool {
    // existing methods…
    fn cost(&self, args: &Value) -> ActionCost {
        ActionCost { reversible: true, blast_radius: Blast::Self_, estimated_tokens: 500 }
    }
}
```

Combining:

```
risk = 0.3 * (is_dangerous as f32)
     + 0.3 * (1 - reversible as f32)
     + 0.2 * blast_weight(blast_radius)   // Self=0, Device=0.5, External=1
     + 0.2 * min(1, estimated_tokens / 10_000)
```

Existing `is_dangerous()` stays as an override veto: risk > 0.8 *and*
`is_dangerous` blocks the action regardless of reward. This is the
hard safety floor that exists today; we just expose the continuous
score alongside.

### 4. Exploration policy

**Purpose:** replace `if curiosity > 0.70 { Explore }` with a continuous
decision that scales explore/exploit with the drive state.

**Signal:** softmax over candidates, temperature = function of curiosity.

```rust
// crates/peko-core/src/life_loop.rs (new helper)
fn select_proposal(cands: &[Candidate], drives: &Motivation) -> Option<&Candidate> {
    let t = (0.5 + drives.curiosity()).clamp(0.3, 2.0);  // explore temp
    let scores: Vec<f32> = cands.iter().map(|c| {
        c.w_gain * c.expected_gain + c.w_nov * c.novelty - c.w_risk * c.risk
    }).collect();
    softmax_sample(&scores, t)
}
```

Weights live in `[autonomy.learning]`; defaults `w_gain=0.5, w_nov=0.3, w_risk=0.4`.

Exploit mode (low curiosity drive) collapses to picking the best score;
explore mode (high curiosity, after stale-tick surge) flattens the
distribution so lower-scored but more novel actions get sampled.

### 5. World model

**Purpose:** supply the pre-hoc `Prediction` that surprise measures against.

**Signal:** keyed LLM cache. Cheap brain predicts; cache survives 24 h.

```rust
// crates/peko-core/src/world_model.rs
pub struct WorldModel {
    store: Arc<Mutex<MemoryStore>>,     // reuse FTS5 for retrieval
    provider: Arc<dyn LlmProvider>,     // cheap side of DualBrain
}
impl WorldModel {
    pub async fn predict(&self, state_sig: &str, action: &Action) -> Prediction {
        // 1. Hit cache (key = (state_sig, action.hash())) in MemoryStore
        //    with category=Prediction. Return if < 24h old.
        // 2. Otherwise: prompt "given state X, predict next state after
        //    running tool Y with args Z. One-line outcome (went_well|
        //    partial|failed), three observation bullets, a token count."
        // 3. Parse + cache under the same key.
    }
}
```

The model is deliberately text-level. An Android state = "top activity
is com.android.settings, WiFi screen visible, three toggles". A
predicted state = "WiFi toggled off, title bar says Off". Embedding
cosine between prediction and actual observation yields the surprise
numerator. Good enough.

### 6. Intrinsic reward

**Purpose:** make motivation drives move in response to *learning*, not
per-event constants.

**Signal:** replace `motivation.record(DriveEvent::…)` constant-delta
table with a computed scalar.

```rust
// crates/peko-core/src/motivation.rs (extend)
pub struct LearningSignal {
    pub surprise: f32,   // 0..1, from world model vs observation
    pub novelty:  f32,   // 0..1, from curiosity::novelty
    pub info_gain: f32,  // 0..1, from memory.new_facts / memory.size
}
impl Motivation {
    pub fn record_learning(&mut self, sig: LearningSignal) {
        let r = 0.5 * sig.surprise + 0.3 * sig.novelty + 0.2 * sig.info_gain;
        // High surprise → curiosity drops (something was learned).
        // High novelty  → competence climbs if surprise was low (mastered a new tool).
        // High gain     → coherence climbs (model improved).
        self.adjust("curiosity",  -0.5 * sig.surprise);
        self.adjust("competence",  0.4 * (1.0 - sig.surprise) * sig.novelty);
        self.adjust("coherence",   0.3 * sig.info_gain);
        self.last_intrinsic_r = r;
    }
}
```

`info_gain` is cheap to compute: number of *new* memories saved during or
after the action, divided by memory count before the tick. Not a real
information-theoretic measure but a working proxy — reflects how much of
the memory store was touched.

Existing `DriveEvent::*` variants (`TaskSucceeded`, `UserRejected`, …)
stay — they handle social/coherence-affecting events that don't fit the
surprise/novelty/gain triple. `record_learning()` is an additional
channel, not a replacement.

## Data flow per tick

```
LifeLoop::tick()
 ├── build candidates: curiosity::candidates() + goal::patterns() + skills::stale()
 ├── for each candidate c:
 │    p = world_model.predict(state_sig, c.action)   // pre-hoc
 │    c.novelty = novelty(c.tool, state_sig, stats)
 │    c.risk    = combine_risk(tool.cost(args), tool.is_dangerous())
 │    c.expected_gain = empirical_mean_info_gain(c.action_kind)
 ├── softmax-select one (or zero, if budget tapped out)
 ├── enqueue as Proposal / auto-execute per propose_only
 └── after execution:
      observed = collect_state_and_outcome()
      surprise = score(p, observed)
      info_gain = memory.new_since(tick_start) / memory.size
      append learning_events row
      motivation.record_learning(LearningSignal { surprise, novelty, info_gain })
      world_model.cache((state_sig, action), observed)
```

All new work is bounded by the existing `max_tokens_per_day` +
`max_internal_tasks_per_hour` budget (checklist item 12 is already
green), so the policy can't runaway-spend itself.

## Config additions

```toml
[autonomy.learning]
enabled            = true              # feature flag — default false in first release
world_model        = true              # off → predictions are empirical-only
prediction_ttl_hrs = 24
w_gain             = 0.5
w_novelty          = 0.3
w_risk             = 0.4
reward_alpha       = 0.5               # surprise weight
reward_beta        = 0.3               # novelty weight
reward_gamma       = 0.2               # info_gain weight
min_surprise_skip  = 0.05              # don't update drives for near-zero surprise
```

Hot-reloaded the same way `[calls]` is (`crates/peko-core/src/call_pipeline.rs`
already shows the pattern).

## Rollout order

1. **Surprise + logging (week 1)** — add `learning_events` table, wire a
   post-hoc observation collector, stub prediction with empirical mean
   outcome from past rows. No UI changes. Confidence: high.
2. **Novelty counters (week 1)** — extend `UserPatterns` with a
   `state_counts: LruMap<StateSig, i64>` alongside `tools_used`. Expose
   `novelty(tool, state)`. Confidence: high.
3. **Risk score (week 1)** — land the `Tool::cost()` default + per-tool
   overrides in `peko-tools-android`. Confidence: high.
4. **Policy swap (week 2)** — replace the `curiosity > 0.70` gate in
   `life_loop.rs` with `select_proposal`; gate behind
   `autonomy.learning.enabled`. Confidence: medium — want to A/B against
   current behaviour before defaulting on.
5. **World model (week 2)** — add `WorldModel`; seed prediction cache
   opportunistically on reflections. Confidence: medium — LLM prompt
   needs tuning.
6. **Intrinsic reward (week 3)** — flip `motivation` updates to
   `record_learning` for actions that have a `LearningSignal`; leave the
   old `record(DriveEvent)` channel for social/coherence events.
   Confidence: medium — weights need real-world tuning.

## Evaluation

Three metrics to watch over a week of autonomous operation:

- **Surprise trend.** Running median of `surprise` per tick should drop
  over time for the same `state_sig × action_kind` pairs. If it doesn't,
  the world model isn't learning.
- **Novelty spread.** Distribution of proposed tools should widen week
  over week (policy correctly pushing into under-explored regions).
- **Drive stability.** `curiosity` drive should oscillate (settle → spike
  after stale ticks → settle again), not pin at 1.0.

All three queryable from the `learning_events` table with plain SQL; an
`/api/autonomy/learning` endpoint + a "Learning" sub-panel in the Life
tab covers the UI side.

## Open questions

- **Cache scope for the world model.** Per-user or per-device? LineageOS
  is single-user in practice, but multi-SIM or a shared family device
  breaks the assumption. Leave as per-device for v1.
- **Embedding source.** Local sentence-transformer via candle (new
  dependency) or LLM-as-embedder (cheap brain with an "embedding" prompt)?
  The latter is uglier but needs no extra binary weight. Lean to LLM-as-
  embedder for v1, switch if cost becomes a problem.
- **When surprise = 0 repeatedly.** That's a sign peko has *solved* the
  environment and should step back — tick interval should stretch out,
  token budget should drop. Revisit after a month of data.
