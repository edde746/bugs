use std::collections::BTreeMap;

use axum::{
    Json, Router,
    extract::{Multipart, Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha1::{Digest as Sha1Digest, Sha1};
use sha2::{Digest as Sha2Digest, Sha256};
use tracing::warn;

use crate::AppState;
use crate::api::ingest::{DecompressError, decompress_gzip_capped};
use crate::models::deploy::*;
use crate::models::release::*;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/internal/projects/{slug}/releases",
            get(list_internal_project_releases),
        )
        .route(
            "/api/0/organizations/{org}/releases",
            get(list_org_releases).post(create_org_release),
        )
        .route(
            "/api/0/organizations/{org}/releases/{version}",
            get(get_org_release).put(update_org_release),
        )
        .route(
            "/api/0/projects/{org}/{project}/releases",
            get(list_project_releases).post(create_project_release),
        )
        .route(
            "/api/0/projects/{org}/{project}/releases/{version}",
            get(get_project_release).put(update_project_release),
        )
        .route(
            "/api/0/projects/{org}/{project}/releases/{version}/files",
            post(upload_release_file).get(list_release_files),
        )
        .route(
            "/api/0/projects/{org}/{project}/releases/{version}/files/{file_id}",
            delete(delete_release_file),
        )
        .route(
            "/api/0/projects/{org}/{project}/files/dsyms",
            post(crate::api::dsyms::upload_dsym),
        )
        .route(
            "/api/0/organizations/{org}/releases/{version}/deploys",
            get(list_deploys).post(create_deploy),
        )
}

#[derive(Deserialize)]
struct ReleaseQuery {
    project: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct ReleaseInput {
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    projects: Option<Vec<String>>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default, rename = "dateStarted")]
    date_started: Option<String>,
    #[serde(default, rename = "dateReleased")]
    date_released: Option<String>,
    #[serde(default)]
    refs: Option<Value>,
    #[serde(default)]
    commits: Option<Value>,
    #[serde(default)]
    status: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ReleaseData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    date_started: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    date_released: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refs: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    commits: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    status: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SentryReleaseResponse {
    version: String,
    url: Option<String>,
    date_created: String,
    date_released: Option<String>,
    last_event: Option<String>,
    new_groups: i64,
    short_version: String,
    projects: Vec<ProjectSlugAndName>,
}

#[derive(Serialize)]
struct ProjectSlugAndName {
    slug: String,
    name: String,
}

