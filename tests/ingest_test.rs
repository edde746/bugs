//! Integration tests for the ingest pipeline.
//! Run with: cargo test --test ingest_test
//!
//! These tests start a full server, send events via HTTP, and verify
//! they are processed correctly.

use std::time::Duration;

#[tokio::test]
async fn test_envelope_ingest_and_processing() {
    let (base_url, _handle) = start_test_server().await;

    // Create a project
    let client = reqwest::Client::new();
    let project: serde_json::Value = client
        .post(format!("{base_url}/api/internal/projects"))
        .json(&serde_json::json!({"name": "Test", "slug": "test", "platform": "javascript"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let project_id = project["id"].as_i64().unwrap();

    // Get DSN key
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
    let public_key = keys[0]["public_key"].as_str().unwrap();

    // Send envelope
    let event_id = "a1b2c3d4e5f60000a1b2c3d4e5f60001";
    let event_json = serde_json::json!({
        "event_id": event_id,
        "level": "error",
        "platform": "javascript",
        "message": "Test error message",
        "exception": {
            "values": [{
                "type": "TypeError",
                "value": "Cannot read properties of undefined",
                "stacktrace": {
                    "frames": [{
                        "filename": "app.js",
                        "function": "handleClick",
                        "lineno": 42,
                        "in_app": true
                    }]
                }
            }]
        },
        "environment": "test",
        "tags": {"browser": "Chrome"}
    });

    let event_str = serde_json::to_string(&event_json).unwrap();
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
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["id"].as_str().unwrap(), event_id);

    // Wait for worker processing
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Verify issue was created
    let issues: serde_json::Value = client
        .get(format!("{base_url}/api/internal/projects/test/issues"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let issue_list = issues["issues"].as_array().unwrap();
    assert_eq!(issue_list.len(), 1);
    assert!(
        issue_list[0]["title"]
            .as_str()
            .unwrap()
            .contains("TypeError")
    );
    assert_eq!(issue_list[0]["event_count"].as_i64().unwrap(), 1);

    // Verify event was stored
    let issue_id = issue_list[0]["id"].as_i64().unwrap();
    let events_resp: serde_json::Value = client
        .get(format!("{base_url}/api/internal/issues/{issue_id}/events"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let events = events_resp["events"].as_array().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["event_id"].as_str().unwrap(), event_id);
}

#[tokio::test]
async fn test_ingest_rejects_mismatched_project_id() {
    let (base_url, _handle) = start_test_server().await;
    let client = reqwest::Client::new();

    // Create a project (gets id=1)
    let project: serde_json::Value = client
        .post(format!("{base_url}/api/internal/projects"))
        .json(&serde_json::json!({"name": "Mismatch Test", "slug": "mismatch-test"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let _project_id = project["id"].as_i64().unwrap();

    // Get DSN key for this project
    let keys: Vec<serde_json::Value> = client
        .get(format!(
            "{base_url}/api/internal/projects/{_project_id}/keys"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let public_key = keys[0]["public_key"].as_str().unwrap();

    // Send envelope to a DIFFERENT project_id (9999) using the valid key
    let event_id = "deadbeef00000000deadbeef00000001";
    let event =
        serde_json::json!({"event_id": event_id, "level": "error", "message": "mismatch test"});
    let event_str = serde_json::to_string(&event).unwrap();
    let envelope = format!(
        "{{\"event_id\":\"{event_id}\"}}\n{{\"type\":\"event\",\"length\":{}}}\n{event_str}\n",
        event_str.len()
    );

    let resp = client
        .post(format!("{base_url}/api/9999/envelope/"))
        .header("X-Sentry-Auth", format!("Sentry sentry_key={public_key}"))
        .body(envelope)
        .send()
        .await
        .unwrap();

    // The mismatch path now returns 401 to match the invalid-key path —
    // we don't want to tell attackers whether a key exists and belongs
    // to a different project vs. doesn't exist at all.
    assert_eq!(
        resp.status(),
        401,
        "Mismatched project_id should be rejected"
    );
}

#[tokio::test]
async fn test_grouping_same_error_same_issue() {
    let (base_url, _handle) = start_test_server().await;
    let client = reqwest::Client::new();

    // Create project + get key
    let project: serde_json::Value = client
        .post(format!("{base_url}/api/internal/projects"))
        .json(&serde_json::json!({"name": "Group Test", "slug": "group-test", "platform": "javascript"}))
        .send().await.unwrap()
        .json().await.unwrap();
    let pid = project["id"].as_i64().unwrap();
    let keys: Vec<serde_json::Value> = client
        .get(format!("{base_url}/api/internal/projects/{pid}/keys"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let key = keys[0]["public_key"].as_str().unwrap();

    // Send 3 identical errors
    for i in 0..3 {
        let eid = format!("00000000000000000000000000000{:03}", i);
        let event = serde_json::json!({
            "event_id": eid,
            "level": "error",
            "exception": {"values": [{"type": "RangeError", "value": "out of bounds",
                "stacktrace": {"frames": [{"filename": "lib.js", "function": "process", "lineno": 10, "in_app": true}]}}]}
        });
        let event_str = serde_json::to_string(&event).unwrap();
        let envelope = format!(
            "{{\"event_id\":\"{eid}\"}}\n{{\"type\":\"event\",\"length\":{}}}\n{event_str}\n",
            event_str.len()
        );
        client
            .post(format!("{base_url}/api/{pid}/envelope/"))
            .header("X-Sentry-Auth", format!("Sentry sentry_key={key}"))
            .body(envelope)
            .send()
            .await
            .unwrap();
    }

    tokio::time::sleep(Duration::from_secs(3)).await;

    // Should be 1 issue with count=3
    let issues: serde_json::Value = client
        .get(format!(
            "{base_url}/api/internal/projects/group-test/issues"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let list = issues["issues"].as_array().unwrap();
    assert_eq!(list.len(), 1, "Same errors should group into 1 issue");
    assert!(list[0]["event_count"].as_i64().unwrap() >= 3);

    // Send a different error
    let event = serde_json::json!({
        "event_id": "00000000000000000000000000000099",
        "level": "warning",
        "message": "Something else entirely"
    });
    let event_str = serde_json::to_string(&event).unwrap();
    let envelope = format!(
        "{{\"event_id\":\"00000000000000000000000000000099\"}}\n{{\"type\":\"event\",\"length\":{}}}\n{event_str}\n",
        event_str.len()
    );
    client
        .post(format!("{base_url}/api/{pid}/envelope/"))
        .header("X-Sentry-Auth", format!("Sentry sentry_key={key}"))
        .body(envelope)
        .send()
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_secs(2)).await;

    let issues: serde_json::Value = client
        .get(format!(
            "{base_url}/api/internal/projects/group-test/issues"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(
        issues["issues"].as_array().unwrap().len(),
        2,
        "Different error should create new issue"
    );
}

#[tokio::test]
async fn test_search() {
    let (base_url, _handle) = start_test_server().await;
    let client = reqwest::Client::new();

    let project: serde_json::Value = client
        .post(format!("{base_url}/api/internal/projects"))
        .json(&serde_json::json!({"name": "Search Test", "slug": "search-test"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let pid = project["id"].as_i64().unwrap();
    let keys: Vec<serde_json::Value> = client
        .get(format!("{base_url}/api/internal/projects/{pid}/keys"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let key = keys[0]["public_key"].as_str().unwrap();

    let event = serde_json::json!({
        "event_id": "aaaa0000bbbb1111cccc2222dddd4444",
        "level": "error",
        "message": "UniqueSearchableMessage12345"
    });
    let event_str = serde_json::to_string(&event).unwrap();
    let envelope = format!(
        "{{\"event_id\":\"aaaa0000bbbb1111cccc2222dddd4444\"}}\n{{\"type\":\"event\",\"length\":{}}}\n{event_str}\n",
        event_str.len()
    );
    client
        .post(format!("{base_url}/api/{pid}/envelope/"))
        .header("X-Sentry-Auth", format!("Sentry sentry_key={key}"))
        .body(envelope)
        .send()
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_secs(2)).await;

    let search_resp: serde_json::Value = client
        .get(format!(
            "{base_url}/api/internal/search?q=UniqueSearchableMessage12345&project={pid}"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let results = search_resp["results"].as_array().unwrap();
    assert!(!results.is_empty(), "FTS5 search should find the event");
}

// --- Security & integrity regression tests (phase 1/2 audit) ---

#[tokio::test]
async fn test_invalid_dsn_returns_401() {
    let (base_url, _handle) = start_test_server().await;
    let client = reqwest::Client::new();

    // Create a project so there's at least one valid key in the DB.
    let project: serde_json::Value = client
        .post(format!("{base_url}/api/internal/projects"))
        .json(&serde_json::json!({"name": "Auth", "slug": "auth"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let pid = project["id"].as_i64().unwrap();

    let event_id = "11111111111111111111111111111111";
    let event = serde_json::json!({"event_id": event_id, "level": "error"});
    let event_str = serde_json::to_string(&event).unwrap();
    let envelope = format!(
        "{{\"event_id\":\"{event_id}\"}}\n{{\"type\":\"event\",\"length\":{}}}\n{event_str}\n",
        event_str.len()
    );

    let resp = client
        .post(format!("{base_url}/api/{pid}/envelope/"))
        .header("X-Sentry-Auth", "Sentry sentry_key=nothingrealhere")
        .body(envelope)
        .send()
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        401,
        "Unknown sentry_key must be rejected with 401"
    );
}

#[tokio::test]
async fn test_gzip_decompression_bomb_is_capped() {
    let (base_url, _handle) = start_test_server().await;
    let client = reqwest::Client::new();

    // Need a valid key just to reach the decompression stage.
    let project: serde_json::Value = client
        .post(format!("{base_url}/api/internal/projects"))
        .json(&serde_json::json!({"name": "Bomb", "slug": "bomb"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let pid = project["id"].as_i64().unwrap();
    let keys: Vec<serde_json::Value> = client
        .get(format!("{base_url}/api/internal/projects/{pid}/keys"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let key = keys[0]["public_key"].as_str().unwrap();

    // Build a highly-compressible payload whose decompressed size is
    // well over max_envelope_bytes (10 MB default). 64 MB of zeros
    // gzips to a few KB.
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::Write;

    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(&vec![0u8; 64 * 1024 * 1024]).unwrap();
    let gzipped = encoder.finish().unwrap();
    assert!(
        gzipped.len() < 1024 * 1024,
        "sanity: compressed bomb should be tiny"
    );

    let resp = client
        .post(format!("{base_url}/api/{pid}/envelope/"))
        .header("X-Sentry-Auth", format!("Sentry sentry_key={key}"))
        .header("Content-Encoding", "gzip")
        .body(gzipped)
        .send()
        .await
        .unwrap();

    // 413 is the fix's primary response; 400 is acceptable if the
    // decoder rejects the stream early for some other reason, but 200
    // would mean the cap didn't fire.
    let status = resp.status().as_u16();
    assert!(
        status == 413 || status == 400,
        "expected 413/400 from capped decompress, got {status}"
    );
}

#[tokio::test]
async fn test_envelope_dedup_same_event_id() {
    let (base_url, _handle) = start_test_server().await;
    let client = reqwest::Client::new();

    let project: serde_json::Value = client
        .post(format!("{base_url}/api/internal/projects"))
        .json(&serde_json::json!({"name": "Dedup", "slug": "dedup"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let pid = project["id"].as_i64().unwrap();
    let keys: Vec<serde_json::Value> = client
        .get(format!("{base_url}/api/internal/projects/{pid}/keys"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let key = keys[0]["public_key"].as_str().unwrap();

    let event_id = "cafebabecafebabecafebabecafebabe";
    let event = serde_json::json!({
        "event_id": event_id,
        "level": "error",
        "message": "dedup-test",
    });
    let event_str = serde_json::to_string(&event).unwrap();
    let envelope = format!(
        "{{\"event_id\":\"{event_id}\"}}\n{{\"type\":\"event\",\"length\":{}}}\n{event_str}\n",
        event_str.len()
    );

    // Send the same envelope twice back to back.
    for _ in 0..2 {
        let resp = client
            .post(format!("{base_url}/api/{pid}/envelope/"))
            .header("X-Sentry-Auth", format!("Sentry sentry_key={key}"))
            .body(envelope.clone())
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
    }

    tokio::time::sleep(Duration::from_secs(3)).await;

    let issues: serde_json::Value = client
        .get(format!("{base_url}/api/internal/projects/dedup/issues"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let list = issues["issues"].as_array().unwrap();
    assert_eq!(
        list.len(),
        1,
        "duplicate event should not create a 2nd issue"
    );
    assert_eq!(
        list[0]["event_count"].as_i64().unwrap(),
        1,
        "duplicate event_id should be dropped via INSERT OR IGNORE — event_count must stay at 1"
    );
}

#[tokio::test]
async fn test_artifact_name_path_traversal_rejected() {
    let (base_url, _handle) = start_test_server().await;
    let client = reqwest::Client::new();

    let project: serde_json::Value = client
        .post(format!("{base_url}/api/internal/projects"))
        .json(&serde_json::json!({"name": "Traversal", "slug": "traversal"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let _pid = project["id"].as_i64().unwrap();

    // Hand-craft a multipart/form-data body with name=".." so we don't
    // depend on reqwest's `multipart` feature (not enabled in this
    // crate). The server must reject the literal ".." artifact name.
    let boundary = "----bugs-test-boundary-abc";
    let body_str = format!(
        "--{b}\r\n\
         Content-Disposition: form-data; name=\"name\"\r\n\r\n\
         ..\r\n\
         --{b}\r\n\
         Content-Disposition: form-data; name=\"dist\"\r\n\r\n\
         \r\n\
         --{b}\r\n\
         Content-Disposition: form-data; name=\"file\"; filename=\"map.js\"\r\n\
         Content-Type: application/octet-stream\r\n\r\n\
         console.log(1);\r\n\
         --{b}--\r\n",
        b = boundary
    );

    let resp = client
        .post(format!(
            "{base_url}/api/0/projects/default/traversal/releases/v1/files/"
        ))
        .header(
            "Content-Type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(body_str)
        .send()
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        400,
        "artifact name '..' must be rejected as BAD_REQUEST"
    );
}

// --- Test harness ---

use std::sync::atomic::{AtomicU16, Ordering};
static PORT_COUNTER: AtomicU16 = AtomicU16::new(19000);

async fn start_test_server() -> (String, tokio::task::JoinHandle<()>) {
    let port = PORT_COUNTER.fetch_add(1, Ordering::Relaxed);
    let db_path = format!("/tmp/bugs-test-{port}.db");

    // Clean up any previous test DB
    let _ = tokio::fs::remove_file(&db_path).await;
    let _ = tokio::fs::remove_file(format!("{db_path}-wal")).await;
    let _ = tokio::fs::remove_file(format!("{db_path}-shm")).await;

    let bind_addr = format!("127.0.0.1:{port}");
    let base_url = format!("http://{bind_addr}");

    let handle = tokio::spawn({
        let bind_addr = bind_addr.clone();
        let db_path = db_path.clone();
        async move {
            let config = bugs::config::Config {
                bind_address: bind_addr,
                database_path: db_path,
                artifacts_dir: "/tmp/bugs-test-artifacts".to_string(),
                ..Default::default()
            };
            let config = std::sync::Arc::new(config);

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
            axum::serve(listener, app).await.unwrap();
        }
    });

    // Wait for server to be ready
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

    (base_url, handle)
}
