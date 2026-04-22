pub mod admin_auth;
pub mod alerts;
pub mod comments;
pub mod dsyms;
pub mod events;
pub mod frontend;
pub mod ingest;
pub mod ingest_auth;
pub mod issues;
pub mod performance;
pub mod projects;
pub mod releases;
pub mod search;
pub mod stats;
pub mod user_reports;

use crate::AppState;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{StatusCode, header},
    middleware,
    response::Response,
    routing::{get, post},
};

pub fn router(state: &AppState) -> Router<AppState> {
    // Public health endpoint — outside auth middleware so container
    // orchestrators (Docker HEALTHCHECK, Kubernetes livenessProbe) can
    // poll it without a token. Intentionally minimal: no DB round-trip.
    let health_routes = Router::new().route("/api/health", get(health_check));

    // Auth routes - outside auth middleware so they're always accessible
    let auth_routes = Router::new()
        .route("/api/internal/auth/status", get(auth_status))
        .route("/api/internal/auth/check", post(auth_check));

    // Management routes that require admin auth
    let management_routes = Router::new()
        .merge(projects::routes())
        .merge(issues::routes())
        .merge(events::routes())
        .merge(releases::routes())
        .merge(alerts::routes())
        .merge(stats::routes())
        .merge(search::routes())
        .merge(comments::routes())
        .merge(user_reports::routes())
        .merge(performance::routes())
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            admin_auth_check,
        ))
        // Admin-authenticated uploads (dSYM bundles, release files) can
        // legitimately exceed the ingest cap. Per-file ceilings are still
        // enforced in the handlers via `max_attachment_bytes`.
        .layer(DefaultBodyLimit::disable());

    // Scope the ingest body cap to the ingest router only. axum's default
    // is 2 MiB, which silently truncates the Bytes/Multipart extractors —
    // the handler-level size checks (max_raw_request_bytes,
    // max_attachment_bytes) never fire because the extractor has already
    // rejected the request. Multipart is especially opaque: the client
    // sees a bare "Error parsing multipart/form-data request".
    let ingest_body_limit = state
        .config
        .ingest
        .max_raw_request_bytes
        .max(state.config.ingest.max_envelope_bytes);

    Router::new()
        .merge(health_routes)
        .merge(ingest::routes().layer(DefaultBodyLimit::max(ingest_body_limit)))
        .merge(auth_routes)
        .merge(management_routes)
        .merge(frontend::routes())
}

async fn health_check() -> StatusCode {
    StatusCode::OK
}

async fn auth_status(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "auth_required": !state.config.auth.admin_token.is_empty()
    }))
}

async fn auth_check(State(state): State<AppState>, request: axum::extract::Request) -> StatusCode {
    let token = &state.config.auth.admin_token;
    if token.is_empty() {
        return StatusCode::OK;
    }
    let auth_header = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    if admin_auth::check_admin_token(token, auth_header) {
        StatusCode::OK
    } else {
        StatusCode::UNAUTHORIZED
    }
}

async fn admin_auth_check(
    State(state): State<AppState>,
    request: axum::extract::Request,
    next: middleware::Next,
) -> Result<Response, StatusCode> {
    let token = &state.config.auth.admin_token;

    // If no token configured, allow all requests
    if token.is_empty() {
        return Ok(next.run(request).await);
    }

    // Check Authorization header
    let auth_header = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    if admin_auth::check_admin_token(token, auth_header) {
        Ok(next.run(request).await)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}
