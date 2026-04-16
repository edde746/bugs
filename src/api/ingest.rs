use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
};
use bytes::Bytes;
use lru::LruCache;
use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::json;

use crate::AppState;
use crate::api::ingest_auth::SentryAuth;
use crate::ingest::auth::resolve_project_key;
use crate::sentry_protocol::envelope::extract_event_id;
use crate::util::id::generate_event_id;

/// Compiled origin allowlist matcher. Built once per project and cached so
/// the ingest hot path doesn't recompile a regex on every request.
enum OriginMatcher {
    Exact(String),
    Wildcard(Regex),
}

impl OriginMatcher {
    fn matches(&self, origin: &str) -> bool {
        // Origin schemes/hosts are case-insensitive per RFC 6454; compare
        // accordingly so an allowlist entry of "https://Example.com"
        // still matches browsers sending "https://example.com".
        match self {
            OriginMatcher::Exact(s) => s.eq_ignore_ascii_case(origin),
            OriginMatcher::Wildcard(re) => re.is_match(origin),
        }
    }
}

struct CachedOrigins {
    fetched_at: Instant,
    matchers: Arc<Vec<OriginMatcher>>,
}

/// Per-project compiled origin allowlist with a short TTL.
/// TTL keeps the cache simple (no explicit invalidation needed) while
/// still capping the rate of DB hits + regex compilations to roughly
/// one per project per minute. LRU-bounded so a large multi-project
/// deployment can't grow this map unbounded.
static ORIGIN_CACHE: Lazy<Mutex<LruCache<i64, CachedOrigins>>> = Lazy::new(|| {
    Mutex::new(LruCache::new(
        NonZeroUsize::new(1024).expect("non-zero cap"),
    ))
});
const ORIGIN_CACHE_TTL: Duration = Duration::from_secs(60);

fn compile_origin_pattern(pattern: &str) -> OriginMatcher {
    if pattern.contains('*') {
        // Enable case-insensitive regex so wildcard patterns match the
        // same way as exact entries (see OriginMatcher::matches).
        let re_pattern = format!("(?i)^{}$", pattern.replace('.', "\\.").replace('*', ".*"));
        match Regex::new(&re_pattern) {
            Ok(re) => OriginMatcher::Wildcard(re),
            // Fall back to literal match if the pattern is somehow invalid
            // (shouldn't happen because we control the substitution).
            Err(_) => OriginMatcher::Exact(pattern.to_string()),
        }
    } else {
        OriginMatcher::Exact(pattern.to_string())
    }
}

