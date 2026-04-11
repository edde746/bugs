use subtle::ConstantTimeEq;

/// Check if a request has a valid admin token (constant-time comparison)
pub fn check_admin_token(token: &str, auth_header: Option<&str>) -> bool {
    if token.is_empty() {
        return true; // No token configured
    }
    match auth_header {
        Some(h) => {
            if let Some(bearer) = h.strip_prefix("Bearer ") {
                let candidate = bearer.trim().as_bytes();
                let expected = token.as_bytes();
                candidate.len() == expected.len() && candidate.ct_eq(expected).into()
            } else {
                false
            }
        }
        None => false,
    }
}
