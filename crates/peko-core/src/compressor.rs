use crate::message::Message;
use tracing::info;

pub struct ContextCompressor {
    max_context_tokens: usize,
    history_share: f32,
}

#[derive(Debug)]
pub struct CompressionResult {
    pub compressed: bool,
    pub original_tokens: usize,
    pub compressed_tokens: usize,
    pub messages_removed: usize,
}

impl ContextCompressor {
    pub fn new(max_context_tokens: usize, history_share: f32) -> Self {
        Self { max_context_tokens, history_share }
    }

    pub fn check_and_compress(&self, conversation: &mut Vec<Message>) -> CompressionResult {
        let original_tokens: usize = conversation.iter().map(|m| m.estimated_tokens()).sum();
        let budget = (self.max_context_tokens as f32 * self.history_share) as usize;

        if original_tokens <= budget || conversation.len() <= 4 {
            return CompressionResult {
                compressed: false,
                original_tokens,
                compressed_tokens: original_tokens,
                messages_removed: 0,
            };
        }

        let keep_tail = 4.min(conversation.len() - 1);

        let mut summary_parts: Vec<String> = Vec::new();
        let remove_start = 1;
        let remove_end = conversation.len() - keep_tail;

        for msg in &conversation[remove_start..remove_end] {
            match msg {
                Message::Assistant { tool_calls, .. } => {
                    for tc in tool_calls {
                        summary_parts.push(format!("Called tool '{}' with args: {}", tc.name, tc.input));
                    }
                }
                Message::ToolResult { name, content, is_error, .. } => {
                    let status = if *is_error { "ERROR" } else { "OK" };
                    let truncated = if content.len() > 200 {
                        format!("{}...", &content[..200])
                    } else {
                        content.clone()
                    };
                    summary_parts.push(format!("Tool '{}' [{}]: {}", name, status, truncated));
                }
                _ => {}
            }
        }

        let summary = format!(
            "[Compressed history — {} previous interactions]\n{}",
            remove_end - remove_start,
            summary_parts.join("\n")
        );

        let messages_removed = remove_end - remove_start;

        let mut new_conversation = Vec::new();
        new_conversation.push(conversation[0].clone()); // system or first user
        new_conversation.push(Message::user(summary));
        new_conversation.extend(conversation[remove_end..].iter().cloned());

        let compressed_tokens: usize = new_conversation.iter().map(|m| m.estimated_tokens()).sum();

        info!(
            original_tokens,
            compressed_tokens,
            messages_removed,
            "compressed conversation context"
        );

        *conversation = new_conversation;

        CompressionResult {
            compressed: true,
            original_tokens,
            compressed_tokens,
            messages_removed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::ToolCall;

    #[test]
    fn test_no_compression_under_budget() {
        let mut conv = vec![
            Message::user("hello"),
            Message::assistant_text("hi there"),
        ];
        let compressor = ContextCompressor::new(200_000, 0.7);
        let result = compressor.check_and_compress(&mut conv);
        assert!(!result.compressed);
        assert_eq!(conv.len(), 2);
    }

    #[test]
    fn test_compression_over_budget() {
        let mut conv = vec![Message::user("start")];
        for i in 0..20 {
            conv.push(Message::assistant_tool_calls(
                None,
                vec![ToolCall {
                    id: format!("tc_{}", i),
                    name: "shell".to_string(),
                    input: serde_json::json!({"command": "x".repeat(1000)}),
                }],
            ));
            conv.push(Message::tool_result(
                format!("tc_{}", i),
                "shell".to_string(),
                "x".repeat(1000),
                false,
            ));
        }
        conv.push(Message::assistant_text("final answer"));

        let compressor = ContextCompressor::new(100, 0.5); // very small budget
        let result = compressor.check_and_compress(&mut conv);
        assert!(result.compressed);
        assert!(conv.len() < 42);
    }
}
