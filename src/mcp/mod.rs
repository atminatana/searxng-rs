//! MCP (Model Context Protocol) server on top of the search engine.
//!
//! Exposes the metasearch as MCP tools so LLM clients (Claude, opencode, ...)
//! can run web searches. Transports: stdio (default) and Streamable HTTP.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::StreamableHttpServerConfig;
use rmcp::transport::stdio;
use rmcp::{schemars, tool, tool_router, ErrorData, ServiceExt};
use serde::{Deserialize, Serialize};

use crate::config::Mcp;
use crate::engine::EngineRegistry;
use crate::search::{parse_query, EngineOutcome, SearchEngine, SearchQuery};

/// Arguments of the `search` MCP tool.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema, Serialize)]
struct SearchToolArgs {
    /// The search query text.
    query: String,
    /// Comma-separated engine names to restrict to (e.g. "google,bing").
    /// Empty means use all enabled engines.
    #[serde(default)]
    engines: Option<String>,
    /// Safe search level: 0 off, 1 moderate, 2 strict.
    #[serde(default)]
    safesearch: Option<u8>,
    /// Language tag such as "en" or "de-DE".
    #[serde(default)]
    language: Option<String>,
    /// Result page number (starts at 1).
    #[serde(default)]
    pageno: Option<u32>,
}

/// Arguments of the `engine_status` MCP tool.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema, Serialize)]
struct EngineStatusArgs {
    /// If given, only show details for this engine name.
    #[serde(default)]
    engine: Option<String>,
}

#[derive(Clone)]
pub struct SearchMcpServer {
    search: Arc<SearchEngine>,
}

impl SearchMcpServer {
    pub fn new(search: Arc<SearchEngine>) -> Self {
        Self { search }
    }
}

#[tool_router(server_handler)]
impl SearchMcpServer {
    /// Metasearch across multiple search engines in parallel. Returns results
    /// with title, url and content snippet for each hit.
    #[tool(description = "Run a web meta-search across multiple engines.")]
    async fn search(
        &self,
        Parameters(args): Parameters<SearchToolArgs>,
    ) -> Result<String, ErrorData> {
        let start = Instant::now();
        tracing::info!("[CALL] mcp::search(query={:?}, engines={:?}, safesearch={:?}, language={:?}, pageno={:?})",
            args.query, args.engines, args.safesearch, args.language, args.pageno);
        let safesearch = args.safesearch.unwrap_or(0).min(2);
        let pageno = args.pageno.unwrap_or(1);

        let rq = parse_query(self.search.registry(), &args.query, safesearch, pageno);
        let mut sq = SearchQuery::simple(rq.get_query(), safesearch);
        sq.languages = rq.languages.clone();
        sq.pageno = pageno;
        sq.redirect_to_first_result = rq.redirect_to_first_result;
        sq.external_bang = rq.external_bang.clone();
        sq.timeout_limit = rq.timeout_limit;

        if let Some(filter) = args.engines {
            if !filter.trim().is_empty() {
                sq.enginerefs = filter
                    .split(',')
                    .map(|name| crate::query::EngineRef {
                        name: name.trim().to_string(),
                        category: "none".to_string(),
                    })
                    .collect();
            }
        } else if !rq.enginerefs.is_empty() {
            sq.enginerefs = rq.enginerefs;
        }
        if let Some(lang) = args.language {
            sq.languages = vec![lang];
        }
        // The search orchestrator silently skips names missing from the
        // registry; collect them here to tell the client.
        let unknown_engines = find_unknown_engines(&sq, self.search.registry());

        let resp = self
            .search
            .search(&sq)
            .await;
        let resp = match resp {
            Ok(r) => r,
            Err(e) => {
                let err = ErrorData::internal_error(e.to_string(), None);
                tracing::warn!("[RESP] mcp::search -> error={}, time={:.3}s", e, start.elapsed().as_secs_f64());
                return Err(err);
            }
        };

        if let Some(url) = resp.redirect_url {
            let r = Ok(format!("redirect: {url}"));
            tracing::info!("[RESP] mcp::search -> redirect={}, time={:.3}s", url, start.elapsed().as_secs_f64());
            return r;
        }

        if resp.results.is_empty() {
            let message = describe_empty_search(&unknown_engines, &resp.outcomes);
            tracing::info!("[RESP] mcp::search -> {}, time={:.3}s", message, start.elapsed().as_secs_f64());
            return Ok(message);
        }

        let json =
            serde_json::to_string(&resp.results).map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        tracing::info!("[RESP] mcp::search -> results={}, time={:.3}s", resp.results.len(), start.elapsed().as_secs_f64());
        Ok(json)
    }

