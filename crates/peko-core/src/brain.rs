use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, warn};

use peko_transport::LlmProvider;
use crate::skills::SkillStore;

/// Which brain should handle this task.
#[derive(Debug, Clone, PartialEq)]
pub enum BrainChoice {
    Local,
    Cloud,
}

impl std::fmt::Display for BrainChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Local => write!(f, "local"),
            Self::Cloud => write!(f, "cloud"),
        }
    }
}

/// Dual-brain architecture: lightweight local LLM for simple/skill-based tasks,
/// powerful cloud provider for complex reasoning.
///
/// Routing rules:
/// 1. If a matching skill exists with good success rate → Local
/// 2. If the task looks simple (short, imperative, known patterns) → Local
/// 3. Otherwise → Cloud
///
/// The local LLM gets an `escalate` tool. If it calls it, the runtime
/// restarts the task with the cloud provider, forwarding the local LLM's
/// analysis as additional context.
pub struct DualBrain {
    local: Box<dyn LlmProvider>,
    cloud: Box<dyn LlmProvider>,
    /// Minimum skill success rate to trust for local routing (0.0–1.0)
    skill_threshold: f32,
    /// Max input length (chars) to consider "simple" for local
    simple_max_len: usize,
}

impl DualBrain {
    pub fn new(local: Box<dyn LlmProvider>, cloud: Box<dyn LlmProvider>) -> Self {
        Self {
            local,
            cloud,
            skill_threshold: 0.6,
            simple_max_len: 200,
        }
    }

    pub fn with_skill_threshold(mut self, threshold: f32) -> Self {
        self.skill_threshold = threshold;
        self
    }

    pub fn with_simple_max_len(mut self, len: usize) -> Self {
        self.simple_max_len = len;
        self
    }

    /// Classify a task to determine which brain handles it.
    pub fn classify(
        &self,
        input: &str,
        skills: Option<&SkillStore>,
    ) -> BrainChoice {
        // 1. Check for matching skills with good success rate
        if let Some(skill_store) = skills {
            let matches = skill_store.search(input);
            let has_good_skill = matches.iter().any(|s| {
                let total = s.success_count + s.fail_count;
                total >= 2 && s.success_rate() >= self.skill_threshold * 100.0
            });
            if has_good_skill {
                info!(task = %truncate(input, 60), "brain: routed to LOCAL (matching skill)");
                return BrainChoice::Local;
            }
        }

        // 2. Check if task is simple enough for local LLM
        if self.is_simple_task(input) {
            info!(task = %truncate(input, 60), "brain: routed to LOCAL (simple task)");
            return BrainChoice::Local;
        }

        // 3. Default to cloud for complex tasks
        info!(task = %truncate(input, 60), "brain: routed to CLOUD (complex task)");
        BrainChoice::Cloud
    }

    /// Heuristic: is this task simple enough for a lightweight LLM?
    fn is_simple_task(&self, input: &str) -> bool {
        let input_len = input.len();
        let lower = input.to_lowercase();

        // Too long → probably complex
        if input_len > self.simple_max_len {
            return false;
        }

        // Simple patterns: single-action commands
        let simple_prefixes = [
            "open ", "launch ", "close ", "stop ",
            "tap ", "click ", "press ", "swipe ",
            "go to ", "navigate to ",
            "type ", "enter ", "input ",
            "take a screenshot", "screenshot",
            "what time", "what day", "what date",
            "show ", "list ", "check ",
            "turn on ", "turn off ", "enable ", "disable ",
            "set ", "change ", "switch ",
            "install ", "uninstall ", "update ",
            "call ", "send sms", "send message",
            "play ", "pause ", "next ", "previous ",
            "volume up", "volume down", "mute",
            "go back", "go home", "recent apps",
            "scroll up", "scroll down",
            "wifi ", "bluetooth ", "brightness ",
        ];

        if simple_prefixes.iter().any(|p| lower.starts_with(p)) {
            return true;
        }

        // Very short input (< 50 chars) with no complex markers
        let complex_markers = [
            "explain", "analyze", "compare", "debug", "investigate",
            "write a", "create a", "build a", "design",
            "how do", "how can", "how should", "why does", "why is",
            "what if", "what would",
            "step by step", "in detail",
            "multiple", "several", "all the",
            "and then", "after that", "finally",
        ];

        if input_len < 50 && !complex_markers.iter().any(|m| lower.contains(m)) {
            return true;
        }

        false
    }

    /// Get the local provider
    pub fn local(&self) -> &dyn LlmProvider {
        self.local.as_ref()
    }

    /// Get the cloud provider
    pub fn cloud(&self) -> &dyn LlmProvider {
        self.cloud.as_ref()
    }

    /// Get provider by brain choice
    pub fn provider(&self, choice: &BrainChoice) -> &dyn LlmProvider {
        match choice {
            BrainChoice::Local => self.local.as_ref(),
            BrainChoice::Cloud => self.cloud.as_ref(),
        }
    }

