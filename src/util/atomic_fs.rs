//! Atomic file writes via temp-file + rename.
//!
//! Crash-safe replacement for blind `tokio::fs::write` / `tokio::fs::copy`:
//! readers see either the old contents or the new contents, never a
//! half-written file. The destination directory is created if missing.
//!
//! `write_atomic` runs the write + `sync_all` on a blocking thread so the
//! durability fence doesn't stall the tokio reactor; `copy_atomic` uses
//! `tokio::fs::copy` directly since it's already on the blocking pool.

use std::io::Write;
use std::path::Path;

/// Stage `bytes` at `dir/target` via tmp + rename. The final
/// `sync_all` runs on a blocking thread because it blocks on disk flush.
pub async fn write_atomic(dir: &str, target: &str, bytes: Vec<u8>) -> std::io::Result<()> {
    tokio::fs::create_dir_all(dir).await?;
    let tmp = format!("{target}.tmp.{}", std::process::id());
    let tmp_move = tmp.clone();
    tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp_move)?;
        f.write_all(&bytes)?;
        f.sync_all()
    })
    .await
    .map_err(std::io::Error::other)??;
    tokio::fs::rename(&tmp, target).await?;
    Ok(())
}

/// Copy `src` to `dir/target` via tmp + rename. No `sync_all` because
/// the source bundles this is used for can tolerate weaker durability —
/// a crash mid-upload just means the client retries.
pub async fn copy_atomic(src: &Path, dir: &str, target: &str) -> std::io::Result<()> {
    tokio::fs::create_dir_all(dir).await?;
    let tmp = format!("{target}.tmp.{}", std::process::id());
    tokio::fs::copy(src, &tmp).await?;
    tokio::fs::rename(&tmp, target).await?;
    Ok(())
}
