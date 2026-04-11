use axum::{Router, Json, extract::{Path, State}, http::StatusCode, routing::get};
use serde::Deserialize;
use crate::AppState;
use crate::models::alert::*;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/internal/projects/{id}/alerts", get(list_alerts).post(create_alert))
        .route("/api/internal/projects/{id}/alerts/{alert_id}",
            axum::routing::put(update_alert).delete(delete_alert))
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

#[derive(Debug, Deserialize)]
struct UpdateAlertRule {
    name: Option<String>,
    enabled: Option<bool>,
    conditions: Option<Vec<AlertCondition>>,
    actions: Option<Vec<AlertAction>>,
    frequency: Option<i64>,
}

async fn update_alert(
    State(state): State<AppState>,
    Path((project_id, alert_id)): Path<(i64, i64)>,
    Json(input): Json<UpdateAlertRule>,
) -> Result<Json<AlertRule>, (StatusCode, String)> {
    // Verify alert exists and belongs to project
    let existing: Option<AlertRule> = sqlx::query_as(
        "SELECT * FROM alert_rules WHERE id = ? AND project_id = ?"
    )
    .bind(alert_id)
    .bind(project_id)
    .fetch_optional(state.db.reader())
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if existing.is_none() {
        return Err((StatusCode::NOT_FOUND, "Alert rule not found".to_string()));
    }

    let existing = existing.unwrap();

    let name = input.name.unwrap_or(existing.name);
    let enabled = input.enabled.unwrap_or(existing.enabled);
    let frequency = input.frequency.unwrap_or(existing.frequency);

    let conditions = if let Some(c) = input.conditions {
        serde_json::to_string(&c).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
    } else {
        existing.conditions
    };

    let actions = if let Some(a) = input.actions {
        serde_json::to_string(&a).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
    } else {
        existing.actions
    };

    let rule = sqlx::query_as::<_, AlertRule>(
        "UPDATE alert_rules SET name = ?, enabled = ?, conditions = ?, actions = ?, frequency = ? \
         WHERE id = ? AND project_id = ? RETURNING *"
    )
    .bind(&name)
    .bind(enabled)
    .bind(&conditions)
    .bind(&actions)
    .bind(frequency)
    .bind(alert_id)
    .bind(project_id)
    .fetch_one(state.db.writer())
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(rule))
}

async fn delete_alert(
    State(state): State<AppState>,
    Path((project_id, alert_id)): Path<(i64, i64)>,
) -> Result<StatusCode, (StatusCode, String)> {
    let result = sqlx::query(
        "DELETE FROM alert_rules WHERE id = ? AND project_id = ?"
    )
    .bind(alert_id)
    .bind(project_id)
    .execute(state.db.writer())
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "Alert rule not found".to_string()));
    }

    Ok(StatusCode::NO_CONTENT)
}
