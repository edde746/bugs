use chrono::{DateTime, Utc};

/// Parse a timestamp from various Sentry formats into ISO 8601
pub fn parse_timestamp(ts: &serde_json::Value) -> Option<String> {
    match ts {
        serde_json::Value::String(s) => {
            // Try ISO 8601 first
            if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
                return Some(dt.with_timezone(&Utc).format("%Y-%m-%dT%H:%M:%SZ").to_string());
            }
            // Try parsing as float string
            if let Ok(f) = s.parse::<f64>() {
                return timestamp_from_float(f);
            }
            None
        }
        serde_json::Value::Number(n) => {
            n.as_f64().and_then(timestamp_from_float)
        }
        _ => None,
    }
}

fn timestamp_from_float(f: f64) -> Option<String> {
    let dt = DateTime::from_timestamp(f as i64, ((f.fract()) * 1_000_000_000.0) as u32)?;
    Some(dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
}

pub fn now_iso() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

pub fn hour_bucket(iso: &str) -> String {
    // Truncate to hour: "2026-04-11T14:23:07Z" -> "2026-04-11T14:00:00Z"
    if iso.len() >= 13 {
        format!("{}:00:00Z", &iso[..13])
    } else {
        iso.to_string()
    }
}
