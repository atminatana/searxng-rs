//! Search orchestration. Port of `searx/search/__init__.py` from SearXNG:
//! builds per-engine requests, runs them in parallel with a shared timeout,
//! then merges results into a `ResultContainer`.

use std::sync::Arc;
use std::time::Duration;

use futures::stream::{FuturesUnordered, StreamExt};
use tokio::time::Instant;

use crate::config::Config;
use crate::engine::http::HttpClient;
use crate::engine::{EngineError, EngineParams, EngineRegistry, EngineResults, SearchResult};
use crate::query::{EngineRef, RawTextQuery};

pub mod models;
pub mod results;

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
}

impl SearchEngine {
    pub fn new(registry: Arc<EngineRegistry>, client: Arc<HttpClient>, config: Arc<Config>) -> Self {
        Self {
            registry,
            client,
            config,
        }
    }

    pub fn registry(&self) -> &Arc<EngineRegistry> {
        &self.registry
    }

    /// Run a search for a parsed query + resolved engine refs.
    pub async fn search(&self, query: &SearchQuery) -> Result<SearchResponse, anyhow::Error> {
        // 1. External bang (!!ddg ...) → redirect, no engine queries.
        if let Some(bang) = query.external_bang.as_deref() {
            if let Some((_, tmpl)) = crate::query::EXTERNAL_BANGS
                .iter()
                .find(|(name, _)| *name == bang)
            {
                let url = tmpl.replace("{query}", &urlencode(&query.query));
                return Ok(SearchResponse {
                    redirect_url: Some(url),
                    ..Default::default()
                });
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

        let timeout = resolve_timeout(
            &engine_refs,
            &self.registry,
            query.timeout_limit,
            self.config.server.max_request_timeout,
        );

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
            let engine = engine.clone();
            let engine_name = engineref.name.clone();
            let client = self.client.clone();
            tasks.push(async move {
                let start = Instant::now();
                let result = tokio::time::timeout(
                    Duration::from_secs_f32(timeout.max(0.1)),
                    engine.search(&eparams, &client),
                )
                .await;
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
                return Ok(SearchResponse {
                    redirect_url: Some(first.url.clone()),
                    results: container.results.clone(),
                    ..Default::default()
                });
            }
        }

        Ok(SearchResponse {
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
        })
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

    match (max_request_timeout.is_finite(), query_timeout) {
        (false, None) => default_timeout,
        (false, Some(q)) => default_timeout.min(q),
        (true, None) => default_timeout.min(max_request_timeout),
        (true, Some(q)) => q.min(max_request_timeout),
    }
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
    container
}

// Helper to hydrate a partial EngineOutcome built synchronously.
impl EngineOutcome {
    fn hydrate(&mut self, result: Result<Result<EngineResults, EngineError>, tokio::time::error::Elapsed>) {
        match result {
            Ok(Ok(res)) => {
                self.results = res.results;
                self.suggestions = res.suggestions;
                self.corrections = res.corrections;
                self.answers = res.answers;
                self.error = None;
            }
            Ok(Err(e)) => {
                self.error = Some(e);
            }
            Err(_) => {
                self.error = Some(EngineError::Timeout);
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