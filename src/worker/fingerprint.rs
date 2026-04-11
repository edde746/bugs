use crate::sentry_protocol::types::SentryEvent;
use crate::util::hash::fingerprint_hash;

/// Compute the grouping fingerprint for an event
pub fn compute_fingerprint(event: &SentryEvent) -> String {
    // 1. Client-provided fingerprint
    if let Some(fp) = &event.fingerprint {
        if !fp.is_empty() {
            let parts: Vec<&str> = fp.iter().map(|s| s.as_str()).collect();
            return fingerprint_hash(&parts);
        }
    }

    // 2. Exception-based: type + in-app stacktrace
    if let Some(exc) = &event.exception {
        if let Some(first) = exc.values.first() {
            let exc_type = first.exception_type.as_deref().unwrap_or("");
            let exc_value = first.value.as_deref().unwrap_or("");

            if let Some(st) = &first.stacktrace {
                let in_app_frames: Vec<String> = st.frames.iter()
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
    }

    // 3. Message-based
    if let Some(msg) = event.message.as_deref() {
        let logger = event.logger.as_deref().unwrap_or("");
        return fingerprint_hash(&[msg, logger]);
    }

    if let Some(le) = &event.logentry {
        if let Some(msg) = le.message.as_deref() {
            let logger = event.logger.as_deref().unwrap_or("");
            return fingerprint_hash(&[msg, logger]);
        }
    }

    // 4. Fallback: hash of level + platform
    let level = event.level.as_deref().unwrap_or("error");
    let platform = event.platform.as_deref().unwrap_or("other");
    fingerprint_hash(&["unknown", level, platform])
}

/// Derive issue title from the event
pub fn derive_title(event: &SentryEvent) -> String {
    if let Some(exc) = &event.exception {
        if let Some(first) = exc.values.first() {
            let t = first.exception_type.as_deref().unwrap_or("Error");
            let v = first.value.as_deref().unwrap_or("");
            if v.is_empty() {
                return t.to_string();
            }
            return format!("{t}: {v}");
        }
    }

    if let Some(msg) = event.message.as_deref() {
        return msg.chars().take(200).collect();
    }

    if let Some(le) = &event.logentry {
        if let Some(msg) = le.message.as_deref() {
            return msg.chars().take(200).collect();
        }
    }

    "<untitled>".to_string()
}

/// Derive culprit (short location) from the event
pub fn derive_culprit(event: &SentryEvent) -> Option<String> {
    if let Some(exc) = &event.exception {
        for ev in &exc.values {
            if let Some(st) = &ev.stacktrace {
                // Find the last in-app frame
                if let Some(frame) = st.frames.iter().rev().find(|f| f.in_app.unwrap_or(false)) {
                    let func = frame.function.as_deref().unwrap_or("?");
                    let file = frame.filename.as_deref().unwrap_or("?");
                    return Some(format!("{file} in {func}"));
                }
            }
        }
    }
    event.transaction.clone()
}
