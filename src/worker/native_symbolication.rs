//! Native symbolication against uploaded dSYM / PDB / ELF symbol caches.
//!
//! Mirrors the JS source-map path in `symbolication.rs`: we iterate the
//! event's stacktrace frames, look up the containing image in
//! `debug_meta.images`, find a matching SymCache on disk keyed by
//! debug_id (populated by `src/api/dsyms.rs` and chunked uploads), and
//! rewrite `frame.function` / `frame.filename` / `frame.lineno`, expanding
//! inlined calls into additional frames.
//!
//! Addressing follows Sentry and symbolicator: `addr_mode` selects absolute
//! or image-relative addresses, and return addresses of caller frames are
//! moved back into the call instruction before the lookup.

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

use memmap2::Mmap;
use once_cell::sync::Lazy;
use symbolic::common::{InstructionInfo, Name};
use symbolic::demangle::{Demangle, DemangleOptions};
use symbolic::symcache::{SourceLocation, SymCache};
use tracing::warn;

use crate::config::SymbolicationConfig;
use crate::db::DbPool;
use crate::sentry_protocol::types::{SentryEvent, StackFrame, Stacktrace};
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
/// MB. Artifact paths are immutable; upgrading the canonical DB pointer
/// opens a new mapping without invalidating readers of the previous file.
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

