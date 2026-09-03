//! Google Images engine. Port of `searx/engines/google_images.py` from SearXNG.

use async_trait::async_trait;
use scraper::{Html, Selector};

use crate::engine::engines::{attr, ScrapeEngineBase};
use crate::engine::http::HttpClient;
use crate::engine::{Engine, EngineError, EngineParams, EngineResults, SearchResult};

const SEARCH_URL: &str = "https://www.google.com/wml/search";

pub struct GoogleImagesEngine {
    base: ScrapeEngineBase,
}

impl GoogleImagesEngine {
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
impl Engine for GoogleImagesEngine {
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
        let filter = if params.safesearch > 0 { "active" } else { "images" };
        let mut args: Vec<(String, String)> = vec![
            ("q".into(), params.query.clone()),
            ("hl".into(), params.language().unwrap_or("en").split('-').next().unwrap_or("en").to_string()),
            ("tbm".into(), "isch".into()),
            ("safe".into(), filter.into()),
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

    let link_sel = Selector::parse(r#"a[href*="/imgres?"]"#).unwrap();

    for link in doc.select(&link_sel) {
        let Some(href) = attr(&link, "href") else { continue };
        let Some(qs) = href.split_once('?').map(|(_, q)| q) else { continue };
        let query = parse_qs(qs);

        let img_src = query.get("imgurl").cloned().unwrap_or_default();
        let url = query.get("imgrefurl").cloned().unwrap_or_default();
        if img_src.is_empty() || url.is_empty() {
            continue;
        }
        let width = query.get("w").cloned().unwrap_or_default();
        let height = query.get("h").cloned().unwrap_or_default();
        let tbnid = query.get("tbnid").cloned().unwrap_or_default();

        let filename = url::Url::parse(&img_src)
            .ok()
            .and_then(|u| {
                u.path_segments()
                    .and_then(|mut s| s.next_back().map(|f| f.to_string()))
            })
            .unwrap_or_default();
        let title = percent_encoding::percent_decode_str(&filename)
            .decode_utf8_lossy()
            .to_string();
        let title = if title.is_empty() {
            url::Url::parse(&url).ok().and_then(|u| u.host_str().map(|h| h.to_string())).unwrap_or_default()
        } else {
            title
        };

        let resolution = if width.is_empty() && height.is_empty() {
            String::new()
        } else {
            format!("{width} x {height}")
        };

        let tbnid = if tbnid.is_empty() {
            // reuse a generic query when no tbnid is present
            img_src.clone()
        } else {
            tbnid
        };
        let thumb_src = format!("https://encrypted-tbn0.gstatic.com/images?q=tbn:{tbnid}");

        out.add(SearchResult {
            url,
            title,
            engine: engine_name.to_string(),
            img_src: Some(img_src),
            thumbnail: Some(thumb_src),
            content: resolution,
            ..Default::default()
        });
    }
    out
}

/// Minimal `a=1&b=2` parser, URL-decoded.
fn parse_qs(qs: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for pair in qs.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            if let Ok(k) = percent_encoding::percent_decode_str(k).decode_utf8() {
                if let Ok(v) = percent_encoding::percent_decode_str(v).decode_utf8() {
                    map.insert(k.into_owned(), v.into_owned());
                }
            }
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fixture() {
        let html = r#"
<html><body>
<a href="/imgres?imgurl=https%3A%2F%2Fimg.com%2Fphoto.jpg&imgrefurl=https%3A%2F%2Fexample.com%2Fpage&w=640&h=480&tbnid=123abc"></a>
</body></html>"#;
        let res = parse_results(html, "google_images");
        assert_eq!(res.results.len(), 1);
        assert_eq!(res.results[0].url, "https://example.com/page");
        assert_eq!(res.results[0].title, "photo.jpg");
        assert_eq!(res.results[0].img_src.as_deref(), Some("https://img.com/photo.jpg"));
        assert_eq!(res.results[0].thumbnail.as_deref(), Some("https://encrypted-tbn0.gstatic.com/images?q=tbn:123abc"));
        assert_eq!(res.results[0].content, "640 x 480");
    }

    #[test]
    fn empty() {
        let res = parse_results("<html></html>", "google_images");
        assert!(res.results.is_empty());
    }
}
