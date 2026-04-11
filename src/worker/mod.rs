pub mod alerts;
pub mod fingerprint;
pub mod indexer;
pub mod normalizer;
pub mod processor;
pub mod symbolication;

use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::info;

use crate::config::Config;
use crate::db::DbPool;
use crate::db::checkpoint::CheckpointManager;

pub type WorkerMessage = i64; // event_envelope ID

pub fn spawn(
    db: DbPool,
    config: Arc<Config>,
    checkpoint: Arc<CheckpointManager>,
    tx: mpsc::Sender<WorkerMessage>,
    rx: mpsc::Receiver<WorkerMessage>,
) {
    let num_workers = config.worker_threads;

    // Wrap the single receiver in Arc<Mutex> so workers compete for messages
    let shared_rx = Arc::new(tokio::sync::Mutex::new(rx));

    // Spawn worker tasks - each competes for the next message
    for worker_id in 0..num_workers {
        let db = db.clone();
        let config = config.clone();
        let checkpoint = checkpoint.clone();
        let rx = shared_rx.clone();

        tokio::spawn(async move {
            info!(worker_id, "Worker started");
            loop {
                // Only one worker receives each message
                let envelope_id = {
                    let mut rx = rx.lock().await;
                    rx.recv().await
                };

                match envelope_id {
                    Some(id) => {
                        processor::process_envelope(&db, &config, &checkpoint, id).await;
                    }
                    None => break, // channel closed
                }
            }
        });
    }

    // Spawn poller for missed/failed/stuck envelopes
    let db_poll = db.clone();
    let config_poll = config.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        let mut tick_count: u64 = 0;
        loop {
            interval.tick().await;
            poll_envelopes(&db_poll, &tx).await;

            tick_count += 1;
            // Every ~60 seconds (12 ticks * 5s)
            if tick_count % 12 == 0 {
                unmute_expired_issues(&db_poll).await;
                update_transaction_percentiles(&db_poll).await;
            }
            // Every ~5 minutes (60 ticks * 5s): clean up old transactions
            if tick_count % 60 == 0 {
                cleanup_old_transactions(&db_poll, config_poll.retention_days).await;
            }
        }
    });
}

/// Unmute issues whose snooze_until has expired. Uses UPDATE RETURNING to avoid TOCTOU race.
async fn unmute_expired_issues(db: &DbPool) {
    let unmuted: Vec<(i64,)> = sqlx::query_as(
        "UPDATE issues SET status = 'unresolved', snooze_until = NULL, snooze_event_count = NULL \
         WHERE status = 'ignored' AND snooze_until IS NOT NULL \
         AND snooze_until <= strftime('%Y-%m-%dT%H:%M:%SZ', 'now') \
         RETURNING id",
    )
    .fetch_all(db.writer())
    .await
    .unwrap_or_default();

    if !unmuted.is_empty() {
        info!(count = unmuted.len(), "Auto-unmuted expired issues");
        for (issue_id,) in &unmuted {
            sqlx::query(
                "INSERT INTO issue_activity (issue_id, kind, data) VALUES (?, 'unignored', '{\"reason\":\"snooze_expired\"}')",
            )
            .bind(issue_id)
            .execute(db.writer())
            .await
            .ok();
        }
    }
}

/// Periodically recalculate p50/p95 for transaction groups using recent data.
/// Reads percentiles from a reader connection, then writes in a single batch on the writer.
async fn update_transaction_percentiles(db: &DbPool) {
    // Read active group IDs from a reader (no writer contention)
    let groups: Vec<(i64,)> = sqlx::query_as(
        "SELECT id FROM transaction_groups \
         WHERE last_seen >= strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-1 hour')",
    )
    .fetch_all(db.reader())
    .await
    .unwrap_or_default();

    for (group_id,) in groups {
        // Compute percentiles on reader
        let durations: Vec<(f64,)> = sqlx::query_as(
            "SELECT duration_ms FROM transactions WHERE group_id = ? ORDER BY duration_ms",
        )
        .bind(group_id)
        .fetch_all(db.reader())
        .await
        .unwrap_or_default();

        if durations.is_empty() {
            continue;
        }

        let n = durations.len();
        let p50 = durations[n / 2].0;
        let p95 = durations[(n * 95 / 100).min(n - 1)].0;

        // Single targeted UPDATE on writer
        sqlx::query(
            "UPDATE transaction_groups SET p50_duration_ms = ?, p95_duration_ms = ? WHERE id = ?",
        )
        .bind(p50)
        .bind(p95)
        .bind(group_id)
        .execute(db.writer())
        .await
        .ok();
    }
}

/// Clean up old transaction rows to prevent unbounded growth.
async fn cleanup_old_transactions(db: &DbPool, retention_days: u32) {
    let result = sqlx::query(
        "DELETE FROM transactions WHERE created_at < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-' || ? || ' days')",
    )
    .bind(retention_days)
    .execute(db.writer())
    .await;

    if let Ok(r) = result {
        if r.rows_affected() > 0 {
            info!(count = r.rows_affected(), "Cleaned up old transactions");
        }
    }
}

async fn poll_envelopes(db: &DbPool, tx: &mpsc::Sender<WorkerMessage>) {
    let envelopes: Vec<(i64,)> = sqlx::query_as(
        "SELECT id FROM event_envelopes \
         WHERE (state = 'pending' AND (next_attempt_at IS NULL OR next_attempt_at <= strftime('%Y-%m-%dT%H:%M:%SZ','now'))) \
         OR (state = 'failed' AND next_attempt_at <= strftime('%Y-%m-%dT%H:%M:%SZ','now')) \
         OR (state = 'processing' AND processing_started_at < strftime('%Y-%m-%dT%H:%M:%SZ','now','-60 seconds')) \
         LIMIT 50"
    )
    .fetch_all(db.reader())
    .await
    .unwrap_or_default();

    for (id,) in envelopes {
        let _ = tx.try_send(id);
    }
}
