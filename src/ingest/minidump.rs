//! Crashpad minidump ingestion: multipart parsing and event synthesis.
//!
//! sentry-native's crashpad backend (bundled by sentry_flutter on Linux
//! and Windows desktop) uploads native crashes as
//! `POST /api/{project_id}/minidump/?sentry_key=…` with a
//! multipart/form-data body — the whole body gzipped by default
//! (crashpad `upload_gzip`). Parts, keyed by *field name*:
//!
//! - `upload_file_minidump` — the minidump (must start with MDMP/PMDM)
//! - `__sentry-event` — msgpack-encoded sidecar event written by
//!   sentry-native at crash time (event_id, level, release, dist,
//!   environment, user, tags, extra, contexts, sdk; no stacktraces)
//! - `__sentry-breadcrumb1` / `__sentry-breadcrumb2` — two rotating
//!   append-only files of concatenated msgpack breadcrumb maps
//! - other file parts — user attachments, keyed by filename
//! - bare form fields (crashpad sends `guid`) — folded into `extra`
//!
//! We stackwalk the dump with rust-minidump — CFI for modules whose
//! debug files were uploaded (`crate::ingest::cfi`), frame-pointer/scan
//! heuristics elsewhere — and synthesize a normal Sentry event: the
//! crashed thread's frames on `exception.values[0]`, other threads under
//! `threads.values`, and `debug_meta.images` from the module list. Each
//! frame's `trust` records how it was found (`cfi`, `fp`, `scan`, …). The
//! synthesized envelope then rides the ordinary worker pipeline, where
//! native symbolication resolves frames against uploaded symcaches.
//!
//! Semantics mirror getsentry/relay's minidump endpoint; deliberate
//! omissions: Electron `sentry__N` chunked form fields and Breakpad
//! `sentry[key]` nesting (crashpad/sentry-native never send them), and
//! crashpad module annotation contexts (sentry-native leaves them empty).

use std::io::Read;

use serde_json::{Value, json};

use crate::db::DbPool;
use crate::ingest::cfi::DumpCfi;
use crate::util::id::generate_event_id;

/// Field names with server-side meaning, per relay's minidump endpoint.
const FIELD_MINIDUMP: &str = "upload_file_minidump";
const FIELD_EVENT: &str = "__sentry-event";
const FIELD_BREADCRUMBS1: &str = "__sentry-breadcrumb1";
const FIELD_BREADCRUMBS2: &str = "__sentry-breadcrumb2";

const MAGIC_LE: &[u8] = b"MDMP";
const MAGIC_BE: &[u8] = b"PMDM";

/// Content types that carry a bare minidump body instead of multipart.
pub fn is_raw_minidump_content_type(content_type: &str) -> bool {
    let ct = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    ct == "application/octet-stream" || ct == "application/x-dmp"
}

pub fn has_minidump_magic(data: &[u8]) -> bool {
    data.len() >= 4 && (&data[..4] == MAGIC_LE || &data[..4] == MAGIC_BE)
}

/// Undo a gzip container around a bare minidump payload (crashpad and
/// some SDKs compress the dump itself). Bounded by `max` like the
/// envelope path. Non-gzip data is returned unchanged.
pub fn decompress_dump_container(data: Vec<u8>, max: usize) -> Result<Vec<u8>, String> {
    if data.len() >= 2 && data[0] == 0x1f && data[1] == 0x8b {
        let mut decoder = flate2::read::GzDecoder::new(&data[..]).take((max as u64) + 1);
        let mut out = Vec::new();
        decoder
            .read_to_end(&mut out)
            .map_err(|e| format!("invalid gzip minidump: {e}"))?;
        if out.len() > max {
            return Err("decompressed minidump too large".to_string());
        }
        return Ok(out);
    }
    Ok(data)
}

/// A non-protocol file part forwarded as an event attachment.
pub struct UploadAttachment {
    pub filename: String,
    pub content_type: Option<String>,
    pub data: Vec<u8>,
}

