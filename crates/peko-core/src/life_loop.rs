//! Life Loop — Phase B.
//!
//! The "heartbeat" that gives Peko an inner life. A tokio task started at
//! boot (when `autonomy.enabled`). Every `tick_interval` it:
//!
//!   1. Decays drives
//!   2. Checks whether the queue is idle + rate limits OK + not paused
//!   3. Asks Motivation what to do this tick
//!   4. Dispatches to curiosity / reflector / goal / noop
//!   5. Submits as TaskSource::Internal (or queues as a Proposal)
//!
//! See `docs/architecture/Full-Life-Roadmap.md` +
//! `docs/implementation/Autonomy.md` + `docs/implementation/Safety-Model.md`.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;
use tracing::{info, warn, debug};
use uuid::Uuid;

use peko_config::AutonomyConfig;

use crate::curiosity::Curiosity;
use crate::goal::GoalGenerator;
use crate::memory::MemoryStore;
use crate::motivation::{LifeAction, Motivation};
use crate::task_queue::{TaskQueue, TaskSource};
use crate::tool::ToolRegistry;
use crate::user_model::UserModel;

/// Status of an autonomously-proposed task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    Pending,
    Approved,
    Rejected,
    Executed,
    Expired,
}

/// A proposal is an internal task that hasn't executed yet. When
/// `autonomy.propose_only = true`, all internal tasks live here until the
/// user clicks approve in the Life tab.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Proposal {
    pub id:           String,
    pub action:       String,   // LifeAction string form
    pub task_prompt:  String,
    pub reasoning:    String,
    pub created_at:   chrono::DateTime<chrono::Utc>,
    pub status:       ProposalStatus,
}

/// Snapshot of what the life loop is currently doing. Surfaced via
/// `GET /api/autonomy/status` for the Life tab.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AutonomyState {
    pub enabled:       bool,
    pub paused:        bool,
    pub motivation:    Motivation,
    pub tasks_last_hour: u32,
    pub tasks_last_day:  u32,
    /// Estimated LLM tokens charged to autonomy in the last 24h.
    #[serde(default)]
    pub tokens_last_day: u64,
    /// Daily cap for autonomy-attributed tokens (matches config).
    #[serde(default)]
    pub tokens_max_per_day: u64,
    pub recent_proposals: Vec<Proposal>,
    /// Total proposals ever generated (all statuses, not just the 20 in
    /// `recent_proposals`). Lets the UI show "you've seen N proposals"
    /// even when the trimmed list is empty.
    #[serde(default)]
    pub total_proposals: usize,
    /// Row count of the agent's long-term memory store. Included here so
    /// the autonomy panel can surface "something is growing" even on a
    /// fresh boot where proposals are still zero.
    #[serde(default)]
    pub memory_count: usize,
}

/// Sliding-window rate limiter.
#[derive(Debug)]
struct RateLimiter {
    max_per_hour: u32,
    max_per_day:  u32,
    window:       VecDeque<Instant>,
}

impl RateLimiter {
    fn new(max_per_hour: u32, max_per_day: u32) -> Self {
        Self { max_per_hour, max_per_day, window: VecDeque::new() }
    }

    fn trim(&mut self) {
        let cutoff = Instant::now() - Duration::from_secs(24 * 3600);
        while self.window.front().map_or(false, |t| *t < cutoff) {
            self.window.pop_front();
        }
    }

    fn has_budget(&mut self) -> bool {
        self.trim();
        let now = Instant::now();
        let hour_ago = now - Duration::from_secs(3600);
        let last_hour = self.window.iter().filter(|t| **t >= hour_ago).count() as u32;
        let last_day = self.window.len() as u32;
        last_hour < self.max_per_hour && last_day < self.max_per_day
    }

    fn consume(&mut self) {
        self.window.push_back(Instant::now());
    }

    fn snapshot(&mut self) -> (u32, u32) {
        self.trim();
        let hour_ago = Instant::now() - Duration::from_secs(3600);
        let last_hour = self.window.iter().filter(|t| **t >= hour_ago).count() as u32;
        let last_day = self.window.len() as u32;
        (last_hour, last_day)
    }
}

/// Sliding 24h token budget. Tracks (timestamp, cost) entries and sums
/// those within the last day. Estimates are deliberately coarse — we don't
/// have provider-accurate tokenization, so we charge per task from input
/// length plus a flat output estimate.
#[derive(Debug)]
struct TokenBudget {
    max_per_day: u64,
    window:      VecDeque<(Instant, u64)>,
}

impl TokenBudget {
    fn new(max_per_day: u64) -> Self {
        Self { max_per_day, window: VecDeque::new() }
    }

