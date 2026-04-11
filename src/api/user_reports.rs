use axum::{Router, Json, extract::{Path, Query, State}, http::StatusCode, routing::get};
use serde::Deserialize;
use crate::AppState;
use crate::models::user_report::UserReport;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/internal/projects/{slug}/user-reports",
            get(list_project_reports),
        )
        .route(
            "/api/internal/issues/{issue_id}/user-reports",
            get(list_issue_reports),
        )
}

#[derive(Deserialize)]
struct ReportQuery {
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 { 50 }

async fn list_project_reports(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(params): Query<ReportQuery>,
) -> Result<Json<Vec<UserReport>>, StatusCode> {
    let project_id: i64 = if let Ok(id) = slug.parse::<i64>() {
        id
    } else {
        let row: Option<(i64,)> = sqlx::query_as("SELECT id FROM projects WHERE slug = ?")
            .bind(&slug)
            .fetch_optional(state.db.reader())
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        row.ok_or(StatusCode::NOT_FOUND)?.0
    };

    let reports: Vec<UserReport> = sqlx::query_as(
        "SELECT * FROM user_reports WHERE project_id = ? ORDER BY created_at DESC LIMIT ?",
    )
    .bind(project_id)
    .bind(params.limit)
    .fetch_all(state.db.reader())
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(reports))
}

async fn list_issue_reports(
    State(state): State<AppState>,
    Path(issue_id): Path<i64>,
) -> Result<Json<Vec<UserReport>>, StatusCode> {
    // Get event_ids for this issue, then find matching reports
    let reports: Vec<UserReport> = sqlx::query_as(
        "SELECT ur.* FROM user_reports ur \
         JOIN events e ON e.event_id = ur.event_id AND e.project_id = ur.project_id \
         WHERE e.issue_id = ? \
         ORDER BY ur.created_at DESC LIMIT 50",
    )
    .bind(issue_id)
    .fetch_all(state.db.reader())
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(reports))
}
