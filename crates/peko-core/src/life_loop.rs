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
    pub recent_proposals: Vec<Proposal>,
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
        Self {
            config,
            motivation,
            motivation_path,
            memory,
            user_model,
            tools,
            task_queue,
            rate_limiter: Arc::new(Mutex::new(limiter)),
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
            motivation:   self.motivation.clone(),
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
        let pa = self.paused.clone();
        let pr = self.proposals.clone();
        let motivation = self.motivation.clone();

        info!(
            tick_secs,
            propose_only = cfg.propose_only,
            max_per_hour = cfg.max_internal_tasks_per_hour,
            max_per_day  = cfg.max_internal_tasks_per_day,
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
                        let user = user_model.lock().await;
                        let p = Curiosity::next(&*user, &tools);
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

                rl.lock().await.consume();

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
    motivation:   Arc<Mutex<Motivation>>,
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
        let proposals = self.proposals.lock().await.clone();
        // Return only the 20 most recent for UI rendering
        let recent: Vec<Proposal> = proposals.into_iter().rev().take(20).collect();
        AutonomyState {
            enabled,
            paused: self.is_paused(),
            motivation: mot,
            tasks_last_hour: last_hour,
            tasks_last_day:  last_day,
            recent_proposals: recent,
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
}
