use axum::{Router, Json, extract::{Path, State}, http::StatusCode, routing::get};
use serde::{Deserialize, Serialize};
use crate::AppState;
use crate::models::alert::*;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/internal/projects/{slug}/alerts", get(list_alerts).post(create_alert))
        .route("/api/internal/projects/{slug}/alerts/{alert_id}",
            axum::routing::put(update_alert).delete(delete_alert))
}

async fn resolve_project(state: &AppState, slug: &str) -> Result<i64, (StatusCode, String)> {
    let project: Option<(i64,)> = sqlx::query_as(
        "SELECT id FROM projects WHERE slug = ?"
    )
    .bind(slug)
    .fetch_optional(state.db.reader())
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(project.ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?.0)
}

#[derive(Serialize)]
struct FlatAlertResponse {
    id: i64,
    name: String,
    condition_type: String,
    webhook_url: String,
    enabled: bool,
    created_at: String,
}

fn alert_to_flat(rule: &AlertRule) -> FlatAlertResponse {
    let condition_type = serde_json::from_str::<Vec<AlertCondition>>(&rule.conditions)
        .ok()
        .and_then(|c| c.first().map(condition_to_type))
        .unwrap_or_default();

    let webhook_url = serde_json::from_str::<Vec<AlertAction>>(&rule.actions)
        .ok()
        .and_then(|a| a.into_iter().find_map(|action| match action {
            AlertAction::Webhook { url } => Some(url),
            _ => None,
        }))
        .unwrap_or_default();

    FlatAlertResponse {
        id: rule.id,
        name: rule.name.clone(),
        condition_type,
        webhook_url,
        enabled: rule.enabled,
        created_at: rule.created_at.clone(),
    }
}

fn condition_to_type(c: &AlertCondition) -> String {
    match c {
        AlertCondition::NewIssue => "new_issue".to_string(),
        AlertCondition::RegressionEvent => "issue_regression".to_string(),
        AlertCondition::FrequencyThreshold { .. } => "event_frequency".to_string(),
        AlertCondition::EventAttribute { .. } => "event_attribute".to_string(),
    }
}

async fn list_alerts(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<Vec<FlatAlertResponse>>, (StatusCode, String)> {
    let project_id = resolve_project(&state, &slug).await?;

    let rules: Vec<AlertRule> = sqlx::query_as(
        "SELECT * FROM alert_rules WHERE project_id = ? ORDER BY created_at DESC"
    )
    .bind(project_id)
    .fetch_all(state.db.reader())
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(rules.iter().map(alert_to_flat).collect()))
}

#[derive(Deserialize)]
struct FlatAlertInput {
    name: String,
    condition_type: String,
    webhook_url: String,
}

async fn create_alert(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Json(input): Json<FlatAlertInput>,
) -> Result<(StatusCode, Json<FlatAlertResponse>), (StatusCode, String)> {
    let project_id = resolve_project(&state, &slug).await?;

    let condition: AlertCondition = match input.condition_type.as_str() {
        "new_issue" => AlertCondition::NewIssue,
        "issue_regression" => AlertCondition::RegressionEvent,
        "event_frequency" => AlertCondition::FrequencyThreshold { threshold: 1, window_seconds: 3600 },
        other => return Err((StatusCode::BAD_REQUEST, format!("Unknown condition type: {other}"))),
    };
    let action = AlertAction::Webhook { url: input.webhook_url };

    let conditions = serde_json::to_string(&vec![condition])
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let actions = serde_json::to_string(&vec![action])
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let rule = sqlx::query_as::<_, AlertRule>(
        "INSERT INTO alert_rules (project_id, name, conditions, actions, frequency) VALUES (?, ?, ?, ?, ?) RETURNING *"
    )
    .bind(project_id)
    .bind(&input.name)
    .bind(&conditions)
    .bind(&actions)
    .bind(1800i64)
    .fetch_one(state.db.writer())
    .await
    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    Ok((StatusCode::CREATED, Json(alert_to_flat(&rule))))
}

#[derive(Debug, Deserialize)]
struct UpdateAlertInput {
    name: Option<String>,
    enabled: Option<bool>,
    condition_type: Option<String>,
    webhook_url: Option<String>,
}

async fn update_alert(
    State(state): State<AppState>,
    Path((slug, alert_id)): Path<(String, i64)>,
    Json(input): Json<UpdateAlertInput>,
) -> Result<Json<FlatAlertResponse>, (StatusCode, String)> {
    let project_id = resolve_project(&state, &slug).await?;

    let existing: AlertRule = sqlx::query_as(
        "SELECT * FROM alert_rules WHERE id = ? AND project_id = ?"
    )
    .bind(alert_id)
    .bind(project_id)
    .fetch_optional(state.db.reader())
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .ok_or((StatusCode::NOT_FOUND, "Alert rule not found".to_string()))?;

    let name = input.name.unwrap_or(existing.name);
    let enabled = input.enabled.unwrap_or(existing.enabled);

    let conditions = if let Some(ref ct) = input.condition_type {
        let condition: AlertCondition = match ct.as_str() {
            "new_issue" => AlertCondition::NewIssue,
            "issue_regression" => AlertCondition::RegressionEvent,
            "event_frequency" => AlertCondition::FrequencyThreshold { threshold: 1, window_seconds: 3600 },
            other => return Err((StatusCode::BAD_REQUEST, format!("Unknown condition type: {other}"))),
        };
        serde_json::to_string(&vec![condition]).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
    } else {
        existing.conditions
    };

    let actions = if let Some(ref url) = input.webhook_url {
        let action = AlertAction::Webhook { url: url.clone() };
        serde_json::to_string(&vec![action]).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
    } else {
        existing.actions
    };

    let rule = sqlx::query_as::<_, AlertRule>(
        "UPDATE alert_rules SET name = ?, enabled = ?, conditions = ?, actions = ? \
         WHERE id = ? AND project_id = ? RETURNING *"
    )
    .bind(&name)
    .bind(enabled)
    .bind(&conditions)
    .bind(&actions)
    .bind(alert_id)
    .bind(project_id)
    .fetch_one(state.db.writer())
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(alert_to_flat(&rule)))
}

async fn delete_alert(
    State(state): State<AppState>,
    Path((slug, alert_id)): Path<(String, i64)>,
) -> Result<StatusCode, (StatusCode, String)> {
    let project_id = resolve_project(&state, &slug).await?;

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
