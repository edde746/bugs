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
}

#[derive(Debug, Deserialize)]
pub struct UpdateIssue {
    pub status: Option<String>,
}
