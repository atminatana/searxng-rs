//! Engine suspension after errors. Port of `SuspendedStatus` from
//! `searx/search/processors/abstract.py` plus the `search.suspended_times`
//! table of `searx/settings.yml`: an engine that answered with a CAPTCHA,
//! an access denial or a network error is left alone for a while instead of
//! being hammered (which only makes the block longer).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::config::Search;
use crate::engine::EngineError;

/// Suspend state of one engine.
#[derive(Debug, Default)]
pub struct SuspendedStatus {
    pub continuous_errors: u32,
    suspend_end: Option<Instant>,
    pub suspend_reason: String,
}

impl SuspendedStatus {
    /// Remaining suspension at `now`, if any.
    pub fn remaining(&self, now: Instant) -> Option<Duration> {
        let end = self.suspend_end?;
        (end > now).then(|| end - now)
    }

    /// Count the error and suspend until `now + duration`. A zero `duration`
    /// counts the error without suspending (SearXNG: a configured time of 0).
    pub fn suspend(&mut self, now: Instant, duration: Duration, reason: &str) {
        self.continuous_errors += 1;
        self.suspend_end = Some(now + duration);
        self.suspend_reason = reason.to_string();
    }

    /// Reset after a successful request.
    pub fn resume(&mut self) {
        *self = Self::default();
    }
}

/// Suspend states of all engines, shared by every search.
#[derive(Debug, Default)]
pub struct SuspensionRegistry {
    statuses: Mutex<HashMap<String, SuspendedStatus>>,
}

impl SuspensionRegistry {
    /// `(reason, remaining)` when `engine` is suspended at `now`.
    pub fn active(&self, engine: &str, now: Instant) -> Option<(String, Duration)> {
        let statuses = self.statuses.lock().expect("suspension registry poisoned");
        let status = statuses.get(engine)?;
        let remaining = status.remaining(now)?;
        Some((status.suspend_reason.clone(), remaining))
    }

    pub fn suspend(&self, engine: &str, now: Instant, duration: Duration, reason: &str) {
        let mut statuses = self.statuses.lock().expect("suspension registry poisoned");
        statuses.entry(engine.to_string()).or_default().suspend(now, duration, reason);
        tracing::warn!(
            "[ENGINE] {engine} suspended for {}s: {reason} (continuous errors: {})",
            duration.as_secs(),
            statuses[engine].continuous_errors
        );
    }

    pub fn resume(&self, engine: &str) {
        let mut statuses = self.statuses.lock().expect("suspension registry poisoned");
        if let Some(status) = statuses.get_mut(engine) {
            status.resume();
        }
    }

    pub fn continuous_errors(&self, engine: &str) -> u32 {
        let statuses = self.statuses.lock().expect("suspension registry poisoned");
        statuses.get(engine).map_or(0, |status| status.continuous_errors)
    }
}

/// How long `error` suspends an engine under the `[search]` config: `None`
/// for errors that never suspend, `Some(0s)` when suspension for this error
/// type is disabled (the error is still counted, as in SearXNG).
pub fn suspension_time(error: &EngineError, search: &Search) -> Option<Duration> {
    let times = &search.suspended_times;
    let seconds = match error {
        EngineError::AccessDenied => times.access_denied,
        EngineError::Captcha => times.captcha,
        EngineError::TooManyRequests => times.too_many_requests,
        EngineError::CloudflareCaptcha => times.cf_captcha,
        EngineError::Timeout | EngineError::Request(_) | EngineError::Http(_) => {
            search.max_ban_time_on_fail.min(search.ban_time_on_fail)
        }
        EngineError::Parse(_)
        | EngineError::NoResults
        | EngineError::Suspended { .. }
        | EngineError::RateLimited(_) => return None,
    };
    Some(Duration::from_secs(u64::from(seconds)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: Duration = Duration::from_secs(1);

    #[test]
    fn suspension_expires_and_counts_errors() {
        let start = Instant::now();
        let mut status = SuspendedStatus::default();
        assert_eq!(status.remaining(start), None);

        status.suspend(start, 10 * SECOND, "CAPTCHA required");
        assert_eq!(status.continuous_errors, 1);
        assert_eq!(status.remaining(start + 4 * SECOND), Some(6 * SECOND));
        assert_eq!(status.remaining(start + 10 * SECOND), None, "end is exclusive");

        status.suspend(start + 10 * SECOND, 10 * SECOND, "CAPTCHA required");
        assert_eq!(status.continuous_errors, 2);

        status.resume();
        assert_eq!(status.continuous_errors, 0);
        assert_eq!(status.remaining(start + 11 * SECOND), None);
    }

    #[test]
    fn zero_duration_counts_the_error_without_suspending() {
        let now = Instant::now();
        let mut status = SuspendedStatus::default();
        status.suspend(now, Duration::ZERO, "Access denied");
        assert_eq!(status.continuous_errors, 1);
        assert_eq!(status.remaining(now), None);
    }

    #[test]
    fn registry_tracks_engines_separately() {
        let now = Instant::now();
        let registry = SuspensionRegistry::default();
        registry.suspend("bing", now, 3 * SECOND, "Too many requests");

        assert_eq!(registry.active("bing", now), Some(("Too many requests".to_string(), 3 * SECOND)));
        assert_eq!(registry.active("google", now), None);
        assert_eq!(registry.active("bing", now + 3 * SECOND), None);

        registry.resume("bing");
        assert_eq!(registry.continuous_errors("bing"), 0);
        assert_eq!(registry.active("bing", now), None);
    }

    #[test]
    fn suspension_times_follow_the_config_table() {
        let search = Search::default();
        let seconds = |error: &EngineError| suspension_time(error, &search).map(|d| d.as_secs());

        assert_eq!(seconds(&EngineError::AccessDenied), Some(180));
        assert_eq!(seconds(&EngineError::Captcha), Some(3600));
        assert_eq!(seconds(&EngineError::TooManyRequests), Some(180));
        assert_eq!(seconds(&EngineError::CloudflareCaptcha), Some(1_296_000));
        assert_eq!(seconds(&EngineError::Timeout), Some(5), "min(max_ban_time_on_fail, ban_time_on_fail)");
        assert_eq!(seconds(&EngineError::Request("reset".into())), Some(5));
        assert_eq!(seconds(&EngineError::Http(503)), Some(5));
        assert_eq!(seconds(&EngineError::Parse("bad json".into())), None);
        assert_eq!(seconds(&EngineError::NoResults), None);
        assert_eq!(seconds(&EngineError::RateLimited(1.0)), None);
    }

    #[test]
    fn disabled_suspension_still_counts() {
        let mut search = Search::default();
        search.suspended_times.captcha = 0;
        search.ban_time_on_fail = 0;
        assert_eq!(suspension_time(&EngineError::Captcha, &search), Some(Duration::ZERO));
        assert_eq!(suspension_time(&EngineError::Timeout, &search), Some(Duration::ZERO));
    }
}
