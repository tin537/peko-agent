# Autonomy — Technical Design

> Detailed module designs for the six autonomy phases (A-F) described
> in [[../architecture/Full-Life-Roadmap]]. Also see
> [[Safety-Model]] for rate limits, kill switches, and audit log.

---

## Module layout

```
crates/peko-core/src/
├── autonomy.rs        [NEW]  AutonomyConfig + rate limiter + kill switch
├── reflector.rs       [NEW]  Phase A — post-action self-evaluation
├── motivation.rs      [NEW]  Phase D — internal drives
├── life_loop.rs       [NEW]  Phase B — idle heartbeat
├── curiosity.rs       [NEW]  Phase E — exploration strategy
├── goal.rs            [NEW]  Phase F — proactive goal generation
├── memory.rs          [MOD]  Phase C — gardener methods
├── task_queue.rs      [MOD]  TaskSource::Internal variant
└── lib.rs             [MOD]  wire exports
```

All modules live in `peko-core` so `peko-agent` (the binary) can compose them
in `main.rs` without leaking cross-crate concerns.

---

## Phase A — Reflector

**Purpose:** after every completed task, the agent evaluates its own performance
and stores a structured reflection as memory. This is the missing "automatic
evaluator" that completes Tier 3's self-reflection criterion.

### Data model

```rust
pub enum ReflectionOutcome { Went Well, Partial, Failed }

pub struct Reflection {
    pub task_id:       String,         // session_id
    pub user_input:    String,
    pub outcome:       ReflectionOutcome,
    pub what_worked:   String,
    pub what_failed:   String,
    pub lessons:       Vec<String>,    // stored as `MemoryCategory::Reflection`
    pub tool_errors:   Vec<String>,    // tool names that errored in this task
    pub timestamp:     String,         // ISO 8601
}
```

### Trigger

Hook inside `AgentRuntime::run_turn` (and `run_task`) after the final `Done`
callback emission:

```rust
if autonomy.reflection_enabled && !task_was_internal {
    tokio::spawn(async move {
        reflector.reflect(&completed_task).await
    });
}
```

Non-blocking — user gets their response immediately, reflection happens in
the background.

### LLM prompt

Uses the local brain if available (cheap). Input includes the final assistant
text, each tool call + its result, the user's original prompt. Output parsed
as JSON with the struct fields above.

### Storage

Reflections live in `MemoryStore` under new category:
```rust
MemoryCategory::Reflection  // searchable alongside fact/preference/etc.
```

`importance` computed from outcome: Failed=0.8, Partial=0.5, WentWell=0.3.

---

## Phase D — Motivation

**Purpose:** four persistent scalar drives that influence the life loop's
decisions. Drives are **updated by code**, not by LLM output (avoids self-modification
loops).

### State

```rust
pub struct Motivation {
    pub curiosity:  f32,   // 0..1
    pub competence: f32,
    pub social:     f32,
    pub coherence:  f32,
    pub updated_at: DateTime<Utc>,
}
```

Persisted as `motivation.json` next to `user_model.json`.

### Update rules

| Event | Drive delta |
|-------|-------------|
| Task completed successfully | competence +0.05 |
| Task failed / escalated | competence -0.05 |
| User thanks / keeps session / replies positively | social +0.05 |
| User ignores / deletes proposed task | social -0.05 |
| New tool used for first time | curiosity -0.1 (fulfilled) |
| N ticks with no new tools/apps/contexts | curiosity +0.02/tick |
| SOUL.md matches observed behavior | coherence +0.02 |
| User correction / escalation | coherence -0.05 |
| Passive tick (baseline decay) | all drives → 0.5 at 1%/hour |

All updates clamped to [0, 1].

### Use

Life loop queries `Motivation::suggest_action()`:
```rust
pub fn suggest_action(&self) -> Option<LifeAction> {
    if self.curiosity > 0.7 { Some(LifeAction::Explore) }
    else if self.competence < 0.3 { Some(LifeAction::ReviewFailures) }
    else if self.social > 0.7 { Some(LifeAction::ProposeHelpful) }
    else { None }  // noop this tick
}
```

