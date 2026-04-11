use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct UserReport {
    pub id: i64,
    pub project_id: i64,
    pub event_id: String,
    pub name: String,
    pub email: String,
    pub comments: String,
    pub created_at: String,
}
