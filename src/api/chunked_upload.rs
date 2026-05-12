//! Chunked debug-file upload protocol for sentry-cli (3.x+).
//!
//! Three endpoints implement Sentry's chunked DIF upload:
//!
//! - `GET  /api/0/organizations/{org}/chunk-upload/` — capabilities discovery
//! - `POST /api/0/organizations/{org}/chunk-upload/` — chunk receive (multipart, SHA1-indexed)
//! - `POST /api/0/projects/{org}/{project}/files/difs/assemble/` — per-DIF assembly + processing
//!
//! Chunks are staged on disk under `<artifacts_dir>/chunks/<sha1[..2]>/<sha1>`
//! (git-objects-style). On assemble, referenced chunks are concatenated
//! into a temp file, parsed via `symbolic::debuginfo::Archive`, and
//! processed the same way as the legacy `/files/dsyms/` endpoint — except
//! that `ObjectKind::Sources` (the `--include-sources` artifact) is
//! stored as-is at `<artifacts_dir>/sources/<sha1[..2]>/<debug_id>.zip`
//! instead of dropped into an empty SymCache.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use axum::{
    Json, Router,
    extract::{Multipart, Path as AxumPath, State},
    http::StatusCode,
    routing::{get, post},
};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha1::{Digest, Sha1};
use symbolic::common::ByteView;
use symbolic::debuginfo::{Archive, Object, ObjectKind};
use symbolic::symcache::SymCacheConverter;
use tempfile::NamedTempFile;
use tracing::warn;

use crate::AppState;
use crate::util::atomic_fs::{copy_atomic, write_atomic};
use crate::util::id::{hyphenate_debug_id, is_sha1_hex, normalize_debug_id};
use crate::worker::native_symbolication;

fn chunk_path(root: &str, hash: &str) -> PathBuf {
    PathBuf::from(format!("{root}/{}/{hash}", &hash[..2]))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/0/organizations/{org}/chunk-upload/",
            get(chunk_options).post(upload_chunks),
        )
        .route(
            "/api/0/projects/{org}/{project}/files/difs/assemble/",
            post(assemble_difs),
        )
}

// =====================================================================
// GET chunk-upload — capabilities discovery
// =====================================================================

async fn chunk_options(
    State(state): State<AppState>,
    AxumPath(org): AxumPath<String>,
) -> Json<Value> {
    let cfg = &state.config.uploads;
    Json(json!({
        "url": format!("/api/0/organizations/{org}/chunk-upload/"),
        "chunkSize": cfg.chunk_size_mib * 1024 * 1024,
        "chunksPerRequest": cfg.chunks_per_request,
        "maxFileSize": cfg.max_bytes,
        "maxRequestSize": cfg.max_request_size_mib * 1024 * 1024,
        "maxWait": cfg.max_wait_secs,
        "concurrency": cfg.chunk_concurrency,
        "hashAlgorithm": "sha1",
        "compression": ["gzip"],
        "accept": ["debug_files", "sources"],
    }))
}

// =====================================================================
// POST chunk-upload — receive chunks (multipart, SHA1-indexed)
// =====================================================================

