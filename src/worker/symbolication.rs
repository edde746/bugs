use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

use lru::LruCache;
use once_cell::sync::Lazy;
use sourcemap::SourceMap;
use tracing::{debug, warn};
use url::Url;

use crate::config::SymbolicationConfig;
use crate::db::DbPool;
use crate::models::release::ReleaseFile;
use crate::sentry_protocol::types::SentryEvent;
use crate::util::byte_capped_lru::ByteCappedLru;

// Initial capacities — intentionally small. `configure_caches` runs once
// at startup to resize these to the user's configured values before any
// envelopes are processed. The SM cache is byte-capped because parsed
// SourceMaps can be several MB each — a pure entry-count cap let a
// handful of large bundles pin 180+ MB of heap (see repro in phase 2).
static SM_CACHE: Lazy<Mutex<ByteCappedLru<String, Arc<SourceMap>>>> = Lazy::new(|| {
    Mutex::new(ByteCappedLru::new(
        NonZeroUsize::new(64).unwrap(),
        32 * 1024 * 1024,
    ))
});

/// Cache release file lookups to avoid repeated DB queries for the same release version.
static RELEASE_FILES_CACHE: Lazy<Mutex<LruCache<String, Arc<Vec<ReleaseFile>>>>> =
    Lazy::new(|| Mutex::new(LruCache::new(NonZeroUsize::new(32).unwrap())));

/// Resize the symbolication caches to match the user's configuration.
/// Call this once during startup before spawning workers. A zero or
/// unreasonable configured size is clamped to a sensible floor rather
/// than panicking.
pub fn configure_caches(cfg: &SymbolicationConfig) {
    let sm_cap = NonZeroUsize::new(cfg.source_map_cache_size.max(1)).expect("max(1) is non-zero");
    let sm_bytes = cfg.source_map_cache_bytes_mb.max(1) * 1024 * 1024;
    let files_cap =
        NonZeroUsize::new(cfg.release_files_cache_size.max(1)).expect("max(1) is non-zero");
    SM_CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .resize(sm_cap, sm_bytes);
    RELEASE_FILES_CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .resize(files_cap);
}

/// Drop the cached release-file list for `version` so the next
/// symbolication attempt re-queries the DB. Must be called after every
/// `release_files` upsert — otherwise an empty-cache entry from a
/// symbolication attempt that ran before any files were uploaded pins
/// "no files" for the life of the process.
pub fn invalidate_release_files(version: &str) {
    RELEASE_FILES_CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .pop(&version.to_string());
}

/// Drop the parsed `SourceMap` cached at `file_path`. Uploading a new
/// artifact to the same release+name overwrites the on-disk file but
/// leaves the old parsed form in memory; without this the worker keeps
/// symbolicating against stale source maps until LRU eviction.
pub fn invalidate_source_map_path(file_path: &str) {
    SM_CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .pop(&file_path.to_string());
}

/// Result of attempting to symbolicate an event. Used by the worker to
/// record why source maps were or weren't applied, so events missing a
/// map can be reprocessed later without re-parsing the whole envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolicationOutcome {
    /// Symbolication wasn't attempted (no release or no applicable frames).
    NotAttempted,
    /// Symbolication ran.
    Ok,
    /// Release is known but no `.map` file registered for it. Eligible
    /// for retry when a release file is later uploaded.
    MissingMap,
}

