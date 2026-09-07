//! Durable DIF assembly queue. HTTP handlers only inspect receipts and pin
//! chunks; bounded workers do the expensive hashing and conversion. Hard links
//! keep admitted chunks alive independently of upload-staging retention.

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use sqlx::SqlitePool;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::api::chunked_upload::{
    AssembleEntry, AssembleResponse, AssembleState, AssemblyResult, assemble_one,
};
use crate::config::Config;
use crate::db::DbPool;
use crate::util::chunk_store::chunk_path;
use crate::util::id::{is_sha1_hex, normalize_debug_id};

const LEASE_SECONDS: i64 = 60;
const ERROR_RETENTION_SECONDS: i64 = 60;

#[derive(sqlx::FromRow)]
struct Receipt {
    id: String,
    state: String,
    result_json: Option<String>,
    updated_at: i64,
}

#[derive(sqlx::FromRow)]
struct Job {
    id: String,
    project_id: i64,
    checksum: String,
    request_json: String,
    staging_root: String,
}

/// A completed receipt is consulted before any staging-chunk lookup. Failed
/// attempts remain visible to polling clients briefly, but do not permanently
/// poison a checksum after a transient disk/database failure.
pub(crate) async fn request(
    db: &DbPool,
    config: &Config,
    project_id: i64,
    checksum: &str,
    entry: &AssembleEntry,
) -> Result<AssembleResponse, String> {
    let checksum = checksum.to_ascii_lowercase();
    let binding = entry
        .debug_id
        .as_deref()
        .map(normalize_debug_id)
        .unwrap_or_default();
    let receipt: Option<Receipt> = sqlx::query_as(
        "SELECT id, state, result_json, updated_at FROM dif_assemblies \
         WHERE project_id = ? AND checksum = ? AND debug_id = ?",
    )
    .bind(project_id)
    .bind(&checksum)
    .bind(&binding)
    .fetch_optional(db.reader())
    .await
    .map_err(|e| format!("assembly lookup: {e}"))?;
    if let Some(receipt) = receipt {
        match receipt.state.as_str() {
            "pending" | "processing" => return Ok(AssembleResponse::assembling()),
            "ok" | "error" => {
                let result: AssemblyResult = serde_json::from_str(
                    receipt
                        .result_json
                        .as_deref()
                        .ok_or("assembly receipt has no result")?,
                )
                .map_err(|e| format!("assembly receipt: {e}"))?;
                let mut usable = receipt.state == "ok"
                    || receipt.updated_at + ERROR_RETENTION_SECONDS
                        > chrono::Utc::now().timestamp();
                for artifact in &result.artifacts {
                    let path: Option<String> = sqlx::query_scalar(
                        "SELECT file_path FROM artifact_debug_ids \
                         WHERE project_id = ? AND debug_id = ? AND arch = ? AND kind = ?",
                    )
                    .bind(project_id)
                    .bind(&artifact.debug_id)
                    .bind(&artifact.arch)
                    .bind(&artifact.kind)
                    .fetch_optional(db.reader())
                    .await
                    .map_err(|e| format!("canonical artifact lookup: {e}"))?;
                    let Some(path) = path else {
                        usable = false;
                        break;
                    };
                    if !tokio::fs::try_exists(&path)
                        .await
                        .map_err(|e| format!("artifact lookup: {e}"))?
                    {
                        usable = false;
                        break;
                    }
                }
                if usable {
                    return Ok(result.response);
                }
                // Match the old ID: a concurrent retry may already have replaced it.
                sqlx::query("DELETE FROM dif_assemblies WHERE id = ? AND state IN ('ok', 'error')")
                    .bind(&receipt.id)
                    .execute(db.writer())
                    .await
                    .map_err(|e| format!("expire assembly receipt: {e}"))?;
            }
            _ => return Err("invalid assembly state".to_string()),
        }
    }

    if entry.chunks.is_empty() {
        return Err("no chunks in request".to_string());
    }
    let mut normalized = entry.clone();
    for chunk in &mut normalized.chunks {
        if !is_sha1_hex(chunk) {
            return Err(format!("invalid chunk hash: {chunk}"));
        }
        chunk.make_ascii_lowercase();
    }
    let id = uuid::Uuid::new_v4().to_string();
    let staging_root = Path::new(&config.artifacts_dir)
        .join("assemblies")
        .join(&id);
    let upload_root = Path::new(&config.artifacts_dir).join("chunks");
    let pin_root = staging_root.clone();
    let chunks = normalized.chunks.clone();
    let missing = tokio::task::spawn_blocking(move || pin_chunks(&upload_root, &pin_root, &chunks))
        .await
        .map_err(|e| format!("pin chunks task: {e}"))?
        .map_err(|e| format!("pin chunks: {e}"))?;
    if !missing.is_empty() {
        return Ok(AssembleResponse {
            state: AssembleState::NotFound,
            missing_chunks: missing,
            detail: None,
            dif: None,
        });
    }

    let inserted = sqlx::query(
        "INSERT INTO dif_assemblies \
         (id, project_id, checksum, debug_id, request_json, staging_root, state, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, 'pending', ?) \
         ON CONFLICT(project_id, checksum, debug_id) DO NOTHING",
    )
    .bind(&id)
    .bind(project_id)
    .bind(&checksum)
    .bind(&binding)
    .bind(serde_json::to_string(&normalized).map_err(|e| e.to_string())?)
    .bind(staging_root.to_string_lossy().as_ref())
    .bind(chrono::Utc::now().timestamp())
    .execute(db.writer())
    .await;
    match inserted {
        Ok(result) if result.rows_affected() == 1 => {}
        Ok(_) => remove_staging(&staging_root).await,
        Err(error) => {
            remove_staging(&staging_root).await;
            return Err(format!("enqueue assembly: {error}"));
        }
    }
    Ok(AssembleResponse::assembling())
}

