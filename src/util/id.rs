use uuid::Uuid;

/// Generate a 32-character hex event ID (UUID v4 without dashes)
pub fn generate_event_id() -> String {
    Uuid::new_v4().simple().to_string()
}

/// Generate a 32-character hex public key for DSN
pub fn generate_public_key() -> String {
    Uuid::new_v4().simple().to_string()
}
