use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use std::str::FromStr;
use tracing::{info, warn};

use crate::config::Config;

#[derive(Clone)]
pub struct DbPool {
    writer: SqlitePool,
    reader: SqlitePool,
}

impl DbPool {
    pub async fn init(config: &Config) -> Result<Self, sqlx::Error> {
        let db_path = &config.database_path;

        // Ensure parent directory exists
        if let Some(parent) = std::path::Path::new(db_path).parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }

        let mmap_bytes = (config.sqlite.mmap_size_mb as i64) * 1024 * 1024;
        let cache_kb = (config.sqlite.cache_size_mb as i32) * 1024;

        // Writer: single connection
        let writer_opts = SqliteConnectOptions::from_str(db_path)?
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .synchronous(match config.sqlite.synchronous.to_uppercase().as_str() {
                "FULL" => sqlx::sqlite::SqliteSynchronous::Full,
                _ => sqlx::sqlite::SqliteSynchronous::Normal,
            })
            .pragma("mmap_size", mmap_bytes.to_string())
            .pragma("cache_size", format!("-{cache_kb}"))
            .pragma("foreign_keys", "ON")
            .pragma("busy_timeout", "5000")
            .pragma("temp_store", "MEMORY");

        let writer = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(writer_opts)
            .await?;

        // Reader: N connections
        let reader_opts = SqliteConnectOptions::from_str(db_path)?
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .synchronous(match config.sqlite.synchronous.to_uppercase().as_str() {
                "FULL" => sqlx::sqlite::SqliteSynchronous::Full,
                _ => sqlx::sqlite::SqliteSynchronous::Normal,
            })
            .pragma("mmap_size", mmap_bytes.to_string())
            .pragma("cache_size", format!("-{cache_kb}"))
            .pragma("foreign_keys", "ON")
            .pragma("busy_timeout", "5000")
            .pragma("temp_store", "MEMORY");

        let reader = SqlitePoolOptions::new()
            .max_connections(config.sqlite.reader_connections)
            .connect_with(reader_opts)
            .await?;

        let pool = Self { writer, reader };

        // Log SQLite version and check WAL fix
        pool.check_sqlite_version().await;

        // Run migrations
        pool.run_migrations().await?;

        Ok(pool)
    }

    pub fn writer(&self) -> &SqlitePool {
        &self.writer
    }

    pub fn reader(&self) -> &SqlitePool {
        &self.reader
    }

    async fn check_sqlite_version(&self) {
        let version: (String,) = sqlx::query_as("SELECT sqlite_version()")
            .fetch_one(&self.reader)
            .await
            .unwrap_or_else(|_| ("unknown".to_string(),));

        info!(sqlite_version = %version.0, "SQLite initialized");

        // Parse version for allowlist check
        let parts: Vec<u32> = version
            .0
            .split('.')
            .filter_map(|p| p.parse().ok())
            .collect();

        if parts.len() >= 3 {
            let (major, minor, patch) = (parts[0], parts[1], parts[2]);
            let ver = major * 1_000_000 + minor * 1_000 + patch;

            // JSONB requires >= 3.45.0
            if ver < 3_045_000 {
                warn!(
                    "SQLite {}.{}.{} is below 3.45.0 - JSONB not available",
                    major, minor, patch
                );
            }

            // WAL fix allowlist: >= 3.51.3 OR (>= 3.50.7 AND < 3.51.0) OR (>= 3.44.6 AND < 3.45.0)
            let wal_safe = ver >= 3_051_003
                || (3_050_007..3_051_000).contains(&ver)
                || (3_044_006..3_045_000).contains(&ver);

            if !wal_safe {
                warn!(
                    "SQLite {}.{}.{} may have WAL corruption bug. \
                     Recommended: >= 3.51.3, or backports 3.50.7+, 3.44.6",
                    major, minor, patch
                );
            }
        }
    }

    /// Apply pending migrations.
    ///
    /// The migration system is intentionally **forward-only**: each SQL
    /// file in `migrations/` is applied once in filename order and
    /// recorded in `_migrations`. There is no down-migration machinery
    /// and no rollback. New migrations must be additive — if you need
    /// to alter an existing table, write a follow-up migration that
    /// migrates data forward (see `014_event_tags_project_fk.sql` for
    /// an example of the table-rebuild pattern that SQLite requires).
    async fn run_migrations(&self) -> Result<(), sqlx::Error> {
        // Set auto_vacuum before creating tables (only effective on empty DB)
        sqlx::query("PRAGMA auto_vacuum = INCREMENTAL")
            .execute(&self.writer)
            .await?;

        // Track which migrations have been applied
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS _migrations (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')))"
        )
        .execute(&self.writer)
        .await?;

        let current_version: i64 =
            sqlx::query_scalar("SELECT COALESCE(MAX(version), 0) FROM _migrations")
                .fetch_one(&self.writer)
                .await?;

        let migrations = [
            include_str!("../../migrations/001_initial_schema.sql"),
            include_str!("../../migrations/002_fts5_indexes.sql"),
            include_str!("../../migrations/003_expression_indexes.sql"),
            include_str!("../../migrations/004_issue_muting.sql"),
            include_str!("../../migrations/005_issue_comments.sql"),
            include_str!("../../migrations/006_resolve_by_release.sql"),
            include_str!("../../migrations/007_issue_activity.sql"),
            include_str!("../../migrations/008_user_feedback.sql"),
            include_str!("../../migrations/009_deploys.sql"),
            include_str!("../../migrations/010_performance.sql"),
            include_str!("../../migrations/011_issue_sort_indexes.sql"),
            include_str!("../../migrations/012_issue_filter_indexes.sql"),
            include_str!("../../migrations/013_fts5_update_trigger.sql"),
            include_str!("../../migrations/014_event_tags_project_fk.sql"),
            include_str!("../../migrations/015_additional_indexes.sql"),
            include_str!("../../migrations/016_symbolication_state.sql"),
            include_str!("../../migrations/017_debug_ids_project_and_arch.sql"),
        ];

        for (i, sql) in migrations.iter().enumerate() {
            let version = (i + 1) as i64;
            if version <= current_version {
                continue;
            }

            for statement in split_sql_statements(sql) {
                let trimmed = statement.trim();
                if trimmed.is_empty() {
                    continue;
                }
                sqlx::query(trimmed)
                    .execute(&self.writer)
                    .await
                    .map_err(|e| {
                        tracing::error!(migration = version, statement = %trimmed, "Migration failed: {e}");
                        e
                    })?;
            }

            sqlx::query("INSERT INTO _migrations (version) VALUES (?)")
                .bind(version)
                .execute(&self.writer)
                .await?;

            info!(migration = version, "Migration applied");
        }

        Ok(())
    }
}

