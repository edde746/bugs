use std::io::Read;

use axum::{Router, Json, extract::{Path, Query, State, Multipart}, http::StatusCode, routing::{get, post}};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};
use tracing::warn;

use crate::AppState;
use crate::models::release::*;
use crate::models::deploy::*;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/internal/projects/{slug}/releases", get(list_project_releases))
        .route("/api/0/organizations/{org}/releases", get(list_releases).post(create_release))
        .route("/api/0/organizations/{org}/releases/{version}", get(get_release))
        .route("/api/0/projects/{org}/{project}/releases/{version}/files/", post(upload_release_file).get(list_release_files))
        .route("/api/0/organizations/{org}/releases/{version}/deploys/", get(list_deploys).post(create_deploy))
}

#[derive(Deserialize)]
struct ReleaseQuery {
    project: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReleaseResponse {
    version: String,
    date_created: String,
    date_released: Option<String>,
    short_version: String,
    new_groups: i64,
}

fn make_short_version(version: &str) -> String {
    if let Some(pos) = version.rfind('@') {
        version[pos + 1..].to_string()
    } else if version.len() > 12 {
        version[version.len() - 12..].to_string()
    } else {
        version.to_string()
    }
}

/// Internal release summary for the admin UI
#[derive(Serialize)]
struct ProjectReleaseSummary {
    version: String,
    created_at: String,
    file_count: i64,
}

async fn list_project_releases(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<Vec<ProjectReleaseSummary>>, StatusCode> {
    // Resolve slug or numeric id
    let project_id: i64 = if let Ok(id) = slug.parse::<i64>() {
        id
    } else {
        let row: Option<(i64,)> = sqlx::query_as("SELECT id FROM projects WHERE slug = ?")
            .bind(&slug)
            .fetch_optional(state.db.reader())
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        row.ok_or(StatusCode::NOT_FOUND)?.0
    };

    let rows: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT r.version, r.created_at, \
                COALESCE((SELECT COUNT(*) FROM release_files rf WHERE rf.release_id = r.id), 0) as file_count \
         FROM releases r \
         JOIN release_projects rp ON rp.release_id = r.id \
         WHERE rp.project_id = ? \
         ORDER BY r.created_at DESC LIMIT 100"
    )
    .bind(project_id)
    .fetch_all(state.db.reader())
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let response: Vec<ProjectReleaseSummary> = rows.into_iter().map(|(version, created_at, file_count)| {
        ProjectReleaseSummary { version, created_at, file_count }
    }).collect();

    Ok(Json(response))
}

async fn list_releases(
    State(state): State<AppState>,
    Path(_org): Path<String>,
    Query(params): Query<ReleaseQuery>,
) -> Result<Json<Vec<ReleaseResponse>>, StatusCode> {
    let releases: Vec<Release> = if let Some(ref project) = params.project {
        // Resolve project slug or id
        let project_id: Option<i64> = if let Ok(id) = project.parse::<i64>() {
            Some(id)
        } else {
            let row: Option<(i64,)> = sqlx::query_as("SELECT id FROM projects WHERE slug = ?")
                .bind(project)
                .fetch_optional(state.db.reader())
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            row.map(|r| r.0)
        };
        if let Some(pid) = project_id {
            sqlx::query_as(
                "SELECT r.* FROM releases r \
                 JOIN release_projects rp ON rp.release_id = r.id \
                 WHERE r.org_id = 1 AND rp.project_id = ? \
                 ORDER BY r.created_at DESC LIMIT 100"
            )
            .bind(pid)
            .fetch_all(state.db.reader())
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        } else {
            Vec::new()
        }
    } else {
        sqlx::query_as(
            "SELECT * FROM releases WHERE org_id = 1 ORDER BY created_at DESC LIMIT 100"
        )
        .fetch_all(state.db.reader())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    };

    let response: Vec<ReleaseResponse> = releases.iter().map(|r| ReleaseResponse {
        version: r.version.clone(),
        date_created: r.created_at.clone(),
        date_released: None,
        short_version: make_short_version(&r.version),
        new_groups: 0,
    }).collect();

    Ok(Json(response))
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

    // Associate with projects and auto-resolve "next release" issues
    if let Some(projects) = &input.projects {
        for slug in projects {
            let project: Option<(i64,)> = sqlx::query_as(
                "SELECT id FROM projects WHERE slug = ?"
            )
            .bind(slug)
            .fetch_optional(state.db.reader())
            .await
            .unwrap_or(None);

            let Some((pid,)) = project else { continue };

            sqlx::query(
                "INSERT OR IGNORE INTO release_projects (release_id, project_id) VALUES (?, ?)"
            )
            .bind(release.id)
            .bind(pid)
            .execute(state.db.writer())
            .await
            .ok();

            // Auto-resolve issues marked "resolve in next release"
            let result = sqlx::query(
                "UPDATE issues SET status = 'resolved', resolved_in_release = ? \
                 WHERE project_id = ? AND status = 'resolved' AND resolved_in_release = '__next__'"
            )
            .bind(&input.version)
            .bind(pid)
            .execute(state.db.writer())
            .await;

            if let Ok(r) = result {
                if r.rows_affected() > 0 {
                    tracing::info!(
                        project = slug,
                        version = input.version,
                        count = r.rows_affected(),
                        "Auto-resolved issues for new release"
                    );
                }
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

async fn upload_release_file(
    State(state): State<AppState>,
    Path((_org, _project, version)): Path<(String, String, String)>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<ReleaseFile>), (StatusCode, String)> {
    let mut file_content: Option<Vec<u8>> = None;
    let mut artifact_name: Option<String> = None;
    let mut dist = String::new();

    // Parse multipart fields
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
    {
        let field_name = field.name().unwrap_or("").to_string();
        match field_name.as_str() {
            "file" => {
                let data = field
                    .bytes()
                    .await
                    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
                file_content = Some(data.to_vec());
            }
            "name" => {
                let text = field
                    .text()
                    .await
                    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
                artifact_name = Some(text);
            }
            "dist" => {
                let text = field
                    .text()
                    .await
                    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
                dist = text;
            }
            _ => {}
        }
    }

    let file_content =
        file_content.ok_or((StatusCode::BAD_REQUEST, "Missing 'file' field".to_string()))?;
    let artifact_name =
        artifact_name.ok_or((StatusCode::BAD_REQUEST, "Missing 'name' field".to_string()))?;

    // Find or create the release
    let release = sqlx::query_as::<_, Release>(
        "INSERT INTO releases (org_id, version) VALUES (1, ?) \
         ON CONFLICT(org_id, version) DO UPDATE SET org_id=org_id \
         RETURNING *",
    )
    .bind(&version)
    .fetch_one(state.db.writer())
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Decompress if gzipped
    let content = if file_content.len() >= 2 && file_content[0] == 0x1f && file_content[1] == 0x8b
    {
        let mut decoder = GzDecoder::new(&file_content[..]);
        let mut decompressed = Vec::new();
        decoder
            .read_to_end(&mut decompressed)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Gzip decode error: {e}")))?;
        decompressed
    } else {
        file_content
    };

    // Compute SHA256
    let mut hasher = Sha256::new();
    hasher.update(&content);
    let sha256 = hex::encode(hasher.finalize());

    // Sanitize name for filesystem path
    let sanitized_name = artifact_name
        .replace("~/", "")
        .replace(['/', '\\'], "_");

    // Build file path
    let org_id: i64 = 1;
    let dir_path = format!(
        "{}/releases/{}/{}",
        state.config.artifacts_dir, org_id, release.id
    );
    let file_path = format!("{}/{}", dir_path, sanitized_name);

    // Save to disk
    tokio::fs::create_dir_all(&dir_path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to create dir: {e}")))?;

    tokio::fs::write(&file_path, &content)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to write file: {e}")))?;

    let file_size = content.len() as i64;

    // Insert into release_files with ON CONFLICT UPDATE
    let release_file = sqlx::query_as::<_, ReleaseFile>(
        "INSERT INTO release_files (release_id, name, file_path, file_size, sha256, dist) \
         VALUES (?, ?, ?, ?, ?, ?) \
         ON CONFLICT(release_id, name, dist) DO UPDATE SET \
            file_path = excluded.file_path, \
            file_size = excluded.file_size, \
            sha256 = excluded.sha256 \
         RETURNING *",
    )
    .bind(release.id)
    .bind(&artifact_name)
    .bind(&file_path)
    .bind(file_size)
    .bind(&sha256)
    .bind(&dist)
    .fetch_one(state.db.writer())
    .await
    .map_err(|e| {
        warn!("Failed to insert release file: {e}");
        (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    })?;

    Ok((StatusCode::CREATED, Json(release_file)))
}

async fn list_release_files(
    State(state): State<AppState>,
    Path((_org, _project, version)): Path<(String, String, String)>,
) -> Result<Json<Vec<ReleaseFile>>, StatusCode> {
    let release: Option<(i64,)> = sqlx::query_as(
        "SELECT id FROM releases WHERE org_id = 1 AND version = ?"
    )
    .bind(&version)
    .fetch_optional(state.db.reader())
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let release_id = release.ok_or(StatusCode::NOT_FOUND)?.0;

    let files: Vec<ReleaseFile> = sqlx::query_as(
        "SELECT * FROM release_files WHERE release_id = ? ORDER BY created_at DESC"
    )
    .bind(release_id)
    .fetch_all(state.db.reader())
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(files))
}

async fn list_deploys(
    State(state): State<AppState>,
    Path((_org, version)): Path<(String, String)>,
) -> Result<Json<Vec<Deploy>>, StatusCode> {
    let release: Option<(i64,)> = sqlx::query_as(
        "SELECT id FROM releases WHERE org_id = 1 AND version = ?",
    )
    .bind(&version)
    .fetch_optional(state.db.reader())
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let release_id = release.ok_or(StatusCode::NOT_FOUND)?.0;

    let deploys: Vec<Deploy> = sqlx::query_as(
        "SELECT * FROM deploys WHERE release_id = ? ORDER BY date_finished DESC",
    )
    .bind(release_id)
    .fetch_all(state.db.reader())
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(deploys))
}

async fn create_deploy(
    State(state): State<AppState>,
    Path((_org, version)): Path<(String, String)>,
    Json(input): Json<CreateDeploy>,
) -> Result<(StatusCode, Json<Deploy>), (StatusCode, String)> {
    let release: Option<(i64,)> = sqlx::query_as(
        "SELECT id FROM releases WHERE org_id = 1 AND version = ?",
    )
    .bind(&version)
    .fetch_optional(state.db.reader())
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let release_id = release.ok_or((StatusCode::NOT_FOUND, "Release not found".to_string()))?.0;

    let deploy: Deploy = sqlx::query_as(
        "INSERT INTO deploys (release_id, environment, name, url, date_started, date_finished) \
         VALUES (?, ?, ?, ?, ?, COALESCE(?, strftime('%Y-%m-%dT%H:%M:%SZ','now'))) \
         RETURNING *",
    )
    .bind(release_id)
    .bind(&input.environment)
    .bind(input.name.as_deref().unwrap_or(""))
    .bind(input.url.as_deref().unwrap_or(""))
    .bind(&input.date_started)
    .bind(&input.date_finished)
    .fetch_one(state.db.writer())
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok((StatusCode::CREATED, Json(deploy)))
}
