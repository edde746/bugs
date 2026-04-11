use figment::{Figment, providers::{Env, Format, Toml, Serialized}};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_bind_address")]
    pub bind_address: String,
    #[serde(default = "default_database_path")]
    pub database_path: String,
    #[serde(default = "default_artifacts_dir")]
    pub artifacts_dir: String,
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,
    #[serde(default = "default_envelope_retention_hours")]
    pub envelope_retention_hours: u32,
    #[serde(default = "default_worker_threads")]
    pub worker_threads: usize,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub sqlite: SqliteConfig,
    #[serde(default)]
    pub public_url: Option<String>,
    #[serde(default)]
    pub ingest: IngestConfig,
    #[serde(default)]
    pub email: EmailConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthConfig {
    #[serde(default)]
    pub admin_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqliteConfig {
    #[serde(default = "default_synchronous")]
    pub synchronous: String,
    #[serde(default = "default_cache_size_mb")]
    pub cache_size_mb: u32,
    #[serde(default = "default_reader_connections")]
    pub reader_connections: u32,
    #[serde(default = "default_checkpoint_interval_batches")]
    pub checkpoint_interval_batches: u32,
    #[serde(default = "default_mmap_size_mb")]
    pub mmap_size_mb: u32,
}

impl Default for SqliteConfig {
    fn default() -> Self {
        Self {
            synchronous: default_synchronous(),
            cache_size_mb: default_cache_size_mb(),
            reader_connections: default_reader_connections(),
            checkpoint_interval_batches: default_checkpoint_interval_batches(),
            mmap_size_mb: default_mmap_size_mb(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestConfig {
    #[serde(default = "default_max_raw_request_bytes")]
    pub max_raw_request_bytes: usize,
    #[serde(default = "default_max_envelope_bytes")]
    pub max_envelope_bytes: usize,
    #[serde(default = "default_max_event_item_bytes")]
    pub max_event_item_bytes: usize,
    #[serde(default = "default_max_attachment_bytes")]
    pub max_attachment_bytes: usize,
    #[serde(default = "default_max_items_per_envelope")]
    pub max_items_per_envelope: usize,
    #[serde(default = "default_max_tag_values_per_key")]
    pub max_tag_values_per_key: u32,
}

impl Default for IngestConfig {
    fn default() -> Self {
        Self {
            max_raw_request_bytes: default_max_raw_request_bytes(),
            max_envelope_bytes: default_max_envelope_bytes(),
            max_event_item_bytes: default_max_event_item_bytes(),
            max_attachment_bytes: default_max_attachment_bytes(),
            max_items_per_envelope: default_max_items_per_envelope(),
            max_tag_values_per_key: default_max_tag_values_per_key(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EmailConfig {
    /// SMTP host (e.g., "smtp.gmail.com"). Empty = email disabled.
    #[serde(default)]
    pub smtp_host: String,
    #[serde(default = "default_smtp_port")]
    pub smtp_port: u16,
    #[serde(default)]
    pub smtp_username: String,
    #[serde(default)]
    pub smtp_password: String,
    /// From address for alert emails
    #[serde(default)]
    pub from_address: String,
    /// Use STARTTLS (default true)
    #[serde(default = "default_true")]
    pub smtp_tls: bool,
}

fn default_smtp_port() -> u16 { 587 }
fn default_true() -> bool { true }

fn default_bind_address() -> String { "0.0.0.0:9000".to_string() }
fn default_database_path() -> String { "./data/bugs.db".to_string() }
fn default_artifacts_dir() -> String { "./data/artifacts".to_string() }
fn default_retention_days() -> u32 { 90 }
fn default_envelope_retention_hours() -> u32 { 24 }
fn default_worker_threads() -> usize { 4 }
fn default_synchronous() -> String { "NORMAL".to_string() }
fn default_cache_size_mb() -> u32 { 64 }
fn default_reader_connections() -> u32 { 8 }
fn default_checkpoint_interval_batches() -> u32 { 10 }
fn default_mmap_size_mb() -> u32 { 256 }
fn default_max_raw_request_bytes() -> usize { 20 * 1024 * 1024 }
fn default_max_envelope_bytes() -> usize { 10 * 1024 * 1024 }
fn default_max_event_item_bytes() -> usize { 1024 * 1024 }
fn default_max_attachment_bytes() -> usize { 10 * 1024 * 1024 }
fn default_max_items_per_envelope() -> usize { 100 }
fn default_max_tag_values_per_key() -> u32 { 1000 }

impl Config {
    #[allow(clippy::result_large_err)]
    pub fn load() -> Result<Self, figment::Error> {
        Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::file("bugs.toml"))
            .merge(Env::prefixed("BUGS_").split("_"))
            .extract()
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind_address: default_bind_address(),
            database_path: default_database_path(),
            artifacts_dir: default_artifacts_dir(),
            retention_days: default_retention_days(),
            envelope_retention_hours: default_envelope_retention_hours(),
            worker_threads: default_worker_threads(),
            public_url: None,
            auth: AuthConfig::default(),
            sqlite: SqliteConfig::default(),
            ingest: IngestConfig::default(),
            email: EmailConfig::default(),
        }
    }
}
