use sqlx::SqlitePool;
use crate::models::project::{Project, ProjectKey};

/// Resolve a public key to its project, verifying it's active
pub async fn resolve_project_key(
    reader: &SqlitePool,
    public_key: &str,
) -> Result<(ProjectKey, Project), AuthError> {
    let key: ProjectKey = sqlx::query_as(
        "SELECT * FROM project_keys WHERE public_key = ? AND is_active = 1"
    )
    .bind(public_key)
    .fetch_optional(reader)
    .await
    .map_err(|_| AuthError::Internal)?
    .ok_or(AuthError::InvalidKey)?;

    let project: Project = sqlx::query_as(
        "SELECT * FROM projects WHERE id = ?"
    )
    .bind(key.project_id)
    .fetch_optional(reader)
    .await
    .map_err(|_| AuthError::Internal)?
    .ok_or(AuthError::ProjectNotFound)?;

    Ok((key, project))
}

#[derive(Debug)]
pub enum AuthError {
    InvalidKey,
    ProjectNotFound,
    Internal,
}
