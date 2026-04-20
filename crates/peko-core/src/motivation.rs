//! Intrinsic motivation drives — Phase D.
//!
//! Four scalar drives (0-1) that influence the life loop's decisions:
//!   - curiosity   — desire to learn new things
//!   - competence  — desire to succeed at tasks
//!   - social      — desire to help the user
//!   - coherence   — desire to stay consistent with SOUL + user model
//!
//! Updated by *code paths* (see `record_event`), NOT by LLM output. This is a
//! deliberate safety choice — an LLM can't self-inflate its curiosity drive to
//! justify exploration. Drives decay toward 0.5 baseline at 1% per hour.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tracing::{info, warn};

/// Events that nudge drives. Emitted from runtime / reflector / user interactions.
#[derive(Debug, Clone, Copy)]
pub enum DriveEvent {
    TaskSucceeded,
    TaskFailed,
    TaskEscalated,
    UserEngaged,            // user kept chatting, approved a proposal
    UserRejected,           // user deleted / ignored a proposal
    NewToolUsed,            // a tool used for the first time
    StalePatterns,          // N ticks with nothing new observed
    BehaviorCoherent,       // action matched SOUL/user model expectation
    BehaviorIncoherent,     // user corrected the agent
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Motivation {
    pub curiosity:  f32,
    pub competence: f32,
    pub social:     f32,
    pub coherence:  f32,
    pub updated_at: DateTime<Utc>,
    /// Total number of ticks observed (for stats/UI)
    #[serde(default)]
    pub tick_count: u64,
}

impl Default for Motivation {
    fn default() -> Self {
        Self {
            curiosity:  0.5,
            competence: 0.5,
            social:     0.5,
            coherence:  0.5,
            updated_at: Utc::now(),
            tick_count: 0,
        }
    }
}

/// What the life loop should do next tick, based on current drives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LifeAction {
    /// Explore something new (curiosity high)
    Explore,
    /// Review a recent failure (competence low)
    ReviewFailures,
    /// Propose a helpful task (social high)
    ProposeHelpful,
    /// Reflect on recent behavior consistency (coherence low)
    ReviewBehavior,
}

impl std::fmt::Display for LifeAction {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Explore         => write!(f, "explore"),
            Self::ReviewFailures  => write!(f, "review-failures"),
            Self::ProposeHelpful  => write!(f, "propose-helpful"),
            Self::ReviewBehavior  => write!(f, "review-behavior"),
        }
    }
}

impl Motivation {
    /// Load from JSON, defaulting if the file is missing or corrupt.
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(s) => match serde_json::from_str(&s) {
                Ok(m) => {
                    info!(path = %path.display(), "motivation loaded");
                    m
                }
                Err(e) => {
                    warn!(error = %e, "corrupt motivation.json, using default");
                    Self::default()
                }
            },
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Apply a drive event, clamping each drive to [0, 1].
    pub fn record(&mut self, event: DriveEvent) {
        let (field, delta) = match event {
            DriveEvent::TaskSucceeded      => ("competence",  0.05),
            DriveEvent::TaskFailed         => ("competence", -0.05),
            DriveEvent::TaskEscalated      => ("competence", -0.03),
            DriveEvent::UserEngaged        => ("social",      0.05),
            DriveEvent::UserRejected       => ("social",     -0.05),
            DriveEvent::NewToolUsed        => ("curiosity",  -0.10),  // fulfilled
            DriveEvent::StalePatterns      => ("curiosity",   0.02),
            DriveEvent::BehaviorCoherent   => ("coherence",   0.02),
            DriveEvent::BehaviorIncoherent => ("coherence",  -0.05),
        };
        self.adjust(field, delta);
        self.updated_at = Utc::now();
    }

    /// Baseline decay — each drive drifts toward 0.5 at 1%/hour.
    pub fn decay(&mut self) {
        let now = Utc::now();
        let hours = (now - self.updated_at).num_seconds() as f32 / 3600.0;
        if hours <= 0.0 {
            return;
        }
        let rate = 0.01 * hours;  // 1% toward center per hour
        for drive in ["curiosity", "competence", "social", "coherence"] {
            let v = self.get(drive);
            let pulled = v + (0.5 - v) * rate.min(1.0);
            self.set(drive, pulled);
        }
        self.updated_at = now;
        self.tick_count += 1;
    }

