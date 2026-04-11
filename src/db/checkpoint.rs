use sqlx::SqlitePool;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::time::{interval, Duration};
use tracing::{debug, info, warn};

pub struct CheckpointManager {
    writer: SqlitePool,
    batch_count: Arc<AtomicU64>,
    checkpoint_interval: u64,
}

impl CheckpointManager {
    pub fn new(writer: SqlitePool, checkpoint_interval_batches: u32) -> Self {
        Self {
            writer,
            batch_count: Arc::new(AtomicU64::new(0)),
            checkpoint_interval: checkpoint_interval_batches as u64,
        }
    }

    pub fn batch_counter(&self) -> Arc<AtomicU64> {
        self.batch_count.clone()
    }

    /// Record a processed batch; returns true if a checkpoint should run
    pub fn record_batch(&self) -> bool {
        let count = self.batch_count.fetch_add(1, Ordering::Relaxed) + 1;
        count % self.checkpoint_interval == 0
    }

    pub async fn passive_checkpoint(&self) {
        match sqlx::query("PRAGMA wal_checkpoint(PASSIVE)")
            .execute(&self.writer)
            .await
        {
            Ok(_) => debug!("Passive WAL checkpoint completed"),
            Err(e) => warn!("Passive checkpoint failed: {e}"),
        }
    }

    pub async fn truncate_checkpoint(&self) {
        match sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&self.writer)
            .await
        {
            Ok(_) => info!("Truncate WAL checkpoint completed"),
            Err(e) => warn!("Truncate checkpoint failed: {e}"),
        }
    }

    /// Spawn a background task that runs truncate checkpoints during quiet periods
    pub fn spawn_quiet_checkpoint_task(self: Arc<Self>) {
        tokio::spawn(async move {
            let mut timer = interval(Duration::from_secs(10));
            let mut last_batch_count = 0u64;

            loop {
                timer.tick().await;
                let current = self.batch_count.load(Ordering::Relaxed);
                if current == last_batch_count && current > 0 {
                    // No new batches in last interval -> quiet period
                    self.truncate_checkpoint().await;
                }
                last_batch_count = current;
            }
        });
    }
}
