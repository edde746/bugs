use axum::{Router, Json, extract::{Path, Query, State}, http::StatusCode, routing::get};
use serde::Deserialize;
use crate::AppState;
use crate::models::issue::*;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/internal/projects/{slug}/issues", get(list_issues))
        .route("/api/internal/issues/{id}", get(get_issue).put(update_issue).delete(delete_issue))
}

#[derive(Deserialize)]
struct IssueQuery {
    status: Option<String>,
    sort: Option<String>,
    cursor: Option<i64>,
    query: Option<String>,
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 { 25 }

async fn list_issues(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(params): Query<IssueQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let project: Option<(i64,)> = sqlx::query_as(
        "SELECT id FROM projects WHERE slug = ?"
    )
    .bind(&slug)
    .fetch_optional(state.db.reader())
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let project_id = project.ok_or(StatusCode::NOT_FOUND)?.0;

    let status = params.status.as_deref().unwrap_or("unresolved");
    let sort_col = match params.sort.as_deref() {
        Some("first_seen") => "first_seen",
        Some("event_count") => "event_count",
        _ => "last_seen",
    };

    let cursor = params.cursor.unwrap_or(0);

    let issues: Vec<Issue> = sqlx::query_as(&format!(
        "SELECT * FROM issues WHERE project_id = ? AND status = ? AND id > ? ORDER BY {sort_col} DESC LIMIT ?"
    ))
    .bind(project_id)
    .bind(status)
    .bind(cursor)
    .bind(params.limit + 1)
    .fetch_all(state.db.reader())
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let has_next = issues.len() as i64 > params.limit;
    let items: Vec<&Issue> = issues.iter().take(params.limit as usize).collect();
    let next_cursor = items.last().map(|i| i.id);

    Ok(Json(serde_json::json!({
        "issues": items,
        "nextCursor": if has_next { next_cursor } else { None },
    })))
}

async fn get_issue(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<Issue>, StatusCode> {
    sqlx::query_as("SELECT * FROM issues WHERE id = ?")
        .bind(id)
        .fetch_optional(state.db.reader())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn update_issue(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<UpdateIssue>,
) -> Result<Json<Issue>, StatusCode> {
    if let Some(status) = &input.status {
        sqlx::query("UPDATE issues SET status = ? WHERE id = ?")
            .bind(status)
            .bind(id)
            .execute(state.db.writer())
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    get_issue(State(state), Path(id)).await
}

async fn delete_issue(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> StatusCode {
    match sqlx::query("DELETE FROM issues WHERE id = ?")
        .bind(id)
        .execute(state.db.writer())
        .await
    {
        Ok(r) if r.rows_affected() > 0 => StatusCode::NO_CONTENT,
        _ => StatusCode::NOT_FOUND,
    }
}
