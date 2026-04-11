pub mod admin_auth;
pub mod alerts;
pub mod comments;
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
    Router,
    extract::State,
    http::{StatusCode, header},
    middleware,
    response::Response,
};

pub fn router(state: &AppState) -> Router<AppState> {
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
        ));

    Router::new()
        .merge(ingest::routes())
        .merge(management_routes)
        .merge(frontend::routes())
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
