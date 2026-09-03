//! npm (npms.io) package search. Port of `searx/engines/npm.py` from SearXNG.

use async_trait::async_trait;
use serde_json::Value;

use crate::engine::engines::ScrapeEngineBase;
use crate::engine::http::HttpClient;
use crate::engine::{Engine, EngineError, EngineParams, EngineResults, SearchResult};

const SEARCH_API: &str = "https://api.npms.io/v2/search?";
const PAGE_SIZE: u32 = 25;

pub struct NpmEngine {
    base: ScrapeEngineBase,
}

impl NpmEngine {
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
impl Engine for NpmEngine {
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
        let from = (params.pageno.saturating_sub(1)) * PAGE_SIZE;
        let qs = format!("from={from}&q={}&size={PAGE_SIZE}", urlencode(&params.query));
        let url = format!("{SEARCH_API}{qs}");

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
    let entries = json.get("results").and_then(|r| r.as_array()).cloned().unwrap_or_default();

    for entry in entries {
        let Some(package) = entry.get("package") else {
            continue;
        };
        let Some(name) = package.get("name").and_then(|v| v.as_str()) else {
            continue;
        };

        out.add(SearchResult {
            url: package
                .get("links")
                .and_then(|l| l.get("npm"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            title: name.to_string(),
            content: package.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            engine: engine_name.to_string(),
            published_date: package.get("date").and_then(|v| v.as_str()).map(|s| s.to_string()),
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
    fn parses_packages() {
        let json = json!({
            "results": [
                {
                    "package": {
                        "name": "lodash",
                        "description": "A modern library",
                        "date": "2023-05-02T16:44:38.770Z",
                        "links": {"npm": "https://www.npmjs.com/package/lodash"}
                    }
                }
            ]
        });
        let res = parse_results(&json, "npm");
        assert_eq!(res.results.len(), 1);
        assert_eq!(res.results[0].title, "lodash");
        assert_eq!(res.results[0].url, "https://www.npmjs.com/package/lodash");
    }

    #[test]
    fn empty() {
        let res = parse_results(&json!({}), "npm");
        assert!(res.results.is_empty());
    }
}
