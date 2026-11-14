//! A Server-Sent Events parser: bytes in, `(event, data)` pairs out.
//!
//! Chunks from the network split anywhere, including inside a UTF-8 character, so bytes are
//! buffered until a whole line is present.

#[derive(Default)]
pub struct SseParser {
    buffer: Vec<u8>,
    event: String,
    data: String,
}

#[derive(Debug, PartialEq)]
pub struct SseEvent {
    /// The `event:` field, or empty when the stream sends data only.
    pub event: String,
    pub data: String,
}

impl SseParser {
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buffer.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some(end) = self.buffer.iter().position(|&byte| byte == b'\n') {
            let mut line: Vec<u8> = self.buffer.drain(..=end).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let line = String::from_utf8_lossy(&line);
            if line.is_empty() {
                if !self.data.is_empty() || !self.event.is_empty() {
                    events.push(SseEvent {
                        event: std::mem::take(&mut self.event),
                        data: std::mem::take(&mut self.data),
                    });
                }
            } else if let Some(value) = field(&line, "data") {
                if !self.data.is_empty() {
                    self.data.push('\n');
                }
                self.data.push_str(value);
            } else if let Some(value) = field(&line, "event") {
                self.event = value.to_string();
            }
        }
        events
    }
}

fn field<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(name)?.strip_prefix(':')?;
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_events_split_across_chunks() {
        let mut parser = SseParser::default();
        assert!(parser.push(b"event: message_start\ndata: {\"a\"").is_empty());
        let events = parser.push(b":1}\n\ndata: [DONE]\r\n\r\n");
        assert_eq!(
            events,
            vec![
                SseEvent { event: "message_start".into(), data: "{\"a\":1}".into() },
                SseEvent { event: String::new(), data: "[DONE]".into() },
            ]
        );
    }

    #[test]
    fn keeps_a_character_split_between_chunks() {
        let mut parser = SseParser::default();
        let bytes = "data: caf\u{e9}\n\n".as_bytes();
        let split = bytes.len() - 3;
        assert!(parser.push(&bytes[..split]).is_empty());
        assert_eq!(parser.push(&bytes[split..])[0].data, "caf\u{e9}");
    }

    #[test]
    fn ignores_comments_and_joins_data_lines() {
        let mut parser = SseParser::default();
        let events = parser.push(b": keep-alive\n\ndata: one\ndata: two\n\n");
        assert_eq!(events, vec![SseEvent { event: String::new(), data: "one\ntwo".into() }]);
    }
}
