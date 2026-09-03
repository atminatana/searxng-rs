//! Brave WEB search engine. Port of `searx/engines/brave.py` from SearXNG
//! (the `brave_category = "search"` WEB category only; images/videos/news are
//! not ported).
//!
//! Brave results are scraped from `search.brave.com/search` HTML; the region,
//! UI language and safe-search level are set via cookies.

use async_trait::async_trait;
use scraper::{Html, Selector};

use crate::engine::engines::{attr, text_of, ScrapeEngineBase};
use crate::engine::http::HttpClient;
use crate::engine::{Engine, EngineError, EngineParams, EngineResults, SearchResult};

const BASE_URL: &str = "https://search.brave.com";
const SAFESEARCH_COOKIE: [&str; 3] = ["off", "moderate", "strict"];
const DEFAULT_UI_LANG: &str = "en-us";
const UI_LANGS: &[&str] = &[
    "ca", "de-de", "en", "en-ca", "en-gb", "en-us", "es", "fr-ca", "fr-fr", "ja-jp", "pt-br",
];

pub struct BraveEngine {
    base: ScrapeEngineBase,
}

impl BraveEngine {
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
impl Engine for BraveEngine {
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
        let url = build_url(&params.query, params.pageno);
        let cookies = cookies_for(params);
        let lang = params.language();

