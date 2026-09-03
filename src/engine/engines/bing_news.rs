//! Bing News engine. Port of `searx/engines/bing_news.py` from SearXNG.

use async_trait::async_trait;
use scraper::{Html, Selector};

use crate::engine::engines::{attr, text_of, ScrapeEngineBase};
use crate::engine::http::HttpClient;
use crate::engine::{Engine, EngineError, EngineParams, EngineResults, SearchResult};

const BASE_URL: &str = "https://www.bing.com";

pub struct BingNewsEngine {
    base: ScrapeEngineBase,
}

impl BingNewsEngine {
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
impl Engine for BingNewsEngine {
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
        let mut query_params: Vec<(String, String)> = vec![
            ("q".into(), params.query.clone()),
            ("InfiniteScroll".into(), "1".into()),
            ("first".into(), (page * 10 + 1).to_string()),
            ("SFX".into(), page.to_string()),
            ("form".into(), "PTFTNR".into()),
        ];
        if let Some(lang) = params.language() {
            let market = market_code(lang);
            query_params.push(("mkt".into(), market));
        }

        let qs = query_params
            .iter()
            .map(|(k, v)| format!("{k}={}", urlencode(v)))
            .collect::<Vec<_>>()
            .join("&");
        let url = format!("{BASE_URL}/news/infinitescrollajax?{qs}");

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

fn market_code(lang: &str) -> String {
    let parts: Vec<&str> = lang.split('-').collect();
    let primary = parts[0].to_lowercase();
    let country = match parts.get(1) {
        Some(c) => c.to_uppercase(),
        None => match primary.as_str() {
            "en" => "US".to_string(),
            "pt" => "BR".to_string(),
            other => other.to_uppercase(),
        },
    };
    format!("{primary}-{country}")
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

    let newsitem_sel = Selector::parse(r#"div[class~="newsitem"]"#).unwrap();
    let title_sel = Selector::parse(r#"a[class="title"]"#).unwrap();
    let snippet_sel = Selector::parse(r#"div[class="snippet"]"#).unwrap();
    let source_sel = Selector::parse(r#"div[class*="source"]"#).unwrap();
    let source_label_sel = Selector::parse(r#"span[aria-label]"#).unwrap();
    let thumb_sel = Selector::parse(r#"a[class="imagelink"] img"#).unwrap();

    for item in doc.select(&newsitem_sel) {
        let Some(title_link) = item.select(&title_sel).next() else { continue };
        let Some(url) = attr(&title_link, "href") else { continue };
        let title = text_of(&title_link);
        if title.trim().is_empty() {
            continue;
        }
        let content = item
            .select(&snippet_sel)
            .next()
            .map(|e| text_of(&e))
            .unwrap_or_default();

        // source metadata: aria-label + author
        let meta_parts: Vec<String> = std::iter::once(
            item.select(&source_sel)
                .next()
                .and_then(|s| s.select(&source_label_sel).next())
                .and_then(|e| attr(&e, "aria-label"))
                .unwrap_or_default(),
        )
        .chain(std::iter::once(
            title_link.value().attr("data-author").map(|s| s.to_string()).unwrap_or_default(),
        ))
        .filter(|s| !s.trim().is_empty())
        .collect();
        let metadata = meta_parts.join(" | ");

        let thumbnail = item
            .select(&thumb_sel)
            .next()
            .and_then(|e| attr(&e, "src"))
            .map(|src| {
                if src.starts_with("https://www.bing.com") {
                    src
                } else {
                    let rel = src.strip_prefix('/').unwrap_or(&src);
                    format!("{BASE_URL}/{rel}")
                }
            });

        let mut content = content;
        if !metadata.is_empty() {
            content = if content.is_empty() {
                metadata
            } else {
                format!("{content}\n{metadata}")
            };
        }
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
<div class="newsitem">
  <a class="title" href="https://www.bing.com/news/1" data-author="Reuters">Rust story</a>
  <div class="snippet">A great article</div>
  <div class="source"><span aria-label="Rust Weekly"></span></div>
  <a class="imagelink"><img src="/th?id=abc"></a>
</div>
</body></html>"#;
        let res = parse_results(html, "bing_news");
        assert_eq!(res.results.len(), 1);
        assert_eq!(res.results[0].title, "Rust story");
        assert!(res.results[0].content.contains("A great article"));
        assert!(res.results[0].content.contains("Rust Weekly"));
        assert!(res.results[0].content.contains("Reuters"));
        assert_eq!(res.results[0].thumbnail.as_deref(), Some("https://www.bing.com/th?id=abc"));
    }

    #[test]
    fn empty() {
        let res = parse_results("<html></html>", "bing_news");
        assert!(res.results.is_empty());
    }
}
