//! Bing Images engine. Port of `searx/engines/bing_images.py` from SearXNG.

use async_trait::async_trait;
use scraper::{Html, Selector};

use crate::engine::engines::{attr, text_of, ScrapeEngineBase};
use crate::engine::http::HttpClient;
use crate::engine::{Engine, EngineError, EngineParams, EngineResults, SearchResult};

const BASE_URL: &str = "https://www.bing.com";

pub struct BingImagesEngine {
    base: ScrapeEngineBase,
}

impl BingImagesEngine {
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
impl Engine for BingImagesEngine {
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
        let first = (params.pageno.saturating_sub(1)) * 35 + 1;
        let mut query_params: Vec<(String, String)> = vec![
            ("q".into(), params.query.clone()),
            ("async".into(), "1".into()),
            ("first".into(), first.to_string()),
            ("count".into(), "35".into()),
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
        let url = format!("{BASE_URL}/images/async?{qs}");

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

    let item_sel = Selector::parse(r#"ul[class*="dgControl_list"] > li"#).unwrap();
    let iusc_sel = Selector::parse(r#"a[class="iusc"]"#).unwrap();
    let title_sel = Selector::parse(r#"div[class="infnmpt"] a"#).unwrap();
    let source_sel = Selector::parse(r#"div[class="lnkw"] a"#).unwrap();

    for item in doc.select(&item_sel) {
        let Some(iusc) = item.select(&iusc_sel).next() else { continue };
        let Some(m_raw) = attr(&iusc, "m") else { continue };
        let Ok(md) = serde_json::from_str::<serde_json::Value>(&m_raw) else { continue };

        let url = md.get("purl").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let img_src = md.get("murl").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let thumb = md.get("turl").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if url.is_empty() || img_src.is_empty() {
            continue;
        }
        let desc = md.get("desc").and_then(|v| v.as_str()).unwrap_or("");

        let title = item
            .select(&title_sel)
            .next()
            .map(|e| text_of(&e))
            .unwrap_or_default();
        let source = item
            .select(&source_sel)
            .next()
            .map(|e| text_of(&e))
            .unwrap_or_default();
        let content = if source.is_empty() {
            desc.to_string()
        } else {
            format!("{source} - {desc}")
        };

        out.add(SearchResult {
            url,
            title,
            content,
            engine: engine_name.to_string(),
            img_src: Some(img_src),
            thumbnail: (!thumb.is_empty()).then_some(thumb),
            ..Default::default()
        });
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
<ul class="dgControl_list">
  <li>
    <a class="iusc" m="{&quot;purl&quot;:&quot;https://example.com/page&quot;,&quot;murl&quot;:&quot;https://img.com/photo.jpg&quot;,&quot;turl&quot;:&quot;https://thumb.com/t.jpg&quot;,&quot;desc&quot;:&quot;a photo&quot;}"></a>
    <div class="infnmpt"><a>Photo title</a></div>
    <div class="imgpt"><div class="lnkw"><a>source.com</a></div></div>
  </li>
</ul>
</body></html>"#;
        let res = parse_results(html, "bing_images");
        assert_eq!(res.results.len(), 1);
        assert_eq!(res.results[0].url, "https://example.com/page");
        assert_eq!(res.results[0].img_src.as_deref(), Some("https://img.com/photo.jpg"));
        assert_eq!(res.results[0].thumbnail.as_deref(), Some("https://thumb.com/t.jpg"));
        assert_eq!(res.results[0].title, "Photo title");
    }

    #[test]
    fn empty() {
        let res = parse_results("<html></html>", "bing_images");
        assert!(res.results.is_empty());
    }
}