        let resp = client
            .get_with(&url, lang, &cookies)
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

fn build_url(query: &str, pageno: u32) -> String {
    let mut args: Vec<(String, String)> = vec![
        ("q".into(), query.to_string()),
        ("source".into(), "web".into()),
    ];
    if pageno > 1 {
        args.push(("offset".into(), (pageno - 1).to_string()));
    }
    format!("{BASE_URL}/search?{}", urlencode_pairs(&args))
}

/// Cookies Brave uses to set the region, UI language and safe search.
/// Port of the `request()` cookie block in `brave.py`.
fn cookies_for(params: &EngineParams) -> Vec<(String, String)> {
    let ss = SAFESEARCH_COOKIE[(params.safesearch as usize).min(2)];
    let country = params.language().map(country_code).unwrap_or("us".to_string());
    vec![
        ("safesearch".to_string(), ss.to_string()),
        ("useLocation".to_string(), "0".to_string()),
        ("summarizer".to_string(), "0".to_string()),
        ("country".to_string(), country),
        ("ui_lang".to_string(), ui_lang(params.language()).to_string()),
    ]
}

/// `searxng_locale → Brave region`. The two-letter country part is used for
/// the `country` cookie; falls back to `us`.
fn country_code(lang: &str) -> String {
    lang.split_once('-')
        .map(|(_, cc)| cc.to_lowercase())
        .unwrap_or_else(|| "us".to_string())
}

/// `ui_lang` cookie from the known Brave UI languages; falls back to `en-us`.
fn ui_lang(lang: Option<&str>) -> &'static str {
    match lang {
        Some(l) => {
            let tag = l.to_lowercase();
            if UI_LANGS.contains(&tag.as_str()) {
                UI_LANGS[UI_LANGS.iter().position(|&x| x == tag).unwrap()]
            } else {
                DEFAULT_UI_LANG
            }
        }
        None => DEFAULT_UI_LANG,
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

    let result_sel = Selector::parse(r#"div[class*="snippet"]"#).unwrap();
    let title_sel = Selector::parse(r#"div[class*="title"]"#).unwrap();
    let content_sel = Selector::parse(r#"div[class~="content"]"#).unwrap();
    let thumb_sel = Selector::parse(r#"a[class*="thumbnail"] img[src]"#).unwrap();

    for result in doc.select(&result_sel) {
        // first anchor with a href under the snippet (as in `brave.py`)
        let Some(link) = result.select(&Selector::parse("a[href]").unwrap()).next() else {
            continue;
        };
        let Some(url) = attr(&link, "href") else {
            continue;
        };
        // partial URLs mean an ad
        if !has_netloc(&url) {
            continue;
        }
        let title = result
            .select(&title_sel)
            .next()
            .map(|e| text_of(&e))
            .unwrap_or_default();
        if title.is_empty() {
            continue;
        }

        let mut content = String::new();
        let mut published_date = None;
        if let Some(content_el) = result.select(&content_sel).next() {
            content = crate::engine::normalize_text(&text_of(&content_el));
            // Brave prints the publish date in a t-secondary span inside the
            // snippet; strip it from the content and keep it as a field.
            let pub_sel = Selector::parse(r#"span[class*="t-secondary"]"#).unwrap();
            if let Some(pub_el) = content_el.select(&pub_sel).next() {
                let pub_text = crate::engine::normalize_text(&text_of(&pub_el));
                if !pub_text.is_empty() {
                    content = strip_date_prefix(&content, &pub_text);
                    published_date = normalize_date(&pub_text);
                }
            }
        }

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
        res.published_date = published_date;
        out.add(res);
    }

    for suggestion in doc.select(&Selector::parse(r#"a[class*="related-query"]"#).unwrap()) {
        let text = text_of(&suggestion);
        if !text.is_empty() {
            out.suggestions.push(text);
        }
    }
    out
}

/// Require an absolute URL with a host; strips ads/partial links.
fn has_netloc(url: &str) -> bool {
    url::Url::parse(url)
        .map(|u| u.host_str().is_some())
        .unwrap_or(false)
}

/// `content.lstrip(pub_date).strip("- \n\t")` from `brave.py`.
fn strip_date_prefix(content: &str, pub_text: &str) -> String {
    content
        .strip_prefix(pub_text)
        .unwrap_or(content)
        .trim_matches(|c: char| c == ' ' || c == '-' || c == '\n' || c == '\t')
        .to_string()
}

/// Minimal date normalizer: recognize common date spellings and emit an ISO
/// `YYYY-MM-DD`. None means "not a date" (published_date stays empty).
fn normalize_date(s: &str) -> Option<String> {
    fn iso() -> &'static regex::Regex {
        static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        RE.get_or_init(|| regex::Regex::new(r"^(\d{4})-(\d{2})-(\d{2})").unwrap())
    }
    fn dmy() -> &'static regex::Regex {
        static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        RE.get_or_init(|| regex::Regex::new(r"^(\d{1,2})\.(\d{1,2})\.(\d{4})").unwrap())
    }
    fn mdy_words() -> &'static regex::Regex {
        static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        RE.get_or_init(|| {
            regex::Regex::new(r"(?i)^(jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)[a-z]*\.?\s+(\d{1,2}),?\s+(\d{4})")
                .unwrap()
        })
    }

    if let Some(c) = iso().captures(s) {
        let (y, m, d): (u32, u32, u32) = tuple3(&c, 1);
        return Some(format!("{y:04}-{m:02}-{d:02}"));
    }
    if let Some(c) = dmy().captures(s) {
        // capture order: day, month, year
        let (d, m, y): (u32, u32, u32) = tuple3(&c, 1);
        return Some(format!("{y:04}-{m:02}-{d:02}"));
    }
    if let Some(c) = mdy_words().captures(s) {
        // capture order: month word, day, year
        let day: u32 = c[2].parse().unwrap_or(1);
        let year: u32 = c[3].parse().unwrap_or(0);
        let month = match c[1].to_ascii_lowercase().as_str() {
            "jan" => 1, "feb" => 2, "mar" => 3, "apr" => 4, "may" => 5, "jun" => 6,
            "jul" => 7, "aug" => 8, "sep" => 9, "oct" => 10, "nov" => 11, "dec" => 12,
            _ => 1,
        };
        return Some(format!("{year:04}-{month:02}-{day:02}"));
    }
    None
}

fn tuple3(caps: &regex::Captures, start: usize) -> (u32, u32, u32) {
    (
        caps[start].parse().unwrap_or(0),
        caps[start + 1].parse().unwrap_or(0),
        caps[start + 2].parse().unwrap_or(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(query: &str, safesearch: u8, lang: Option<&str>) -> EngineParams {
        EngineParams {
            query: query.to_string(),
            languages: lang.map(|l| vec![l.to_string()]).unwrap_or_default(),
            safesearch,
            pageno: 1,
            category: "general".to_string(),
        }
    }

    #[test]
    fn builds_search_url() {
        let url = build_url("rust lang", 1);
        assert!(url.contains("q=rust%20lang"));
        assert!(url.contains("source=web"));
        assert!(!url.contains("offset"));
        assert!(build_url("r", 3).contains("offset=2"));
    }

    #[test]
    fn cookies_set_region_and_ui() {
        let c = cookies_for(&params("q", 0, Some("en-US")));
        let map: std::collections::HashMap<&str, &str> =
            c.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert_eq!(map["country"], "us");
        assert_eq!(map["ui_lang"], "en-us");
        assert_eq!(map["safesearch"], "off");
        assert_eq!(map["useLocation"], "0");
    }

    #[test]
    fn cookies_map_safesearch() {
        let c = cookies_for(&params("q", 2, None));
        let map: std::collections::HashMap<&str, &str> =
            c.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert_eq!(map["safesearch"], "strict");
        assert_eq!(map["country"], "us");
    }

    #[test]
    fn parses_fixture() {
        let html = r#"
<html><body>
<div class="container">
  <div class="snippet fdb">
    <div class="title"><a href="https://rust-lang.org">Rust Lang</a></div>
    <div class="content">
      <span class="t-secondary">Mar 3, 2024</span>A systems language
      <a class="thumbnail" href="https://search.brave.com/thumb"><img src="https://thumbs.brave.com/x"></a>
    </div>
  </div>
  <div class="snippet fdb">
    <div class="title"><a href="/ads/site">Sponsored ad</a></div>
    <div class="content">ad content</div>
  </div>
</div>
<a class="related-query">rust webassembly</a>
</body></html>"#;
        let res = parse_results(html, "brave");
        assert_eq!(res.results.len(), 1);
        assert_eq!(res.results[0].url, "https://rust-lang.org");
        assert_eq!(res.results[0].title, "Rust Lang");
        assert_eq!(res.results[0].content, "A systems language");
        assert_eq!(res.results[0].published_date.as_deref(), Some("2024-03-03"));
        assert_eq!(res.results[0].thumbnail.as_deref(), Some("https://thumbs.brave.com/x"));
        assert_eq!(res.suggestions, vec!["rust webassembly"]);
    }

    #[test]
    fn normalizes_dates() {
        assert_eq!(normalize_date("2024-03-03"), Some("2024-03-03".into()));
        assert_eq!(normalize_date("03.03.2024"), Some("2024-03-03".into()));
        assert_eq!(normalize_date("Mar 3, 2024"), Some("2024-03-03".into()));
        assert_eq!(normalize_date("not a date"), None);
    }
}