async fn upload_chunks(
    State(state): State<AppState>,
    AxumPath(_org): AxumPath<String>,
    mut multipart: Multipart,
) -> Result<StatusCode, (StatusCode, String)> {
    let chunk_size = state.config.uploads.chunk_size_mib * 1024 * 1024;
    let max_request_size = state.config.uploads.max_request_size_mib * 1024 * 1024;
    let chunks_root = format!("{}/chunks", state.config.artifacts_dir);

    let mut request_bytes: usize = 0;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("multipart: {e}")))?
    {
        let part_name = field.name().unwrap_or("").to_string();
        let is_gzip = match part_name.as_str() {
            "file" => false,
            "file_gzip" => true,
            _ => continue,
        };
        let expected_hash = field.file_name().map(|s| s.to_string()).ok_or((
            StatusCode::BAD_REQUEST,
            "chunk part missing filename (sha1)".to_string(),
        ))?;
        if !is_sha1_hex(&expected_hash) {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("invalid chunk filename: {expected_hash}"),
            ));
        }
        let expected_hash = expected_hash.to_ascii_lowercase();

        // Stream the part body so we abort on oversize without
        // buffering the full overflow. chunk_size + 1 MiB gives gzip
        // some headroom on tiny inputs.
        let raw_cap = chunk_size + 1024 * 1024;
        let mut raw: Vec<u8> = Vec::new();
        let mut field = field;
        while let Some(chunk) = field
            .chunk()
            .await
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("read chunk: {e}")))?
        {
            if raw.len() + chunk.len() > raw_cap {
                return Err((
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "chunk part exceeds chunk_size + 1 MiB".to_string(),
                ));
            }
            raw.extend_from_slice(&chunk);
        }

        let decoded: Vec<u8> = if is_gzip {
            // take(chunk_size + 1) lets us detect overflow without buffering it.
            let mut dec = GzDecoder::new(&raw[..]).take((chunk_size as u64) + 1);
            let mut buf = Vec::new();
            dec.read_to_end(&mut buf)
                .map_err(|e| (StatusCode::BAD_REQUEST, format!("gzip decode: {e}")))?;
            buf
        } else {
            raw
        };
        if decoded.len() > chunk_size {
            return Err((
                StatusCode::PAYLOAD_TOO_LARGE,
                "decoded chunk exceeds chunk_size".to_string(),
            ));
        }

        request_bytes = request_bytes.checked_add(decoded.len()).ok_or((
            StatusCode::PAYLOAD_TOO_LARGE,
            "request size overflow".to_string(),
        ))?;
        if request_bytes > max_request_size {
            return Err((
                StatusCode::PAYLOAD_TOO_LARGE,
                "request exceeds max_request_size".to_string(),
            ));
        }

        let mut hasher = Sha1::new();
        hasher.update(&decoded);
        let actual_hash = hex::encode(hasher.finalize());
        if actual_hash != expected_hash {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("chunk hash mismatch: claimed {expected_hash}, computed {actual_hash}"),
            ));
        }

        let target_path = chunk_path(&chunks_root, &actual_hash);
        let target = target_path.to_string_lossy().into_owned();
        let dir = target_path
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| chunks_root.clone());
        if !tokio::fs::try_exists(&target).await.unwrap_or(false) {
            write_atomic(&dir, &target, decoded).await.map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("write chunk: {e}"),
                )
            })?;
        }
    }

    Ok(StatusCode::OK)
}

// =====================================================================
// POST files/difs/assemble — concatenate chunks, parse, store
// =====================================================================

#[derive(Debug, Deserialize)]
struct AssembleEntry {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    debug_id: Option<String>,
    #[serde(default)]
    chunks: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AssembleResponse {
    state: AssembleState,
    missing_chunks: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dif: Option<AssembleDif>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum AssembleState {
    NotFound,
    Error,
    Ok,
}

/// `uuid` and `debug_id` both carry the same hyphenated debug-id —
/// sentry-cli's `DebugInfoFile` deserializer reads `id.or(uuid)` so
/// either is sufficient, but we populate both for compatibility across
/// client versions.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AssembleDif {
    uuid: String,
    debug_id: String,
    object_name: String,
    cpu_name: String,
    sha1: String,
    data: AssembleDifData,
}

#[derive(Debug, Serialize)]
struct AssembleDifData {
    features: Vec<&'static str>,
}

async fn assemble_difs(
    State(state): State<AppState>,
    AxumPath((_org, project_slug)): AxumPath<(String, String)>,
    Json(request): Json<std::collections::HashMap<String, AssembleEntry>>,
) -> Result<Json<std::collections::HashMap<String, AssembleResponse>>, (StatusCode, String)> {
    let project_id: i64 = {
        let row: Option<(i64,)> = sqlx::query_as("SELECT id FROM projects WHERE slug = ?")
            .bind(&project_slug)
            .fetch_optional(state.db.reader())
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        row.ok_or((StatusCode::NOT_FOUND, "project not found".to_string()))?
            .0
    };

    let chunks_root = format!("{}/chunks", state.config.artifacts_dir);
    let mut response = std::collections::HashMap::<String, AssembleResponse>::new();

    for (checksum, entry) in request {
        if !is_sha1_hex(&checksum) {
            response.insert(
                checksum,
                AssembleResponse {
                    state: AssembleState::Error,
                    missing_chunks: Vec::new(),
                    detail: Some("invalid checksum (expect SHA1 hex)".to_string()),
                    dif: None,
                },
            );
            continue;
        }
        let result = assemble_one(&state, project_id, &chunks_root, &checksum, &entry).await;
        let resp = match result {
            Ok(resp) => resp,
            Err(e) => AssembleResponse {
                state: AssembleState::Error,
                missing_chunks: Vec::new(),
                detail: Some(e),
                dif: None,
            },
        };
        response.insert(checksum, resp);
    }

    Ok(Json(response))
}

async fn assemble_one(
    state: &AppState,
    project_id: i64,
    chunks_root: &str,
    checksum: &str,
    entry: &AssembleEntry,
) -> Result<AssembleResponse, String> {
    // 1. Verify all chunks present on disk.
    let mut missing = Vec::new();
    let mut chunk_paths: Vec<PathBuf> = Vec::with_capacity(entry.chunks.len());
    for ch in &entry.chunks {
        let lower = ch.to_ascii_lowercase();
        if !is_sha1_hex(&lower) {
            return Err(format!("invalid chunk hash: {ch}"));
        }
        let path = chunk_path(chunks_root, &lower);
        if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
            missing.push(lower);
        } else {
            chunk_paths.push(path);
        }
    }
    if !missing.is_empty() {
        return Ok(AssembleResponse {
            state: AssembleState::NotFound,
            missing_chunks: missing,
            detail: None,
            dif: None,
        });
    }
    if chunk_paths.is_empty() {
        return Err("no chunks in request".to_string());
    }