#[derive(Serialize)]
struct ArtifactResponse {
    id: String,
    sha1: String,
    name: String,
    size: u64,
    dist: Option<String>,
    headers: BTreeMap<String, String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SentryDeployResponse {
    id: String,
    environment: String,
    date_started: Option<String>,
    date_finished: String,
    name: String,
    url: Option<String>,
    version: String,
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

fn parse_release_data(release: &Release) -> ReleaseData {
    release
        .data
        .as_deref()
        .and_then(|data| serde_json::from_str(data).ok())
        .unwrap_or_default()
}

fn merge_release_data(mut data: ReleaseData, input: &ReleaseInput) -> ReleaseData {
    if input.url.is_some() {
        data.url = input.url.clone();
    }
    if input.date_started.is_some() {
        data.date_started = input.date_started.clone();
    }
    if input.date_released.is_some() {
        data.date_released = input.date_released.clone();
    }
    if input.refs.is_some() {
        data.refs = input.refs.clone();
    }
    if input.commits.is_some() {
        data.commits = input.commits.clone();
    }
    if input.status.is_some() {
        data.status = input.status.clone();
    }
    data
}

fn sha1_hex(content: &[u8]) -> String {
    let mut sha1 = Sha1::new();
    sha1.update(content);
    hex::encode(sha1.finalize())
}

fn artifact_response_with_sha1(
    release_file: ReleaseFile,
    headers: BTreeMap<String, String>,
    sha1: String,
) -> ArtifactResponse {
    ArtifactResponse {
        id: release_file.id.to_string(),
        sha1,
        name: release_file.name,
        size: release_file.file_size.max(0) as u64,
        dist: if release_file.dist.is_empty() {
            None
        } else {
            Some(release_file.dist)
        },
        headers,
    }
}

async fn artifact_response_from_disk(
    release_file: ReleaseFile,
    headers: BTreeMap<String, String>,
) -> ArtifactResponse {
    let sha1 = match tokio::fs::read(&release_file.file_path).await {
        Ok(content) => sha1_hex(&content),
        Err(_) => release_file.sha256.clone().unwrap_or_default(),
    };
    artifact_response_with_sha1(release_file, headers, sha1)
}

fn deploy_response(deploy: Deploy, version: String) -> SentryDeployResponse {
    SentryDeployResponse {
        id: deploy.id.to_string(),
        environment: deploy.environment,
        date_started: deploy.date_started,
        date_finished: deploy.date_finished,
        name: deploy.name,
        url: if deploy.url.is_empty() {
            None
        } else {
            Some(deploy.url)
        },
        version,
    }
}

async fn resolve_project_id(state: &AppState, slug_or_id: &str) -> Result<i64, StatusCode> {
    if let Ok(id) = slug_or_id.parse::<i64>() {
        let row: Option<(i64,)> = sqlx::query_as("SELECT id FROM projects WHERE id = ?")
            .bind(id)
            .fetch_optional(state.db.reader())
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        return row.map(|r| r.0).ok_or(StatusCode::NOT_FOUND);
    }

    let row: Option<(i64,)> = sqlx::query_as("SELECT id FROM projects WHERE slug = ?")
        .bind(slug_or_id)
        .fetch_optional(state.db.reader())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    row.map(|r| r.0).ok_or(StatusCode::NOT_FOUND)
}

async fn associate_project(
    state: &AppState,
    release_id: i64,
    project_id: i64,
    project_slug: &str,
    version: &str,
) -> Result<(), (StatusCode, String)> {
    sqlx::query("INSERT OR IGNORE INTO release_projects (release_id, project_id) VALUES (?, ?)")
        .bind(release_id)
        .bind(project_id)
        .execute(state.db.writer())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let result = sqlx::query(
        "UPDATE issues SET status = 'resolved', resolved_in_release = ? \
         WHERE project_id = ? AND status = 'resolved' AND resolved_in_release = '__next__'",
    )
    .bind(version)
    .bind(project_id)
    .execute(state.db.writer())
    .await;

    if let Ok(r) = result
        && r.rows_affected() > 0
    {
        tracing::info!(
            project = project_slug,
            version,
            count = r.rows_affected(),
            "Auto-resolved issues for new release"
        );
    }

    Ok(())
}

async fn associate_projects(
    state: &AppState,
    release_id: i64,
    version: &str,
    projects: &[String],
) -> Result<(), (StatusCode, String)> {
    for slug in projects {
        let project_id = match resolve_project_id(state, slug).await {
            Ok(id) => id,
            Err(StatusCode::NOT_FOUND) => continue,
            Err(status) => return Err((status, "project lookup failed".to_string())),
        };
        associate_project(state, release_id, project_id, slug, version).await?;
    }
    Ok(())
}

async fn upsert_release(
    state: &AppState,
    version: &str,
    input: &ReleaseInput,
    route_project: Option<&str>,
) -> Result<Release, (StatusCode, String)> {
    let projects = input.projects.clone().unwrap_or_default();
    if let Some(project) = route_project {
        let project_id = resolve_project_id(state, project)
            .await
            .map_err(|status| (status, "project not found".to_string()))?;

        let existing: Option<Release> =
            sqlx::query_as("SELECT * FROM releases WHERE org_id = 1 AND version = ?")
                .bind(version)
                .fetch_optional(state.db.reader())
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        let data = merge_release_data(
            existing
                .as_ref()
                .map(parse_release_data)
                .unwrap_or_default(),
            input,
        );
        let data = serde_json::to_string(&data)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        let release = sqlx::query_as::<_, Release>(
            "INSERT INTO releases (org_id, version, data) VALUES (1, ?, ?) \
             ON CONFLICT(org_id, version) DO UPDATE SET data=excluded.data \
             RETURNING *",
        )
        .bind(version)
        .bind(&data)
        .fetch_one(state.db.writer())
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

        associate_project(state, release.id, project_id, project, version).await?;
        let extra_projects: Vec<String> = projects
            .into_iter()
            .filter(|slug| slug != project)
            .collect();
        associate_projects(state, release.id, version, &extra_projects).await?;
        return Ok(release);
    }

    let existing: Option<Release> =
        sqlx::query_as("SELECT * FROM releases WHERE org_id = 1 AND version = ?")
            .bind(version)
            .fetch_optional(state.db.reader())
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let data = merge_release_data(
        existing
            .as_ref()
            .map(parse_release_data)
            .unwrap_or_default(),
        input,
    );
    let data = serde_json::to_string(&data)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let release = sqlx::query_as::<_, Release>(
        "INSERT INTO releases (org_id, version, data) VALUES (1, ?, ?) \
         ON CONFLICT(org_id, version) DO UPDATE SET data=excluded.data \
         RETURNING *",
    )
    .bind(version)
    .bind(&data)
    .fetch_one(state.db.writer())
    .await
    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    associate_projects(state, release.id, version, &projects).await?;
    Ok(release)
}

async fn update_existing_release(
    state: &AppState,
    version: &str,
    input: &ReleaseInput,
    route_project: Option<&str>,
) -> Result<Release, (StatusCode, String)> {
    let exists: Option<(i64,)> =
        sqlx::query_as("SELECT id FROM releases WHERE org_id = 1 AND version = ?")
            .bind(version)
            .fetch_optional(state.db.reader())
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    exists.ok_or((StatusCode::NOT_FOUND, "Release not found".to_string()))?;
    upsert_release(state, version, input, route_project).await
}

async fn sentry_release_response(
    state: &AppState,
    release: Release,
) -> Result<SentryReleaseResponse, StatusCode> {
    let projects: Vec<(String, String)> = sqlx::query_as(
        "SELECT p.slug, p.name FROM projects p \
         JOIN release_projects rp ON rp.project_id = p.id \
         WHERE rp.release_id = ? ORDER BY p.slug",
    )
    .bind(release.id)
    .fetch_all(state.db.reader())
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let data = parse_release_data(&release);
    Ok(SentryReleaseResponse {
        version: release.version.clone(),
        url: data.url,
        date_created: release.created_at,
        date_released: data.date_released,
        last_event: None,
        new_groups: 0,
        short_version: make_short_version(&release.version),
        projects: projects
            .into_iter()
            .map(|(slug, name)| ProjectSlugAndName { slug, name })
            .collect(),
    })
}

async fn list_releases_for_project(
    state: &AppState,
    project: &str,
) -> Result<Vec<Release>, StatusCode> {
    let project_id = resolve_project_id(state, project).await?;
    sqlx::query_as(
        "SELECT r.* FROM releases r \
         JOIN release_projects rp ON rp.release_id = r.id \
         WHERE r.org_id = 1 AND rp.project_id = ? \
         ORDER BY r.created_at DESC LIMIT 100",
    )
    .bind(project_id)
    .fetch_all(state.db.reader())
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn list_release_responses(
    state: &AppState,
    releases: Vec<Release>,
) -> Result<Json<Vec<SentryReleaseResponse>>, StatusCode> {
    let mut response = Vec::with_capacity(releases.len());
    for release in releases {
        response.push(sentry_release_response(state, release).await?);
    }
    Ok(Json(response))
}

/// Internal release summary for the admin UI
#[derive(Serialize)]
struct ProjectReleaseSummary {
    version: String,
    created_at: String,
    file_count: i64,
}

async fn list_internal_project_releases(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<Vec<ProjectReleaseSummary>>, StatusCode> {
    let project_id = resolve_project_id(&state, &slug).await?;

    let rows: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT r.version, r.created_at, \
                COALESCE((SELECT COUNT(*) FROM release_files rf WHERE rf.release_id = r.id), 0) as file_count \
         FROM releases r \
         JOIN release_projects rp ON rp.release_id = r.id \
         WHERE rp.project_id = ? \
         ORDER BY r.created_at DESC LIMIT 100",
    )
    .bind(project_id)
    .fetch_all(state.db.reader())
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let response: Vec<ProjectReleaseSummary> = rows
        .into_iter()
        .map(|(version, created_at, file_count)| ProjectReleaseSummary {
            version,
            created_at,
            file_count,
        })
        .collect();

    Ok(Json(response))
}

async fn list_org_releases(
    State(state): State<AppState>,
    Path(_org): Path<String>,
    Query(params): Query<ReleaseQuery>,
) -> Result<Json<Vec<SentryReleaseResponse>>, StatusCode> {
    let releases = if let Some(project) = params.project {
        list_releases_for_project(&state, &project).await?
    } else {
        sqlx::query_as("SELECT * FROM releases WHERE org_id = 1 ORDER BY created_at DESC LIMIT 100")
            .fetch_all(state.db.reader())
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    };

    list_release_responses(&state, releases).await
}

async fn list_project_releases(
    State(state): State<AppState>,
    Path((_org, project)): Path<(String, String)>,
) -> Result<Json<Vec<SentryReleaseResponse>>, StatusCode> {
    let releases = list_releases_for_project(&state, &project).await?;
    list_release_responses(&state, releases).await
}

async fn create_org_release(
    State(state): State<AppState>,
    Path(_org): Path<String>,
    Json(input): Json<ReleaseInput>,
) -> Result<(StatusCode, Json<SentryReleaseResponse>), (StatusCode, String)> {
    let version = input
        .version
        .as_deref()
        .ok_or((StatusCode::BAD_REQUEST, "missing version".to_string()))?;
    let release = upsert_release(&state, version, &input, None).await?;
    let response = sentry_release_response(&state, release)
        .await
        .map_err(|status| (status, "release response failed".to_string()))?;
    Ok((StatusCode::CREATED, Json(response)))
}

async fn create_project_release(
    State(state): State<AppState>,
    Path((_org, project)): Path<(String, String)>,
    Json(input): Json<ReleaseInput>,
) -> Result<(StatusCode, Json<SentryReleaseResponse>), (StatusCode, String)> {
    let version = input
        .version
        .as_deref()
        .ok_or((StatusCode::BAD_REQUEST, "missing version".to_string()))?;
    let release = upsert_release(&state, version, &input, Some(&project)).await?;
    let response = sentry_release_response(&state, release)
        .await
        .map_err(|status| (status, "release response failed".to_string()))?;
    Ok((StatusCode::CREATED, Json(response)))
}

async fn get_org_release(
    State(state): State<AppState>,
    Path((_org, version)): Path<(String, String)>,
) -> Result<Json<SentryReleaseResponse>, StatusCode> {
    let release: Option<Release> =
        sqlx::query_as("SELECT * FROM releases WHERE org_id = 1 AND version = ?")
            .bind(&version)
            .fetch_optional(state.db.reader())
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    sentry_release_response(&state, release.ok_or(StatusCode::NOT_FOUND)?)
        .await
        .map(Json)
}

async fn get_project_release(
    State(state): State<AppState>,
    Path((_org, project, version)): Path<(String, String, String)>,
) -> Result<Json<SentryReleaseResponse>, StatusCode> {
    let project_id = resolve_project_id(&state, &project).await?;
    let release: Option<Release> = sqlx::query_as(
        "SELECT r.* FROM releases r \
         JOIN release_projects rp ON rp.release_id = r.id \
         WHERE r.org_id = 1 AND rp.project_id = ? AND r.version = ?",
    )
    .bind(project_id)
    .bind(&version)
    .fetch_optional(state.db.reader())
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    sentry_release_response(&state, release.ok_or(StatusCode::NOT_FOUND)?)
        .await
        .map(Json)
}

async fn update_org_release(
    State(state): State<AppState>,
    Path((_org, version)): Path<(String, String)>,
    Json(input): Json<ReleaseInput>,
) -> Result<Json<SentryReleaseResponse>, (StatusCode, String)> {
    let release = update_existing_release(&state, &version, &input, None).await?;
    sentry_release_response(&state, release)
        .await
        .map(Json)
        .map_err(|status| (status, "release response failed".to_string()))
}

async fn update_project_release(
    State(state): State<AppState>,
    Path((_org, project, version)): Path<(String, String, String)>,
    Json(input): Json<ReleaseInput>,
) -> Result<Json<SentryReleaseResponse>, (StatusCode, String)> {
    let release = update_existing_release(&state, &version, &input, Some(&project)).await?;
    sentry_release_response(&state, release)
        .await
        .map(Json)
        .map_err(|status| (status, "release response failed".to_string()))
}

async fn upload_release_file(
    State(state): State<AppState>,
    Path((_org, project, version)): Path<(String, String, String)>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<ArtifactResponse>), (StatusCode, String)> {
    let mut file_content: Option<Vec<u8>> = None;
    let mut artifact_name: Option<String> = None;
    let mut dist = String::new();
    let mut headers = BTreeMap::new();

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
            "header" => {
                let text = field
                    .text()
                    .await
                    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
                if let Some((key, value)) = text.split_once(':') {
                    headers.insert(key.trim().to_string(), value.trim().to_string());
                }
            }
            _ => {}
        }
    }

    let file_content =
        file_content.ok_or((StatusCode::BAD_REQUEST, "Missing 'file' field".to_string()))?;
    let artifact_name =
        artifact_name.ok_or((StatusCode::BAD_REQUEST, "Missing 'name' field".to_string()))?;

    let input = ReleaseInput::default();
    let release = upsert_release(&state, &version, &input, Some(&project)).await?;

    let max_upload = state.config.uploads.max_bytes;
    let content = if file_content.len() >= 2 && file_content[0] == 0x1f && file_content[1] == 0x8b {
        decompress_gzip_capped(&file_content, max_upload).map_err(|e| match e {
            DecompressError::SizeExceeded => (
                StatusCode::PAYLOAD_TOO_LARGE,
                "Decompressed artifact too large".into(),
            ),
            DecompressError::Io(io) => {
                (StatusCode::BAD_REQUEST, format!("Gzip decode error: {io}"))
            }
        })?
    } else {
        if file_content.len() > max_upload {
            return Err((StatusCode::PAYLOAD_TOO_LARGE, "Artifact too large".into()));
        }
        file_content
    };

    let mut sha256 = Sha256::new();
    sha256.update(&content);
    let sha256 = hex::encode(sha256.finalize());

    let sha1 = sha1_hex(&content);

    let stripped = artifact_name.strip_prefix("~/").unwrap_or(&artifact_name);
    let sanitized_name = stripped.replace(['/', '\\'], "_");
    let invalid_name = sanitized_name.is_empty()
        || sanitized_name == "."
        || sanitized_name == ".."
        || sanitized_name.starts_with('.')
        || sanitized_name.contains('\0');
    if invalid_name {
        return Err((StatusCode::BAD_REQUEST, "Invalid artifact name".to_string()));
    }

    let org_id: i64 = 1;
    let dir_path = format!(
        "{}/releases/{}/{}",
        state.config.artifacts_dir, org_id, release.id
    );
    let file_path = format!("{}/{}", dir_path, sanitized_name);

    tokio::fs::create_dir_all(&dir_path).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to create dir: {e}"),
        )
    })?;

    let canonical_dir = tokio::fs::canonicalize(&dir_path).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to resolve dir: {e}"),
        )
    })?;
    let target = canonical_dir.join(&sanitized_name);
    if !target.starts_with(&canonical_dir) {
        return Err((StatusCode::BAD_REQUEST, "Invalid artifact name".to_string()));
    }

    tokio::fs::write(&file_path, &content).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to write file: {e}"),
        )
    })?;

    let file_size = content.len() as i64;
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

    crate::worker::symbolication::invalidate_release_files(&version);
    crate::worker::symbolication::invalidate_source_map_path(&file_path);

    Ok((
        StatusCode::CREATED,
        Json(artifact_response_with_sha1(release_file, headers, sha1)),
    ))
}

