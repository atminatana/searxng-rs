//! Minimum interval between requests to the same engine. Not part of SearXNG;
//! see `docs/adr/0003-engine-rate-limit.md`. A burst of requests from an LLM
//! agent looks like scraping to an engine; spacing them out keeps the
//! instance under the engine's radar.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The next slot is further away than the caller can wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimited {
    pub wait: Duration,
}

/// Per-engine request slots, shared by every search.
pub struct RateLimiter {
    min_interval: Duration,
    next_allowed: Mutex<HashMap<String, Instant>>,
}

impl RateLimiter {
    pub fn new(min_interval: Duration) -> Self {
        Self {
            min_interval,
            next_allowed: Mutex::new(HashMap::new()),
        }
    }

    pub fn is_enabled(&self) -> bool {
        !self.min_interval.is_zero()
    }

    /// Reserve the next request slot of `engine` and return how long the
    /// caller has to wait before sending. When the wait would exceed
    /// `max_wait`, nothing is reserved and `Err(RateLimited)` is returned.
    pub fn reserve(&self, engine: &str, now: Instant, max_wait: Duration) -> Result<Duration, RateLimited> {
        if !self.is_enabled() {
            return Ok(Duration::ZERO);
        }
        let mut slots = self.next_allowed.lock().expect("rate limiter poisoned");
        let slot = slots.get(engine).copied().unwrap_or(now).max(now);
        let wait = slot - now;
        if wait > max_wait {
            return Err(RateLimited { wait });
        }
        slots.insert(engine.to_string(), slot + self.min_interval);
        Ok(wait)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: Duration = Duration::from_secs(1);
    const TIMEOUT: Duration = Duration::from_secs(5);

    #[test]
    fn consecutive_requests_are_spaced_by_the_interval() {
        let now = Instant::now();
        let limiter = RateLimiter::new(SECOND);

        assert_eq!(limiter.reserve("bing", now, TIMEOUT), Ok(Duration::ZERO));
        assert_eq!(limiter.reserve("bing", now, TIMEOUT), Ok(SECOND));
        assert_eq!(limiter.reserve("bing", now, TIMEOUT), Ok(2 * SECOND));
        assert_eq!(limiter.reserve("bing", now + 3 * SECOND, TIMEOUT), Ok(Duration::ZERO), "slots are not banked");
    }

    #[test]
    fn engines_have_independent_slots() {
        let now = Instant::now();
        let limiter = RateLimiter::new(SECOND);

        assert_eq!(limiter.reserve("bing", now, TIMEOUT), Ok(Duration::ZERO));
        assert_eq!(limiter.reserve("duckduckgo", now, TIMEOUT), Ok(Duration::ZERO));
    }

    #[test]
    fn wait_beyond_max_wait_is_refused_without_reserving() {
        let now = Instant::now();
        let limiter = RateLimiter::new(2 * SECOND);

        assert_eq!(limiter.reserve("bing", now, TIMEOUT), Ok(Duration::ZERO));
        assert_eq!(limiter.reserve("bing", now, SECOND), Err(RateLimited { wait: 2 * SECOND }));
        assert_eq!(limiter.reserve("bing", now, TIMEOUT), Ok(2 * SECOND), "refused call did not take a slot");
    }

    #[test]
    fn zero_interval_disables_the_limit() {
        let now = Instant::now();
        let limiter = RateLimiter::new(Duration::ZERO);
        assert!(!limiter.is_enabled());
        for _ in 0..3 {
            assert_eq!(limiter.reserve("bing", now, Duration::ZERO), Ok(Duration::ZERO));
        }
    }
}
