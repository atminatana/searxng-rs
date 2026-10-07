//! Short-lived cache of successful engine results. Not part of SearXNG; see
//! `docs/adr/0002-engine-result-cache.md`. LLM clients repeat the same query
//! often, and every repeat that reaches the engine counts against the
//! instance's reputation there.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::engine::{EngineParams, EngineResults};

/// Everything that changes what an engine returns.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    engine: String,
    query: String,
    language: Option<String>,
    safesearch: u8,
    pageno: u32,
    category: String,
}

impl CacheKey {
    pub fn new(engine: &str, params: &EngineParams) -> Self {
        Self {
            engine: engine.to_string(),
            query: params.query.clone(),
            language: params.language().map(str::to_string),
            safesearch: params.safesearch,
            pageno: params.pageno,
            category: params.category.clone(),
        }
    }
}

struct Entry {
    results: EngineResults,
    inserted_at: Instant,
}

#[derive(Default)]
struct CacheState {
    entries: HashMap<CacheKey, Entry>,
    /// Keys in insertion order; the front is the oldest.
    order: VecDeque<CacheKey>,
}

/// Results cache with a fixed lifetime per entry and a bound on entries.
pub struct ResultCache {
    ttl: Duration,
    max_entries: usize,
    state: Mutex<CacheState>,
}

impl ResultCache {
    pub fn new(ttl: Duration, max_entries: usize) -> Self {
        Self {
            ttl,
            max_entries,
            state: Mutex::new(CacheState::default()),
        }
    }

    pub fn is_enabled(&self) -> bool {
        !self.ttl.is_zero() && self.max_entries > 0
    }

    /// Cached results for `key` that are still fresh at `now`.
    pub fn get(&self, key: &CacheKey, now: Instant) -> Option<EngineResults> {
        if !self.is_enabled() {
            return None;
        }
        let state = self.state.lock().expect("result cache poisoned");
        let entry = state.entries.get(key)?;
        self.is_fresh(entry, now).then(|| entry.results.clone())
    }

    /// Store `results`; the oldest entries make room when the cache is full.
    pub fn insert(&self, key: CacheKey, results: EngineResults, now: Instant) {
        if !self.is_enabled() {
            return;
        }
        let mut state = self.state.lock().expect("result cache poisoned");
        if state.entries.remove(&key).is_some() {
            state.order.retain(|k| *k != key);
        }
        self.evict_expired(&mut state, now);
        while state.entries.len() >= self.max_entries {
            let Some(oldest) = state.order.pop_front() else {
                break;
            };
            state.entries.remove(&oldest);
        }
        state.entries.insert(key.clone(), Entry { results, inserted_at: now });
        state.order.push_back(key);
    }

