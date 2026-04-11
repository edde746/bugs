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

    assert_eq!(
        resp.status(),
        400,
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

            bugs::worker::spawn(
                db.clone(),
                config.clone(),
                checkpoint.clone(),
                worker_tx.clone(),
                worker_rx,
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
