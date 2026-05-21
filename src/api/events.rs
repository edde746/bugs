use crate::AppState;
use crate::models::event::{Event, EventAttachment, EventSummary};
use axum::{
    Json, Router,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderValue, StatusCode, header},
    response::Response,
    routing::get,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Deserialize;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/internal/issues/{id}/events",
            get(list_events_for_issue),
        )
        .route("/api/internal/issues/{id}/events/latest", get(latest_event))
        .route(
            "/api/internal/events/{id}/attachments",
            get(list_event_attachments),
        )
        .route(
            "/api/internal/events/{id}/attachments/{attachment_id}/text",
            get(get_attachment_text),
        )
        .route(
            "/api/internal/events/{id}/attachments/{attachment_id}/download",
            get(download_attachment),
        )
        .route("/api/internal/events/{id}", get(get_event))
}

#[derive(Deserialize)]
struct EventQuery {
    #[serde(default = "default_event_limit")]
    limit: i64,
    cursor: Option<String>,
}

fn default_event_limit() -> i64 {
    50
}

#[derive(Deserialize, serde::Serialize)]
struct EventCursor {
    ts: String,
    id: i64,
}

fn decode_event_cursor(cursor: &str) -> Option<EventCursor> {
    let bytes = URL_SAFE_NO_PAD.decode(cursor).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn encode_event_cursor(timestamp: &str, id: i64) -> String {
    let data = EventCursor {
        ts: timestamp.to_string(),
        id,
    };
    let json = serde_json::to_vec(&data).unwrap();
    URL_SAFE_NO_PAD.encode(&json)
}

const EVENT_LIST_COLUMNS: &str = "id, event_id, project_id, issue_id, timestamp, received_at, level, \
     platform, release, environment, transaction_name, trace_id, message, \
     title, exception_values, stacktrace_functions";

async fn list_events_for_issue(
    State(state): State<AppState>,
    Path(issue_id): Path<i64>,
    Query(params): Query<EventQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let limit = params.limit.clamp(1, 100);

    let events: Vec<EventSummary> = if let Some(ref cursor_str) = params.cursor {
        let cursor = decode_event_cursor(cursor_str).ok_or(StatusCode::BAD_REQUEST)?;
        sqlx::query_as(&format!(
            "SELECT {EVENT_LIST_COLUMNS} FROM events \
             WHERE issue_id = ? AND (timestamp < ? OR (timestamp = ? AND id < ?)) \
             ORDER BY timestamp DESC, id DESC LIMIT ?"
        ))
        .bind(issue_id)
        .bind(&cursor.ts)
        .bind(&cursor.ts)
        .bind(cursor.id)
        .bind(limit + 1)
        .fetch_all(state.db.reader())
        .await
    } else {
        sqlx::query_as(&format!(
            "SELECT {EVENT_LIST_COLUMNS} FROM events \
             WHERE issue_id = ? ORDER BY timestamp DESC, id DESC LIMIT ?"
        ))
        .bind(issue_id)
        .bind(limit + 1)
        .fetch_all(state.db.reader())
        .await
    }
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let has_next = events.len() as i64 > limit;
    let items: Vec<&EventSummary> = events.iter().take(limit as usize).collect();
    let next_cursor = if has_next {
        items
            .last()
            .map(|e| encode_event_cursor(&e.timestamp, e.id))
    } else {
        None
    };

    Ok(Json(serde_json::json!({
        "events": items,
        "nextCursor": next_cursor,
    })))
}

async fn latest_event(
    State(state): State<AppState>,
    Path(issue_id): Path<i64>,
) -> Result<Json<Event>, StatusCode> {
    sqlx::query_as("SELECT * FROM events WHERE issue_id = ? ORDER BY timestamp DESC LIMIT 1")
        .bind(issue_id)
        .fetch_optional(state.db.reader())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn get_event(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<Event>, StatusCode> {
    sqlx::query_as("SELECT * FROM events WHERE id = ?")
        .bind(id)
        .fetch_optional(state.db.reader())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn list_event_attachments(
    State(state): State<AppState>,
    Path(event_id): Path<i64>,
) -> Result<Json<Vec<EventAttachment>>, StatusCode> {
    let attachments: Vec<EventAttachment> = sqlx::query_as(
        "SELECT id, event_id, name, content_type, attachment_type, size, created_at \
         FROM event_attachments WHERE event_id = ? ORDER BY id ASC",
    )
    .bind(event_id)
    .fetch_all(state.db.reader())
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(attachments))
}

#[derive(sqlx::FromRow)]
struct AttachmentBody {
    name: String,
    content_type: Option<String>,
    body: Vec<u8>,
}

async fn get_attachment_text(
    State(state): State<AppState>,
    Path((event_id, attachment_id)): Path<(i64, i64)>,
) -> Result<Response, StatusCode> {
    let attachment = fetch_attachment_body(&state, event_id, attachment_id).await?;
    let text = String::from_utf8_lossy(&attachment.body).into_owned();
    let mut response = Response::new(Body::from(text));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    Ok(response)
}

async fn download_attachment(
    State(state): State<AppState>,
    Path((event_id, attachment_id)): Path<(i64, i64)>,
) -> Result<Response, StatusCode> {
    let attachment = fetch_attachment_body(&state, event_id, attachment_id).await?;
    let mut response = Response::new(Body::from(attachment.body));
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        header_value_or_octet(attachment.content_type.as_deref()),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!(
            "attachment; filename=\"{}\"",
            content_disposition_filename(&attachment.name)
        ))
        .unwrap_or_else(|_| HeaderValue::from_static("attachment")),
    );
    Ok(response)
}

async fn fetch_attachment_body(
    state: &AppState,
    event_id: i64,
    attachment_id: i64,
) -> Result<AttachmentBody, StatusCode> {
    sqlx::query_as(
        "SELECT name, content_type, body FROM event_attachments WHERE event_id = ? AND id = ?",
    )
    .bind(event_id)
    .bind(attachment_id)
    .fetch_optional(state.db.reader())
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .ok_or(StatusCode::NOT_FOUND)
}

fn header_value_or_octet(value: Option<&str>) -> HeaderValue {
    value
        .and_then(|v| HeaderValue::from_str(v).ok())
        .unwrap_or_else(|| HeaderValue::from_static("application/octet-stream"))
}

fn content_disposition_filename(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .take(200)
        .map(|c| match c {
            '/' => '_',
            ' '..='!' | '#'..='[' | ']'..='~' => c,
            _ => '_',
        })
        .collect();

    if sanitized.trim().is_empty() {
        "attachment".to_string()
    } else {
        sanitized
    }
}
