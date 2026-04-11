use crate::sentry_protocol::types::SentryEvent;
use crate::util::time::{parse_timestamp, now_iso};
use crate::util::id::generate_event_id;

/// Normalize a raw Sentry event: fill defaults, parse timestamps
pub fn normalize(event: &mut SentryEvent) {
    // Ensure event_id
    if event.event_id.is_none() || event.event_id.as_deref() == Some("") {
        event.event_id = Some(generate_event_id());
    }

    // Normalize timestamp
    if let Some(ts) = &event.timestamp
        && let Some(parsed) = parse_timestamp(ts)
    {
        event.timestamp = Some(serde_json::Value::String(parsed));
    }
    if event.timestamp.is_none() {
        event.timestamp = Some(serde_json::Value::String(now_iso()));
    }

    // Default level
    if event.level.is_none() {
        event.level = Some("error".to_string());
    }
}
