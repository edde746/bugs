//! Call frame information (CFI) for server-side minidump stackwalking.
//!
//! Native uploads persist only SymCaches, which carry no unwind tables.
//! DIF processing therefore also extracts each object's CFI — ELF
//! `.eh_frame`/`.debug_frame`, Mach-O compact unwind, PE `.pdata`, x86 PDB
//! frame data, Breakpad `STACK` records — as a symbolic `CfiCache`
//! (Breakpad `STACK CFI`/`STACK WIN` text behind a version preamble),
//! indexed in `artifact_debug_ids` under `kind = 'cfi'` next to the
//! object's SymCache.
//!
//! Minidump ingest matches the dump's modules to those rows by debug id
//! and hands the parsed caches to rust-minidump's Breakpad CFI evaluator
//! (`walk_frame`). Modules without stored CFI keep the walker's
//! frame-pointer and stack-scan heuristics, exactly as before.

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use minidump::system_info::Cpu;
use minidump::{MinidumpModuleList, Module};
use minidump_unwind::{
    FileError, FileKind, FillSymbolError, FrameSymbolizer, FrameWalker, SymbolFile, SymbolProvider,
};
use symbolic::cfi::CfiCache;
use symbolic::common::{Arch, ByteView, CpuFamily};
use symbolic::debuginfo::Object;
use tracing::warn;

use crate::db::DbPool;
use crate::util::artifact_store::{ArtifactMetadata, ArtifactPayload, store_artifact};
use crate::util::byte_capped_lru::ByteCappedLru;
use crate::util::id::normalize_debug_id;

/// `artifact_debug_ids.kind` for stored CFI caches.
pub const KIND: &str = "cfi";

/// Ingest runs in the crashpad request; the CFI lookup and any cold
/// parses share this budget. Parsing that misses it keeps running and
/// warms the cache for later dumps; this dump walks without it.
const LOAD_BUDGET: Duration = Duration::from_secs(5);

/// file_path → parsed CFI. Paths are content-addressed and immutable.
/// A Linux dump references ~100 bundled libraries, hence the entry cap.
/// The byte cost is the serialized cache size; the parsed rules live on
/// the heap (unlike mmapped SymCaches) and take a small multiple of it.
static CFI_CACHE: LazyLock<Mutex<ByteCappedLru<String, Arc<SymbolFile>>>> = LazyLock::new(|| {
    Mutex::new(ByteCappedLru::new(
        NonZeroUsize::new(512).unwrap(),
        128 * 1024 * 1024,
    ))
});

/// Serialized `CfiCache` for `object`, or `None` when it carries no unwind
/// records (e.g. `objcopy --only-keep-debug` ELF files, whose `.eh_frame`
/// is NOBITS). Never fails: the object's SymCache must not depend on it.
pub fn cache_bytes(object: &Object<'_>) -> Option<Vec<u8>> {
    if !object.has_unwind_info() {
        return None;
    }
    // Uploads are untrusted input to symbolic's unwind-table readers; a
    // panic there must not abort the conversion of the same object.
    let extracted = std::panic::catch_unwind(AssertUnwindSafe(|| CfiCache::from_object(object)));
    let cache = match extracted {
        Ok(Ok(cache)) => cache,
        Ok(Err(e)) => {
            warn!(debug_id = %object.debug_id(), "CFI extraction failed: {e}");
            return None;
        }
        Err(_) => {
            warn!(debug_id = %object.debug_id(), "CFI extraction panicked");
            return None;
        }
    };
    let has_records = cache
        .as_slice()
        .split(|&b| b == b'\n')
        .any(|line| line.starts_with(b"STACK "));
    if !has_records {
        return None;
    }
    let mut bytes = Vec::new();
    cache.write_to(&mut bytes).ok()?;
    Some(bytes)
}

/// Publishes `cache` under [`KIND`] with the identity of the object it was
/// extracted from (`object.kind` is replaced). CFI only refines stackwalks,
/// so a failure is logged and never fails the upload; returns whether the
/// cache was stored.
pub async fn store(
    db: &DbPool,
    artifacts_dir: &str,
    object: ArtifactMetadata<'_>,
    cache: Vec<u8>,
) -> bool {
    let debug_id = object.debug_id;
    let metadata = ArtifactMetadata {
        kind: KIND,
        ..object
    };
    match store_artifact(db, artifacts_dir, metadata, ArtifactPayload::Bytes(cache)).await {
        Ok(_) => true,
        Err(e) => {
            warn!(debug_id, "CFI store failed: {e}");
            false
        }
    }
}

