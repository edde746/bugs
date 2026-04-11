use axum::{Router, Json, extract::{Path, Query, State}, http::StatusCode, routing::get};
use serde::Deserialize;
use crate::AppState;
use crate::models::event::Event;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/internal/issues/{id}/events", get(list_events_for_issue))
        .route("/api/internal/issues/{id}/events/latest", get(latest_event))
        .route("/api/internal/events/{id}", get(get_event))
}

#[derive(Deserialize)]
struct EventQuery {
    #[serde(default = "default_event_limit")]
    limit: i64,
    cursor: Option<i64>,
}

fn default_event_limit() -> i64 { 50 }

async fn list_events_for_issue(
    State(state): State<AppState>,
    Path(issue_id): Path<i64>,
    Query(params): Query<EventQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let query = if params.cursor.is_some() {
        "SELECT * FROM events WHERE issue_id = ? AND id < ? ORDER BY timestamp DESC LIMIT ?"
    } else {
        "SELECT * FROM events WHERE issue_id = ? ORDER BY timestamp DESC LIMIT ?"
    };

    let events: Vec<Event> = if let Some(cursor) = params.cursor {
        sqlx::query_as(query)
            .bind(issue_id)
            .bind(cursor)
            .bind(params.limit + 1)
            .fetch_all(state.db.reader())
            .await
    } else {
        sqlx::query_as(query)
            .bind(issue_id)
            .bind(params.limit + 1)
            .fetch_all(state.db.reader())
            .await
    }
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let has_next = events.len() as i64 > params.limit;
    let items: Vec<&Event> = events.iter().take(params.limit as usize).collect();
    let next_cursor = if has_next { items.last().map(|e| e.id) } else { None };

    Ok(Json(serde_json::json!({
        "events": items,
        "nextCursor": next_cursor,
    })))
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
