use axum::http::header;
use std::sync::Arc;
use tokio::sync::mpsc;
use tower_http::set_header::SetResponseHeaderLayer;
use tracing::info;

use bugs::AppState;
use bugs::config::Config;
use bugs::db::DbPool;
use bugs::db::checkpoint::CheckpointManager;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "bugs=info,tower_http=info".parse().unwrap()),
        )
        .init();

    let config = Config::load().unwrap_or_else(|e| {
        eprintln!("Config error: {e}. Using defaults.");
        Config::default()
    });
    let config = Arc::new(config);

    // Warn if no admin token on a non-loopback address
    if config.auth.admin_token.is_empty() && !is_loopback_address(&config.bind_address) {
        tracing::warn!(
            address = %config.bind_address,
            "No admin_token configured on a non-loopback address — management API is unauthenticated"
        );
    }

    info!(bind = %config.bind_address, db = %config.database_path, "Starting Bugs");

    let db = DbPool::init(&config).await?;

    let (worker_tx, worker_rx) = mpsc::channel::<bugs::worker::WorkerMessage>(10_000);

    let checkpoint = Arc::new(CheckpointManager::new(
        db.writer().clone(),
        config.sqlite.checkpoint_interval_batches,
    ));
    checkpoint.clone().spawn_quiet_checkpoint_task();

    bugs::worker::spawn(
        db.clone(),
        config.clone(),
        checkpoint.clone(),
        worker_tx.clone(),
        worker_rx,
    );

    bugs::db::retention::spawn_retention_task(
        db.writer().clone(),
        config.retention_days,
        config.envelope_retention_hours,
    );

    let state = AppState {
        db,
        config: config.clone(),
        worker_tx,
        rate_limiter: bugs::ingest::abuse::RateLimiter::new(),
    };

    let app = bugs::api::router(&state)
        .route("/health", axum::routing::get(|| async { "ok" }))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .layer(tower_http::compression::CompressionLayer::new())
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            header::HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_FRAME_OPTIONS,
            header::HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::HeaderName::from_static("x-xss-protection"),
            header::HeaderValue::from_static("1; mode=block"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            header::HeaderValue::from_static(
                "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; font-src 'self' https://fonts.gstatic.com; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'"
            ),
        ))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&config.bind_address).await?;
    info!(address = %config.bind_address, "Listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

fn is_loopback_address(addr: &str) -> bool {
    let host = if let Some(bracket_end) = addr.find(']') {
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
