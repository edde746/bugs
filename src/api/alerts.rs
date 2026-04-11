use crate::AppState;
use crate::models::alert::*;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use serde::{Deserialize, Serialize};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/internal/projects/{slug}/alerts",
            get(list_alerts).post(create_alert),
        )
        .route(
            "/api/internal/projects/{slug}/alerts/{alert_id}",
            axum::routing::put(update_alert).delete(delete_alert),
        )
}

async fn resolve_project(state: &AppState, slug: &str) -> Result<i64, (StatusCode, String)> {
    // Accept numeric id or slug
    if let Ok(id) = slug.parse::<i64>() {
        return Ok(id);
    }
    let project: Option<(i64,)> = sqlx::query_as("SELECT id FROM projects WHERE slug = ?")
        .bind(slug)
        .fetch_optional(state.db.reader())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(project
        .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?
        .0)
}

/// API response with parsed conditions/actions (not raw JSON strings)
#[derive(Serialize)]
struct AlertRuleResponse {
    id: i64,
    project_id: i64,
    name: String,
    enabled: bool,
    conditions: Vec<AlertCondition>,
    actions: Vec<AlertAction>,
    frequency: i64,
    last_fired: Option<String>,
    created_at: String,
}

fn rule_to_response(rule: &AlertRule) -> AlertRuleResponse {
    let conditions: Vec<AlertCondition> =
        serde_json::from_str(&rule.conditions).unwrap_or_default();
    let actions: Vec<AlertAction> = serde_json::from_str(&rule.actions).unwrap_or_default();

    AlertRuleResponse {
        id: rule.id,
        project_id: rule.project_id,
        name: rule.name.clone(),
        enabled: rule.enabled,
        conditions,
        actions,
        frequency: rule.frequency,
        last_fired: rule.last_fired.clone(),
        created_at: rule.created_at.clone(),
    }
}

async fn list_alerts(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<Vec<AlertRuleResponse>>, (StatusCode, String)> {
    let project_id = resolve_project(&state, &slug).await?;

    let rules: Vec<AlertRule> =
        sqlx::query_as("SELECT * FROM alert_rules WHERE project_id = ? ORDER BY created_at DESC")
            .bind(project_id)
            .fetch_all(state.db.reader())
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(rules.iter().map(rule_to_response).collect()))
}

#[derive(Deserialize)]
struct CreateAlertInput {
    name: String,
    conditions: Vec<AlertCondition>,
    actions: Vec<AlertAction>,
    #[serde(default = "default_frequency")]
    frequency: i64,
}

fn default_frequency() -> i64 {
    1800
}

async fn create_alert(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Json(input): Json<CreateAlertInput>,
) -> Result<(StatusCode, Json<AlertRuleResponse>), (StatusCode, String)> {
    let project_id = resolve_project(&state, &slug).await?;

    let conditions = serde_json::to_string(&input.conditions)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let actions = serde_json::to_string(&input.actions)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let rule = sqlx::query_as::<_, AlertRule>(
        "INSERT INTO alert_rules (project_id, name, conditions, actions, frequency) VALUES (?, ?, ?, ?, ?) RETURNING *"
    )
    .bind(project_id)
    .bind(&input.name)
    .bind(&conditions)
    .bind(&actions)
    .bind(input.frequency)
    .fetch_one(state.db.writer())
    .await
    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    Ok((StatusCode::CREATED, Json(rule_to_response(&rule))))
}

#[derive(Debug, Deserialize)]
struct UpdateAlertInput {
    name: Option<String>,
    enabled: Option<bool>,
    conditions: Option<Vec<AlertCondition>>,
    actions: Option<Vec<AlertAction>>,
    frequency: Option<i64>,
}

async fn update_alert(
    State(state): State<AppState>,
    Path((slug, alert_id)): Path<(String, i64)>,
    Json(input): Json<UpdateAlertInput>,
) -> Result<Json<AlertRuleResponse>, (StatusCode, String)> {
    let project_id = resolve_project(&state, &slug).await?;

    let existing: AlertRule =
        sqlx::query_as("SELECT * FROM alert_rules WHERE id = ? AND project_id = ?")
            .bind(alert_id)
            .bind(project_id)
            .fetch_optional(state.db.reader())
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .ok_or((StatusCode::NOT_FOUND, "Alert rule not found".to_string()))?;

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
         WHERE id = ? AND project_id = ? RETURNING *",
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

    Ok(Json(rule_to_response(&rule)))
}

async fn delete_alert(
    State(state): State<AppState>,
    Path((slug, alert_id)): Path<(String, i64)>,
) -> Result<StatusCode, (StatusCode, String)> {
    let project_id = resolve_project(&state, &slug).await?;

    let result = sqlx::query("DELETE FROM alert_rules WHERE id = ? AND project_id = ?")
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
