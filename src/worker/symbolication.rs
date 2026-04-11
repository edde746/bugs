use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

use lru::LruCache;
use once_cell::sync::Lazy;
use sourcemap::SourceMap;
use tracing::{debug, warn};
use url::Url;

use crate::db::DbPool;
use crate::models::release::ReleaseFile;
use crate::sentry_protocol::types::SentryEvent;

static SM_CACHE: Lazy<Mutex<LruCache<String, Arc<SourceMap>>>> =
    Lazy::new(|| Mutex::new(LruCache::new(NonZeroUsize::new(64).unwrap())));

/// Cache release file lookups to avoid repeated DB queries for the same release version.
static RELEASE_FILES_CACHE: Lazy<Mutex<LruCache<String, Arc<Vec<ReleaseFile>>>>> =
    Lazy::new(|| Mutex::new(LruCache::new(NonZeroUsize::new(32).unwrap())));

pub async fn symbolicate_event(
    event: &mut SentryEvent,
    db: &DbPool,
    _artifacts_dir: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // 1. Check if event has a release field
    let release_version = match &event.release {
        Some(v) if !v.is_empty() => v.clone(),
        _ => return Ok(()),
    };

    // 2. Load release files (cached by release version to avoid repeated DB lookups)
    let release_files = {
        let mut cache = RELEASE_FILES_CACHE.lock().unwrap();
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
                Some((id,)) => id,
                None => return Ok(()),
            };

            let files: Vec<ReleaseFile> =
                sqlx::query_as("SELECT * FROM release_files WHERE release_id = ?")
                    .bind(release_id)
                    .fetch_all(db.reader())
                    .await?;

            let files = Arc::new(files);
            {
                let mut cache = RELEASE_FILES_CACHE.lock().unwrap();
                cache.put(release_version.clone(), Arc::clone(&files));
            }
            files
        }
    };

    if release_files.is_empty() {
        return Ok(());
    }

    // 4. For each exception value's stacktrace frames, symbolicate
    let exception = match &mut event.exception {
        Some(exc) => exc,
        None => return Ok(()),
    };

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
                None => continue,
            };

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

    Ok(())
}

async fn load_source_map(
    file_path: &str,
) -> Result<Arc<SourceMap>, Box<dyn std::error::Error + Send + Sync>> {
    // Check cache first
    {
        let mut cache = SM_CACHE.lock().unwrap();
        if let Some(sm) = cache.get(file_path) {
            return Ok(Arc::clone(sm));
        }
    }

    // Read from disk
    let data = tokio::fs::read(file_path).await?;

    // Parse source map (blocking operation, use spawn_blocking)
    let sm = tokio::task::spawn_blocking(move || SourceMap::from_reader(&data[..])).await??;

    let sm = Arc::new(sm);

    // Store in cache
    {
        let mut cache = SM_CACHE.lock().unwrap();
        cache.put(file_path.to_string(), Arc::clone(&sm));
    }

    Ok(sm)
}
