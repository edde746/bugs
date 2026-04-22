//! Native symbolication against uploaded dSYM / PDB / ELF symbol caches.
//!
//! Mirrors the JS source-map path in `symbolication.rs`: we iterate the
//! event's stacktrace frames, look up the containing image in
//! `debug_meta.images`, find a matching SymCache on disk keyed by
//! debug_id (populated by `src/api/dsyms.rs`), and mutate
//! `frame.function` / `frame.filename` / `frame.lineno` in place.

use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

use memmap2::Mmap;
use once_cell::sync::Lazy;
use symbolic::common::Name;
use symbolic::demangle::{Demangle, DemangleOptions};
use symbolic::symcache::SymCache;
use tracing::warn;

use crate::config::SymbolicationConfig;
use crate::db::DbPool;
use crate::sentry_protocol::types::{SentryEvent, StackFrame};
use crate::util::byte_capped_lru::ByteCappedLru;
use crate::util::id::normalize_debug_id;

/// Outcome of a native-symbolication attempt. Shape parallels
/// `symbolication::SymbolicationOutcome` so `combine_outcomes` in the
/// processor can fold both into the single `events.symbolication_state`
/// column.
#[derive(Debug, Clone)]
pub enum NativeSymbolicationOutcome {
    /// No native frames / no debug_meta — nothing to do.
    NotAttempted,
    /// Attempted a lookup. `resolved` is the number of frames whose
    /// `function` we populated.
    Ok { resolved: usize, total: usize },
    /// At least one native frame had a parseable debug_id, but no
    /// SymCache was on file for any of them. Eligible for retry once
    /// symbols are uploaded.
    MissingMap,
}

/// file_path → mmap. Byte-capped because an mmap handle's "cost" is its
/// length in bytes (faulted pages count toward RSS on access); a pure
/// entry-count cap would let a handful of large dSYMs map hundreds of
/// MB. Invalidated explicitly on upload via `invalidate_symcache_path`;
/// atomic rename-over means an unevicted entry would otherwise keep
/// mapping the replaced inode's stale bytes.
static NATIVE_CACHE: Lazy<Mutex<ByteCappedLru<String, Arc<Mmap>>>> = Lazy::new(|| {
    Mutex::new(ByteCappedLru::new(
        NonZeroUsize::new(64).unwrap(),
        256 * 1024 * 1024,
    ))
});

pub fn configure_cache(cfg: &SymbolicationConfig) {
    let cap = NonZeroUsize::new(cfg.native_symcache_cache_size.max(1)).expect("max(1) is non-zero");
    let bytes = cfg.native_symcache_cache_bytes_mb.max(1) * 1024 * 1024;
    NATIVE_CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .resize(cap, bytes);
}

/// Drop any mmap cached at `file_path`. The upload handler calls this
/// after a successful atomic rename so subsequent lookups re-open the
/// new inode instead of reading stale bytes from the old mapping.
pub fn invalidate_symcache_path(file_path: &str) {
    NATIVE_CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .pop(&file_path.to_string());
}

/// Parsed image from `debug_meta.images[*]`.
#[derive(Debug, Clone)]
struct DebugImage {
    debug_id: String,
    image_addr: u64,
    image_size: u64,
    code_file: Option<String>,
    image_type: Option<String>,
}

pub async fn symbolicate_native(
    event: &mut SentryEvent,
    project_id: i64,
    db: &DbPool,
) -> NativeSymbolicationOutcome {
    // Cheap rejections first: the vast majority of events have no
    // stacktraces or no debug_meta, so bail before walking JSON.
    if event.debug_meta.is_none() && event.exception.is_none() && event.threads.is_none() {
        return NativeSymbolicationOutcome::NotAttempted;
    }

    let images = match parse_images(event.debug_meta.as_ref()) {
        Some(i) if !i.is_empty() => i,
        _ => return NativeSymbolicationOutcome::NotAttempted,
    };

    if count_native_frames(event) == 0 {
        return NativeSymbolicationOutcome::NotAttempted;
    }

    let debug_ids: Vec<String> = images.iter().map(|i| i.debug_id.clone()).collect();
    if debug_ids.is_empty() {
        return NativeSymbolicationOutcome::NotAttempted;
    }

    let dif_rows = match load_dif_rows(db, project_id, &debug_ids).await {
        Ok(rows) => rows,
        Err(e) => {
            warn!("native symbolication DB query failed: {e}");
            return NativeSymbolicationOutcome::NotAttempted;
        }
    };

    if dif_rows.is_empty() {
        return NativeSymbolicationOutcome::MissingMap;
    }

    let (resolved, total) =
        tokio::task::block_in_place(|| lookup_frames(event, &images, &dif_rows));

    NativeSymbolicationOutcome::Ok { resolved, total }
}

