use axum::{Router, Json, extract::{Path, State}, http::StatusCode, routing::get};
use crate::AppState;
use crate::models::comment::*;
use crate::models::activity::IssueActivity;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/internal/issues/{issue_id}/comments",
            get(list_comments).post(create_comment),
        )
        .route(
            "/api/internal/issues/{issue_id}/activity",
            get(list_activity),
        )
        .route(
            "/api/internal/comments/{id}",
            axum::routing::delete(delete_comment),
        )
}

async fn list_comments(
    State(state): State<AppState>,
    Path(issue_id): Path<i64>,
) -> Result<Json<Vec<IssueComment>>, StatusCode> {
    let comments: Vec<IssueComment> = sqlx::query_as(
        "SELECT * FROM issue_comments WHERE issue_id = ? ORDER BY created_at ASC",
    )
    .bind(issue_id)
    .fetch_all(state.db.reader())
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(comments))
}

async fn create_comment(
    State(state): State<AppState>,
    Path(issue_id): Path<i64>,
    Json(input): Json<CreateComment>,
) -> Result<(StatusCode, Json<IssueComment>), StatusCode> {
    let comment: IssueComment = sqlx::query_as(
        "INSERT INTO issue_comments (issue_id, text) VALUES (?, ?) RETURNING *",
    )
    .bind(issue_id)
    .bind(&input.text)
    .fetch_one(state.db.writer())
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok((StatusCode::CREATED, Json(comment)))
}

async fn list_activity(
    State(state): State<AppState>,
    Path(issue_id): Path<i64>,
) -> Result<Json<Vec<IssueActivity>>, StatusCode> {
    let activity: Vec<IssueActivity> = sqlx::query_as(
        "SELECT * FROM issue_activity WHERE issue_id = ? ORDER BY created_at DESC LIMIT 100",
    )
    .bind(issue_id)
    .fetch_all(state.db.reader())
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(activity))
}

async fn delete_comment(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> StatusCode {
    match sqlx::query("DELETE FROM issue_comments WHERE id = ?")
        .bind(id)
        .execute(state.db.writer())
        .await
    {
        Ok(r) if r.rows_affected() > 0 => StatusCode::NO_CONTENT,
        _ => StatusCode::NOT_FOUND,
    }
}
