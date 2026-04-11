use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AlertRule {
    pub id: i64,
    pub project_id: i64,
    pub name: String,
    pub enabled: bool,
    pub conditions: String,
    pub actions: String,
    pub frequency: i64,
    pub last_fired: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AlertCondition {
    NewIssue,
    RegressionEvent,
    FrequencyThreshold {
        threshold: u64,
        window_seconds: u64,
    },
    EventAttribute {
        attribute: String,
        match_type: String,
        value: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AlertAction {
    Webhook { url: String },
    Slack { webhook_url: String },
    Discord { webhook_url: String },
    Email { to: String },
    LogFile { path: String },
}

#[derive(Debug, Deserialize)]
pub struct CreateAlertRule {
    pub name: String,
    pub conditions: Vec<AlertCondition>,
    pub actions: Vec<AlertAction>,
    pub frequency: Option<i64>,
}