fn parse_images(debug_meta: Option<&serde_json::Value>) -> Option<Vec<DebugImage>> {
    let images = debug_meta?.get("images")?.as_array()?;
    let mut out = Vec::new();
    for img in images {
        let debug_id = img.get("debug_id").and_then(|v| v.as_str()).unwrap_or("");
        let debug_id = normalize_debug_id(debug_id);
        if debug_id.is_empty() {
            continue;
        }
        let image_addr = match img.get("image_addr").and_then(parse_hex_or_int) {
            Some(a) => a,
            None => continue,
        };
        let image_size = img
            .get("image_size")
            .and_then(parse_hex_or_int)
            .unwrap_or(0);
        let code_file = img
            .get("code_file")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let image_type = img
            .get("type")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        out.push(DebugImage {
            debug_id,
            image_addr,
            image_size,
            code_file,
            image_type,
        });
    }
    Some(out)
}

fn parse_hex_or_int(v: &serde_json::Value) -> Option<u64> {
    if let Some(n) = v.as_u64() {
        return Some(n);
    }
    if let Some(s) = v.as_str() {
        let s = s.trim();
        let s = s
            .strip_prefix("0x")
            .or_else(|| s.strip_prefix("0X"))
            .unwrap_or(s);
        return u64::from_str_radix(s, 16).ok();
    }
    None
}

fn count_native_frames(event: &SentryEvent) -> usize {
    let mut count = 0;
    if let Some(ex) = &event.exception {
        for v in &ex.values {
            if let Some(st) = &v.stacktrace {
                count += st.frames.iter().filter(|f| has_addr(f)).count();
            }
        }
    }
    if let Some(th) = &event.threads {
        for t in &th.values {
            if let Some(st) = &t.stacktrace {
                count += st.frames.iter().filter(|f| has_addr(f)).count();
            }
        }
    }
    count
}

fn has_addr(f: &StackFrame) -> bool {
    f.instruction_addr
        .as_deref()
        .map(|s| !s.is_empty())
        .unwrap_or(false)
}

async fn load_dif_rows(
    db: &DbPool,
    project_id: i64,
    debug_ids: &[String],
) -> Result<std::collections::HashMap<String, String>, sqlx::Error> {
    if debug_ids.is_empty() {
        return Ok(Default::default());
    }
    let placeholders = std::iter::repeat_n("?", debug_ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT debug_id, file_path FROM artifact_debug_ids \
         WHERE project_id = ? AND kind = 'native' AND debug_id IN ({placeholders})"
    );
    let mut q = sqlx::query_as::<_, (String, String)>(&sql).bind(project_id);
    for id in debug_ids {
        q = q.bind(id);
    }
    Ok(q.fetch_all(db.reader()).await?.into_iter().collect())
}

fn lookup_frames(
    event: &mut SentryEvent,
    images: &[DebugImage],
    dif_rows: &std::collections::HashMap<String, String>,
) -> (usize, usize) {
    let mut resolved = 0;
    let mut total = 0;

    // Process each frame on the event. We fetch the mmap lazily per
    // (containing-image) so repeated frames within the same image share
    // the cache entry.
    if let Some(ex) = event.exception.as_mut() {
        for v in ex.values.iter_mut() {
            if let Some(st) = v.stacktrace.as_mut() {
                for frame in st.frames.iter_mut() {
                    if resolve_one(frame, images, dif_rows) {
                        resolved += 1;
                    }
                    if has_addr(frame) {
                        total += 1;
                    }
                }
            }
        }
    }
    if let Some(th) = event.threads.as_mut() {
        for t in th.values.iter_mut() {
            if let Some(st) = t.stacktrace.as_mut() {
                for frame in st.frames.iter_mut() {
                    if resolve_one(frame, images, dif_rows) {
                        resolved += 1;
                    }
                    if has_addr(frame) {
                        total += 1;
                    }
                }
            }
        }
    }

    (resolved, total)
}

