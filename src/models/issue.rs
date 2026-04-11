use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Issue {
    pub id: i64,
    pub project_id: i64,
    pub fingerprint: String,
    pub title: String,
    pub culprit: Option<String>,
    pub level: String,
    pub status: String,
    pub first_seen: String,
    pub last_seen: String,
    pub event_count: i64,
    pub metadata: Option<String>,
    pub snooze_until: Option<String>,
    pub snooze_event_count: Option<i64>,
    pub resolved_in_release: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateIssue {
    pub status: Option<String>,
    /// For muting: ISO timestamp when the issue should auto-unmute
    #[serde(default, rename = "snoozeUntil")]
    pub snooze_until: Option<String>,
    /// For muting: auto-unmute when event_count reaches this value
    #[serde(default, rename = "snoozeEventCount")]
    pub snooze_event_count: Option<i64>,
    /// For resolve-by-release: version string or "__next__"
    #[serde(default, rename = "resolvedInRelease")]
    pub resolved_in_release: Option<String>,
}