    /// Pick what to do this tick, or None if nothing's above threshold.
    /// Thresholds chosen so an idle tick with all drives at 0.5 → None.
    pub fn suggest_action(&self) -> Option<LifeAction> {
        if self.curiosity  > 0.70 { return Some(LifeAction::Explore); }
        if self.competence < 0.30 { return Some(LifeAction::ReviewFailures); }
        if self.social     > 0.70 { return Some(LifeAction::ProposeHelpful); }
        if self.coherence  < 0.30 { return Some(LifeAction::ReviewBehavior); }
        None
    }

    // ── field helpers ──
    fn get(&self, name: &str) -> f32 {
        match name {
            "curiosity"  => self.curiosity,
            "competence" => self.competence,
            "social"     => self.social,
            "coherence"  => self.coherence,
            _ => 0.5,
        }
    }
    fn set(&mut self, name: &str, v: f32) {
        let clamped = v.clamp(0.0, 1.0);
        match name {
            "curiosity"  => self.curiosity  = clamped,
            "competence" => self.competence = clamped,
            "social"     => self.social     = clamped,
            "coherence"  => self.coherence  = clamped,
            _ => {}
        }
    }
    /// Weber-Fechner-style update: dampen the effective delta as the
    /// drive approaches whichever extreme it's moving toward.
    ///
    /// Without this, a drive that's been repeatedly reinforced pins at
    /// the boundary (we saw competence=0.9997 after 20 successful tests
    /// and never budged from there regardless of later behaviour). The
    /// linear +0.05 clamps to 1.0 whenever the previous value was ≥ 0.95,
    /// which makes every subsequent update a no-op — the drive becomes
    /// a signal flag rather than a gradient.
    ///
    /// Scaling: `room = 1-v` when going up, `v` when going down. At the
    /// midpoint v=0.5, room=0.5, we multiply by 2 so behaviour matches
    /// the old linear model exactly. Away from midpoint the scaling
    /// shrinks the delta proportionally, so reinforcement converges to
    /// the asymptote instead of landing on it.
    ///
    /// Verified by the existing `record_clamps_drives` test: competence
    /// stays within [0, 1] after 100 TaskSucceeded + 100 TaskFailed.
    fn adjust(&mut self, name: &str, delta: f32) {
        if delta == 0.0 { return; }
        let v = self.get(name);
        let room = if delta > 0.0 { 1.0 - v } else { v };
        // 2.0 factor so v=0.5 midpoint preserves the previous linear
        // delta magnitude exactly — callers calibrated their deltas
        // against that baseline, no need to re-tune numbers in record().
        let effective = delta * 2.0 * room;
        self.set(name, v + effective);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_clamps_drives() {
        let mut m = Motivation::default();
        for _ in 0..100 { m.record(DriveEvent::TaskSucceeded); }
        assert!(m.competence <= 1.0);
        for _ in 0..100 { m.record(DriveEvent::TaskFailed); }
        assert!(m.competence >= 0.0);
    }

    #[test]
    fn suggest_action_high_curiosity() {
        let mut m = Motivation::default();
        m.curiosity = 0.9;
        assert_eq!(m.suggest_action(), Some(LifeAction::Explore));
    }

    #[test]
    fn suggest_action_low_competence() {
        let mut m = Motivation::default();
        m.curiosity = 0.5;
        m.competence = 0.2;
        assert_eq!(m.suggest_action(), Some(LifeAction::ReviewFailures));
    }

    #[test]
    fn suggest_action_idle_noop() {
        let m = Motivation::default();  // all 0.5
        assert_eq!(m.suggest_action(), None);
    }

    #[test]
    fn decay_pulls_toward_center() {
        let mut m = Motivation::default();
        m.curiosity = 0.9;
        m.updated_at = Utc::now() - chrono::Duration::hours(10);  // 10 hours ago
        m.decay();
        // Curiosity should have decayed meaningfully toward 0.5
        assert!(m.curiosity < 0.9);
        assert!(m.curiosity >= 0.5);
    }

    #[test]
    fn save_load_roundtrip() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("peko_motivation_test_{}.json", std::process::id()));
        let mut m = Motivation::default();
        m.record(DriveEvent::TaskSucceeded);
        m.record(DriveEvent::UserEngaged);
        m.save(&path).unwrap();
        let loaded = Motivation::load(&path);
        assert!((loaded.competence - m.competence).abs() < 1e-6);
        assert!((loaded.social     - m.social    ).abs() < 1e-6);
        let _ = std::fs::remove_file(path);
    }
}
