use axum::{Router, Json, extract::{Path, State}, http::StatusCode, routing::{get, post}};
use crate::AppState;
use crate::models::release::*;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/0/organizations/{org}/releases", get(list_releases).post(create_release))
        .route("/api/0/organizations/{org}/releases/{version}", get(get_release))
}

async fn list_releases(
    State(state): State<AppState>,
    Path(_org): Path<String>,
) -> Result<Json<Vec<Release>>, StatusCode> {
    let releases: Vec<Release> = sqlx::query_as(
        "SELECT * FROM releases WHERE org_id = 1 ORDER BY created_at DESC LIMIT 100"
    )
    .fetch_all(state.db.reader())
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(releases))
}

async fn create_release(
    State(state): State<AppState>,
    Path(_org): Path<String>,
    Json(input): Json<CreateRelease>,
) -> Result<(StatusCode, Json<Release>), (StatusCode, String)> {
    let release = sqlx::query_as::<_, Release>(
        "INSERT INTO releases (org_id, version) VALUES (1, ?) \
         ON CONFLICT(org_id, version) DO UPDATE SET org_id=org_id \
         RETURNING *"
    )
    .bind(&input.version)
    .fetch_one(state.db.writer())
    .await
    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    // Associate with projects if specified
    if let Some(projects) = &input.projects {
        for slug in projects {
            let project: Option<(i64,)> = sqlx::query_as(
                "SELECT id FROM projects WHERE slug = ?"
            )
            .bind(slug)
            .fetch_optional(state.db.reader())
            .await
            .unwrap_or(None);

            if let Some((pid,)) = project {
                sqlx::query(
                    "INSERT OR IGNORE INTO release_projects (release_id, project_id) VALUES (?, ?)"
                )
                .bind(release.id)
                .bind(pid)
                .execute(state.db.writer())
                .await
                .ok();
            }
        }
    }

    Ok((StatusCode::CREATED, Json(release)))
}

async fn get_release(
    State(state): State<AppState>,
    Path((_org, version)): Path<(String, String)>,
) -> Result<Json<Release>, StatusCode> {
    sqlx::query_as("SELECT * FROM releases WHERE org_id = 1 AND version = ?")
        .bind(&version)
        .fetch_optional(state.db.reader())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}