    /// List configured search engines and whether they are enabled.
    /// Supported modules: google, bing, brave, yandex, duckduckgo, wikipedia,
    /// yahoo, baidu, naver, startpage, github, gitlab, npm, pypi, docker_hub,
    /// crates, hackernews, google_news, bing_news, google_images, bing_images,
    /// plus custom xpath/json engines.
    #[tool(description = "List all configured search engines (built-in and custom) with their status and categories. Supported modules: google, bing, brave, yandex, duckduckgo, wikipedia, yahoo, baidu, naver, startpage, github, gitlab, npm, pypi, docker_hub, crates, hackernews, google_news, bing_news, google_images, bing_images, plus custom xpath/json engines.")]
    fn engine_status(
        &self,
        Parameters(args): Parameters<EngineStatusArgs>,
    ) -> Result<String, ErrorData> {
        tracing::info!("[CALL] mcp::engine_status(engine={:?})", args.engine);
        let registry = self.search.registry();
        let mut lines = Vec::new();
        for spec in &registry.specs {
            if let Some(filter) = &args.engine {
                if &spec.name != filter {
                    continue;
                }
            }
            let status = if spec.enabled { "enabled" } else { "disabled" };
            lines.push(format!("{}: {} ({})", spec.name, status, spec.categories.join(",")));
        }
        if lines.is_empty() {
            let r = Ok("no engines".to_string());
            tracing::info!("[RESP] mcp::engine_status -> 0 engines");
            return r;
        }
        let r = Ok(lines.join("\n"));
        tracing::info!("[RESP] mcp::engine_status -> {} engines", lines.len());
        r
    }
}

/// Requested engine names that are not in the registry (not configured in
/// `[engines]` or misspelled), in request order, without duplicates.
fn find_unknown_engines(query: &SearchQuery, registry: &EngineRegistry) -> Vec<String> {
    let mut unknown: Vec<String> = Vec::new();
    for engine_ref in &query.enginerefs {
        let name = &engine_ref.name;
        let is_unknown = !name.is_empty() && !registry.is_loaded(name);
        if is_unknown && !unknown.contains(name) {
            unknown.push(name.clone());
        }
    }
    unknown
}

/// Reply for a search without results. Unknown engine names and engine
/// failures (CAPTCHA, HTTP 403, timeout, ...) are listed so the client can
/// tell "nothing found" from "the engine did not answer". Engine errors are
/// sorted by engine name: outcomes arrive in completion order, which is not
/// deterministic.
fn describe_empty_search(unknown_engines: &[String], outcomes: &[EngineOutcome]) -> String {
    let mut engine_errors: Vec<String> = outcomes
        .iter()
        .filter_map(|outcome| {
            let error = outcome.error.as_ref()?;
            Some(format!("{}: {error}", outcome.engine))
        })
        .collect();
    engine_errors.sort();

    let mut message = "no results".to_string();
    if !unknown_engines.is_empty() {
        message.push_str(&format!("; unknown engine: {}", unknown_engines.join(", ")));
    }
    if !engine_errors.is_empty() {
        message.push_str(&format!("; engine errors: {}", engine_errors.join("; ")));
    }
    message
}

/// Serve the MCP server over stdio until the client disconnects.
pub async fn serve_stdio(search: Arc<SearchEngine>) -> anyhow::Result<()> {
    let server = SearchMcpServer::new(search);
    tracing::info!("MCP server started (stdio transport)");
    let service = server
        .serve(stdio())
        .await
        .map_err(|e| anyhow::anyhow!("failed to serve MCP: {e}"))?;
    service.waiting().await?;
    Ok(())
}