/// Parsed image from `debug_meta.images[*]`.
#[derive(Debug, Clone)]
struct DebugImage {
    /// Position in `debug_meta.images`, the target of `addr_mode: "rel:<index>"`.
    index: usize,
    debug_id: String,
    arch: Option<String>,
    /// Absent from Android ANR thread-dump images, whose frames carry
    /// image-relative addresses (`addr_mode: "rel:<debug_id>"`).
    image_addr: Option<u64>,
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
    for (index, img) in images.iter().enumerate() {
        let debug_id = img.get("debug_id").and_then(|v| v.as_str()).unwrap_or("");
        let debug_id = normalize_debug_id(debug_id);
        if debug_id.is_empty() {
            continue;
        }
        let image_addr = img.get("image_addr").and_then(parse_hex_or_int);
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
        let arch = img
            .get("arch")
            .and_then(|v| v.as_str())
            .filter(|arch| !arch.is_empty())
            .map(|arch| {
                arch.parse::<symbolic::common::Arch>()
                    .map(|arch| arch.name().to_string())
                    .unwrap_or_else(|_| arch.to_string())
            });
        out.push(DebugImage {
            index,
            debug_id,
            arch,
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
    parse_hex(v.as_str()?)
}

fn parse_hex(s: &str) -> Option<u64> {
    let s = s.trim();
    let s = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    u64::from_str_radix(s, 16).ok()
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

type DifRows = HashMap<String, HashMap<String, String>>;

async fn load_dif_rows(
    db: &DbPool,
    project_id: i64,
    debug_ids: &[String],
) -> Result<DifRows, sqlx::Error> {
    if debug_ids.is_empty() {
        return Ok(Default::default());
    }
    let placeholders = std::iter::repeat_n("?", debug_ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT debug_id, arch, file_path FROM artifact_debug_ids \
         WHERE project_id = ? AND kind = 'native' AND debug_id IN ({placeholders})"
    );
    let mut q = sqlx::query_as::<_, (String, String, String)>(&sql).bind(project_id);
    for id in debug_ids {
        q = q.bind(id);
    }
    let mut rows = DifRows::new();
    for (debug_id, arch, path) in q.fetch_all(db.reader()).await? {
        rows.entry(debug_id).or_default().insert(arch, path);
    }
    Ok(rows)
}

fn lookup_frames(
    event: &mut SentryEvent,
    images: &[DebugImage],
    dif_rows: &DifRows,
) -> (usize, usize) {
    let signal = crash_signal(event);
    let exceptions = event
        .exception
        .iter_mut()
        .flat_map(|ex| ex.values.iter_mut())
        .filter_map(|v| v.stacktrace.as_mut());
    let threads = event
        .threads
        .iter_mut()
        .flat_map(|th| th.values.iter_mut())
        .filter_map(|t| t.stacktrace.as_mut());
    let mut counts = (0, 0);
    for stacktrace in exceptions.chain(threads) {
        let (resolved, total) = symbolicate_stacktrace(stacktrace, images, dif_rows, signal);
        counts.0 += resolved;
        counts.1 += total;
    }
    counts
}

/// Signal number Sentry hands symbolicator for its crashing-frame
/// heuristics: `exception.values[0].mechanism.meta.signal.number`.
fn crash_signal(event: &SentryEvent) -> Option<u32> {
    let exception = event.exception.as_ref()?.values.first()?;
    let meta = exception.mechanism.as_ref()?.meta.as_ref()?;
    u32::try_from(meta.get("signal")?.get("number")?.as_u64()?).ok()
}

/// Whether a frame's address is a return address (symbolicator's
/// `AdjustInstructionAddr`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Adjustment {
    /// Look up the previous instruction.
    Yes,
    /// Exact program counter; only align it.
    No,
    /// `InstructionInfo::caller_address`: adjust unless this is the crashing
    /// frame (with signal/register heuristics for the crashing frame).
    Auto,
}

struct FrameContext<'a> {
    adjustment: Adjustment,
    crashing: bool,
    signal: Option<u32>,
    registers: Option<&'a serde_json::Value>,
}

fn symbolicate_stacktrace(
    stacktrace: &mut Stacktrace,
    images: &[DebugImage],
    dif_rows: &DifRows,
    signal: Option<u32>,
) -> (usize, usize) {
    let adjustments = caller_adjustments(stacktrace);
    let registers = stacktrace.other.get("registers");
    let frames = std::mem::take(&mut stacktrace.frames);
    let mut out = Vec::with_capacity(frames.len());
    let (mut resolved, mut total) = (0, 0);
    for (frame, adjustment) in frames.into_iter().zip(adjustments) {
        let Some((adjustment, crashing)) = adjustment else {
            out.push(frame);
            continue;
        };
        total += 1;
        let context = FrameContext {
            adjustment,
            crashing,
            signal,
            registers,
        };
        match resolve_frame(&frame, images, dif_rows, &context) {
            Some((image, locations)) => {
                resolved += 1;
                push_resolved(frame, image, locations, &mut out);
            }
            None => out.push(frame),
        }
    }
    stacktrace.frames = out;
    (resolved, total)
}

/// Per-frame adjustment and crashing-frame flag, `None` for frames without an
/// address. Mirrors Sentry's `get_frames_for_symbolication` (stacktrace
/// `instruction_addr_adjustment`: `auto`, `all`, `all_but_first`, `none`)
/// and symbolicator's `AdjustInstructionAddr::{for_frame, default_for_thread}`
/// (frame `adjust_instruction_addr`). Frames are stored caller-first, so the
/// innermost addressed frame is the crashing (or suspended) one.
fn caller_adjustments(stacktrace: &Stacktrace) -> Vec<Option<(Adjustment, bool)>> {
    let frames = &stacktrace.frames;
    let innermost = frames.iter().rposition(has_addr);
    let mut explicit: Vec<Option<bool>> = frames
        .iter()
        .map(|frame| {
            frame
                .other
                .get("adjust_instruction_addr")
                .and_then(|v| v.as_bool())
        })
        .collect();
    let mode = stacktrace
        .other
        .get("instruction_addr_adjustment")
        .and_then(|v| v.as_str());
    match (mode, innermost) {
        (Some("none"), _) => explicit.fill(Some(false)),
        (Some("all"), Some(i)) => explicit[i] = Some(true),
        (Some("all_but_first"), Some(i)) => explicit[i] = Some(false),
        _ => {}
    }
    let default = if frames
        .iter()
        .zip(&explicit)
        .any(|(frame, explicit)| has_addr(frame) && explicit.is_some())
    {
        Adjustment::Yes
    } else {
        Adjustment::Auto
    };
    frames
        .iter()
        .zip(explicit)
        .enumerate()
        .map(|(i, (frame, explicit))| {
            has_addr(frame).then(|| {
                let adjustment = match explicit {
                    Some(true) => Adjustment::Yes,
                    Some(false) => Adjustment::No,
                    None => default,
                };
                // A context frame holds the exact register PC wherever it sits.
                let crashing = Some(i) == innermost || frame.trust.as_deref() == Some("context");
                (adjustment, crashing)
            })
        })
        .collect()
}

/// One level of an inlining hierarchy.
struct ResolvedLocation {
    function: String,
    file: Option<String>,
    line: u32,
}

/// Returns the frame's image and its locations, innermost inlinee first.
fn resolve_frame<'a>(
    frame: &StackFrame,
    images: &'a [DebugImage],
    dif_rows: &DifRows,
    context: &FrameContext<'_>,
) -> Option<(&'a DebugImage, Vec<ResolvedLocation>)> {
    let addr = parse_hex(frame.instruction_addr.as_deref()?)?;
    let (image, relative) = locate(frame, addr, images)?;
    let candidates = dif_rows.get(&image.debug_id)?;
    let file_path = match image.arch.as_deref() {
        Some(arch) => candidates.get(arch).or_else(|| candidates.get(""))?,
        // Without an architecture, never pick arbitrarily between different
        // slices of the same debug ID.
        None if candidates.len() == 1 => candidates.values().next()?,
        None => return None,
    };

    let mmap = get_mmap(file_path)?;
    let symcache = SymCache::parse(&mmap[..]).ok()?;
    if let Some(arch) = image.arch.as_deref()
        && symcache.arch().name() != arch
    {
        return None;
    }
    let lookup_addr = caller_lookup_addr(&symcache, image, relative, context)?;
    let locations: Vec<SourceLocation<'_, '_>> = symcache.lookup(lookup_addr).collect();
    let innermost = locations.first()?;

    // Line tables and inlinee records only exist in real debug info (DWARF,
    // PDB). A symbol-table hit is a nearest-preceding-symbol guess, no better
    // than the dladdr / on-device unwinder name a client already sent.
    let has_debug_info = locations.len() > 1
        || locations
            .iter()
            .any(|location| location.line() > 0 || location.file().is_some());
    if !has_debug_info && (has_client_function(frame) || in_open_tail(&symcache, innermost)) {
        return None;
    }
    Some((image, locations.iter().map(resolved_location).collect()))
}

fn has_client_function(frame: &StackFrame) -> bool {
    frame
        .function
        .as_deref()
        .is_some_and(|f| f != "<redacted>" && !f.is_empty())
}

/// Finds the frame's image and image-relative address. With `addr_mode:
/// "rel:<debug_id>"` (sentry-java's Android ANR thread dumps) or
/// `"rel:<index>"`, `instruction_addr` is already relative to that image;
/// like Sentry, a debug id is tried before an index and an unresolvable mode
/// falls back to absolute addressing.
fn locate<'a>(
    frame: &StackFrame,
    addr: u64,
    images: &'a [DebugImage],
) -> Option<(&'a DebugImage, u64)> {
    if let Some(reference) = frame
        .addr_mode
        .as_deref()
        .and_then(|mode| mode.strip_prefix("rel:"))
    {
        let debug_id = normalize_debug_id(reference);
        if let Some(image) = images.iter().find(|image| image.debug_id == debug_id) {
            return Some((image, addr));
        }
        if let Ok(index) = reference.parse::<usize>() {
            // An index naming an image without a debug id cannot resolve.
            return images
                .iter()
                .find(|image| image.index == index)
                .map(|image| (image, addr));
        }
    }
    // SymCache stores function entries as offsets from the image's
    // link-time base. At runtime the image is loaded at image_addr (with
    // ASLR slide = image_addr - image_vmaddr), so the offset is just
    // iaddr - image_addr — image_vmaddr drops out of the formula.
    let image = find_containing_image(addr, images)?;
    Some((image, addr.checked_sub(image.image_addr?)?))
}

