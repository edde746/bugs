use axum::{Router, Json, extract::{Path, State}, http::StatusCode, routing::get};
use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/internal/projects/{slug}/stats", get(project_stats))
        .route("/api/internal/issues/{id}/stats", get(issue_stats))
}

async fn project_stats(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let project: Option<(i64,)> = sqlx::query_as("SELECT id FROM projects WHERE slug = ?")
        .bind(&slug)
        .fetch_optional(state.db.reader())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let project_id = project.ok_or(StatusCode::NOT_FOUND)?.0;

    let stats: Vec<(String, i64)> = sqlx::query_as(
        "SELECT bucket, SUM(count) as total FROM issue_stats_hourly \
         WHERE project_id = ? AND bucket >= strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-14 days') \
         GROUP BY bucket ORDER BY bucket"
    )
    .bind(project_id)
    .fetch_all(state.db.reader())
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(serde_json::json!({ "timeseries": stats })))
}

async fn issue_stats(
    State(state): State<AppState>,
    Path(issue_id): Path<i64>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let stats: Vec<(String, i64)> = sqlx::query_as(
        "SELECT bucket, count FROM issue_stats_hourly \
         WHERE issue_id = ? AND bucket >= strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-14 days') \
         ORDER BY bucket"
    )
    .bind(issue_id)
    .fetch_all(state.db.reader())
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(serde_json::json!({ "timeseries": stats })))
}
