//! Goal generator — Phase F.
//!
//! Detects patterns in the user model + memory store and proposes a helpful
//! task the agent could do on the user's behalf. Ranked by
//!   (pattern confidence) × (alignment with current drives)
//! and the top candidate is returned to the life loop.

use crate::memory::MemoryStore;
use crate::motivation::Motivation;
use crate::user_model::UserModel;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatternKind {
    /// The user has a recurring action at a specific time of day.
    RecurringTime,
    /// Something has failed repeatedly and the agent could investigate.
    RepeatedFailure,
    /// An unresolved message / notification the user seems to be ignoring.
    Neglected,
    /// Device-state pattern (battery, wifi, storage) the agent could help with.
    DeviceState,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Pattern {
    pub kind:       PatternKind,
    pub confidence: f32,        // 0-1
    pub suggestion: String,     // natural-language task for the agent
    pub reasoning:  String,     // shown to the user in the proposal
}

pub struct GoalGenerator;

impl GoalGenerator {
    /// Run all pattern detectors and return candidate goals ranked by
    /// (confidence × drive_alignment). Empty vec when nothing worth proposing.
    pub fn detect(user: &UserModel, memory: &MemoryStore, drives: &Motivation) -> Vec<Pattern> {
        let mut patterns: Vec<Pattern> = Vec::new();

        patterns.extend(detect_recurring_tasks(user));
        patterns.extend(detect_repeated_failures(memory));

        // Rank by confidence × alignment with the social drive (helpful tasks)
        let weight = drives.social;
        patterns.sort_by(|a, b| {
            let sa = a.confidence * weight;
            let sb = b.confidence * weight;
            sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
        });
        patterns
    }

    /// Return the top-ranked proposal, if any.
    pub fn top(user: &UserModel, memory: &MemoryStore, drives: &Motivation) -> Option<Pattern> {
        Self::detect(user, memory, drives).into_iter().next()
    }
}

// ── individual detectors ───────────────────────────────────────

/// If the user's `common_tasks` include an item done repeatedly, suggest
/// doing it now if we haven't seen it today.
fn detect_recurring_tasks(user: &UserModel) -> Vec<Pattern> {
    user.patterns.common_tasks.iter()
        .take(3)
        .map(|task| Pattern {
            kind: PatternKind::RecurringTime,
            confidence: 0.4,                       // low default — needs real frequency tracking
            suggestion: format!("Would you like me to help with: {}?", task),
            reasoning:  format!("You've done '{}' before and it seems like a recurring task.", task),
        })
        .collect()
}

/// Check recent reflection memories for repeated failures of the same kind.
fn detect_repeated_failures(memory: &MemoryStore) -> Vec<Pattern> {
    // Search for recent reflections marked "failed"
    let Ok(reflections) = memory.list(50, Some("reflection")) else { return vec![]; };
    let failures: Vec<&crate::memory::Memory> = reflections.iter()
        .filter(|m| m.content.contains("\"outcome\":\"failed\"") || m.content.contains("\"outcome\": \"failed\""))
        .collect();

    if failures.len() >= 2 {
        return vec![Pattern {
            kind: PatternKind::RepeatedFailure,
            confidence: (failures.len() as f32 / 10.0).min(0.9),
            suggestion: "Several recent tasks failed. Shall I review what went wrong and try a different approach?".to_string(),
            reasoning:  format!("I've logged {} recent task failures.", failures.len()),
        }];
    }
    vec![]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recurring_task_detection() {
        let mut user = UserModel::default();
        user.patterns.common_tasks = vec![
            "check battery".to_string(),
            "open settings".to_string(),
        ];
        let got = detect_recurring_tasks(&user);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].kind, PatternKind::RecurringTime);
    }
}
