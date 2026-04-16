use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub enum StreamEvent {
    MessageStart { id: String, input_tokens: usize },
    ContentBlockStart { index: usize, block_type: ContentBlockType },
    TextDelta(String),
    ToolUseStart { id: String, name: String },
    ToolInputDelta(String),
    ThinkingDelta(String),
    ContentBlockStop { index: usize },
    MessageDelta { output_tokens: usize, stop_reason: StopReason },
    MessageStop,
    Ping,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ContentBlockType {
    Text,
    ToolUse,
    Thinking,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    StopSequence,
    #[serde(other)]
    Unknown,
}
