use uuid::Uuid;

/// Generate a 32-character hex event ID (UUID v4 without dashes)
pub fn generate_event_id() -> String {
    Uuid::new_v4().simple().to_string()
}

/// Generate a 32-character hex public key for DSN
pub fn generate_public_key() -> String {
    Uuid::new_v4().simple().to_string()
}

/// Canonical form of a DebugId: lowercase hex, no dashes / appendix.
///
/// Mach-O / ELF DebugIds round-trip as hyphenated UUIDs from the
/// `debugid` crate, and sometimes arrive with mixed case from clients.
/// Centralizing the normalization here keeps storage keys and event
/// lookups agreeing regardless of source.
pub fn normalize_debug_id(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_hexdigit())
        .flat_map(|c| c.to_lowercase())
        .collect()
}