fn resolve_one(
    frame: &mut StackFrame,
    images: &[DebugImage],
    dif_rows: &std::collections::HashMap<String, String>,
) -> bool {
    try_resolve(frame, images, dif_rows).is_some()
}

fn try_resolve(
    frame: &mut StackFrame,
    images: &[DebugImage],
    dif_rows: &std::collections::HashMap<String, String>,
) -> Option<()> {
    if frame
        .function
        .as_deref()
        .is_some_and(|f| f != "<redacted>" && !f.is_empty())
    {
        return None;
    }
    let iaddr_raw = frame.instruction_addr.as_deref()?.trim();
    let iaddr_str = iaddr_raw
        .strip_prefix("0x")
        .or_else(|| iaddr_raw.strip_prefix("0X"))
        .unwrap_or(iaddr_raw);
    let iaddr = u64::from_str_radix(iaddr_str, 16).ok()?;

    let image = find_containing_image(iaddr, images)?;
    let file_path = dif_rows.get(&image.debug_id)?;

    // SymCache stores function entries as offsets from the image's
    // link-time base. At runtime the image is loaded at image_addr (with
    // ASLR slide = image_addr - image_vmaddr), so the offset is just
    // iaddr - image_addr — image_vmaddr drops out of the formula.
    let relative = iaddr.checked_sub(image.image_addr)?;

    let mmap = get_mmap(file_path)?;
    let symcache = SymCache::parse(&mmap[..]).ok()?;
    let mut locs = symcache.lookup(relative);
    let loc = locs.next()?;

    let func = loc.function();
    let name = func.name();
    let demangled = Name::new(
        name,
        symbolic::common::NameMangling::Unknown,
        func.language(),
    )
    .demangle(DemangleOptions::name_only())
    .unwrap_or_else(|| name.to_string());

    frame.function = Some(demangled);
    if let Some(file) = loc.file() {
        let full = file.full_path();
        if !full.is_empty() {
            frame.abs_path = Some(full.clone());
            frame.filename = Some(full);
        }
    }
    let line = loc.line();
    if line > 0 {
        frame.lineno = Some(line);
    }
    if frame.in_app.is_none() {
        frame.in_app = Some(heuristic_in_app(image));
    }
    Some(())
}

fn find_containing_image(iaddr: u64, images: &[DebugImage]) -> Option<&DebugImage> {
    images.iter().find(|img| {
        iaddr >= img.image_addr
            && (img.image_size == 0 || iaddr < img.image_addr.saturating_add(img.image_size))
    })
}

fn heuristic_in_app(image: &DebugImage) -> bool {
    let Some(cf) = image.code_file.as_deref() else {
        return true;
    };
    if image.image_type.as_deref() == Some("macho")
        && (cf.starts_with("/System/")
            || cf.starts_with("/usr/lib/")
            || cf.contains("/System/Library/")
            || cf.contains(".framework/"))
    {
        return false;
    }
    true
}

fn get_mmap(file_path: &str) -> Option<Arc<Mmap>> {
    if let Some(cached) = NATIVE_CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&file_path.to_string())
        .map(Arc::clone)
    {
        return Some(cached);
    }

    let file = std::fs::File::open(file_path).ok()?;
    let mmap = unsafe { Mmap::map(&file).ok()? };
    let bytes = mmap.len();
    let arc = Arc::new(mmap);
    NATIVE_CACHE.lock().unwrap_or_else(|e| e.into_inner()).put(
        file_path.to_string(),
        arc.clone(),
        bytes,
    );
    Some(arc)
}
