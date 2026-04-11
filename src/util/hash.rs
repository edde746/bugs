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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha256_hex_known_value() {
        // SHA256 of empty string
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn test_sha256_hex_deterministic() {
        assert_eq!(sha256_hex(b"hello"), sha256_hex(b"hello"));
        assert_ne!(sha256_hex(b"hello"), sha256_hex(b"world"));
    }

    #[test]
    fn test_fingerprint_hash_deterministic() {
        let fp1 = fingerprint_hash(&["TypeError", "handleClick"]);
        let fp2 = fingerprint_hash(&["TypeError", "handleClick"]);
        assert_eq!(fp1, fp2);
    }

    #[test]
    fn test_fingerprint_hash_order_matters() {
        let fp1 = fingerprint_hash(&["a", "b"]);
        let fp2 = fingerprint_hash(&["b", "a"]);
        assert_ne!(fp1, fp2);
    }

    #[test]
    fn test_fingerprint_hash_length() {
        let fp = fingerprint_hash(&["test"]);
        assert_eq!(fp.len(), 64); // SHA256 hex = 64 chars
    }
}
