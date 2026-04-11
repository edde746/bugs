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
        loop {
            interval.tick().await;
            poll_envelopes(&db_poll, &config_poll, &checkpoint_poll).await;
        }
    });
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