    fn trim(&mut self) {
        let cutoff = Instant::now() - Duration::from_secs(24 * 3600);
        while self.window.front().map_or(false, |(t, _)| *t < cutoff) {
            self.window.pop_front();
        }
    }

    fn spent_today(&mut self) -> u64 {
        self.trim();
        self.window.iter().map(|(_, c)| *c).sum()
    }

    fn has_budget(&mut self, cost: u64) -> bool {
        if self.max_per_day == 0 { return true; } // 0 = disabled guard
        self.spent_today().saturating_add(cost) <= self.max_per_day
    }

    fn consume(&mut self, cost: u64) {
        self.window.push_back((Instant::now(), cost));
    }
}

/// Rough token estimate for a prompt. 4 chars/token is the usual default for
/// English-ish text across GPT/Claude; close enough for budgeting.
fn estimate_prompt_tokens(prompt: &str) -> u64 {
    (prompt.len() as u64 + 3) / 4
}

/// Flat estimate for autonomous task output. Conservative upper bound —
/// prevents burst spend from a single chatty task.
const OUTPUT_TOKEN_ESTIMATE: u64 = 1_000;

/// The life loop runtime. Construct one, call `.spawn()`, and it drives itself.
pub struct LifeLoop {
    config:       AutonomyConfig,
    motivation:   Arc<Mutex<Motivation>>,
    motivation_path: PathBuf,
    memory:       Arc<Mutex<MemoryStore>>,
    user_model:   Arc<Mutex<UserModel>>,
    tools:        Arc<ToolRegistry>,
    task_queue:   TaskQueue,
    rate_limiter: Arc<Mutex<RateLimiter>>,
    token_budget: Arc<Mutex<TokenBudget>>,
    proposals:    Arc<Mutex<Vec<Proposal>>>,
    paused:       Arc<std::sync::atomic::AtomicBool>,
}

