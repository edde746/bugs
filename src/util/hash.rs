use sha2::{Sha256, Digest};

pub fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    hex::encode(result)
}

/// Compute a fingerprint hash from a list of strings
pub fn fingerprint_hash(parts: &[&str]) -> String {
    let combined = parts.join("|");
    sha256_hex(combined.as_bytes())
}
