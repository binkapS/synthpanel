//! Fixed-window rate limiter.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Default)]
pub struct RateLimiter {
    inner: Mutex<HashMap<String, Vec<Instant>>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true when the request is allowed. `key` is typically an IP or
    /// an IP+username pair.
    pub fn check(&self, key: &str, max: usize, window: Duration) -> bool {
        let now = Instant::now();
        let mut map = self.inner.lock().unwrap();
        let entry = map.entry(key.to_string()).or_default();
        entry.retain(|ts| now.duration_since(*ts) < window);
        if entry.len() >= max {
            return false;
        }
        entry.push(now);
        true
    }

    pub fn reset(&self, key: &str) {
        self.inner.lock().unwrap().remove(key);
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_up_to_the_limit() {
        let limiter = RateLimiter::new();
        for _ in 0..3 {
            assert!(limiter.check("ip", 3, Duration::from_secs(60)));
        }
        assert!(!limiter.check("ip", 3, Duration::from_secs(60)));
        assert!(limiter.check("other", 3, Duration::from_secs(60)));
    }

    #[test]
    fn reset_clears_the_window() {
        let limiter = RateLimiter::new();
        assert!(limiter.check("ip", 1, Duration::from_secs(60)));
        assert!(!limiter.check("ip", 1, Duration::from_secs(60)));
        limiter.reset("ip");
        assert!(limiter.check("ip", 1, Duration::from_secs(60)));
    }
}