/// Parsed contents of a crashpad upload.
#[derive(Default)]
pub struct MinidumpUpload {
    pub dump: Option<Vec<u8>>,
    pub dump_filename: Option<String>,
    pub sidecar_event: Option<Value>,
    pub breadcrumbs1: Vec<Value>,
    pub breadcrumbs2: Vec<Value>,
    pub attachments: Vec<UploadAttachment>,
    /// Bare form fields (e.g. crashpad's `guid`) → `event.extra`.
    pub form_extra: Vec<(String, String)>,
}

/// Parses the multipart body. `max_part` bounds each part's payload.
pub async fn parse_multipart(
    body: bytes::Bytes,
    boundary: &str,
    max_part: usize,
) -> Result<MinidumpUpload, String> {
    let stream =
        futures_util::stream::once(
            async move { Ok::<bytes::Bytes, std::convert::Infallible>(body) },
        );
    let mut multipart = multer::Multipart::with_constraints(
        stream,
        boundary,
        multer::Constraints::new().size_limit(multer::SizeLimit::new().per_field(max_part as u64)),
    );

    let mut upload = MinidumpUpload::default();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| format!("invalid multipart: {e}"))?
    {
        let name = field.name().unwrap_or("").to_string();
        let filename = field.file_name().map(|s| s.to_string());
        let content_type = field.content_type().map(|m| m.to_string());
        let data = field
            .bytes()
            .await
            .map_err(|e| format!("invalid multipart part: {e}"))?;

        match name.as_str() {
            FIELD_MINIDUMP => {
                upload.dump = Some(data.to_vec());
                upload.dump_filename = filename;
            }
            FIELD_EVENT => {
                upload.sidecar_event = parse_msgpack_event(&data);
            }
            FIELD_BREADCRUMBS1 => {
                upload.breadcrumbs1 = parse_msgpack_breadcrumbs(&data);
            }
            FIELD_BREADCRUMBS2 => {
                upload.breadcrumbs2 = parse_msgpack_breadcrumbs(&data);
            }
            // The `sentry` bare field carries a JSON event (Breakpad
            // clients). Only used when no msgpack sidecar arrived.
            "sentry" if filename.is_none() => {
                if upload.sidecar_event.is_none() {
                    upload.sidecar_event = serde_json::from_slice(&data).ok();
                }
            }
            _ => {
                if let Some(filename) = filename {
                    upload.attachments.push(UploadAttachment {
                        filename,
                        content_type,
                        data: data.to_vec(),
                    });
                } else if !name.is_empty()
                    && let Ok(value) = std::str::from_utf8(&data)
                {
                    upload.form_extra.push((name, value.to_string()));
                }
            }
        }
    }
    Ok(upload)
}

/// `__sentry-event` is a single msgpack-encoded event object.
fn parse_msgpack_event(data: &[u8]) -> Option<Value> {
    let value: Value = rmp_serde::from_slice(data).ok()?;
    value.is_object().then_some(value)
}

/// Breadcrumb files are append-only streams of concatenated msgpack
/// maps (one per breadcrumb, no array wrapper). Decode until EOF;
/// tolerate a trailing partial write (crash mid-append).
fn parse_msgpack_breadcrumbs(data: &[u8]) -> Vec<Value> {
    use serde::Deserialize;
    let mut crumbs = Vec::new();
    let mut de = rmp_serde::Deserializer::new(data);
    while let Ok(value) = Value::deserialize(&mut de) {
        if value.is_object() {
            crumbs.push(value);
        }
    }
    crumbs
}

