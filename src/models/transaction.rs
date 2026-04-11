use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct TransactionGroup {
    pub id: i64,
    pub project_id: i64,
    pub transaction_name: String,
    pub op: String,
    pub method: String,
    pub count: i64,
    pub error_count: i64,
    pub sum_duration_ms: f64,
    pub min_duration_ms: Option<f64>,
    pub max_duration_ms: Option<f64>,
    pub p50_duration_ms: Option<f64>,
    pub p95_duration_ms: Option<f64>,
    pub last_seen: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Transaction {
    pub id: i64,
    pub project_id: i64,
    pub group_id: Option<i64>,
    pub trace_id: Option<String>,
    pub transaction_name: String,
    pub op: String,
    pub method: String,
    pub status: String,
    pub duration_ms: f64,
    pub timestamp: String,
    pub environment: Option<String>,
    pub release: Option<String>,
    pub data: String,
    pub created_at: String,
}
