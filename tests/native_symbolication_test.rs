//! End-to-end native symbolication test.
//!
//! Compiles a tiny C fixture with `cc` + `dsymutil` at test time, uploads
//! the dSYM, sends a synthetic event whose instruction_addr lands in the
//! fixture's known function, and asserts the worker resolved the frame.
//!
//! Requires `cc` and `dsymutil` on PATH. Gated on macOS.

#![cfg(target_os = "macos")]

use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use sqlx::SqlitePool;
use sqlx::sqlite::SqliteConnectOptions;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_native_symbolication_end_to_end() {
    let fixture = build_fixture();

    let (base_url, db_path, _handle) = start_test_server().await;
    let client = reqwest::Client::new();

    let project: serde_json::Value = client
        .post(format!("{base_url}/api/internal/projects"))
        .json(&serde_json::json!({"name": "Native", "slug": "native"}))
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

    // --- Upload the dSYM
    let dsym_bytes = std::fs::read(&fixture.dsym_path).unwrap();
    let boundary = "----bugs-test-dsym";
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\n\
             Content-Disposition: form-data; name=\"file\"; filename=\"fixture\"\r\n\
             Content-Type: application/octet-stream\r\n\r\n",
        )
        .as_bytes(),
    );
    body.extend_from_slice(&dsym_bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let resp = client
        .post(format!(
            "{base_url}/api/0/projects/default/native/files/dsyms/"
        ))
        .header(
            "Content-Type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201, "dsym upload must succeed");
    let upload_body: serde_json::Value = resp.json().await.unwrap();
    let uploaded = upload_body["uploaded"].as_array().expect("uploaded array");
    assert!(!uploaded.is_empty(), "at least one DIF should be uploaded");
    let returned_id = uploaded[0]["debug_id"].as_str().unwrap().to_string();
    assert_eq!(
        returned_id.to_lowercase(),
        fixture.debug_id.to_lowercase(),
        "uploaded debug_id should match dwarfdump --uuid"
    );

    // --- Synthetic event with matching debug_id
    let event_id = "aaaabbbbccccddddeeeeffff00001111";
    let iaddr = format!("0x{:x}", 0x100000000u64 + fixture.function_offset);
    let event_json = serde_json::json!({
        "event_id": event_id,
        "level": "error",
        "platform": "native",
        "message": "native test",
        "exception": {
            "values": [{
                "type": "SIGSEGV",
                "value": "segfault",
                "stacktrace": {
                    "frames": [{
                        "instruction_addr": iaddr,
                        "image_addr": "0x100000000",
                    }]
                }
            }]
        },
        "debug_meta": {
            "images": [{
                "type": "macho",
                "debug_id": &fixture.debug_id,
                "image_addr": "0x100000000",
                "image_size": 0x4000,
                "image_vmaddr": "0x100000000",
                "code_file": "/Users/test/fixture",
                "arch": "arm64"
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
        row.state.as_deref(),
        Some("ok"),
        "symbolication_state should be 'ok' (frames: {})",
        data["exception"]["values"][0]["stacktrace"]["frames"],
    );
    let function = data["exception"]["values"][0]["stacktrace"]["frames"][0]["function"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(
        function.contains("bugs_native_fixture"),
        "expected resolved function to contain 'bugs_native_fixture', got: {function:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_native_symbolication_missing_map() {
    let (base_url, db_path, _handle) = start_test_server().await;
    let client = reqwest::Client::new();

    let project: serde_json::Value = client
        .post(format!("{base_url}/api/internal/projects"))
        .json(&serde_json::json!({"name": "NativeMiss", "slug": "native-miss"}))
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

    let event_id = "ff00ff00ff00ff00ff00ff00ff001111";
    let event_json = serde_json::json!({
        "event_id": event_id,
        "level": "error",
        "platform": "native",
        "message": "missing debug file",
        "exception": {
            "values": [{
                "type": "SIGSEGV",
                "stacktrace": {
                    "frames": [{
                        "instruction_addr": "0x100000328",
                        "image_addr": "0x100000000",
                    }]
                }
            }]
        },
        "debug_meta": {
            "images": [{
                "type": "macho",
                "debug_id": "deadbeefdeadbeefdeadbeefdeadbeef",
                "image_addr": "0x100000000",
                "image_size": 0x4000,
                "code_file": "/Users/test/missing",
                "arch": "arm64"
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
    assert_eq!(
        row.state.as_deref(),
        Some("missing_map"),
        "event with unknown debug_id should land in 'missing_map'"
    );
}

// --- Helpers -------------------------------------------------------

struct Fixture {
    dsym_path: PathBuf,
    debug_id: String,
    /// Offset of bugs_native_fixture relative to the Mach-O image base 0x100000000.
    function_offset: u64,
}

fn build_fixture() -> Fixture {
    let dir = PathBuf::from("/tmp/bugs-native-fixture");
    std::fs::create_dir_all(&dir).unwrap();

    let src = dir.join("fixture.c");
    std::fs::write(
        &src,
        "int bugs_native_fixture(int x) { return x * 7 + 3; }\n\
         int main(int argc, char **argv) { return bugs_native_fixture(argc); }\n",
    )
    .unwrap();

    let obj = dir.join("fixture.o");
    let bin = dir.join("fixture");
    let _ = std::fs::remove_dir_all(dir.join("fixture.dSYM"));

    run(&[
        "cc",
        "-g",
        "-O0",
        "-c",
        src.to_str().unwrap(),
        "-o",
        obj.to_str().unwrap(),
    ]);
    run(&[
        "cc",
        "-g",
        "-O0",
        "-o",
        bin.to_str().unwrap(),
        obj.to_str().unwrap(),
    ]);
    run(&["dsymutil", bin.to_str().unwrap()]);

    let dsym = dir.join("fixture.dSYM/Contents/Resources/DWARF/fixture");
    assert!(dsym.exists(), "fixture dSYM missing");

    let uuid_out = std::process::Command::new("dwarfdump")
        .args(["--uuid", dsym.to_str().unwrap()])
        .output()
        .unwrap();
    let uuid_text = String::from_utf8_lossy(&uuid_out.stdout);
    let hex: String = uuid_text
        .split_whitespace()
        .find(|tok| tok.len() == 36 && tok.chars().filter(|c| *c == '-').count() == 4)
        .expect("dwarfdump --uuid output not recognized")
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .flat_map(|c| c.to_lowercase())
        .collect();

    let nm_out = std::process::Command::new("nm")
        .args(["-arch", "arm64", bin.to_str().unwrap()])
        .output()
        .unwrap();
    let nm_text = String::from_utf8_lossy(&nm_out.stdout);
    let mut abs_addr: Option<u64> = None;
    for line in nm_text.lines() {
        if line.ends_with("_bugs_native_fixture")
            && let Some(hex_addr) = line.split_whitespace().next()
        {
            abs_addr = u64::from_str_radix(hex_addr, 16).ok();
            break;
        }
    }
    let abs_addr = abs_addr.expect("nm did not find _bugs_native_fixture");
    let function_offset = abs_addr - 0x100000000;

    Fixture {
        dsym_path: dsym,
        debug_id: hex,
        function_offset,
    }
}

fn run(cmd: &[&str]) {
    let status = std::process::Command::new(cmd[0])
        .args(&cmd[1..])
        .status()
        .unwrap_or_else(|e| panic!("spawning {cmd:?} failed: {e}"));
    assert!(status.success(), "{cmd:?} exited {status}");
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
    state: Option<String>,
}

async fn poll_event_row(db_path: &str, project_id: i64, event_id: &str) -> EventRow {
    let opts = SqliteConnectOptions::from_str(db_path)
        .unwrap()
        .read_only(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);
    let pool = SqlitePool::connect_with(opts).await.unwrap();

    for _ in 0..50 {
        let row: Option<(String, Option<String>)> = sqlx::query_as(
            "SELECT data, symbolication_state FROM events WHERE project_id = ? AND event_id = ?",
        )
        .bind(project_id)
        .bind(event_id)
        .fetch_optional(&pool)
        .await
        .unwrap();
        if let Some((data, state)) = row {
            return EventRow { data, state };
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    panic!("timed out waiting for event {event_id} in DB");
}

// --- Test server harness (copied minimally from ingest_test.rs) ---

use std::sync::atomic::{AtomicU16, Ordering};
static PORT_COUNTER: AtomicU16 = AtomicU16::new(21000);

async fn start_test_server() -> (String, String, tokio::task::JoinHandle<()>) {
    let port = PORT_COUNTER.fetch_add(1, Ordering::Relaxed);
    let db_path = format!("/tmp/bugs-native-test-{port}.db");

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
                artifacts_dir: format!("/tmp/bugs-native-test-{port}-artifacts"),
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
