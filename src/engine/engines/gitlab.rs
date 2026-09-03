//! GitLab project search. Port of `searx/engines/gitlab.py` from SearXNG.

use async_trait::async_trait;
use serde_json::Value;

use crate::engine::engines::ScrapeEngineBase;
use crate::engine::http::HttpClient;
use crate::engine::{Engine, EngineError, EngineParams, EngineResults, SearchResult};

const API_URL: &str = "https://gitlab.com/api/v4/projects";

pub struct GitlabEngine {
    base: ScrapeEngineBase,
}

impl GitlabEngine {
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
impl Engine for GitlabEngine {
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
            "search={}&page={}",
            urlencode(&params.query),
            params.pageno.max(1)
        );
        let url = format!("{API_URL}?{qs}");

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

        let json: Vec<Value> =
            serde_json::from_str(&body).map_err(|e| EngineError::Parse(e.to_string()))?;
        Ok(parse_results(&json, &self.base.name))
    }
}

fn urlencode(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

fn parse_results(json: &[Value], engine_name: &str) -> EngineResults {
    let mut out = EngineResults::default();
    for item in json {
        let Some(name) = item.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        let avatar_url = item
            .get("avatar_url")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let published = item
            .get("last_activity_at")
            .and_then(|v| v.as_str())
            .or_else(|| item.get("created_at").and_then(|v| v.as_str()))
            .map(|s| s.to_string());

        out.add(SearchResult {
            url: item.get("web_url").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            title: name.to_string(),
            content: item.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            engine: engine_name.to_string(),
            thumbnail: avatar_url,
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
    fn parses_projects() {
        let json = vec![
            json!({
                "name": "gitlab",
                "web_url": "https://gitlab.com/gitlab-org/gitlab",
                "description": "GitLab CE",
                "last_activity_at": "2024-01-01T00:00:00Z",
                "avatar_url": "https://gitlab.com/a.png"
            }),
        ];
        let res = parse_results(&json, "gitlab");
        assert_eq!(res.results.len(), 1);
        assert_eq!(res.results[0].title, "gitlab");
        assert_eq!(res.results[0].published_date.as_deref(), Some("2024-01-01T00:00:00Z"));
    }

    #[test]
    fn empty() {
        let res = parse_results(&[], "gitlab");
        assert!(res.results.is_empty());
    }
}
