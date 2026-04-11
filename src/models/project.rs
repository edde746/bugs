use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Project {
    pub id: i64,
    pub org_id: i64,
    pub name: String,
    pub slug: String,
    pub platform: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ProjectKey {
    pub id: i64,
    pub project_id: i64,
    pub public_key: String,
    pub label: String,
    pub is_active: bool,
    pub created_at: String,
    pub rate_limit: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ProjectSettings {
    pub project_id: i64,
    pub allowed_origins: Option<String>,
    pub max_event_size: Option<i64>,
    pub retention_days: Option<i64>,
    pub rate_limit_per_min: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct CreateProject {
    pub name: String,
    pub slug: String,
    pub platform: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateProjectKey {
    pub label: Option<String>,
    pub rate_limit: Option<i64>,
}