async fn list_release_files(
    State(state): State<AppState>,
    Path((_org, _project, version)): Path<(String, String, String)>,
) -> Result<Json<Vec<ArtifactResponse>>, StatusCode> {
    let release: Option<(i64,)> =
        sqlx::query_as("SELECT id FROM releases WHERE org_id = 1 AND version = ?")
            .bind(&version)
            .fetch_optional(state.db.reader())
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let release_id = release.ok_or(StatusCode::NOT_FOUND)?.0;
    let files: Vec<ReleaseFile> =
        sqlx::query_as("SELECT * FROM release_files WHERE release_id = ? ORDER BY created_at DESC")
            .bind(release_id)
            .fetch_all(state.db.reader())
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut response = Vec::with_capacity(files.len());
    for file in files {
        response.push(artifact_response_from_disk(file, BTreeMap::new()).await);
    }

    Ok(Json(response))
}

async fn delete_release_file(
    State(state): State<AppState>,
    Path((_org, _project, version, file_id)): Path<(String, String, String, i64)>,
) -> Result<StatusCode, (StatusCode, String)> {
    let release_file: Option<ReleaseFile> = sqlx::query_as(
        "SELECT rf.* FROM release_files rf \
         JOIN releases r ON r.id = rf.release_id \
         WHERE r.org_id = 1 AND r.version = ? AND rf.id = ?",
    )
    .bind(&version)
    .bind(file_id)
    .fetch_optional(state.db.reader())
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let release_file = release_file.ok_or((StatusCode::NOT_FOUND, "file not found".to_string()))?;
    sqlx::query("DELETE FROM release_files WHERE id = ?")
        .bind(file_id)
        .execute(state.db.writer())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let _ = tokio::fs::remove_file(&release_file.file_path).await;
    crate::worker::symbolication::invalidate_release_files(&version);
    crate::worker::symbolication::invalidate_source_map_path(&release_file.file_path);

    Ok(StatusCode::NO_CONTENT)
}