fn pin_chunks(upload_root: &Path, pin_root: &Path, chunks: &[String]) -> io::Result<Vec<String>> {
    let result = (|| {
        let mut missing = Vec::new();
        let mut seen = HashSet::with_capacity(chunks.len());
        let mut shards = HashSet::new();
        for hash in chunks {
            if !seen.insert(hash) {
                continue;
            }
            let source = chunk_path(upload_root, hash);
            let target = chunk_path(pin_root, hash);
            let parent = target.parent().expect("chunk path has parent");
            crate::util::atomic_fs::create_dir_all_durable(parent)?;
            match std::fs::hard_link(&source, &target) {
                Ok(()) => {
                    shards.insert(parent.to_path_buf());
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => missing.push(hash.clone()),
                Err(e) => return Err(e),
            }
        }
        if missing.is_empty() {
            // Ancestors were synced when created; persist the new hard links.
            for shard in shards {
                std::fs::File::open(shard)?.sync_all()?;
            }
        }
        Ok(missing)
    })();
    if !matches!(&result, Ok(missing) if missing.is_empty()) {
        match std::fs::remove_dir_all(pin_root) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => {
                warn!(path = %pin_root.display(), "failed to remove rejected assembly pins: {e}")
            }
        }
    }
    result
}

pub(crate) fn spawn(
    db: DbPool,
    config: Arc<Config>,
    shutdown: watch::Receiver<bool>,
) -> Vec<JoinHandle<()>> {
    (0..config.uploads.assembly_concurrency.max(1))
        .map(|worker| {
            let db = db.clone();
            let config = config.clone();
            let mut shutdown = shutdown.clone();
            tokio::spawn(async move {
                info!(worker, "DIF assembly worker started");
                loop {
                    if *shutdown.borrow() || shutdown.has_changed().is_err() {
                        break;
                    }
                    let owner = uuid::Uuid::new_v4().to_string();
                    match claim(&db, &owner).await {
                        Ok(Some(job)) => {
                            run_job(&db, &config, &owner, job).await;
                            continue;
                        }
                        Ok(None) => {}
                        Err(e) => warn!(worker, "claim assembly: {e}"),
                    }
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_millis(100)) => {}
                        _ = shutdown.changed() => break,
                    }
                }
            })
        })
        .collect()
}

