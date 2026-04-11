use bytes::Bytes;
use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EnvelopeError {
    #[error("empty envelope")]
    Empty,
    #[error("invalid envelope header: {0}")]
    InvalidHeader(String),
    #[error("invalid item header: {0}")]
    InvalidItemHeader(String),
    #[error("incomplete item payload")]
    IncompletePayload,
}

#[derive(Debug)]
pub struct Envelope {
    pub headers: EnvelopeHeaders,
    pub items: Vec<EnvelopeItem>,
}

#[derive(Debug, Deserialize)]
pub struct EnvelopeHeaders {
    pub event_id: Option<String>,
    pub dsn: Option<String>,
    pub sent_at: Option<String>,
}

#[derive(Debug)]
pub struct EnvelopeItem {
    pub headers: ItemHeaders,
    pub payload: Bytes,
}

fn default_item_type() -> String {
    "event".to_string()
}

#[derive(Debug, Deserialize)]
pub struct ItemHeaders {
    #[serde(default = "default_item_type", rename = "type")]
    pub item_type: String,
    pub length: Option<usize>,
    pub content_type: Option<String>,
    pub filename: Option<String>,
}

/// Extract only the event_id from the first line of an envelope.
/// Used in the fast path (ingest handler) without full parsing.
pub fn extract_event_id(data: &[u8]) -> Option<String> {
    let first_newline = data.iter().position(|&b| b == b'\n')?;
    let header_line = &data[..first_newline];
    let headers: EnvelopeHeaders = serde_json::from_slice(header_line).ok()?;
    headers.event_id
}

impl Envelope {
    /// Full parse of an envelope from raw bytes
    pub fn parse(data: &[u8]) -> Result<Self, EnvelopeError> {
        if data.is_empty() {
            return Err(EnvelopeError::Empty);
        }

        let first_newline = data
            .iter()
            .position(|&b| b == b'\n')
            .ok_or_else(|| EnvelopeError::InvalidHeader("no newline found".into()))?;

        let header_line = &data[..first_newline];
        let headers: EnvelopeHeaders = serde_json::from_slice(header_line)
            .map_err(|e| EnvelopeError::InvalidHeader(e.to_string()))?;

        let mut items = Vec::new();
        let remaining = &data[first_newline + 1..];
        let mut cursor = 0;

        while cursor < remaining.len() {
            // Skip trailing whitespace/newlines
            if remaining[cursor..]
                .iter()
                .all(|&b| b == b'\n' || b == b'\r')
            {
                break;
            }

            // Find item header line
            let header_end = remaining[cursor..]
                .iter()
                .position(|&b| b == b'\n')
                .ok_or(EnvelopeError::IncompletePayload)?;

            let item_header_bytes = &remaining[cursor..cursor + header_end];
            if item_header_bytes.is_empty() {
                break;
            }

            let item_headers: ItemHeaders = serde_json::from_slice(item_header_bytes)
                .map_err(|e| EnvelopeError::InvalidItemHeader(e.to_string()))?;

            cursor += header_end + 1; // skip past newline

            let payload_len = if let Some(len) = item_headers.length {
                len
            } else {
                // No explicit length: read until next newline
                remaining[cursor..]
                    .iter()
                    .position(|&b| b == b'\n')
                    .unwrap_or(remaining.len() - cursor)
            };

            let end = cursor + payload_len;
            if end > remaining.len() {
                return Err(EnvelopeError::IncompletePayload);
            }

            let payload = Bytes::copy_from_slice(&remaining[cursor..end]);
            cursor = end;

            // Skip trailing newline after payload
            if cursor < remaining.len() && remaining[cursor] == b'\n' {
                cursor += 1;
            }

            items.push(EnvelopeItem {
                headers: item_headers,
                payload,
            });
        }

        Ok(Envelope { headers, items })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_event_id() {
        let data = b"{\"event_id\":\"abc123\"}\n{\"type\":\"event\"}\n{}";
        assert_eq!(extract_event_id(data), Some("abc123".to_string()));
    }

    #[test]
    fn test_extract_event_id_missing() {
        let data = b"{\"dsn\":\"https://key@host/1\"}\n{\"type\":\"event\"}\n{}";
        assert_eq!(extract_event_id(data), None);
    }

    #[test]
    fn test_parse_single_event_with_length() {
        // {"level":"error"} is exactly 16 bytes
        let payload = b"{\"level\":\"error\"}";
        let envelope_str = format!(
            "{{\"event_id\":\"aabb\"}}\n{{\"type\":\"event\",\"length\":{}}}\n{}\n",
            payload.len(),
            std::str::from_utf8(payload).unwrap()
        );
        let parsed = Envelope::parse(envelope_str.as_bytes()).unwrap();
        assert_eq!(parsed.items.len(), 1);
        assert_eq!(parsed.items[0].headers.item_type, "event");
        assert_eq!(parsed.items[0].payload.as_ref(), payload);
    }

    #[test]
    fn test_parse_multiple_items() {
        let envelope = b"{\"event_id\":\"aabb\"}\n\
            {\"type\":\"event\",\"length\":2}\n\
            {}\n\
            {\"type\":\"attachment\",\"length\":5}\nhello\n";
        let parsed = Envelope::parse(envelope).unwrap();
        assert_eq!(parsed.items.len(), 2);
        assert_eq!(parsed.items[0].headers.item_type, "event");
        assert_eq!(parsed.items[1].headers.item_type, "attachment");
        assert_eq!(parsed.items[1].payload.as_ref(), b"hello");
    }

    #[test]
    fn test_parse_no_length_uses_newline() {
        let envelope = b"{\"event_id\":\"aabb\"}\n{\"type\":\"event\"}\n{\"msg\":\"hi\"}\n";
        let parsed = Envelope::parse(envelope).unwrap();
        assert_eq!(parsed.items.len(), 1);
        assert_eq!(parsed.items[0].payload.as_ref(), b"{\"msg\":\"hi\"}");
    }

    #[test]
    fn test_parse_empty_envelope() {
        assert!(Envelope::parse(b"").is_err());
    }

    #[test]
    fn test_parse_header_only() {
        let envelope = b"{\"event_id\":\"aabb\"}\n";
        let parsed = Envelope::parse(envelope).unwrap();
        assert_eq!(parsed.items.len(), 0);
    }
}
