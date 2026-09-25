//! Bing WEB engine. Port of `searx/engines/bing.py` from SearXNG.

use async_trait::async_trait;
use scraper::{Html, Selector};

use crate::engine::engines::{attr, text_of, ScrapeEngineBase};
use crate::engine::http::HttpClient;
use crate::engine::{Engine, EngineError, EngineParams, EngineResults, SearchResult};

const BASE_URL: &str = "https://www.bing.com";
const SAFESEARCH_MAP: [&str; 3] = ["off", "moderate", "strict"];

pub struct BingEngine {
    base: ScrapeEngineBase,
}

impl BingEngine {
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
impl Engine for BingEngine {
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
        let adlt = SAFESEARCH_MAP[(params.safesearch as usize).min(2)];
        let mut query_params: Vec<(String, String)> = vec![
            ("q".into(), params.query.clone()),
            ("adlt".into(), adlt.to_string()),
        ];
        if let Some(lang) = params.language() {
            let market = market_code(lang);
            query_params.push(("mkt".into(), market.to_string()));
        }

        let qs = query_params
            .iter()
            .map(|(k, v)| format!("{k}={}", google_encode(v)))
            .collect::<Vec<_>>()
            .join("&");
        let url = format!("{BASE_URL}/search?{qs}");

        tracing::info!("[CALL] engine::bing(url={})", url);

        let resp = client
            .get(&url, Some(lang_header(params.language())))
            .await
            .map_err(|e| EngineError::Request(e.to_string()))?;

        let status = resp.status().as_u16();
        tracing::info!("[RESP] engine::bing status={}, url={}", status, url);

        if let Some(err) = EngineError::from_status(status) {
            return Err(err);
        }

        let body = resp
            .text()
            .await
            .map_err(|e| EngineError::Request(e.to_string()))?;

        tracing::info!("[RESP] engine::bing body_len={}, url={}", body.len(), url);
        
        let results = parse_results(&body, &self.base.name);
        tracing::info!("[RESP] engine::bing results={}, url={}", results.results.len(), url);
        
        Ok(results)
    }
}

/// en-US → en-US; en → en-US; de → de-DE, etc. (Bing `mkt` parameter).
fn market_code(lang: &str) -> String {
    let parts: Vec<&str> = lang.split('-').collect();
    let primary = parts[0].to_lowercase();
    let country = match parts.get(1) {
        Some(c) => c.to_uppercase(),
        None => default_country(&primary),
    };
    format!("{primary}-{country}")
}

fn default_country(lang: &str) -> String {
    match lang {
        "en" => "US".to_string(),
        "pt" => "BR".to_string(),
        other => other.to_uppercase(),
    }
}

fn lang_header(lang: Option<&str>) -> &str {
    lang.unwrap_or("en-US,en;q=0.9")
}

fn google_encode(s: &str) -> String {
    const SET: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'_')
        .remove(b'.')
        .remove(b'~');
    percent_encoding::utf8_percent_encode(s, SET).to_string()
}

/// Decode Bing's `/ck/a?u=a1<base64url>` redirect URLs (port of the `response`
/// in `bing.py`).
fn decode_bing_redirect(href: &str) -> Option<String> {
    let stripped = href.strip_prefix("https://www.bing.com/ck/a?")?;
    let u_val = stripped
        .split('&')
        .find_map(|kv| kv.strip_prefix("u="))?;
    if let Some(encoded) = u_val.strip_prefix("a1") {
        let mut encoded = encoded.to_string();
        // base64url without padding
        while encoded.len() % 4 != 0 {
            encoded.push('=');
        }
        use base64::Engine;
        let decoded = base64::engine::general_purpose::URL_SAFE
            .decode(encoded.as_bytes())
            .ok()?;
        return Some(String::from_utf8_lossy(&decoded).to_string());
    }
    None
}

/// Naive removal of Bing's `algoSlug_icon` spans from an element's inner HTML.
/// These are decorative glyphs, not search snippets.
fn strip_algo_icons(inner_html: &str) -> String {
    use regex::Regex;
    let re = Regex::new(r#"(?s)<span\s+class="?algoSlug_icon"?[^>]*>.*?</span>"#)
        .unwrap_or_else(|_| Regex::new(r"(?s)<span[^>]*>.*?</span>").unwrap());
    re.replace_all(inner_html, "").to_string()
}

fn parse_results(body: &str, engine_name: &str) -> EngineResults {
    let mut out = EngineResults::default();
    let doc = Html::parse_document(body);

    let algo_sel = Selector::parse("ol#b_results > li.b_algo").unwrap();
    let link_sel = Selector::parse("h2 > a").unwrap();
    let content_sel = Selector::parse("p").unwrap();

    for item in doc.select(&algo_sel) {
        let Some(link) = item.select(&link_sel).next() else {
            continue;
        };
        let Some(href) = attr(&link, "href") else {
            continue;
        };
        let title = text_of(&link);
        if href.is_empty() || title.is_empty() {
            continue;
        }
        let url = decode_bing_redirect(&href).unwrap_or_else(|| href.clone());

        // Bing injects decorative icons into <p>; drop their markup before
        // extracting text (see `bing.py`).
        let content_parts: Vec<String> = item
            .select(&content_sel)
            .filter_map(|p| {
                let icon_free_html = strip_algo_icons(&p.inner_html());
                let fragment = Html::parse_fragment(&icon_free_html);
                let text = fragment.root_element().text().collect::<Vec<_>>().join(" ");
                (!text.trim().is_empty()).then_some(text)
            })
            .collect();
        let content = content_parts.join(" ");

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_ck_redirect() {
        // base64url of "https://example.com/actual"
        let encoded = "aHR0cHM6Ly9leGFtcGxlLmNvbS9hY3R1YWw";
        let href = format!("https://www.bing.com/ck/a?u=a1{encoded}&ntb=1");
        assert_eq!(
            decode_bing_redirect(&href).unwrap(),
            "https://example.com/actual"
        );
    }

    #[test]
    fn leaves_plain_url() {
        let href = "https://example.com/page?x=1";
        assert_eq!(decode_bing_redirect(href), None);
    }

    #[test]
    fn parses_fixture() {
        let html = r#"
<html><body><ol id="b_results">
<li class="b_algo"><h2><a href="https://rust-lang.org">Rust</a></h2>
<p><span class="algoSlug_icon">x</span>A systems language</p></li>
<li class="b_algo"><h2><a href="https://example.com">Example</a></h2><p>Text</p></li>
</ol></body></html>"#;
        let res = parse_results(html, "bing");
        assert_eq!(res.results.len(), 2);
        assert_eq!(res.results[0].url, "https://rust-lang.org");
        assert_eq!(res.results[0].title, "Rust");
        assert!(res.results[0].content.contains("systems"));
    }

    #[test]
    fn market_code_builds_mkt() {
        assert_eq!(market_code("en"), "en-US");
        assert_eq!(market_code("en-US"), "en-US");
        assert_eq!(market_code("de"), "de-DE");
        assert_eq!(market_code("zh-CN"), "zh-CN");
    }
}