/// Moves a return address back into its call instruction, as symbolicator's
/// `get_relative_caller_addr` does. The heuristics need an absolute address,
/// so relative frames of images without a load address (Android ANR dumps,
/// whose unwinder already adjusted caller PCs) are looked up unchanged.
fn caller_lookup_addr(
    symcache: &SymCache<'_>,
    image: &DebugImage,
    relative: u64,
    context: &FrameContext<'_>,
) -> Option<u64> {
    let Some(base) = image.image_addr.filter(|base| *base != 0) else {
        return Some(relative);
    };
    let absolute = base.checked_add(relative)?;
    // The first page is never code; stepping back from it would underflow.
    if absolute < 0x1000 {
        return Some(relative);
    }
    let arch = symcache.arch();
    let ip_register = if context.crashing {
        arch.cpu_family()
            .ip_register_name()
            .and_then(|name| context.registers?.get(name))
            .and_then(parse_hex_or_int)
    } else {
        None
    };
    let mut info = InstructionInfo::new(arch, absolute);
    info.is_crashing_frame(context.crashing)
        .signal(context.signal)
        .ip_register_value(ip_register);
    let caller = match context.adjustment {
        Adjustment::Yes => info.previous_address(),
        Adjustment::No => info.aligned_address(),
        Adjustment::Auto => info.caller_address(),
    };
    caller.checked_sub(base)
}

