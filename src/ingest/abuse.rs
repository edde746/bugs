use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

/// In-memory sliding window rate limiter
#[derive(Clone)]
pub struct RateLimiter {
    windows: Arc<Mutex<HashMap<String, RateWindow>>>,
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
        Self {
            windows: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Check if a key is rate-limited. Returns true if allowed.
    pub async fn check(&self, key: &str, limit_per_min: u64) -> bool {
        if limit_per_min == 0 {
            return true;
        }

        let mut windows = self.windows.lock().await;
        let now = Instant::now();

        // Periodically evict stale entries to prevent unbounded growth
        if windows.len() > 100 {
            windows.retain(|_, w| now.duration_since(w.window_start).as_secs() < 120);
        }

        let window = windows.entry(key.to_string()).or_insert(RateWindow {
            count: 0,
            window_start: now,
        });

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
}
