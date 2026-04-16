/// Truncate a string for safe logging. Caps at `max` chars (not bytes,
/// to avoid splitting UTF-8) and appends an ellipsis when truncated.
///
/// Used at log sites that interpolate user-supplied data or upstream
/// error messages, where a malicious or oversized value could otherwise
/// flood logs or leak sensitive payload fragments.
pub fn truncate(s: &str, max: usize) -> String {
    // Fast path: byte length is an upper bound on char count, so if the
    // bytes fit we can skip the UTF-8 walk entirely. Most log inputs
    // (short errors) hit this path.
    if s.len() <= max {
        return s.to_string();
    }
    match s.char_indices().nth(max) {
        Some((idx, _)) => {
            let mut out = String::with_capacity(idx + 3);
            out.push_str(&s[..idx]);
            out.push('…');
            out
        }
        None => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_short_unchanged() {
        assert_eq!(truncate("hi", 10), "hi");
    }

    #[test]
    fn truncate_long_appends_ellipsis() {
        let out = truncate("abcdefghij", 4);
        assert_eq!(out, "abcd…");
    }

    #[test]
    fn truncate_handles_multibyte() {
        // Each emoji is multiple bytes — must not slice mid-codepoint.
        let out = truncate("🦀🦀🦀🦀🦀", 2);
        assert_eq!(out, "🦀🦀…");
    }
}
