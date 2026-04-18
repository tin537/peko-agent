//! Curiosity — Phase E.
//!
//! When `motivation.curiosity > 0.7`, the life loop asks the Curiosity module
//! for the next exploration task. Strategy: identify dimensions where the
//! agent hasn't explored yet, and propose a safe read-only task.

use std::collections::HashSet;

use crate::tool::ToolRegistry;
use crate::user_model::UserModel;

pub struct Curiosity;

impl Curiosity {
    /// Return a task prompt for the next exploration, or None if nothing
    /// interesting is unexplored right now.
    ///
    /// Candidates are generated in priority order and filtered against
    /// `recent_prompts` — any candidate already proposed (still pending, or
    /// within the life loop's recent history) is skipped so we don't loop
    /// on the same suggestion. The life loop passes in pending + approved
    /// + recently executed proposal prompts.
    ///
    /// Dimensions considered:
    ///   1. Any registered tool the agent has never used
    ///   2. Unusual hour of day (no activity logged for this hour before)
    ///   3. Apps seen but never interacted with (not implemented yet)
    pub fn next(
        user: &UserModel,
        tools: &ToolRegistry,
        recent_prompts: &[String],
    ) -> Option<String> {
        Self::candidates(user, tools)
            .into_iter()
            .find(|c| !recent_prompts.iter().any(|p| p == c))
    }

    /// Enumerate all currently-plausible exploration prompts in priority
    /// order. Exposed for testing the dedup behavior.
    pub fn candidates(user: &UserModel, tools: &ToolRegistry) -> Vec<String> {
        let mut out = Vec::new();

        // 1. Registered tools that have never been used
        let registered: HashSet<String> = tools.available_tools()
            .into_iter()
            .map(String::from)
            .collect();
        let used: HashSet<String> = user.patterns.common_tasks.iter().cloned().collect();

        // Prefer safe, read-only tools — never propose exploring dangerous ones.
        let safe_explore: &[&str] = &[
            "screenshot", "ui_inspect", "memory", "skills",
        ];
        for tool_name in safe_explore {
            if registered.contains(*tool_name)
                && !used.iter().any(|t| t.contains(tool_name))
            {
                out.push(format!(
                    "I've noticed I haven't used the `{}` tool yet. \
                     Please use it briefly to see what it shows on the current device.",
                    tool_name
                ));
            }
        }

        // 2. Unusual hour — if this hour of day doesn't appear in patterns
        let now = chrono::Local::now();
        let hour = now.format("%H").to_string();
        let active_hours = user.patterns.active_hours.as_deref().unwrap_or("");
        if !active_hours.contains(&hour) {
            out.push(format!(
                "It's {} — an unusual hour for you. \
                 Take a screenshot to observe what's on screen right now; \
                 this may help build context for future tasks.",
                now.format("%H:%M")
            ));
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::user_model::UserModel;

    #[test]
    fn no_tools_registered_returns_hour_or_none() {
        let user = UserModel::default();
        let tools = ToolRegistry::new();
        // Just ensure no panic — may or may not return an hour-based task.
        let _ = Curiosity::next(&user, &tools, &[]);
    }

    #[test]
    fn skips_prompts_already_in_recent() {
        // Force the hour-based prompt to be the first candidate by using an
        // empty registry. The first call produces some candidate; a second
        // call with that candidate in `recent_prompts` must return something
        // different or None — never the same string again.
        let user = UserModel::default();
        let tools = ToolRegistry::new();
        let Some(first) = Curiosity::next(&user, &tools, &[]) else {
            // No candidates generated — this test is vacuously OK.
            return;
        };
        let next = Curiosity::next(&user, &tools, &[first.clone()]);
        assert_ne!(
            next.as_deref(),
            Some(first.as_str()),
            "curiosity must not re-propose a prompt present in recent_prompts",
        );
    }

    #[test]
    fn candidates_list_is_stable_given_same_state() {
        // Same user/tools → same candidate list (modulo hour-of-day entry,
        // which doesn't flap within a test run).
        let user = UserModel::default();
        let tools = ToolRegistry::new();
        let a = Curiosity::candidates(&user, &tools);
        let b = Curiosity::candidates(&user, &tools);
        assert_eq!(a, b);
    }
}
