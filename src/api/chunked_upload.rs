//! Chunked debug-file upload protocol for sentry-cli (3.x+).
//!
//! Three endpoints implement Sentry's chunked DIF upload:
//!
//! - `GET  /api/0/organizations/{org}/chunk-upload/` — capabilities discovery
//! - `POST /api/0/organizations/{org}/chunk-upload/` — chunk receive (multipart, SHA1-indexed)
//! - `POST /api/0/projects/{org}/{project}/files/difs/assemble/` — per-DIF assembly + processing
//!
//! Chunks are content-addressed on disk. Assemble requests inspect durable
//! receipts and enqueue missing work; a bounded background worker hashes,
//! converts and publishes artifacts independently of the HTTP connection.

use std::collections::HashMap;
use std::io::{self, Read, Write};
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
use symbolic::common::{ByteView, DebugId};
use symbolic::debuginfo::{Archive, Object, ObjectKind};
use symbolic::symcache::SymCacheConverter;
use tempfile::NamedTempFile;
use tracing::warn;

use crate::AppState;
use crate::config::{Config, UploadsConfig};
use crate::db::DbPool;
use crate::ingest::cfi;
use crate::util::artifact_store::{ArtifactMetadata, ArtifactPayload, store_artifact};
use crate::util::atomic_fs::write_atomic;
use crate::util::chunk_store::{chunk_path, touch_chunk};
use crate::util::id::{hyphenate_debug_id, is_sha1_hex, normalize_debug_id};
use crate::worker::{dart_symbol_map, dif_assembly};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/0/organizations/{org}/chunk-upload",
            get(chunk_options).post(upload_chunks),
        )
        .route(
            "/api/0/projects/{org}/{project}/files/difs/assemble",
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
    Json(chunk_upload_options(&state.config.uploads, &org))
}