async fn claim(db: &DbPool, owner: &str) -> Result<Option<Job>, sqlx::Error> {
    let now = chrono::Utc::now().timestamp();
    sqlx::query_as(
        "UPDATE dif_assemblies SET state = 'processing', lease_owner = ?, lease_until = ?, updated_at = ? \
         WHERE id = (SELECT id FROM dif_assemblies \
             WHERE state = 'pending' OR (state = 'processing' AND lease_until < ?) \
             ORDER BY updated_at, id LIMIT 1) \
         RETURNING id, project_id, checksum, request_json, staging_root",
    )
    .bind(owner)
    .bind(now + LEASE_SECONDS)
    .bind(now)
    .bind(now)
    .fetch_optional(db.writer())
    .await
}

async fn run_job(db: &DbPool, config: &Arc<Config>, owner: &str, job: Job) {
    let started = std::time::Instant::now();
    // Conversion must keep progressing while the lease task waits for SQLite's
    // single writer. Polling both in one task can deadlock if conversion owns a
    // transaction when the heartbeat tries to acquire that same connection.
    let process_db = db.clone();
    let process_config = config.clone();
    let request_json = job.request_json.clone();
    let staging_root = job.staging_root.clone();
    let checksum = job.checksum.clone();
    let project_id = job.project_id;
    let mut processing = tokio::spawn(async move {
        let entry: AssembleEntry = serde_json::from_str(&request_json)
            .map_err(|e| format!("decode assembly request: {e}"))?;
        assemble_one(
            &process_db,
            &process_config,
            project_id,
            &staging_root,
            &checksum,
            &entry,
        )
        .await
    });
    let mut heartbeat = tokio::time::interval(Duration::from_secs(10));
    let result = loop {
        tokio::select! {
            result = &mut processing => break result.unwrap_or_else(|e| Err(format!("assembly task: {e}"))),
            _ = heartbeat.tick() => {
                let renewed = sqlx::query(
                    "UPDATE dif_assemblies SET lease_until = ? WHERE id = ? AND lease_owner = ? AND state = 'processing'",
                )
                .bind(chrono::Utc::now().timestamp() + LEASE_SECONDS)
                .bind(&job.id)
                .bind(owner)
                .execute(db.writer()).await;
                if !matches!(&renewed, Ok(row) if row.rows_affected() == 1) {
                    warn!(job = %job.id, "assembly lease lost; leaving recovery to queue");
                    // spawn_blocking cannot be aborted. Keep this worker's
                    // capacity occupied until its conversion really finishes.
                    let _ = processing.await;
                    return;
                }
            }
        }
    };
    let (state, result) = match result {
        Ok(result) => ("ok", result),
        Err(detail) => {
            warn!(job = %job.id, checksum = %job.checksum, "DIF assembly failed: {detail}");
            (
                "error",
                AssemblyResult {
                    response: AssembleResponse::error(detail),
                    artifacts: Vec::new(),
                },
            )
        }
    };
    let result_json = match serde_json::to_string(&result) {
        Ok(json) => json,
        Err(e) => {
            warn!(job = %job.id, "serialize assembly result: {e}");
            return;
        }
    };
    let published = sqlx::query(
        "UPDATE dif_assemblies SET state = ?, result_json = ?, lease_owner = NULL, lease_until = NULL, updated_at = ? \
         WHERE id = ? AND lease_owner = ? AND state = 'processing'",
    )
    .bind(state)
    .bind(result_json)
    .bind(chrono::Utc::now().timestamp())
    .bind(&job.id)
    .bind(owner)
    .execute(db.writer())
    .await;
    match published {
        Ok(row) if row.rows_affected() == 1 => {
            remove_staging(Path::new(&job.staging_root)).await;
            info!(job = %job.id, checksum = %job.checksum, state, elapsed_ms = started.elapsed().as_millis(), "DIF assembly finished");
        }
        Ok(_) => warn!(job = %job.id, "assembly finished after ownership changed"),
        Err(e) => warn!(job = %job.id, "publish assembly result: {e}"),
    }
}

