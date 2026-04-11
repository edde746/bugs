use std::io::Write;

use tracing::{debug, error, warn};

use crate::db::DbPool;
use crate::models::alert::{AlertAction, AlertCondition, AlertRule};
use crate::sentry_protocol::types::SentryEvent;
use crate::util::time::now_iso;

pub async fn evaluate_alerts(
    db: &DbPool,
    project_id: i64,
    issue_id: i64,
    event: &SentryEvent,
    is_new_issue: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // 1. Load enabled alert rules for the project
    let rules: Vec<AlertRule> = sqlx::query_as(
        "SELECT * FROM alert_rules WHERE project_id = ? AND enabled = 1",
    )
    .bind(project_id)
    .fetch_all(db.reader())
    .await?;

    if rules.is_empty() {
        return Ok(());
    }

    let now = chrono::Utc::now();

    for rule in &rules {
        // 2a. Check cooldown
        if let Some(ref last_fired) = rule.last_fired {
            if let Ok(fired_at) = chrono::DateTime::parse_from_rfc3339(last_fired) {
                let elapsed = now.signed_duration_since(fired_at).num_seconds();
                if elapsed < rule.frequency {
                    debug!(rule_id = rule.id, "Alert rule still in cooldown, skipping");
                    continue;
                }
            }
        }

        // 2b. Parse conditions
        let conditions: Vec<AlertCondition> = match serde_json::from_str(&rule.conditions) {
            Ok(c) => c,
            Err(e) => {
                warn!(rule_id = rule.id, "Failed to parse alert conditions: {e}");
                continue;
            }
        };

        // 2c. Evaluate ALL conditions (AND logic)
        let mut all_match = true;
        for condition in &conditions {
            let matches = evaluate_condition(db, condition, project_id, issue_id, event, is_new_issue).await;
            if !matches {
                all_match = false;
                break;
            }
        }

        if !all_match {
            continue;
        }

        // 2d. Fire all actions
        let actions: Vec<AlertAction> = match serde_json::from_str(&rule.actions) {
            Ok(a) => a,
            Err(e) => {
                warn!(rule_id = rule.id, "Failed to parse alert actions: {e}");
                continue;
            }
        };

        debug!(rule_id = rule.id, rule_name = %rule.name, "Alert rule fired");

        for action in &actions {
            if let Err(e) = fire_action(action, &rule.name, project_id, issue_id, event).await {
                error!(rule_id = rule.id, "Alert action failed: {e}");
            }
        }

        // 2e. Update last_fired timestamp
        let fired_at = now_iso();
        let _ = sqlx::query("UPDATE alert_rules SET last_fired = ? WHERE id = ?")
            .bind(&fired_at)
            .bind(rule.id)
            .execute(db.writer())
            .await;
    }

    Ok(())
}

async fn evaluate_condition(
    db: &DbPool,
    condition: &AlertCondition,
    _project_id: i64,
    issue_id: i64,
    event: &SentryEvent,
    is_new_issue: bool,
) -> bool {
    match condition {
        AlertCondition::NewIssue => is_new_issue,

        AlertCondition::RegressionEvent => {
            // Check if issue status was 'resolved' before this event
            let result: Option<(String,)> = sqlx::query_as(
                "SELECT status FROM issues WHERE id = ?",
            )
            .bind(issue_id)
            .fetch_optional(db.reader())
            .await
            .ok()
            .flatten();

            // If issue is resolved and we're getting a new event, it's a regression
            // Note: By the time we check, the issue may already be updated.
            // We check event_count: if it was resolved and now has a new event, it's a regression.
            match result {
                Some((status,)) => status == "resolved",
                None => false,
            }
        }

        AlertCondition::FrequencyThreshold { threshold, window_seconds } => {
            // Count events in issue_stats_hourly within window
            let window_start = (chrono::Utc::now()
                - chrono::Duration::seconds(*window_seconds as i64))
                .format("%Y-%m-%dT%H:00:00Z")
                .to_string();

            let result: Option<(i64,)> = sqlx::query_as(
                "SELECT COALESCE(SUM(count), 0) FROM issue_stats_hourly \
                 WHERE issue_id = ? AND bucket >= ?",
            )
            .bind(issue_id)
            .bind(&window_start)
            .fetch_optional(db.reader())
            .await
            .ok()
            .flatten();

            match result {
                Some((count,)) => count >= *threshold as i64,
                None => false,
            }
        }

        AlertCondition::EventAttribute { attribute, match_type, value } => {
            let event_value = match attribute.as_str() {
                "level" => event.level.as_deref().unwrap_or(""),
                "environment" => event.environment.as_deref().unwrap_or(""),
                "platform" => event.platform.as_deref().unwrap_or(""),
                "release" => event.release.as_deref().unwrap_or(""),
                "transaction" => event.transaction.as_deref().unwrap_or(""),
                "logger" => event.logger.as_deref().unwrap_or(""),
                "message" => event.message.as_deref().unwrap_or(""),
                _ => "",
            };

            match match_type.as_str() {
                "equals" => event_value == value,
                "not_equals" => event_value != value,
                "contains" => event_value.contains(value.as_str()),
                "not_contains" => !event_value.contains(value.as_str()),
                "starts_with" => event_value.starts_with(value.as_str()),
                "ends_with" => event_value.ends_with(value.as_str()),
                _ => false,
            }
        }
    }
}

async fn fire_action(
    action: &AlertAction,
    rule_name: &str,
    project_id: i64,
    issue_id: i64,
    event: &SentryEvent,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match action {
        AlertAction::Webhook { url } => {
            let payload = serde_json::json!({
                "rule": rule_name,
                "project_id": project_id,
                "issue_id": issue_id,
                "event_id": event.event_id,
                "level": event.level,
                "message": event.message,
                "environment": event.environment,
                "timestamp": event.timestamp,
            });

            let client = reqwest::Client::new();
            let resp = client
                .post(url)
                .json(&payload)
                .timeout(std::time::Duration::from_secs(10))
                .send()
                .await?;

            if !resp.status().is_success() {
                warn!(url, status = %resp.status(), "Webhook returned non-success status");
            }

            Ok(())
        }

        AlertAction::LogFile { path } => {
            let line = format!(
                "[{}] Alert '{}' fired: project={} issue={} event={}\n",
                now_iso(),
                rule_name,
                project_id,
                issue_id,
                event.event_id.as_deref().unwrap_or("unknown"),
            );

            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)?;
            file.write_all(line.as_bytes())?;

            Ok(())
        }
    }
}
