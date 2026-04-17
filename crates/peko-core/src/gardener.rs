//! Memory gardener — Phase C.
//!
//! A background cron task that keeps the memory store bounded. Runs once per
//! minute but only *acts* when the configured cron expression matches the
//! current minute. Default: `0 6 * * *` (06:00 UTC daily).
//!
//! On each firing it:
//!   1. Decays importance of un-accessed memories (factor 0.95 after 14 days)
//!   2. Prunes old low-importance, never-accessed memories (30 days, < 0.3)
//!
//! Skills are exempt from both passes — they're managed by the skills store.

use std::sync::Arc;
use std::time::Duration;

use chrono::{Timelike, Utc};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::{info, warn, error};

use crate::cron::CronExpr;
use crate::memory::MemoryStore;

/// Parameters for the gardener passes. Kept small — most users want defaults.
#[derive(Debug, Clone)]
pub struct GardenerConfig {
    /// Cron expression (5 fields). Default: `0 6 * * *` (06:00 UTC daily).
    pub cron: String,
    /// Memories not accessed for this many days get their importance multiplied
    /// by `decay_factor`. Default: 14 days.
    pub decay_age_days: i64,
    /// Decay multiplier (0.0–1.0). Default: 0.95.
    pub decay_factor: f64,
    /// Memories older than this many days are eligible for pruning. Default: 30.
    pub prune_age_days: i64,
    /// Prune threshold — memories with importance below this get deleted
    /// (if also never accessed). Default: 0.3.
    pub prune_min_importance: f64,
}

impl Default for GardenerConfig {
    fn default() -> Self {
        Self {
            cron:                 "0 6 * * *".to_string(),
            decay_age_days:       14,
            decay_factor:         0.95,
            prune_age_days:       30,
            prune_min_importance: 0.3,
        }
    }
}

/// Spawn the gardener as a detached tokio task. Returns its JoinHandle.
/// Pass `Some(config)` to customize timing or thresholds.
pub fn spawn(
    memory: Arc<Mutex<MemoryStore>>,
    config: GardenerConfig,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let cron = match CronExpr::parse(&config.cron) {
            Ok(c) => {
                info!(cron = %config.cron, desc = %c.describe(), "gardener scheduled");
                c
            }
            Err(e) => {
                error!(cron = %config.cron, error = %e, "invalid gardener cron, disabling");
                return;
            }
        };

        // Align to the top of the next minute so we tick once per minute.
        let now = Utc::now();
        let secs_to_next_min = 60u64.saturating_sub(now.second() as u64).max(1);
        tokio::time::sleep(Duration::from_secs(secs_to_next_min)).await;

        loop {
            if cron.matches_now() {
                run_once(&memory, &config).await;
            }
            // Sleep to the top of the next minute.
            let now = Utc::now();
            let secs = 60u64.saturating_sub(now.second() as u64).max(55);
            tokio::time::sleep(Duration::from_secs(secs)).await;
        }
    })
}

/// Run one gardener pass. Exposed for admin/debug endpoints and tests.
pub async fn run_once(memory: &Arc<Mutex<MemoryStore>>, config: &GardenerConfig) {
    let store = memory.lock().await;

    let decayed = match store.decay_importance(config.decay_age_days, config.decay_factor) {
        Ok(n) => n,
        Err(e) => {
            warn!(error = %e, "gardener: decay_importance failed");
            0
        }
    };

    let pruned = match store.prune(config.prune_age_days, config.prune_min_importance) {
        Ok(n) => n,
        Err(e) => {
            warn!(error = %e, "gardener: prune failed");
            0
        }
    };

    info!(
        decayed,
        pruned,
        decay_days = config.decay_age_days,
        prune_days = config.prune_age_days,
        "gardener pass complete"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::MemoryCategory;

    #[tokio::test]
    async fn gardener_run_once_prunes_and_decays() {
        let store = Arc::new(Mutex::new(MemoryStore::open_in_memory().unwrap()));
        // Fresh memory — prune/decay should both succeed with 0 affected.
        let cfg = GardenerConfig::default();
        run_once(&store, &cfg).await;

        let s = store.lock().await;
        s.save("k1", "fresh fact", &MemoryCategory::Fact, 0.8, None).unwrap();
        drop(s);

        // Running again doesn't wipe the fresh memory (too new + high importance)
        run_once(&store, &cfg).await;
        let s = store.lock().await;
        assert_eq!(s.count().unwrap(), 1);
    }

    #[tokio::test]
    async fn gardener_default_cron_parses() {
        let cfg = GardenerConfig::default();
        // If this panics the binary won't start — guard here so the default
        // is always a valid cron expression.
        CronExpr::parse(&cfg.cron).expect("default gardener cron should parse");
    }
}
