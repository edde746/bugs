use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct IssueActivity {
    pub id: i64,
    pub issue_id: i64,
    pub kind: String,
    pub data: Option<String>,
    pub created_at: String,
}
