//! DuckDuckGo WEB engine (no-JS html form). Port of
//! `searx/engines/duckduckgo.py` from SearXNG.

use async_trait::async_trait;
use scraper::{Html, Selector};

use crate::engine::engines::{attr, text_of, ScrapeEngineBase};
use crate::engine::http::HttpClient;
use crate::engine::{Engine, EngineError, EngineParams, EngineResults, SearchResult};

const DDG_URL: &str = "https://html.duckduckgo.com/html/";

pub struct DuckDuckGoEngine {
    base: ScrapeEngineBase,
}

impl DuckDuckGoEngine {
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
impl Engine for DuckDuckGoEngine {
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
        if params.query.len() >= 500 {
            // DDG does not accept queries with more than 499 chars
            return Ok(EngineResults::default());
        }

        let mut form: Vec<(&str, String)> = vec![
            ("q", quote_ddg_bangs(&params.query)),
            ("b", String::new()),
        ];
        // kl: region (default: all regions)
        let kl = params
            .language()
            .map(ddg_region)
            .unwrap_or_else(|| "wt-wt".to_string());
        form.push(("kl", kl));
        if params.pageno > 1 {
            // vqd is required for follow-up pages; without it we risk being
            // flagged as a bot (see `duckduckgo.py`).
            return Err(EngineError::Captcha);
        }

        let resp = client
            .post(DDG_URL, &form, params.language())
            .await
            .map_err(|e| EngineError::Request(e.to_string()))?;

        let status = resp.status().as_u16();
        if status == 303 {
            return Ok(EngineResults::default());
        }
        if let Some(err) = EngineError::from_status(status) {
            return Err(err);
        }

        let body = resp
            .text()
            .await
            .map_err(|e| EngineError::Request(e.to_string()))?;

        parse_results(&body, &self.base.name)
    }
}

/// Quote DDG `!bang` directives so DDG doesn't redirect on them.
fn quote_ddg_bangs(query: &str) -> String {
    let known: [&str; 10] = ["g", "ddg", "bing", "yt", "wt", "gh", "so", "npm", "crates", "rstdocs"];
    query
        .split_whitespace()
        .map(|tok| {
            if tok.starts_with('!') && known.contains(&&tok[1..]) {
                format!("'{tok}'")
            } else {
                tok.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Simple DDG region mapping from a language tag.
fn ddg_region(lang: &str) -> String {
    match lang.to_lowercase().as_str() {
        "en" => "us-en",
        "en-us" => "us-en",
        "en-gb" => "uk-en",
        "de" => "de-de",
        "fr" => "fr-fr",
        "fr-fr" => "fr-fr",
        "ru" => "ru-ru",
        "es" => "es-es",
        "pt" => "br-pt",
        "pt-br" => "br-pt",
        "zh" => "cn-zh",
        "zh-cn" => "cn-zh",
        _ => "wt-wt",
    }
    .to_string()
}

fn parse_results(body: &str, engine_name: &str) -> Result<EngineResults, EngineError> {
    let mut out = EngineResults::default();
    let doc = Html::parse_document(body);

    // DDG CAPTCHA dialog.
    if doc.select(&Selector::parse("form#challenge-form").unwrap()).next().is_some() {
        return Err(EngineError::Captcha);
    }

    let result_sel = Selector::parse("div#links > div.web-result").unwrap();
    let title_sel = Selector::parse("h2 > a").unwrap();
    let snippet_sel = Selector::parse("a.result__snippet").unwrap();
    let zero_click_sel = Selector::parse("div#zero_click_abstract").unwrap();

    for div in doc.select(&result_sel) {
        let Some(title_el) = div.select(&title_sel).next() else {
            continue;
        };
        let title = text_of(&title_el);
        let Some(raw_url) = attr(&title_el, "href") else {
            continue;
        };
        let content = div
            .select(&snippet_sel)
            .next()
            .map(|e| text_of(&e))
            .unwrap_or_default();

        out.add(SearchResult {
            url: raw_url,
            title,
            content,
            engine: engine_name.to_string(),
            ..Default::default()
        });
    }

    // DDG instant answer (zero_click_abstract).
    if let Some(zc) = doc.select(&zero_click_sel).next() {
        let text = text_of(&zc);
        if !text.is_empty()
            && !text.contains("Your IP address is")
            && !text.contains("Your user agent:")
            && !text.contains("URL Decoded:")
        {
            out.answers.push(text);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_bangs() {
        assert_eq!(quote_ddg_bangs("!g rust"), "'!g' rust");
        assert_eq!(quote_ddg_bangs("hello world"), "hello world");
    }

    #[test]
    fn ddg_region_map() {
        assert_eq!(ddg_region("en-US"), "us-en");
        assert_eq!(ddg_region("de"), "de-de");
        assert_eq!(ddg_region("xx"), "wt-wt");
    }

    #[test]
    fn parses_fixture() {
        let html = r#"
<html><body>
<div id="links">
  <div class="web-result"><h2><a href="https://rust-lang.org">Rust Lang</a></h2>
    <a class="result__snippet">Systems programming</a></div>
  <div class="result--ad"><h2><a href="https://ads.com">Ad</a></h2></div>
</div>
</body></html>"#;
        let res = parse_results(html, "duckduckgo").unwrap();
        assert_eq!(res.results.len(), 1);
        assert_eq!(res.results[0].url, "https://rust-lang.org");
    }

    #[test]
    fn detects_captcha() {
        let html = "<html><body><form id=\"challenge-form\"></form></body></html>";
        assert!(matches!(parse_results(html, "ddg"), Err(EngineError::Captcha)));
    }
}