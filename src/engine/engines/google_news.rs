//! Google News engine. Port of `searx/engines/google_news.py` from SearXNG.
//!
//! Reuses Google's WML layout with `tbm=nws` and unwraps `/url?q=` redirects.

use async_trait::async_trait;
use scraper::{Html, Selector};

use crate::engine::engines::{attr, text_of, ScrapeEngineBase};
use crate::engine::http::HttpClient;
use crate::engine::{Engine, EngineError, EngineParams, EngineResults, SearchResult};

const SEARCH_URL: &str = "https://www.google.com/wml/search";

pub struct GoogleNewsEngine {
    base: ScrapeEngineBase,
}

impl GoogleNewsEngine {
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
impl Engine for GoogleNewsEngine {
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
        let start = (params.pageno.saturating_sub(1)) * 10;
        let mut args: Vec<(String, String)> = vec![
            ("q".into(), params.query.clone()),
            ("hl".into(), params.language().unwrap_or("en").split('-').next().unwrap_or("en").to_string()),
            ("tbm".into(), "nws".into()),
            ("ie".into(), "utf8".into()),
            ("oe".into(), "utf8".into()),
        ];
        if start > 0 {
            args.push(("start".into(), start.to_string()));
        }

        let qs = urlencode_pairs(&args);
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
        if status == 302 || (body.len() < 2000 && body.contains("/sorry/")) {
            return Err(EngineError::Captcha);
        }

        Ok(parse_results(&body, &self.base.name))
    }
}

fn unwrap_google_url(raw_url: &str) -> String {
    if let Some(rest) = raw_url.strip_prefix("/url?q=") {
        let decoded = percent_encoding::percent_decode_str(rest)
            .decode_utf8_lossy()
            .to_string();
        if let Some(pos) = decoded.find("&sa=U") {
            return decoded[..pos].to_string();
        }
        return decoded;
    }
    raw_url.to_string()
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

    let link_sel = Selector::parse(r#"a[href*="/url?q="]"#).unwrap();
    let mut seen = std::collections::HashSet::new();

    for link in doc.select(&link_sel) {
        let Some(raw_url) = attr(&link, "href") else { continue };
        let url = unwrap_google_url(&raw_url);
        if seen.contains(&url) || url.contains("google.com/search") {
            continue;
        }
        let title = span_text(&link, "M3vVJe")
            .or_else(|| span_text(&link, "fuLhoc"))
            .unwrap_or_default();
        if title.trim().is_empty() {
            continue;
        }
        let source = span_text(&link, "dXDvrc").unwrap_or_default();
        let pub_date = span_text(&link, "YVIcad").unwrap_or_default();
        let content = [source, pub_date]
            .into_iter()
            .filter(|s| !s.trim().is_empty())
            .collect::<Vec<_>>()
            .join(" / ");
        let thumbnail = link
            .select(&Selector::parse(r#"img[src*="encrypted-tbn"]"#).unwrap())
            .next()
            .and_then(|e| attr(&e, "src"));

        seen.insert(url.clone());
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

fn span_text(el: &scraper::ElementRef, class: &str) -> Option<String> {
    let sel = Selector::parse(&format!(r#"span[class*="{class}"]"#)).unwrap();
    el.select(&sel).next().map(|e| text_of(&e).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unwraps_google_redirect() {
        let raw = "/url?q=https%3A%2F%2Fexample.com%2Fa&sa=U&ved=x";
        assert_eq!(unwrap_google_url(raw), "https://example.com/a");
    }

    #[test]
    fn parses_fixture() {
        let html = r#"
<html><body>
<a href="/url?q=https%3A%2F%2Fnews.com%2Fs&sa=U">
  <span class="M3vVJe">Big Rust story</span>
  <span class="dXDvrc">Rust Weekly</span>
  <span class="YVIcad">2 days ago</span>
  <img src="https://encrypted-tbn0.gstatic.com/thumb">
</a>
<a href="/url?q=https%3A%2F%2Fgoogle.com%2Fsearch%3Fq%3Dx&sa=U">
  <span class="M3vVJe">goog</span>
</a>
</body></html>"#;
        let res = parse_results(html, "google_news");
        assert_eq!(res.results.len(), 1);
        assert_eq!(res.results[0].url, "https://news.com/s");
        assert_eq!(res.results[0].title, "Big Rust story");
        assert!(res.results[0].content.contains("Rust Weekly"));
        assert_eq!(res.results[0].thumbnail.as_deref(), Some("https://encrypted-tbn0.gstatic.com/thumb"));
    }
}
