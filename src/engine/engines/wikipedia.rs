//! Wikipedia engine. Port of `searx/engines/mediawiki.py` from SearXNG.

use async_trait::async_trait;
use serde_json::Value;

use crate::engine::engines::ScrapeEngineBase;
use crate::engine::http::HttpClient;
use crate::engine::{Engine, EngineError, EngineParams, EngineResults, SearchResult};

const BASE_URL: &str = "https://{language}.wikipedia.org/";
const PAGE_SIZE: u32 = 5;

pub struct WikipediaEngine {
    base: ScrapeEngineBase,
}

impl WikipediaEngine {
    pub fn new(spec: &crate::engine::EngineSpec) -> Self {
        Self {
            base: ScrapeEngineBase {
                name: spec.name.clone(),
                weight: spec.weight,
                timeout: spec.timeout,
            },
        }
    }

    fn language(&self, lang: Option<&str>) -> String {
        // 'all' maps to English, otherwise use the primary language subtag.
        match lang {
            Some(l) if !l.is_empty() && l != "all" => l.split('-').next().unwrap_or("en").to_string(),
            _ => "en".to_string(),
        }
    }
}

#[async_trait]
impl Engine for WikipediaEngine {
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
        let language = self.language(params.language());
        let base = BASE_URL.replace("{language}", &language);

        let offset = (params.pageno.saturating_sub(1)) * PAGE_SIZE;
        let args = [
            ("action", "query"),
            ("list", "search"),
            ("format", "json"),
            ("srsearch", &params.query),
            ("sroffset", &offset.to_string()),
            ("srlimit", &PAGE_SIZE.to_string()),
            ("srwhat", "nearmatch"),
            ("srprop", "snippet|timestamp"),
            ("srsort", "relevance"),
            ("srenablerewrites", "1"),
        ];
        let qs = args
            .iter()
            .map(|(k, v)| format!("{k}={}", urlencode(v)))
            .collect::<Vec<_>>()
            .join("&");
        let url = format!("{base}w/api.php?{qs}");

        let resp = client
            .get(&url, Some(&language))
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

        parse_results(&json, &base, &language, &self.base.name)
    }
}

fn urlencode(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

fn parse_results(
    json: &Value,
    base: &str,
    language: &str,
    engine_name: &str,
) -> Result<EngineResults, EngineError> {
    let mut out = EngineResults::default();
    let search = json
        .get("query")
        .and_then(|q| q.get("search"))
        .and_then(|s| s.as_array())
        .cloned()
        .unwrap_or_default();

    for item in search {
        let title = item.get("title").and_then(|t| t.as_str()).unwrap_or("");
        if title.trim().is_empty() {
            continue;
        }
        let snippet = item
            .get("snippet")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();
        // strip <span class="searchmatch">..</span> markup
        let content = strip_html(&snippet);
        let url = format!("{base}wiki/{}", title.replace(' ', "_"));
        let timestamp = item.get("timestamp").and_then(|t| t.as_str()).map(|s| s.to_string());

        out.add(SearchResult {
            url,
            title: title.to_string(),
            content,
            engine: engine_name.to_string(),
            published_date: timestamp,
            category: "general".to_string(),
            ..Default::default()
        });
    }
    let _ = language;
    Ok(out)
}

fn strip_html(s: &str) -> String {
    // Remove all tags, then normalize whitespace (SearXNG's html_to_text).
    // Insert a space between inline elements so the extracted text keeps
    // word separation (e.g. "<b>a</b><i>b</i>" → "a b").
    let inline_end = regex::Regex::new(r"(?s)</(span|a|abbr|b|i|em|u|s|strong|sub|sup|small|q|code)>").unwrap();
    let s = inline_end.replace_all(s, "$0 ");
    let re = regex::Regex::new(r"(?s)<[^>]*>").unwrap();
    crate::engine::normalize_text(&re.replace_all(&s, ""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_api_response() {
        let json = json!({
            "query": {"search": [
                {"title": "Rust (programming language)", "snippet": "<span>Type-safe</span> systems language", "timestamp": "2024-01-01T00:00:00Z"}
            ]}
        });
        let res = parse_results(&json, "https://en.wikipedia.org/", "en", "wikipedia").unwrap();
        assert_eq!(res.results.len(), 1);
        assert!(res.results[0].content.contains("Type-safe"));
        assert_eq!(
            res.results[0].url,
            "https://en.wikipedia.org/wiki/Rust_(programming_language)"
        );
    }

    #[test]
    fn empty_results() {
        let res = parse_results(&json!({}), "https://en.wikipedia.org/", "en", "wikipedia").unwrap();
        assert!(res.results.is_empty());
    }

    #[test]
    fn strips_markup() {
        assert_eq!(strip_html("<b>a</b><i>b</i>  c"), "a b c");
    }
}