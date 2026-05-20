use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ChunkCleanupStats {
    pub deleted_files: u64,
    pub deleted_bytes: u64,
    pub removed_dirs: u64,
}

impl ChunkCleanupStats {
    fn add_file(&mut self, bytes: u64) {
        self.deleted_files += 1;
        self.deleted_bytes += bytes;
    }
}

pub fn chunk_path(root: &Path, hash: &str) -> PathBuf {
    root.join(&hash[..2]).join(hash)
}

pub async fn touch_chunk(path: &Path) -> io::Result<()> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let file = std::fs::OpenOptions::new().write(true).open(path)?;
        let times = std::fs::FileTimes::new().set_modified(SystemTime::now());
        file.set_times(times)
    })
    .await
    .map_err(io::Error::other)?
}

pub async fn cleanup_stale_chunks(
    root: &Path,
    retention: Duration,
) -> io::Result<ChunkCleanupStats> {
    let mut stats = ChunkCleanupStats::default();
    if !tokio::fs::try_exists(root).await? {
        return Ok(stats);
    }

    let cutoff = SystemTime::now()
        .checked_sub(retention)
        .unwrap_or(UNIX_EPOCH);
    let mut shard_entries = tokio::fs::read_dir(root).await?;

    while let Some(shard_entry) = shard_entries.next_entry().await? {
        let shard_path = shard_entry.path();
        let shard_type = shard_entry.file_type().await?;
        if !shard_type.is_dir() {
            continue;
        }

        let mut chunk_entries = tokio::fs::read_dir(&shard_path).await?;
        while let Some(chunk_entry) = chunk_entries.next_entry().await? {
            let chunk_type = chunk_entry.file_type().await?;
            if !chunk_type.is_file() {
                continue;
            }

            let chunk_path = chunk_entry.path();
            let meta = chunk_entry.metadata().await?;
            let modified = meta.modified().unwrap_or(UNIX_EPOCH);
            if modified > cutoff {
                continue;
            }

            let bytes = meta.len();
            match tokio::fs::remove_file(&chunk_path).await {
                Ok(()) => stats.add_file(bytes),
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }

        if is_empty_dir(&shard_path).await? {
            match tokio::fs::remove_dir(&shard_path).await {
                Ok(()) => stats.removed_dirs += 1,
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) if e.kind() == io::ErrorKind::DirectoryNotEmpty => {}
                Err(e) => return Err(e),
            }
        }
    }

    Ok(stats)
}

async fn is_empty_dir(path: &Path) -> io::Result<bool> {
    match tokio::fs::read_dir(path).await {
        Ok(mut entries) => Ok(entries.next_entry().await?.is_none()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_modified(path: &Path, modified: SystemTime) {
        let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        let times = std::fs::FileTimes::new().set_modified(modified);
        file.set_times(times).unwrap();
    }

    #[tokio::test]
    async fn cleanup_stale_chunks_deletes_old_files() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("chunks");
        let hash = "abcdefabcdefabcdefabcdefabcdefabcdefabcd";
        let path = chunk_path(&root, hash);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"stale").unwrap();
        set_modified(&path, SystemTime::now() - Duration::from_secs(7200));

        let stats = cleanup_stale_chunks(&root, Duration::from_secs(3600))
            .await
            .unwrap();

        assert_eq!(stats.deleted_files, 1);
        assert_eq!(stats.deleted_bytes, 5);
        assert!(!path.exists());
        assert!(!path.parent().unwrap().exists());
    }

    #[tokio::test]
    async fn cleanup_stale_chunks_preserves_fresh_files() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("chunks");
        let hash = "bcdefabcdefabcdefabcdefabcdefabcdefabcda";
        let path = chunk_path(&root, hash);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"fresh").unwrap();

        let stats = cleanup_stale_chunks(&root, Duration::from_secs(3600))
            .await
            .unwrap();

        assert_eq!(stats.deleted_files, 0);
        assert!(path.exists());
    }

    #[tokio::test]
    async fn touch_chunk_renews_existing_chunk() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("chunks");
        let hash = "cdefabcdefabcdefabcdefabcdefabcdefabcdab";
        let path = chunk_path(&root, hash);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"active").unwrap();

        let old = SystemTime::now() - Duration::from_secs(7200);
        set_modified(&path, old);

        touch_chunk(&path).await.unwrap();

        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert!(modified > old);
    }
}
