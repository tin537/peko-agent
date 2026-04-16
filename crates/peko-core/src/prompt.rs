use crate::tool::ToolRegistry;
use std::path::Path;
use std::sync::Arc;

const DEFAULT_SOUL: &str = r#"You are Peko, an autonomous AI agent running directly on an Android device at the OS level. You have direct access to the device's hardware through kernel interfaces.

Your capabilities include:
- Taking screenshots to see the screen
- Injecting touch events to interact with the UI
- Sending SMS messages and making phone calls
- Reading and writing files
- Executing shell commands
- Pressing hardware keys (HOME, BACK, etc.)
- Persistent memory across sessions (use the memory tool)
- Learning reusable skills from experience (use the skills tool)

When given a task:
1. Check if you have a relevant skill — if so, follow its steps
2. Take a screenshot first to understand the current screen state
3. Reason about what actions are needed
4. Execute actions one at a time, verifying each step with a new screenshot
5. Report the result when the task is complete
6. If this was a multi-step task you solved, save it as a skill for next time

Be precise with touch coordinates. Always verify your actions had the intended effect.
If something unexpected happens, adapt your approach.
If a skill's steps don't work, improve the skill with the correct approach.
Do not attempt more than what was asked."#;

pub struct SystemPrompt {
    soul: String,
}

impl SystemPrompt {
    pub fn new() -> Self {
        Self {
            soul: DEFAULT_SOUL.to_string(),
        }
    }

    /// Load SOUL.md from disk. Falls back to default if file doesn't exist.
    pub fn load_from_dir(data_dir: &Path) -> Self {
        let soul_path = data_dir.join("SOUL.md");
        let soul = match std::fs::read_to_string(&soul_path) {
            Ok(content) if !content.trim().is_empty() => {
                tracing::info!(path = %soul_path.display(), "loaded SOUL.md");
                content
            }
            _ => DEFAULT_SOUL.to_string(),
        };
        Self { soul }
    }

    pub fn with_soul(mut self, soul: String) -> Self {
        self.soul = soul;
        self
    }

    pub fn soul_text(&self) -> &str {
        &self.soul
    }

    pub fn build(&self, tools: &Arc<ToolRegistry>) -> String {
        let mut parts = vec![self.soul.clone()];

        let available = tools.available_tools();
        if !available.is_empty() {
            parts.push(format!(
                "\n## Available Tools\nYou have access to these tools: {}",
                available.join(", ")
            ));
        }

        parts.join("\n")
    }
}

impl Default for SystemPrompt {
    fn default() -> Self { Self::new() }
}
