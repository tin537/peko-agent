#[derive(Debug, Clone)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
    pub id: Option<String>,
}

pub struct SseParser {
    buffer: String,
}

impl SseParser {
    pub fn new() -> Self {
        Self { buffer: String::new() }
    }

    pub fn feed(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        let text = String::from_utf8_lossy(chunk);
        self.buffer.push_str(&text);

        let mut events = Vec::new();

        loop {
            let Some(end) = self.buffer.find("\n\n") else { break };
            let block = self.buffer[..end].to_string();
            self.buffer = self.buffer[end + 2..].to_string();

            if let Some(event) = Self::parse_block(&block) {
                events.push(event);
            }
        }

        events
    }

    fn parse_block(block: &str) -> Option<SseEvent> {
        let mut event_type = None;
        let mut data_lines: Vec<&str> = Vec::new();
        let mut id = None;

        for line in block.lines() {
            if line.starts_with("event:") {
                event_type = Some(line["event:".len()..].trim().to_string());
            } else if line.starts_with("data:") {
                let val = line["data:".len()..].trim_start();
                if val == "[DONE]" {
                    return None;
                }
                data_lines.push(val);
            } else if line.starts_with("id:") {
                id = Some(line["id:".len()..].trim().to_string());
            } else if line.starts_with(':') {
                // comment, skip
            }
        }

        if data_lines.is_empty() {
            return None;
        }

        Some(SseEvent {
            event: event_type,
            data: data_lines.join("\n"),
            id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_event() {
        let mut parser = SseParser::new();
        let events = parser.feed(b"event: message_start\ndata: {\"type\":\"message_start\"}\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event.as_deref(), Some("message_start"));
    }

    #[test]
    fn test_chunked_event() {
        let mut parser = SseParser::new();
        let events1 = parser.feed(b"event: text\nda");
        assert!(events1.is_empty());
        let events2 = parser.feed(b"ta: hello\n\n");
        assert_eq!(events2.len(), 1);
        assert_eq!(events2[0].data, "hello");
    }

    #[test]
    fn test_multiple_events() {
        let mut parser = SseParser::new();
        let input = b"data: first\n\ndata: second\n\n";
        let events = parser.feed(input);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].data, "first");
        assert_eq!(events[1].data, "second");
    }

    #[test]
    fn test_done_sentinel() {
        let mut parser = SseParser::new();
        let events = parser.feed(b"data: [DONE]\n\n");
        assert!(events.is_empty());
    }

    #[test]
    fn test_multiline_data() {
        let mut parser = SseParser::new();
        let events = parser.feed(b"data: line1\ndata: line2\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "line1\nline2");
    }
}
