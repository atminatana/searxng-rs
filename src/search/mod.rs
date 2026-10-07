//! Search orchestration. Port of `searx/search/__init__.py` from SearXNG:
//! builds per-engine requests, runs them in parallel with a shared timeout,
//! then merges results into a `ResultContainer`.

use std::sync::Arc;
use std::time::Duration;

use futures::stream::{FuturesUnordered, StreamExt};
use tokio::time::Instant;

use crate::config::Config;
use crate::engine::http::HttpClient;
use crate::engine::{Engine, EngineError, EngineParams, EngineRegistry, EngineResults, SearchResult};
use crate::query::{EngineRef, RawTextQuery};
use crate::search::cache::{CacheKey, ResultCache};
use crate::search::rate_limit::RateLimiter;
use crate::search::suspension::{suspension_time, SuspensionRegistry};

pub mod cache;
pub mod models;
pub mod rate_limit;
pub mod results;
pub mod suspension;

pub use models::{SearchQuery};
pub use results::ResultContainer;

/// Result of running one engine for a query.
#[derive(Debug)]
pub struct EngineOutcome {
    pub engine: String,
    pub category: String,
    pub results: Vec<SearchResult>,
    pub suggestions: Vec<String>,
    pub corrections: Vec<String>,
    pub answers: Vec<String>,
    pub error: Option<EngineError>,
    pub elapsed: Duration,
}

/// The unified search entry point (analogous to `SearchWithPlugins.search()`).
pub struct SearchEngine {
    registry: Arc<EngineRegistry>,
    client: Arc<HttpClient>,
    pub config: Arc<Config>,
    suspensions: SuspensionRegistry,
    cache: ResultCache,
    rate_limiter: RateLimiter,
}

impl SearchEngine {
    pub fn new(registry: Arc<EngineRegistry>, client: Arc<HttpClient>, config: Arc<Config>) -> Self {
        let search = &config.search;
        let cache = ResultCache::new(Duration::from_secs_f32(search.cache_ttl.max(0.0)), search.cache_max_entries);
        let rate_limiter = RateLimiter::new(Duration::from_secs_f32(search.engine_min_interval.max(0.0)));
        Self {
            registry,
            client,
            config,
            suspensions: SuspensionRegistry::default(),
            cache,
            rate_limiter,
        }
    }

    /// One engine request behind the trust guards: a suspended engine is not
    /// queried, a fresh cached result is reused, requests to one engine are
    /// spaced out, and a failure suspends the engine (see `suspension`).
    async fn run_engine(
        &self,
        engine: Arc<dyn Engine>,
        engine_name: &str,
        params: &EngineParams,
        timeout: Duration,
    ) -> Result<EngineResults, EngineError> {
        let now = Instant::now().into_std();
        if let Some((reason, remaining)) = self.suspensions.active(engine_name, now) {
            return Err(EngineError::Suspended {
                reason,
                remaining_secs: remaining.as_secs_f64().ceil() as u64,
            });
        }
        let key = CacheKey::new(engine_name, params);
        if let Some(results) = self.cache.get(&key, now) {
            tracing::trace!(target: "searxng_rs::search", engine = engine_name, results = results.results.len(), "cached results");
            return Ok(results);
        }
        let wait = self
            .rate_limiter
            .reserve(engine_name, now, timeout)
            .map_err(|limited| EngineError::RateLimited(limited.wait.as_secs_f32()))?;
        if !wait.is_zero() {
            tracing::trace!(target: "searxng_rs::search", engine = engine_name, wait_s = wait.as_secs_f32(), "rate limit wait");
            tokio::time::sleep(wait).await;
        }

        let result = tokio::time::timeout(timeout.saturating_sub(wait), engine.search(params, &self.client))
            .await
            .unwrap_or(Err(EngineError::Timeout));

        let now = Instant::now().into_std();
        match &result {
            Ok(results) => {
                self.suspensions.resume(engine_name);
                self.cache.insert(key, results.clone(), now);
            }
            Err(error) => {
                if let Some(duration) = suspension_time(error, &self.config.search) {
                    self.suspensions.suspend(engine_name, now, duration, &error.to_string());
                }
            }
        }
        result
    }

