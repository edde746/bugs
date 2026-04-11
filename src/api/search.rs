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
    project: Option<String>,
}

async fn search_events(
    State(state): State<AppState>,
    Query(params): Query<SearchParams>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if params.q.len() < 2 {
        return Err((StatusCode::BAD_REQUEST, "Query must be at least 2 characters".to_string()));
    }

    // Resolve project param: could be numeric id or slug
    let project_id: Option<i64> = if let Some(ref project) = params.project {
        if let Ok(id) = project.parse::<i64>() {
            Some(id)
        } else {
            let row: Option<(i64,)> = sqlx::query_as(
                "SELECT id FROM projects WHERE slug = ?"
            )
            .bind(project)
            .fetch_optional(state.db.reader())
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            Some(row.ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?.0)
        }
    } else {
        None
    };

    // Escape FTS5 special characters and wrap in quotes for safe matching
    let fts_query = format!("\"{}\"", params.q.replace('"', "\"\""));

    let events: Vec<Event> = if let Some(pid) = project_id {
        sqlx::query_as(
            "SELECT events.* FROM events_fts \
             JOIN events ON events.id = events_fts.rowid \
             WHERE events_fts MATCH ? AND events.project_id = ? \
             ORDER BY rank LIMIT 50",
        )
        .bind(&fts_query)
        .bind(pid)
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

    Ok(Json(serde_json::json!({ "results": events })))
}