impl LifeLoop {
    pub fn new(
        config: AutonomyConfig,
        motivation: Arc<Mutex<Motivation>>,
        motivation_path: PathBuf,
        memory: Arc<Mutex<MemoryStore>>,
        user_model: Arc<Mutex<UserModel>>,
        tools: Arc<ToolRegistry>,
        task_queue: TaskQueue,
    ) -> Self {
        let limiter = RateLimiter::new(
            config.max_internal_tasks_per_hour,
            config.max_internal_tasks_per_day,
        );
        let budget = TokenBudget::new(config.max_tokens_per_day);
        Self {
            config,
            motivation,
            motivation_path,
            memory,
            user_model,
            tools,
            task_queue,
            rate_limiter: Arc::new(Mutex::new(limiter)),
            token_budget: Arc::new(Mutex::new(budget)),
            proposals:    Arc::new(Mutex::new(Vec::new())),
            paused:       Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Start the heartbeat as a background tokio task.
    /// No-op when `autonomy.enabled == false`.
    pub fn spawn(self) -> LifeLoopHandle {
        let handle = LifeLoopHandle {
            paused:       self.paused.clone(),
            proposals:    self.proposals.clone(),
            rate_limiter: self.rate_limiter.clone(),
            token_budget: self.token_budget.clone(),
            max_tokens_per_day: self.config.max_tokens_per_day,
            motivation:   self.motivation.clone(),
            memory:       self.memory.clone(),
        };

        if !self.config.enabled {
            info!("autonomy disabled — life loop not started");
            return handle;
        }

        let cfg = self.config.clone();
        let tick_secs = cfg.tick_interval_secs.max(5);
        let memory = self.memory.clone();
        let user_model = self.user_model.clone();
        let tools = self.tools.clone();
        let queue = self.task_queue.clone();
        let mot_path = self.motivation_path.clone();
        let rl = self.rate_limiter.clone();
        let tb = self.token_budget.clone();
        let pa = self.paused.clone();
        let pr = self.proposals.clone();
        let motivation = self.motivation.clone();

        info!(
            tick_secs,
            propose_only = cfg.propose_only,
            max_per_hour = cfg.max_internal_tasks_per_hour,
            max_per_day  = cfg.max_internal_tasks_per_day,
            max_tokens_per_day = cfg.max_tokens_per_day,
            "life loop starting"
        );

        let cfg_task = cfg.clone();
        tokio::spawn(async move {
            let cfg = cfg_task;
            let mut interval = tokio::time::interval(Duration::from_secs(tick_secs));
            loop {
                interval.tick().await;
                if pa.load(std::sync::atomic::Ordering::Relaxed) {
                    debug!("life loop paused, skipping tick");
                    continue;
                }
                // Decay drives, persist state
                {
                    let mut mot = motivation.lock().await;
                    mot.decay();
                    let _ = mot.save(&mot_path);
                }

                // Expire pending proposals older than 24h so the list doesn't
                // grow unbounded. Threshold is deliberately generous — users
                // may ignore the Life tab for a day.
                {
                    let cutoff = chrono::Utc::now() - chrono::Duration::hours(24);
                    let mut ps = pr.lock().await;
                    let mut expired = 0;
                    for p in ps.iter_mut() {
                        if p.status == ProposalStatus::Pending && p.created_at < cutoff {
                            p.status = ProposalStatus::Expired;
                            expired += 1;
                        }
                    }
                    if expired > 0 {
                        info!(expired, "life loop: expired stale proposals");
                    }
                }

                // Gate: queue idle + rate-limit budget
                if !queue.is_idle().await {
                    debug!("queue busy, skipping tick");
                    continue;
                }
                if !rl.lock().await.has_budget() {
                    debug!("rate limited, skipping tick");
                    continue;
                }

                // Decide
                let action = {
                    let mot = motivation.lock().await;
                    mot.suggest_action()
                };
                let Some(action) = action else {
                    debug!("no action this tick (drives at baseline)");
                    continue;
                };

                // Build task prompt based on chosen action
                let (prompt, reasoning) = match action {
                    LifeAction::Explore => {
                        // Collect prompts from still-live proposals so Curiosity
                        // doesn't re-propose the same exploration on every tick.
                        // "Live" = pending or approved-but-unexecuted; once a
                        // proposal is rejected/executed/expired the suggestion
                        // becomes fair game again.
                        let recent_prompts: Vec<String> = {
                            let proposals = pr.lock().await;
                            proposals.iter()
                                .filter(|p| matches!(
                                    p.status,
                                    ProposalStatus::Pending | ProposalStatus::Approved,
                                ))
                                .map(|p| p.task_prompt.clone())
                                .collect()
                        };
                        let user = user_model.lock().await;
                        let p = Curiosity::next(&*user, &tools, &recent_prompts);
                        (p, "Curiosity: explore something unseen".to_string())
                    }
                    LifeAction::ReviewFailures => {
                        (
                            Some("I noticed some recent task failures. Please review my most recent reflections and suggest improvements.".to_string()),
                            "Competence drive low — reviewing failures".to_string(),
                        )
                    }
                    LifeAction::ProposeHelpful => {
                        let mem = memory.lock().await;
                        let user = user_model.lock().await;
                        let mot = motivation.lock().await;
                        let p = GoalGenerator::top(&*user, &*mem, &*mot);
                        let reasoning = p.as_ref()
                            .map(|g| g.reasoning.clone())
                            .unwrap_or_else(|| "No pattern strong enough".to_string());
                        (p.map(|g| g.suggestion), reasoning)
                    }
                    LifeAction::ReviewBehavior => {
                        (
                            Some("Briefly review whether my recent behavior matches my SOUL.md personality. Report findings to memory.".to_string()),
                            "Coherence drive low — reviewing consistency".to_string(),
                        )
                    }
                };

                let Some(prompt) = prompt else {
                    debug!(action = %action, "no prompt generated");
                    continue;
                };

                // Token budget gate — estimate cost and skip if over cap.
                let est_cost = estimate_prompt_tokens(&prompt) + OUTPUT_TOKEN_ESTIMATE;
                {
                    let mut budget = tb.lock().await;
                    if !budget.has_budget(est_cost) {
                        info!(
                            spent = budget.spent_today(),
                            cap = cfg.max_tokens_per_day,
                            est_cost,
                            "daily token budget exhausted, skipping tick"
                        );
                        continue;
                    }
                }

                rl.lock().await.consume();
                tb.lock().await.consume(est_cost);

                if cfg.propose_only {
                    let proposal = Proposal {
                        id:          format!("p_{}", Uuid::new_v4()),
                        action:      action.to_string(),
                        task_prompt: prompt,
                        reasoning,
                        created_at:  chrono::Utc::now(),
                        status:      ProposalStatus::Pending,
                    };
                    info!(id = %proposal.id, action = %proposal.action, "life loop: proposal queued");
                    pr.lock().await.push(proposal);
                } else {
                    info!(action = %action, "life loop: executing autonomously (propose_only=false)");
                    let source = TaskSource::Internal {
                        action: action.to_string(),
                        reason: reasoning,
                    };
                    let (_stream_rx, _result_rx) =
                        queue.submit_and_wait(prompt, None, source).await;
                    // We don't wait for the result here — fire-and-forget.
                }
            }
        });

        handle
    }
}

/// Handle returned by `LifeLoop::spawn` — the public surface for pausing,
/// inspecting drives, and managing proposals. Web API uses this.
#[derive(Clone)]
pub struct LifeLoopHandle {
    paused:       Arc<std::sync::atomic::AtomicBool>,
    proposals:    Arc<Mutex<Vec<Proposal>>>,
    rate_limiter: Arc<Mutex<RateLimiter>>,
    token_budget: Arc<Mutex<TokenBudget>>,
    max_tokens_per_day: u64,
    motivation:   Arc<Mutex<Motivation>>,
    /// Shared with LifeLoop — lets snapshot() report the long-term
    /// memory row count for the Autonomy UI without routing through
    /// a separate endpoint.
    memory:       Arc<Mutex<MemoryStore>>,
}

impl LifeLoopHandle {
    pub fn pause(&self)  { self.paused.store(true,  std::sync::atomic::Ordering::Relaxed); }
    pub fn resume(&self) { self.paused.store(false, std::sync::atomic::Ordering::Relaxed); }
    pub fn is_paused(&self) -> bool { self.paused.load(std::sync::atomic::Ordering::Relaxed) }

    pub async fn list_proposals(&self) -> Vec<Proposal> {
        self.proposals.lock().await.clone()
    }

    pub async fn get_proposal(&self, id: &str) -> Option<Proposal> {
        self.proposals.lock().await.iter().find(|p| p.id == id).cloned()
    }

    pub async fn set_proposal_status(&self, id: &str, status: ProposalStatus) -> bool {
        let mut pr = self.proposals.lock().await;
        if let Some(p) = pr.iter_mut().find(|p| p.id == id) {
            p.status = status;
            true
        } else {
            false
        }
    }

    /// Auto-expire proposals older than `hours` hours.
    pub async fn expire_old(&self, hours: i64) -> usize {
        let cutoff = chrono::Utc::now() - chrono::Duration::hours(hours);
        let mut pr = self.proposals.lock().await;
        let mut n = 0;
        for p in pr.iter_mut() {
            if p.status == ProposalStatus::Pending && p.created_at < cutoff {
                p.status = ProposalStatus::Expired;
                n += 1;
            }
        }
        n
    }

    pub async fn snapshot(&self, enabled: bool) -> AutonomyState {
        let mot = self.motivation.lock().await.clone();
        let (last_hour, last_day) = self.rate_limiter.lock().await.snapshot();
        let tokens_last_day = self.token_budget.lock().await.spent_today();
        let proposals = self.proposals.lock().await.clone();
        let total_proposals = proposals.len();
        // Return only the 20 most recent for UI rendering
        let recent: Vec<Proposal> = proposals.into_iter().rev().take(20).collect();
        let memory_count: usize = self.memory.lock().await.count().unwrap_or(0);
        AutonomyState {
            enabled,
            paused: self.is_paused(),
            motivation: mot,
            tasks_last_hour: last_hour,
            tasks_last_day:  last_day,
            tokens_last_day,
            tokens_max_per_day: self.max_tokens_per_day,
            recent_proposals: recent,
            total_proposals,
            memory_count,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limiter_respects_hourly_limit() {
        let mut rl = RateLimiter::new(3, 100);
        assert!(rl.has_budget());
        for _ in 0..3 { rl.consume(); }
        assert!(!rl.has_budget()); // hit hourly cap
    }

    #[test]
    fn token_budget_respects_daily_cap() {
        let mut tb = TokenBudget::new(1000);
        assert!(tb.has_budget(400));
        tb.consume(400);
        assert_eq!(tb.spent_today(), 400);
        assert!(tb.has_budget(600));  // exactly at cap
        tb.consume(600);
        assert!(!tb.has_budget(1));   // exceeded
    }

    #[test]
    fn token_budget_zero_cap_is_disabled() {
        // max=0 means "don't gate" — used when autonomy is off.
        let mut tb = TokenBudget::new(0);
        assert!(tb.has_budget(100_000));
    }

    #[test]
    fn estimate_prompt_tokens_is_reasonable() {
        // 12 chars / 4 = 3 tokens
        assert_eq!(estimate_prompt_tokens("abcdefghijkl"), 3);
        // Empty prompt = 0 tokens
        assert_eq!(estimate_prompt_tokens(""), 0);
        // Round up — 5 chars becomes 2 tokens (ceil of 5/4)
        assert_eq!(estimate_prompt_tokens("abcde"), 2);
    }
}