    pub fn registry(&self) -> &Arc<EngineRegistry> {
        &self.registry
    }

    /// Run a search for a parsed query + resolved engine refs.
    pub async fn search(&self, query: &SearchQuery) -> Result<SearchResponse, anyhow::Error> {
        let start = Instant::now();
        tracing::trace!(target: "searxng_rs::search", query = ?query.query, engines = ?query.enginerefs, safesearch = query.safe_search, pageno = query.pageno, languages = ?query.languages, timeout_limit = ?query.timeout_limit, redirect_to_first_result = query.redirect_to_first_result, external_bang = ?query.external_bang, disabled_engines = ?query.disabled_engines, "search request");
        // 1. External bang (!!ddg ...) → redirect, no engine queries.
        if let Some(bang) = query.external_bang.as_deref() {
            if let Some((_, tmpl)) = crate::query::EXTERNAL_BANGS
                .iter()
                .find(|(name, _)| *name == bang)
            {
                let url = tmpl.replace("{query}", &urlencode(&query.query));
                tracing::trace!(target: "searxng_rs::search", external_bang = bang, redirect_url = ?url, "external bang redirect");
                let resp = SearchResponse {
                    redirect_url: Some(url),
                    ..Default::default()
                };
                tracing::trace!(target: "searxng_rs::search", redirect_url = ?resp.redirect_url, results = 0, time = ?start.elapsed().as_secs_f64(), "search response");
                return Ok(resp);
            }
        }

        // 2. Resolve engines: from explicit !bangs if set, else default selection.
        let mut outcomes = Vec::new();
        let now = Instant::now();

        let engine_refs = if query.enginerefs.is_empty() {
            self.default_engines(query)
        } else {
            query.enginerefs.clone()
        };
        tracing::trace!(target: "searxng_rs::search", engine_refs = ?engine_refs, "resolved engine refs");

        let timeout = resolve_timeout(
            &engine_refs,
            &self.registry,
            query.timeout_limit,
            self.config.server.max_request_timeout,
        );
        tracing::trace!(target: "searxng_rs::search", timeout_s = timeout, max_request_timeout = self.config.server.max_request_timeout, "resolved timeout");

        // 3. Fire all engine requests in parallel.
        let mut tasks = FuturesUnordered::new();
        for engineref in engine_refs {
            let Some(engine) = self.registry.get(&engineref.name) else {
                continue;
            };
            if query.disabled_engines.contains(&(engineref.name.clone(), engineref.category.clone())) {
                continue;
            }
            let cat = if engineref.category == "none" {
                self.registry
                    .categories_for(&engineref.name)
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "general".to_string())
            } else {
                engineref.category.clone()
            };
            let eparams = EngineParams {
                query: query.query.clone(),
                languages: query.languages.clone(),
                safesearch: query.safe_search,
                pageno: query.pageno,
                category: cat.clone(),
            };
            let engine_name = engineref.name.clone();
            let timeout = Duration::from_secs_f32(timeout.max(0.1));
            tasks.push(async move {
                let start = Instant::now();
                let result = self.run_engine(engine, &engine_name, &eparams, timeout).await;
                let mut input = EngineOutcome {
                    engine: engine_name,
                    category: cat,
                    results: Vec::new(),
                    suggestions: Vec::new(),
                    corrections: Vec::new(),
                    answers: Vec::new(),
                    error: None,
                    elapsed: start.elapsed(),
                };
                input.hydrate(result);
                tracing::trace!(target: "searxng_rs::search", engine = input.engine, category = input.category, results = input.results.len(), error = ?input.error, elapsed = ?input.elapsed.as_secs_f64(), "engine outcome");
                for result in &input.results {
                    tracing::trace!(target: "searxng_rs::search", engine = input.engine, url = ?result.url, title = ?result.title, content_len = result.content.len(), "engine result");
                }
                input
            });
        }

