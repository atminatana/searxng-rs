//! Startpage WEB search. Port of `searx/engines/startpage.py` from SearXNG
//! (the `web` category only).
//!
//! Startpage requires a POST with an `sc` code scraped from its form, plus a
//! `preferences` cookie. Results are embedded as JSON in the page's script.

use async_trait::async_trait;
use scraper::{Html, Selector};

use crate::engine::engines::{attr, ScrapeEngineBase};
use crate::engine::http::HttpClient;
use crate::engine::{Engine, EngineError, EngineParams, EngineResults, SearchResult};

const BASE_URL: &str = "https://www.startpage.com";
const SEARCH_URL: &str = "https://www.startpage.com/sp/search";

const SAFESEARCH_MAP: [&str; 3] = ["none", "moderate", "heavy"];

pub struct StartpageEngine {
    base: ScrapeEngineBase,
}

impl StartpageEngine {
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
impl Engine for StartpageEngine {
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
        let sc = fetch_sc_code(client).await?;

        let mut args: Vec<(String, String)> = vec![
            ("query".into(), params.query.clone()),
            ("cat".into(), "web".into()),
            ("t".into(), "device".into()),
            ("sc".into(), sc),
            ("with_date".into(), String::new()),
            ("abd".into(), "1".into()),
            ("abe".into(), "1".into()),
            ("qsr".into(), "all".into()),
            ("qadf".into(), SAFESEARCH_MAP[(params.safesearch as usize).min(2)].into()),
        ];
        if let Some(lang) = params.language() {
            let lang = lang.split('-').next().unwrap_or("en").to_string();
            args.push(("language".into(), lang.clone()));
            args.push(("lui".into(), lang));
        }
        if params.pageno > 1 {
            args.push(("page".into(), params.pageno.to_string()));
            args.push(("segment".into(), "startpage.udog".into()));
        }

        let cookie = build_preferences_cookie(params);

        let resp = client
            .post_with(SEARCH_URL, &args, params.language(), &[("preferences".to_string(), cookie)])
            .await
            .map_err(|e| EngineError::Request(e.to_string()))?;