    // 2. Stream-concatenate into a temp file, hashing as we go. Verify
    //    the final SHA1 matches the overall checksum.
    let checksum_owned = checksum.to_ascii_lowercase();
    let max_bytes = state.config.uploads.max_bytes;
    let assembled = tokio::task::spawn_blocking(move || -> Result<NamedTempFile, String> {
        let temp = tempfile::Builder::new()
            .prefix("bugs-assemble-")
            .tempfile()
            .map_err(|e| format!("tempfile: {e}"))?;
        let mut writer = temp.reopen().map_err(|e| format!("reopen temp: {e}"))?;
        let mut hasher = Sha1::new();
        let mut total: usize = 0;
        let mut buf = vec![0u8; 64 * 1024];
        for path in &chunk_paths {
            let mut f = std::fs::File::open(path).map_err(|e| format!("open chunk: {e}"))?;
            loop {
                let n = f.read(&mut buf).map_err(|e| format!("read chunk: {e}"))?;
                if n == 0 {
                    break;
                }
                total = total
                    .checked_add(n)
                    .ok_or_else(|| "assembled size overflow".to_string())?;
                if total > max_bytes {
                    return Err("assembled file exceeds uploads.max_bytes".to_string());
                }
                hasher.update(&buf[..n]);
                writer
                    .write_all(&buf[..n])
                    .map_err(|e| format!("write temp: {e}"))?;
            }
        }
        writer.flush().map_err(|e| format!("flush temp: {e}"))?;
        drop(writer);
        let actual = hex::encode(hasher.finalize());
        if actual != checksum_owned {
            return Err(format!(
                "checksum mismatch: claimed {checksum_owned}, computed {actual}"
            ));
        }
        Ok(temp)
    })
    .await
    .map_err(|e| format!("join: {e}"))??;

    // 3. Parse the assembled file as a DIF archive. CPU-bound, blocking.
    let assembled_path = assembled.path().to_path_buf();
    let request_name = entry.name.clone().unwrap_or_else(|| "upload".to_string());
    let processed = tokio::task::spawn_blocking(move || -> Result<Vec<ProcessedObject>, String> {
        process_assembled(&assembled_path, &request_name)
    })
    .await
    .map_err(|e| format!("join: {e}"))??;

    if processed.is_empty() {
        return Err("no parseable debug-info objects in upload".to_string());
    }

    // 4. Pick the primary object — prefer one whose debug_id matches the
    //    request's hint, else the first.
    let primary_idx = entry
        .debug_id
        .as_ref()
        .and_then(|hint| {
            let want = normalize_debug_id(hint);
            if want.is_empty() {
                None
            } else {
                processed.iter().position(|p| p.debug_id == want)
            }
        })
        .unwrap_or(0);