/// Relay's file-granular merge: order the two rotating files by their
/// last timestamp (older first), concatenate, and keep the newest
/// `max(len1, len2)` entries.
pub fn merge_breadcrumbs(mut first: Vec<Value>, mut second: Vec<Value>) -> Vec<Value> {
    fn last_timestamp(crumbs: &[Value]) -> &str {
        crumbs
            .last()
            .and_then(|c| c.get("timestamp"))
            .and_then(|t| t.as_str())
            .unwrap_or("")
    }
    if last_timestamp(&first) > last_timestamp(&second) {
        std::mem::swap(&mut first, &mut second);
    }
    let max_length = first.len().max(second.len());
    first.append(&mut second);
    if first.len() > max_length {
        first.drain(0..first.len() - max_length);
    }
    first
}

/// Stackwalks `dump` — with the CFI uploaded to `project_id` for its
/// modules — and merges the result over the sidecar event. Returns the
/// event id (32-hex) and the event JSON.
///
/// Performs CPU-bound work; call from a blocking context.
pub async fn synthesize_event(
    db: &DbPool,
    project_id: i64,
    dump: &[u8],
    upload: &MinidumpUpload,
) -> Result<(String, Value), String> {
    let minidump = minidump::Minidump::read(dump).map_err(|e| format!("invalid minidump: {e}"))?;
    // Unreadable streams surface as process_minidump's error below.
    let cfi = match (
        minidump.get_stream::<minidump::MinidumpModuleList>(),
        minidump.get_stream::<minidump::MinidumpSystemInfo>(),
    ) {
        (Ok(modules), Ok(system_info)) => {
            DumpCfi::load(db, project_id, &modules, system_info.cpu).await
        }
        _ => DumpCfi::default(),
    };
    let state = minidump_processor::process_minidump(&minidump, &cfi)
        .await
        .map_err(|e| format!("minidump processing failed: {e}"))?;

    let mut event = upload.sidecar_event.clone().unwrap_or_else(|| json!({}));
    let map = event
        .as_object_mut()
        .expect("sidecar events are validated as objects");

    // Event id: prefer the sidecar's pre-generated crash id so native
    // user feedback can attach to it. Fall back to a fresh id.
    let event_id = map
        .get("event_id")
        .and_then(|v| v.as_str())
        .map(normalize_event_id)
        .filter(|id| id.len() == 32)
        .unwrap_or_else(generate_event_id);
    map.insert("event_id".into(), json!(event_id));

    // Crash placeholders force these regardless of sidecar contents.
    map.insert("platform".into(), json!("native"));
    if !map.get("level").is_some_and(|l| l.is_string()) {
        map.insert("level".into(), json!("fatal"));
    }
    let timestamp: chrono::DateTime<chrono::Utc> = state.time.into();
    map.insert("timestamp".into(), json!(timestamp.to_rfc3339()));

    // Exception + threads from the walked stacks.
    let crashed_idx = state.requesting_thread;
    let crash_reason = state
        .exception_info
        .as_ref()
        .map(|info| format!("{} / {:#x}", info.reason, info.address.0));
    let (exc_type, exc_value) = match &crash_reason {
        Some(reason) => (reason.clone(), format!("Fatal Error: {reason}")),
        None => (
            "Minidump".to_string(),
            "Native crash without exception stream".to_string(),
        ),
    };

    let mut threads = Vec::with_capacity(state.threads.len());
    let mut crashed_stacktrace = None;
    let mut crashed_thread_id = None;
    for (index, stack) in state.threads.iter().enumerate() {
        let crashed = crashed_idx == Some(index);
        let frames: Vec<Value> = stack
            .frames
            .iter()
            .rev() // walker yields callee-first; Sentry wants caller-first
            .map(|frame| {
                json!({
                    "instruction_addr": format!("{:#x}", frame.instruction),
                    "package": frame.module.as_ref().map(|m| {
                        use minidump::Module as _;
                        m.code_file().into_owned()
                    }),
                    "trust": frame_trust_name(frame.trust),
                })
            })
            .collect();
        // The walker already reports callers at return address - 1.
        let stacktrace = (!frames.is_empty())
            .then(|| json!({ "frames": frames, "instruction_addr_adjustment": "none" }));

        if crashed {
            crashed_stacktrace = stacktrace.clone();
            crashed_thread_id = Some(stack.thread_id);
        }
        let mut thread = json!({
            "id": stack.thread_id,
            "crashed": crashed,
        });
        if let Some(name) = &stack.thread_name {
            thread["name"] = json!(name);
        }
        // The crashed thread's stack lives on the exception; carrying it
        // twice would double-render in the UI.
        if !crashed && let Some(st) = stacktrace {
            thread["stacktrace"] = st;
        }
        threads.push(thread);
    }

    let mut exception = json!({
        "type": exc_type,
        "value": exc_value,
        "mechanism": { "type": "minidump", "handled": false, "synthetic": true },
    });
    if let Some(st) = crashed_stacktrace {
        exception["stacktrace"] = st;
    }
    if let Some(tid) = crashed_thread_id {
        exception["thread_id"] = json!(tid);
    }

    // Insert at index 0: Sentry keys minidump handling off
    // exception.values[0].mechanism.type. Sidecar exceptions (none from
    // sentry-native today) are retained after it.
    let mut exc_values = vec![exception];
    if let Some(existing) = map
        .get_mut("exception")
        .and_then(|e| e.get_mut("values"))
        .and_then(|v| v.as_array_mut())
    {
        exc_values.append(existing);
    }
    map.insert("exception".into(), json!({ "values": exc_values }));
    map.insert("threads".into(), json!({ "values": threads }));

    // debug_meta.images from the module list; the worker's native
    // symbolication keys symcache lookups off these debug ids.
    let os_image_type = match state.system_info.os {
        minidump::system_info::Os::Windows => "pe",
        minidump::system_info::Os::MacOs | minidump::system_info::Os::Ios => "macho",
        _ => "elf",
    };
    let images: Vec<Value> = state
        .modules
        .iter()
        .filter_map(|module| {
            use minidump::Module as _;
            let debug_id = module.debug_identifier();
            let code_id = module.code_identifier();
            // Modules without any identifier can never resolve; drop them.
            if debug_id.is_none() && code_id.is_none() {
                return None;
            }
            let mut image = json!({
                "type": os_image_type,
                "image_addr": format!("{:#x}", module.base_address()),
                "image_size": module.size(),
                "code_file": module.code_file().into_owned(),
            });
            if let Some(id) = debug_id {
                image["debug_id"] = json!(id.to_string());
            }
            if let Some(id) = code_id {
                image["code_id"] = json!(id.to_string().to_lowercase());
            }
            if let Some(file) = module.debug_file() {
                image["debug_file"] = json!(file.into_owned());
            }
            Some(image)
        })
        .collect();
    map.insert("debug_meta".into(), json!({ "images": images }));

    // Merged breadcrumbs (sidecar events never carry any).
    let crumbs = merge_breadcrumbs(upload.breadcrumbs1.clone(), upload.breadcrumbs2.clone());
    if !crumbs.is_empty() && !map.contains_key("breadcrumbs") {
        map.insert("breadcrumbs".into(), json!({ "values": crumbs }));
    }

    // Bare form fields (crashpad's guid) → extra, without clobbering.
    if !upload.form_extra.is_empty() {
        let extra = map.entry("extra").or_insert_with(|| json!({}));
        if let Some(extra) = extra.as_object_mut() {
            for (key, value) in &upload.form_extra {
                extra.entry(key.clone()).or_insert_with(|| json!(value));
            }
        }
    }

    // OS context from the dump's system info, when the sidecar has none.
    let contexts = map.entry("contexts").or_insert_with(|| json!({}));
    if let Some(contexts) = contexts.as_object_mut()
        && !contexts.contains_key("os")
    {
        let mut os = json!({ "name": state.system_info.os.long_name() });
        if let Some(version) = &state.system_info.os_version {
            os["version"] = json!(version);
        }
        if let Some(build) = &state.system_info.os_build {
            os["build"] = json!(build);
        }
        contexts.insert("os".into(), os);
    }

    if !map.contains_key("sdk") {
        map.insert(
            "sdk".into(),
            json!({ "name": "minidump.crashpad", "version": "0.0.0" }),
        );
    }

    Ok((event_id, event))
}

