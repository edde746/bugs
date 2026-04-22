//! LRU cache that evicts by both entry count and total byte size.
//!
//! Wraps `lru::LruCache<K, (V, usize)>` and tracks the sum of the byte
//! costs stored alongside each value. Inserting a new entry evicts the
//! least-recently-used entries until the total fits within the byte cap.
//!
//! Needed because the symbolication caches hold `Arc<SourceMap>` /
//! `Arc<Mmap>` values whose actual memory footprint can be many MB per
//! entry — a pure entry-count cap of 64 lets a handful of large inputs
//! pin hundreds of MB of RSS.

use std::hash::Hash;
use std::num::NonZeroUsize;

use lru::LruCache;

pub struct ByteCappedLru<K: Hash + Eq, V> {
    inner: LruCache<K, (V, usize)>,
    max_bytes: usize,
    current_bytes: usize,
}

impl<K: Hash + Eq, V> ByteCappedLru<K, V> {
    pub fn new(max_entries: NonZeroUsize, max_bytes: usize) -> Self {
        Self {
            inner: LruCache::new(max_entries),
            max_bytes,
            current_bytes: 0,
        }
    }

    pub fn resize(&mut self, max_entries: NonZeroUsize, max_bytes: usize) {
        self.inner.resize(max_entries);
        self.max_bytes = max_bytes;
        while self.current_bytes > self.max_bytes {
            if let Some((_, (_, bytes))) = self.inner.pop_lru() {
                self.current_bytes = self.current_bytes.saturating_sub(bytes);
            } else {
                break;
            }
        }
    }

    pub fn get(&mut self, key: &K) -> Option<&V> {
        self.inner.get(key).map(|(v, _)| v)
    }

    pub fn put(&mut self, key: K, value: V, bytes: usize) {
        // Evict by byte budget first. A single outsized entry still gets
        // stored (so we don't livelock rejecting every insert), but we
        // free up room for it.
        if bytes < self.max_bytes {
            while self.current_bytes + bytes > self.max_bytes {
                match self.inner.pop_lru() {
                    Some((_, (_, b))) => self.current_bytes = self.current_bytes.saturating_sub(b),
                    None => break,
                }
            }
        }
        if let Some((_, prev_bytes)) = self.inner.put(key, (value, bytes)) {
            self.current_bytes = self.current_bytes.saturating_sub(prev_bytes);
        }
        self.current_bytes = self.current_bytes.saturating_add(bytes);
    }

    /// Drop an entry if present.
    pub fn pop(&mut self, key: &K) {
        if let Some((_, bytes)) = self.inner.pop(key) {
            self.current_bytes = self.current_bytes.saturating_sub(bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_on_byte_limit() {
        let mut c: ByteCappedLru<String, u8> =
            ByteCappedLru::new(NonZeroUsize::new(100).unwrap(), 10);
        c.put("a".into(), 0, 4);
        c.put("b".into(), 0, 4);
        // Insert 'c' → 4+4+4=12 > 10, evict LRU ("a", least recently used)
        c.put("c".into(), 0, 4);
        assert!(c.get(&"a".into()).is_none());
        assert!(c.get(&"b".into()).is_some());
        assert!(c.get(&"c".into()).is_some());
    }

    #[test]
    fn single_oversized_entry_still_stored() {
        let mut c: ByteCappedLru<String, u8> =
            ByteCappedLru::new(NonZeroUsize::new(100).unwrap(), 10);
        c.put("big".into(), 0, 100);
        assert!(c.get(&"big".into()).is_some());
    }

    #[test]
    fn resize_shrinks_bytes() {
        let mut c: ByteCappedLru<String, u8> =
            ByteCappedLru::new(NonZeroUsize::new(100).unwrap(), 1000);
        for i in 0..10 {
            c.put(format!("k{i}"), 0, 10);
        }
        c.resize(NonZeroUsize::new(100).unwrap(), 25);
        let survivors: usize = (0..10)
            .filter(|i| c.get(&format!("k{i}")).is_some())
            .count();
        assert!(
            survivors <= 3,
            "survivors={survivors} (25 byte budget, 10 bytes/entry)"
        );
    }

    #[test]
    fn overwriting_same_key_updates_bytes() {
        let mut c: ByteCappedLru<String, u8> =
            ByteCappedLru::new(NonZeroUsize::new(100).unwrap(), 10);
        c.put("a".into(), 0, 4);
        c.put("a".into(), 0, 8);
        // Should not double-count: current_bytes == 8, not 12
        c.put("b".into(), 0, 2);
        assert!(c.get(&"a".into()).is_some());
        assert!(c.get(&"b".into()).is_some());
    }
}
