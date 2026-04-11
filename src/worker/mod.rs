pub mod processor;
pub mod normalizer;
pub mod fingerprint;
pub mod symbolication;
pub mod indexer;
pub mod alerts;

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
    let checkpoint_poll = checkpoint.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        let mut tick_count: u64 = 0;
        loop {
            interval.tick().await;
            poll_envelopes(&db_poll, &config_poll, &checkpoint_poll).await;

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
async fn update_transaction_percentiles(db: &DbPool) {
    // Only recalculate for groups that had recent activity (last hour)
    sqlx::query(
        "UPDATE transaction_groups SET \
            p50_duration_ms = ( \
                SELECT duration_ms FROM transactions WHERE group_id = transaction_groups.id \
                ORDER BY duration_ms LIMIT 1 \
                OFFSET (SELECT MIN(count, (SELECT COUNT(*) FROM transactions t2 WHERE t2.group_id = transaction_groups.id)) / 2 \
                        FROM transaction_groups tg2 WHERE tg2.id = transaction_groups.id) \
            ), \
            p95_duration_ms = ( \
                SELECT duration_ms FROM transactions WHERE group_id = transaction_groups.id \
                ORDER BY duration_ms LIMIT 1 \
                OFFSET (SELECT MIN(count, (SELECT COUNT(*) FROM transactions t2 WHERE t2.group_id = transaction_groups.id)) * 95 / 100 \
                        FROM transaction_groups tg2 WHERE tg2.id = transaction_groups.id) \
            ) \
         WHERE last_seen >= strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-1 hour')",
    )
    .execute(db.writer())
    .await
    .ok();
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

async fn poll_envelopes(db: &DbPool, config: &Config, checkpoint: &CheckpointManager) {
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
        processor::process_envelope(db, config, checkpoint, id).await;
    }
}
