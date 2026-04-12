use crate::AppState;
use crate::models::issue::*;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Deserialize;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/internal/projects/{slug}/issues", get(list_issues))
        .route(
            "/api/internal/projects/{slug}/issues/filters",
            get(list_issue_filters),
        )
        .route(
            "/api/internal/issues/{id}",
            get(get_issue).put(update_issue).delete(delete_issue),
        )
}

#[derive(Deserialize)]
struct IssueQuery {
    status: Option<String>,
    sort: Option<String>,
    cursor: Option<String>,
    query: Option<String>,
    release: Option<String>,
    environment: Option<String>,
    level: Option<String>,
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 {
    25
}

#[derive(Deserialize, serde::Serialize)]
struct CursorData {
    v: String,
    id: i64,
}

fn decode_cursor(cursor: &str) -> Option<CursorData> {
    let bytes = URL_SAFE_NO_PAD.decode(cursor).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn encode_cursor(sort_value: &str, id: i64) -> String {
    let data = CursorData {
        v: sort_value.to_string(),
        id,
    };
    let json = serde_json::to_vec(&data).unwrap();
    URL_SAFE_NO_PAD.encode(&json)
}

async fn resolve_project_id(state: &AppState, slug: &str) -> Result<i64, StatusCode> {
    let project: Option<(i64,)> = sqlx::query_as("SELECT id FROM projects WHERE slug = ?")
        .bind(slug)
        .fetch_optional(state.db.reader())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    project.ok_or(StatusCode::NOT_FOUND).map(|p| p.0)
}

async fn list_issues(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(params): Query<IssueQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let project_id = resolve_project_id(&state, &slug).await?;

    let limit = params.limit.min(100);
    let status = params.status.as_deref().unwrap_or("unresolved");
    let sort_col = match params.sort.as_deref() {
        Some("first_seen") => "first_seen",
        Some("event_count") | Some("events") => "event_count",
        _ => "last_seen",
    };

    let query_like = params
        .query
        .as_ref()
        .filter(|q| !q.is_empty())
        .map(|q| format!("%{q}%"));

    let mut extra_where = String::new();
    if params.level.is_some() {
        extra_where.push_str(" AND level = ?");
    }
    if params.release.is_some() || params.environment.is_some() {
        let mut event_conds = vec!["issue_id = issues.id", "project_id = ?"];
        if params.release.is_some() {
            event_conds.push("release = ?");
        }
        if params.environment.is_some() {
            event_conds.push("environment = ?");
        }
        extra_where.push_str(&format!(
            " AND EXISTS (SELECT 1 FROM events WHERE {})",
            event_conds.join(" AND ")
        ));
    }
    if query_like.is_some() {
        extra_where.push_str(" AND title LIKE ?");
    }

    macro_rules! bind_filters {
        ($q:expr) => {{
            let mut q = $q;
            if let Some(ref v) = params.level {
                q = q.bind(v.as_str());
            }
            if params.release.is_some() || params.environment.is_some() {
                q = q.bind(project_id);
                if let Some(ref v) = params.release {
                    q = q.bind(v.as_str());
                }
                if let Some(ref v) = params.environment {
                    q = q.bind(v.as_str());
                }
            }
            if let Some(ref v) = query_like {
                q = q.bind(v.as_str());
            }
            q
        }};
    }

    let issues: Vec<Issue> = if let Some(ref cursor_str) = params.cursor {
        if let Some(cursor) = decode_cursor(cursor_str) {
            let sql = format!(
                "SELECT * FROM issues WHERE project_id = ? AND status = ? \
                 AND ({sort_col} < ? OR ({sort_col} = ? AND id < ?)){extra_where} \
                 ORDER BY {sort_col} DESC, id DESC LIMIT ?"
            );
            let q = sqlx::query_as::<_, Issue>(&sql)
                .bind(project_id)
                .bind(status)
                .bind(cursor.v.as_str())
                .bind(cursor.v.as_str())
                .bind(cursor.id);
            bind_filters!(q)
                .bind(limit + 1)
                .fetch_all(state.db.reader())
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        } else {
            return Err(StatusCode::BAD_REQUEST);
        }
    } else {
        let sql = format!(
            "SELECT * FROM issues WHERE project_id = ? AND status = ?{extra_where} \
             ORDER BY {sort_col} DESC, id DESC LIMIT ?"
        );
        let q = sqlx::query_as::<_, Issue>(&sql)
            .bind(project_id)
            .bind(status);
        bind_filters!(q)
            .bind(limit + 1)
            .fetch_all(state.db.reader())
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    };

    let has_next = issues.len() as i64 > limit;
    let items: Vec<&Issue> = issues.iter().take(limit as usize).collect();
    let next_cursor = if has_next {
        items.last().map(|i| {
            let sort_value = match sort_col {
                "first_seen" => &i.first_seen,
                "event_count" => return encode_cursor(&i.event_count.to_string(), i.id),
                _ => &i.last_seen,
            };
            encode_cursor(sort_value, i.id)
        })
    } else {
        None
    };

    Ok(Json(serde_json::json!({
        "issues": items,
        "nextCursor": next_cursor,
    })))
}

async fn list_issue_filters(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let project_id = resolve_project_id(&state, &slug).await?;

    let (releases, environments, levels) = tokio::try_join!(
        sqlx::query_scalar::<_, String>(
            "SELECT DISTINCT release FROM events \
             WHERE project_id = ? AND release IS NOT NULL ORDER BY release LIMIT 100"
        )
        .bind(project_id)
        .fetch_all(state.db.reader()),
        sqlx::query_scalar::<_, String>(
            "SELECT DISTINCT environment FROM events \
             WHERE project_id = ? AND environment IS NOT NULL ORDER BY environment LIMIT 100"
        )
        .bind(project_id)
        .fetch_all(state.db.reader()),
        sqlx::query_scalar::<_, String>(
            "SELECT DISTINCT level FROM issues WHERE project_id = ? ORDER BY level"
        )
        .bind(project_id)
        .fetch_all(state.db.reader()),
    )
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(serde_json::json!({
        "releases": releases,
        "environments": environments,
        "levels": levels,
    })))
}

async fn get_issue(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<Issue>, StatusCode> {
    sqlx::query_as("SELECT * FROM issues WHERE id = ?")
        .bind(id)
        .fetch_optional(state.db.reader())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn update_issue(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<UpdateIssue>,
) -> Result<Json<Issue>, StatusCode> {
    if let Some(status) = &input.status {
        match status.as_str() {
            "ignored" => {
                sqlx::query(
                    "UPDATE issues SET status = 'ignored', snooze_until = ?, snooze_event_count = ?, \
                     resolved_in_release = NULL WHERE id = ?",
                )
                .bind(&input.snooze_until)
                .bind(input.snooze_event_count)
                .bind(id)
                .execute(state.db.writer())
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

                sqlx::query("INSERT INTO issue_activity (issue_id, kind) VALUES (?, 'ignored')")
                    .bind(id)
                    .execute(state.db.writer())
                    .await
                    .ok();
            }
            "resolved" => {
                sqlx::query(
                    "UPDATE issues SET status = 'resolved', snooze_until = NULL, snooze_event_count = NULL, \
                     resolved_in_release = ? WHERE id = ?",
                )
                .bind(&input.resolved_in_release)
                .bind(id)
                .execute(state.db.writer())
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

                let data = input
                    .resolved_in_release
                    .as_ref()
                    .map(|r| serde_json::json!({ "release": r }).to_string());
                sqlx::query(
                    "INSERT INTO issue_activity (issue_id, kind, data) VALUES (?, 'resolved', ?)",
                )
                .bind(id)
                .bind(&data)
                .execute(state.db.writer())
                .await
                .ok();
            }
            "unresolved" => {
                sqlx::query(
                    "UPDATE issues SET status = 'unresolved', snooze_until = NULL, snooze_event_count = NULL, \
                     resolved_in_release = NULL WHERE id = ?",
                )
                .bind(id)
                .execute(state.db.writer())
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

                sqlx::query("INSERT INTO issue_activity (issue_id, kind) VALUES (?, 'unresolved')")
                    .bind(id)
                    .execute(state.db.writer())
                    .await
                    .ok();
            }
            _ => {
                return Err(StatusCode::BAD_REQUEST);
            }
        }
    }

    get_issue(State(state), Path(id)).await
}

async fn delete_issue(State(state): State<AppState>, Path(id): Path<i64>) -> StatusCode {
    match sqlx::query("DELETE FROM issues WHERE id = ?")
        .bind(id)
        .execute(state.db.writer())
        .await
    {
        Ok(r) if r.rows_affected() > 0 => StatusCode::NO_CONTENT,
        _ => StatusCode::NOT_FOUND,
    }
}
