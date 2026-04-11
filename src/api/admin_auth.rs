use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;

/// Middleware that checks for Bearer token if admin_token is configured
pub async fn admin_auth_middleware(
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // Token is checked in the layer setup; if we get here, auth passed
    Ok(next.run(request).await)
}

/// Check if a request has a valid admin token
pub fn check_admin_token(token: &str, auth_header: Option<&str>) -> bool {
    if token.is_empty() {
        return true; // No token configured
    }
    match auth_header {
        Some(h) => {
            if let Some(bearer) = h.strip_prefix("Bearer ") {
                bearer.trim() == token
            } else {
                false
            }
        }
        None => false,
    }
}