/// Unwind tables for one dump's modules, keyed by normalized debug id.
/// The default (empty) value walks with frame pointers and scanning only.
#[derive(Default)]
pub struct DumpCfi {
    modules: HashMap<String, Arc<SymbolFile>>,
}

impl DumpCfi {
    /// Loads stored CFI for `modules` within `LOAD_BUDGET`. The cache lock
    /// is held only for lookups and inserts; file reads and parsing run on
    /// a blocking thread. Failures degrade to walking without CFI.
    pub async fn load(
        db: &DbPool,
        project_id: i64,
        modules: &MinidumpModuleList,
        cpu: Cpu,
    ) -> Self {
        let Some(family) = cpu_family(cpu) else {
            return Self::default();
        };
        let mut debug_ids: Vec<String> = modules.iter().filter_map(module_key).collect();
        debug_ids.sort_unstable();
        debug_ids.dedup();
        if debug_ids.is_empty() {
            return Self::default();
        }

        let deadline = tokio::time::Instant::now() + LOAD_BUDGET;
        let lookup = load_rows(db, project_id, &debug_ids);
        let rows = match tokio::time::timeout_at(deadline, lookup).await {
            Ok(Ok(rows)) => select_rows(rows, family),
            Ok(Err(e)) => {
                warn!("CFI lookup failed: {e}");
                return Self::default();
            }
            Err(_) => {
                warn!("CFI lookup exceeded {LOAD_BUDGET:?}");
                return Self::default();
            }
        };

        let mut loaded = HashMap::with_capacity(rows.len());
        let mut missing = Vec::new();
        {
            let mut cache = CFI_CACHE.lock().unwrap_or_else(|e| e.into_inner());
            for (debug_id, path) in rows {
                match cache.get(&path) {
                    Some(file) => {
                        loaded.insert(debug_id, file.clone());
                    }
                    None => missing.push((debug_id, path)),
                }
            }
        }
        if !missing.is_empty() {
            let parse = tokio::task::spawn_blocking(move || {
                missing
                    .into_iter()
                    .filter_map(|(debug_id, path)| Some((debug_id, load_file(&path)?)))
                    .collect::<Vec<_>>()
            });
            match tokio::time::timeout_at(deadline, parse).await {
                Ok(Ok(parsed)) => loaded.extend(parsed),
                Ok(Err(e)) => warn!("CFI parse task: {e}"),
                Err(_) => warn!("CFI parsing exceeded {LOAD_BUDGET:?}; walking without it"),
            }
        }
        Self { modules: loaded }
    }
}

#[async_trait::async_trait]
impl SymbolProvider for DumpCfi {
    /// Names resolve later against uploaded SymCaches. Reporting "no
    /// symbols" keeps stack-scan candidate validation identical whether or
    /// not a module has CFI.
    async fn fill_symbol(
        &self,
        _module: &(dyn Module + Sync),
        _frame: &mut (dyn FrameSymbolizer + Send),
    ) -> Result<(), FillSymbolError> {
        Err(FillSymbolError {})
    }

    async fn walk_frame(
        &self,
        module: &(dyn Module + Sync),
        walker: &mut (dyn FrameWalker + Send),
    ) -> Option<()> {
        self.modules
            .get(&module_key(module)?)?
            .walk_frame(module, walker)
    }

    async fn get_file_path(
        &self,
        _module: &(dyn Module + Sync),
        _file_kind: FileKind,
    ) -> Result<std::path::PathBuf, FileError> {
        Err(FileError::NotFound)
    }
}

/// Storage key of a module's debug id. A nil id (no build id / CodeView
/// record) identifies nothing and must never select another module's CFI.
fn module_key(module: &(impl Module + ?Sized)) -> Option<String> {
    let id = module.debug_identifier()?;
    (!id.is_nil()).then(|| normalize_debug_id(&id.to_string()))
}

/// CFI register names are architecture-specific, so only caches for the
/// dump's CPU family may drive its walk.
fn cpu_family(cpu: Cpu) -> Option<CpuFamily> {
    match cpu {
        Cpu::X86 => Some(CpuFamily::Intel32),
        Cpu::X86_64 => Some(CpuFamily::Amd64),
        Cpu::Arm => Some(CpuFamily::Arm32),
        Cpu::Arm64 => Some(CpuFamily::Arm64),
        Cpu::Ppc => Some(CpuFamily::Ppc32),
        Cpu::Ppc64 => Some(CpuFamily::Ppc64),
        Cpu::Mips => Some(CpuFamily::Mips32),
        Cpu::Mips64 => Some(CpuFamily::Mips64),
        _ => None,
    }
}

