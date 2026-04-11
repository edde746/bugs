/// Legacy /store/ endpoint: accepts a single event JSON body.
/// Wraps it into an envelope-like structure for the worker pipeline.
/// Best-effort compatibility.
pub fn wrap_store_body(body: &[u8], event_id: &str) -> Vec<u8> {
    let header = format!("{{\"event_id\":\"{event_id}\"}}\n");
    let item_header = format!("{{\"type\":\"event\",\"length\":{}}}\n", body.len());

    let mut envelope = Vec::with_capacity(header.len() + item_header.len() + body.len() + 1);
    envelope.extend_from_slice(header.as_bytes());
    envelope.extend_from_slice(item_header.as_bytes());
    envelope.extend_from_slice(body);
    envelope.push(b'\n');
    envelope
}
