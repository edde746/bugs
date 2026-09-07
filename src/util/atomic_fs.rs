//! Atomic file writes via temp-file + rename.
//!
//! Crash-safe replacement for blind `tokio::fs::write` / `tokio::fs::copy`:
//! readers see either the old contents or the new contents, never a
//! half-written file. The destination directory is created if missing.
//!
//! Blocking filesystem work stays off the reactor. Unique, same-directory
//! temporary files are removed automatically on every failure path.

use std::io::Write;
use std::path::{Path, PathBuf};

use tempfile::NamedTempFile;

/// Persist newly created directory entries up to the first existing ancestor.
/// Syncing only the leaf does not make its newly created parents durable.
pub(crate) fn create_dir_all_durable(path: &Path) -> std::io::Result<()> {
    let mut missing = Vec::new();
    let mut ancestor = path;
    while !ancestor.try_exists()? {
        missing.push(ancestor.to_path_buf());
        ancestor = ancestor
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
    }
    std::fs::create_dir_all(path)?;
    if !missing.is_empty() {
        for directory in missing {
            std::fs::File::open(directory)?.sync_all()?;
        }
        std::fs::File::open(ancestor)?.sync_all()?;
    }
    Ok(())
}

/// Stage `bytes` at `dir/target` via tmp + rename. The final
/// `sync_all` runs on a blocking thread because it blocks on disk flush.
pub async fn write_atomic(dir: &str, target: &str, bytes: Vec<u8>) -> std::io::Result<()> {
    let dir = PathBuf::from(dir);
    let target = PathBuf::from(target);
    tokio::task::spawn_blocking(move || {
        create_dir_all_durable(&dir)?;
        let mut tmp = NamedTempFile::new_in(target.parent().unwrap_or(&dir))?;
        tmp.write_all(&bytes)?;
        tmp.as_file().sync_all()?;
        tmp.persist(&target).map_err(|e| e.error)?;
        std::fs::File::open(target.parent().unwrap_or(&dir))?.sync_all()?;
        Ok(())
    })
    .await
    .map_err(std::io::Error::other)?
}

/// Copy `src` through a unique temporary file; readers never see a partial copy.
pub async fn copy_atomic(src: &Path, dir: &str, target: &str) -> std::io::Result<()> {
    let src = src.to_path_buf();
    let dir = PathBuf::from(dir);
    let target = PathBuf::from(target);
    tokio::task::spawn_blocking(move || {
        create_dir_all_durable(&dir)?;
        let mut tmp = NamedTempFile::new_in(target.parent().unwrap_or(&dir))?;
        std::io::copy(&mut std::fs::File::open(src)?, &mut tmp)?;
        tmp.as_file().sync_all()?;
        tmp.persist(&target).map_err(|e| e.error)?;
        std::fs::File::open(target.parent().unwrap_or(&dir))?.sync_all()?;
        Ok(())
    })
    .await
    .map_err(std::io::Error::other)?
}
