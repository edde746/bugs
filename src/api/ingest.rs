use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::post,
    Json,
};
use bytes::Bytes;
use serde_json::json;

use crate::AppState;
use crate::api::ingest_auth::SentryAuth;
use crate::ingest::auth::resolve_project_key;
use crate::sentry_protocol::envelope::extract_event_id;
use crate::util::id::generate_event_id;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/{project_id}/envelope/", post(ingest_envelope))
        .route("/api/{project_id}/store/", post(ingest_store))
}

async fn ingest_envelope(
    State(state): State<AppState>,
    Path(project_id): Path<i64>,
    auth: SentryAuth,
    body: Bytes,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // Check raw size
    if body.len() > state.config.ingest.max_raw_request_bytes {
        return Err((StatusCode::PAYLOAD_TOO_LARGE, "Request too large".into()));
    }

    // Validate auth
    let (_key, _project) = resolve_project_key(state.db.reader(), &auth.sentry_key)
        .await
        .map_err(|_| (StatusCode::UNAUTHORIZED, "Invalid project key".into()))?;

    // Decompress if gzipped
    let data = if body.len() >= 2 && body[0] == 0x1f && body[1] == 0x8b {
        decompress_gzip(&body)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Decompression failed: {e}")))?
    } else {
        body.to_vec()
    };

    // Check decompressed size
    if data.len() > state.config.ingest.max_envelope_bytes {
        return Err((StatusCode::PAYLOAD_TOO_LARGE, "Decompressed envelope too large".into()));
    }

    // Extract event_id from first line (fast path)
    let event_id = extract_event_id(&data)
        .unwrap_or_else(generate_event_id);

    // Store raw envelope
    sqlx::query(
        "INSERT OR IGNORE INTO event_envelopes (project_id, event_id, body, state) VALUES (?, ?, ?, 'pending')"
    )
    .bind(project_id)
    .bind(&event_id)
    .bind(&data)
    .execute(state.db.writer())
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Storage failed: {e}")))?;

    // Best-effort channel send
    let envelope_id: Option<(i64,)> = sqlx::query_as(
        "SELECT id FROM event_envelopes WHERE project_id = ? AND event_id = ?"
    )
    .bind(project_id)
    .bind(&event_id)
    .fetch_optional(state.db.reader())
    .await
    .unwrap_or(None);

    if let Some((id,)) = envelope_id {
        let _ = state.worker_tx.try_send(id);
    }

    Ok((StatusCode::OK, Json(json!({ "id": event_id }))))
}

async fn ingest_store(
    State(state): State<AppState>,
    Path(project_id): Path<i64>,
    auth: SentryAuth,
    body: Bytes,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // Wrap legacy store body as envelope
    let event_id = generate_event_id();
    let envelope_body = crate::ingest::store::wrap_store_body(&body, &event_id);

    // Re-use envelope handler logic
    let envelope_bytes = Bytes::from(envelope_body);
    ingest_envelope(
        State(state),
        Path(project_id),
        auth,
        envelope_bytes,
    ).await
}

fn decompress_gzip(data: &[u8]) -> Result<Vec<u8>, std::io::Error> {
    use flate2::read::GzDecoder;
    use std::io::Read;

    let mut decoder = GzDecoder::new(data);
    let mut decompressed = Vec::new();
    decoder.read_to_end(&mut decompressed)?;
    Ok(decompressed)
}
