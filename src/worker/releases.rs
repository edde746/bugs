use tracing::info;

use crate::db::DbPool;

/// Ensure a release record exists for the given version and is linked to the project.
/// Uses upserts so repeated calls for the same (version, project) are no-ops.
pub async fn ensure_release(
    db: &DbPool,
    project_id: i64,
    version: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // 1. Upsert the release record
    let release: (i64,) = sqlx::query_as(
        "INSERT INTO releases (org_id, version) VALUES (1, ?) \
         ON CONFLICT(org_id, version) DO UPDATE SET org_id=org_id \
         RETURNING id",
    )
    .bind(version)
    .fetch_one(db.writer())
    .await?;

    let release_id = release.0;

    // 2. Link to project
    sqlx::query("INSERT OR IGNORE INTO release_projects (release_id, project_id) VALUES (?, ?)")
        .bind(release_id)
        .bind(project_id)
        .execute(db.writer())
        .await?;

    // 3. Auto-resolve issues marked "resolve in next release"
    let result = sqlx::query(
        "UPDATE issues SET status = 'resolved', resolved_in_release = ? \
         WHERE project_id = ? AND status = 'resolved' AND resolved_in_release = '__next__'",
    )
    .bind(version)
    .bind(project_id)
    .execute(db.writer())
    .await?;

    if result.rows_affected() > 0 {
        info!(
            project_id,
            version,
            count = result.rows_affected(),
            "Auto-resolved issues for new release"
        );
    }

    Ok(())
}
