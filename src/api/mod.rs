//! JSON API server built with axum. Endpoints:
//!
//! - `GET /healthz` — liveness probe
//! - `GET /search?q=...` — search results (SearXNG-compatible JSON shape)
//! - `GET /config` — engine configuration

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::config::Config;
use crate::engine::EngineRegistry;
use crate::search::{parse_query, SearchEngine, SearchQuery, SearchResponse};

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub registry: Arc<EngineRegistry>,
    pub search: Arc<SearchEngine>,
}

#[derive(Deserialize)]
struct SearchQueryParams {
    q: String,
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    safesearch: Option<u8>,
    #[serde(default)]
    pageno: Option<u32>,
    #[serde(default)]
    engines: Option<String>,
    #[serde(default)]
    language: Option<String>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/search", get(search))
        .route("/config", get(config))
        .with_state(state)
}

/// Build and run the API server + Streamable HTTP MCP endpoint, binding per
/// the given address/port (defaults come from config unless overridden).
pub async fn serve(
    config: Arc<Config>,
    registry: Arc<EngineRegistry>,
    search: Arc<SearchEngine>,
    bind_addr: &str,
    port: u16,
) -> anyhow::Result<()> {
    let addr = format!("{bind_addr}:{port}");
    let app = router(AppState {
        config: config.clone(),
        registry,
        search: search.clone(),
    })
    .merge(crate::mcp::mcp_router(search, &config.mcp));
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| anyhow::anyhow!("failed to bind {addr}: {e}"))?;
    tracing::info!("JSON API listening on http://{addr}");
    println!("MCP (Streamable HTTP) endpoint: http://{addr}/mcp");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn healthz() -> &'static str {
    "OK"
}

async fn search(
    State(state): State<AppState>,
    Query(params): Query<SearchQueryParams>,
) -> Response {
    let safesearch = params.safesearch.unwrap_or(state.config.search.safe_search).min(2);
    let pageno = params.pageno.unwrap_or(1);

    if let Some(format) = &params.format {
        if format != "json" {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("format '{format}' is not supported, only 'json'")})),
            )
                .into_response();
        }
    }

    let rq = parse_query(&state.registry, &params.q, safesearch, pageno);
    let mut sq = SearchQuery::simple(rq.get_query(), safesearch);
    sq.languages = rq.languages.clone();
    sq.pageno = pageno;
    sq.redirect_to_first_result = rq.redirect_to_first_result;
    sq.external_bang = rq.external_bang.clone();
    sq.timeout_limit = rq.timeout_limit;
    if let Some(filter) = params.engines {
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
    if let Some(lang) = params.language {
        sq.languages = vec![lang];
    }

    if sq.query.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "query parameter 'q' is required"})),
        )
            .into_response();
    }

    let resp = match state.search.search(&sq).await {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response();
        }
    };

    if let Some(url) = &resp.redirect_url {
        return Json(json!({"redirect": url})).into_response();
    }

    let payload = search_api_payload(&resp);
    Json(payload).into_response()
}

/// Produce a response shaped like SearXNG's `format=json` output.
fn search_api_payload(resp: &SearchResponse) -> Value {
    json!({
        "query": resp.results.first().map(|r| r.engine.as_str()).unwrap_or(""),
        "number_of_results": resp.results.len(),
        "results": resp.results,
        "suggestions": resp.suggestions,
        "corrections": resp.corrections,
        "answers": resp.answers,
        "unresponsive_engines": resp.unresponsive_engines,
    })
}

async fn config(State(state): State<AppState>) -> Json<Value> {
    let engines: Vec<Value> = state
        .registry
        .specs
        .iter()
        .map(|spec| {
            json!({
                "name": spec.name,
                "enabled": spec.enabled,
                "weight": spec.weight,
                "categories": spec.categories,
            })
        })
        .collect();

    Json(json!({
        "instance_name": state.config.general.instance_name,
        "default_lang": state.config.general.default_lang,
        "max_page": state.config.search.max_page,
        "safesearch": state.config.search.safe_search,
        "engines": engines,
    }))
}