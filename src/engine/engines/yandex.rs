//! Yandex WEB engine. Port of `searx/engines/yandex.py` from SearXNG.
//!
//! Yandex serves a plain HTML result page to the `/search/site/` endpoint as
//! long as the `yp` cookie is present. A CAPTCHA is signalled via the
//! `x-yandex-captcha` response header.

use async_trait::async_trait;
use scraper::{Html, Selector};

use crate::engine::engines::{attr, text_of, ScrapeEngineBase};
use crate::engine::http::HttpClient;
use crate::engine::{Engine, EngineError, EngineParams, EngineResults, SearchResult};

const SEARCH_URL: &str = "https://yandex.com/search/site/";
const BASE_HOST: &str = "https://yandex.com";
const SUPPORTED_LANGS: &[&str] = &["ru", "en", "be", "fr", "de", "id", "kk", "tt", "tr", "uk"];
const YP_VALUE: &str = "1716337604.sp.family%3A0%231686405411.szm.1%3A1920x1080%3A1920x999";

pub struct YandexEngine {
    base: ScrapeEngineBase,
}

impl YandexEngine {
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
impl Engine for YandexEngine {
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
        let url = build_url(&params.query, params.language(), params.pageno);

        let resp = client
            .get_with(&url, params.language(), &[("yp".to_string(), YP_VALUE.to_string())])
            .await
            .map_err(|e| EngineError::Request(e.to_string()))?;

        let status = resp.status().as_u16();
        if let Some(err) = EngineError::from_status(status) {
            return Err(err);
        }
        if is_captcha(&resp) {
            return Err(EngineError::Captcha);
        }

        let body = resp
            .text()
            .await
            .map_err(|e| EngineError::Request(e.to_string()))?;

        Ok(parse_results(&body, &self.base.name))
    }
}

fn build_url(query: &str, lang: Option<&str>, pageno: u32) -> String {
    let mut args: Vec<(String, String)> = vec![
        ("tmpl_version".into(), "releases".into()),
        ("text".into(), query.to_string()),
        ("web".into(), "1".into()),
        ("frame".into(), "1".into()),
        ("searchid".into(), "3131712".into()),
    ];
    if let Some(primary) = lang.and_then(|l| l.split('-').next()) {
        if SUPPORTED_LANGS.contains(&primary) {
            args.push(("lang".into(), primary.to_string()));
        }
    }
    if pageno > 1 {
        args.push(("p".into(), (pageno - 1).to_string()));
    }
    format!("{SEARCH_URL}?{}", urlencode_pairs(&args))
}

/// Yandex replies to bots with a CAPTCHA page flagged by this header.
fn is_captcha(resp: &reqwest::Response) -> bool {
    resp.headers()
        .get("x-yandex-captcha")
        .and_then(|v| v.to_str().ok()) == Some("captcha")
}

fn urlencode_pairs(pairs: &[(String, String)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", urlencode(k), urlencode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

fn urlencode(s: &str) -> String {
    const SET: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'_')
        .remove(b'.')
        .remove(b'~');
    percent_encoding::utf8_percent_encode(s, SET).to_string()
}

fn parse_results(body: &str, engine_name: &str) -> EngineResults {
    let mut out = EngineResults::default();
    let doc = Html::parse_document(body);

    let item_sel = Selector::parse("li[class*='serp-item']").unwrap();
    let link_sel = Selector::parse("a.b-serp-item__title-link").unwrap();
    let content_sel = Selector::parse("div.b-serp-item__content div.b-serp-item__text").unwrap();

    for item in doc.select(&item_sel) {
        let Some(link) = item.select(&link_sel).next() else {
            continue;
        };
        let Some(raw_url) = attr(&link, "href") else {
            continue;
        };
        if raw_url.is_empty() {
            continue;
        }
        let url = resolve_url(&raw_url);
        let title = text_of(&link).trim().to_string();
        if title.is_empty() {
            continue;
        }
        let content = item
            .select(&content_sel)
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
    }
    out
}

fn resolve_url(raw: &str) -> String {
    if raw.starts_with("http://") || raw.starts_with("https://") {
        raw.to_string()
    } else if let Ok(abs) = url::Url::parse(BASE_HOST).and_then(|b| b.join(raw)) {
        abs.to_string()
    } else {
        raw.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_search_url() {
        let url = build_url("rust lang", Some("en-US"), 1);
        assert!(url.starts_with("https://yandex.com/search/site/?"));
        assert!(url.contains("text=rust%20lang"));
        assert!(url.contains("lang=en"));
        assert!(url.contains("searchid=3131712"));
        assert!(!url.contains("&p="));
    }

    #[test]
    fn adds_pageno_offset() {
        let url = build_url("rust", None, 2);
        assert!(url.contains("&p=1"));
    }

    #[test]
    fn only_supports_known_langs() {
        assert!(build_url("q", Some("ru"), 1).contains("lang=ru"));
        assert!(!build_url("q", Some("es"), 1).contains("lang="));
    }

    #[test]
    fn parses_fixture() {
        let html = r#"
<ol>
<li class="serp-item">
  <h3 class="b-serp-item__title"><a class="b-serp-item__title-link" href="https://rust-lang.org"><span>Rust Lang</span></a></h3>
  <div class="b-serp-item__content"><div class="b-serp-item__text">Systems programming language</div></div>
</li>
<li class="serp-item">
  <h3 class="b-serp-item__title"><a class="b-serp-item__title-link" href="/search/site/?text=ad"><span>Relative ad</span></a></h3>
</li>
</ol>"#;
        let res = parse_results(html, "yandex");
        assert_eq!(res.results.len(), 2);
        assert_eq!(res.results[0].url, "https://rust-lang.org");
        assert_eq!(res.results[0].title, "Rust Lang");
        assert_eq!(res.results[0].content, "Systems programming language");
        // relative URLs get resolved against the yandex host
        assert!(res.results[1].url.starts_with("https://yandex.com"));
    }

    #[test]
    fn url_encode_keeps_safe_chars() {
        assert_eq!(urlencode("a-b_c.d~"), "a-b_c.d~");
    }
}