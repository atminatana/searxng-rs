//! GitHub repository search. Port of `searx/engines/github.py` from SearXNG.

use async_trait::async_trait;
use serde_json::Value;

use crate::engine::engines::ScrapeEngineBase;
use crate::engine::http::HttpClient;
use crate::engine::{Engine, EngineError, EngineParams, EngineResults, SearchResult};

const SEARCH_URL: &str = "https://api.github.com/search/repositories?sort=stars&order=desc&";

pub struct GithubEngine {
    base: ScrapeEngineBase,
}

impl GithubEngine {
    pub fn new(spec: &crate::engine::EngineSpec) -> Self {
        Self {
            base: ScrapeEngineBase {
                name: spec.name.clone(),
                weight: spec.weight,
                timeout: spec.timeout,
            },
        }
    }
}

#[async_trait]
impl Engine for GithubEngine {
    fn name(&self) -> &str {
        &self.base.name
    }

    fn weight(&self) -> f32 {
        self.base.weight
    }

    fn timeout(&self) -> Option<f32> {
        self.base.timeout
    }

    async fn search(
        &self,
        params: &EngineParams,
        client: &HttpClient,
    ) -> Result<EngineResults, EngineError> {
        let qs = format!("q={}", urlencode(&params.query));
        let url = format!("{SEARCH_URL}{qs}");

        let resp = client
            .get(&url, params.language())
            .await
            .map_err(|e| EngineError::Request(e.to_string()))?;

        let status = resp.status().as_u16();
        if let Some(err) = EngineError::from_status(status) {
            return Err(err);
        }

        let body = resp
            .text()
            .await
            .map_err(|e| EngineError::Request(e.to_string()))?;

        let json: Value =
            serde_json::from_str(&body).map_err(|e| EngineError::Parse(e.to_string()))?;
        Ok(parse_results(&json, &self.base.name))
    }
}

fn urlencode(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

fn parse_results(json: &Value, engine_name: &str) -> EngineResults {
    let mut out = EngineResults::default();
    let items = json.get("items").and_then(|i| i.as_array()).cloned().unwrap_or_default();

    for item in items {
        let Some(full_name) = item.get("full_name").and_then(|v| v.as_str()) else {
            continue;
        };
        let mut content_parts = Vec::new();
        if let Some(l) = item.get("language").and_then(|v| v.as_str()) {
            content_parts.push(l.to_string());
        }
        if let Some(d) = item.get("description").and_then(|v| v.as_str()) {
            content_parts.push(d.to_string());
        }

        let updated = item
            .get("updated_at")
            .and_then(|v| v.as_str())
            .or_else(|| item.get("created_at").and_then(|v| v.as_str()))
            .map(|s| s.to_string());

        let thumbnail = item
            .get("owner")
            .and_then(|o| o.get("avatar_url"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        out.add(SearchResult {
            url: item.get("html_url").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            title: full_name.to_string(),
            content: content_parts.join(" / "),
            engine: engine_name.to_string(),
            thumbnail,
            published_date: updated,
            ..Default::default()
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_repositories() {
        let json = json!({
            "items": [
                {
                    "full_name": "rust-lang/rust",
                    "html_url": "https://github.com/rust-lang/rust",
                    "language": "Rust",
                    "description": "Empowering everyone",
                    "updated_at": "2024-01-01T00:00:00Z",
                    "owner": {"avatar_url": "https://avatars.com/rust"}
                }
            ]
        });
        let res = parse_results(&json, "github");
        assert_eq!(res.results.len(), 1);
        assert_eq!(res.results[0].title, "rust-lang/rust");
        assert!(res.results[0].content.contains("Rust"));
        assert_eq!(res.results[0].published_date.as_deref(), Some("2024-01-01T00:00:00Z"));
    }

    #[test]
    fn empty_items() {
        let res = parse_results(&json!({}), "github");
        assert!(res.results.is_empty());
    }
}
