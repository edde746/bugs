use tracing::{debug, error};

use crate::config::Config;
use crate::db::DbPool;
use crate::db::checkpoint::CheckpointManager;
use crate::util::time::now_iso;

pub async fn process_envelope(
    db: &DbPool,
    _config: &Config,
    checkpoint: &CheckpointManager,
    envelope_id: i64,
) {
    // Atomic claim: pending/failed -> processing
    let now = now_iso();
    let claimed = sqlx::query(
        "UPDATE event_envelopes SET state = 'processing', processing_started_at = ? \
         WHERE id = ? AND state IN ('pending', 'failed', 'processing')"
    )
    .bind(&now)
    .bind(envelope_id)
    .execute(db.writer())
    .await;

    match claimed {
        Ok(r) if r.rows_affected() == 0 => return, // already claimed
        Err(e) => {
            error!(envelope_id, "Failed to claim envelope: {e}");
            return;
        }
        _ => {}
    }

    debug!(envelope_id, "Processing envelope");

    // TODO: Full pipeline implementation
    // 1. Load envelope body
    // 2. Decompress + parse
    // 3. Normalize
    // 4. Symbolicate
    // 5. Fingerprint
    // 6. Upsert issue
    // 7. Insert event
    // 8. Index tags
    // 9. Update stats
    // 10. Evaluate alerts
    // 11. Mark done

    // For now, just mark as done
    let _ = sqlx::query(
        "UPDATE event_envelopes SET state = 'done' WHERE id = ?"
    )
    .bind(envelope_id)
    .execute(db.writer())
    .await;

    if checkpoint.record_batch() {
        checkpoint.passive_checkpoint().await;
    }
}
