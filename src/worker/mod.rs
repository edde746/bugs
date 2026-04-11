pub mod processor;
pub mod normalizer;
pub mod fingerprint;
pub mod symbolication;
pub mod indexer;
pub mod alerts;

use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::config::Config;
use crate::db::DbPool;
use crate::db::checkpoint::CheckpointManager;

pub type WorkerMessage = i64; // event_envelope ID

pub fn spawn(
    db: DbPool,
    config: Arc<Config>,
    checkpoint: Arc<CheckpointManager>,
    mut rx: mpsc::Receiver<WorkerMessage>,
) {
    let num_workers = config.worker_threads;
    let (dispatch_tx, _) = tokio::sync::broadcast::channel::<WorkerMessage>(10_000);

    // Spawn channel receiver -> broadcast dispatcher
    let dtx = dispatch_tx.clone();
    tokio::spawn(async move {
        while let Some(envelope_id) = rx.recv().await {
            if dtx.send(envelope_id).is_err() {
                warn!("No active workers to receive message");
            }
        }
    });

    // Spawn worker tasks
    for worker_id in 0..num_workers {
        let db = db.clone();
        let config = config.clone();
        let checkpoint = checkpoint.clone();
        let mut sub = dispatch_tx.subscribe();

        tokio::spawn(async move {
            info!(worker_id, "Worker started");
            loop {
                tokio::select! {
                    Ok(envelope_id) = sub.recv() => {
                        processor::process_envelope(&db, &config, &checkpoint, envelope_id).await;
                    }
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
    // Claim pending and failed envelopes
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
