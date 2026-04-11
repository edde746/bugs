use axum::{Router, Json, extract::{Path, Query, State}, http::StatusCode, routing::{get, post}};
use serde::Deserialize;
use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/internal/projects/{slug}/stats", get(project_stats))
        .route("/api/internal/issues/{id}/stats", get(issue_stats))
        .route("/api/internal/projects/{slug}/tags", get(list_tag_keys))
        .route("/api/internal/projects/{slug}/tags/{key}/values", get(list_tag_values))
        .route("/api/internal/cleanup", post(manual_cleanup))
        .route("/health/ready", get(health_ready))
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

async fn list_tag_keys(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<Vec<serde_json::Value>>, StatusCode> {
    let project: Option<(i64,)> = sqlx::query_as("SELECT id FROM projects WHERE slug = ?")
        .bind(&slug)
        .fetch_optional(state.db.reader())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let project_id = project.ok_or(StatusCode::NOT_FOUND)?.0;

    let keys: Vec<(String, i64)> = sqlx::query_as(
        "SELECT key, values_seen FROM tag_keys WHERE project_id = ? ORDER BY values_seen DESC"
    )
    .bind(project_id)
    .fetch_all(state.db.reader())
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let result: Vec<serde_json::Value> = keys.iter().map(|(k, v)| {
        serde_json::json!({"key": k, "values_seen": v})
    }).collect();

    Ok(Json(result))
}

#[derive(Deserialize)]
struct TagValuesQuery {
    #[serde(default = "default_tag_limit")]
    limit: i64,
}
fn default_tag_limit() -> i64 { 100 }

async fn list_tag_values(
    State(state): State<AppState>,
    Path((slug, key)): Path<(String, String)>,
    Query(params): Query<TagValuesQuery>,
) -> Result<Json<Vec<serde_json::Value>>, StatusCode> {
    let project: Option<(i64,)> = sqlx::query_as("SELECT id FROM projects WHERE slug = ?")
        .bind(&slug)
        .fetch_optional(state.db.reader())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let project_id = project.ok_or(StatusCode::NOT_FOUND)?.0;

    let values: Vec<(String, i64, String)> = sqlx::query_as(
        "SELECT value, times_seen, last_seen FROM tag_values \
         WHERE project_id = ? AND key = ? ORDER BY times_seen DESC LIMIT ?"
    )
    .bind(project_id)
    .bind(&key)
    .bind(params.limit)
    .fetch_all(state.db.reader())
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let result: Vec<serde_json::Value> = values.iter().map(|(v, count, last)| {
        serde_json::json!({"value": v, "times_seen": count, "last_seen": last})
    }).collect();

    Ok(Json(result))
}

async fn manual_cleanup(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    crate::db::retention::run_cleanup_now(
        state.db.writer(),
        state.config.retention_days,
        state.config.envelope_retention_hours,
    ).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(serde_json::json!({"status": "ok"})))
}

async fn health_ready(
    State(state): State<AppState>,
) -> Result<&'static str, StatusCode> {
    // Check DB is accessible
    sqlx::query("SELECT 1")
        .execute(state.db.reader())
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok("ready")
}