---

## Phase B — Life Loop

**Purpose:** the background "heartbeat" — the thing that makes Peko feel alive
when no one is talking to it.

### Architecture

Separate `tokio::spawn`-ed task, started at boot if `autonomy.enabled`:

```rust
pub struct LifeLoop {
    config:    AutonomyConfig,
    motivation: Arc<Mutex<Motivation>>,
    memory:    Arc<Mutex<MemoryStore>>,
    user:      Arc<Mutex<UserModel>>,
    queue:     TaskQueue,
    rate_limiter: RateLimiter,
}

impl LifeLoop {
    pub async fn run(self) {
        loop {
            tokio::time::sleep(self.config.tick_interval).await;
            if !self.can_run().await { continue; }   // queue busy, rate limit, paused
            self.tick().await;
        }
    }

    async fn tick(&self) {
        // 1. baseline decay of drives
        self.motivation.lock().await.decay();

        // 2. what should we do this tick?
        let Some(action) = self.motivation.lock().await.suggest_action() else {
            return;
        };

        // 3. dispatch
        let task_prompt = match action {
            LifeAction::Explore          => curiosity.next_exploration(…),
            LifeAction::ReviewFailures   => reflector.pick_failure_to_revisit(…),
            LifeAction::ProposeHelpful   => goal.top_pattern(…),
        };

        // 4. submit as TaskSource::Internal
        if let Some(prompt) = task_prompt {
            self.rate_limiter.consume();
            self.queue.submit_internal(prompt, action).await;
        }
    }

    async fn can_run(&self) -> bool {
        !self.is_paused
            && self.rate_limiter.has_budget()
            && self.queue.is_idle().await
    }
}
```

### Idle detection

`TaskQueue::is_idle()` returns true when there are no user-sourced tasks in the
queue and none running. Internal tasks don't block this — but the life loop
shouldn't submit more if one is already running.

### TaskSource

Extend existing enum:
```rust
pub enum TaskSource {
    WebUI,
    Telegram { chat_id: i64 },
    Scheduler { task_name: String },
    Api,
    Internal { action: LifeAction, reason: String },  // NEW
}
```

Internal tasks are tagged so UI/logs/audit can identify them and apply the
autonomous tool allowlist.

---

## Phase C — Memory Gardener

**Purpose:** keep memory store from growing unbounded; preserve important
patterns; summarize clusters.

### Operations

```rust
impl MemoryStore {
    /// Decay importance of unaccessed memories by age.
    /// Delete items with importance < 0.1 AND access_count == 0 AND age > 30d.
    pub fn prune(&self) -> PruneReport;

    /// Cluster by FTS5 similarity; for clusters ≥ 5, ask LLM to synthesize
    /// a single replacement memory + delete the originals.
    pub async fn summarize_clusters(&self, llm: &dyn LlmProvider) -> usize;
}

pub struct PruneReport {
    pub deleted_count: usize,
    pub summarized_count: usize,
    pub remaining_count: usize,
}
```

### Scheduling

Runs daily via the existing `Scheduler` — a new built-in task
`"__gardener__"` with cron `0 6 * * *` (6am local). Completed gardener runs
emit one summary memory (`"pruned 47 old observations, kept 12"`).

---

## Phase E — Curiosity

**Purpose:** when `motivation.curiosity > 0.7`, generate an exploration task.

### Coverage tracking

New fields on `UserModel` (already exists):

```rust
pub struct UserPatterns {
    // existing…
    pub tools_used:       HashMap<String, u32>,   // per-tool count
    pub apps_observed:    HashSet<String>,
    pub active_hours:     HashSet<u8>,            // 0..23
    pub last_exploration: Option<DateTime<Utc>>,
}
```

Updated in `AgentRuntime` after each tool execution (tools_used++) and when
apps appear in screenshots or package_manager output.

