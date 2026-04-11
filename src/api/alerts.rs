use axum::{Router, Json, extract::{Path, State}, http::StatusCode, routing::get};
use crate::AppState;
use crate::models::alert::*;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/internal/projects/{id}/alerts", get(list_alerts).post(create_alert))
}

async fn list_alerts(
    State(state): State<AppState>,
    Path(project_id): Path<i64>,
) -> Result<Json<Vec<AlertRule>>, StatusCode> {
    let rules: Vec<AlertRule> = sqlx::query_as(
        "SELECT * FROM alert_rules WHERE project_id = ? ORDER BY created_at DESC"
    )
    .bind(project_id)
    .fetch_all(state.db.reader())
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(rules))
}

async fn create_alert(
    State(state): State<AppState>,
    Path(project_id): Path<i64>,
    Json(input): Json<CreateAlertRule>,
) -> Result<(StatusCode, Json<AlertRule>), (StatusCode, String)> {
    let conditions = serde_json::to_string(&input.conditions)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let actions = serde_json::to_string(&input.actions)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let frequency = input.frequency.unwrap_or(1800);

    let rule = sqlx::query_as::<_, AlertRule>(
        "INSERT INTO alert_rules (project_id, name, conditions, actions, frequency) VALUES (?, ?, ?, ?, ?) RETURNING *"
    )
    .bind(project_id)
    .bind(&input.name)
    .bind(&conditions)
    .bind(&actions)
    .bind(frequency)
    .fetch_one(state.db.writer())
    .await
    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    Ok((StatusCode::CREATED, Json(rule)))
}