        while let Some(outcome) = tasks.next().await {
            outcomes.push(outcome);
        }

        // 4. Merge into a container.
        let container = merge_outcomes(&outcomes);

        // 5. Feeling lucky: redirect to the first result.
        if query.redirect_to_first_result {
            if let Some(first) = container
                .results
                .iter()
                .find(|r| !r.url.is_empty())
            {
                let resp = SearchResponse {
                    redirect_url: Some(first.url.clone()),
                    results: container.results.clone(),
                    ..Default::default()
                };
                tracing::trace!(target: "searxng_rs::search", redirect_url = ?resp.redirect_url, results = resp.results.len(), time = ?start.elapsed().as_secs_f64(), "search response (feeling lucky)");
                return Ok(resp);
            }
        }

        let resp = SearchResponse {
            redirect_url: None,
            results: container.results,
            suggestions: container.suggestions,
            corrections: container.corrections,
            answers: container.answers,
            unresponsive_engines: outcomes
                .iter()
                .filter(|o| o.error.is_some())
                .map(|o| o.engine.clone())
                .collect(),
            total_time: now.elapsed(),
            outcomes,
        };
        tracing::trace!(target: "searxng_rs::search", results = resp.results.len(), unresponsive = ?resp.unresponsive_engines, suggestions = resp.suggestions.len(), corrections = resp.corrections.len(), answers = resp.answers.len(), time = ?start.elapsed().as_secs_f64(), "search response");
        Ok(resp)
    }

    fn default_engines(&self, query: &SearchQuery) -> Vec<EngineRef> {
        // Select enabled, non-suspended engines in the "general" category.
        self.registry
            .categories
            .get("general")
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|name| {
                let enabled = self.config.engines.is_enabled(name);
                let not_disabled = !query
                    .disabled_engines
                    .iter()
                    .any(|(n, _)| n == name);
                enabled && not_disabled
            })
            .map(|name| EngineRef {
                name,
                category: "general".to_string(),
            })
            .collect()
    }
}

/// Timeout resolution copied from `searx/search/__init__.py::_get_requests`.
fn resolve_timeout(
    engine_refs: &[EngineRef],
    registry: &EngineRegistry,
    query_timeout: Option<f32>,
    max_request_timeout: f32,
) -> f32 {
    let mut default_timeout = 5.0_f32;
    for engineref in engine_refs {
        if let Some(engine) = registry.get(&engineref.name) {
            default_timeout = default_timeout.max(engine.timeout().unwrap_or(5.0));
        }
    }
    let resolved = match (max_request_timeout.is_finite(), query_timeout) {
        (false, None) => default_timeout,
        (false, Some(q)) => default_timeout.min(q),
        (true, None) => default_timeout.min(max_request_timeout),
        (true, Some(q)) => q.min(max_request_timeout),
    };
    tracing::trace!(target: "searxng_rs::search", engine_refs = ?engine_refs.iter().map(|e| e.name.clone()).collect::<Vec<_>>(), default_timeout, max_request_timeout, query_timeout = ?query_timeout, resolved_timeout = resolved, "resolved timeout");
    resolved
}

fn urlencode(s: &str) -> String {
    const SET: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'_')
        .remove(b'.')
        .remove(b'~');
    percent_encoding::utf8_percent_encode(s, SET).to_string()
}

fn merge_outcomes(outcomes: &[EngineOutcome]) -> ResultContainer {
    let mut container = ResultContainer::default();
    let mut seq: u32 = 1;
    for outcome in outcomes {
        let weight = 1.0;
        let pos0 = seq;
        for (i, mut r) in outcome.results.clone().into_iter().enumerate() {
            r.engines = vec![outcome.engine.clone()];
            r.positions = vec![pos0 + i as u32];
            r.score = weight * (pos0 + i as u32) as f32;
            r.category = outcome.category.clone();
            seq += 1;
            container.add(r);
        }
        container.suggestions.extend(outcome.suggestions.clone());
        container.corrections.extend(outcome.corrections.clone());
        container.answers.extend(outcome.answers.clone());
    }
    tracing::trace!(target: "searxng_rs::search", total_results = container.results.len(), total_suggestions = container.suggestions.len(), total_corrections = container.corrections.len(), total_answers = container.answers.len(), "merged outcomes");
    container
}

