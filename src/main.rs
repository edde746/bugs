mod config;
mod db;
mod api;
mod ingest;
mod worker;
mod models;
mod sentry_protocol;
mod util;

use std::sync::Arc;
use clap::Parser;
use tokio::sync::mpsc;
use tracing::info;

use crate::config::Config;
use crate::db::DbPool;
use crate::db::checkpoint::CheckpointManager;

#[derive(Clone)]
pub struct AppState {
    pub db: DbPool,
    pub config: Arc<Config>,
    pub worker_tx: mpsc::Sender<worker::WorkerMessage>,
}

#[derive(clap::Parser)]
#[command(name = "bugs", about = "Sentry-compatible error tracker")]
struct Cli {
    /// Allow running without an admin token on non-loopback addresses.
    /// WARNING: This exposes management APIs without authentication.
    #[arg(long)]
    insecure_open_admin: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse CLI args
    let cli = Cli::parse();

    // Init tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "bugs=info,tower_http=info".parse().unwrap())
        )
        .init();

    // Load config
    let config = Config::load().unwrap_or_else(|e| {
        eprintln!("Config error: {e}. Using defaults.");
        Config::default()
    });
    let config = Arc::new(config);

    // Safety check: if no admin token and not loopback, require --insecure-open-admin
    if config.auth.admin_token.is_empty() && !is_loopback_address(&config.bind_address) {
        if !cli.insecure_open_admin {
            eprintln!(
                "ERROR: No admin_token configured and bind address '{}' is not loopback.\n\
                 This would expose management APIs without authentication.\n\
                 Either:\n  \
                 - Set auth.admin_token in bugs.toml or BUGS_AUTH_ADMIN_TOKEN env var\n  \
                 - Bind to 127.0.0.1 (loopback)\n  \
                 - Pass --insecure-open-admin to override this check",
                config.bind_address
            );
            std::process::exit(1);
        }
    }

    info!(bind = %config.bind_address, db = %config.database_path, "Starting Bugs");

    // Init database
    let db = DbPool::init(&config).await?;

    // Create worker channel
    let (worker_tx, worker_rx) = mpsc::channel::<worker::WorkerMessage>(10_000);

    // Create checkpoint manager
    let checkpoint = Arc::new(CheckpointManager::new(
        db.writer().clone(),
        config.sqlite.checkpoint_interval_batches,
    ));
    checkpoint.clone().spawn_quiet_checkpoint_task();

    // Spawn workers
    worker::spawn(db.clone(), config.clone(), checkpoint.clone(), worker_rx);

    // Spawn retention task
    db::retention::spawn_retention_task(
        db.writer().clone(),
        config.retention_days,
        config.envelope_retention_hours,
    );

    // Build app state
    let state = AppState {
        db,
        config: config.clone(),
        worker_tx,
    };

    // Build router
    let app = api::router(&state)
        .route("/health", axum::routing::get(|| async { "ok" }))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .layer(tower_http::compression::CompressionLayer::new())
        .layer(tower_http::cors::CorsLayer::permissive())
        .with_state(state);

    // Bind and serve
    let listener = tokio::net::TcpListener::bind(&config.bind_address).await?;
    info!(address = %config.bind_address, "Listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

/// Check if the bind address is a loopback address (127.x.x.x or [::1])
fn is_loopback_address(addr: &str) -> bool {
    // Extract host part (before the port)
    let host = if let Some(bracket_end) = addr.find(']') {
        // IPv6: [::1]:port
        &addr[1..bracket_end]
    } else if let Some(colon_pos) = addr.rfind(':') {
        &addr[..colon_pos]
    } else {
        addr
    };

    host == "127.0.0.1" || host == "localhost" || host == "::1"
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to install CTRL+C handler");
    info!("Shutting down");
}
