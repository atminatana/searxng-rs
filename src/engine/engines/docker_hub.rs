//! Docker Hub image search. Port of `searx/engines/docker_hub.py` from SearXNG.

use async_trait::async_trait;
use serde_json::Value;

use crate::engine::engines::ScrapeEngineBase;
use crate::engine::http::HttpClient;
use crate::engine::{Engine, EngineError, EngineParams, EngineResults, SearchResult};

const BASE_URL: &str = "https://hub.docker.com";
const PAGE_SIZE: u32 = 10;

pub struct DockerHubEngine {
    base: ScrapeEngineBase,
}

impl DockerHubEngine {
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
impl Engine for DockerHubEngine {
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
        let from = PAGE_SIZE * params.pageno.saturating_sub(1);
        let qs = format!("query={}&from={from}&size={PAGE_SIZE}", urlencode(&params.query));
        let url = format!("{BASE_URL}/api/search/v3/catalog/search?{qs}");

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
    let items = json.get("results").and_then(|r| r.as_array()).cloned().unwrap_or_default();

    for item in items {
        let Some(name) = item.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        let slug = item.get("slug").and_then(|v| v.as_str()).unwrap_or("");
        let source = item.get("source").and_then(|v| v.as_str()).unwrap_or("");
        let is_official = source == "store" || source == "official";
        let path_prefix = if is_official { "/_/" } else { "/r/" };
        let thumbnail = item
            .get("logo_url")
            .and_then(|l| l.get("large"))
            .and_then(|v| v.as_str())
            .or_else(|| {
                item.get("logo_url")
                    .and_then(|l| l.get("small"))
                    .and_then(|v| v.as_str())
            });

        let published = item
            .get("updated_at")
            .and_then(|v| v.as_str())
            .or_else(|| item.get("created_at").and_then(|v| v.as_str()))
            .map(|s| s.to_string());

        out.add(SearchResult {
            url: format!("{BASE_URL}{path_prefix}{slug}"),
            title: name.to_string(),
            content: item
                .get("short_description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            engine: engine_name.to_string(),
            thumbnail: thumbnail.map(|s| s.to_string()),
            published_date: published,
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
    fn parses_images() {
        let json = json!({
            "results": [
                {
                    "name": "postgres",
                    "slug": "library/postgres",
                    "source": "official",
                    "short_description": "PostgreSQL",
                    "updated_at": "2024-01-01T00:00:00Z",
                    "logo_url": {"large": "https://hub.docker.com/l.png"}
                }
            ]
        });
        let res = parse_results(&json, "docker_hub");
        assert_eq!(res.results.len(), 1);
        assert_eq!(res.results[0].title, "postgres");
        assert_eq!(res.results[0].url, "https://hub.docker.com/_/library/postgres");
        assert_eq!(res.results[0].thumbnail.as_deref(), Some("https://hub.docker.com/l.png"));
    }

    #[test]
    fn empty() {
        let res = parse_results(&json!({}), "docker_hub");
        assert!(res.results.is_empty());
    }
}
