use tracing::{debug, error, warn};

use crate::config::Config;
use crate::db::DbPool;
use crate::models::alert::{AlertAction, AlertCondition, AlertRule};
use crate::sentry_protocol::types::SentryEvent;
use crate::util::time::now_iso;
use crate::worker::fingerprint;

use once_cell::sync::Lazy;

static HTTP_CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
});

pub async fn evaluate_alerts(
    db: &DbPool,
    config: &Config,
    project_id: i64,
    issue_id: i64,
    event: &SentryEvent,
    is_new_issue: bool,
    is_regression: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // 1. Load enabled alert rules for the project
    let rules: Vec<AlertRule> =
        sqlx::query_as("SELECT * FROM alert_rules WHERE project_id = ? AND enabled = 1")
            .bind(project_id)
            .fetch_all(db.reader())
            .await?;

    if rules.is_empty() {
        return Ok(());
    }

    let now = chrono::Utc::now();

    for rule in &rules {
        // 2a. Check cooldown
        if let Some(ref last_fired) = rule.last_fired
            && let Ok(fired_at) = chrono::DateTime::parse_from_rfc3339(last_fired)
        {
            let elapsed = now.signed_duration_since(fired_at).num_seconds();
            if elapsed < rule.frequency {
                debug!(rule_id = rule.id, "Alert rule still in cooldown, skipping");
                continue;
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
            let matches = evaluate_condition(
                db,
                condition,
                project_id,
                issue_id,
                event,
                is_new_issue,
                is_regression,
            )
            .await;
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
            if let Err(e) =
                fire_action(action, &rule.name, project_id, issue_id, event, config).await
            {
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
    is_regression: bool,
) -> bool {
    match condition {
        AlertCondition::NewIssue => is_new_issue,

        AlertCondition::RegressionEvent => is_regression,

        AlertCondition::FrequencyThreshold {
            threshold,
            window_seconds,
        } => {
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

        AlertCondition::EventAttribute {
            attribute,
            match_type,
            value,
        } => {
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

fn level_color_int(level: &str) -> u32 {
    match level {
        "fatal" => 0xd32f2f,
        "error" => 0xe53935,
        "warning" => 0xff9800,
        "info" => 0x2196f3,
        "debug" => 0x9e9e9e,
        _ => 0xe53935,
    }
}

fn level_color_hex(level: &str) -> String {
    format!("#{:06x}", level_color_int(level))
}

async fn send_webhook(
    url: &str,
    payload: &serde_json::Value,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let resp = HTTP_CLIENT.post(url).json(payload).send().await?;

    if !resp.status().is_success() {
        warn!(url, status = %resp.status(), "Webhook returned non-success status");
    }

    Ok(())
}

async fn fire_action(
    action: &AlertAction,
    rule_name: &str,
    project_id: i64,
    issue_id: i64,
    event: &SentryEvent,
    config: &Config,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let level = event.level.as_deref().unwrap_or("error");
    let title = fingerprint::derive_title(event);
    let env_text = event.environment.as_deref().unwrap_or("unknown");

    match action {
        AlertAction::Webhook { url } => {
            let payload = serde_json::json!({
                "rule": rule_name,
                "project_id": project_id,
                "issue_id": issue_id,
                "event_id": event.event_id,
                "level": level,
                "title": title,
                "message": event.message,
                "environment": event.environment,
                "timestamp": event.timestamp,
            });

            send_webhook(url, &payload).await
        }

        AlertAction::Slack { webhook_url } => {
            let payload = serde_json::json!({
                "attachments": [{
                    "color": level_color_hex(level),
                    "blocks": [
                        {
                            "type": "section",
                            "text": {
                                "type": "mrkdwn",
                                "text": format!("*{title}*\nRule: {rule_name} | Level: {level} | Env: {env_text}")
                            }
                        },
                        {
                            "type": "context",
                            "elements": [
                                {
                                    "type": "mrkdwn",
                                    "text": format!("Issue #{issue_id} | Project #{project_id}")
                                }
                            ]
                        }
                    ]
                }]
            });

            send_webhook(webhook_url, &payload).await
        }

        AlertAction::Discord { webhook_url } => {
            let payload = serde_json::json!({
                "embeds": [{
                    "title": title,
                    "color": level_color_int(level),
                    "fields": [
                        { "name": "Rule", "value": rule_name, "inline": true },
                        { "name": "Level", "value": level, "inline": true },
                        { "name": "Environment", "value": env_text, "inline": true },
                        { "name": "Issue", "value": format!("#{issue_id}"), "inline": true },
                    ],
                    "timestamp": event.timestamp,
                }]
            });

            send_webhook(webhook_url, &payload).await
        }

        AlertAction::Email { to } => {
            if config.email.smtp_host.is_empty() {
                warn!("Email alert action skipped: SMTP not configured");
                return Ok(());
            }

            let from = if config.email.from_address.is_empty() {
                format!("bugs@{}", config.email.smtp_host)
            } else {
                config.email.from_address.clone()
            };

            let subject = format!("[{}] {} - Issue #{}", level.to_uppercase(), title, issue_id);
            let body = format!(
                "Alert rule \"{}\" triggered\n\n\
                 Title: {}\n\
                 Level: {}\n\
                 Environment: {}\n\
                 Project ID: {}\n\
                 Issue ID: {}\n\
                 Event ID: {}\n",
                rule_name,
                title,
                level,
                env_text,
                project_id,
                issue_id,
                event.event_id.as_deref().unwrap_or("unknown"),
            );

            use lettre::{
                Message, SmtpTransport, Transport, transport::smtp::authentication::Credentials,
            };

            let email = Message::builder()
                .from(from.parse()?)
                .to(to.parse()?)
                .subject(subject)
                .body(body)?;

            let smtp_host = config.email.smtp_host.clone();
            let smtp_tls = config.email.smtp_tls;
            let smtp_port = config.email.smtp_port;
            let smtp_username = config.email.smtp_username.clone();
            let smtp_password = config.email.smtp_password.clone();

            // Run blocking SMTP send off the async runtime
            tokio::task::spawn_blocking(move || {
                let mut builder = if smtp_tls {
                    SmtpTransport::starttls_relay(&smtp_host)?
                } else {
                    SmtpTransport::builder_dangerous(&smtp_host)
                };

                builder = builder.port(smtp_port);

                if !smtp_username.is_empty() {
                    builder = builder.credentials(Credentials::new(smtp_username, smtp_password));
                }

                let mailer = builder.build();
                mailer.send(&email)?;
                Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
            })
            .await??;

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

            let path = path.clone();
            // Run blocking file I/O off the async runtime
            tokio::task::spawn_blocking(move || {
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)?;
                file.write_all(line.as_bytes())?;
                Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
            })
            .await??;

            Ok(())
        }
    }
}
