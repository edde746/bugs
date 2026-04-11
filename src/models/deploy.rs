use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Deploy {
    pub id: i64,
    pub release_id: i64,
    pub environment: String,
    pub name: String,
    pub url: String,
    pub date_started: Option<String>,
    pub date_finished: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateDeploy {
    pub environment: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default, rename = "dateStarted")]
    pub date_started: Option<String>,
    #[serde(default, rename = "dateFinished")]
    pub date_finished: Option<String>,
}