/// (debug_id, arch, file_path) rows → one file per debug id whose
/// architecture belongs to `family`.
fn select_rows(rows: Vec<(String, String, String)>, family: CpuFamily) -> Vec<(String, String)> {
    let mut selected = HashMap::new();
    for (debug_id, arch, path) in rows {
        if arch
            .parse::<Arch>()
            .is_ok_and(|arch| arch.cpu_family() == family)
        {
            selected.entry(debug_id).or_insert(path);
        }
    }
    selected.into_iter().collect()
}

async fn load_rows(
    db: &DbPool,
    project_id: i64,
    debug_ids: &[String],
) -> Result<Vec<(String, String, String)>, sqlx::Error> {
    let placeholders = std::iter::repeat_n("?", debug_ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT debug_id, arch, file_path FROM artifact_debug_ids \
         WHERE project_id = ? AND kind = ? AND debug_id IN ({placeholders})"
    );
    let mut q = sqlx::query_as::<_, (String, String, String)>(&sql)
        .bind(project_id)
        .bind(KIND);
    for id in debug_ids {
        q = q.bind(id);
    }
    q.fetch_all(db.reader()).await
}

/// Blocking: reads and parses one stored cache, then publishes it.
fn load_file(path: &str) -> Option<Arc<SymbolFile>> {
    let view = match ByteView::open(path) {
        Ok(view) => view,
        Err(e) => {
            warn!(path, "CFI cache read failed: {e}");
            return None;
        }
    };
    let bytes = view.len();
    let file = Arc::new(parse(view)?);
    CFI_CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .put(path.to_string(), file.clone(), bytes);
    Some(file)
}

fn parse(view: ByteView<'_>) -> Option<SymbolFile> {
    let cache = match CfiCache::from_bytes(view) {
        Ok(cache) => cache,
        Err(e) => {
            warn!("invalid CFI cache: {e}");
            return None;
        }
    };
    match SymbolFile::from_bytes(cache.as_slice()) {
        Ok(file) => Some(file),
        Err(e) => {
            warn!("unparseable CFI records: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use symbolic::debuginfo::Archive;

    /// Breakpad text for Plezy 2.23.0's x86_64 `plezy` executable identity
    /// (code_id 93cc47e0c436dc7b839225c0c2c7ee87b37a1cfc, Linux #2506).
    const PLEZY_SYM: &str = "MODULE Linux x86_64 E047CC9336C47BDC839225C0C2C7EE870 plezy\n\
        INFO CODE_ID 93CC47E0C436DC7B839225C0C2C7EE87B37A1CFC\n\
        FUNC 1000 100 0 crash_here\n\
        STACK CFI INIT 1000 100 .cfa: $rsp 16 + .ra: .cfa -8 + ^\n";

    fn object_cache(sym: &str) -> Option<Vec<u8>> {
        let archive = Archive::parse(sym.as_bytes()).unwrap();
        let object = archive.objects().next().unwrap().unwrap();
        cache_bytes(&object)
    }

    #[test]
    fn extracts_stack_records_into_a_versioned_cache() {
        let bytes = object_cache(PLEZY_SYM).expect("STACK CFI present");
        let cache = CfiCache::from_bytes(ByteView::from_slice(&bytes)).unwrap();
        assert!(cache.is_latest());
        assert_eq!(
            std::str::from_utf8(cache.as_slice()).unwrap(),
            "STACK CFI INIT 1000 100 .cfa: $rsp 16 + .ra: .cfa -8 + ^\n"
        );
        assert!(parse(ByteView::from_slice(&bytes)).is_some());
    }

    #[test]
    fn objects_without_unwind_records_store_nothing() {
        let symbols_only = "MODULE Linux x86_64 E047CC9336C47BDC839225C0C2C7EE870 plezy\n\
            FUNC 1000 100 0 crash_here\n";
        assert_eq!(object_cache(symbols_only), None);
    }

    #[test]
    fn selects_only_caches_for_the_dumps_cpu_family() {
        let rows = vec![
            ("a".into(), "x86_64".into(), "/a-x64".into()),
            ("a".into(), "arm64".into(), "/a-arm64".into()),
            ("b".into(), "arm64".into(), "/b-arm64".into()),
            ("c".into(), "".into(), "/c-unknown".into()),
        ];
        let mut selected = select_rows(rows, cpu_family(Cpu::X86_64).unwrap());
        selected.sort();
        assert_eq!(selected, vec![("a".to_string(), "/a-x64".to_string())]);
    }
}
