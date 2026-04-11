use axum::{
    Router, Json,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post, delete},
};

use crate::AppState;
use crate::models::project::*;
use crate::util::id::generate_public_key;
use crate::sentry_protocol::dsn::build_dsn;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/internal/projects", get(list_projects).post(create_project))
        .route("/api/internal/projects/{id}", get(get_project).put(update_project).delete(delete_project))
        .route("/api/internal/projects/{id}/keys", get(list_keys).post(create_key))
        .route("/api/internal/projects/{id}/keys/{key_id}", delete(delete_key))
}

async fn list_projects(State(state): State<AppState>) -> Result<Json<Vec<Project>>, StatusCode> {
    let projects: Vec<Project> = sqlx::query_as("SELECT * FROM projects ORDER BY created_at DESC")
        .fetch_all(state.db.reader())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(projects))
}

async fn create_project(
    State(state): State<AppState>,
    Json(input): Json<CreateProject>,
) -> Result<(StatusCode, Json<Project>), (StatusCode, String)> {
    let result = sqlx::query_as::<_, Project>(
        "INSERT INTO projects (name, slug, platform) VALUES (?, ?, ?) RETURNING *"
    )
    .bind(&input.name)
    .bind(&input.slug)
    .bind(&input.platform)
    .fetch_one(state.db.writer())
    .await
    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    // Auto-create a default project key
    let public_key = generate_public_key();
    sqlx::query("INSERT INTO project_keys (project_id, public_key) VALUES (?, ?)")
        .bind(result.id)
        .bind(&public_key)
        .execute(state.db.writer())
        .await
        .ok();

    // Auto-create project settings
    sqlx::query("INSERT OR IGNORE INTO project_settings (project_id) VALUES (?)")
        .bind(result.id)
        .execute(state.db.writer())
        .await
        .ok();

    Ok((StatusCode::CREATED, Json(result)))
}

async fn get_project(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<Project>, StatusCode> {
    sqlx::query_as("SELECT * FROM projects WHERE id = ?")
        .bind(id)
        .fetch_optional(state.db.reader())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn update_project(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<CreateProject>,
) -> Result<Json<Project>, StatusCode> {
    sqlx::query_as(
        "UPDATE projects SET name = ?, slug = ?, platform = ? WHERE id = ? RETURNING *"
    )
    .bind(&input.name)
    .bind(&input.slug)
    .bind(&input.platform)
    .bind(id)
    .fetch_optional(state.db.writer())
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .map(Json)
    .ok_or(StatusCode::NOT_FOUND)
}

async fn delete_project(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> StatusCode {
    match sqlx::query("DELETE FROM projects WHERE id = ?")
        .bind(id)
        .execute(state.db.writer())
        .await
    {
        Ok(r) if r.rows_affected() > 0 => StatusCode::NO_CONTENT,
        _ => StatusCode::NOT_FOUND,
    }
}

async fn list_keys(
    State(state): State<AppState>,
    Path(project_id): Path<i64>,
) -> Result<Json<Vec<serde_json::Value>>, StatusCode> {
    let keys: Vec<ProjectKey> = sqlx::query_as(
        "SELECT * FROM project_keys WHERE project_id = ? ORDER BY created_at DESC"
    )
    .bind(project_id)
    .fetch_all(state.db.reader())
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Enrich with DSN
    let host = "localhost:9000"; // TODO: derive from config/request
    let enriched: Vec<serde_json::Value> = keys.iter().map(|k| {
        let dsn = build_dsn("http", host, &k.public_key, k.project_id);
        serde_json::json!({
            "id": k.id,
            "project_id": k.project_id,
            "public_key": k.public_key,
            "label": k.label,
            "is_active": k.is_active,
            "dsn": dsn,
            "created_at": k.created_at,
        })
    }).collect();

    Ok(Json(enriched))
}

async fn create_key(
    State(state): State<AppState>,
    Path(project_id): Path<i64>,
    Json(input): Json<CreateProjectKey>,
) -> Result<(StatusCode, Json<ProjectKey>), (StatusCode, String)> {
    let public_key = generate_public_key();
    let label = input.label.unwrap_or_else(|| "Default".to_string());

    let key = sqlx::query_as::<_, ProjectKey>(
        "INSERT INTO project_keys (project_id, public_key, label, rate_limit) VALUES (?, ?, ?, ?) RETURNING *"
    )
    .bind(project_id)
    .bind(&public_key)
    .bind(&label)
    .bind(input.rate_limit)
    .fetch_one(state.db.writer())
    .await
    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    Ok((StatusCode::CREATED, Json(key)))
}

async fn delete_key(
    State(state): State<AppState>,
    Path((_, key_id)): Path<(i64, i64)>,
) -> StatusCode {
    match sqlx::query("UPDATE project_keys SET is_active = 0 WHERE id = ?")
        .bind(key_id)
        .execute(state.db.writer())
        .await
    {
        Ok(r) if r.rows_affected() > 0 => StatusCode::NO_CONTENT,
        _ => StatusCode::NOT_FOUND,
    }
}
