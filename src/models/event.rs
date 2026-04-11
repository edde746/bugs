use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct EventEnvelope {
    pub id: i64,
    pub project_id: i64,
    pub event_id: String,
    pub received_at: String,
    pub content_encoding: Option<String>,
    pub body: Vec<u8>,
    pub state: String,
    pub attempts: i64,
    pub last_error: Option<String>,
    pub next_attempt_at: Option<String>,
    pub processing_started_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Event {
    pub id: i64,
    pub event_id: String,
    pub project_id: i64,
    pub issue_id: Option<i64>,
    pub timestamp: String,
    pub received_at: String,
    pub level: String,
    pub platform: Option<String>,
    pub release: Option<String>,
    pub environment: Option<String>,
    pub transaction_name: Option<String>,
    pub trace_id: Option<String>,
    pub message: Option<String>,
    pub title: Option<String>,
    pub exception_values: Option<String>,
    pub stacktrace_functions: Option<String>,
    pub data: String,
}

/// Lightweight event summary for list endpoints (excludes the large `data` JSON blob).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct EventSummary {
    pub id: i64,
    pub event_id: String,
    pub project_id: i64,
    pub issue_id: Option<i64>,
    pub timestamp: String,
    pub received_at: String,
    pub level: String,
    pub platform: Option<String>,
    pub release: Option<String>,
    pub environment: Option<String>,
    pub transaction_name: Option<String>,
    pub trace_id: Option<String>,
    pub message: Option<String>,
    pub title: Option<String>,
    pub exception_values: Option<String>,
    pub stacktrace_functions: Option<String>,
}
