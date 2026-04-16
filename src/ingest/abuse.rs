use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Instant;

use lru::LruCache;
use tokio::sync::Mutex;

/// Hard upper bound on tracked keys. Picked to keep peak memory tiny
/// (each entry is ~64 bytes) while still tolerating bursts of legitimate
/// distinct sentry keys. The LRU eviction guarantees the map cannot
/// grow without bound regardless of how many distinct keys an attacker
/// invents.
const MAX_TRACKED_KEYS: usize = 4096;

/// In-memory sliding window rate limiter
#[derive(Clone)]
pub struct RateLimiter {
    windows: Arc<Mutex<LruCache<String, RateWindow>>>,
}

struct RateWindow {
    count: u64,
    window_start: Instant,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl RateLimiter {
    pub fn new() -> Self {
        let cap = NonZeroUsize::new(MAX_TRACKED_KEYS).expect("non-zero cap");
        Self {
            windows: Arc::new(Mutex::new(LruCache::new(cap))),
        }
    }

    /// Check if a key is rate-limited. Returns true if allowed.
    pub async fn check(&self, key: &str, limit_per_min: u64) -> bool {
        if limit_per_min == 0 {
            return true;
        }

        let mut windows = self.windows.lock().await;
        let now = Instant::now();

        if let Some(window) = windows.get_mut(key) {
            // Reset window if it's been more than 60 seconds
            if now.duration_since(window.window_start).as_secs() >= 60 {
                window.count = 0;
                window.window_start = now;
            }
            if window.count >= limit_per_min {
                return false;
            }
            window.count += 1;
            true
        } else {
            // New key: insert with count=1. LRU evicts the oldest if full.
            windows.put(
                key.to_string(),
                RateWindow {
                    count: 1,
                    window_start: now,
                },
            );
            true
        }
    }

    #[cfg(test)]
    async fn tracked_keys(&self) -> usize {
        self.windows.lock().await.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_rate_limiter_allows_under_limit() {
        let rl = RateLimiter::new();
        assert!(rl.check("key1", 10).await);
        assert!(rl.check("key1", 10).await);
        assert!(rl.check("key1", 10).await);
    }

    #[tokio::test]
    async fn test_rate_limiter_blocks_over_limit() {
        let rl = RateLimiter::new();
        for _ in 0..5 {
            assert!(rl.check("key1", 5).await);
        }
        // 6th should be blocked
        assert!(!rl.check("key1", 5).await);
    }

    #[tokio::test]
    async fn test_rate_limiter_separate_keys() {
        let rl = RateLimiter::new();
        for _ in 0..5 {
            rl.check("key1", 5).await;
        }
        // key2 should still be allowed
        assert!(rl.check("key2", 5).await);
    }

    #[tokio::test]
    async fn test_rate_limiter_zero_limit_allows_all() {
        let rl = RateLimiter::new();
        assert!(rl.check("key1", 0).await);
    }

    #[tokio::test]
    async fn test_rate_limiter_bounded_by_lru_cap() {
        let rl = RateLimiter::new();
        // Insert way more distinct keys than the LRU can hold.
        for i in 0..(MAX_TRACKED_KEYS + 500) {
            rl.check(&format!("k{i}"), 5).await;
        }
        // The map must not grow past the configured cap.
        assert!(rl.tracked_keys().await <= MAX_TRACKED_KEYS);
    }
}