/// Unsized symbols (ELF `_fini`, PE exports) extend to the next range
/// marker. When the last symbol is unsized, the SymCache maps everything
/// after it (`.rodata`, `.data`, stack-scanning garbage) to that name.
fn in_open_tail(symcache: &SymCache<'_>, hit: &SourceLocation<'_, '_>) -> bool {
    symcache
        .lookup(u64::from(u32::MAX))
        .next()
        .is_some_and(|tail| tail.function().entry_pc() == hit.function().entry_pc())
}

fn resolved_location(location: &SourceLocation<'_, '_>) -> ResolvedLocation {
    let function = location.function();
    let name = function.name();
    let demangled = Name::new(
        name,
        symbolic::common::NameMangling::Unknown,
        function.language(),
    )
    .demangle(DemangleOptions::name_only())
    .unwrap_or_else(|| name.to_string());
    ResolvedLocation {
        function: demangled,
        file: location
            .file()
            .map(|file| file.full_path())
            .filter(|path| !path.is_empty()),
        line: location.line(),
    }
}

/// Replaces `frame` with one frame per inlining level. Stacktraces are stored
/// caller-first, so the outermost function comes first and the innermost
/// inlinee keeps the original frame's place at the end.
fn push_resolved(
    mut frame: StackFrame,
    image: &DebugImage,
    locations: Vec<ResolvedLocation>,
    out: &mut Vec<StackFrame>,
) {
    let mut locations = locations.into_iter();
    let Some(innermost) = locations.next() else {
        out.push(frame);
        return;
    };
    for location in locations.rev() {
        let mut inlined = frame.clone();
        apply_location(&mut inlined, location, image);
        out.push(inlined);
    }
    apply_location(&mut frame, innermost, image);
    out.push(frame);
}

fn apply_location(frame: &mut StackFrame, location: ResolvedLocation, image: &DebugImage) {
    frame.function = Some(location.function);
    // Replace rather than merge: a client's file/line belongs to the name it
    // guessed (Android dumps even report the symbol offset as `lineno`).
    frame.abs_path = location.file.clone();
    frame.filename = location.file;
    frame.lineno = (location.line > 0).then_some(location.line);
    if frame.in_app.is_none() {
        frame.in_app = Some(heuristic_in_app(image));
    }
}

