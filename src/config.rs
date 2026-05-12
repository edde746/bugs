use figment::{
    Figment,
    providers::{Env, Format, Serialized, Toml},
};
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
    pub ingest: IngestConfig,
    #[serde(default)]
    pub symbolication: SymbolicationConfig,
    #[serde(default)]
    pub email: EmailConfig,
    #[serde(default)]
    pub uploads: UploadsConfig,
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
            max_items_per_envelope: default_max_items_per_envelope(),
            max_tag_values_per_key: default_max_tag_values_per_key(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolicationConfig {
    /// Max number of parsed source maps held in memory. Acts as a secondary
    /// bound alongside `source_map_cache_bytes_mb` — whichever trips first
    /// triggers eviction.
    #[serde(default = "default_source_map_cache_size")]
    pub source_map_cache_size: usize,
    /// Aggregate byte budget for parsed source maps. A single 2 MB source
    /// map can expand 2–3× when parsed; without a byte cap a handful of
    /// large entries pin hundreds of MB of heap for the life of the
    /// process.
    #[serde(default = "default_source_map_cache_bytes_mb")]
    pub source_map_cache_bytes_mb: usize,
    /// Max number of (release version -> release_files) lookups cached.
    /// Cheap to hold; raising it lets busy multi-release deployments skip
    /// the DB on the symbolication hot path.
    #[serde(default = "default_release_files_cache_size")]
    pub release_files_cache_size: usize,
    /// Max number of mmap'd native SymCache files retained in memory.
    /// Each entry's cost is the mmap length; combined with
    /// `native_symcache_cache_bytes_mb` this caps both count and size.
    #[serde(default = "default_native_symcache_cache_size")]
    pub native_symcache_cache_size: usize,
    /// Aggregate byte budget for retained native SymCache mmap handles.
    /// Counts mmap length, not resident pages — useful ceiling because a
    /// dropped Arc<Mmap> is munmapped and its physical pages freed.
    #[serde(default = "default_native_symcache_cache_bytes_mb")]
    pub native_symcache_cache_bytes_mb: usize,
}

impl Default for SymbolicationConfig {
    fn default() -> Self {
        Self {
            source_map_cache_size: default_source_map_cache_size(),
            source_map_cache_bytes_mb: default_source_map_cache_bytes_mb(),
            release_files_cache_size: default_release_files_cache_size(),
            native_symcache_cache_size: default_native_symcache_cache_size(),
            native_symcache_cache_bytes_mb: default_native_symcache_cache_bytes_mb(),
        }
    }
}

fn default_source_map_cache_size() -> usize {
    64
}
fn default_source_map_cache_bytes_mb() -> usize {
    // Budget is measured in raw input bytes. The parsed `SourceMap` form
    // roughly doubles that in heap (sources + sourcesContent + token
    // tables), so 32 MB of input translates to ~60–80 MB of retained heap.
    32
}
fn default_release_files_cache_size() -> usize {
    32
}
fn default_native_symcache_cache_size() -> usize {
    64
}
fn default_native_symcache_cache_bytes_mb() -> usize {
    256
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadsConfig {
    /// Ceiling for admin-authenticated uploads (dSYMs, release artifacts).
    /// The admin auth gate means we trust the caller; this is a sanity
    /// bound against a corrupt archive or runaway gzip, not an abuse
    /// defense. Applied both to the raw upload and to each entry inside a
    /// zipped dSYM bundle.
    #[serde(default = "default_uploads_max_bytes")]
    pub max_bytes: usize,
    /// Advertised chunk size for the sentry-cli chunked-upload protocol.
    /// 8 MiB matches sentry-cli's canonical fixture and is the size the
    /// client is most-tested at.
    #[serde(default = "default_chunk_size_mib")]
    pub chunk_size_mib: usize,
    /// Max chunks sentry-cli will pack into a single POST. With 8 MiB
    /// chunks and a 32 MiB request cap, the effective batch is ~4 chunks
    /// per request after gzip — enough to amortize TLS overhead.
    #[serde(default = "default_chunks_per_request")]
    pub chunks_per_request: u64,
    /// Max body size per chunk POST. Smaller than chunk_size_mib *
    /// chunks_per_request on purpose: 32 MiB keeps the multipart parser
    /// well within axum's memory comfort zone.
    #[serde(default = "default_max_request_size_mib")]
    pub max_request_size_mib: usize,
    /// Advertised polling budget. We assemble synchronously and return
    /// `ok` on the first call, so the client never actually sleeps.
    #[serde(default = "default_max_wait_secs")]
    pub max_wait_secs: u64,
    /// Advertised client-side upload concurrency. axum + SQLite under
    /// multipart upload is not embarrassingly parallel; 4 is sufficient.
    #[serde(default = "default_chunk_concurrency")]
    pub chunk_concurrency: u8,
}

impl Default for UploadsConfig {
    fn default() -> Self {
        Self {
            max_bytes: default_uploads_max_bytes(),
            chunk_size_mib: default_chunk_size_mib(),
            chunks_per_request: default_chunks_per_request(),
            max_request_size_mib: default_max_request_size_mib(),
            max_wait_secs: default_max_wait_secs(),
            chunk_concurrency: default_chunk_concurrency(),
        }
    }
}

fn default_uploads_max_bytes() -> usize {
    2 * 1024 * 1024 * 1024
}
fn default_chunk_size_mib() -> usize {
    8
}
fn default_chunks_per_request() -> u64 {
    64
}
fn default_max_request_size_mib() -> usize {
    32
}
fn default_max_wait_secs() -> u64 {
    60
}
fn default_chunk_concurrency() -> u8 {
    4
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

fn default_smtp_port() -> u16 {
    587
}
fn default_true() -> bool {
    true
}

fn default_bind_address() -> String {
    "0.0.0.0:9000".to_string()
}
fn default_database_path() -> String {
    "./data/bugs.db".to_string()
}
fn default_artifacts_dir() -> String {
    "./data/artifacts".to_string()
}
fn default_retention_days() -> u32 {
    90
}
fn default_envelope_retention_hours() -> u32 {
    24
}
fn default_worker_threads() -> usize {
    4
}
fn default_synchronous() -> String {
    "NORMAL".to_string()
}
fn default_cache_size_mb() -> u32 {
    64
}
fn default_reader_connections() -> u32 {
    8
}
fn default_checkpoint_interval_batches() -> u32 {
    10
}
fn default_mmap_size_mb() -> u32 {
    256
}
fn default_max_raw_request_bytes() -> usize {
    20 * 1024 * 1024
}
fn default_max_envelope_bytes() -> usize {
    10 * 1024 * 1024
}
fn default_max_event_item_bytes() -> usize {
    1024 * 1024
}
fn default_max_items_per_envelope() -> usize {
    100
}
fn default_max_tag_values_per_key() -> u32 {
    1000
}

impl Config {
    #[allow(clippy::result_large_err)]
    pub fn load() -> Result<Self, figment::Error> {
        Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::file("bugs.toml"))
            .merge(Env::prefixed("BUGS_").split("__"))
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
            auth: AuthConfig::default(),
            sqlite: SqliteConfig::default(),
            ingest: IngestConfig::default(),
            symbolication: SymbolicationConfig::default(),
            email: EmailConfig::default(),
            uploads: UploadsConfig::default(),
        }
    }
}