    pub fn len(&self) -> usize {
        self.state.lock().expect("result cache poisoned").entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn is_fresh(&self, entry: &Entry, now: Instant) -> bool {
        now.saturating_duration_since(entry.inserted_at) < self.ttl
    }

    /// Entries expire in insertion order, so expired ones are at the front.
    fn evict_expired(&self, state: &mut CacheState, now: Instant) {
        while let Some(oldest) = state.order.front() {
            let expired = state
                .entries
                .get(oldest)
                .is_some_and(|entry| !self.is_fresh(entry, now));
            if !expired {
                break;
            }
            let oldest = state.order.pop_front().expect("front exists");
            state.entries.remove(&oldest);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::SearchResult;

    const SECOND: Duration = Duration::from_secs(1);

    fn params(query: &str) -> EngineParams {
        EngineParams {
            query: query.to_string(),
            languages: vec![],
            safesearch: 0,
            pageno: 1,
            category: "general".to_string(),
        }
    }

    fn results(url: &str) -> EngineResults {
        let mut results = EngineResults::default();
        results.add(SearchResult {
            url: url.to_string(),
            ..Default::default()
        });
        results
    }

    #[test]
    fn key_covers_every_result_shaping_parameter() {
        let base = params("rust");
        let mut other_language = params("rust");
        other_language.languages = vec!["de".to_string()];
        let mut all_language = params("rust");
        all_language.languages = vec!["all".to_string()];
        let mut other_page = params("rust");
        other_page.pageno = 2;
        let mut other_safesearch = params("rust");
        other_safesearch.safesearch = 2;

        let key = CacheKey::new("bing", &base);
        assert_eq!(key, CacheKey::new("bing", &all_language), "\"all\" is no language");
        assert_ne!(key, CacheKey::new("duckduckgo", &base));
        assert_ne!(key, CacheKey::new("bing", &params("rust lang")));
        assert_ne!(key, CacheKey::new("bing", &other_language));
        assert_ne!(key, CacheKey::new("bing", &other_page));
        assert_ne!(key, CacheKey::new("bing", &other_safesearch));
    }

    #[test]
    fn entries_live_for_ttl() {
        let now = Instant::now();
        let cache = ResultCache::new(10 * SECOND, 100);
        let key = CacheKey::new("bing", &params("rust"));

        cache.insert(key.clone(), results("https://a"), now);
        assert_eq!(cache.get(&key, now + 9 * SECOND).map(|r| r.results.len()), Some(1));
        assert!(cache.get(&key, now + 10 * SECOND).is_none(), "ttl is exclusive");
    }

    #[test]
    fn oldest_entries_are_evicted_when_full() {
        let now = Instant::now();
        let cache = ResultCache::new(10 * SECOND, 2);
        let keys: Vec<CacheKey> = ["a", "b", "c"]
            .iter()
            .map(|q| CacheKey::new("bing", &params(q)))
            .collect();

        for (i, key) in keys.iter().enumerate() {
            cache.insert(key.clone(), results("https://x"), now + i as u32 * SECOND);
        }

        assert_eq!(cache.len(), 2);
        assert!(cache.get(&keys[0], now + 3 * SECOND).is_none(), "oldest evicted");
        assert!(cache.get(&keys[1], now + 3 * SECOND).is_some());
        assert!(cache.get(&keys[2], now + 3 * SECOND).is_some());
    }

    #[test]
    fn reinsert_refreshes_the_entry() {
        let now = Instant::now();
        let cache = ResultCache::new(10 * SECOND, 100);
        let key = CacheKey::new("bing", &params("rust"));

        cache.insert(key.clone(), results("https://old"), now);
        cache.insert(key.clone(), results("https://new"), now + 5 * SECOND);

        assert_eq!(cache.len(), 1);
        let fresh = cache.get(&key, now + 12 * SECOND).expect("refreshed entry");
        assert_eq!(fresh.results[0].url, "https://new");
    }

    #[test]
    fn expired_entries_make_room_before_eviction() {
        let now = Instant::now();
        let cache = ResultCache::new(10 * SECOND, 2);
        let old = CacheKey::new("bing", &params("old"));
        let kept = CacheKey::new("bing", &params("kept"));
        let new = CacheKey::new("bing", &params("new"));

        cache.insert(old, results("https://x"), now);
        cache.insert(kept.clone(), results("https://x"), now + 5 * SECOND);
        cache.insert(new, results("https://x"), now + 11 * SECOND);

        assert_eq!(cache.len(), 2);
        assert!(cache.get(&kept, now + 11 * SECOND).is_some(), "fresh entry survives");
    }

    #[test]
    fn zero_ttl_or_zero_capacity_disables_the_cache() {
        let now = Instant::now();
        let key = CacheKey::new("bing", &params("rust"));
        for cache in [ResultCache::new(Duration::ZERO, 100), ResultCache::new(10 * SECOND, 0)] {
            assert!(!cache.is_enabled());
            cache.insert(key.clone(), results("https://x"), now);
            assert!(cache.is_empty());
            assert!(cache.get(&key, now).is_none());
        }
    }
}
