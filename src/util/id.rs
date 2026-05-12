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

/// Inverse of `normalize_debug_id`: insert UUID hyphens into a 32-hex
/// debug id. Used when responding to clients that prefer the canonical
/// 8-4-4-4-12 form (sentry-cli accepts both).
pub fn hyphenate_debug_id(id: &str) -> String {
    if id.len() == 32 && id.chars().all(|c| c.is_ascii_hexdigit()) {
        format!(
            "{}-{}-{}-{}-{}",
            &id[0..8],
            &id[8..12],
            &id[12..16],
            &id[16..20],
            &id[20..32],
        )
    } else {
        id.to_string()
    }
}

/// True for 40-character lowercase-or-mixed-case ASCII-hex strings —
/// the wire format Sentry's chunk-upload protocol uses for SHA1 digests.
pub fn is_sha1_hex(s: &str) -> bool {
    s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit())
}
