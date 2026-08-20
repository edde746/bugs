//! Crashpad minidump ingest endpoint.
//!
//! `POST /api/{project_id}/minidump[/]?sentry_key=…` — the hard-crash
//! upload path used by sentry-native's out-of-process crashpad handler
//! (Linux and Windows desktop builds). See `crate::ingest::minidump`
//! for the payload contract. The handler stackwalks the dump, wraps the
//! synthesized event plus attachments into an ordinary envelope, and
//! feeds the existing worker pipeline.
//!
//! Responds `200 text/plain` with the hyphenated event UUID — crashpad
//! stores the response body as the report id.

use axum::{
    Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::post,
};
use bytes::Bytes;
use tracing::warn;

use crate::AppState;
use crate::api::ingest_auth::SentryAuth;
use crate::ingest::auth::resolve_project_key;
use crate::ingest::minidump::{
    MinidumpUpload, decompress_dump_container, has_minidump_magic, is_raw_minidump_content_type,
    parse_multipart, synthesize_event,
};

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/{project_id}/minidump", post(ingest_minidump))
}

async fn ingest_minidump(
    State(state): State<AppState>,
    Path(project_id): Path<i64>,
    headers: HeaderMap,
    auth: SentryAuth,
    body: Bytes,
) -> Result<(StatusCode, String), (StatusCode, String)> {
    let max_raw = state.config.ingest.max_raw_request_bytes;
    if body.len() > max_raw {
        return Err((StatusCode::PAYLOAD_TOO_LARGE, "Request too large".into()));
    }

    // Same opaque auth handling as the envelope endpoint.
    let (key, _project) = resolve_project_key(state.db.reader(), &auth.sentry_key)
        .await
        .map_err(|_| (StatusCode::UNAUTHORIZED, "unauthorized".into()))?;
    if project_id != key.project_id {
        return Err((StatusCode::UNAUTHORIZED, "unauthorized".into()));
    }
    if let Some(limit) = key.rate_limit
        && limit > 0
        && !state
            .rate_limiter
            .check(&auth.sentry_key, limit as u64)
            .await
    {
        return Err((StatusCode::TOO_MANY_REQUESTS, "Rate limit exceeded".into()));
    }

    // Crashpad gzips the entire request body by default
    // (Content-Encoding: gzip); sniff the magic as well since proxies
    // sometimes strip the header.
    let body = if body.len() >= 2 && body[0] == 0x1f && body[1] == 0x8b {
        super::ingest::decompress_gzip_capped(&body, max_raw).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                format!("Decompression failed: {e}"),
            )
        })?
    } else {
        body.to_vec()
    };

    // Raw-dump bodies vs multipart, decided by Content-Type like relay.
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let upload = if is_raw_minidump_content_type(content_type) || content_type.is_empty() {
        MinidumpUpload {
            dump: Some(body),
            ..Default::default()
        }
    } else {
        let boundary = multer::parse_boundary(content_type).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                format!("invalid content type: {e}"),
            )
        })?;
        parse_multipart(Bytes::from(body), &boundary, max_raw)
            .await
            .map_err(|e| (StatusCode::BAD_REQUEST, e))?
    };

    let mut upload = upload;
    let dump = upload
        .dump
        .take()
        .ok_or((StatusCode::BAD_REQUEST, "missing minidump".into()))?;
    // Some clients compress the dump payload itself, independent of the
    // request body encoding.
    let dump =
        decompress_dump_container(dump, max_raw).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    if !has_minidump_magic(&dump) {
        return Err((StatusCode::BAD_REQUEST, "invalid minidump".into()));
    }

    // Stackwalk off the async executor: scanning-heuristic unwinds on a
    // multi-MB dump are CPU-bound.
    let handle = tokio::runtime::Handle::current();
    let (event_id, event, dump, upload) = tokio::task::spawn_blocking(move || {
        let result = handle.block_on(synthesize_event(&dump, &upload));
        result.map(|(event_id, event)| (event_id, event, dump, upload))
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("join: {e}")))?
    .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    let envelope = build_envelope(
        &event_id,
        &event,
        &dump,
        upload.dump_filename.as_deref(),
        &upload.attachments,
        state.config.ingest.max_envelope_bytes,
    )
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    super::ingest::persist_envelope(&state, key.project_id, &event_id, &envelope)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Storage failed: {e}"),
            )
        })?;

    Ok((StatusCode::OK, hyphenate(&event_id)))
}

/// Wraps the synthesized event, the minidump, and any user attachments
/// into an ordinary envelope for the worker pipeline. Attachments that
/// would push the envelope past `max_bytes` are dropped (user parts
/// first, the dump last) — the event itself always survives.
fn build_envelope(
    event_id: &str,
    event: &serde_json::Value,
    dump: &[u8],
    dump_filename: Option<&str>,
    attachments: &[crate::ingest::minidump::UploadAttachment],
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    let event_json = serde_json::to_vec(event).map_err(|e| format!("serialize event: {e}"))?;

    struct Part<'a> {
        header: String,
        payload: &'a [u8],
    }
    let mut parts = vec![Part {
        header: format!(
            "{}\n",
            serde_json::json!({
                "type": "attachment",
                "length": dump.len(),
                "filename": dump_filename.unwrap_or("Minidump"),
                "attachment_type": "event.minidump",
                "content_type": "application/octet-stream",
            })
        ),
        payload: dump,
    }];
    for attachment in attachments {
        parts.push(Part {
            header: format!(
                "{}\n",
                serde_json::json!({
                    "type": "attachment",
                    "length": attachment.data.len(),
                    "filename": attachment.filename,
                    "attachment_type": "event.attachment",
                    "content_type": attachment
                        .content_type
                        .as_deref()
                        .unwrap_or("application/octet-stream"),
                })
            ),
            payload: &attachment.data,
        });
    }

    let envelope_header = format!("{{\"event_id\":\"{event_id}\"}}\n");
    let event_header = format!("{{\"type\":\"event\",\"length\":{}}}\n", event_json.len());
    let base_len = envelope_header.len() + event_header.len() + event_json.len() + 1;

    // Budget check: drop user attachments before the dump.
    let mut kept: Vec<&Part> = parts.iter().collect();
    loop {
        let total: usize = base_len
            + kept
                .iter()
                .map(|p| p.header.len() + p.payload.len() + 1)
                .sum::<usize>();
        if total <= max_bytes || kept.is_empty() {
            break;
        }
        let dropped = kept.remove(kept.len() - 1);
        warn!(
            "minidump envelope over {max_bytes} bytes; dropping attachment part ({} bytes)",
            dropped.payload.len()
        );
    }

    let mut envelope = Vec::with_capacity(base_len);
    envelope.extend_from_slice(envelope_header.as_bytes());
    envelope.extend_from_slice(event_header.as_bytes());
    envelope.extend_from_slice(&event_json);
    envelope.push(b'\n');
    for part in kept {
        envelope.extend_from_slice(part.header.as_bytes());
        envelope.extend_from_slice(part.payload);
        envelope.push(b'\n');
    }
    Ok(envelope)
}

/// Crashpad expects a hyphenated UUID as the response body.
fn hyphenate(event_id: &str) -> String {
    match uuid::Uuid::parse_str(event_id) {
        Ok(uuid) => uuid.hyphenated().to_string(),
        Err(_) => event_id.to_string(),
    }
}
