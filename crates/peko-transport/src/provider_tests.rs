#[cfg(test)]
mod tests {
    use crate::openai_compat::OpenAICompatProvider;
    use crate::provider::*;
    use crate::stream::*;

    #[test]
    fn tool_result_with_image_emits_tool_then_user_image_message() {
        // Regression test for the screenshot-hallucination bug:
        // when an agent's tool result carries an attached image
        // (screenshot tool returning a base64 PNG), the OpenAI-compat
        // serializer must emit BOTH:
        //   1. role:tool with the text caption
        //   2. role:user with the image_url block
        // Previously the function returned early on the first
        // ToolResult block and silently dropped the image.
        let msg = Message {
            role: "user".to_string(),
            content: MessageContent::Blocks(vec![
                ContentBlock::ToolResult {
                    tool_use_id: "call_xyz".to_string(),
                    content: "Screenshot via screencap (1080x2340, 80KB).".to_string(),
                    is_error: false,
                },
                ContentBlock::Image {
                    source: ImageSource {
                        source_type: "base64".to_string(),
                        media_type: "image/jpeg".to_string(),
                        data: "AAAA".to_string(),
                    },
                },
            ]),
        };
        let out = OpenAICompatProvider::to_openai_message(&msg);
        assert_eq!(out.len(), 2, "must emit tool + user-image");
        assert_eq!(out[0]["role"], "tool");
        assert_eq!(out[0]["tool_call_id"], "call_xyz");
        assert!(out[0]["content"].as_str().unwrap().contains("Screenshot"));
        assert_eq!(out[1]["role"], "user");
        let parts = out[1]["content"].as_array().expect("user content must be array");
        assert!(parts.iter().any(|p| p["type"] == "image_url"));
        let image_url = parts
            .iter()
            .find(|p| p["type"] == "image_url")
            .unwrap()
            ["image_url"]["url"]
            .as_str()
            .unwrap();
        assert!(image_url.starts_with("data:image/jpeg;base64,"));
        assert!(image_url.contains("AAAA"));
    }

    #[test]
    fn tool_result_without_image_emits_single_tool_message() {
        let msg = Message {
            role: "user".to_string(),
            content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "call_no_img".to_string(),
                content: "{ok: true}".to_string(),
                is_error: false,
            }]),
        };
        let out = OpenAICompatProvider::to_openai_message(&msg);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["role"], "tool");
    }

    #[test]
    fn user_text_round_trips_unchanged() {
        let msg = Message {
            role: "user".to_string(),
            content: MessageContent::Text("hello".to_string()),
        };
        let out = OpenAICompatProvider::to_openai_message(&msg);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["role"], "user");
        assert_eq!(out[0]["content"], "hello");
    }

    #[test]
    fn test_message_content_text_serialize() {
        let msg = Message {
            role: "user".to_string(),
            content: MessageContent::Text("hello".to_string()),
        };
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["role"], "user");
        assert_eq!(json["content"], "hello");
    }

    #[test]
    fn test_message_content_blocks_serialize() {
        let msg = Message {
            role: "assistant".to_string(),
            content: MessageContent::Blocks(vec![
                ContentBlock::Text { text: "Let me help.".to_string() },
                ContentBlock::ToolUse {
                    id: "tu_1".to_string(),
                    name: "shell".to_string(),
                    input: serde_json::json!({"command": "ls"}),
                },
            ]),
        };
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["role"], "assistant");
        let blocks = json["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["type"], "text");
        assert_eq!(blocks[1]["type"], "tool_use");
        assert_eq!(blocks[1]["name"], "shell");
    }

    #[test]
    fn test_tool_result_block_serialize() {
        let block = ContentBlock::ToolResult {
            tool_use_id: "tu_1".to_string(),
            content: "file1.txt\nfile2.txt".to_string(),
            is_error: false,
        };
        let json = serde_json::to_value(&block).unwrap();
        assert_eq!(json["type"], "tool_result");
        assert_eq!(json["tool_use_id"], "tu_1");
        assert_eq!(json["is_error"], false);
    }

    #[test]
    fn test_stop_reason_deserialize() {
        let sr: StopReason = serde_json::from_str("\"end_turn\"").unwrap();
        assert_eq!(sr, StopReason::EndTurn);

        let sr: StopReason = serde_json::from_str("\"tool_use\"").unwrap();
        assert_eq!(sr, StopReason::ToolUse);

        let sr: StopReason = serde_json::from_str("\"max_tokens\"").unwrap();
        assert_eq!(sr, StopReason::MaxTokens);

        let sr: StopReason = serde_json::from_str("\"something_new\"").unwrap();
        assert_eq!(sr, StopReason::Unknown);
    }

    #[test]
    fn test_stream_event_variants() {
        let ev = StreamEvent::TextDelta("hello".to_string());
        match ev {
            StreamEvent::TextDelta(t) => assert_eq!(t, "hello"),
            _ => panic!("wrong variant"),
        }

        let ev = StreamEvent::ToolUseStart {
            id: "tu_1".to_string(),
            name: "screenshot".to_string(),
        };
        match ev {
            StreamEvent::ToolUseStart { id, name } => {
                assert_eq!(id, "tu_1");
                assert_eq!(name, "screenshot");
            }
            _ => panic!("wrong variant"),
        }

        let ev = StreamEvent::MessageDelta {
            output_tokens: 42,
            stop_reason: StopReason::ToolUse,
        };
        match ev {
            StreamEvent::MessageDelta { output_tokens, stop_reason } => {
                assert_eq!(output_tokens, 42);
                assert_eq!(stop_reason, StopReason::ToolUse);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_image_source_serialize() {
        let img = ImageSource {
            source_type: "base64".to_string(),
            media_type: "image/png".to_string(),
            data: "iVBOR...".to_string(),
        };
        let json = serde_json::to_value(&img).unwrap();
        assert_eq!(json["type"], "base64");
        assert_eq!(json["media_type"], "image/png");
    }
}