        let status = resp.status().as_u16();
        if let Some(location) = resp.headers().get("location").and_then(|v| v.to_str().ok()) {
            if location.starts_with("https://www.startpage.com/sp/captcha") {
                return Err(EngineError::Captcha);
            }
        }
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

/// Scrape a fresh `sc` code from Startpage's home page search form.
async fn fetch_sc_code(client: &HttpClient) -> Result<String, EngineError> {
    let resp = client
        .get(&format!("{BASE_URL}/"), Some("en-US,en;q=0.5"))
        .await
        .map_err(|e| EngineError::Request(e.to_string()))?;

    if let Some(location) = resp.headers().get("location").and_then(|v| v.to_str().ok()) {
        if location.starts_with("https://www.startpage.com/sp/captcha") {
            return Err(EngineError::Captcha);
        }
    }

    let body = resp
        .text()
        .await
        .map_err(|e| EngineError::Request(e.to_string()))?;
    let doc = Html::parse_document(&body);
    let form_sel = Selector::parse(r#"form[id="search"]"#).unwrap();
    let sc_sel = Selector::parse(r#"input[name="sc"]"#).unwrap();

    if let Some(input) = doc
        .select(&form_sel)
        .next()
        .and_then(|form| form.select(&sc_sel).next())
    {
        if let Some(value) = attr(&input, "value") {
            if !value.is_empty() {
                return Ok(value);
            }
        }
    }
    Err(EngineError::Captcha)
}

/// Build the `preferences` cookie exactly as Startpage's search form does.
fn build_preferences_cookie(params: &EngineParams) -> String {
    let language = params.language().and_then(|l| l.split('-').next()).unwrap_or("en").to_string();
    let engine_language = if params.language().is_some() { Some(language) } else { None };

    let mut cookie: Vec<(String, String)> = vec![
        ("date_time".into(), "world".into()),
        ("disable_family_filter".into(), SAFESEARCH_MAP[(params.safesearch as usize).min(2)].into()),
        ("disable_open_in_new_window".into(), "0".into()),
        ("enable_post_method".into(), "1".into()),
        ("enable_proxy_safety_suggest".into(), "1".into()),
        ("enable_stay_control".into(), "1".into()),
        ("instant_answers".into(), "1".into()),
        ("lang_homepage".into(), "s/device/en/".into()),
        ("num_of_results".into(), "10".into()),
        ("suggestions".into(), "1".into()),
        ("wt_unit".into(), "celsius".into()),
    ];
    if let Some(lang) = &engine_language {
        cookie.push(("language".into(), lang.clone()));
        cookie.push(("language_ui".into(), lang.clone()));
    }
    cookie
        .into_iter()
        .map(|(k, v)| format!("{k}EEE{v}"))
        .collect::<Vec<_>>()
        .join("N1N")
}

/// Extract the embedded JSON results object from Startpage's page.
fn parse_results(body: &str, engine_name: &str) -> EngineResults {
    let mut out = EngineResults::default();
    let Some(json) = extract_embedded_json(body) else {
        return out;
    };

    let regions = json
        .get("render")
        .and_then(|r| r.get("presenter"))
        .and_then(|p| p.get("regions"))
        .and_then(|r| r.get("mainline"))
        .and_then(|m| m.as_array());

    let Some(regions) = regions else {
        return out;
    };

    for region in regions {
        let Some(display_type) = region.get("display_type").and_then(|v| v.as_str()) else {
            continue;
        };
        if display_type != "web-google" {
            continue;
        }
        let results = region.get("results").and_then(|r| r.as_array());
        let Some(results) = results else { continue };
        for item in results {
            let Some(click_url) = item.get("clickUrl").and_then(|v| v.as_str()) else {
                continue;
            };
            let Some(title) = item.get("title").and_then(|v| v.as_str()) else {
                continue;
            };
            let description = item.get("description").and_then(|v| v.as_str()).unwrap_or("");
            out.add(SearchResult {
                url: click_url.to_string(),
                title: title.to_string(),
                content: description.to_string(),
                engine: engine_name.to_string(),
                ..Default::default()
            });
        }
    }
    out
}

/// Extract the JSON object passed to `React.createElement(UIStartpage.AppSerpWeb, {...})`
/// by balanced-brace scanning (ignoring braces inside quoted strings).
fn extract_embedded_json(body: &str) -> Option<serde_json::Value> {
    let marker = "React.createElement(UIStartpage.AppSerpWeb, {";
    let marker_end = body.find(marker)? + marker.len();
    let bytes = &body[marker_end - 1..];
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut escaped = false;
    for (i, c) in bytes.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return serde_json::from_str(&bytes[..=i]).ok();
                }
            }
            '"' => in_string = true,
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_embedded_json() {
        let html = r#"<html>
<script>
React.createElement(UIStartpage.AppSerpWeb, {"render":{"presenter":{"regions":{"mainline":[
  {"display_type":"web-google","results":[
    {"clickUrl":"https://rust-lang.org","title":"Rust Lang","description":"A systems language"}
  ]},
  {"display_type":"news-bing","results":[]}
]}}}}, {})
</script>
</html>"#;
        let res = parse_results(html, "startpage");
        assert_eq!(res.results.len(), 1);
        assert_eq!(res.results[0].url, "https://rust-lang.org");
        assert_eq!(res.results[0].title, "Rust Lang");
    }

    #[test]
    fn builds_preferences_cookie() {
        let params = EngineParams {
            query: "q".into(),
            languages: vec!["en".into()],
            safesearch: 1,
            pageno: 1,
            category: "general".into(),
        };
        let cookie = build_preferences_cookie(&params);
        assert!(cookie.contains("disable_family_filterEEEmoderate"));
        assert!(cookie.contains("languageEEEen"));
    }
}
