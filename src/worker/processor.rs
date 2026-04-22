use tracing::{debug, error, warn};

use crate::api::ingest::decompress_gzip_capped;
use crate::config::Config;
use crate::db::DbPool;
use crate::db::checkpoint::CheckpointManager;
use crate::sentry_protocol::envelope::Envelope;
use crate::sentry_protocol::types::SentryEvent;
use crate::util::log::truncate;
use crate::util::time::{hour_bucket, now_iso};
use crate::worker::{
    alerts, fingerprint, indexer, native_symbolication, normalizer, symbolication,
};

/// Cap on the length (in chars) of any user-derived string we put through
/// tracing macros. Big enough to keep real error messages useful, small
/// enough to prevent log floods from hostile or oversized payloads.
const LOG_TRUNCATE: usize = 256;

pub async fn process_envelope(
    db: &DbPool,
    config: &Config,
    checkpoint: &CheckpointManager,
    envelope_id: i64,
) {
    // Atomic claim: pending/failed -> processing. The third branch
    // ('processing' + stale processing_started_at) is the stuck-worker
    // recovery path — we only steal an in-flight envelope if the original
    // worker appears to be dead (started more than 60s ago). Without the
    // staleness guard two workers would race on a freshly-claimed envelope
    // and duplicate the decompress+parse work.
    let now = now_iso();
    let claimed = sqlx::query(
        "UPDATE event_envelopes SET state = 'processing', processing_started_at = ? \
         WHERE id = ? AND ( \
            state IN ('pending', 'failed') \
            OR (state = 'processing' AND processing_started_at < strftime('%Y-%m-%dT%H:%M:%SZ','now','-60 seconds')) \
         )",
    )
    .bind(&now)
    .bind(envelope_id)
    .execute(db.writer())
    .await;

    match claimed {
        Ok(r) if r.rows_affected() == 0 => return, // already claimed
        Err(e) => {
            error!(
                envelope_id,
                "Failed to claim envelope: {}",
                truncate(&e.to_string(), LOG_TRUNCATE)
            );
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
            let msg = e.to_string();
            warn!(
                envelope_id,
                "Processing failed: {}",
                truncate(&msg, LOG_TRUNCATE)
            );
            mark_failed(db, envelope_id, &msg).await;
        }
    }

    if checkpoint.record_batch() {
        checkpoint.passive_checkpoint().await;
    }
}

