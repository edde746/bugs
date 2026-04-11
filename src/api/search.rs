use axum::{Router, Json, extract::{Query, State}, http::StatusCode, routing::get};
use serde::Deserialize;
use crate::AppState;
use crate::models::event::Event;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/internal/search", get(search_events))
}

#[derive(Debug, Deserialize)]
struct SearchParams {
    q: String,
    project: Option<i64>,
}

async fn search_events(
    State(state): State<AppState>,
    Query(params): Query<SearchParams>,
) -> Result<Json<Vec<Event>>, (StatusCode, String)> {
    if params.q.len() < 2 {
        return Err((StatusCode::BAD_REQUEST, "Query must be at least 2 characters".to_string()));
    }

    // Escape FTS5 special characters and wrap in quotes for safe matching
    let fts_query = format!("\"{}\"", params.q.replace('"', "\"\""));

    let events: Vec<Event> = if let Some(project_id) = params.project {
        sqlx::query_as(
            "SELECT events.* FROM events_fts \
             JOIN events ON events.id = events_fts.rowid \
             WHERE events_fts MATCH ? AND events.project_id = ? \
             ORDER BY rank LIMIT 50",
        )
        .bind(&fts_query)
        .bind(project_id)
        .fetch_all(state.db.reader())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    } else {
        sqlx::query_as(
            "SELECT events.* FROM events_fts \
             JOIN events ON events.id = events_fts.rowid \
             WHERE events_fts MATCH ? \
             ORDER BY rank LIMIT 50",
        )
        .bind(&fts_query)
        .fetch_all(state.db.reader())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    };

    Ok(Json(events))
}
