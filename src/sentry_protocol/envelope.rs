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

#[derive(Debug, Deserialize)]
pub struct ItemHeaders {
    #[serde(rename = "type")]
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

        let first_newline = data.iter().position(|&b| b == b'\n')
            .ok_or_else(|| EnvelopeError::InvalidHeader("no newline found".into()))?;

        let header_line = &data[..first_newline];
        let headers: EnvelopeHeaders = serde_json::from_slice(header_line)
            .map_err(|e| EnvelopeError::InvalidHeader(e.to_string()))?;

        let mut items = Vec::new();
        let remaining = &data[first_newline + 1..];
        let mut cursor = 0;

        while cursor < remaining.len() {
            // Skip trailing whitespace/newlines
            if remaining[cursor..].iter().all(|&b| b == b'\n' || b == b'\r') {
                break;
            }

            // Find item header line
            let header_end = remaining[cursor..].iter().position(|&b| b == b'\n')
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
                remaining[cursor..].iter().position(|&b| b == b'\n')
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

            items.push(EnvelopeItem { headers: item_headers, payload });
        }

        Ok(Envelope { headers, items })
    }
}
