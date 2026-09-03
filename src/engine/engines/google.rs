//! Google WEB engine. Port of `searx/engines/google.py` from SearXNG.
//!
//! Uses Google's WML (mobile) layout with a Nokia user agent to avoid JS.

use async_trait::async_trait;
use scraper::{Html, Selector};

use crate::engine::http::HttpClient;
use crate::engine::{Engine, EngineError, EngineParams, EngineResults, SearchResult};
use crate::engine::engines::{attr, text_of, ScrapeEngineBase};

const SEARCH_URL: &str = "https://www.google.com/wml/search";

/// Nokia user agents used to request Google's WML layout. Not yet wired into
/// request generation.
#[allow(dead_code)]
const NOKIA_USERAGENTS: [&str; 6] = [
    "Nokia7610/2.0 (5.0509.0) SymbianOS/7.0s Series60/2.1 Profile/MIDP-2.0 Configuration/CLDC-1.0",
    "Nokia7610/2.0 (7.0642.0) SymbianOS/7.0s Series60/2.1 Profile/MIDP-2.0 Configuration/CLDC-1.0",
    "Nokia6230/2.0 (05.50) Profile/MIDP-2.0 Configuration/CLDC-1.1",
    "Nokia6230i/2.0 (03.80) Profile/MIDP-2.0 Configuration/CLDC-1.1",
    "Nokia6280/2.0 (03.60) Profile/MIDP-2.0 Configuration/CLDC-1.1",
    "NokiaN72/2.0617.1.0.3 Series60/2.8 Profile/MIDP-2.0 Configuration/CLDC-1.1",
];

const FILTER_MAPPING: [&str; 3] = ["off", "medium", "high"];

pub struct GoogleEngine {
    base: ScrapeEngineBase,
}

impl GoogleEngine {
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
impl Engine for GoogleEngine {
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
            (
                "lr".into(),
                if params.language().is_some() {
                    format!("lang_{}", params.language().unwrap_or("en").split('-').next().unwrap_or("en"))
                } else {
                    String::new()
                },
            ),
            ("ie".into(), "utf8".into()),
            ("oe".into(), "utf8".into()),
            ("sca_esv".into(), "1".into()),
        ];
        if start > 0 {
            args.push(("start".into(), start.to_string()));
        }
        if params.safesearch > 0 {
            let idx = (params.safesearch as usize).min(2);
            args.push(("safe".into(), FILTER_MAPPING[idx].to_string()));
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

        detect_sorry(&body, status)?;

        Ok(parse_results(&body, &self.base.name))
    }
}

fn detect_sorry(body: &str, status: u16) -> Result<(), EngineError> {
    if status == 302 {
        return Err(EngineError::Captcha);
    }
    if body.len() < 2000 && body.contains("/sorry/") {
        return Err(EngineError::Captcha);
    }
    Ok(())
}

/// Port of `unwrap_google_url`: strip the `/url?q=` redirector.
fn unwrap_google_url(raw_url: &str) -> String {
    if let Some(rest) = raw_url.strip_prefix("/url?q=") {
        let decoded = percent_encoding_unescape(rest);
        if let Some(pos) = decoded.find("&sa=U") {
            return decoded[..pos].to_string();
        }
        return decoded;
    }
    raw_url.to_string()
}

fn percent_encoding_unescape(s: &str) -> String {
    percent_encoding::percent_decode_str(s)
        .decode_utf8_lossy()
        .to_string()
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

    let results_sel = Selector::parse("div.zMzFAb").unwrap();
    let title_sel = Selector::parse("a.fuLhoc span.CVA68e").unwrap();
    let link_sel = Selector::parse("a.fuLhoc").unwrap();
    let content_sel = Selector::parse("div.taTFJ span.FrIlee").unwrap();
    let thumb_sel = Selector::parse("img[src*='encrypted-tbn']").unwrap();
    let suggestion_sel = Selector::parse("table.HExoMb a.ZWRArf").unwrap();

    for result in doc.select(&results_sel) {
        let Some(title_el) = result.select(&title_sel).next() else {
            continue;
        };
        let title = text_of(&title_el);
        let Some(link_el) = result.select(&link_sel).next() else {
            continue;
        };
        let Some(raw_url) = attr(&link_el, "href") else {
            continue;
        };
        if raw_url.is_empty() || title.is_empty() {
            continue;
        }
        let url = unwrap_google_url(&raw_url);
        let content = result
            .select(&content_sel)
            .next()
            .map(|e| text_of(&e))
            .unwrap_or_default();
        let thumbnail = result
            .select(&thumb_sel)
            .next()
            .and_then(|e| attr(&e, "src"));

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

    for suggestion in doc.select(&suggestion_sel) {
        let text = text_of(&suggestion);
        if !text.is_empty() {
            out.suggestions.push(text);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unwraps_google_redirect() {
        let raw = "/url?q=https%3A%2F%2Fexample.com%2Fpage%3Fa%3D1&sa=U&ved=...";
        assert_eq!(unwrap_google_url(raw), "https://example.com/page?a=1");
    }

    #[test]
    fn detects_sorry_short_page() {
        let body = "<html>... <a href='/sorry/index'>...</html>";
        assert!(matches!(detect_sorry(body, 200), Err(EngineError::Captcha)));
    }

    #[test]
    fn parses_fixture() {
        let html = r#"
<html><body>
<div class="zMzFAb">
  <a class="fuLhoc" href="/url?q=https%3A%2F%2Frust-lang.org&sa=U"><span class="CVA68e">Rust</span></a>
  <div class="taTFJ"><span class="FrIlee">A language empowering everyone.</span></div>
</div>
<table class="HExoMb"><tr><td><a class="ZWRArf">rust</a></td></tr></table>
</body></html>"#;
        let res = parse_results(html, "google");
        assert_eq!(res.results.len(), 1);
        assert_eq!(res.results[0].url, "https://rust-lang.org");
        assert_eq!(res.results[0].title, "Rust");
        assert_eq!(res.suggestions, vec!["rust"]);
    }

    #[test]
    fn url_encode_keeps_safe_chars() {
        assert_eq!(urlencode("rust lang"), "rust%20lang");
        assert_eq!(urlencode("a-b_c.d~"), "a-b_c.d~");
    }
}