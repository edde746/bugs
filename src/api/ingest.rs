use axum::{
    Router,
    extract::{Path, State},
    http::{StatusCode, HeaderMap},
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
        .route("/api/{project_id}/security/", post(ingest_security))
}

async fn ingest_envelope(
    State(state): State<AppState>,
    Path(project_id): Path<i64>,
    headers: HeaderMap,
    auth: SentryAuth,
    body: Bytes,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // Check raw size (pre-decompression)
    if body.len() > state.config.ingest.max_raw_request_bytes {
        return Err((StatusCode::PAYLOAD_TOO_LARGE, "Request too large".into()));
    }

    // Validate auth -> resolve project key
    let (key, _project) = resolve_project_key(state.db.reader(), &auth.sentry_key)
        .await
        .map_err(|_| (StatusCode::UNAUTHORIZED, "Invalid project key".into()))?;

    // Check rate limit
    let rate_limit = key.rate_limit
        .or_else(|| {
            // Could check project_settings.rate_limit_per_min here too
            None
        });
    if let Some(limit) = rate_limit {
        if limit > 0 && !state.rate_limiter.check(&auth.sentry_key, limit as u64).await {
            return Err((
                StatusCode::TOO_MANY_REQUESTS,
                "Rate limit exceeded".into(),
            ));
        }
    }

    // Check origin allowlist
    if let Some(origin) = headers.get("origin").and_then(|v| v.to_str().ok()) {
        let settings: Option<(Option<String>,)> = sqlx::query_as(
            "SELECT allowed_origins FROM project_settings WHERE project_id = ?"
        )
        .bind(key.project_id)
        .fetch_optional(state.db.reader())
        .await
        .unwrap_or(None);

        if let Some((Some(allowed_json),)) = settings {
            if let Ok(allowed) = serde_json::from_str::<Vec<String>>(&allowed_json) {
                if !allowed.is_empty() {
                    let origin_allowed = allowed.iter().any(|pattern| {
                        if pattern.contains('*') {
                            let re_pattern = pattern.replace('.', "\\.").replace('*', ".*");
                            regex::Regex::new(&re_pattern)
                                .map(|re| re.is_match(origin))
                                .unwrap_or(false)
                        } else {
                            origin == pattern
                        }
                    });
                    if !origin_allowed {
                        return Err((StatusCode::FORBIDDEN, "Origin not allowed".into()));
                    }
                }
            }
        }
    }

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

    // Extract event_id from first line (fast path — no full envelope parse)
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

    // Best-effort channel send (envelope is durable in DB; poller picks up if channel full)
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
    headers: HeaderMap,
    auth: SentryAuth,
    body: Bytes,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let event_id = generate_event_id();
    let envelope_body = crate::ingest::store::wrap_store_body(&body, &event_id);
    let envelope_bytes = Bytes::from(envelope_body);
    ingest_envelope(State(state), Path(project_id), headers, auth, envelope_bytes).await
}

/// CSP / security report ingest (best-effort: store as event envelope)
async fn ingest_security(
    State(state): State<AppState>,
    Path(project_id): Path<i64>,
    headers: HeaderMap,
    auth: SentryAuth,
    body: Bytes,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let event_id = generate_event_id();
    let envelope_body = crate::ingest::store::wrap_store_body(&body, &event_id);
    let envelope_bytes = Bytes::from(envelope_body);
    ingest_envelope(State(state), Path(project_id), headers, auth, envelope_bytes).await
}

fn decompress_gzip(data: &[u8]) -> Result<Vec<u8>, std::io::Error> {
    use flate2::read::GzDecoder;
    use std::io::Read;
    let mut decoder = GzDecoder::new(data);
    let mut decompressed = Vec::new();
    decoder.read_to_end(&mut decompressed)?;
    Ok(decompressed)
}