async fn list_deploys(
    State(state): State<AppState>,
    Path((_org, version)): Path<(String, String)>,
) -> Result<Json<Vec<SentryDeployResponse>>, StatusCode> {
    let release: Option<(i64,)> =
        sqlx::query_as("SELECT id FROM releases WHERE org_id = 1 AND version = ?")
            .bind(&version)
            .fetch_optional(state.db.reader())
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let release_id = release.ok_or(StatusCode::NOT_FOUND)?.0;
    let deploys: Vec<Deploy> =
        sqlx::query_as("SELECT * FROM deploys WHERE release_id = ? ORDER BY date_finished DESC")
            .bind(release_id)
            .fetch_all(state.db.reader())
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(
        deploys
            .into_iter()
            .map(|deploy| deploy_response(deploy, version.clone()))
            .collect(),
    ))
}

async fn create_deploy(
    State(state): State<AppState>,
    Path((_org, version)): Path<(String, String)>,
    Json(input): Json<CreateDeploy>,
) -> Result<(StatusCode, Json<SentryDeployResponse>), (StatusCode, String)> {
    let release: Option<(i64,)> =
        sqlx::query_as("SELECT id FROM releases WHERE org_id = 1 AND version = ?")
            .bind(&version)
            .fetch_optional(state.db.reader())
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let release_id = release
        .ok_or((StatusCode::NOT_FOUND, "Release not found".to_string()))?
        .0;

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

    Ok((StatusCode::CREATED, Json(deploy_response(deploy, version))))
}