/// Strips dashes/braces and lowercases, so both `xxxxxxxx-xxxx-…` and
/// bare 32-hex sidecar ids normalize to the storage format.
fn normalize_event_id(id: &str) -> String {
    id.chars()
        .filter(|c| c.is_ascii_hexdigit())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn frame_trust_name(trust: minidump_unwind::FrameTrust) -> &'static str {
    use minidump_unwind::FrameTrust;
    match trust {
        FrameTrust::Context => "context",
        FrameTrust::CallFrameInfo => "cfi",
        FrameTrust::CfiScan => "cfi_scan",
        FrameTrust::FramePointer => "fp",
        FrameTrust::Scan => "scan",
        FrameTrust::PreWalked => "prewalked",
        FrameTrust::None => "none",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    use minidump::format::{CvSignature, PlatformId, ProcessorArchitecture};
    use minidump_synth::{
        DumpString, Exception, Memory, Module as SynthModule, STOCK_VERSION_INFO, SynthMinidump,
        SystemInfo, Thread, amd64_context,
    };
    use sha1::{Digest, Sha1};
    use test_assembler::{Endian, Section};

    use crate::api::chunked_upload::{AssembleEntry, assemble_one};
    use crate::config::Config;
    use crate::ingest::cfi;
    use crate::util::chunk_store::chunk_path;

    const PLEZY_BASE: u64 = 0x5555_5555_0000;
    const LIBC_BASE: u64 = 0x7f00_0000_0000;
    const STACK_BASE: u64 = 0x7ffd_0000_0000;
    /// ELF build ids of `/opt/plezy/plezy` (Plezy 2.23.0 x86_64) and
    /// Fedora 44 `libc.so.6` from Linux crash #2506's module list.
    const PLEZY_BUILD_ID: &str = "93cc47e0c436dc7b839225c0c2c7ee87b37a1cfc";
    const LIBC_BUILD_ID: &str = "85075e2ae2e13352a1413d41c327dacf6461526c";
    /// Breakpad files for those identities, as the DIF upload accepts them.
    /// The crashing plezy function keeps its return address at CFA-8 with
    /// CFA = rsp+16 and never sets up a frame pointer.
    const PLEZY_SYM: &str = "MODULE Linux x86_64 E047CC9336C47BDC839225C0C2C7EE870 plezy\n\
        INFO CODE_ID 93CC47E0C436DC7B839225C0C2C7EE87B37A1CFC\n\
        FUNC 1000 100 0 crash_here\n\
        STACK CFI INIT 1000 100 .cfa: $rsp 16 + .ra: .cfa -8 + ^\n";
    /// libc's CFI covers no address on the test stack.
    const LIBC_SYM: &str = "MODULE Linux x86_64 2A5E0785E1E25233A1413D41C327DACF0 libc.so.6\n\
        INFO CODE_ID 85075E2AE2E13352A1413D41C327DACF6461526C\n\
        FUNC 9000 10 0 raise\n\
        STACK CFI INIT 9000 10 .cfa: $rsp 8 + .ra: .cfa -8 + ^\n";

    /// SIGSEGV at plezy+0x1010 with rbp = 0 and a stale plezy code
    /// pointer below the real return address into libc: the shape that
    /// made scanning invent `plezy` callers in #2125/#2506.
    fn linux_dump() -> Vec<u8> {
        let e = Endian::Little;
        let cv = |build_id: &str| {
            Section::with_endian(e)
                .D32(CvSignature::Elf as u32)
                .append_bytes(&hex::decode(build_id).unwrap())
        };
        let module = |base: u64, name: &DumpString, cv: &Section| {
            SynthModule::new(e, base, 0x10000, name, 0, 0, Some(&STOCK_VERSION_INFO)).cv_record(cv)
        };
        let plezy_name = DumpString::new("/opt/plezy/plezy", e);
        let plezy_cv = cv(PLEZY_BUILD_ID);
        let plezy = module(PLEZY_BASE, &plezy_name, &plezy_cv);
        let libc_name = DumpString::new("/usr/lib64/libc.so.6", e);
        let libc_cv = cv(LIBC_BUILD_ID);
        let libc = module(LIBC_BASE, &libc_name, &libc_cv);

        let stack = Memory::with_section(
            Section::with_endian(e)
                .D64(PLEZY_BASE + 0x2000) // stale code pointer at rsp
                .D64(LIBC_BASE + 0x3000) // return address at CFA-8
                .D64(0)
                .D64(PLEZY_BASE + 0x4000), // libc's caller
            STACK_BASE,
        );
        let context = amd64_context(e, PLEZY_BASE + 0x1010, STACK_BASE);
        let thread = Thread::new(e, 1, &stack, &context);
        let system_info = SystemInfo::new(e)
            .set_processor_architecture(ProcessorArchitecture::PROCESSOR_ARCHITECTURE_AMD64 as u16)
            .set_platform_id(PlatformId::Linux as u32);
        let mut exception = Exception::new(e);
        exception.thread_id = 1;
        exception.exception_record.exception_code = 11; // SIGSEGV
        exception.exception_record.exception_address = 0x8;

        SynthMinidump::with_endian(e)
            .add_system_info(system_info)
            .add_exception(exception)
            .add_thread(thread)
            .add_module(plezy)
            .add_module(libc)
            .add(plezy_name)
            .add(plezy_cv)
            .add(libc_name)
            .add(libc_cv)
            .add(context)
            .add_memory(stack)
            .finish()
            .unwrap()
    }

    struct Fixture {
        _temp: tempfile::TempDir,
        db: DbPool,
        config: Config,
    }

    impl Fixture {
        async fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let config = Config {
                database_path: temp.path().join("bugs.db").to_string_lossy().into_owned(),
                artifacts_dir: temp.path().join("artifacts").to_string_lossy().into_owned(),
                ..Config::default()
            };
            let db = DbPool::init(&config).await.unwrap();
            sqlx::query(
                "INSERT INTO projects (id, org_id, name, slug) VALUES (1, 1, 'Plezy', 'plezy')",
            )
            .execute(db.writer())
            .await
            .unwrap();
            Self {
                _temp: temp,
                db,
                config,
            }
        }

        /// Runs the DIF assembly step for a single-chunk upload.
        async fn upload(&self, sym: &str) {
            let checksum = hex::encode(Sha1::digest(sym));
            let chunks = Path::new(&self.config.artifacts_dir).join("chunks");
            let path = chunk_path(&chunks, &checksum);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, sym).unwrap();
            let entry = AssembleEntry {
                name: Some("module.sym".to_string()),
                debug_id: None,
                chunks: vec![checksum.clone()],
            };
            let chunks = chunks.to_string_lossy();
            let result = assemble_one(&self.db, &self.config, 1, &chunks, &checksum, &entry)
                .await
                .unwrap();
            let kinds: Vec<&str> = result.artifacts.iter().map(|a| a.kind.as_str()).collect();
            assert_eq!(kinds, [cfi::KIND, "native"]);
        }

        /// Crashed-thread frames, caller-first, as `(instruction_addr, trust)`.
        async fn walk(&self) -> Vec<(String, String)> {
            let dump = linux_dump();
            let (_, event) = synthesize_event(&self.db, 1, &dump, &MinidumpUpload::default())
                .await
                .unwrap();
            let stacktrace = &event["exception"]["values"][0]["stacktrace"];
            // Caller addresses are already return address - 1.
            assert_eq!(stacktrace["instruction_addr_adjustment"], "none");
            stacktrace["frames"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| {
                    (
                        f["instruction_addr"].as_str().unwrap().to_owned(),
                        f["trust"].as_str().unwrap().to_owned(),
                    )
                })
                .collect()
        }
    }

    fn frame(addr: u64, trust: &str) -> (String, String) {
        (format!("{addr:#x}"), trust.to_owned())
    }

    #[tokio::test]
    async fn uploaded_cfi_replaces_the_scanned_caller() {
        let fixture = Fixture::new().await;
        // rbp = 0 defeats frame pointers, so without CFI every caller is
        // scanned and the stale plezy pointer becomes a bogus frame.
        let scanned = vec![
            frame(PLEZY_BASE + 0x3fff, "scan"),
            frame(LIBC_BASE + 0x2fff, "scan"),
            frame(PLEZY_BASE + 0x1fff, "scan"),
            frame(PLEZY_BASE + 0x1010, "context"),
        ];
        assert_eq!(fixture.walk().await, scanned);

        // CFI that covers no frame leaves the walk unchanged.
        fixture.upload(LIBC_SYM).await;
        assert_eq!(fixture.walk().await, scanned);

        // plezy's CFI unwinds the crash straight into libc; libc still scans.
        fixture.upload(PLEZY_SYM).await;
        assert_eq!(
            fixture.walk().await,
            vec![
                frame(PLEZY_BASE + 0x3fff, "scan"),
                frame(LIBC_BASE + 0x2fff, "cfi"),
                frame(PLEZY_BASE + 0x1010, "context"),
            ]
        );
    }

    #[test]
    fn merge_breadcrumbs_orders_by_last_timestamp() {
        let older = vec![json!({"message": "a", "timestamp": "2026-01-01T00:00:01Z"})];
        let newer = vec![
            json!({"message": "b", "timestamp": "2026-01-01T00:00:02Z"}),
            json!({"message": "c", "timestamp": "2026-01-01T00:00:03Z"}),
        ];
        // Newest max(len1, len2) = 2 entries survive, oldest dropped.
        let merged = merge_breadcrumbs(newer.clone(), older.clone());
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0]["message"], "b");
        assert_eq!(merged[1]["message"], "c");
        // Symmetric regardless of which file is passed first.
        assert_eq!(merge_breadcrumbs(older, newer), merged);
    }

    #[test]
    fn breadcrumb_stream_tolerates_truncated_tail() {
        let mut stream = Vec::new();
        rmp_serde::encode::write(&mut stream, &json!({"message": "ok"})).unwrap();
        let mut partial = Vec::new();
        rmp_serde::encode::write(&mut partial, &json!({"message": "cut"})).unwrap();
        stream.extend_from_slice(&partial[..partial.len() / 2]);
        let crumbs = parse_msgpack_breadcrumbs(&stream);
        assert_eq!(crumbs.len(), 1);
        assert_eq!(crumbs[0]["message"], "ok");
    }

    #[test]
    fn raw_content_types() {
        assert!(is_raw_minidump_content_type("application/octet-stream"));
        assert!(is_raw_minidump_content_type(
            "application/x-dmp; charset=binary"
        ));
        assert!(!is_raw_minidump_content_type(
            "multipart/form-data; boundary=x"
        ));
    }

    #[test]
    fn magic_detection() {
        assert!(has_minidump_magic(b"MDMP1234"));
        assert!(has_minidump_magic(b"PMDM1234"));
        assert!(!has_minidump_magic(b"ELF!"));
    }

    #[test]
    fn event_id_normalization() {
        assert_eq!(
            normalize_event_id("AABBCCDD-EEFF-0011-2233-445566778899"),
            "aabbccddeeff00112233445566778899"
        );
    }
}
