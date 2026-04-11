use axum::{Router, Json, extract::{Path, State}, http::StatusCode, routing::get};
use crate::AppState;
use crate::models::event::Event;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/internal/issues/{id}/events", get(list_events_for_issue))
        .route("/api/internal/issues/{id}/events/latest", get(latest_event))
        .route("/api/internal/events/{id}", get(get_event))
}

async fn list_events_for_issue(
    State(state): State<AppState>,
    Path(issue_id): Path<i64>,
) -> Result<Json<Vec<Event>>, StatusCode> {
    let events: Vec<Event> = sqlx::query_as(
        "SELECT * FROM events WHERE issue_id = ? ORDER BY timestamp DESC LIMIT 100"
    )
    .bind(issue_id)
    .fetch_all(state.db.reader())
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(events))
}

async fn latest_event(
    State(state): State<AppState>,
    Path(issue_id): Path<i64>,
) -> Result<Json<Event>, StatusCode> {
    sqlx::query_as(
        "SELECT * FROM events WHERE issue_id = ? ORDER BY timestamp DESC LIMIT 1"
    )
    .bind(issue_id)
    .fetch_optional(state.db.reader())
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .map(Json)
    .ok_or(StatusCode::NOT_FOUND)
}

async fn get_event(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<Event>, StatusCode> {
    sqlx::query_as("SELECT * FROM events WHERE id = ?")
        .bind(id)
        .fetch_optional(state.db.reader())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}
