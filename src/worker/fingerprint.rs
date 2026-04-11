use crate::sentry_protocol::types::SentryEvent;
use crate::util::hash::fingerprint_hash;

/// Compute the grouping fingerprint for an event
pub fn compute_fingerprint(event: &SentryEvent) -> String {
    // 1. Client-provided fingerprint
    if let Some(fp) = &event.fingerprint
        && !fp.is_empty()
    {
        let parts: Vec<String> = fp
            .iter()
            .map(|v| match v {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .collect();
        let parts_ref: Vec<&str> = parts.iter().map(|s| s.as_str()).collect();
        return fingerprint_hash(&parts_ref);
    }

    // 2. Exception-based: type + in-app stacktrace
    if let Some(exc) = &event.exception
        && let Some(first) = exc.values.first()
    {
        let exc_type = first.exception_type.as_deref().unwrap_or("");
        let exc_value = first.value.as_deref().unwrap_or("");

        if let Some(st) = &first.stacktrace {
            let in_app_frames: Vec<String> = st
                .frames
                .iter()
                .filter(|f| f.in_app.unwrap_or(false))
                .map(|f| {
                    format!(
                        "{}:{}",
                        f.function.as_deref().unwrap_or("?"),
                        f.filename.as_deref().unwrap_or("?")
                    )
                })
                .collect();

            if !in_app_frames.is_empty() {
                let frames_str = in_app_frames.join("|");
                return fingerprint_hash(&[exc_type, &frames_str]);
            }
        }

        // Exception type + value only
        if !exc_type.is_empty() {
            return fingerprint_hash(&[exc_type, exc_value]);
        }
    }

    // 3. Message-based
    if let Some(msg) = event.message.as_deref() {
        let logger = event.logger.as_deref().unwrap_or("");
        return fingerprint_hash(&[msg, logger]);
    }

    if let Some(le) = &event.logentry
        && let Some(msg) = le.message.as_deref()
    {
        let logger = event.logger.as_deref().unwrap_or("");
        return fingerprint_hash(&[msg, logger]);
    }

    // 4. Fallback: hash of level + platform
    let level = event.level.as_deref().unwrap_or("error");
    let platform = event.platform.as_deref().unwrap_or("other");
    fingerprint_hash(&["unknown", level, platform])
}

/// Derive issue title from the event
pub fn derive_title(event: &SentryEvent) -> String {
    if let Some(exc) = &event.exception
        && let Some(first) = exc.values.first()
    {
        let t = first.exception_type.as_deref().unwrap_or("Error");
        let v = first.value.as_deref().unwrap_or("");
        if v.is_empty() {
            return t.to_string();
        }
        return format!("{t}: {v}");
    }

    if let Some(msg) = event.message.as_deref() {
        return msg.chars().take(200).collect();
    }

    if let Some(le) = &event.logentry
        && let Some(msg) = le.message.as_deref()
    {
        return msg.chars().take(200).collect();
    }

    "<untitled>".to_string()
}

/// Derive culprit (short location) from the event
pub fn derive_culprit(event: &SentryEvent) -> Option<String> {
    if let Some(exc) = &event.exception {
        for ev in &exc.values {
            if let Some(st) = &ev.stacktrace
                && let Some(frame) = st.frames.iter().rev().find(|f| f.in_app.unwrap_or(false))
            {
                let func = frame.function.as_deref().unwrap_or("?");
                let file = frame.filename.as_deref().unwrap_or("?");
                return Some(format!("{file} in {func}"));
            }
        }
    }
    event.transaction.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sentry_protocol::types::*;

    fn make_exception_event(exc_type: &str, value: &str, func: &str) -> SentryEvent {
        SentryEvent {
            exception: Some(ExceptionInterface {
                values: vec![ExceptionValue {
                    exception_type: Some(exc_type.to_string()),
                    value: Some(value.to_string()),
                    module: None,
                    stacktrace: Some(Stacktrace {
                        frames: vec![StackFrame {
                            filename: Some("app.js".to_string()),
                            function: Some(func.to_string()),
                            module: None,
                            lineno: Some(42),
                            colno: None,
                            abs_path: None,
                            in_app: Some(true),
                            context_line: None,
                            pre_context: None,
                            post_context: None,
                        }],
                    }),
                }],
            }),
            ..Default::default()
        }
    }

    #[test]
    fn test_same_exception_same_fingerprint() {
        let e1 = make_exception_event("TypeError", "null ref", "handleClick");
        let e2 = make_exception_event("TypeError", "null ref", "handleClick");
        assert_eq!(compute_fingerprint(&e1), compute_fingerprint(&e2));
    }

    #[test]
    fn test_different_exception_different_fingerprint() {
        let e1 = make_exception_event("TypeError", "null ref", "handleClick");
        let e2 = make_exception_event("RangeError", "out of bounds", "process");
        assert_ne!(compute_fingerprint(&e1), compute_fingerprint(&e2));
    }

    #[test]
    fn test_client_fingerprint_overrides() {
        let mut e1 = make_exception_event("TypeError", "a", "fn1");
        let mut e2 = make_exception_event("RangeError", "b", "fn2");
        e1.fingerprint = Some(vec![serde_json::Value::String("custom-group".to_string())]);
        e2.fingerprint = Some(vec![serde_json::Value::String("custom-group".to_string())]);
        assert_eq!(compute_fingerprint(&e1), compute_fingerprint(&e2));
    }

    #[test]
    fn test_message_fingerprint() {
        let e1 = SentryEvent {
            message: Some("Connection timeout".to_string()),
            ..Default::default()
        };
        let e2 = SentryEvent {
            message: Some("Connection timeout".to_string()),
            ..Default::default()
        };
        assert_eq!(compute_fingerprint(&e1), compute_fingerprint(&e2));

        let e3 = SentryEvent {
            message: Some("Different message".to_string()),
            ..Default::default()
        };
        assert_ne!(compute_fingerprint(&e1), compute_fingerprint(&e3));
    }

    #[test]
    fn test_derive_title_exception() {
        let e = make_exception_event("TypeError", "Cannot read x", "fn");
        assert_eq!(derive_title(&e), "TypeError: Cannot read x");
    }

    #[test]
    fn test_derive_title_message() {
        let e = SentryEvent {
            message: Some("Hello world".to_string()),
            ..Default::default()
        };
        assert_eq!(derive_title(&e), "Hello world");
    }

    #[test]
    fn test_derive_title_empty() {
        let e = SentryEvent::default();
        assert_eq!(derive_title(&e), "<untitled>");
    }

    #[test]
    fn test_derive_culprit() {
        let e = make_exception_event("TypeError", "err", "handleClick");
        assert_eq!(
            derive_culprit(&e),
            Some("app.js in handleClick".to_string())
        );
    }

    #[test]
    fn test_derive_culprit_no_exception() {
        let e = SentryEvent {
            transaction: Some("/api/users".to_string()),
            ..Default::default()
        };
        assert_eq!(derive_culprit(&e), Some("/api/users".to_string()));
    }
}
