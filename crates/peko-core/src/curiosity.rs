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
    /// Dimensions considered:
    ///   1. Any registered tool the agent has never used
    ///   2. Unusual hour of day (no activity logged for this hour before)
    ///   3. Apps seen but never interacted with (not implemented yet)
    pub fn next(user: &UserModel, tools: &ToolRegistry) -> Option<String> {
        // 1. Find a registered tool that's never been used
        let registered: HashSet<String> = tools.available_tools()
            .into_iter()
            .map(String::from)
            .collect();

        let used: HashSet<String> = user.patterns.common_tasks.iter().cloned().collect();
        let ever_used_tool_names = used;

        // Prefer safe, read-only tools — never propose exploring dangerous ones.
        let safe_explore: &[&str] = &[
            "screenshot", "ui_inspect", "memory", "skills",
        ];

        for tool_name in safe_explore {
            if registered.contains(*tool_name)
                && !ever_used_tool_names.iter().any(|t| t.contains(tool_name))
            {
                return Some(format!(
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
            return Some(format!(
                "It's {} — an unusual hour for you. \
                 Take a screenshot to observe what's on screen right now; \
                 this may help build context for future tasks.",
                now.format("%H:%M")
            ));
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::user_model::UserModel;

    #[test]
    fn no_tools_registered_returns_none_or_time_task() {
        // With an empty ToolRegistry, curiosity still might propose an hour-based task
        let user = UserModel::default();
        let tools = ToolRegistry::new();
        let _ = Curiosity::next(&user, &tools); // just ensure no panic
    }
}
