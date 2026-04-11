use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct IssueComment {
    pub id: i64,
    pub issue_id: i64,
    pub text: String,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateComment {
    pub text: String,
}
