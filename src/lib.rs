pub mod api;
pub mod config;
pub mod db;
pub mod ingest;
pub mod models;
pub mod sentry_protocol;
pub mod util;
pub mod worker;

use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Clone)]
pub struct AppState {
    pub db: db::DbPool,
    pub config: Arc<config::Config>,
    pub worker_tx: mpsc::Sender<worker::WorkerMessage>,
    pub rate_limiter: ingest::abuse::RateLimiter,
}