/// Split SQL text into individual statements, respecting BEGIN...END blocks (triggers).
/// Trigger statements like `CREATE TRIGGER ... BEGIN ... END;` are kept as one unit.
fn split_sql_statements(sql: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut depth = 0; // nesting depth for BEGIN...END

    for line in sql.lines() {
        let trimmed = line.trim();

        // Skip empty lines and comments when outside a statement
        if trimmed.is_empty() || trimmed.starts_with("--") {
            if !current.is_empty() {
                current.push('\n');
                current.push_str(line);
            }
            continue;
        }

        let upper = trimmed.to_uppercase();

        // Track BEGIN/END nesting
        if upper.contains("BEGIN") && !upper.contains("BEGIN TRANSACTION") {
            // Check it's a keyword, not part of a string
            let words: Vec<&str> = upper.split_whitespace().collect();
            if words.last() == Some(&"BEGIN") || words.contains(&"BEGIN") {
                depth += 1;
            }
        }

        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);

        if (upper.starts_with("END") || upper.starts_with("END;")) && depth > 0 {
            depth -= 1;
        }

        // Only split on `;` when not inside a BEGIN...END block
        if depth == 0 && trimmed.ends_with(';') {
            let stmt = current.trim().trim_end_matches(';').trim().to_string();
            if !stmt.is_empty() {
                statements.push(stmt);
            }
            current.clear();
        }
    }

    // Remaining
    let remaining = current.trim().trim_end_matches(';').trim().to_string();
    if !remaining.is_empty() {
        statements.push(remaining);
    }

    statements
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_simple_statements() {
        let sql = "CREATE TABLE foo (id INTEGER);\nCREATE TABLE bar (id INTEGER);";
        let stmts = split_sql_statements(sql);
        assert_eq!(stmts.len(), 2);
        assert!(stmts[0].starts_with("CREATE TABLE foo"));
        assert!(stmts[1].starts_with("CREATE TABLE bar"));
    }

    #[test]
    fn test_split_preserves_trigger_block() {
        let sql = "\
CREATE TABLE events (id INTEGER);\n\
CREATE TRIGGER my_trigger AFTER INSERT ON events BEGIN\n\
    INSERT INTO log (msg) VALUES ('inserted');\n\
END;\n\
CREATE TABLE other (id INTEGER);";
        let stmts = split_sql_statements(sql);
        assert_eq!(stmts.len(), 3);
        assert!(stmts[0].contains("CREATE TABLE events"));
        assert!(stmts[1].contains("CREATE TRIGGER"));
        assert!(stmts[1].contains("BEGIN"));
        assert!(stmts[1].contains("END"));
        assert!(stmts[2].contains("CREATE TABLE other"));
    }

    #[test]
    fn test_split_empty_input() {
        assert_eq!(split_sql_statements("").len(), 0);
        assert_eq!(split_sql_statements("  \n  ").len(), 0);
    }

    #[test]
    fn test_split_comments_only() {
        let sql = "-- this is a comment\n-- another comment";
        assert_eq!(split_sql_statements(sql).len(), 0);
    }

    #[test]
    fn test_split_no_trailing_semicolon() {
        let sql = "SELECT 1";
        let stmts = split_sql_statements(sql);
        assert_eq!(stmts.len(), 1);
        assert_eq!(stmts[0], "SELECT 1");
    }
}
