use sqlx::sqlite::{SqlitePool, SqlitePoolOptions, SqliteConnectOptions};
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
        let parts: Vec<u32> = version.0
            .split('.')
            .filter_map(|p| p.parse().ok())
            .collect();

        if parts.len() >= 3 {
            let (major, minor, patch) = (parts[0], parts[1], parts[2]);
            let ver = major * 1_000_000 + minor * 1_000 + patch;

            // JSONB requires >= 3.45.0
            if ver < 3_045_000 {
                warn!("SQLite {}.{}.{} is below 3.45.0 - JSONB not available", major, minor, patch);
            }

            // WAL fix allowlist: >= 3.51.3 OR (>= 3.50.7 AND < 3.51.0) OR (>= 3.44.6 AND < 3.45.0)
            let wal_safe = ver >= 3_051_003
                || (ver >= 3_050_007 && ver < 3_051_000)
                || (ver >= 3_044_006 && ver < 3_045_000);

            if !wal_safe {
                warn!(
                    "SQLite {}.{}.{} may have WAL corruption bug. \
                     Recommended: >= 3.51.3, or backports 3.50.7+, 3.44.6",
                    major, minor, patch
                );
            }
        }
    }

    async fn run_migrations(&self) -> Result<(), sqlx::Error> {
        // Set auto_vacuum before creating tables (only effective on empty DB)
        sqlx::query("PRAGMA auto_vacuum = INCREMENTAL")
            .execute(&self.writer)
            .await?;

        let migrations = [
            include_str!("../../migrations/001_initial_schema.sql"),
            include_str!("../../migrations/002_fts5_indexes.sql"),
            include_str!("../../migrations/003_expression_indexes.sql"),
        ];

        for (i, sql) in migrations.iter().enumerate() {
            // Execute each statement separately (SQLite doesn't support multi-statement in one query)
            for statement in sql.split(';') {
                let trimmed = statement.trim();
                if !trimmed.is_empty() {
                    sqlx::query(trimmed)
                        .execute(&self.writer)
                        .await
                        .map_err(|e| {
                            tracing::error!(migration = i + 1, statement = %trimmed, "Migration failed: {e}");
                            e
                        })?;
                }
            }
            info!(migration = i + 1, "Migration applied");
        }

        Ok(())
    }
}
