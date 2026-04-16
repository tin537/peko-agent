use serde::{Deserialize, Serialize};
use peko_transport::provider::{ContentBlock, ImageSource, Message as TransportMessage, MessageContent};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

#[derive(Debug, Clone)]
pub enum Message {
    System(String),
    User(String),
    Assistant {
        text: Option<String>,
        tool_calls: Vec<ToolCall>,
    },
    ToolResult {
        tool_use_id: String,
        name: String,
        content: String,
        is_error: bool,
        image: Option<ImageData>,
    },
}

#[derive(Debug, Clone)]
pub struct ImageData {
    pub base64: String,
    pub media_type: String,
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Message::User(text.into())
    }

    pub fn assistant_text(text: impl Into<String>) -> Self {
        Message::Assistant {
            text: Some(text.into()),
            tool_calls: vec![],
        }
    }

    pub fn assistant_tool_calls(text: Option<String>, tool_calls: Vec<ToolCall>) -> Self {
        Message::Assistant { text, tool_calls }
    }

    pub fn tool_result(tool_use_id: String, name: String, content: String, is_error: bool) -> Self {
        Message::ToolResult { tool_use_id, name, content, is_error, image: None }
    }

    pub fn tool_result_with_image(
        tool_use_id: String,
        name: String,
        content: String,
        is_error: bool,
        image: ImageData,
    ) -> Self {
        Message::ToolResult { tool_use_id, name, content, is_error, image: Some(image) }
    }

    /// Convert to neutral transport messages.
    /// Each provider converts these to its own wire format.
    pub fn to_transport_messages(&self) -> Vec<TransportMessage> {
        match self {
            Message::System(_) => vec![],
            Message::User(text) => vec![TransportMessage {
                role: "user".to_string(),
                content: MessageContent::Text(text.clone()),
            }],
            Message::Assistant { text, tool_calls } => {
                let mut blocks = Vec::new();
                if let Some(t) = text {
                    blocks.push(ContentBlock::Text { text: t.clone() });
                }
                for tc in tool_calls {
                    blocks.push(ContentBlock::ToolUse {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                        input: tc.input.clone(),
                    });
                }
                if blocks.is_empty() {
                    vec![TransportMessage {
                        role: "assistant".to_string(),
                        content: MessageContent::Text(String::new()),
                    }]
                } else if tool_calls.is_empty() {
                    vec![TransportMessage {
                        role: "assistant".to_string(),
                        content: MessageContent::Text(text.clone().unwrap_or_default()),
                    }]
                } else {
                    vec![TransportMessage {
                        role: "assistant".to_string(),
                        content: MessageContent::Blocks(blocks),
                    }]
                }
            }
            Message::ToolResult { tool_use_id, name, content, is_error, image, .. } => {
                let mut blocks = vec![ContentBlock::ToolResult {
                    tool_use_id: tool_use_id.clone(),
                    content: content.clone(),
                    is_error: *is_error,
                }];
                if let Some(img) = image {
                    blocks.push(ContentBlock::Image {
                        source: ImageSource {
                            source_type: "base64".to_string(),
                            media_type: img.media_type.clone(),
                            data: img.base64.clone(),
                        },
                    });
                }
                vec![TransportMessage {
                    role: "user".to_string(),
                    content: MessageContent::Blocks(blocks),
                }]
            }
        }
    }

    pub fn estimated_tokens(&self) -> usize {
        match self {
            Message::System(t) => t.len() / 4,
            Message::User(t) => t.len() / 4,
            Message::Assistant { text, tool_calls } => {
                let text_tokens = text.as_ref().map(|t| t.len() / 4).unwrap_or(0);
                let tool_tokens: usize = tool_calls.iter()
                    .map(|tc| (tc.name.len() + tc.input.to_string().len()) / 4)
                    .sum();
                text_tokens + tool_tokens
            }
            Message::ToolResult { content, image, .. } => {
                let text_tokens = content.len() / 4;
                let image_tokens = image.as_ref().map(|i| i.base64.len() / 4).unwrap_or(0);
                text_tokens + image_tokens
            }
        }
    }
}
