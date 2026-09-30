//! A tiny fixed-window rate limiter, keyed by string (IP or username).
//!
//! Used to slow brute-force login attempts (plan Section 9). Deliberately
//! simple: a lock-protected map of counters with a window start. For a single
//! school server this is more than enough; a distributed deployment would move
//! this to Redis.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Map size at which a sweep of expired windows is triggered. The key is
/// attacker-controlled (client IP), so without a cap a flood from many IPs
/// would grow the map without bound and exhaust memory. Memory is O(live
/// windows), not O(distinct keys ever seen).
const SWEEP_THRESHOLD: usize = 1024;

struct Window {
    count: u32,
    started: Instant,
}

pub struct RateLimiter {
    max_requests: u32,
    window: Duration,
    windows: Mutex<HashMap<String, Window>>,
}

impl RateLimiter {
    pub fn new(max_requests: u32, window_seconds: u64) -> Self {
        Self {
            max_requests,
            window: Duration::from_secs(window_seconds),
            windows: Mutex::new(HashMap::new()),
        }
    }

    /// Record a request. Returns `true` if allowed, `false` if rate-limited.
    pub fn check(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut map = self.windows.lock().expect("rate limiter lock");

        // Periodically drop windows that have fully expired. Bounded work: only
        // runs once the map is large, and leaves it O(live keys).
        if map.len() >= SWEEP_THRESHOLD {
            map.retain(|_, w| now.duration_since(w.started) < self.window);
        }

        let window = map.entry(key.to_string()).or_insert(Window {
            count: 0,
            started: now,
        });
        if now.duration_since(window.started) >= self.window {
            window.count = 0;
            window.started = now;
        }
        if window.count >= self.max_requests {
            false
        } else {
            window.count += 1;
            true
        }
    }

    /// Clear a key's counter (e.g. after a successful login).
    pub fn reset(&self, key: &str) {
        let mut map = self.windows.lock().expect("rate limiter lock");
        map.remove(key);
    }

    /// Number of tracked keys. Exposed for tests.
    #[cfg(test)]
    fn len(&self) -> usize {
        self.windows.lock().expect("rate limiter lock").len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_up_to_limit_then_blocks() {
        let rl = RateLimiter::new(3, 60);
        assert!(rl.check("ip"));
        assert!(rl.check("ip"));
        assert!(rl.check("ip"));
        assert!(!rl.check("ip"), "fourth request must be blocked");
    }

    #[test]
    fn separate_keys_are_independent() {
        let rl = RateLimiter::new(1, 60);
        assert!(rl.check("a"));
        assert!(!rl.check("a"));
        assert!(rl.check("b"));
    }

    #[test]
    fn reset_clears_counter() {
        let rl = RateLimiter::new(1, 60);
        assert!(rl.check("k"));
        assert!(!rl.check("k"));
        rl.reset("k");
        assert!(rl.check("k"));
    }

    #[test]
    fn expired_windows_are_swept_so_map_stays_bounded() {
        // window = 0s: every recorded window expires immediately. Inserting many
        // distinct keys (as an IP flood would) must not grow the map without
        // bound once the sweep threshold is crossed.
        let rl = RateLimiter::new(10, 0);
        for i in 0..(SWEEP_THRESHOLD * 4) {
            rl.check(&format!("ip-{i}"));
        }
        assert!(
            rl.len() <= SWEEP_THRESHOLD + 1,
            "rate-limiter map grew to {} entries",
            rl.len()
        );
    }
}
