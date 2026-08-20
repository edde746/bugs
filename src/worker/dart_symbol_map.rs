//! Dart/Flutter symbol-map deobfuscation.
//!
//! Flutter builds with `--obfuscate` report exception class names as
//! minified symbols. sentry-cli's `dart-symbol-map upload` pairs the
//! app's obfuscation map (a flat JSON array) with the debug_id of a
//! native debug file (stored by `src/api/chunked_upload.rs`); at event
//! time we look the map up by any of the event's
//! `debug_meta.images[].debug_id` and rewrite
//! `exception.values[*].type` (exact match) plus `Instance of '<sym>'`
//! substrings in `exception.values[*].value` — nothing else. Frames are
//! already covered by native symbolication. Mirrors upstream Sentry's
//! `deobfuscate_exception_type` (src/sentry/lang/dart/utils.py).
//!
//! Map format is a flat array `[deobf0, obf0, deobf1, obf1, …]`: even
//! indices are the plain names, odd indices the obfuscated names. The
//! `["SENTRY_DEBUG_ID_MARKER", <debug_id>]` pair sentry-dart-plugin
//! prepends as a checksum discriminator needs no special handling — it
//! parses into the inert entry `{<debug_id> → "SENTRY_DEBUG_ID_MARKER"}`.

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::{Arc, LazyLock, Mutex};

use tracing::warn;

use crate::db::DbPool;
use crate::sentry_protocol::types::SentryEvent;
use crate::util::byte_capped_lru::ByteCappedLru;
use crate::util::id::normalize_debug_id;

/// `artifact_debug_ids.kind` for stored Dart symbol maps.
pub const KIND: &str = "dart_symbol_map";

/// file_path → parsed {obfuscated → deobfuscated} map. One Flutter app
/// build ships one map (a few MB of JSON), so a small cache covers the
/// hot path; the byte cost is the summed string lengths. Invalidated on
/// upload via `invalidate_map_path` for the same reason as
/// `NATIVE_CACHE`: uploads atomically rename over the stored file.
static MAP_CACHE: LazyLock<Mutex<ByteCappedLru<String, Arc<HashMap<String, String>>>>> =
    LazyLock::new(|| {
        Mutex::new(ByteCappedLru::new(
            NonZeroUsize::new(8).unwrap(),
            64 * 1024 * 1024,
        ))
    });

/// Drop a cached parsed map. The upload handler calls this after a
/// successful atomic rename so subsequent events re-read the new bytes.
pub fn invalidate_map_path(file_path: &str) {
    MAP_CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .pop(&file_path.to_string());
}

/// Rewrites obfuscated Dart exception type/value in place. Best-effort:
/// any failure logs and leaves the event unchanged. Runs after
/// symbolication and before fingerprint/title derivation so the
/// deobfuscated names flow into grouping, titles, and search columns.
pub async fn deobfuscate_dart_event(event: &mut SentryEvent, project_id: i64, db: &DbPool) {
    if !is_dart_sdk(event) {
        return;
    }
    if !event
        .exception
        .as_ref()
        .is_some_and(|e| !e.values.is_empty())
    {
        return;
    }
    let debug_ids = image_debug_ids(event.debug_meta.as_ref());
    if debug_ids.is_empty() {
        return;
    }

    let file_path = match load_map_path(db, project_id, &debug_ids).await {
        Ok(Some(path)) => path,
        Ok(None) => return,
        Err(e) => {
            warn!("dart symbol map DB query failed: {e}");
            return;
        }
    };
    let Some(map) = load_map(&file_path).await else {
        return;
    };

    if let Some(exception) = event.exception.as_mut() {
        for value in exception.values.iter_mut() {
            if let Some(mapped) = value
                .exception_type
                .as_ref()
                .and_then(|t| map.get(t.as_str()))
            {
                value.exception_type = Some(mapped.clone());
            }
            if let Some(rewritten) = value
                .value
                .as_ref()
                .and_then(|v| rewrite_instance_of(v, &map))
            {
                value.value = Some(rewritten);
            }
        }
    }
}

fn is_dart_sdk(event: &SentryEvent) -> bool {
    matches!(
        event
            .sdk
            .as_ref()
            .and_then(|s| s.get("name"))
            .and_then(|n| n.as_str()),
        Some("sentry.dart") | Some("sentry.dart.flutter")
    )
}