/// MCP Streamable HTTP endpoint (externally hosted) with session management.
pub fn mcp_router(search: Arc<SearchEngine>, mcp: &Mcp) -> axum::Router {
    use rmcp::transport::StreamableHttpService;
    use tower::ServiceBuilder;
    use tower_http::trace::{TraceLayer, DefaultMakeSpan, DefaultOnRequest, DefaultOnResponse};
    use tracing::Level;

    let mut config = StreamableHttpServerConfig::default()
        .with_sse_keep_alive(Some(Duration::from_secs(15)))
        .with_sse_retry(Some(Duration::from_secs(3)));
    if !mcp.allowed_hosts.is_empty() {
        config = config.with_allowed_hosts(mcp.allowed_hosts.clone());
    }

    let service: StreamableHttpService<SearchMcpServer, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok(SearchMcpServer::new(search.clone())),
            Default::default(),
            config,
        );

    let trace_layer = TraceLayer::new_for_http()
        .make_span_with(DefaultMakeSpan::new().include_headers(true))
        .on_request(DefaultOnRequest::new().level(Level::INFO))
        .on_response(DefaultOnResponse::new().level(Level::INFO).latency_unit(tower_http::LatencyUnit::Micros));

    axum::Router::new()
        .nest_service("/mcp", ServiceBuilder::new().layer(trace_layer).service(service))
}

/// Serve the MCP server over Streamable HTTP on its own port.
pub async fn serve_http(search: Arc<SearchEngine>, bind_addr: &str, port: u16) -> anyhow::Result<()> {
    let addr: SocketAddr = format!("{bind_addr}:{port}")
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid bind address {bind_addr}:{port}: {e}"))?;
    let mcp = Mcp::default();
    let app = mcp_router(search, &mcp);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| anyhow::anyhow!("failed to bind {addr}: {e}"))?;
    tracing::info!("MCP HTTP server listening on http://{addr}/mcp");
    tracing::info!("MCP (Streamable HTTP) endpoint: http://{addr}/mcp");
    axum::serve(listener, app).await?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::engine::EngineError;

    fn outcome(engine: &str, error: Option<EngineError>) -> EngineOutcome {
        EngineOutcome {
            engine: engine.to_string(),
            category: "general".to_string(),
            results: Vec::new(),
            suggestions: Vec::new(),
            corrections: Vec::new(),
            answers: Vec::new(),
            error,
            elapsed: Duration::ZERO,
        }
    }

    #[test]
    fn empty_search_without_engine_errors_says_no_results() {
        assert_eq!(describe_empty_search(&[], &[]), "no results");
        assert_eq!(describe_empty_search(&[], &[outcome("bing", None)]), "no results");
    }

    #[test]
    fn empty_search_lists_engine_errors_sorted_by_engine() {
        let outcomes = [
            outcome("google", Some(EngineError::AccessDenied)),
            outcome("bing", None),
            outcome("brave", Some(EngineError::Timeout)),
        ];
        assert_eq!(
            describe_empty_search(&[], &outcomes),
            "no results; engine errors: brave: request timeout; google: Access denied"
        );
    }

    #[test]
    fn empty_search_lists_unknown_engines_before_engine_errors() {
        let unknown = vec!["yahoo".to_string(), "no_such_engine".to_string()];
        assert_eq!(
            describe_empty_search(&unknown, &[]),
            "no results; unknown engine: yahoo, no_such_engine"
        );
        assert_eq!(
            describe_empty_search(&unknown[..1], &[outcome("google", Some(EngineError::AccessDenied))]),
            "no results; unknown engine: yahoo; engine errors: google: Access denied"
        );
    }

    #[test]
    fn finds_only_unregistered_engine_names_once() {
        let config = crate::config::Config::default();
        let client = crate::engine::http::HttpClient::from_config(&config).expect("http client");
        let registry = EngineRegistry::from_config(&config, &client);
        assert!(registry.is_loaded("bing"), "default config must register bing");

        let mut query = SearchQuery::simple("rust".to_string(), 0);
        query.enginerefs = ["bing", "bign", "", "bign", "no_such_engine"]
            .iter()
            .map(|name| crate::query::EngineRef {
                name: name.to_string(),
                category: "none".to_string(),
            })
            .collect();

        assert_eq!(find_unknown_engines(&query, &registry), vec!["bign", "no_such_engine"]);
    }
}
