//! End-to-end tests for Dart symbol map (dartsymbolmap) support:
//! sentry-cli's chunked upload of the Flutter obfuscation map and the
//! ingest-time deobfuscation of exception type/value it enables.

use std::str::FromStr;
use std::time::Duration;

use sha1::{Digest, Sha1};
use sqlx::SqlitePool;
use sqlx::sqlite::SqliteConnectOptions;

const DEBUG_ID: &str = "aabbccdd-eeff-0011-2233-445566778899";

/// The flat pair format sentry-dart-plugin uploads: even index =
/// deobfuscated, odd index = obfuscated, with the marker pair prepended.
const MAP_JSON: &str = r#"["SENTRY_DEBUG_ID_MARKER","aabbccddeeff00112233445566778899","AsyncStateError","aB","SecretClass","cD"]"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_chunk_upload_advertises_dartsymbolmap() {
    let (base_url, _db_path, _handle) = start_test_server().await;
    let client = reqwest::Client::new();

    let options: serde_json::Value = client
        .get(format!(
            "{base_url}/api/0/organizations/default/chunk-upload/"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let accept: Vec<&str> = options["accept"]
        .as_array()
        .expect("accept array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    // sentry-cli requires "dartsymbolmap" to attempt the upload at all,
    // and strips debug_id from assemble requests without "debug_files".
    assert!(accept.contains(&"dartsymbolmap"), "accept: {accept:?}");
    assert!(accept.contains(&"debug_files"), "accept: {accept:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_dart_symbol_map_upload_and_deobfuscation() {
    let (base_url, db_path, _handle) = start_test_server().await;
    let client = reqwest::Client::new();
    let (project_id, slug, public_key) = create_project(&client, &base_url, "dartmap").await;

    // --- Chunk-upload the map and assemble it against the debug_id.
    let checksum = upload_chunk(&client, &base_url, MAP_JSON.as_bytes()).await;
    let response = assemble(
        &client,
        &base_url,
        &slug,
        &checksum,
        serde_json::json!({
            "name": "obfuscation.map.json",
            "debug_id": DEBUG_ID,
            "chunks": [checksum],
        }),
    )
    .await;
    let entry = &response[&checksum];
    assert_eq!(entry["state"], "ok", "assemble response: {response}");
    let dif = &entry["dif"];
    assert_eq!(dif["debugId"], DEBUG_ID);
    assert_eq!(dif["cpuName"], "any");
    assert_eq!(dif["objectName"], "obfuscation.map.json");
    assert_eq!(dif["sha1"], serde_json::json!(checksum));
    assert_eq!(dif["data"]["features"], serde_json::json!(["mapping"]));

    // Re-assembling the same checksum is idempotent (sentry-cli polls).
    let again = assemble(
        &client,
        &base_url,
        &slug,
        &checksum,
        serde_json::json!({
            "name": "obfuscation.map.json",
            "debug_id": DEBUG_ID,
            "chunks": [checksum],
        }),
    )
    .await;
    assert_eq!(again[&checksum]["state"], "ok");

    // --- Obfuscated Flutter event: type and "Instance of '…'" rewritten.
    let event_id = "0123456789abcdef0123456789abcdef";
    let event_json = serde_json::json!({
        "event_id": event_id,
        "level": "error",
        "platform": "other",
        "sdk": {"name": "sentry.dart.flutter", "version": "9.6.0"},
        "exception": {
            "values": [{
                "type": "aB",
                "value": "Bad state: Instance of 'cD' exploded",
            }]
        },
        "debug_meta": {
            "images": [{
                "type": "elf",
                "debug_id": DEBUG_ID,
                "image_addr": "0x0",
            }]
        }
    });
    send_event(
        &client,
        &base_url,
        project_id,
        &public_key,
        event_id,
        &event_json,
    )
    .await;

    let row = poll_event_row(&db_path, project_id, event_id).await;
    let data: serde_json::Value = serde_json::from_str(&row.data).unwrap();
    assert_eq!(
        data["exception"]["values"][0]["type"], "AsyncStateError",
        "exception type should be deobfuscated: {data}"
    );
    assert_eq!(
        data["exception"]["values"][0]["value"],
        "Bad state: Instance of 'SecretClass' exploded",
    );

    // Title (and therefore grouping inputs) uses the deobfuscated names.
    let title = fetch_issue_title(&db_path, project_id).await;
    assert_eq!(
        title,
        "AsyncStateError: Bad state: Instance of 'SecretClass' exploded"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_dart_event_without_map_is_unchanged() {
    let (base_url, db_path, _handle) = start_test_server().await;
    let client = reqwest::Client::new();
    let (project_id, _slug, public_key) = create_project(&client, &base_url, "dartmap-none").await;

    let event_id = "fedcba9876543210fedcba9876543210";
    let event_json = serde_json::json!({
        "event_id": event_id,
        "level": "error",
        "sdk": {"name": "sentry.dart.flutter", "version": "9.6.0"},
        "exception": {"values": [{"type": "aB", "value": "Instance of 'cD'"}]},
        "debug_meta": {"images": [{"type": "elf", "debug_id": DEBUG_ID}]}
    });
    send_event(
        &client,
        &base_url,
        project_id,
        &public_key,
        event_id,
        &event_json,
    )
    .await;

    let row = poll_event_row(&db_path, project_id, event_id).await;
    let data: serde_json::Value = serde_json::from_str(&row.data).unwrap();
    assert_eq!(data["exception"]["values"][0]["type"], "aB");
    assert_eq!(data["exception"]["values"][0]["value"], "Instance of 'cD'");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_dart_symbol_map_assemble_errors() {
    let (base_url, _db_path, _handle) = start_test_server().await;
    let client = reqwest::Client::new();
    let (_project_id, slug, _public_key) = create_project(&client, &base_url, "dartmap-err").await;

    // Missing debug_id — mirrors upstream's error string.
    let checksum = upload_chunk(&client, &base_url, MAP_JSON.as_bytes()).await;
    let response = assemble(
        &client,
        &base_url,
        &slug,
        &checksum,
        serde_json::json!({"name": "obfuscation.map.json", "chunks": [checksum]}),
    )
    .await;
    assert_eq!(response[&checksum]["state"], "error");
    let detail = response[&checksum]["detail"].as_str().unwrap();
    assert!(detail.contains("Missing debug_id"), "detail: {detail}");

    // Odd number of elements.
    let odd = br#"["a","b","c"]"#;
    let checksum = upload_chunk(&client, &base_url, odd).await;
    let response = assemble(
        &client,
        &base_url,
        &slug,
        &checksum,
        serde_json::json!({"name": "m.json", "debug_id": DEBUG_ID, "chunks": [checksum]}),
    )
    .await;
    assert_eq!(response[&checksum]["state"], "error");
    let detail = response[&checksum]["detail"].as_str().unwrap();
    assert!(detail.contains("even number"), "detail: {detail}");

    // Leading '[' but not valid JSON.
    let truncated = br#"[1,2"#;
    let checksum = upload_chunk(&client, &base_url, truncated).await;
    let response = assemble(
        &client,
        &base_url,
        &slug,
        &checksum,
        serde_json::json!({"name": "m.json", "debug_id": DEBUG_ID, "chunks": [checksum]}),
    )
    .await;
    assert_eq!(response[&checksum]["state"], "error");
    let detail = response[&checksum]["detail"].as_str().unwrap();
    assert!(detail.contains("Invalid dartsymbolmap"), "detail: {detail}");

    // Unknown chunks — not_found with the missing hash listed, so
    // sentry-cli knows to upload them.
    let missing_sha = "0000000000000000000000000000000000000001";
    let response = assemble(
        &client,
        &base_url,
        &slug,
        missing_sha,
        serde_json::json!({"name": "m.json", "debug_id": DEBUG_ID, "chunks": [missing_sha]}),
    )
    .await;
    assert_eq!(response[missing_sha]["state"], "not_found");
    assert_eq!(
        response[missing_sha]["missingChunks"],
        serde_json::json!([missing_sha])
    );
}

// --- Helpers ---

async fn create_project(
    client: &reqwest::Client,
    base_url: &str,
    slug: &str,
) -> (i64, String, String) {
    let project: serde_json::Value = client
        .post(format!("{base_url}/api/internal/projects"))
        .json(&serde_json::json!({"name": slug, "slug": slug}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let project_id = project["id"].as_i64().unwrap();

    let keys: Vec<serde_json::Value> = client
        .get(format!(
            "{base_url}/api/internal/projects/{project_id}/keys"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let public_key = keys[0]["public_key"].as_str().unwrap().to_string();
    (project_id, slug.to_string(), public_key)
}

/// Uploads one chunk through the multipart chunk endpoint and returns
/// its SHA1 (which is also the whole-file checksum for one-chunk files).
async fn upload_chunk(client: &reqwest::Client, base_url: &str, bytes: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(bytes);
    let sha = hex::encode(hasher.finalize());

    let boundary = "----bugs-test-dartmap";
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\n\
             Content-Disposition: form-data; name=\"file\"; filename=\"{sha}\"\r\n\
             Content-Type: application/octet-stream\r\n\r\n",
        )
        .as_bytes(),
    );
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let resp = client
        .post(format!(
            "{base_url}/api/0/organizations/default/chunk-upload/"
        ))
        .header(
            "Content-Type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "chunk upload must succeed");
    sha
}

async fn assemble(
    client: &reqwest::Client,
    base_url: &str,
    slug: &str,
    checksum: &str,
    entry: serde_json::Value,
) -> serde_json::Value {
    let resp = client
        .post(format!(
            "{base_url}/api/0/projects/default/{slug}/files/difs/assemble/"
        ))
        .json(&serde_json::json!({checksum: entry}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "assemble must return 200");
    resp.json().await.unwrap()
}

async fn send_event(
    client: &reqwest::Client,
    base_url: &str,
    project_id: i64,
    public_key: &str,
    event_id: &str,
    event_json: &serde_json::Value,
) {
    let event_str = serde_json::to_string(event_json).unwrap();
    let envelope = format!(
        "{{\"event_id\":\"{event_id}\"}}\n{{\"type\":\"event\",\"length\":{}}}\n{event_str}\n",
        event_str.len()
    );
    let resp = client
        .post(format!("{base_url}/api/{project_id}/envelope/"))
        .header("X-Sentry-Auth", format!("Sentry sentry_key={public_key}"))
        .body(envelope)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

struct EventRow {
    data: String,
}

async fn open_reader(db_path: &str) -> SqlitePool {
    let opts = SqliteConnectOptions::from_str(db_path)
        .unwrap()
        .read_only(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);
    SqlitePool::connect_with(opts).await.unwrap()
}

async fn poll_event_row(db_path: &str, project_id: i64, event_id: &str) -> EventRow {
    let pool = open_reader(db_path).await;
    for _ in 0..50 {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT data FROM events WHERE project_id = ? AND event_id = ?")
                .bind(project_id)
                .bind(event_id)
                .fetch_optional(&pool)
                .await
                .unwrap();
        if let Some((data,)) = row {
            return EventRow { data };
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    panic!("timed out waiting for event {event_id} in DB");
}

async fn fetch_issue_title(db_path: &str, project_id: i64) -> String {
    let pool = open_reader(db_path).await;
    let row: (String,) = sqlx::query_as("SELECT title FROM issues WHERE project_id = ?")
        .bind(project_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    row.0
}

// --- Test server harness (copied minimally from ingest_test.rs) ---

use std::sync::atomic::{AtomicU16, Ordering};
static PORT_COUNTER: AtomicU16 = AtomicU16::new(23100);

async fn start_test_server() -> (String, String, tokio::task::JoinHandle<()>) {
    let port = PORT_COUNTER.fetch_add(1, Ordering::Relaxed);
    let db_path = format!("/tmp/bugs-dartmap-test-{port}.db");

    let _ = tokio::fs::remove_file(&db_path).await;
    let _ = tokio::fs::remove_file(format!("{db_path}-wal")).await;
    let _ = tokio::fs::remove_file(format!("{db_path}-shm")).await;

    let bind_addr = format!("127.0.0.1:{port}");
    let base_url = format!("http://{bind_addr}");

    let handle = tokio::spawn({
        let bind_addr = bind_addr.clone();
        let db_path = db_path.clone();
        async move {
            let config = std::sync::Arc::new(bugs::config::Config {
                bind_address: bind_addr,
                database_path: db_path,
                artifacts_dir: format!("/tmp/bugs-dartmap-test-{port}-artifacts"),
                ..Default::default()
            });

            let db = bugs::db::DbPool::init(&config).await.unwrap();
            let (worker_tx, worker_rx) = tokio::sync::mpsc::channel(1000);

            let checkpoint = std::sync::Arc::new(bugs::db::checkpoint::CheckpointManager::new(
                db.writer().clone(),
                10,
            ));

            let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
            bugs::worker::spawn(
                db.clone(),
                config.clone(),
                checkpoint.clone(),
                worker_tx.clone(),
                worker_rx,
                shutdown_rx,
            );

            let state = bugs::AppState {
                db,
                config: config.clone(),
                worker_tx,
                rate_limiter: bugs::ingest::abuse::RateLimiter::new(),
            };

            let app = bugs::api::router(&state)
                .route("/health", axum::routing::get(|| async { "ok" }))
                .with_state(state);

            let listener = tokio::net::TcpListener::bind(&config.bind_address)
                .await
                .unwrap();
            axum::serve(listener, bugs::api::normalized_make_service(app))
                .await
                .unwrap();
        }
    });

    let client = reqwest::Client::new();
    for _ in 0..50 {
        if client
            .get(format!("{base_url}/health"))
            .send()
            .await
            .is_ok()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    (base_url, db_path, handle)
}
