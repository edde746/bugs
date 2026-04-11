use std::io::Read;

use axum::{Router, Json, extract::{Path, Query, State, Multipart}, http::StatusCode, routing::{get, post}};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};
use tracing::warn;

use crate::AppState;
use crate::models::release::*;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/0/organizations/{org}/releases", get(list_releases).post(create_release))
        .route("/api/0/organizations/{org}/releases/{version}", get(get_release))
        .route("/api/0/projects/{org}/{project}/releases/{version}/files/", post(upload_release_file).get(list_release_files))
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