async fn remove_staging(path: &Path) {
    match tokio::fs::remove_dir_all(path).await {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => warn!(path = %path.display(), "remove assembly staging: {e}"),
    }
}

/// Clean admission attempts interrupted before enqueue and completed-job pins
/// left behind by a crash. Pending/processing jobs retain pins regardless of age.
pub(crate) async fn cleanup_staging(
    writer: &SqlitePool,
    artifacts_dir: &str,
    retention: Duration,
) -> Result<(), String> {
    let root = Path::new(artifacts_dir).join("assemblies");
    let mut dirs = match tokio::fs::read_dir(&root).await {
        Ok(dirs) => dirs,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("read assembly staging: {e}")),
    };
    let cutoff = SystemTime::now()
        .checked_sub(retention)
        .unwrap_or(SystemTime::UNIX_EPOCH);
    while let Some(dir) = dirs.next_entry().await.map_err(|e| e.to_string())? {
        if !dir.file_type().await.map_err(|e| e.to_string())?.is_dir() {
            continue;
        }
        let modified = dir
            .metadata()
            .await
            .map_err(|e| e.to_string())?
            .modified()
            .map_err(|e| e.to_string())?;
        if modified > cutoff {
            continue;
        }
        let path: PathBuf = dir.path();
        let active: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM dif_assemblies WHERE staging_root = ? AND state IN ('pending', 'processing'))",
        )
        .bind(path.to_string_lossy().as_ref())
        .fetch_one(writer).await.map_err(|e| e.to_string())?;
        if !active {
            remove_staging(&path).await;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha1::{Digest, Sha1};

    const DEBUG_ID: &str = "aabbccdd-eeff-0011-2233-445566778899";

    struct Fixture {
        _temp: tempfile::TempDir,
        db: DbPool,
        config: Arc<Config>,
    }

    impl Fixture {
        async fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let config = Arc::new(Config {
                database_path: temp.path().join("bugs.db").to_string_lossy().into_owned(),
                artifacts_dir: temp.path().join("artifacts").to_string_lossy().into_owned(),
                ..Config::default()
            });
            let db = DbPool::init(&config).await.unwrap();
            sqlx::query(
                "INSERT INTO projects (id, org_id, name, slug) VALUES (1, 1, 'Symbols', 'symbols')",
            )
            .execute(db.writer())
            .await
            .unwrap();
            Self {
                _temp: temp,
                db,
                config,
            }
        }

        fn upload(&self, bytes: &[u8]) -> (String, AssembleEntry) {
            let checksum = hex::encode(Sha1::digest(bytes));
            let path = chunk_path(
                &Path::new(&self.config.artifacts_dir).join("chunks"),
                &checksum,
            );
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
            let entry = AssembleEntry {
                name: Some("obfuscation.map.json".to_string()),
                debug_id: Some(DEBUG_ID.to_string()),
                chunks: vec![checksum.clone()],
            };
            (checksum, entry)
        }

        async fn request(&self, checksum: &str, entry: &AssembleEntry) -> AssembleResponse {
            request(&self.db, &self.config, 1, checksum, entry)
                .await
                .unwrap()
        }

        async fn finish(&self, checksum: &str, entry: &AssembleEntry) -> AssembleResponse {
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let response = self.request(checksum, entry).await;
                    if !matches!(response.state, AssembleState::Assembling) {
                        return response;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("assembly did not finish")
        }
    }

    #[tokio::test]
    async fn pinned_jobs_survive_retention_and_recover_expired_worker_leases() {
        let fixture = Fixture::new().await;
        let (checksum, entry) = fixture.upload(br#"["ReadableError","a"]"#);
        assert!(matches!(
            fixture.request(&checksum, &entry).await.state,
            AssembleState::Assembling
        ));

        // Model a process dying after it claimed the job, before conversion.
        let job = claim(&fixture.db, "dead-worker").await.unwrap().unwrap();
        sqlx::query("UPDATE dif_assemblies SET lease_until = 0 WHERE id = ?")
            .bind(&job.id)
            .execute(fixture.db.writer())
            .await
            .unwrap();
        crate::util::chunk_store::cleanup_stale_chunks(
            &Path::new(&fixture.config.artifacts_dir).join("chunks"),
            Duration::ZERO,
        )
        .await
        .unwrap();
        cleanup_staging(
            fixture.db.writer(),
            &fixture.config.artifacts_dir,
            Duration::ZERO,
        )
        .await
        .unwrap();
        assert!(chunk_path(Path::new(&job.staging_root), &checksum).exists());

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let workers = spawn(fixture.db.clone(), fixture.config.clone(), shutdown_rx);
        let response = fixture.finish(&checksum, &entry).await;
        assert!(matches!(response.state, AssembleState::Ok), "{response:?}");
        shutdown_tx.send(true).unwrap();
        for worker in workers {
            worker.await.unwrap();
        }

        // No worker or upload chunks remain, but the durable receipt is usable.
        assert!(matches!(
            fixture.request(&checksum, &entry).await.state,
            AssembleState::Ok
        ));
        assert!(!Path::new(&job.staging_root).exists());
    }

    #[tokio::test]
    async fn concurrent_submissions_and_claims_have_one_owner() {
        let fixture = Fixture::new().await;
        let (checksum, entry) = fixture.upload(br#"["ReadableError","a"]"#);
        let (left, right) = tokio::join!(
            fixture.request(&checksum, &entry),
            fixture.request(&checksum, &entry),
        );
        assert!(matches!(left.state, AssembleState::Assembling));
        assert!(matches!(right.state, AssembleState::Assembling));
        let (left, right) = tokio::join!(claim(&fixture.db, "left"), claim(&fixture.db, "right"),);
        let left = left.unwrap();
        let right = right.unwrap();
        assert_ne!(
            left.is_some(),
            right.is_some(),
            "one submitted DIF has exactly one claimant"
        );
        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM dif_assemblies")
            .fetch_one(fixture.db.reader())
            .await
            .unwrap();
        assert_eq!(rows, 1, "retries must not multiply queued conversions");
    }

    #[tokio::test]
    async fn equal_map_content_keeps_distinct_debug_id_bindings() {
        let fixture = Fixture::new().await;
        let (checksum, first) = fixture.upload(br#"["ReadableError","a"]"#);
        let mut second = first.clone();
        second.debug_id = Some("11223344-5566-7788-9900-aabbccddeeff".to_string());
        fixture.request(&checksum, &first).await;
        fixture.request(&checksum, &second).await;
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let workers = spawn(fixture.db.clone(), fixture.config.clone(), shutdown_rx);
        let left = serde_json::to_value(fixture.finish(&checksum, &first).await).unwrap();
        let right = serde_json::to_value(fixture.finish(&checksum, &second).await).unwrap();
        assert_eq!(left["dif"]["debugId"], first.debug_id.as_deref().unwrap());
        assert_eq!(right["dif"]["debugId"], second.debug_id.as_deref().unwrap());
        shutdown_tx.send(true).unwrap();
        for worker in workers {
            worker.await.unwrap();
        }
    }

    #[tokio::test]
    async fn missing_processed_artifact_is_reassembled_instead_of_false_success() {
        let fixture = Fixture::new().await;
        let (checksum, entry) = fixture.upload(br#"["ReadableError","a"]"#);
        fixture.request(&checksum, &entry).await;
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let workers = spawn(fixture.db.clone(), fixture.config.clone(), shutdown_rx);
        assert!(matches!(
            fixture.finish(&checksum, &entry).await.state,
            AssembleState::Ok
        ));
        let path: String =
            sqlx::query_scalar("SELECT file_path FROM artifact_debug_ids WHERE project_id = 1")
                .fetch_one(fixture.db.reader())
                .await
                .unwrap();
        tokio::fs::remove_file(&path).await.unwrap();
        assert!(matches!(
            fixture.request(&checksum, &entry).await.state,
            AssembleState::Assembling
        ));
        assert!(matches!(
            fixture.finish(&checksum, &entry).await.state,
            AssembleState::Ok
        ));
        assert!(Path::new(&path).exists());
        shutdown_tx.send(true).unwrap();
        for worker in workers {
            worker.await.unwrap();
        }
    }

    #[tokio::test]
    async fn completed_receipt_checks_current_canonical_target_after_replacement() {
        let fixture = Fixture::new().await;
        let mut maps: [&[u8]; 2] = [br#"["ReadableError","a"]"#, br#"["OtherError","a"]"#];
        // Equal-quality map variants converge on the smaller content digest.
        maps.sort_by_key(|bytes| std::cmp::Reverse(crate::util::hash::sha256_hex(bytes)));
        let (first_hash, first) = fixture.upload(maps[0]);
        let (second_hash, second) = fixture.upload(maps[1]);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let workers = spawn(fixture.db.clone(), fixture.config.clone(), shutdown_rx);
        fixture.request(&first_hash, &first).await;
        assert!(matches!(
            fixture.finish(&first_hash, &first).await.state,
            AssembleState::Ok
        ));
        let original: String =
            sqlx::query_scalar("SELECT file_path FROM artifact_debug_ids WHERE project_id = 1")
                .fetch_one(fixture.db.reader())
                .await
                .unwrap();
        fixture.request(&second_hash, &second).await;
        assert!(matches!(
            fixture.finish(&second_hash, &second).await.state,
            AssembleState::Ok
        ));
        let replacement: String =
            sqlx::query_scalar("SELECT file_path FROM artifact_debug_ids WHERE project_id = 1")
                .fetch_one(fixture.db.reader())
                .await
                .unwrap();
        assert_ne!(original, replacement);
        assert!(Path::new(&original).exists());
        tokio::fs::remove_file(&replacement).await.unwrap();

        // A's historical payload still exists, but symbolication's current
        // pointer does not. A cached success must not hide that missing target.
        assert!(matches!(
            fixture.request(&first_hash, &first).await.state,
            AssembleState::Assembling
        ));
        assert!(matches!(
            fixture.finish(&first_hash, &first).await.state,
            AssembleState::Ok
        ));
        let repaired: String =
            sqlx::query_scalar("SELECT file_path FROM artifact_debug_ids WHERE project_id = 1")
                .fetch_one(fixture.db.reader())
                .await
                .unwrap();
        assert!(Path::new(&repaired).exists());
        shutdown_tx.send(true).unwrap();
        for worker in workers {
            worker.await.unwrap();
        }
    }

    #[tokio::test]
    async fn corrupt_checksum_finishes_with_error_not_a_successful_receipt() {
        let fixture = Fixture::new().await;
        let (_, entry) = fixture.upload(br#"["ReadableError","a"]"#);
        let checksum = "1111111111111111111111111111111111111111";
        fixture.request(checksum, &entry).await;
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let workers = spawn(fixture.db.clone(), fixture.config.clone(), shutdown_rx);
        assert!(matches!(
            fixture.finish(checksum, &entry).await.state,
            AssembleState::Error
        ));
        assert!(matches!(
            fixture.request(checksum, &entry).await.state,
            AssembleState::Error
        ));
        shutdown_tx.send(true).unwrap();
        for worker in workers {
            worker.await.unwrap();
        }
    }
}
