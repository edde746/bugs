use axum::http::header;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tower_http::set_header::SetResponseHeaderLayer;
use tracing::{info, warn};

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

    // Apply symbolication cache sizing before any envelopes are processed.
    bugs::worker::symbolication::configure_caches(&config.symbolication);
    bugs::worker::native_symbolication::configure_cache(&config.symbolication);

    let (worker_tx, worker_rx) = mpsc::channel::<bugs::worker::WorkerMessage>(10_000);

    // Shutdown coordination: flipped to `true` when SIGINT/SIGTERM arrives.
    // Background tasks select on this alongside their own work loops and
    // exit cleanly instead of being killed mid-operation when the process
    // exits.
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let checkpoint = Arc::new(CheckpointManager::new(
        db.writer().clone(),
        config.sqlite.checkpoint_interval_batches,
    ));
    let checkpoint_handle = checkpoint
        .clone()
        .spawn_quiet_checkpoint_task(shutdown_rx.clone());

    let worker_handles = bugs::worker::spawn(
        db.clone(),
        config.clone(),
        checkpoint.clone(),
        worker_tx.clone(),
        worker_rx,
        shutdown_rx.clone(),
    );

    let retention_handle = bugs::db::retention::spawn_retention_task(
        db.writer().clone(),
        config.retention_days,
        config.envelope_retention_hours,
        shutdown_rx.clone(),
    );

    let state = AppState {
        db: db.clone(),
        config: config.clone(),
        worker_tx: worker_tx.clone(),
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

    axum::serve(listener, bugs::api::normalized_make_service(app))
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            // Notify background tasks so they begin exiting in parallel with
            // the HTTP drain. receiver side of the watch only sees `changed()`
            // fire when the value actually differs from the initial `false`.
            let _ = shutdown_tx.send(true);
        })
        .await?;

    info!("HTTP server drained; waiting for background tasks");

    // Drop the main-scope sender so the worker channel can close once the
    // remaining AppState clones (held by axum) have been dropped above.
    drop(worker_tx);

    // Bound the wait so we still exit within Docker's 10s SIGKILL grace
    // period even if something is stuck.
    let tasks = async {
        for h in worker_handles {
            let _ = h.await;
        }
        let _ = checkpoint_handle.await;
        let _ = retention_handle.await;
    };
    if tokio::time::timeout(Duration::from_secs(8), tasks)
        .await
        .is_err()
    {
        warn!("Background tasks did not finish within 8s; proceeding to shutdown");
    }

    // Final truncate checkpoint now that no more writes are in flight.
    checkpoint.truncate_checkpoint().await;

    // Close sqlx pools so pending writes flush before process exit.
    db.writer().close().await;
    db.reader().close().await;

    info!("Shutdown complete");
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
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install CTRL+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => info!("SIGINT received; shutting down"),
        _ = terminate => info!("SIGTERM received; shutting down"),
    }
}