pub async fn symbolicate_event(
    event: &mut SentryEvent,
    db: &DbPool,
    _artifacts_dir: &str,
) -> Result<SymbolicationOutcome, Box<dyn std::error::Error + Send + Sync>> {
    // 1. Check if event has a release field
    let release_version = match &event.release {
        Some(v) if !v.is_empty() => v.clone(),
        _ => return Ok(SymbolicationOutcome::NotAttempted),
    };

    // 2. Load release files (cached by release version to avoid repeated DB lookups)
    let release_files = {
        let mut cache = RELEASE_FILES_CACHE
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        cache.get(&release_version).cloned()
    };

    let release_files = match release_files {
        Some(cached) => cached,
        None => {
            let release_row: Option<(i64,)> =
                sqlx::query_as("SELECT id FROM releases WHERE org_id = 1 AND version = ?")
                    .bind(&release_version)
                    .fetch_optional(db.reader())
                    .await?;

            let release_id = match release_row {
                // Release not registered at all — SDK sent a `release` tag but
                // nobody has created the release record. Nothing to retry
                // from our side, so treat it as "no attempt" rather than
                // "missing map".
                Some((id,)) => id,
                None => return Ok(SymbolicationOutcome::NotAttempted),
            };

            let files: Vec<ReleaseFile> =
                sqlx::query_as("SELECT * FROM release_files WHERE release_id = ?")
                    .bind(release_id)
                    .fetch_all(db.reader())
                    .await?;

            let files = Arc::new(files);
            {
                let mut cache = RELEASE_FILES_CACHE
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                cache.put(release_version.clone(), Arc::clone(&files));
            }
            files
        }
    };

    // Release exists but no files uploaded yet — flag so a later upload
    // can requeue these events.
    if release_files.is_empty() {
        return Ok(SymbolicationOutcome::MissingMap);
    }

    // 4. For each exception value's stacktrace frames, symbolicate
    let exception = match &mut event.exception {
        Some(exc) => exc,
        None => return Ok(SymbolicationOutcome::NotAttempted),
    };

    // Track whether we had frames that looked symbolicatable but couldn't
    // find a matching .map — that's the "map uploaded for some bundles but
    // not this one" case, still worth flagging for retry.
    let mut had_map_miss = false;
    let mut any_frame_mapped = false;

    for exc_value in &mut exception.values {
        let stacktrace = match &mut exc_value.stacktrace {
            Some(st) => st,
            None => continue,
        };

        for frame in &mut stacktrace.frames {
            let abs_path = match &frame.abs_path {
                Some(p) if !p.is_empty() => p.clone(),
                _ => continue,
            };

            // Convert URL to artifact name: strip scheme+host, prefix with ~
            let artifact_name = match Url::parse(&abs_path) {
                Ok(parsed) => format!("~{}", parsed.path()),
                Err(_) => continue,
            };

            // Look for {artifact_name}.map in release_files
            let map_name = format!("{}.map", artifact_name);
            let release_file = match release_files.iter().find(|rf| rf.name == map_name) {
                Some(rf) => rf,
                None => {
                    had_map_miss = true;
                    continue;
                }
            };
            any_frame_mapped = true;

            // Load and parse the source map
            let sm = match load_source_map(&release_file.file_path).await {
                Ok(sm) => sm,
                Err(e) => {
                    warn!(
                        path = %release_file.file_path,
                        "Failed to load source map: {e}"
                    );
                    continue;
                }
            };

            // Use sm.lookup_token(line - 1, col) to find original position
            let line = match frame.lineno {
                Some(l) if l > 0 => l,
                _ => continue,
            };
            let col = frame.colno.unwrap_or(0);

            let token = match sm.lookup_token(line - 1, col) {
                Some(t) => t,
                None => {
                    debug!(
                        abs_path = %abs_path,
                        line,
                        col,
                        "No source map token found"
                    );
                    continue;
                }
            };

            // Update frame with original position
            if let Some(src) = token.get_source() {
                frame.filename = Some(src.to_string());
            }
            frame.lineno = Some(token.get_src_line() + 1); // convert 0-based to 1-based
            frame.colno = Some(token.get_src_col());
            if let Some(name) = token.get_name() {
                frame.function = Some(name.to_string());
            }

            // Extract source context
            {
                let source_id = token.get_src_id();
                if let Some(source_contents) = sm.get_source_contents(source_id) {
                    let lines: Vec<&str> = source_contents.lines().collect();
                    let src_line = token.get_src_line() as usize;

                    // context_line
                    if src_line < lines.len() {
                        frame.context_line = Some(lines[src_line].to_string());
                    }

                    // pre_context: up to 5 lines before
                    let pre_start = src_line.saturating_sub(5);
                    let pre: Vec<String> = lines[pre_start..src_line]
                        .iter()
                        .map(|s| s.to_string())
                        .collect();
                    if !pre.is_empty() {
                        frame.pre_context = Some(pre);
                    }

                    // post_context: up to 5 lines after
                    let post_start = src_line + 1;
                    let post_end = (post_start + 5).min(lines.len());
                    if post_start < lines.len() {
                        let post: Vec<String> = lines[post_start..post_end]
                            .iter()
                            .map(|s| s.to_string())
                            .collect();
                        if !post.is_empty() {
                            frame.post_context = Some(post);
                        }
                    }
                }
            }

            // Set in_app based on filename heuristic
            if let Some(ref filename) = frame.filename {
                let is_in_app =
                    !filename.contains("node_modules") && !filename.contains("webpack/");
                frame.in_app = Some(is_in_app);
            }
        }
    }

    // If we saw frames that wanted a map but none mapped, that's a miss.
    // If we mapped at least one frame, treat the event as "ok" even if
    // other frames lacked a matching bundle — retry won't help those.
    Ok(if !any_frame_mapped && had_map_miss {
        SymbolicationOutcome::MissingMap
    } else {
        SymbolicationOutcome::Ok
    })
}

async fn load_source_map(
    file_path: &str,
) -> Result<Arc<SourceMap>, Box<dyn std::error::Error + Send + Sync>> {
    // Check cache first
    {
        let mut cache = SM_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(sm) = cache.get(&file_path.to_string()) {
            return Ok(Arc::clone(sm));
        }
    }

    // Read from disk
    let data = tokio::fs::read(file_path).await?;
    // Charge the cache by input bytes — a reasonable proxy for the
    // parsed SourceMap's heap footprint (sources + sourcesContent
    // dominate, and both are retained in the parsed form).
    let bytes = data.len();

    // Parse source map (blocking operation, use spawn_blocking)
    let sm = tokio::task::spawn_blocking(move || SourceMap::from_reader(&data[..])).await??;

    let sm = Arc::new(sm);

    // Store in cache
    {
        let mut cache = SM_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        cache.put(file_path.to_string(), Arc::clone(&sm), bytes);
    }

    Ok(sm)
}
