use axum::{Router, Json, extract::{Path, Query, State}, http::StatusCode, routing::get};
use serde::Deserialize;
use crate::AppState;
use crate::models::transaction::*;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/internal/projects/{slug}/transactions", get(list_transaction_groups))
        .route("/api/internal/transaction-groups/{id}", get(get_transaction_group))
        .route("/api/internal/transaction-groups/{id}/transactions", get(list_transactions))
}

#[derive(Deserialize)]
struct TransactionQuery {
    #[serde(default = "default_limit")]
    limit: i64,
    sort: Option<String>,
}

fn default_limit() -> i64 { 50 }

async fn list_transaction_groups(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(params): Query<TransactionQuery>,
) -> Result<Json<Vec<TransactionGroup>>, StatusCode> {
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

    let sort_col = match params.sort.as_deref() {
        Some("count") => "count DESC",
        Some("avg_duration") => "(sum_duration_ms / NULLIF(count, 0)) DESC",
        Some("p95") => "p95_duration_ms DESC",
        Some("error_rate") => "(CAST(error_count AS REAL) / NULLIF(count, 0)) DESC",
        _ => "last_seen DESC",
    };

    let groups: Vec<TransactionGroup> = sqlx::query_as(&format!(
        "SELECT * FROM transaction_groups WHERE project_id = ? ORDER BY {sort_col} LIMIT ?"
    ))
    .bind(project_id)
    .bind(params.limit)
    .fetch_all(state.db.reader())
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(groups))
}

async fn get_transaction_group(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<TransactionGroup>, StatusCode> {
    sqlx::query_as("SELECT * FROM transaction_groups WHERE id = ?")
        .bind(id)
        .fetch_optional(state.db.reader())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn list_transactions(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(params): Query<TransactionQuery>,
) -> Result<Json<Vec<Transaction>>, StatusCode> {
    let txns: Vec<Transaction> = sqlx::query_as(
        "SELECT * FROM transactions WHERE group_id = ? ORDER BY timestamp DESC LIMIT ?",
    )
    .bind(id)
    .bind(params.limit)
    .fetch_all(state.db.reader())
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(txns))
}