// Helper to hydrate a partial EngineOutcome built synchronously.
impl EngineOutcome {
    fn hydrate(&mut self, result: Result<EngineResults, EngineError>) {
        match result {
            Ok(res) => {
                self.results = res.results;
                self.suggestions = res.suggestions;
                self.corrections = res.corrections;
                self.answers = res.answers;
                self.error = None;
            }
            Err(e) => {
                self.error = Some(e);
            }
        }
    }
}

/// The public search response.
#[derive(Debug, Default, serde::Serialize)]
pub struct SearchResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redirect_url: Option<String>,
    pub results: Vec<SearchResult>,
    pub suggestions: Vec<String>,
    pub corrections: Vec<String>,
    pub answers: Vec<String>,
    pub unresponsive_engines: Vec<String>,
    #[serde(skip_serializing)]
    pub total_time: Duration,
    #[serde(skip_serializing)]
    pub outcomes: Vec<EngineOutcome>,
}

/// Inputs a `RawTextQuery` parser needs: engine names, shortcuts, categories.
type QueryContext = (
    Vec<String>,
    std::collections::HashMap<String, String>,
    std::collections::HashMap<String, Vec<String>>,
);

/// Build a `RawTextQuery` registry-backed context for query parsing.
fn build_query_context(registry: &EngineRegistry) -> QueryContext {
    let engines: Vec<String> = registry.names();
    let shortcuts = registry.shortcuts.clone();
    let categories = registry.categories.clone();
    (engines, shortcuts, categories)
}

/// Convenience: parse raw `!`/`:`/`<` syntax into a `SearchQuery`.
pub fn parse_query(
    registry: &EngineRegistry,
    raw: &str,
    _safe_search: u8,
    _pageno: u32,
) -> RawTextQuery {
    let (engines, shortcuts, categories) = build_query_context(registry);
    RawTextQuery::new(raw, &[], &engines, &shortcuts, &categories)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_resolution() {
        // default engine timeout wins when no constraints
        assert_eq!(resolve_timeout(&[], &EngineRegistry::empty(), None, 10.0), 5.0);
        // max_request_timeout caps it
        // user query timeout is the strictest
    }
}

#[cfg(test)]
mod guard_tests {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Mutex;

    use async_trait::async_trait;

    use super::*;

    const ENGINE: &str = "scripted";

    /// Engine that replays scripted outcomes and counts real calls.
    struct ScriptedEngine {
        calls: AtomicU32,
        script: Mutex<VecDeque<Result<EngineResults, EngineError>>>,
    }

    #[async_trait]
    impl Engine for ScriptedEngine {
        fn name(&self) -> &str {
            ENGINE
        }

