//! Immutable artifact publication and project/architecture-scoped canonical selection.
//!
//! Selection and publication hold a SQLite write transaction, including across
//! processes. Rejected candidates are never written. Published files are never
//! replaced, so readers may safely retain mappings of earlier canonical files.

use std::io::Write;
use std::path::Path;

use symbolic::common::ByteView;
use symbolic::symcache::SymCache;
use tempfile::NamedTempFile;

use crate::db::DbPool;
use crate::util::hash::sha256_hex;

pub struct ArtifactMetadata<'a> {
    pub project_id: i64,
    pub release_id: Option<i64>,
    pub debug_id: &'a str,
    pub arch: &'a str,
    pub code_id: Option<&'a str>,
    pub source_name: &'a str,
    pub kind: &'a str,
}

pub enum ArtifactPayload<'a> {
    Bytes(Vec<u8>),
    File(&'a Path),
}

#[derive(Clone, Copy, Default, Eq, Ord, PartialEq, PartialOrd)]
struct Quality {
    lines: i64,
    files: i64,
    functions: i64,
}

// Use supported SymCache APIs, not filenames, byte size, or private binary
// layouts. Actual line-bearing function lookups dominate file coverage, which
// dominates named-function coverage. Equal measured quality uses a digest tie
// break; this does not claim to totally order differing DWARF line tables.
fn native_quality(bytes: &[u8]) -> Result<Quality, String> {
    let cache = SymCache::parse(bytes).map_err(|e| format!("symcache: {e}"))?;
    let mut quality = Quality::default();
    for function in cache.functions() {
        if !function.name().is_empty() {
            quality.functions += 1;
        }
        if cache.lookup(u64::from(function.entry_pc())).any(|loc| {
            loc.line() > 0 && loc.file().is_some_and(|file| !file.full_path().is_empty())
        }) {
            quality.lines += 1;
        }
    }
    quality.files = cache
        .files()
        .filter(|file| !file.full_path().is_empty())
        .count() as i64;
    Ok(quality)
}

#[derive(sqlx::FromRow)]
struct StoredArtifact {
    id: i64,
    file_path: String,
    arch: String,
    quality_lines: Option<i64>,
    quality_files: Option<i64>,
    quality_functions: Option<i64>,
    content_hash: Option<String>,
}

impl StoredArtifact {
    fn quality(&self) -> Option<Quality> {
        Some(Quality {
            lines: self.quality_lines?,
            files: self.quality_files?,
            functions: self.quality_functions?,
        })
    }
}

/// Publish a candidate only if selected, returning the durable canonical path
/// (which can be a richer historical file outside the content-addressed tree).
///
/// Native quality is measured from the cache itself. Equal-quality candidates,
/// including other artifact kinds, converge on the lexicographically smaller
/// SHA-256 digest regardless of upload order. Kinds never compete with each other.
pub async fn store_artifact(
    db: &DbPool,
    artifacts_dir: &str,
    metadata: ArtifactMetadata<'_>,
    payload: ArtifactPayload<'_>,
) -> Result<String, String> {
    let native = metadata.kind == "native";
    // Own the payload across blocking work without cloning its bytes. File
    // payloads are mapped rather than loaded into another full-size allocation.
    let bytes = match payload {
        ArtifactPayload::Bytes(bytes) => ByteView::from_vec(bytes),
        ArtifactPayload::File(path) => {
            let path = path.to_path_buf();
            tokio::task::spawn_blocking(move || ByteView::open(path))
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| format!("read: {e}"))?
        }
    };
    let (bytes, quality) = tokio::task::spawn_blocking(move || {
        let quality = if native {
            native_quality(&bytes)?
        } else {
            Quality::default()
        };
        Ok::<_, String>((bytes, quality))
    })
    .await
    .map_err(|e| e.to_string())??;

    let mut tx = db
        .writer()
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|e| e.to_string())?;
    let mut existing = sqlx::query_as::<_, StoredArtifact>(
        "SELECT id, file_path, arch, quality_lines, quality_files, quality_functions, content_hash \
         FROM artifact_debug_ids WHERE project_id = ? AND debug_id = ? AND kind = ? \
         AND (arch = ? OR (? AND arch = '')) ORDER BY arch = ? DESC LIMIT 1",
    )
    .bind(metadata.project_id)
    .bind(metadata.debug_id)
    .bind(metadata.kind)
    .bind(metadata.arch)
    .bind(native)
    .bind(metadata.arch)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    if let Some(stored) = &mut existing {
        // Migration leaves quality NULL deliberately. Never mistake an old rich
        // cache for a stripped file. Also recover the architecture of old NULLs.
        if stored.quality().is_none() || stored.content_hash.is_none() || stored.arch.is_empty() {
            let path = stored.file_path.clone();
            let inspected = tokio::task::spawn_blocking(move || {
                let bytes = match ByteView::open(&path) {
                    Ok(bytes) => bytes,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                    Err(e) => return Err(format!("read existing artifact: {e}")),
                };
                let quality = if native {
                    native_quality(&bytes)?
                } else {
                    Quality::default()
                };
                let arch = if native {
                    SymCache::parse(&bytes)
                        .map_err(|e| e.to_string())?
                        .arch()
                        .name()
                        .to_string()
                } else {
                    String::new()
                };
                Ok::<_, String>(Some((quality, sha256_hex(&bytes), arch)))
            })
            .await
            .map_err(|e| e.to_string())??;
            if let Some((old_quality, hash, arch)) = inspected {
                if native && stored.arch.is_empty() && arch != metadata.arch {
                    // This legacy row belongs to a different architecture.
                    existing = None;
                } else {
                    stored.quality_lines = Some(old_quality.lines);
                    stored.quality_files = Some(old_quality.files);
                    stored.quality_functions = Some(old_quality.functions);
                    stored.content_hash = Some(hash);
                }
            } else {
                // The canonical file has disappeared; a valid upload repairs it.
                stored.quality_lines = None;
                stored.quality_files = None;
                stored.quality_functions = None;
                stored.content_hash = None;
            }
        } else if !tokio::fs::try_exists(&stored.file_path)
            .await
            .map_err(|e| e.to_string())?
        {
            stored.quality_lines = None;
            stored.quality_files = None;
            stored.quality_functions = None;
            stored.content_hash = None;
        }
    }

    // Don't even hash an upload that is strictly worse than a known cache.
    if let Some(stored) = &existing
        && stored.quality().is_some_and(|old| old > quality)
    {
        update_existing(&mut tx, stored, &metadata).await?;
        tx.commit().await.map_err(|e| e.to_string())?;
        return Ok(stored.file_path.clone());
    }
    let (bytes, hash) = tokio::task::spawn_blocking(move || {
        let hash = sha256_hex(&bytes);
        (bytes, hash)
    })
    .await
    .map_err(|e| e.to_string())?;
    finish_selection(tx, artifacts_dir, metadata, bytes, quality, existing, hash).await
}

