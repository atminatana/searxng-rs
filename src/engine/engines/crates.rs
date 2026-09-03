//! crates.io package search. Port of `searx/engines/crates.py` from SearXNG.

use async_trait::async_trait;
use serde_json::Value;

use crate::engine::engines::ScrapeEngineBase;
use crate::engine::http::HttpClient;
use crate::engine::{Engine, EngineError, EngineParams, EngineResults, SearchResult};

const SEARCH_URL: &str = "https://crates.io/api/v1/crates";
const PAGE_SIZE: u32 = 10;

pub struct CratesEngine {
    base: ScrapeEngineBase,
}

impl CratesEngine {
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
impl Engine for CratesEngine {
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
        let qs = format!(
            "page={}&q={}&per_page={PAGE_SIZE}",
            params.pageno.max(1),
            urlencode(&params.query)
        );
        let url = format!("{SEARCH_URL}?{qs}");

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
    let crates = json.get("crates").and_then(|c| c.as_array()).cloned().unwrap_or_default();

    for crate_ in crates {
        let Some(name) = crate_.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        let version = crate_
            .get("newest_version")
            .and_then(|v| v.as_str())
            .or_else(|| crate_.get("max_version").and_then(|v| v.as_str()))
            .or_else(|| crate_.get("max_stable_version").and_then(|v| v.as_str()))
            .unwrap_or("")
            .to_string();

        out.add(SearchResult {
            url: format!("https://crates.io/crates/{name}"),
            title: name.to_string(),
            content: crate_.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            engine: engine_name.to_string(),
            published_date: crate_.get("updated_at").and_then(|v| v.as_str()).map(|s| s.to_string()),
            ..Default::default()
        });
        let _ = version;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_crates() {
        let json = json!({
            "crates": [
                {
                    "name": "serde",
                    "newest_version": "1.0.0",
                    "description": "Serialization framework",
                    "updated_at": "2024-01-01T00:00:00Z"
                }
            ]
        });
        let res = parse_results(&json, "crates");
        assert_eq!(res.results.len(), 1);
        assert_eq!(res.results[0].title, "serde");
        assert_eq!(res.results[0].url, "https://crates.io/crates/serde");
    }

    #[test]
    fn empty() {
        let res = parse_results(&json!({}), "crates");
        assert!(res.results.is_empty());
    }
}