/// All normalized `debug_meta.images[].debug_id` values, deduplicated.
/// Unlike native symbolication we do not require `image_addr`: the map
/// is keyed by image identity alone, matching upstream's
/// `get_debug_meta_image_ids`.
fn image_debug_ids(debug_meta: Option<&serde_json::Value>) -> Vec<String> {
    let Some(images) = debug_meta
        .and_then(|m| m.get("images"))
        .and_then(|i| i.as_array())
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for img in images {
        let id = normalize_debug_id(img.get("debug_id").and_then(|v| v.as_str()).unwrap_or(""));
        if !id.is_empty() && !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

/// First stored map matching any of the event's image debug ids. There
/// is one mapping file per Flutter build, so first-hit semantics match
/// upstream's `generate_dart_symbols_map`.
async fn load_map_path(
    db: &DbPool,
    project_id: i64,
    debug_ids: &[String],
) -> Result<Option<String>, sqlx::Error> {
    let placeholders = std::iter::repeat_n("?", debug_ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT file_path FROM artifact_debug_ids \
         WHERE project_id = ? AND kind = '{KIND}' AND debug_id IN ({placeholders}) \
         LIMIT 1"
    );
    let mut q = sqlx::query_as::<_, (String,)>(&sql).bind(project_id);
    for id in debug_ids {
        q = q.bind(id);
    }
    Ok(q.fetch_optional(db.reader()).await?.map(|(path,)| path))
}

async fn load_map(file_path: &str) -> Option<Arc<HashMap<String, String>>> {
    if let Some(map) = MAP_CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&file_path.to_string())
    {
        return Some(map.clone());
    }

    let bytes = match tokio::fs::read(file_path).await {
        Ok(bytes) => bytes,
        Err(e) => {
            warn!(file_path, "dart symbol map read failed: {e}");
            return None;
        }
    };
    let map = match tokio::task::block_in_place(|| parse_map(&bytes)) {
        Ok(map) => Arc::new(map),
        Err(e) => {
            warn!(file_path, "dart symbol map parse failed: {e}");
            return None;
        }
    };

    // Approximate heap cost: string contents plus per-entry HashMap
    // overhead (two String headers + bucket slot).
    let cost = map
        .iter()
        .map(|(k, v)| k.len() + v.len() + 64)
        .sum::<usize>();
    MAP_CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .put(file_path.to_string(), map.clone(), cost);
    Some(map)
}

/// `[deobf0, obf0, deobf1, obf1, …]` → `{obf → deobf}`. Pairs with
/// non-string members are skipped; assemble already validated array-ness
/// and even length, so a trailing element cannot occur for stored maps.
fn parse_map(bytes: &[u8]) -> Result<HashMap<String, String>, serde_json::Error> {
    let entries: Vec<serde_json::Value> = serde_json::from_slice(bytes)?;
    let mut map = HashMap::with_capacity(entries.len() / 2);
    for pair in entries.chunks_exact(2) {
        if let (Some(deobfuscated), Some(obfuscated)) = (pair[0].as_str(), pair[1].as_str()) {
            map.insert(obfuscated.to_string(), deobfuscated.to_string());
        }
    }
    Ok(map)
}

const INSTANCE_PREFIX: &str = "Instance of '";

/// Replaces `<sym>` in every `Instance of '<sym>'` occurrence via the
/// map. Returns `None` when nothing changed so callers avoid replacing
/// the original allocation for the common unobfuscated case.
fn rewrite_instance_of(value: &str, map: &HashMap<String, String>) -> Option<String> {
    let mut out = String::new();
    let mut changed = false;
    let mut rest = value;
    while let Some(start) = rest.find(INSTANCE_PREFIX) {
        let sym_start = start + INSTANCE_PREFIX.len();
        let Some(sym_len) = rest[sym_start..].find('\'') else {
            break;
        };
        let sym = &rest[sym_start..sym_start + sym_len];
        out.push_str(&rest[..sym_start]);
        match map.get(sym) {
            Some(mapped) => {
                out.push_str(mapped);
                changed = true;
            }
            None => out.push_str(sym),
        }
        // Keep the closing quote in `rest` so it is copied verbatim.
        rest = &rest[sym_start + sym_len..];
    }
    if !changed {
        return None;
    }
    out.push_str(rest);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map_of(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(obf, deobf)| (obf.to_string(), deobf.to_string()))
            .collect()
    }

    #[test]
    fn rewrites_instance_of_occurrences() {
        let map = map_of(&[("aB", "SecretClass"), ("cD", "OtherClass")]);
        assert_eq!(
            rewrite_instance_of("Bad state: Instance of 'aB' and Instance of 'cD'", &map),
            Some("Bad state: Instance of 'SecretClass' and Instance of 'OtherClass'".to_string())
        );
    }

    #[test]
    fn unknown_symbols_leave_value_unchanged() {
        let map = map_of(&[("aB", "SecretClass")]);
        assert_eq!(rewrite_instance_of("Instance of 'zZ'", &map), None);
        assert_eq!(rewrite_instance_of("no instances here", &map), None);
    }

    #[test]
    fn unterminated_instance_prefix_is_copied_verbatim() {
        let map = map_of(&[("aB", "SecretClass")]);
        assert_eq!(
            rewrite_instance_of("Instance of 'aB' then Instance of 'broken", &map),
            Some("Instance of 'SecretClass' then Instance of 'broken".to_string())
        );
    }

    #[test]
    fn parse_map_pairs_odd_first_even_second() {
        // [deobf, obf, ...] — the plugin's marker pair becomes inert.
        let json = br#"["SENTRY_DEBUG_ID_MARKER", "abc123", "SecretClass", "aB"]"#;
        let map = parse_map(json).unwrap();
        assert_eq!(map.get("aB").map(String::as_str), Some("SecretClass"));
        assert_eq!(
            map.get("abc123").map(String::as_str),
            Some("SENTRY_DEBUG_ID_MARKER")
        );
        assert_eq!(map.get("SecretClass"), None);
    }

    #[test]
    fn parse_map_skips_non_string_pairs() {
        let json = br#"[1, 2, "Real", "rB"]"#;
        let map = parse_map(json).unwrap();
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("rB").map(String::as_str), Some("Real"));
    }
}