async fn update_existing(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    stored: &StoredArtifact,
    metadata: &ArtifactMetadata<'_>,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE artifact_debug_ids SET arch = ?, release_id = COALESCE(release_id, ?), \
         quality_lines = ?, quality_files = ?, quality_functions = ?, content_hash = ? WHERE id = ?",
    )
    .bind(metadata.arch)
    .bind(metadata.release_id)
    .bind(stored.quality_lines)
    .bind(stored.quality_files)
    .bind(stored.quality_functions)
    .bind(&stored.content_hash)
    .bind(stored.id)
    .execute(&mut **tx)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

async fn finish_selection(
    mut tx: sqlx::Transaction<'_, sqlx::Sqlite>,
    artifacts_dir: &str,
    metadata: ArtifactMetadata<'_>,
    bytes: ByteView<'static>,
    quality: Quality,
    existing: Option<StoredArtifact>,
    hash: String,
) -> Result<String, String> {
    if let Some(stored) = &existing
        && stored.quality() == Some(quality)
        && stored.content_hash.as_ref().is_some_and(|old| old <= &hash)
    {
        update_existing(&mut tx, stored, &metadata).await?;
        tx.commit().await.map_err(|e| e.to_string())?;
        return Ok(stored.file_path.clone());
    }

    let target = format!(
        "{artifacts_dir}/objects/{}/{}/{hash}",
        metadata.project_id,
        &hash[..2]
    );
    let publish_target = target.clone();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let target = Path::new(&publish_target);
        if target.try_exists().map_err(|e| e.to_string())? {
            return Ok(());
        }
        let dir = target.parent().expect("content path has parent");
        crate::util::atomic_fs::create_dir_all_durable(dir).map_err(|e| e.to_string())?;
        let mut tmp = NamedTempFile::new_in(dir).map_err(|e| e.to_string())?;
        tmp.write_all(&bytes).map_err(|e| e.to_string())?;
        tmp.as_file().sync_all().map_err(|e| e.to_string())?;
        match tmp.persist_noclobber(target) {
            Ok(_) => {}
            Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.error.to_string()),
        }
        // The directory tree is durable; now persist the new payload entry.
        std::fs::File::open(dir)
            .and_then(|file| file.sync_all())
            .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())??;

    sqlx::query(
        "INSERT INTO artifact_debug_ids \
         (id, project_id, release_id, debug_id, arch, code_id, source_name, kind, file_path, \
          quality_lines, quality_files, quality_functions, content_hash) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(id) DO UPDATE SET \
          arch = excluded.arch, code_id = excluded.code_id, source_name = excluded.source_name, \
          file_path = excluded.file_path, release_id = COALESCE(artifact_debug_ids.release_id, excluded.release_id), \
          quality_lines = excluded.quality_lines, quality_files = excluded.quality_files, \
          quality_functions = excluded.quality_functions, content_hash = excluded.content_hash",
    )
    .bind(existing.as_ref().map(|stored| stored.id))
    .bind(metadata.project_id)
    .bind(metadata.release_id)
    .bind(metadata.debug_id)
    .bind(metadata.arch)
    .bind(metadata.code_id)
    .bind(metadata.source_name)
    .bind(metadata.kind)
    .bind(&target)
    .bind(quality.lines)
    .bind(quality.files)
    .bind(quality.functions)
    .bind(hash)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(target)
}