/// sentry-cli silently skips every PE and PDB file (no warning, "Found 0
/// debug information files") unless `pdbs` is accepted; that capability also
/// enables the PDB age overrides honored by `effective_debug_id`.
fn chunk_upload_options(cfg: &UploadsConfig, org: &str) -> Value {
    json!({
        "url": format!("/api/0/organizations/{org}/chunk-upload/"),
        "chunkSize": cfg.chunk_size_mib * 1024 * 1024,
        "chunksPerRequest": cfg.chunks_per_request,
        "maxFileSize": cfg.max_bytes,
        "maxRequestSize": cfg.max_request_size_mib * 1024 * 1024,
        "maxWait": cfg.max_wait_secs,
        "concurrency": cfg.chunk_concurrency,
        "hashAlgorithm": "sha1",
        "compression": ["gzip"],
        "accept": ["debug_files", "pdbs", "sources", "dartsymbolmap"],
    })
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

        let target_path = chunk_path(Path::new(&chunks_root), &actual_hash);
        let target = target_path.to_string_lossy().into_owned();
        let dir = target_path
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| chunks_root.clone());
        let mut needs_write = !tokio::fs::try_exists(&target).await.unwrap_or(false);
        if !needs_write {
            match touch_chunk(&target_path).await {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => needs_write = true,
                Err(e) => warn!(path = %target, "failed to refresh existing chunk mtime: {e}"),
            }
        }
        if needs_write {
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct AssembleEntry {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub debug_id: Option<String>,
    #[serde(default)]
    pub chunks: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AssembleResponse {
    pub state: AssembleState,
    pub missing_chunks: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dif: Option<AssembleDif>,
}

impl AssembleResponse {
    pub fn error(detail: impl Into<String>) -> Self {
        Self {
            state: AssembleState::Error,
            missing_chunks: Vec::new(),
            detail: Some(detail.into()),
            dif: None,
        }
    }

    pub fn assembling() -> Self {
        Self {
            state: AssembleState::Assembling,
            missing_chunks: Vec::new(),
            detail: None,
            dif: None,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AssembleState {
    NotFound,
    Assembling,
    Error,
    Ok,
}

/// Both ID fields are populated for compatibility with sentry-cli versions.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AssembleDif {
    uuid: String,
    debug_id: String,
    object_name: String,
    cpu_name: String,
    sha1: String,
    data: AssembleDifData,
}

#[derive(Debug, Deserialize, Serialize)]
struct AssembleDifData {
    features: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct AssemblyArtifact {
    pub debug_id: String,
    pub arch: String,
    pub kind: String,
}

/// Version of the processing behind an assembly result. Bump it whenever
/// processing derives a new artifact from an upload: completed receipts
/// from older processing are then reprocessed instead of reused, because
/// sentry-cli never re-sends a file whose assemble state is `ok`.
/// 1: CFI caches stored next to SymCaches.
pub(crate) const PROCESSING_VERSION: u32 = 1;

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct AssemblyResult {
    pub response: AssembleResponse,
    pub artifacts: Vec<AssemblyArtifact>,
    /// Receipts written before versioning read as 0.
    #[serde(default)]
    pub processing_version: u32,
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

    let mut response = HashMap::<String, AssembleResponse>::new();

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
        let resp = dif_assembly::request(&state.db, &state.config, project_id, &checksum, &entry)
            .await
            .unwrap_or_else(AssembleResponse::error);
        response.insert(checksum, resp);
    }

    Ok(Json(response))
}

pub(crate) async fn assemble_one(
    db: &DbPool,
    config: &Config,
    project_id: i64,
    chunks_root: &str,
    checksum: &str,
    entry: &AssembleEntry,
) -> Result<AssemblyResult, String> {
    // These are job-owned hard links, not expiring upload staging chunks.
    let mut missing = Vec::new();
    let mut chunk_paths: Vec<PathBuf> = Vec::with_capacity(entry.chunks.len());
    for ch in &entry.chunks {
        let lower = ch.to_ascii_lowercase();
        if !is_sha1_hex(&lower) {
            return Err(format!("invalid chunk hash: {ch}"));
        }
        let path = chunk_path(Path::new(chunks_root), &lower);
        if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
            missing.push(lower);
        } else {
            chunk_paths.push(path);
        }
    }
    if !missing.is_empty() {
        return Err(format!(
            "assembly staging chunks missing: {}",
            missing.join(", ")
        ));
    }
    if chunk_paths.is_empty() {
        return Err("no chunks in request".to_string());
    }

    // 2. Stream-concatenate into a temp file, hashing as we go. Verify
    //    the final SHA1 matches the overall checksum.
    let checksum_owned = checksum.to_ascii_lowercase();
    let max_bytes = config.uploads.max_bytes;
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

    // 2b. Dart symbol maps are JSON arrays, not object files — like
    //     upstream Sentry (models/debugfile.py, determine_dif_kind) we
    //     detect them by the leading '[' byte. They are stored verbatim,
    //     keyed by the debug_id sentry-cli sends with the assemble
    //     request; the map bytes themselves carry no identifier.
    if leading_byte(assembled.path()).await == Some(b'[') {
        return assemble_dart_symbol_map(db, config, project_id, checksum, entry, assembled.path())
            .await;
    }

    // 3. Parse the assembled file as a DIF archive. CPU-bound, blocking.
    let assembled_path = assembled.path().to_path_buf();
    let request_name = entry.name.clone().unwrap_or_else(|| "upload".to_string());
    let requested_debug_id = entry
        .debug_id
        .as_deref()
        .and_then(|id| id.parse::<DebugId>().ok());
    let processed = tokio::task::spawn_blocking(move || -> Result<Vec<ProcessedObject>, String> {
        process_assembled(&assembled_path, &request_name, requested_debug_id)
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

    // Build the response before moving the processed payloads into storage.
    let primary = &processed[primary_idx];
    let response = AssembleResponse {
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
                features: primary
                    .features
                    .iter()
                    .map(|value| (*value).to_string())
                    .collect(),
            },
        }),
    };
    let mut artifacts = Vec::with_capacity(processed.len());
    for obj in processed {
        let metadata = |kind| ArtifactMetadata {
            project_id,
            release_id: None,
            debug_id: &obj.debug_id,
            arch: &obj.arch,
            code_id: obj.code_id.as_deref(),
            source_name: &obj.source_name,
            kind,
        };
        let (kind, payload) = match obj.payload {
            ProcessedPayload::SymCache(bytes) => ("native", ArtifactPayload::Bytes(bytes)),
            ProcessedPayload::SourceBundle => {
                ("source_bundle", ArtifactPayload::File(assembled.path()))
            }
        };
        store_artifact(db, &config.artifacts_dir, metadata(kind), payload).await?;
        let cfi_stored = match obj.cfi {
            Some(cache) => cfi::store(db, &config.artifacts_dir, metadata(kind), cache).await,
            None => false,
        };
        if cfi_stored {
            artifacts.push(AssemblyArtifact {
                debug_id: obj.debug_id.clone(),
                arch: obj.arch.clone(),
                kind: cfi::KIND.to_string(),
            });
        }
        artifacts.push(AssemblyArtifact {
            debug_id: obj.debug_id,
            arch: obj.arch,
            kind: kind.to_string(),
        });
    }
    Ok(AssemblyResult {
        response,
        artifacts,
        processing_version: PROCESSING_VERSION,
    })
}

// =====================================================================
// Dart symbol maps (dartsymbolmap)
// =====================================================================

async fn leading_byte(path: &Path) -> Option<u8> {
    use tokio::io::AsyncReadExt;
    let mut file = tokio::fs::File::open(path).await.ok()?;
    let mut byte = [0u8; 1];
    match file.read(&mut byte).await {
        Ok(1) => Some(byte[0]),
        _ => None,
    }
}

/// Stores a Dart obfuscation map uploaded via `sentry-cli
/// dart-symbol-map upload`. Validation mirrors upstream Sentry
/// (detect_dif_from_path, DifKind.DartSymbolMap): the payload must be a
/// JSON array with an even number of elements and the request must name
/// a debug_id. The map is stored verbatim and indexed under the
/// `dart_symbol_map` kind; `src/worker/dart_symbol_map.rs` applies it
/// to obfuscated Dart events at processing time.
async fn assemble_dart_symbol_map(
    db: &DbPool,
    config: &Config,
    project_id: i64,
    checksum: &str,
    entry: &AssembleEntry,
    assembled_path: &Path,
) -> Result<AssemblyResult, String> {
    let debug_id = entry
        .debug_id
        .as_deref()
        .map(normalize_debug_id)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| "Missing debug_id for dartsymbolmap".to_string())?;
    // Every real debug-id format (UUID, ELF build-id, PDB id + age)
    // normalizes to at least 32 hex chars; shorter values could never
    // match an event image and would corrupt the shard path below.
    if debug_id.len() < 32 {
        return Err(format!("invalid debug_id for dartsymbolmap: {debug_id}"));
    }

    let bytes = tokio::fs::read(assembled_path)
        .await
        .map_err(|e| format!("read assembled: {e}"))?;
    let parsed: Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("Invalid dartsymbolmap: {e}"))?;
    let entries = parsed
        .as_array()
        .ok_or_else(|| "dartsymbolmap must be a JSON array".to_string())?;
    if !entries.len().is_multiple_of(2) {
        return Err("dartsymbolmap array must have an even number of elements".to_string());
    }

    let object_name = entry
        .name
        .clone()
        .unwrap_or_else(|| "dartsymbolmap.json".to_string());
    store_artifact(
        db,
        &config.artifacts_dir,
        ArtifactMetadata {
            project_id,
            release_id: None,
            debug_id: &debug_id,
            arch: "any",
            code_id: None,
            source_name: &object_name,
            kind: dart_symbol_map::KIND,
        },
        ArtifactPayload::Bytes(bytes),
    )
    .await?;

    Ok(AssemblyResult {
        response: AssembleResponse {
            state: AssembleState::Ok,
            missing_chunks: Vec::new(),
            detail: None,
            dif: Some(AssembleDif {
                uuid: hyphenate_debug_id(&debug_id),
                debug_id: hyphenate_debug_id(&debug_id),
                object_name,
                cpu_name: "any".to_string(),
                sha1: checksum.to_ascii_lowercase(),
                data: AssembleDifData {
                    features: vec!["mapping".to_string()],
                },
            }),
        },
        artifacts: vec![AssemblyArtifact {
            debug_id,
            arch: "any".to_string(),
            kind: dart_symbol_map::KIND.to_string(),
        }],
        processing_version: PROCESSING_VERSION,
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
    /// Serialized CFI cache, for objects with unwind records.
    cfi: Option<Vec<u8>>,
}

enum ProcessedPayload {
    SymCache(Vec<u8>),
    /// Bytes live on disk at the assembled file path; we'll copy on
    /// commit. Keeping them out of memory avoids 100s of MB allocs for
    /// large `--include-sources` bundles.
    SourceBundle,
}

fn process_assembled(
    path: &Path,
    source_name: &str,
    requested_debug_id: Option<DebugId>,
) -> Result<Vec<ProcessedObject>, String> {
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
        if let Some(po) = object_to_processed(&object, source_name, requested_debug_id) {
            out.push(po);
        }
    }
    Ok(out)
}

/// sentry-cli rewrites a PDB's age to the age in the matching PE's CodeView
/// record, because that is the identifier events carry. Like Sentry
/// (`DifMeta.from_object`), only the age may be overridden: a request whose
/// GUID differs from the object's keeps the object's own identifier.
fn effective_debug_id(object: DebugId, requested: Option<DebugId>) -> DebugId {
    match requested {
        Some(requested) if requested.uuid() == object.uuid() => requested,
        _ => object,
    }
}

fn object_to_processed(
    object: &Object<'_>,
    source_name: &str,
    requested_debug_id: Option<DebugId>,
) -> Option<ProcessedObject> {
    let debug_id = effective_debug_id(object.debug_id(), requested_debug_id);
    let debug_id = normalize_debug_id(&debug_id.to_string());
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
            cfi: None,
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
        cfi: cfi::cache_bytes(object),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Plezy 2.23.0's Windows upload (sentry-cli 2.58.6) reported "Found 0
    /// debug information files" for every PE/PDB batch because `pdbs` was
    /// not accepted, so `flutter_windows.dll.pdb` never reached the tracker.
    #[test]
    fn chunk_upload_options_accept_pdbs() {
        let options = chunk_upload_options(&UploadsConfig::default(), "default");
        let accept: Vec<&str> = options["accept"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        for capability in ["debug_files", "pdbs", "sources", "dartsymbolmap"] {
            assert!(accept.contains(&capability), "accept: {accept:?}");
        }
    }

    #[test]
    fn assemble_debug_id_overrides_only_pdb_age() {
        // flutter_windows.dll from Plezy 2.23.0 Windows event #1581.
        let pe: DebugId = "fd848531-4654-4681-9325-1d5c015a7dad-3".parse().unwrap();
        let pdb: DebugId = "fd848531-4654-4681-9325-1d5c015a7dad-1".parse().unwrap();
        assert_eq!(effective_debug_id(pdb, Some(pe)), pe);
        assert_eq!(
            normalize_debug_id(&effective_debug_id(pdb, Some(pe)).to_string()),
            normalize_debug_id("fd848531-4654-4681-9325-1d5c015a7dad-3"),
        );
        assert_eq!(effective_debug_id(pdb, None), pdb);
        let unrelated: DebugId = "480b273d-5bc7-66ca-4c4c-44205044422e-1".parse().unwrap();
        assert_eq!(effective_debug_id(pdb, Some(unrelated)), pdb);
    }
}
