//! Baidu general web search. Port of `searx/engines/baidu.py` from SearXNG
//! (the `general` category only — it uses Baidu's JSON `tn=json` endpoint).

use async_trait::async_trait;
use serde_json::Value;

use crate::engine::engines::ScrapeEngineBase;
use crate::engine::http::HttpClient;
use crate::engine::{Engine, EngineError, EngineParams, EngineResults, SearchResult};

const ENDPOINT: &str = "https://www.baidu.com/s";
const RESULTS_PER_PAGE: u32 = 10;

pub struct BaiduEngine {
    base: ScrapeEngineBase,
}

impl BaiduEngine {
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
impl Engine for BaiduEngine {
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
        let page_num = params.pageno.max(1);
        let pn = (page_num - 1) * RESULTS_PER_PAGE;
        let qs = format!(
            "wd={}&rn={RESULTS_PER_PAGE}&pn={pn}&tn=json",
            urlencode(&params.query)
        );
        let url = format!("{ENDPOINT}?{qs}");

        let resp = client
            .get(&url, params.language())
            .await
            .map_err(|e| EngineError::Request(e.to_string()))?;

        // Baidu CAPTCHA redirects to wappass.baidu.com
        if let Some(location) = resp.headers().get("location").and_then(|v| v.to_str().ok()) {
            if location.contains("wappass.baidu.com/static/captcha") {
                return Err(EngineError::Captcha);
            }
        }

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

        if json.get("antiFlag").and_then(|v| v.as_i64()) == Some(1) {
            return Err(EngineError::AccessDenied);
        }

        Ok(parse_results(&json, &self.base.name))
    }
}

fn urlencode(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

fn parse_results(json: &Value, engine_name: &str) -> EngineResults {
    let mut out = EngineResults::default();
    let entries = json
        .get("feed")
        .and_then(|f| f.get("entry"))
        .and_then(|e| e.as_array())
        .cloned()
        .unwrap_or_default();

    for entry in entries {
        let Some(title) = entry.get("title").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(url) = entry.get("url").and_then(|v| v.as_str()) else {
            continue;
        };
        let content = entry
            .get("abs")
            .and_then(|v| v.as_str())
            .map(html_unescape)
            .unwrap_or_default();
        let published = entry
            .get("time")
            .and_then(|v| v.as_i64())
            .map(|ts| ts.to_string());

        out.add(SearchResult {
            url: url.to_string(),
            title: html_unescape(title),
            content,
            engine: engine_name.to_string(),
            published_date: published,
            ..Default::default()
        });
    }
    out
}

/// Decode HTML entities like `&amp;`, `&#39;`, `&quot;`.
fn html_unescape(s: &str) -> String {
    let re = regex::Regex::new(r"&#(\d+);|&(amp|lt|gt|quot|apos);").unwrap();
    re.replace_all(s, |caps: &regex::Captures| {
        if let Some(num) = caps.get(1) {
            if let Ok(cp) = num.as_str().parse::<u32>() {
                if let Some(c) = char::from_u32(cp) {
                    return c.to_string();
                }
            }
            String::new()
        } else {
            let named = caps.get(2).unwrap().as_str();
            match named {
                "amp" => "&".to_string(),
                "lt" => "<".to_string(),
                "gt" => ">".to_string(),
                "quot" => "\"".to_string(),
                "apos" => "'".to_string(),
                _ => String::new(),
            }
        }
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_general() {
        let json = json!({
            "feed": {"entry": [
                {"title": "Rust &amp; C", "url": "https://www.example.com", "abs": "A language", "time": 1700000000}
            ]}
        });
        let res = parse_results(&json, "baidu");
        assert_eq!(res.results.len(), 1);
        assert_eq!(res.results[0].title, "Rust & C");
        assert_eq!(res.results[0].content, "A language");
        assert_eq!(res.results[0].published_date.as_deref(), Some("1700000000"));
    }

    #[test]
    fn unescapes_entities() {
        assert_eq!(html_unescape("a&amp;b &#39;q&#39; &quot;x&quot;"), "a&b 'q' \"x\"");
    }

    #[test]
    fn empty() {
        let res = parse_results(&json!({}), "baidu");
        assert!(res.results.is_empty());
    }
}
