//! Hacker News search via the Algolia API. Port of `searx/engines/hackernews.py`
//! from SearXNG.

use async_trait::async_trait;
use serde_json::Value;

use crate::engine::engines::ScrapeEngineBase;
use crate::engine::http::HttpClient;
use crate::engine::{Engine, EngineError, EngineParams, EngineResults, SearchResult};

const BASE_URL: &str = "https://hn.algolia.com/api/v1";
const RESULTS_PER_PAGE: u32 = 30;

pub struct HackernewsEngine {
    base: ScrapeEngineBase,
}

impl HackernewsEngine {
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
impl Engine for HackernewsEngine {
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
        let page = params.pageno.saturating_sub(1);
        if params.query.trim().is_empty() {
            // show results from HN front page when no query
            let qs = format!("tags=front_page&page={page}");
            let url = format!("{BASE_URL}/search_by_date?{qs}");
            let resp = client
                .get(&url, params.language())
                .await
                .map_err(|e| EngineError::Request(e.to_string()))?;
            return handle_resp(resp, &self.base.name).await;
        }

        let qs = format!(
            "query={}&page={page}&hitsPerPage={RESULTS_PER_PAGE}&minWordSizefor1Typo=4\
             &minWordSizefor2Typos=8&advancedSyntax=true&ignorePlurals=false&minProximity=7\
             &numericFilters=[]&tagFilters=[\"story\",[]]&typoTolerance=true&queryType=prefixLast\
             &restrictSearchableAttributes=[\"title\",\"comment_text\",\"url\",\"story_text\",\"author\"]\
             &getRankingInfo=true",
            urlencode(&params.query)
        );
        let url = format!("{BASE_URL}/search?{qs}");
        let resp = client
            .get(&url, params.language())
            .await
            .map_err(|e| EngineError::Request(e.to_string()))?;
        handle_resp(resp, &self.base.name).await
    }
}

async fn handle_resp(
    resp: reqwest::Response,
    engine_name: &str,
) -> Result<EngineResults, EngineError> {
    let status = resp.status().as_u16();
    if let Some(err) = EngineError::from_status(status) {
        return Err(err);
    }
    let body = resp
        .text()
        .await
        .map_err(|e| EngineError::Request(e.to_string()))?;
    let json: Value = serde_json::from_str(&body).map_err(|e| EngineError::Parse(e.to_string()))?;
    Ok(parse_results(&json, engine_name))
}

fn urlencode(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

fn parse_results(json: &Value, engine_name: &str) -> EngineResults {
    let mut out = EngineResults::default();
    let hits = json.get("hits").and_then(|h| h.as_array()).cloned().unwrap_or_default();

    for hit in hits {
        let Some(object_id) = hit.get("objectID").and_then(|v| v.as_str()) else {
            continue;
        };
        let points = hit.get("points").and_then(|v| v.as_i64()).unwrap_or(0);
        let num_comments = hit.get("num_comments").and_then(|v| v.as_i64()).unwrap_or(0);

        let content = hit
            .get("url")
            .and_then(|v| v.as_str())
            .or_else(|| hit.get("comment_text").and_then(|v| v.as_str()))
            .or_else(|| hit.get("story_text").and_then(|v| v.as_str()))
            .map(strip_html)
            .unwrap_or_default();

        let title = hit
            .get("title")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                hit.get("author")
                    .and_then(|v| v.as_str())
                    .map(|a| format!("author: {a}"))
                    .unwrap_or_default()
            });

        let published = hit
            .get("created_at_i")
            .and_then(|v| v.as_i64())
            .map(|ts| {
                let secs = ts as u64;
                let _ = secs;
                ts.to_string()
            });

        out.add(SearchResult {
            url: format!("https://news.ycombinator.com/item?id={object_id}"),
            title,
            content,
            engine: engine_name.to_string(),
            published_date: published,
            ..Default::default()
        });
        let _ = points;
        let _ = num_comments;
    }
    out
}

fn strip_html(s: &str) -> String {
    let re = regex::Regex::new(r"(?s)<[^>]*>").unwrap();
    crate::engine::normalize_text(&re.replace_all(s, " "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_hits() {
        let json = json!({
            "hits": [
                {
                    "objectID": "1",
                    "title": "Rust is awesome",
                    "url": "https://example.com/rust",
                    "author": "alice",
                    "points": 10,
                    "num_comments": 3,
                    "created_at_i": 1700000000
                }
            ]
        });
        let res = parse_results(&json, "hackernews");
        assert_eq!(res.results.len(), 1);
        assert_eq!(res.results[0].title, "Rust is awesome");
        assert_eq!(res.results[0].url, "https://news.ycombinator.com/item?id=1");
        assert_eq!(res.results[0].published_date.as_deref(), Some("1700000000"));
    }

    #[test]
    fn empty() {
        let res = parse_results(&json!({}), "hackernews");
        assert!(res.results.is_empty());
    }
}
