pub mod ingest;
pub mod ingest_auth;
pub mod admin_auth;
pub mod projects;
pub mod releases;
pub mod issues;
pub mod events;
pub mod alerts;
pub mod stats;
pub mod frontend;

use axum::Router;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .merge(ingest::routes())
        .merge(projects::routes())
        .merge(issues::routes())
        .merge(events::routes())
        .merge(releases::routes())
        .merge(alerts::routes())
        .merge(stats::routes())
        .merge(frontend::routes())
}
