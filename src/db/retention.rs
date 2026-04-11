use sqlx::SqlitePool;
use tokio::time::{interval, Duration};
use tracing::{info, warn};

pub fn spawn_retention_task(writer: SqlitePool, retention_days: u32, envelope_retention_hours: u32) {
    tokio::spawn(async move {
        let mut timer = interval(Duration::from_secs(3600));
        loop {
            timer.tick().await;
            if let Err(e) = run_cleanup(&writer, retention_days, envelope_retention_hours).await {
                warn!("Retention cleanup failed: {e}");
            }
        }
    });
}

async fn run_cleanup(
    writer: &SqlitePool,
    retention_days: u32,
    envelope_retention_hours: u32,
) -> Result<(), sqlx::Error> {
    let event_cutoff = format!("-{retention_days} days");
    let envelope_cutoff = format!("-{envelope_retention_hours} hours");

    // Delete done/dead envelopes older than retention
    let envelopes_deleted = sqlx::query(
        "DELETE FROM event_envelopes WHERE state IN ('done', 'dead') \
         AND received_at < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?)"
    )
    .bind(&envelope_cutoff)
    .execute(writer)
    .await?;

    // Delete old events
    let events_deleted = sqlx::query(
        "DELETE FROM events WHERE received_at < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?)"
    )
    .bind(&event_cutoff)
    .execute(writer)
    .await?;

    // Delete orphaned issues
    let issues_deleted = sqlx::query(
        "DELETE FROM issues WHERE id NOT IN (SELECT DISTINCT issue_id FROM events WHERE issue_id IS NOT NULL)"
    )
    .execute(writer)
    .await?;

    // Prune old stats
    sqlx::query(
        "DELETE FROM issue_stats_hourly WHERE bucket < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?)"
    )
    .bind(&event_cutoff)
    .execute(writer)
    .await?;

    // Incremental vacuum
    sqlx::query("PRAGMA incremental_vacuum(1000)")
        .execute(writer)
        .await?;

    info!(
        envelopes = envelopes_deleted.rows_affected(),
        events = events_deleted.rows_affected(),
        issues = issues_deleted.rows_affected(),
        "Retention cleanup completed"
    );

    Ok(())
}
