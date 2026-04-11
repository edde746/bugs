use chrono::{DateTime, Utc};

/// Parse a timestamp from various Sentry formats into ISO 8601
pub fn parse_timestamp(ts: &serde_json::Value) -> Option<String> {
    match ts {
        serde_json::Value::String(s) => {
            // Try ISO 8601 first
            if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
                return Some(
                    dt.with_timezone(&Utc)
                        .format("%Y-%m-%dT%H:%M:%SZ")
                        .to_string(),
                );
            }
            // Try parsing as float string
            if let Ok(f) = s.parse::<f64>() {
                return timestamp_from_float(f);
            }
            None
        }
        serde_json::Value::Number(n) => n.as_f64().and_then(timestamp_from_float),
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_rfc3339() {
        let ts = json!("2026-04-11T14:23:07Z");
        assert_eq!(
            parse_timestamp(&ts),
            Some("2026-04-11T14:23:07Z".to_string())
        );
    }

    #[test]
    fn test_parse_rfc3339_with_offset() {
        let ts = json!("2026-04-11T14:23:07+02:00");
        assert_eq!(
            parse_timestamp(&ts),
            Some("2026-04-11T12:23:07Z".to_string())
        );
    }

    #[test]
    fn test_parse_unix_timestamp_number() {
        let ts = json!(1744380187);
        let result = parse_timestamp(&ts);
        assert!(result.is_some());
        assert!(result.unwrap().contains("2025-"));
    }

    #[test]
    fn test_parse_unix_timestamp_float() {
        let ts = json!(1744380187.5);
        assert!(parse_timestamp(&ts).is_some());
    }

    #[test]
    fn test_parse_invalid() {
        assert_eq!(parse_timestamp(&json!("not a date")), None);
        assert_eq!(parse_timestamp(&json!(null)), None);
        assert_eq!(parse_timestamp(&json!(true)), None);
    }

    #[test]
    fn test_hour_bucket() {
        assert_eq!(hour_bucket("2026-04-11T14:23:07Z"), "2026-04-11T14:00:00Z");
        assert_eq!(hour_bucket("2026-04-11T00:59:59Z"), "2026-04-11T00:00:00Z");
    }

    #[test]
    fn test_now_iso_format() {
        let now = now_iso();
        assert!(now.ends_with('Z'));
        assert!(now.contains('T'));
        assert_eq!(now.len(), 20); // "2026-04-11T14:23:07Z"
    }
}