async fn mark_failed(db: &DbPool, envelope_id: i64, error_msg: &str) {
    // Char-aware truncation — slicing on bytes can panic on multibyte
    // UTF-8 boundaries when the upstream error contains non-ASCII data.
    let truncated = truncate(error_msg, 1000);

    // Atomic update: dead-letter after 5 attempts, otherwise retry with exponential backoff.
    // No separate read — avoids TOCTOU race with concurrent workers.
    let _ = sqlx::query(
        "UPDATE event_envelopes SET \
            attempts = attempts + 1, \
            last_error = ?, \
            state = CASE WHEN attempts >= 4 THEN 'dead' ELSE 'failed' END, \
            next_attempt_at = CASE WHEN attempts >= 4 THEN NULL \
                ELSE strftime('%Y-%m-%dT%H:%M:%SZ', 'now', \
                    '+' || CAST(MIN(300, 5 * (1 << MIN(attempts, 8))) AS TEXT) || ' seconds') \
                END \
         WHERE id = ?",
    )
    .bind(truncated)
    .bind(envelope_id)
    .execute(db.writer())
    .await;
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

    // 2. Decompress if gzipped (check 0x1f 0x8b magic bytes); cap at the
    //    same envelope limit used at ingest so a stored bomb can't OOM the
    //    worker if it ever reaches us.
    let body = if raw_body.len() >= 2 && raw_body[0] == 0x1f && raw_body[1] == 0x8b {
        decompress_gzip_capped(&raw_body, config.ingest.max_envelope_bytes)?
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

        // 6. Symbolicate: JS source maps, then native DIFs. Each path
        //    mutates `event` in place and returns an outcome; we fold
        //    both into the single `symbolication_state` column so a
        //    later release-file / dSYM upload can identify which events
        //    need re-symbolication.
        let js_outcome =
            match symbolication::symbolicate_event(&mut event, db, &config.artifacts_dir).await {
                Ok(outcome) => outcome,
                Err(e) => {
                    warn!(
                        envelope_id,
                        "Symbolication failed (non-fatal): {}",
                        truncate(&e.to_string(), LOG_TRUNCATE)
                    );
                    symbolication::SymbolicationOutcome::NotAttempted
                }
            };
        let native_outcome =
            native_symbolication::symbolicate_native(&mut event, project_id, db).await;
        let symbolication_state: Option<&'static str> =
            combine_outcomes(js_outcome, &native_outcome);

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
            .or_else(|| event.logentry.as_ref().and_then(|le| le.message.clone()));

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

        // 9. Check for regression and snooze state before upsert
        let existing: Option<(String, Option<i64>)> = sqlx::query_as(
            "SELECT status, snooze_event_count FROM issues WHERE project_id = ? AND fingerprint = ?",
        )
        .bind(project_id)
        .bind(&fp)
        .fetch_optional(db.writer())
        .await?;

        let old_status = existing.as_ref().map(|(s, _)| s.as_str());

        // UPSERT into issues table — auto-reopen resolved issues on new events,
        // and auto-unmute ignored issues when snooze_event_count threshold is reached
        let issue: (i64, i64) = sqlx::query_as(
            "INSERT INTO issues (project_id, fingerprint, title, culprit, level, status, first_seen, last_seen, event_count) \
             VALUES (?, ?, ?, ?, ?, 'unresolved', ?, ?, 1) \
             ON CONFLICT(project_id, fingerprint) DO UPDATE SET \
                title = excluded.title, \
                culprit = excluded.culprit, \
                level = excluded.level, \
                last_seen = excluded.last_seen, \
                event_count = event_count + 1, \
                status = CASE \
                    WHEN issues.status = 'resolved' THEN 'unresolved' \
                    WHEN issues.status = 'ignored' AND issues.snooze_event_count IS NOT NULL \
                         AND (event_count + 1) >= issues.snooze_event_count THEN 'unresolved' \
                    ELSE issues.status \
                END, \
                snooze_until = CASE \
                    WHEN issues.status = 'resolved' THEN NULL \
                    WHEN issues.status = 'ignored' AND issues.snooze_event_count IS NOT NULL \
                         AND (event_count + 1) >= issues.snooze_event_count THEN NULL \
                    ELSE issues.snooze_until \
                END, \
                snooze_event_count = CASE \
                    WHEN issues.status = 'resolved' THEN NULL \
                    WHEN issues.status = 'ignored' AND issues.snooze_event_count IS NOT NULL \
                         AND (event_count + 1) >= issues.snooze_event_count THEN NULL \
                    ELSE issues.snooze_event_count \
                END, \
                resolved_in_release = CASE \
                    WHEN issues.status = 'resolved' THEN NULL \
                    ELSE issues.resolved_in_release \
                END \
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
        let is_regression = !is_new_issue && old_status == Some("resolved");

        // Record activity for new issues and regressions
        if is_new_issue {
            sqlx::query("INSERT INTO issue_activity (issue_id, kind) VALUES (?, 'first_seen')")
                .bind(issue_id)
                .execute(db.writer())
                .await
                .ok();
        } else if is_regression {
            sqlx::query("INSERT INTO issue_activity (issue_id, kind) VALUES (?, 'regression')")
                .bind(issue_id)
                .execute(db.writer())
                .await
                .ok();
        }

        // 10. INSERT into events table
        let event_row: (i64,) = sqlx::query_as(
            "INSERT INTO events (event_id, project_id, issue_id, timestamp, received_at, level, \
                platform, release, environment, transaction_name, trace_id, message, title, \
                exception_values, stacktrace_functions, data, symbolication_state) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
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
        .bind(symbolication_state)
        .fetch_one(db.writer())
        .await?;

        let event_row_id = event_row.0;

        // 11. Auto-create release record if event has a release string
        if let Some(ref release_str) = release
            && let Err(e) = super::releases::ensure_release(db, project_id, release_str).await
        {
            warn!(
                envelope_id,
                "Failed to auto-create release (non-fatal): {e}"
            );
        }

        // Index tags and update stats
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
        alerts::evaluate_alerts(
            db,
            config,
            project_id,
            issue_id,
            &event,
            is_new_issue,
            is_regression,
        )
        .await
        .ok();

        processed_any = true;
    }

    // Process transaction items (performance monitoring)
    for item in &envelope.items {
        if item.headers.item_type != "transaction" {
            continue;
        }

        if let Ok(txn) = serde_json::from_slice::<serde_json::Value>(&item.payload) {
            let transaction_name = txn
                .get("transaction")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();

            let op = txn
                .get("contexts")
                .and_then(|c| c.get("trace"))
                .and_then(|t| t.get("op"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let status = txn
                .get("contexts")
                .and_then(|c| c.get("trace"))
                .and_then(|t| t.get("status"))
                .and_then(|v| v.as_str())
                .unwrap_or("ok")
                .to_string();

            let method = txn
                .get("request")
                .and_then(|r| r.get("method"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            // Calculate duration from start_timestamp and timestamp (both in seconds as f64).
            // Clamp to zero: clock skew or a missing timestamp (unwrap_or(0.0)) can make
            // end_ts < start_ts, which poisons MIN/SUM aggregates and percentile sorts.
            let start_ts = txn
                .get("start_timestamp")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let end_ts = txn.get("timestamp").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let duration_ms = (end_ts - start_ts).max(0.0) * 1000.0;

            let timestamp_str = txn
                .get("timestamp")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let trace_id = txn
                .get("contexts")
                .and_then(|c| c.get("trace"))
                .and_then(|t| t.get("trace_id"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let environment = txn
                .get("environment")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let release = txn
                .get("release")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let is_error = status != "ok" && status != "cancelled";

            let data = serde_json::to_string(&txn).unwrap_or_default();

            // Upsert transaction group
            let group: (i64,) = sqlx::query_as(
                "INSERT INTO transaction_groups (project_id, transaction_name, op, method, count, error_count, sum_duration_ms, min_duration_ms, max_duration_ms, last_seen) \
                 VALUES (?, ?, ?, ?, 1, ?, ?, ?, ?, ?) \
                 ON CONFLICT(project_id, transaction_name, op, method) DO UPDATE SET \
                    count = count + 1, \
                    error_count = error_count + excluded.error_count, \
                    sum_duration_ms = sum_duration_ms + excluded.sum_duration_ms, \
                    min_duration_ms = MIN(COALESCE(transaction_groups.min_duration_ms, excluded.min_duration_ms), excluded.min_duration_ms), \
                    max_duration_ms = MAX(COALESCE(transaction_groups.max_duration_ms, excluded.max_duration_ms), excluded.max_duration_ms), \
                    last_seen = excluded.last_seen \
                 RETURNING id",
            )
            .bind(project_id)
            .bind(&transaction_name)
            .bind(&op)
            .bind(&method)
            .bind(if is_error { 1i64 } else { 0 })
            .bind(duration_ms)
            .bind(duration_ms)
            .bind(duration_ms)
            .bind(&timestamp_str)
            .fetch_one(db.writer())
            .await?;

            // Insert individual transaction record
            sqlx::query(
                "INSERT INTO transactions (project_id, group_id, trace_id, transaction_name, op, method, status, duration_ms, timestamp, environment, release, data) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(project_id)
            .bind(group.0)
            .bind(&trace_id)
            .bind(&transaction_name)
            .bind(&op)
            .bind(&method)
            .bind(&status)
            .bind(duration_ms)
            .bind(&timestamp_str)
            .bind(&environment)
            .bind(&release)
            .bind(&data)
            .execute(db.writer())
            .await?;

            // Auto-create release record if transaction has a release string
            if let Some(ref release_str) = release
                && let Err(e) = super::releases::ensure_release(db, project_id, release_str).await
            {
                warn!(
                    envelope_id,
                    "Failed to auto-create release for transaction (non-fatal): {e}"
                );
            }

            processed_any = true;
        }
    }

    // Process user_report items
    for item in &envelope.items {
        if item.headers.item_type != "user_report" {
            continue;
        }

        #[derive(serde::Deserialize)]
        struct UserReportPayload {
            event_id: Option<String>,
            name: Option<String>,
            email: Option<String>,
            comments: Option<String>,
        }

        if let Ok(report) = serde_json::from_slice::<UserReportPayload>(&item.payload) {
            let report_event_id = report
                .event_id
                .or_else(|| envelope.headers.event_id.clone())
                .unwrap_or_default();

            sqlx::query(
                "INSERT INTO user_reports (project_id, event_id, name, email, comments) \
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(project_id)
            .bind(&report_event_id)
            .bind(report.name.as_deref().unwrap_or(""))
            .bind(report.email.as_deref().unwrap_or(""))
            .bind(report.comments.as_deref().unwrap_or(""))
            .execute(db.writer())
            .await
            .ok();

            processed_any = true;
        }
    }

    if !processed_any {
        debug!(
            envelope_id,
            "Envelope contained no processable items, skipping"
        );
    }

    Ok(())
}

/// Fold JS + native symbolication outcomes into the single
/// `events.symbolication_state` column. Precedence: any successful
/// resolution → "ok"; else any MissingMap → "missing_map" (eligible
/// for retry on later upload); else NULL.
fn combine_outcomes(
    js: symbolication::SymbolicationOutcome,
    native: &native_symbolication::NativeSymbolicationOutcome,
) -> Option<&'static str> {
    use native_symbolication::NativeSymbolicationOutcome as N;
    use symbolication::SymbolicationOutcome as J;

    let js_ok = matches!(js, J::Ok);
    let native_ok = matches!(native, N::Ok { resolved, .. } if *resolved > 0);
    if js_ok || native_ok {
        return Some("ok");
    }

    let js_missing = matches!(js, J::MissingMap);
    let native_missing = matches!(native, N::MissingMap);
    if js_missing || native_missing {
        return Some("missing_map");
    }

    None
}
