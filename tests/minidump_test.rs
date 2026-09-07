//! End-to-end tests for the crashpad minidump ingest endpoint and the
//! method-agnostic API fallback.
//!
//! `tests/fixtures/windows.dmp` is rust-minidump's `testdata/test.dmp`
//! (MIT licensed): a real Windows XP x86 crash dump with an exception at
//! address 0x45, two threads, and a module list including
//! `c:\test_app.exe` — deterministic input for the whole
//! multipart → stackwalk → worker → SQLite pipeline.

use std::io::Write;
use std::str::FromStr;
use std::time::Duration;

use sqlx::SqlitePool;
use sqlx::sqlite::SqliteConnectOptions;

fn fixture_dump() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/windows.dmp"
    ))
    .unwrap()
}

fn gzip(data: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

/// Multipart body shaped exactly like crashpad's upload: bare `guid`
/// field, sentry-native's msgpack sidecar attachments, and the dump.
fn crashpad_body(
    boundary: &str,
    dump: &[u8],
    sidecar: Option<&[u8]>,
    crumbs: Option<(&[u8], &[u8])>,
) -> Vec<u8> {
    let mut body = Vec::new();
    let mut field = |name: &str, filename: Option<&str>, data: &[u8]| {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        match filename {
            Some(filename) => body.extend_from_slice(
                format!(
                    "Content-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\n\
                     Content-Type: application/octet-stream\r\n\r\n"
                )
                .as_bytes(),
            ),
            None => body.extend_from_slice(
                format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
            ),
        }
        body.extend_from_slice(data);
        body.extend_from_slice(b"\r\n");
    };

    field("guid", None, b"e6a4c8f0-1111-2222-3333-444455556666");
    if let Some(sidecar) = sidecar {
        field("__sentry-event", Some("__sentry-event"), sidecar);
    }
    if let Some((c1, c2)) = crumbs {
        field("__sentry-breadcrumb1", Some("__sentry-breadcrumb1"), c1);
        field("__sentry-breadcrumb2", Some("__sentry-breadcrumb2"), c2);
    }
    field("upload_file_minidump", Some("deadbeef.dmp"), dump);
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_crashpad_multipart_upload_end_to_end() {
    let (base_url, db_path, _handle) = start_test_server().await;
    let client = reqwest::Client::new();
    let (project_id, _slug, public_key) = create_project(&client, &base_url, "crashpad").await;

    // Sidecar event exactly as sentry-native writes it (msgpack).
    let crash_event_id = "aabbccddeeff00112233445566778899";
    let sidecar = rmp_serde::to_vec_named(&serde_json::json!({
        "event_id": crash_event_id,
        "level": "fatal",
        "platform": "native",
        "release": "plezy@2.16.1",
        "environment": "production",
        "sdk": {"name": "sentry.native.flutter", "version": "0.16.3"},
        "user": {"id": "install-1234"},
    }))
    .unwrap();

    // Two rotating breadcrumb files of concatenated msgpack maps.
    let mut crumbs1 = Vec::new();
    rmp_serde::encode::write_named(
        &mut crumbs1,
        &serde_json::json!({"message": "old", "timestamp": "2026-01-01T00:00:01Z"}),
    )
    .unwrap();
    let mut crumbs2 = Vec::new();
    rmp_serde::encode::write_named(
        &mut crumbs2,
        &serde_json::json!({"message": "new", "timestamp": "2026-01-01T00:00:02Z"}),
    )
    .unwrap();

    let boundary = "---MultipartBoundary-test---";
    let body = crashpad_body(
        boundary,
        &fixture_dump(),
        Some(&sidecar),
        Some((&crumbs1, &crumbs2)),
    );

    // Crashpad gzips the whole request body by default.
    let resp = client
        .post(format!(
            "{base_url}/api/{project_id}/minidump/?sentry_key={public_key}&sentry_client=test"
        ))
        .header(
            "Content-Type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .header("Content-Encoding", "gzip")
        .body(gzip(&body))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let response_id = resp.text().await.unwrap();
    assert_eq!(response_id, "aabbccdd-eeff-0011-2233-445566778899");

    let row = poll_event_row(&db_path, project_id, crash_event_id).await;
    let data: serde_json::Value = serde_json::from_str(&row.data).unwrap();

    // Crash synthesis over the sidecar base.
    assert_eq!(data["platform"], "native");
    assert_eq!(data["level"], "fatal");
    assert_eq!(data["release"], "plezy@2.16.1");
    assert_eq!(data["sdk"]["name"], "sentry.native.flutter");

    let exception = &data["exception"]["values"][0];
    let exc_type = exception["type"].as_str().unwrap();
    assert!(
        exc_type.contains("0x45"),
        "crash address in type: {exc_type}"
    );
    assert_eq!(exception["mechanism"]["type"], "minidump");
    assert_eq!(exception["mechanism"]["handled"], false);
    let frames = exception["stacktrace"]["frames"].as_array().unwrap();
    assert!(frames.len() >= 4, "walked frames: {}", frames.len());
    // Caller-first order: the crashing (context) frame is last.
    assert_eq!(frames.last().unwrap()["trust"], "context");
    assert_eq!(frames.last().unwrap()["instruction_addr"], "0x40429e");

    let threads = data["threads"]["values"].as_array().unwrap();
    assert_eq!(threads.len(), 2);
    assert_eq!(threads[0]["crashed"], true);

    let images = data["debug_meta"]["images"].as_array().unwrap();
    assert!(!images.is_empty());
    let app = images
        .iter()
        .find(|img| {
            img["code_file"]
                .as_str()
                .unwrap_or("")
                .ends_with("test_app.exe")
        })
        .expect("test_app.exe module present");
    assert_eq!(app["type"], "pe");
    assert!(app["debug_id"].as_str().is_some());

    // Breadcrumbs merged (newer file's last timestamp wins ordering).
    let crumbs = data["breadcrumbs"]["values"].as_array().unwrap();
    assert_eq!(crumbs.last().unwrap()["message"], "new");

    // Crashpad's bare guid field lands in extra.
    assert_eq!(
        data["extra"]["guid"],
        "e6a4c8f0-1111-2222-3333-444455556666"
    );

    // The dump itself is stored as an event attachment.
    let (name, attachment_type, size) =
        fetch_attachment(&db_path, project_id, crash_event_id).await;
    assert_eq!(name, "deadbeef.dmp");
    assert_eq!(attachment_type.as_deref(), Some("event.minidump"));
    assert_eq!(size as usize, fixture_dump().len());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_raw_minidump_body_upload() {
    let (base_url, db_path, _handle) = start_test_server().await;
    let client = reqwest::Client::new();
    let (project_id, _slug, public_key) = create_project(&client, &base_url, "crashpad-raw").await;

    let resp = client
        .post(format!(
            "{base_url}/api/{project_id}/minidump/?sentry_key={public_key}"
        ))
        .header("Content-Type", "application/octet-stream")
        .body(fixture_dump())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let event_id = resp.text().await.unwrap().replace('-', "");

    let row = poll_event_row(&db_path, project_id, &event_id).await;
    let data: serde_json::Value = serde_json::from_str(&row.data).unwrap();
    assert_eq!(data["platform"], "native");
    assert_eq!(
        data["exception"]["values"][0]["mechanism"]["type"],
        "minidump"
    );
    // No sidecar: SDK is synthesized.
    assert_eq!(data["sdk"]["name"], "minidump.crashpad");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_minidump_rejections() {
    let (base_url, _db_path, _handle) = start_test_server().await;
    let client = reqwest::Client::new();
    let (project_id, _slug, public_key) = create_project(&client, &base_url, "crashpad-bad").await;

    // Invalid magic.
    let resp = client
        .post(format!(
            "{base_url}/api/{project_id}/minidump/?sentry_key={public_key}"
        ))
        .header("Content-Type", "application/octet-stream")
        .body("this is not a minidump".as_bytes().to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);

    // Multipart without the dump part.
    let boundary = "---MultipartBoundary-test---";
    let body = crashpad_body(boundary, &[], None, None);
    let body = {
        // strip the dump part by building a body with only guid
        let mut b = Vec::new();
        b.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        b.extend_from_slice(b"Content-Disposition: form-data; name=\"guid\"\r\n\r\nx\r\n");
        b.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        drop(body);
        b
    };
    let resp = client
        .post(format!(
            "{base_url}/api/{project_id}/minidump/?sentry_key={public_key}"
        ))
        .header(
            "Content-Type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);

    // Missing auth.
    let resp = client
        .post(format!("{base_url}/api/{project_id}/minidump/"))
        .header("Content-Type", "application/octet-stream")
        .body(fixture_dump())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_api_fallback_is_method_agnostic() {
    let (base_url, _db_path, _handle) = start_test_server().await;
    let client = reqwest::Client::new();

    // Unmatched API POST: JSON 404, not 405 (the old `fallback(get(..))`
    // regression that ate crashpad uploads).
    let resp = client
        .post(format!("{base_url}/api/1/nonexistent"))
        .body("x")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["detail"], "not found");

    // SPA shell still served on GET.
    let resp = client
        .get(format!("{base_url}/issues"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert!(
        resp.headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );
}

// --- Helpers (mirroring tests/dart_symbol_map_test.rs) ---

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

async fn fetch_attachment(
    db_path: &str,
    project_id: i64,
    event_id: &str,
) -> (String, Option<String>, i64) {
    let pool = open_reader(db_path).await;
    for _ in 0..50 {
        let row: Option<(String, Option<String>, i64)> = sqlx::query_as(
            "SELECT a.name, a.attachment_type, a.size FROM event_attachments a \
             JOIN events e ON e.id = a.event_id \
             WHERE e.project_id = ? AND e.event_id = ?",
        )
        .bind(project_id)
        .bind(event_id)
        .fetch_optional(&pool)
        .await
        .unwrap();
        if let Some(row) = row {
            return row;
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    panic!("timed out waiting for attachment of {event_id}");
}

// --- Test server harness (copied minimally from ingest_test.rs) ---

use std::sync::atomic::{AtomicU16, Ordering};
static PORT_COUNTER: AtomicU16 = AtomicU16::new(24200);

async fn start_test_server() -> (String, String, tokio::task::JoinHandle<()>) {
    let port = PORT_COUNTER.fetch_add(1, Ordering::Relaxed);
    let db_path = format!("/tmp/bugs-minidump-test-{port}.db");

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
                artifacts_dir: format!("/tmp/bugs-minidump-test-{port}-artifacts"),
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
