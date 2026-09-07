//! End-to-end native symbolication test.
//!
//! Compiles a tiny C fixture with `cc` + `dsymutil` at test time, uploads
//! the dSYM, sends a synthetic event whose instruction_addr lands in the
//! fixture's known function, and asserts the worker resolved the frame.
//!
//! Requires `cc` and `dsymutil` on PATH. Gated on macOS.

#![cfg(target_os = "macos")]

use std::io::{Cursor, Write};
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_dsym_zip_upload_extracts_dif() {
    let fixture = build_fixture();

    let (base_url, _db_path, _handle) = start_test_server().await;
    let client = reqwest::Client::new();
    create_project(&client, &base_url, "NativeZip", "native-zip").await;

    let zip_bytes = zip_fixture_dsym(&fixture);
    let boundary = "----bugs-test-dsym-zip";
    let body = dsym_upload_body(boundary, "fixture.zip", &zip_bytes);

    let resp = client
        .post(format!(
            "{base_url}/api/0/projects/default/native-zip/files/dsyms/"
        ))
        .header(
            "Content-Type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201, "zipped dsym upload must succeed");
    let upload_body: serde_json::Value = resp.json().await.unwrap();
    let uploaded = upload_body["uploaded"].as_array().expect("uploaded array");
    assert!(
        uploaded.iter().any(|entry| entry["debug_id"]
            .as_str()
            .is_some_and(|debug_id| debug_id.eq_ignore_ascii_case(&fixture.debug_id))),
        "uploaded zip should contain fixture debug_id; response: {upload_body}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_dsym_upload_over_limit_returns_413() {
    let (base_url, _db_path, _handle) = start_test_server_with_upload_limit(Some(4)).await;
    let client = reqwest::Client::new();
    create_project(&client, &base_url, "NativeLimit", "native-limit").await;

    let boundary = "----bugs-test-dsym-limit";
    let body = dsym_upload_body(boundary, "too-large", b"12345");
    let resp = client
        .post(format!(
            "{base_url}/api/0/projects/default/native-limit/files/dsyms/"
        ))
        .header(
            "Content-Type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(body)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 413, "oversized dsym upload must be rejected");
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("Upload too large"),
        "expected explicit upload limit error, got: {body}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_rich_dsym_survives_stripped_uploads_and_project_isolation() {
    let fixture = build_fixture();
    let rich = std::fs::read(&fixture.dsym_path).unwrap();
    let stripped = std::fs::read(&fixture.bin_path).unwrap();
    let (base_url, db_path, handle) = start_test_server().await;
    let client = reqwest::Client::new();
    let pool = SqlitePool::connect_with(
        SqliteConnectOptions::from_str(&db_path)
            .unwrap()
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal),
    )
    .await
    .unwrap();

    let rich_first = create_project(&client, &base_url, "Rich First", "rich-first").await;
    upload_fixture(&client, &base_url, "rich-first", &rich).await;
    let original_path = assert_canonical_source(&pool, rich_first, &fixture).await;
    let original_bytes = std::fs::read(&original_path).unwrap();

    // Emulate a migrated production row: an older mutable path, no measured
    // quality or digest, and a historically nullable architecture.
    let legacy_path = fixture._dir.path().join("legacy.symc");
    std::fs::write(&legacy_path, &original_bytes).unwrap();
    sqlx::query(
        "UPDATE artifact_debug_ids SET file_path = ?, arch = '', quality_lines = NULL, \
         quality_files = NULL, quality_functions = NULL, content_hash = NULL WHERE project_id = ?",
    )
    .bind(legacy_path.to_str().unwrap())
    .bind(rich_first)
    .execute(&pool)
    .await
    .unwrap();
    upload_fixture(&client, &base_url, "rich-first", &stripped).await;
    assert_eq!(
        assert_canonical_source(&pool, rich_first, &fixture).await,
        legacy_path.to_str().unwrap(),
        "an unmeasured historical dSYM must not be downgraded",
    );

    let stripped_first =
        create_project(&client, &base_url, "Stripped First", "stripped-first").await;
    upload_fixture(&client, &base_url, "stripped-first", &stripped).await;
    let stripped_path: String = sqlx::query_scalar(
        "SELECT file_path FROM artifact_debug_ids WHERE project_id = ? AND kind = 'native'",
    )
    .bind(stripped_first)
    .fetch_one(&pool)
    .await
    .unwrap();
    let stripped_cache = std::fs::read(&stripped_path).unwrap();
    let stripped_symcache = symbolic::symcache::SymCache::parse(&stripped_cache).unwrap();
    let stripped_location = stripped_symcache
        .lookup(fixture.function_offset)
        .next()
        .unwrap();
    assert_eq!(
        stripped_location.line(),
        0,
        "fixture binary must have no DWARF lines"
    );
    let release_id: i64 =
        sqlx::query_scalar("INSERT INTO releases(version) VALUES ('fixture-release') RETURNING id")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("INSERT INTO release_projects(release_id, project_id) VALUES (?, ?)")
        .bind(release_id)
        .bind(stripped_first)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE artifact_debug_ids SET release_id = ? WHERE project_id = ?")
        .bind(release_id)
        .bind(stripped_first)
        .execute(&pool)
        .await
        .unwrap();
    upload_fixture(&client, &base_url, "stripped-first", &rich).await;
    assert_canonical_source(&pool, stripped_first, &fixture).await;
    let preserved_release: i64 = sqlx::query_scalar(
        "SELECT release_id FROM artifact_debug_ids WHERE project_id = ? AND kind = 'native'",
    )
    .bind(stripped_first)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        preserved_release, release_id,
        "richer upload must retain the original release"
    );
    assert_eq!(
        std::fs::read(&stripped_path).unwrap(),
        stripped_cache,
        "upgrading the canonical pointer must not mutate an existing payload",
    );

    let concurrent = create_project(&client, &base_url, "Concurrent", "concurrent").await;
    tokio::join!(
        upload_fixture(&client, &base_url, "concurrent", &rich),
        upload_fixture(&client, &base_url, "concurrent", &stripped),
        upload_fixture(&client, &base_url, "concurrent", &rich),
        upload_fixture(&client, &base_url, "concurrent", &stripped),
    );
    assert_canonical_source(&pool, concurrent, &fixture).await;

    let isolated = create_project(&client, &base_url, "Isolated", "isolated").await;
    upload_fixture(&client, &base_url, "isolated", &stripped).await;
    let isolated_path: String = sqlx::query_scalar(
        "SELECT file_path FROM artifact_debug_ids WHERE project_id = ? AND kind = 'native'",
    )
    .bind(isolated)
    .fetch_one(&pool)
    .await
    .unwrap();
    // The same debug ID can have another architecture whose payload is absent.
    // Exact architecture lookup must still use the rich arm64 cache.
    sqlx::query(
        "INSERT INTO artifact_debug_ids(project_id, debug_id, arch, kind, file_path) \
         VALUES (?, ?, 'x86_64', 'native', ?)",
    )
    .bind(concurrent)
    .bind(&fixture.debug_id)
    .bind(
        fixture
            ._dir
            .path()
            .join("absent-x86.symc")
            .to_str()
            .unwrap(),
    )
    .execute(&pool)
    .await
    .unwrap();
    let keys: Vec<serde_json::Value> = client
        .get(format!(
            "{base_url}/api/internal/projects/{concurrent}/keys"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let event_id = "1234567890abcdef1234567890abcdef";
    let event = serde_json::json!({
        "event_id": event_id, "platform": "native", "level": "error",
        "exception": {"values": [{"type": "SIGSEGV", "stacktrace": {"frames": [{
            "instruction_addr": format!("0x{:x}", 0x100000000u64 + fixture.function_offset)
        }]}}]},
        "debug_meta": {"images": [{
            "debug_id": fixture.debug_id, "arch": "arm64", "type": "macho",
            "image_addr": "0x100000000", "image_size": 0x4000,
            "code_file": "/fixture"
        }]}
    });
    send_event(
        &client,
        &base_url,
        concurrent,
        keys[0]["public_key"].as_str().unwrap(),
        event_id,
        &event,
    )
    .await;
    let row = poll_event_row(&db_path, concurrent, event_id).await;
    let data: serde_json::Value = serde_json::from_str(&row.data).unwrap();
    let frame = &data["exception"]["values"][0]["stacktrace"]["frames"][0];
    assert!(
        frame["filename"].as_str().unwrap().ends_with("fixture.c"),
        "{frame}"
    );
    assert_eq!(frame["lineno"], 1, "{frame}");
    let isolated_bytes = std::fs::read(isolated_path).unwrap();
    let isolated_cache = symbolic::symcache::SymCache::parse(&isolated_bytes).unwrap();
    assert_eq!(
        isolated_cache
            .lookup(fixture.function_offset)
            .next()
            .unwrap()
            .line(),
        0
    );
    assert_canonical_source(&pool, rich_first, &fixture).await;
    assert_canonical_source(&pool, stripped_first, &fixture).await;
    assert_eq!(std::fs::read(original_path).unwrap(), original_bytes);
    handle.abort();
}

async fn upload_fixture(client: &reqwest::Client, base_url: &str, slug: &str, bytes: &[u8]) {
    let boundary = "----bugs-test-canonical";
    let response = client
        .post(format!(
            "{base_url}/api/0/projects/default/{slug}/files/dsyms/"
        ))
        .header(
            "Content-Type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(dsym_upload_body(boundary, "fixture", bytes))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    let response: serde_json::Value = response.json().await.unwrap();
    assert_eq!(response["errors"], serde_json::json!([]), "{response}");
}

async fn assert_canonical_source(pool: &SqlitePool, project_id: i64, fixture: &Fixture) -> String {
    let path: String = sqlx::query_scalar(
        "SELECT file_path FROM artifact_debug_ids \
         WHERE project_id = ? AND debug_id = ? AND arch = 'arm64' AND kind = 'native'",
    )
    .bind(project_id)
    .bind(&fixture.debug_id)
    .fetch_one(pool)
    .await
    .unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let cache = symbolic::symcache::SymCache::parse(&bytes).unwrap();
    let location = cache
        .lookup(fixture.function_offset)
        .next()
        .expect("fixture address resolves");
    assert!(location.function().name().contains("bugs_native_fixture"));
    assert!(location.file().unwrap().full_path().ends_with("fixture.c"));
    assert_eq!(
        location.line(),
        1,
        "rich DWARF source line must survive all upload orders"
    );
    path
}

#[tokio::test]
async fn test_artifact_identity_migration_preserves_historical_ownership() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::raw_sql(include_str!("../migrations/001_initial_schema.sql"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!(
        "../migrations/017_debug_ids_project_and_arch.sql"
    ))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::raw_sql(
        "INSERT INTO projects(id, name, slug) VALUES (1, 'One', 'one'), (2, 'Two', 'two'); \
         INSERT INTO releases(id, version) VALUES (1, 'old-release'); \
         INSERT INTO release_projects(release_id, project_id) VALUES (1, 1), (1, 2); \
         INSERT INTO artifact_debug_ids(id, debug_id, release_id, file_path, kind) \
          VALUES (10, 'shared', 1, '/old/shared.symc', 'native'), \
                 (20, 'unowned', NULL, '/old/map', 'sourcemap');",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::raw_sql(include_str!("../migrations/019_artifact_identity.sql"))
        .execute(&pool)
        .await
        .unwrap();
    let shared: Vec<(i64, i64, String, String, Option<String>, Option<i64>)> = sqlx::query_as(
        "SELECT project_id, release_id, file_path, arch, source_name, quality_lines \
         FROM artifact_debug_ids WHERE debug_id = 'shared' ORDER BY project_id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        shared,
        vec![
            (1, 1, "/old/shared.symc".into(), "".into(), None, None),
            (2, 1, "/old/shared.symc".into(), "".into(), None, None),
        ]
    );
    let unowned: (i64, Option<i64>, Option<i64>, String) = sqlx::query_as(
        "SELECT id, project_id, release_id, file_path FROM artifact_debug_ids WHERE debug_id = 'unowned'",
    ).fetch_one(&pool).await.unwrap();
    assert_eq!(unowned, (20, None, None, "/old/map".into()));
    // Removing one project must not discard a formerly global artifact from
    // another project on the same release.
    sqlx::query("DELETE FROM projects WHERE id = 1")
        .execute(&pool)
        .await
        .unwrap();
    let survivor: (i64, i64, String) = sqlx::query_as(
        "SELECT project_id, release_id, file_path FROM artifact_debug_ids WHERE debug_id = 'shared'",
    ).fetch_one(&pool).await.unwrap();
    assert_eq!(survivor, (2, 1, "/old/shared.symc".into()));
}

// --- Helpers -------------------------------------------------------

struct Fixture {
    _dir: tempfile::TempDir,
    bin_path: PathBuf,
    dsym_path: PathBuf,
    debug_id: String,
    /// Offset of bugs_native_fixture relative to the Mach-O image base 0x100000000.
    function_offset: u64,
}

fn build_fixture() -> Fixture {
    let temp_dir = tempfile::tempdir().unwrap();
    let dir = temp_dir.path();

    let src = dir.join("fixture.c");
    std::fs::write(
        &src,
        "int bugs_native_fixture(int x) { return x * 7 + 3; }\n\
         int main(int argc, char **argv) { return bugs_native_fixture(argc); }\n",
    )
    .unwrap();

    let obj = dir.join("fixture.o");
    let bin = dir.join("fixture");

    run(&[
        "cc",
        "-arch",
        "arm64",
        "-g",
        "-O0",
        "-c",
        src.to_str().unwrap(),
        "-o",
        obj.to_str().unwrap(),
    ]);
    run(&[
        "cc",
        "-arch",
        "arm64",
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
    run(&["strip", "-S", bin.to_str().unwrap()]);

    Fixture {
        _dir: temp_dir,
        bin_path: bin,
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

async fn create_project(client: &reqwest::Client, base_url: &str, name: &str, slug: &str) -> i64 {
    let project: serde_json::Value = client
        .post(format!("{base_url}/api/internal/projects"))
        .json(&serde_json::json!({"name": name, "slug": slug}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    project["id"].as_i64().unwrap()
}

fn dsym_upload_body(boundary: &str, filename: &str, file_content: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\n\
             Content-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n\
             Content-Type: application/octet-stream\r\n\r\n",
        )
        .as_bytes(),
    );
    body.extend_from_slice(file_content);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

fn zip_fixture_dsym(fixture: &Fixture) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file("fixture.dSYM/Contents/Resources/DWARF/fixture", options)
        .unwrap();
    zip.write_all(&std::fs::read(&fixture.dsym_path).unwrap())
        .unwrap();
    zip.finish().unwrap().into_inner()
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
    start_test_server_with_upload_limit(None).await
}

async fn start_test_server_with_upload_limit(
    upload_limit: Option<usize>,
) -> (String, String, tokio::task::JoinHandle<()>) {
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
            let mut config = bugs::config::Config {
                bind_address: bind_addr,
                database_path: db_path,
                artifacts_dir: format!("/tmp/bugs-native-test-{port}-artifacts"),
                ..Default::default()
            };
            if let Some(upload_limit) = upload_limit {
                config.uploads.max_bytes = upload_limit;
            }
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

            let app = bugs::api::router(&state, tokio_util::sync::CancellationToken::new())
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