/// Look up (and possibly populate) the cached origin matchers for a project.
/// Returns `None` if the project has no allowlist configured (any origin OK).
async fn project_origin_matchers(
    state: &AppState,
    project_id: i64,
) -> Option<Arc<Vec<OriginMatcher>>> {
    let now = Instant::now();
    {
        let mut cache = ORIGIN_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = cache.get(&project_id)
            && now.duration_since(entry.fetched_at) < ORIGIN_CACHE_TTL
        {
            return Some(Arc::clone(&entry.matchers));
        }
    }

    let settings: Option<(Option<String>,)> =
        sqlx::query_as("SELECT allowed_origins FROM project_settings WHERE project_id = ?")
            .bind(project_id)
            .fetch_optional(state.db.reader())
            .await
            .unwrap_or(None);

    let matchers: Vec<OriginMatcher> = settings
        .and_then(|(json,)| json)
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|p| compile_origin_pattern(&p))
        .collect();

    let arc = Arc::new(matchers);
    let mut cache = ORIGIN_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.put(
        project_id,
        CachedOrigins {
            fetched_at: now,
            matchers: Arc::clone(&arc),
        },
    );
    Some(arc)
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/{project_id}/envelope/", post(ingest_envelope))
        .route("/api/{project_id}/store/", post(ingest_store))
        .route("/api/{project_id}/security/", post(ingest_security))
        .layer(tower_http::cors::CorsLayer::permissive())
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

    // Validate auth -> resolve project key. Use a single opaque error for
    // both "key not found" and "key found but project missing" so an
    // attacker can't distinguish the two cases via response wording.
    let (key, _project) = resolve_project_key(state.db.reader(), &auth.sentry_key)
        .await
        .map_err(|_| (StatusCode::UNAUTHORIZED, "unauthorized".into()))?;

    // Same opaque response if the URL project doesn't match the key's project.
    if project_id != key.project_id {
        return Err((StatusCode::UNAUTHORIZED, "unauthorized".into()));
    }

    // Check rate limit
    if let Some(limit) = key.rate_limit
        && limit > 0
        && !state
            .rate_limiter
            .check(&auth.sentry_key, limit as u64)
            .await
    {
        return Err((StatusCode::TOO_MANY_REQUESTS, "Rate limit exceeded".into()));
    }

    // Check origin allowlist (matchers are compiled once per project)
    if let Some(origin) = headers.get("origin").and_then(|v| v.to_str().ok())
        && let Some(matchers) = project_origin_matchers(&state, key.project_id).await
        && !matchers.is_empty()
        && !matchers.iter().any(|m| m.matches(origin))
    {
        return Err((StatusCode::FORBIDDEN, "Origin not allowed".into()));
    }

    // Decompress if gzipped (with a hard cap to defeat decompression bombs)
    let max_envelope = state.config.ingest.max_envelope_bytes;
    let data = if body.len() >= 2 && body[0] == 0x1f && body[1] == 0x8b {
        decompress_gzip_capped(&body, max_envelope).map_err(|e| match e {
            DecompressError::SizeExceeded => (
                StatusCode::PAYLOAD_TOO_LARGE,
                "Decompressed envelope too large".into(),
            ),
            DecompressError::Io(io) => (
                StatusCode::BAD_REQUEST,
                format!("Decompression failed: {io}"),
            ),
        })?
    } else {
        if body.len() > max_envelope {
            return Err((StatusCode::PAYLOAD_TOO_LARGE, "Envelope too large".into()));
        }
        body.to_vec()
    };

    // Extract event_id from first line (fast path — no full envelope parse)
    let event_id = extract_event_id(&data).unwrap_or_else(generate_event_id);

    // Store raw envelope (use key.project_id, not URL param, as source of truth)
    let insert_result = sqlx::query(
        "INSERT OR IGNORE INTO event_envelopes (project_id, event_id, body, state) VALUES (?, ?, ?, 'pending')"
    )
    .bind(key.project_id)
    .bind(&event_id)
    .bind(&data)
    .execute(state.db.writer())
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Storage failed: {e}")))?;

    // Only queue for processing if we actually inserted a new envelope.
    // Duplicate submissions (rows_affected == 0) are ignored — the existing
    // envelope is already queued or being processed, and the poller handles recovery.
    if insert_result.rows_affected() > 0 {
        let envelope_id: Option<(i64,)> =
            sqlx::query_as("SELECT id FROM event_envelopes WHERE project_id = ? AND event_id = ?")
                .bind(key.project_id)
                .bind(&event_id)
                .fetch_optional(state.db.reader())
                .await
                .unwrap_or(None);

        if let Some((id,)) = envelope_id {
            let _ = state.worker_tx.try_send(id);
        }
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
    ingest_envelope(
        State(state),
        Path(project_id),
        headers,
        auth,
        envelope_bytes,
    )
    .await
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
    ingest_envelope(
        State(state),
        Path(project_id),
        headers,
        auth,
        envelope_bytes,
    )
    .await
}

/// Decompress a gzip stream, refusing to allocate beyond `max` bytes.
/// Error from [`decompress_gzip_capped`]. Structured so callers can
/// distinguish "bomb detected" from "malformed gzip" without sniffing
/// error message text.
#[derive(Debug)]
pub(crate) enum DecompressError {
    /// Decompressed output would exceed the configured maximum — probable
    /// decompression bomb.
    SizeExceeded,
    /// Underlying gzip read failed (bad framing, truncated, etc.).
    Io(std::io::Error),
}

impl std::fmt::Display for DecompressError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecompressError::SizeExceeded => {
                f.write_str("decompressed payload exceeds maximum size")
            }
            DecompressError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for DecompressError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DecompressError::Io(e) => Some(e),
            DecompressError::SizeExceeded => None,
        }
    }
}

/// Reads at most `max + 1` bytes from the decompressor; if that many are
/// produced, returns an error so a gzip bomb cannot exhaust memory before
/// the caller's size check fires.
pub(crate) fn decompress_gzip_capped(data: &[u8], max: usize) -> Result<Vec<u8>, DecompressError> {
    use flate2::read::GzDecoder;
    use std::io::Read;
    let mut decoder = GzDecoder::new(data).take((max as u64).saturating_add(1));
    let mut decompressed = Vec::new();
    decoder
        .read_to_end(&mut decompressed)
        .map_err(DecompressError::Io)?;
    if decompressed.len() > max {
        return Err(DecompressError::SizeExceeded);
    }
    Ok(decompressed)
}