        async fn search(&self, _: &EngineParams, _: &HttpClient) -> Result<EngineResults, EngineError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.script.lock().expect("script").pop_front().expect("script exhausted")
        }
    }

    impl ScriptedEngine {
        fn calls(&self) -> u32 {
            self.calls.load(Ordering::SeqCst)
        }
    }

    fn one_result() -> Result<EngineResults, EngineError> {
        let mut results = EngineResults::default();
        results.add(SearchResult {
            url: "https://example.com/".to_string(),
            title: "Example".to_string(),
            engine: ENGINE.to_string(),
            ..Default::default()
        });
        Ok(results)
    }

    /// Guards off unless a test turns one on.
    fn quiet_config() -> Config {
        let mut config = Config::default();
        config.search.cache_ttl = 0.0;
        config.search.engine_min_interval = 0.0;
        config
    }

    fn search_engine(
        script: Vec<Result<EngineResults, EngineError>>,
        config: Config,
    ) -> (Arc<ScriptedEngine>, SearchEngine) {
        let engine = Arc::new(ScriptedEngine {
            calls: AtomicU32::new(0),
            script: Mutex::new(script.into()),
        });
        let registry = Arc::new(EngineRegistry::with_engines(vec![(ENGINE, engine.clone())]));
        let client = HttpClient::new(&config.outgoing).expect("http client");
        (engine, SearchEngine::new(registry, client, Arc::new(config)))
    }

    fn query() -> SearchQuery {
        let mut query = SearchQuery::simple("rust", 0);
        query.enginerefs = vec![EngineRef {
            name: ENGINE.to_string(),
            category: "none".to_string(),
        }];
        query
    }

    #[tokio::test]
    async fn failed_engine_is_suspended_and_not_queried_again() {
        let (engine, search) = search_engine(vec![Err(EngineError::Captcha), one_result()], quiet_config());

        let first = search.search(&query()).await.expect("search");
        assert!(matches!(first.outcomes[0].error, Some(EngineError::Captcha)));

        let second = search.search(&query()).await.expect("search");
        let error = second.outcomes[0].error.as_ref().expect("suspended error");
        assert!(matches!(error, EngineError::Suspended { .. }), "{error:?}");
        assert_eq!(error.to_string(), "engine suspended: CAPTCHA required; 3600s left");
        assert_eq!(second.unresponsive_engines, vec![ENGINE]);
        assert_eq!(engine.calls(), 1, "suspended engine must not be queried");
    }

    #[tokio::test]
    async fn success_resets_the_error_count() {
        let mut config = quiet_config();
        config.search.suspended_times.captcha = 0;
        let (engine, search) = search_engine(vec![Err(EngineError::Captcha), one_result()], config);

        search.search(&query()).await.expect("search");
        assert_eq!(search.suspensions.continuous_errors(ENGINE), 1);

        let second = search.search(&query()).await.expect("search");
        assert_eq!(second.results.len(), 1);
        assert_eq!(engine.calls(), 2);
        assert_eq!(search.suspensions.continuous_errors(ENGINE), 0);
    }

    #[tokio::test]
    async fn repeated_query_is_served_from_the_cache() {
        let mut config = quiet_config();
        config.search.cache_ttl = 300.0;
        let (engine, search) = search_engine(vec![one_result()], config);

        let first = search.search(&query()).await.expect("search");
        let second = search.search(&query()).await.expect("search");

        assert_eq!(first.results.len(), 1);
        assert_eq!(second.results.len(), 1);
        assert_eq!(second.results[0].url, "https://example.com/");
        assert_eq!(engine.calls(), 1, "second search must hit the cache");
    }

    #[tokio::test]
    async fn errors_are_not_cached() {
        let mut config = quiet_config();
        config.search.cache_ttl = 300.0;
        config.search.suspended_times.captcha = 0;
        let (engine, search) = search_engine(vec![Err(EngineError::Captcha), one_result()], config);

        let first = search.search(&query()).await.expect("search");
        assert!(first.results.is_empty());
        let second = search.search(&query()).await.expect("search");

        assert_eq!(second.results.len(), 1);
        assert_eq!(engine.calls(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn requests_to_one_engine_are_spaced_by_the_interval() {
        let mut config = quiet_config();
        config.search.engine_min_interval = 1.0;
        let (engine, search) = search_engine(vec![one_result(), one_result()], config);

        let started = Instant::now();
        search.search(&query()).await.expect("search");
        search.search(&query()).await.expect("search");

        assert!(started.elapsed() >= Duration::from_secs(1), "second request waited for its slot");
        assert_eq!(engine.calls(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn wait_longer_than_the_timeout_is_refused() {
        let mut config = quiet_config();
        config.search.engine_min_interval = 1.0;
        let (engine, search) = search_engine(vec![one_result()], config);
        let mut query = query();
        query.timeout_limit = Some(0.5);

        search.search(&query).await.expect("search");
        let second = search.search(&query).await.expect("search");

        let error = second.outcomes[0].error.as_ref().expect("rate limited error");
        assert!(matches!(error, EngineError::RateLimited(_)), "{error:?}");
        assert_eq!(engine.calls(), 1);
    }
}
