use std::sync::Arc;
use clap::Parser;
use tokio::sync::mpsc;
use tracing::info;

use bugs::config::Config;
use bugs::db::DbPool;
use bugs::db::checkpoint::CheckpointManager;
use bugs::AppState;

#[derive(clap::Parser)]
#[command(name = "bugs", about = "Lightweight Sentry-compatible error tracker")]
struct Cli {
    /// Allow running without an admin token on non-loopback addresses.
    #[arg(long)]
    insecure_open_admin: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "bugs=info,tower_http=info".parse().unwrap())
        )
        .init();

    let config = Config::load().unwrap_or_else(|e| {
        eprintln!("Config error: {e}. Using defaults.");
        Config::default()
    });
    let config = Arc::new(config);

    // Safety check: no admin token + non-loopback -> require flag
    if config.auth.admin_token.is_empty() && !is_loopback_address(&config.bind_address) {
        if !cli.insecure_open_admin {
            eprintln!(
                "ERROR: No admin_token configured and bind address '{}' is not loopback.\n\
                 Either set auth.admin_token, bind to 127.0.0.1, or pass --insecure-open-admin",
                config.bind_address
            );
            std::process::exit(1);
        }
    }

    info!(bind = %config.bind_address, db = %config.database_path, "Starting Bugs");

    let db = DbPool::init(&config).await?;

    let (worker_tx, worker_rx) = mpsc::channel::<bugs::worker::WorkerMessage>(10_000);

    let checkpoint = Arc::new(CheckpointManager::new(
        db.writer().clone(),
        config.sqlite.checkpoint_interval_batches,
    ));
    checkpoint.clone().spawn_quiet_checkpoint_task();

    bugs::worker::spawn(db.clone(), config.clone(), checkpoint.clone(), worker_rx);

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

    let admin_token = config.auth.admin_token.clone();
    let app = bugs::api::router(&state)
        .route("/health", axum::routing::get(|| async { "ok" }))
        .layer(axum::middleware::from_fn(move |request: axum::extract::Request, next: axum::middleware::Next| {
            let token = admin_token.clone();
            async move {
                let path = request.uri().path().to_string();
                let needs_auth = path.starts_with("/api/internal") || path.starts_with("/api/0");
                if !needs_auth || token.is_empty() {
                    return Ok::<_, axum::http::StatusCode>(next.run(request).await);
                }
                let auth_ok = request.headers()
                    .get(axum::http::header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|h| h.strip_prefix("Bearer "))
                    .is_some_and(|bearer| bearer.trim() == token);
                if auth_ok { Ok(next.run(request).await) } else { Err(axum::http::StatusCode::UNAUTHORIZED) }
            }
        }))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .layer(tower_http::compression::CompressionLayer::new())
        .layer(tower_http::cors::CorsLayer::permissive())
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
    tokio::signal::ctrl_c().await.expect("Failed to install CTRL+C handler");
    info!("Shutting down");
}
