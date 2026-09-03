//! PyPI package search. Port of `searx/engines/pypi.py` from SearXNG.

use async_trait::async_trait;
use scraper::{Html, Selector};

use crate::engine::engines::{text_of, ScrapeEngineBase};
use crate::engine::http::HttpClient;
use crate::engine::{Engine, EngineError, EngineParams, EngineResults, SearchResult};

const BASE_URL: &str = "https://pypi.org";

pub struct PypiEngine {
    base: ScrapeEngineBase,
}

impl PypiEngine {
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
impl Engine for PypiEngine {
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
        let qs = format!("q={}&page={}", urlencode(&params.query), params.pageno.max(1));
        let url = format!("{BASE_URL}/search/?{qs}");

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

        Ok(parse_results(&body, &self.base.name))
    }
}

fn urlencode(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

fn parse_results(body: &str, engine_name: &str) -> EngineResults {
    let mut out = EngineResults::default();
    let doc = Html::parse_document(body);

    let package_sel = Selector::parse("a.package-snippet").unwrap();
    let name_sel = Selector::parse("span.package-snippet__name").unwrap();
    let version_sel = Selector::parse("span.package-snippet__version").unwrap();
    let desc_sel = Selector::parse("p").unwrap();

    for link in doc.select(&package_sel) {
        let title = link
            .select(&name_sel)
            .next()
            .map(|e| text_of(&e))
            .unwrap_or_default();
        if title.trim().is_empty() {
            continue;
        }
        let Some(href) = link.value().attr("href") else {
            continue;
        };
        let url = if href.starts_with("http") {
            href.to_string()
        } else {
            format!("{BASE_URL}{href}")
        };
        let content = link
            .select(&desc_sel)
            .next()
            .map(|e| text_of(&e))
            .unwrap_or_default();
        let version = link
            .select(&version_sel)
            .next()
            .map(|e| text_of(&e))
            .unwrap_or_default();

        out.add(SearchResult {
            url,
            title,
            content,
            engine: engine_name.to_string(),
            ..Default::default()
        });
        let _ = version;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fixture() {
        let html = r#"
<html><body><main><div><div><div><form><div><ul><li>
<a class="package-snippet" href="/project/requests/">
  <h3><span class="package-snippet__name">requests</span></h3>
  <p>HTTP for humans</p>
</a>
</li></ul></div></form></div></div></div></main></body></html>"#;
        let res = parse_results(html, "pypi");
        assert_eq!(res.results.len(), 1);
        assert_eq!(res.results[0].title, "requests");
        assert_eq!(res.results[0].url, "https://pypi.org/project/requests/");
        assert!(res.results[0].content.contains("HTTP"));
    }

    #[test]
    fn empty_body() {
        let res = parse_results("<html></html>", "pypi");
        assert!(res.results.is_empty());
    }
}
