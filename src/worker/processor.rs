use std::io::Read;

use flate2::read::GzDecoder;
use tracing::{debug, error, warn};

use crate::config::Config;
use crate::db::DbPool;
use crate::db::checkpoint::CheckpointManager;
use crate::sentry_protocol::envelope::Envelope;
use crate::sentry_protocol::types::SentryEvent;
use crate::util::time::{now_iso, hour_bucket};
use crate::worker::{alerts, fingerprint, indexer, normalizer, symbolication};

pub async fn process_envelope(
    db: &DbPool,
    config: &Config,
    checkpoint: &CheckpointManager,
    envelope_id: i64,
) {
    // Atomic claim: pending/failed -> processing
    let now = now_iso();
    let claimed = sqlx::query(
        "UPDATE event_envelopes SET state = 'processing', processing_started_at = ? \
         WHERE id = ? AND state IN ('pending', 'failed', 'processing')",
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

    match process_inner(db, config, envelope_id).await {
        Ok(()) => {
            // Mark done
            let _ = sqlx::query("UPDATE event_envelopes SET state = 'done' WHERE id = ?")
                .bind(envelope_id)
                .execute(db.writer())
                .await;
        }
        Err(e) => {
            warn!(envelope_id, "Processing failed: {e}");
            mark_failed(db, envelope_id, &e.to_string()).await;
        }
    }

    if checkpoint.record_batch() {
        checkpoint.passive_checkpoint().await;
    }
}

async fn mark_failed(db: &DbPool, envelope_id: i64, error_msg: &str) {
    let truncated = if error_msg.len() > 1000 {
        &error_msg[..1000]
    } else {
        error_msg
    };

    // Check current attempts to decide: dead-letter after 5
    let current: Option<(i64,)> = sqlx::query_as(
        "SELECT attempts FROM event_envelopes WHERE id = ?"
    )
    .bind(envelope_id)
    .fetch_optional(db.reader())
    .await
    .unwrap_or(None);

    let attempts = current.map(|r| r.0).unwrap_or(0);

    if attempts >= 4 {
        // 5th failure -> dead letter (no more retries)
        let _ = sqlx::query(
            "UPDATE event_envelopes SET state = 'dead', attempts = attempts + 1, last_error = ? WHERE id = ?",
        )
        .bind(truncated)
        .bind(envelope_id)
        .execute(db.writer())
        .await;
    } else {
        // Exponential backoff: 5s, 10s, 20s, 40s, ... capped at 300s
        let _ = sqlx::query(
            "UPDATE event_envelopes SET \
                state = 'failed', \
                attempts = attempts + 1, \
                last_error = ?, \
                next_attempt_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', \
                    '+' || CAST(MIN(300, 5 * (1 << MIN(attempts, 8))) AS TEXT) || ' seconds') \
             WHERE id = ?",
        )
        .bind(truncated)
        .bind(envelope_id)
        .execute(db.writer())
        .await;
    }
}

async fn process_inner(
    db: &DbPool,
    config: &Config,
    envelope_id: i64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // 1. Load envelope body from DB
    let row: (i64, Vec<u8>, Option<String>) = sqlx::query_as(
        "SELECT project_id, body, content_encoding FROM event_envelopes WHERE id = ?",
    )
    .bind(envelope_id)
    .fetch_one(db.reader())
    .await?;

    let project_id = row.0;
    let raw_body = row.1;
    let _content_encoding = row.2;

    // 2. Decompress if gzipped (check 0x1f 0x8b magic bytes)
    let body = if raw_body.len() >= 2 && raw_body[0] == 0x1f && raw_body[1] == 0x8b {
        let mut decoder = GzDecoder::new(&raw_body[..]);
        let mut decompressed = Vec::new();
        decoder.read_to_end(&mut decompressed)?;
        decompressed
    } else {
        raw_body
    };

    // 3. Parse envelope
    let envelope = Envelope::parse(&body)?;

    // 4. Process each event item
    let mut processed_any = false;
    for item in &envelope.items {
        if item.headers.item_type != "event" {
            continue;
        }

        // Parse as SentryEvent
        let mut event: SentryEvent = serde_json::from_slice(&item.payload)?;

        // 5. Normalize
        normalizer::normalize(&mut event);

        // 6. Symbolicate using source maps
        if let Err(e) = symbolication::symbolicate_event(&mut event, db, &config.artifacts_dir).await {
            warn!(envelope_id, "Symbolication failed (non-fatal): {e}");
        }

        // 7. Compute fingerprint
        let fp = fingerprint::compute_fingerprint(&event);

        // 8. Derive title and culprit
        let title = fingerprint::derive_title(&event);
        let culprit = fingerprint::derive_culprit(&event);

        // Extract fields for DB insertion
        let event_id = event.event_id.clone().unwrap_or_default();
        let timestamp = event
            .timestamp
            .as_ref()
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let level = event.level.clone().unwrap_or_else(|| "error".to_string());
        let platform = event.platform.clone();
        let release = event.release.clone();
        let environment = event.environment.clone();
        let transaction_name = event.transaction.clone();

        // Extract trace_id from contexts.trace.trace_id
        let trace_id = event
            .contexts
            .as_ref()
            .and_then(|c| c.get("trace"))
            .and_then(|t| t.get("trace_id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Extract message
        let message = event
            .message
            .clone()
            .or_else(|| {
                event
                    .logentry
                    .as_ref()
                    .and_then(|le| le.message.clone())
            });

        // Extract exception_values: "Type: value" joined by newlines
        let exception_values = event.exception.as_ref().map(|exc| {
            exc.values
                .iter()
                .map(|ev| {
                    let t = ev.exception_type.as_deref().unwrap_or("Error");
                    let v = ev.value.as_deref().unwrap_or("");
                    if v.is_empty() {
                        t.to_string()
                    } else {
                        format!("{t}: {v}")
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        });

        // Extract stacktrace_functions: function names joined by newlines
        let stacktrace_functions = event.exception.as_ref().map(|exc| {
            exc.values
                .iter()
                .filter_map(|ev| ev.stacktrace.as_ref())
                .flat_map(|st| st.frames.iter())
                .filter_map(|f| f.function.as_ref())
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        });

        // Serialize normalized event as JSON for the data column
        let data = serde_json::to_string(&event)?;

        let received_at = now_iso();

        // 9. UPSERT into issues table
        let issue: (i64, i64) = sqlx::query_as(
            "INSERT INTO issues (project_id, fingerprint, title, culprit, level, status, first_seen, last_seen, event_count) \
             VALUES (?, ?, ?, ?, ?, 'unresolved', ?, ?, 1) \
             ON CONFLICT(project_id, fingerprint) DO UPDATE SET \
                title = excluded.title, \
                culprit = excluded.culprit, \
                level = excluded.level, \
                last_seen = excluded.last_seen, \
                event_count = event_count + 1 \
             RETURNING id, event_count",
        )
        .bind(project_id)
        .bind(&fp)
        .bind(&title)
        .bind(&culprit)
        .bind(&level)
        .bind(&timestamp)
        .bind(&timestamp)
        .fetch_one(db.writer())
        .await?;

        let issue_id = issue.0;
        let is_new_issue = issue.1 == 1;

        // 10. INSERT into events table
        let event_row: (i64,) = sqlx::query_as(
            "INSERT INTO events (event_id, project_id, issue_id, timestamp, received_at, level, \
                platform, release, environment, transaction_name, trace_id, message, title, \
                exception_values, stacktrace_functions, data) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             RETURNING id",
        )
        .bind(&event_id)
        .bind(project_id)
        .bind(issue_id)
        .bind(&timestamp)
        .bind(&received_at)
        .bind(&level)
        .bind(&platform)
        .bind(&release)
        .bind(&environment)
        .bind(&transaction_name)
        .bind(&trace_id)
        .bind(&message)
        .bind(&title)
        .bind(&exception_values)
        .bind(&stacktrace_functions)
        .bind(&data)
        .fetch_one(db.writer())
        .await?;

        let event_row_id = event_row.0;

        // 11. Index tags and update stats
        indexer::index_event(
            db,
            config,
            event_row_id,
            project_id,
            issue_id,
            &timestamp,
            &event,
        )
        .await?;

        // Update issue_stats_hourly
        let bucket = hour_bucket(&timestamp);
        sqlx::query(
            "INSERT INTO issue_stats_hourly (issue_id, project_id, bucket, count) \
             VALUES (?, ?, ?, 1) \
             ON CONFLICT(issue_id, bucket) DO UPDATE SET count = count + 1",
        )
        .bind(issue_id)
        .bind(project_id)
        .bind(&bucket)
        .execute(db.writer())
        .await?;

        // 12. Evaluate alert rules
        alerts::evaluate_alerts(db, project_id, issue_id, &event, is_new_issue)
            .await
            .ok();

        processed_any = true;
    }

    if !processed_any {
        return Err("No event items found in envelope".into());
    }

    Ok(())
}