    pub fn local_model_name(&self) -> &str {
        self.local.model_name()
    }

    pub fn cloud_model_name(&self) -> &str {
        self.cloud.model_name()
    }
}

/// The escalate tool definition — injected when running on local brain.
/// When the local LLM calls this, the runtime catches it and switches to cloud.
pub const ESCALATE_TOOL_NAME: &str = "escalate";

pub fn escalate_tool_schema() -> serde_json::Value {
    serde_json::json!({
        "name": ESCALATE_TOOL_NAME,
        "description": "Escalate this task to the more powerful cloud AI model. \
            Use this when you realize the task is too complex for you to handle well — \
            for example: multi-step reasoning, unfamiliar situations, tasks requiring \
            deep analysis, or when you are unsure about the correct approach. \
            Include your analysis so far so the cloud model can continue from where you left off.",
        "input_schema": {
            "type": "object",
            "properties": {
                "reason": {
                    "type": "string",
                    "description": "Why you are escalating (e.g. 'task requires multi-step planning I cannot do reliably')"
                },
                "analysis": {
                    "type": "string",
                    "description": "Your analysis of the task so far — what you understood, what you tried, what the next steps might be"
                }
            },
            "required": ["reason"]
        }
    })
}

/// Build an escalation context message for the cloud provider.
/// Includes the local LLM's analysis so the cloud doesn't start from scratch.
pub fn build_escalation_context(
    original_input: &str,
    reason: &str,
    analysis: Option<&str>,
    local_model: &str,
) -> String {
    let mut ctx = format!(
        "[Escalated from local model ({})]\n\
         Reason: {}\n\
         Original task: {}",
        local_model, reason, original_input
    );
    if let Some(a) = analysis {
        if !a.is_empty() {
            ctx.push_str(&format!("\n\nLocal model's analysis:\n{}", a));
        }
    }
    ctx.push_str("\n\nPlease complete this task. The local model's analysis above may be helpful context.");
    ctx
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_task_detection() {
        let brain = DualBrain {
            local: Box::new(DummyProvider),
            cloud: Box::new(DummyProvider),
            skill_threshold: 0.6,
            simple_max_len: 200,
        };

        // Simple tasks
        assert!(brain.is_simple_task("open youtube"));
        assert!(brain.is_simple_task("take a screenshot"));
        assert!(brain.is_simple_task("tap 100 200"));
        assert!(brain.is_simple_task("send sms to bob hello"));
        assert!(brain.is_simple_task("go back"));
        assert!(brain.is_simple_task("volume up"));
        assert!(brain.is_simple_task("install com.example.app"));
        assert!(brain.is_simple_task("wifi on"));
        assert!(brain.is_simple_task("ok")); // very short

        // Complex tasks
        assert!(!brain.is_simple_task("explain how the Android activity lifecycle works and compare it to iOS"));
        assert!(!brain.is_simple_task("analyze the battery usage and investigate which app is draining"));
        assert!(!brain.is_simple_task("write a script that monitors CPU usage and sends alerts"));
    }

    #[test]
    fn test_classify_defaults_to_cloud() {
        let brain = DualBrain {
            local: Box::new(DummyProvider),
            cloud: Box::new(DummyProvider),
            skill_threshold: 0.6,
            simple_max_len: 200,
        };

        assert_eq!(
            brain.classify("explain the meaning of life in detail", None),
            BrainChoice::Cloud
        );
    }

    #[test]
    fn test_classify_simple_to_local() {
        let brain = DualBrain {
            local: Box::new(DummyProvider),
            cloud: Box::new(DummyProvider),
            skill_threshold: 0.6,
            simple_max_len: 200,
        };

        assert_eq!(
            brain.classify("open settings", None),
            BrainChoice::Local
        );
    }

    #[test]
    fn test_escalation_context() {
        let ctx = build_escalation_context(
            "debug why wifi keeps disconnecting",
            "requires network diagnostics I can't do",
            Some("The user's wifi drops every 5 minutes. Might be related to DHCP lease."),
            "qwen-2.5-7b",
        );

        assert!(ctx.contains("Escalated from local model"));
        assert!(ctx.contains("qwen-2.5-7b"));
        assert!(ctx.contains("DHCP lease"));
    }

    // Dummy provider for tests
    struct DummyProvider;

    #[async_trait::async_trait]
    impl LlmProvider for DummyProvider {
        async fn stream_completion(
            &self, _: &str, _: &[peko_transport::provider::Message],
            _: &[serde_json::Value],
        ) -> anyhow::Result<futures::stream::BoxStream<'static, anyhow::Result<peko_transport::StreamEvent>>> {
            Ok(Box::pin(futures::stream::empty()))
        }
        fn model_name(&self) -> &str { "dummy" }
        fn max_context_tokens(&self) -> usize { 4096 }
    }
}