    // 5. Write each object's artifact to disk and insert DB row.
    for obj in &processed {
        let shard = &obj.debug_id[..2];
        let (dir_path, target, kind) = match obj.payload {
            ProcessedPayload::SymCache(_) => {
                let dir = format!(
                    "{}/native/{}/{}",
                    state.config.artifacts_dir, shard, obj.debug_id
                );
                let target = format!("{dir}/{}.symc", obj.arch);
                (dir, target, "native")
            }
            ProcessedPayload::SourceBundle => {
                let dir = format!("{}/sources/{}", state.config.artifacts_dir, shard);
                let target = format!("{dir}/{}.zip", obj.debug_id);
                (dir, target, "source_bundle")
            }
        };

        match &obj.payload {
            ProcessedPayload::SymCache(bytes) => {
                if let Err(e) = write_atomic(&dir_path, &target, bytes.clone()).await {
                    return Err(format!("write {target}: {e}"));
                }
                native_symbolication::invalidate_symcache_path(&target);
            }
            ProcessedPayload::SourceBundle => {
                if let Err(e) = copy_atomic(assembled.path(), &dir_path, &target).await {
                    return Err(format!("copy {target}: {e}"));
                }
            }
        }

        let result = sqlx::query(
            "INSERT INTO artifact_debug_ids \
                 (debug_id, project_id, release_id, file_path, source_name, arch, code_id, kind) \
             VALUES (?, ?, NULL, ?, ?, ?, ?, ?) \
             ON CONFLICT(debug_id, kind) DO UPDATE SET \
                 project_id = excluded.project_id, \
                 release_id = excluded.release_id, \
                 file_path = excluded.file_path, \
                 source_name = excluded.source_name, \
                 arch = excluded.arch, \
                 code_id = excluded.code_id",
        )
        .bind(&obj.debug_id)
        .bind(project_id)
        .bind(&target)
        .bind(&obj.source_name)
        .bind(&obj.arch)
        .bind(&obj.code_id)
        .bind(kind)
        .execute(state.db.writer())
        .await;
        if let Err(e) = result {
            warn!("artifact_debug_ids insert failed: {e}");
            return Err(format!("db: {e}"));
        }
    }

    let primary = &processed[primary_idx];
    Ok(AssembleResponse {
        state: AssembleState::Ok,
        missing_chunks: Vec::new(),
        detail: None,
        dif: Some(AssembleDif {
            uuid: hyphenate_debug_id(&primary.debug_id),
            debug_id: hyphenate_debug_id(&primary.debug_id),
            object_name: primary.source_name.clone(),
            cpu_name: primary.arch.clone(),
            sha1: checksum.to_ascii_lowercase(),
            data: AssembleDifData {
                features: primary.features.clone(),
            },
        }),
    })
}

// =====================================================================
// Object processing helpers
// =====================================================================

struct ProcessedObject {
    debug_id: String,
    arch: String,
    code_id: Option<String>,
    source_name: String,
    payload: ProcessedPayload,
    features: Vec<&'static str>,
}

enum ProcessedPayload {
    SymCache(Vec<u8>),
    /// Bytes live on disk at the assembled file path; we'll copy on
    /// commit. Keeping them out of memory avoids 100s of MB allocs for
    /// large `--include-sources` bundles.
    SourceBundle,
}

fn process_assembled(path: &Path, source_name: &str) -> Result<Vec<ProcessedObject>, String> {
    let view = ByteView::open(path).map_err(|e| format!("open assembled: {e}"))?;
    let archive = Archive::parse(&view).map_err(|e| format!("archive parse: {e}"))?;
    let mut out = Vec::new();
    for object_result in archive.objects() {
        let object = match object_result {
            Ok(o) => o,
            Err(e) => {
                warn!("object parse: {e}");
                continue;
            }
        };
        if let Some(po) = object_to_processed(&object, source_name) {
            out.push(po);
        }
    }
    Ok(out)
}

fn object_to_processed(object: &Object<'_>, source_name: &str) -> Option<ProcessedObject> {
    let debug_id = normalize_debug_id(&object.debug_id().to_string());
    if debug_id.is_empty() {
        return None;
    }
    let arch = object.arch().name().to_string();
    let code_id = object.code_id().map(|c| c.to_string());
    let features = collect_features(object);

    if object.kind() == ObjectKind::Sources {
        return Some(ProcessedObject {
            debug_id,
            arch,
            code_id,
            source_name: source_name.to_string(),
            payload: ProcessedPayload::SourceBundle,
            features,
        });
    }

    let mut converter = SymCacheConverter::new();
    if let Err(e) = converter.process_object(object) {
        warn!("symcache process {source_name} ({arch}): {e}");
        return None;
    }
    let mut bytes: Vec<u8> = Vec::new();
    if let Err(e) = converter.serialize(&mut bytes) {
        warn!("symcache serialize {source_name} ({arch}): {e}");
        return None;
    }
    Some(ProcessedObject {
        debug_id,
        arch,
        code_id,
        source_name: source_name.to_string(),
        payload: ProcessedPayload::SymCache(bytes),
        features,
    })
}

fn collect_features(object: &Object<'_>) -> Vec<&'static str> {
    let mut f = Vec::new();
    if object.has_symbols() {
        f.push("symtab");
    }
    if object.has_debug_info() {
        f.push("debug");
    }
    if object.has_unwind_info() {
        f.push("unwind");
    }
    if object.has_sources() {
        f.push("sources");
    }
    f
}
