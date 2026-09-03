//! Naver general web search. Port of `searx/engines/naver.py` from SearXNG
//! (the `general` category only — `where=web`).

use async_trait::async_trait;
use scraper::{Html, Selector};

use crate::engine::engines::{attr, text_of, ScrapeEngineBase};
use crate::engine::http::HttpClient;
use crate::engine::{Engine, EngineError, EngineParams, EngineResults, SearchResult};

const BASE_URL: &str = "https://search.naver.com";
const START: u32 = 15;

pub struct NaverEngine {
    base: ScrapeEngineBase,
}

impl NaverEngine {
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
impl Engine for NaverEngine {
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
        let start = (params.pageno.max(1).saturating_sub(1)) * START + 1;
        let qs = format!("query={}&start={start}&where=web", urlencode(&params.query));
        let url = format!("{BASE_URL}/search.naver?{qs}");

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

    let item_sel = Selector::parse(r#"div[class*="fds-web-normal-doc-root"]"#).unwrap();
    let title_sel =
        Selector::parse(r#"span[class*="sds-comps-text-type-headline1"]"#).unwrap();
    let content_sel = Selector::parse(r#"span[class*="sds-comps-text-type-body1"]"#).unwrap();
    let thumb_sel = Selector::parse(
        r#"div[class*="sds-comps-image"]:not([class*="sds-comps-image-circle"]) img[src]"#,
    )
    .unwrap();

    for item in doc.select(&item_sel) {
        let title = item
            .select(&title_sel)
            .next()
            .map(|e| text_of(&e))
            .unwrap_or_default();
        let Some(link) = item
            .select(&Selector::parse(r#"a[href^="http"]:not([href*="keep.naver.com"])"#).unwrap())
            .next()
        else {
            continue;
        };
        let Some(url) = attr(&link, "href") else {
            continue;
        };
        if title.trim().is_empty() {
            continue;
        }
        let content = item
            .select(&content_sel)
            .next()
            .map(|e| text_of(&e))
            .unwrap_or_default();
        let thumbnail = item.select(&thumb_sel).next().and_then(|e| attr(&e, "src"));

        let mut res = SearchResult {
            url,
            title,
            content,
            engine: engine_name.to_string(),
            ..Default::default()
        };
        res.thumbnail = thumbnail;
        out.add(res);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fixture() {
        let html = r#"
<html><body>
<div class="fds-web-normal-doc-root">
  <div class="sds-comps-image"><img src="https://thumb.com/1.png"></div>
  <span class="sds-comps-text-type-headline1">Rust Lang</span>
  <a href="https://rust-lang.org">rust-lang.org</a>
  <span class="sds-comps-text-type-body1">A systems language</span>
</div>
<div class="fds-web-normal-doc-root">
  <span class="sds-comps-text-type-headline1">Skip me</span>
  <a href="https://keep.naver.com/x">keep</a>
</div>
</body></html>"#;
        let res = parse_results(html, "naver");
        assert_eq!(res.results.len(), 1);
        assert_eq!(res.results[0].url, "https://rust-lang.org");
        assert_eq!(res.results[0].title, "Rust Lang");
        assert_eq!(res.results[0].thumbnail.as_deref(), Some("https://thumb.com/1.png"));
    }

    #[test]
    fn empty() {
        let res = parse_results("<html></html>", "naver");
        assert!(res.results.is_empty());
    }
}
