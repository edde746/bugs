mod config;
mod db;
mod api;
mod ingest;
mod worker;
mod models;
mod sentry_protocol;
mod util;

use std::sync::Arc;
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
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
    let app = api::router()
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

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to install CTRL+C handler");
    info!("Shutting down");
}
