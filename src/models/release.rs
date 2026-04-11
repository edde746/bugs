use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Release {
    pub id: i64,
    pub org_id: i64,
    pub version: String,
    pub created_at: String,
    pub data: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ReleaseFile {
    pub id: i64,
    pub release_id: i64,
    pub name: String,
    pub file_path: String,
    pub file_size: i64,
    pub sha256: Option<String>,
    pub dist: String,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateRelease {
    pub version: String,
    pub projects: Option<Vec<String>>,
}