### Exploration strategy

```rust
impl Curiosity {
    pub fn next(&self, user: &UserModel) -> Option<String> {
        // 1. Any tool never used? → "Try using {tool}: <sample task>"
        // 2. Unusual time of day? → "Observe state now; this is rare for you"
        // 3. An app seen but never interacted → "Open {app} and explore briefly"
        None  // nothing to explore
    }
}
```

All exploration tasks are read-only (screenshot, ui_inspect, filesystem read).

---

## Phase F — Goal generator

**Purpose:** detect patterns in user behavior + memory, propose helpful tasks.

### Pattern detectors

```rust
pub struct Pattern {
    pub kind:        PatternKind,
    pub confidence:  f32,    // 0..1
    pub suggestion:  String, // natural-language task
}

pub enum PatternKind {
    RecurringTime,        // "User opens calendar every 8am"
    RepeatedFailure,      // "WiFi dropped 5 times this week"
    NeglectedMessage,     // "SMS from Alice unread for 3 days"
    BatteryPattern,       // "Battery < 20% by 6pm most days"
    UnreadNotification,   // "5 notifications unread from critical apps"
}
```

Each detector is a pure function over `(MemoryStore, UserModel)`. Runs in the
life loop when `motivation.social > 0.7`.

### Ranking & proposal

```rust
fn rank(patterns: Vec<Pattern>, drives: &Motivation, user: &UserModel) -> Vec<Pattern> {
    // Score = confidence × user_acceptance_history × drive_alignment
}
```

Top pattern → submitted as internal task with the pattern as the
reasoning attached.

---

## Proposal queue & approval flow

When `propose_only = true`, internal tasks go into a **Proposal Queue** instead
of executing:

```rust
pub struct Proposal {
    pub id:          Uuid,
    pub action:      LifeAction,
    pub task_prompt: String,
    pub reasoning:   String,
    pub created_at:  DateTime<Utc>,
    pub status:      ProposalStatus,
}

pub enum ProposalStatus { Pending, Approved, Rejected, Expired }
```

- UI shows pending proposals in the Life tab
- User clicks approve → task executes with `Internal` source
- Reject → `social -= 0.05` (drive tuning)
- Auto-expire after 6 hours

---

## Integration points

### `AgentRuntime`

- After `Done` callback: spawn `Reflector` (Phase A)
- After each tool execution: bump `UserModel.patterns.tools_used` (Phase E)

### `TaskQueue`

- New `TaskSource::Internal` variant
- `submit_internal()` helper
- `is_idle()` predicate (used by life loop gate)
- Internal tasks get the autonomous tool allowlist filter

### `main.rs`

- Parse `[autonomy]` config section
- Construct `LifeLoop` + `Motivation` + `Reflector` + `Curiosity` + `Goal`
- Spawn life loop task when `autonomy.enabled`
- Wire endpoints for life UI

### Web API

- `GET /api/autonomy/status` → `{ enabled, paused, drives, recent_actions }`
- `GET /api/autonomy/proposals` → pending proposals
- `POST /api/autonomy/proposals/{id}/approve`
- `POST /api/autonomy/proposals/{id}/reject`
- `POST /api/autonomy/pause` → kill switch
- `POST /api/autonomy/resume`

---

## Testing strategy

### Unit tests (fast, deterministic)

- `Motivation::suggest_action` under synthetic drive vectors
- `Reflection` JSON parsing edge cases
- `Curiosity` unexplored detection (seed `UserModel`, assert output)
- Rate limiter sliding window
- `Proposal` lifecycle (pending → approved → executed)

### Integration tests (in-process)

- Seed memory store → run life loop once → assert proposal generated
- Feed reflector synthetic task outcomes → assert correct memory categories
- Verify `propose_only = false` path actually executes tasks

### Live test on device

- Enable autonomy with `propose_only = true`
- Leave idle 30 minutes
- Check life log; confirm reasonable proposals
- Approve one; confirm execution via life UI