fn find_containing_image(iaddr: u64, images: &[DebugImage]) -> Option<&DebugImage> {
    images.iter().find(|img| {
        img.image_addr.is_some_and(|base| {
            iaddr >= base && (img.image_size == 0 || iaddr < base.saturating_add(img.image_size))
        })
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

#[cfg(test)]
mod tests {
    use std::io::Write;

    use serde_json::json;
    use symbolic::common::{Arch, Language, NameMangling};
    use symbolic::debuginfo::{FileInfo, Function, LineInfo, Symbol};
    use symbolic::symcache::SymCacheConverter;

    use super::*;
    use crate::config::Config;

    const LIBART: &str = "fcb3cc65-2717-4d60-20b3-ada9810acf55";
    const FLUTTER_WINDOWS: &str = "fd848531-4654-4681-9325-1d5c015a7dad-3";
    const LIBMPV_ARMV7: &str = "7fd0d185-f1f6-f739-83d1-65cc7099d4ad";
    const PLEZY_LINUX: &str = "bae40995-2eee-a99a-1345-4c16633128b3";

    /// A function with real debug info: one line record spanning its body.
    fn debug_function<'a>(
        name: &'a str,
        address: u64,
        size: u64,
        file: &'a str,
        line: u64,
    ) -> Function<'a> {
        Function {
            address,
            size,
            name: Name::new(name, NameMangling::Unmangled, Language::Cpp),
            compilation_dir: b"",
            lines: vec![LineInfo {
                address,
                size: Some(size),
                file: FileInfo::from_path(file.as_bytes()),
                line,
            }],
            inlinees: Vec::new(),
            inline: false,
        }
    }

    fn symbol(name: &str, address: u64, size: u64) -> Symbol<'_> {
        Symbol {
            name: Some(name.into()),
            address,
            size,
        }
    }

    /// Serializes a SymCache the way assembly does and stores it on disk.
    fn symcache_file(
        arch: Arch,
        functions: &[Function<'_>],
        symbols: &[Symbol<'_>],
    ) -> tempfile::NamedTempFile {
        let mut converter = SymCacheConverter::new();
        converter.set_arch(arch);
        for function in functions {
            converter.process_symbolic_function(function);
        }
        for symbol in symbols {
            converter.process_symbolic_symbol(symbol);
        }
        let mut bytes = Vec::new();
        converter.serialize(&mut bytes).unwrap();
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(&bytes).unwrap();
        file
    }

    fn dif_rows(debug_id: &str, cache: &tempfile::NamedTempFile) -> DifRows {
        let path = cache.path().to_string_lossy().into_owned();
        DifRows::from([(
            normalize_debug_id(debug_id),
            HashMap::from([(String::new(), path)]),
        )])
    }

    fn symbolicate(mut event: SentryEvent, rows: &DifRows) -> (SentryEvent, (usize, usize)) {
        let images = parse_images(event.debug_meta.as_ref()).unwrap();
        let counts = lookup_frames(&mut event, &images, rows);
        (event, counts)
    }

    fn frames(event: &SentryEvent) -> Vec<(Option<&str>, Option<&str>, Option<u32>)> {
        event.exception.as_ref().unwrap().values[0]
            .stacktrace
            .as_ref()
            .unwrap()
            .frames
            .iter()
            .map(|f| (f.function.as_deref(), f.filename.as_deref(), f.lineno))
            .collect()
    }

    /// Trimmed from #62 event 194296 (AppExitInfo ANR, Android 14): images
    /// carry build ids but no load address, and native frames are relative to
    /// the image named by `addr_mode`. Names and `lineno` (the symbol offset)
    /// come from the on-device unwinder.
    fn anr_event() -> SentryEvent {
        serde_json::from_value(json!({
            "platform": "java",
            "exception": {"values": [{
                "type": "ApplicationNotResponding",
                "value": "ANR",
                "stacktrace": {"frames": [
                    {"function": "nativePollOnce", "module": "android.os.MessageQueue",
                     "native": true},
                    {"function": "artJniMethodEnd", "lineno": 288,
                     "instruction_addr": "0x004b73c9",
                     "package": "/apex/com.android.art/lib/libart.so",
                     "addr_mode": format!("rel:{LIBART}"), "platform": "native"},
                    {"function": "syscall", "lineno": 28, "instruction_addr": "0x00045700",
                     "package": "/apex/com.android.runtime/lib/bionic/libc.so",
                     "addr_mode": "rel:ce3f5632-7f89-5325-db21-b5725395bc5c",
                     "platform": "native"}
                ]}
            }]},
            "debug_meta": {"images": [
                {"code_file": "/system/lib/libutils.so", "code_id": "51bb27e0ca6c61de7c83abe277ff9aab",
                 "debug_id": "e027bb51-6cca-de61-7c83-abe277ff9aab", "type": "elf"},
                {"code_file": "/apex/com.android.art/lib/libart.so",
                 "code_id": "65ccb3fc1727604d20b3ada9810acf55", "debug_id": LIBART, "type": "elf"},
                {"code_file": "/apex/com.android.runtime/lib/bionic/libc.so",
                 "code_id": "32563fce897f2553db21b5725395bc5c",
                 "debug_id": "ce3f5632-7f89-5325-db21-b5725395bc5c", "type": "elf"}
            ]}
        }))
        .unwrap()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rel_addr_mode_resolves_image_relative_anr_frames() {
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
        // `pc 004b73c9` starts artJniMethodEnd. The device unwinder already
        // adjusted caller PCs, so stepping back would land in the prelude.
        let libart = symcache_file(
            Arch::Arm,
            &[
                debug_function(
                    "art::JniMethodEndPrelude",
                    0x4b7300,
                    0xc8,
                    "art/runtime/jni/jni_internal.cc",
                    41,
                ),
                debug_function(
                    "art::artJniMethodEnd",
                    0x4b73c8,
                    0x80,
                    "art/runtime/entrypoints/quick/quick_jni_entrypoints.cc",
                    172,
                ),
            ],
            &[],
        );
        sqlx::query(
            "INSERT INTO artifact_debug_ids (project_id, debug_id, arch, kind, file_path) \
             VALUES (1, ?, '', 'native', ?)",
        )
        .bind(normalize_debug_id(LIBART))
        .bind(libart.path().to_str().unwrap())
        .execute(db.writer())
        .await
        .unwrap();

        let mut event = anr_event();
        let outcome = symbolicate_native(&mut event, 1, &db).await;
        assert!(
            matches!(
                outcome,
                NativeSymbolicationOutcome::Ok {
                    resolved: 1,
                    total: 2
                }
            ),
            "{outcome:?}"
        );
        assert_eq!(
            frames(&event),
            vec![
                (Some("nativePollOnce"), None, None),
                (
                    Some("art::artJniMethodEnd"),
                    Some("art/runtime/entrypoints/quick/quick_jni_entrypoints.cc"),
                    Some(172),
                ),
                // No libc DIF: the client frame stays as sent.
                (Some("syscall"), None, Some(28)),
            ]
        );
    }

    #[test]
    fn rel_addr_mode_accepts_image_index() {
        let images = parse_images(anr_event().debug_meta.as_ref()).unwrap();
        let frame = StackFrame {
            instruction_addr: Some("0x4b73c9".into()),
            addr_mode: Some("rel:1".into()),
            ..Default::default()
        };
        let (image, relative) = locate(&frame, 0x4b73c9, &images).unwrap();
        assert_eq!(image.debug_id, normalize_debug_id(LIBART));
        assert_eq!(relative, 0x4b73c9);
    }

    /// `flutter_windows.dll` from #1581 event 194476 (sentry-native 0.16.6,
    /// client-side stackwalk: bare addresses, `rip` register, no adjustment
    /// hint).
    fn windows_event(frames: serde_json::Value, adjustment: Option<&str>) -> SentryEvent {
        let mut stacktrace = json!({
            "frames": frames,
            "registers": {"rip": "0x7ffce5945f9b"},
        });
        if let Some(adjustment) = adjustment {
            stacktrace["instruction_addr_adjustment"] = json!(adjustment);
        }
        serde_json::from_value(json!({
            "platform": "native",
            "exception": {"values": [{
                "type": "EXCEPTION_ACCESS_VIOLATION",
                "mechanism": {"type": "signalhandler", "handled": false,
                              "meta": {"signal": {"name": "EXCEPTION_ACCESS_VIOLATION",
                                                  "number": 3221225477u64}}},
                "stacktrace": stacktrace,
            }]},
            "debug_meta": {"images": [{
                "arch": "x86_64", "type": "pe", "debug_id": FLUTTER_WINDOWS,
                "code_file": "D:\\Users\\Ivan\\AppData\\Local\\Programs\\Plezy\\flutter_windows.dll",
                "image_addr": "0x7ffce5930000", "image_size": 21565440
            }]}
        }))
        .unwrap()
    }

    #[test]
    fn inlined_calls_expand_outermost_first() {
        let mut run = debug_function(
            "flutter::FlutterWindowsEngine::Run",
            0x15f00,
            0x100,
            "flutter/shell/platform/windows/flutter_windows_engine.cc",
            210,
        );
        let mut flush = debug_function(
            "fml::MessageLoopImpl::FlushTasks",
            0x15f80,
            0x40,
            "flutter/fml/message_loop_impl.cc",
            125,
        );
        flush.inline = true;
        run.inlinees.push(flush);
        let cache = symcache_file(Arch::Amd64, &[run], &[]);
        let event = windows_event(json!([{"instruction_addr": "0x7ffce5945f90"}]), None);

        let (event, counts) = symbolicate(event, &dif_rows(FLUTTER_WINDOWS, &cache));
        assert_eq!(counts, (1, 1));
        assert_eq!(
            frames(&event),
            vec![
                (
                    Some("flutter::FlutterWindowsEngine::Run"),
                    Some("flutter/shell/platform/windows/flutter_windows_engine.cc"),
                    Some(210),
                ),
                (
                    Some("fml::MessageLoopImpl::FlushTasks"),
                    Some("flutter/fml/message_loop_impl.cc"),
                    Some(125),
                ),
            ]
        );
        let st = event.exception.as_ref().unwrap().values[0]
            .stacktrace
            .as_ref()
            .unwrap();
        assert!(
            st.frames
                .iter()
                .all(|f| f.instruction_addr.as_deref() == Some("0x7ffce5945f90"))
        );
    }

    #[test]
    fn caller_frames_look_up_the_call_instruction() {
        // `Run` ends with the call whose return address starts `Stop`.
        let cache = symcache_file(
            Arch::Amd64,
            &[
                debug_function("Run", 0x15f90, 0xb, "engine.cc", 10),
                debug_function("Stop", 0x15f9b, 0x65, "engine.cc", 20),
            ],
            &[],
        );
        let rows = dif_rows(FLUTTER_WINDOWS, &cache);
        let stack = json!([
            {"instruction_addr": "0x7ffce5945f9b"},
            {"instruction_addr": "0x7ffce5945f9b"}
        ]);
        let names = |adjustment| {
            let (event, _) = symbolicate(windows_event(stack.clone(), adjustment), &rows);
            frames(&event)
                .iter()
                .map(|f| f.0.unwrap().to_string())
                .collect::<Vec<_>>()
        };
        // Caller-first: the outer frame holds a return address, the crashing
        // frame the exact `rip`.
        assert_eq!(names(None), ["Run", "Stop"]);
        assert_eq!(names(Some("auto")), ["Run", "Stop"]);
        assert_eq!(names(Some("all_but_first")), ["Run", "Stop"]);
        assert_eq!(names(Some("all")), ["Run", "Run"]);
        // sentry-native on Android and our minidump stackwalk report
        // already-adjusted caller addresses.
        assert_eq!(names(Some("none")), ["Stop", "Stop"]);
    }

    /// Trimmed from #2869 event 192583 (sentry-native 0.16.6 on Android): the
    /// client names frames on the device from whatever symbol table the shipped
    /// library has, so the name is a nearest-symbol guess without file/line.
    fn android_client_named_event() -> SentryEvent {
        serde_json::from_value(json!({
            "platform": "native",
            "exception": {"values": [{"type": "SIGSEGV", "stacktrace": {
                "instruction_addr_adjustment": "none",
                "frames": [{
                    "function": "uninit_video_chain",
                    "in_app": true,
                    "instruction_addr": "0x6dceffd4",
                    "image_addr": "0x6db09000",
                    "package": "/data/app/com.edde746.plezy-mMis8fYBrIbeQ7wmUHHOZA==/split_config.armeabi_v7a.apk!/lib/armeabi-v7a/libmpv.so"
                }]
            }}]},
            "debug_meta": {"images": [{
                "code_file": "/data/app/com.edde746.plezy-mMis8fYBrIbeQ7wmUHHOZA==/split_config.armeabi_v7a.apk",
                "debug_id": LIBMPV_ARMV7, "image_addr": "0x6db09000", "image_size": 5160960,
                "type": "elf"
            }]}
        }))
        .unwrap()
    }

    #[test]
    fn debug_info_overrides_client_names_but_symbols_do_not() {
        let dwarf = symcache_file(
            Arch::Arm,
            &[debug_function(
                "vo_render_frame",
                0x1e6f00,
                0x100,
                "video/out/vo.c",
                1003,
            )],
            &[],
        );
        let (event, counts) = symbolicate(
            android_client_named_event(),
            &dif_rows(LIBMPV_ARMV7, &dwarf),
        );
        assert_eq!(counts, (1, 1));
        assert_eq!(
            frames(&event),
            [(Some("vo_render_frame"), Some("video/out/vo.c"), Some(1003))]
        );

        let symtab = symcache_file(
            Arch::Arm,
            &[],
            &[symbol("vo_render_frame", 0x1e6f00, 0x100)],
        );
        let (event, counts) = symbolicate(
            android_client_named_event(),
            &dif_rows(LIBMPV_ARMV7, &symtab),
        );
        assert_eq!(counts, (0, 1));
        assert_eq!(frames(&event), [(Some("uninit_video_chain"), None, None)]);
    }

    #[test]
    fn symbols_do_not_claim_addresses_past_an_unsized_last_symbol() {
        // Symbol layout of the 2.23.0 `/opt/plezy/plezy` runner: `_fini` is
        // the last FUNC (in `.fini`, size 0), followed by `.rodata`, `.eh_frame`
        // and data. #2125 event 194419 had stack-scan frames at +0x9ac37
        // reported as `_fini`.
        let cache = symcache_file(
            Arch::Amd64,
            &[],
            &[
                symbol("_init", 0xb000, 0),
                symbol("main", 0xbdd0, 0x54),
                symbol("simdutf_binary_to_base64", 0x76d20, 0x40),
                symbol("_fini", 0x76d60, 0),
            ],
        );
        let event: SentryEvent = serde_json::from_value(json!({
            "platform": "native",
            "exception": {"values": [{"type": "SIGSEGV", "stacktrace": {
                "instruction_addr_adjustment": "none",
                "frames": [
                    {"instruction_addr": "0x5bd055e74ded", "package": "/opt/plezy/plezy", "trust": "scan"},
                    {"instruction_addr": "0x5bd055f03c37", "package": "/opt/plezy/plezy", "trust": "scan"}
                ]
            }}]},
            "debug_meta": {"images": [{
                "code_file": "/opt/plezy/plezy", "debug_id": PLEZY_LINUX,
                "image_addr": "0x5bd055e69000", "image_size": 645392, "type": "elf"
            }]}
        }))
        .unwrap();
        let (event, counts) = symbolicate(event, &dif_rows(PLEZY_LINUX, &cache));
        assert_eq!(counts, (1, 2));
        assert_eq!(
            frames(&event),
            [(Some("main"), None, None), (None, None, None)]
        );
    }
}
