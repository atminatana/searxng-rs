//! Yahoo WEB search. Port of `searx/engines/yahoo.py` from SearXNG.

use async_trait::async_trait;
use scraper::{Html, Selector};

use crate::engine::engines::{attr, text_of, ScrapeEngineBase};
use crate::engine::http::HttpClient;
use crate::engine::{Engine, EngineError, EngineParams, EngineResults, SearchResult};

const SAFESEARCH_MAP: [&str; 3] = ["p", "i", "r"];

fn region2domain(region: Option<&str>) -> Option<String> {
    match region {
        Some(r) => {
            let d = match r.to_ascii_uppercase().as_str() {
                "CO" => "co.search.yahoo.com",
                "TH" => "th.search.yahoo.com",
                "VE" => "ve.search.yahoo.com",
                "CL" => "cl.search.yahoo.com",
                "HK" => "hk.search.yahoo.com",
                "PE" => "pe.search.yahoo.com",
                "CA" => "ca.search.yahoo.com",
                "DE" => "de.search.yahoo.com",
                "FR" => "fr.search.yahoo.com",
                "TW" => "tw.search.yahoo.com",
                "GB" | "UK" => "uk.search.yahoo.com",
                "BR" => "br.search.yahoo.com",
                "IN" => "in.search.yahoo.com",
                "ES" => "espanol.search.yahoo.com",
                "PH" => "ph.search.yahoo.com",
                "AR" => "ar.search.yahoo.com",
                "MX" => "mx.search.yahoo.com",
                "SG" => "sg.search.yahoo.com",
                _ => return None,
            };
            Some(d.to_string())
        }
        None => None,
    }
}

fn lang_to_domain(lang: &str) -> String {
    match lang {
        "zh_chs" | "zh-CN" | "zh" => "hk.search.yahoo.com".to_string(),
        "zh_cht" | "zh-HK" | "zh-TW" => "tw.search.yahoo.com".to_string(),
        _ => "search.yahoo.com".to_string(),
    }
}

fn yahoo_language(lang: &str) -> String {
    match lang {
        "ar" => "ar", "bg" => "bg", "cs" => "cs", "da" => "da", "de" => "de",
        "el" => "el", "en" => "en", "es" => "es", "et" => "et", "fi" => "fi",
        "fr" => "fr", "he" => "he", "hr" => "hr", "hu" => "hu", "it" => "it",
        "ja" => "ja", "ko" => "ko", "lt" => "lt", "lv" => "lv", "nl" => "nl",
        "no" => "no", "pl" => "pl", "pt" => "pt", "ro" => "ro", "ru" => "ru",
        "sk" => "sk", "sl" => "sl", "sv" => "sv", "th" => "th", "tr" => "tr",
        "zh" | "zh-CN" | "zh_Hans" => "zh_chs",
        "zh-HK" | "zh-TW" | "zh_Hant" => "zh_cht",
        _ => "any",
    }
    .to_string()
}

pub struct YahooEngine {
    base: ScrapeEngineBase,
}

impl YahooEngine {
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
impl Engine for YahooEngine {
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
        let lang_s = params.language().unwrap_or("all").to_string();
        let (lang, region) = match lang_s.split_once('-') {
            Some((l, r)) => (l.to_string(), Some(r.to_string())),
            None => (lang_s.clone(), None),
        };
        let lang = yahoo_language(&lang);

        let mut url_params: Vec<(String, String)> = vec![("p".into(), params.query.clone())];
        if params.pageno == 1 {
            url_params.push(("iscqry".into(), String::new()));
        } else if params.pageno >= 2 {
            url_params.push(("b".into(), (params.pageno * 7 + 1).to_string()));
            url_params.push(("pz".into(), "7".into()));
            url_params.push(("bct".into(), "0".into()));
            url_params.push(("xargs".into(), "0".into()));
        }

        // sB cookie
        let sbcookie = format!(
            "v=1&vm={}&fl=1&vl=lang_{lang}&pn=10&rw=new&userset=1",
            SAFESEARCH_MAP[(params.safesearch as usize).min(2)]
        );

        let domain = region2domain(region.as_deref())
            .unwrap_or_else(|| lang_to_domain(&lang));
        let qs = urlencode_pairs(&url_params);
        let url = format!("https://{domain}/search?{qs}");

        let resp = client
            .get_with(&url, params.language(), &[("sB".to_string(), sbcookie)])
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

        Ok(parse_results(&body, &self.base.name, domain == "search.yahoo.com"))
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

/// Remove Yahoo's `/RU=` tracking prefix, returning the real destination URL.
fn parse_url(url: &str) -> String {
    let endings = ["/RS", "/RK"];
    let mut endpositions: Vec<usize> = Vec::new();
    let ru_pos = url.find("/RU=");
    let start = ru_pos
        .and_then(|r| url[r..].find("http").map(|o| r + o))
        .unwrap_or(0);

    for ending in endings {
        if let Some(endpos) = url.rfind(ending) {
            endpositions.push(endpos);
        }
    }
    if start == 0 || endpositions.is_empty() {
        return url.to_string();
    }
    let end = endpositions.into_iter().min().unwrap();
    percent_encoding::percent_decode_str(&url[start..end])
        .decode_utf8_lossy()
        .to_string()
}

fn parse_results(body: &str, engine_name: &str, root_domain: bool) -> EngineResults {
    let mut out = EngineResults::default();
    let doc = Html::parse_document(body);

    let result_sel = Selector::parse(r#"div[class*="algo-sr"]"#).unwrap();

    for result in doc.select(&result_sel) {
        let link: Option<scraper::ElementRef> = if root_domain {
            result
                .select(&Selector::parse(r#"div[class*="compTitle"] > a"#).unwrap())
                .next()
        } else {
            result
                .select(&Selector::parse(r#"div[class*="compTitle"] h3 > a"#).unwrap())
                .next()
        };
        let Some(link) = link else { continue };
        let Some(raw_url) = attr(&link, "href") else { continue };
        if raw_url.is_empty() {
            continue;
        }
        let url = parse_url(&raw_url);

        let title = if root_domain {
            link.select(&Selector::parse("h3 span").unwrap())
                .next()
                .map(|e| text_of(&e))
                .unwrap_or_default()
        } else {
            link.value().attr("aria-label").map(|s| s.to_string()).unwrap_or_default()
        };
        if title.trim().is_empty() {
            continue;
        }
        let content = result
            .select(&Selector::parse(r#"div[class*="compText"]"#).unwrap())
            .next()
            .map(|e| text_of(&e))
            .unwrap_or_default();

        out.add(SearchResult {
            url,
            title: crate::engine::normalize_text(&title),
            content: crate::engine::normalize_text(&content),
            engine: engine_name.to_string(),
            ..Default::default()
        });
    }

    for suggestion in doc.select(&Selector::parse(r#"div[class*="AlsoTry"] table a"#).unwrap()) {
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
    fn unescapes_tracking_url() {
        let raw = "https://r.search.yahoo.com/_ylt=.../RU=https%3A%2F%2Frust-lang.org%2Fdoc/RK=2/RS=abc";
        let parsed = parse_url(raw);
        assert_eq!(parsed, "https://rust-lang.org/doc");
    }

    #[test]
    fn leaves_plain_url() {
        assert_eq!(parse_url("https://example.com"), "https://example.com");
    }

    #[test]
    fn maps_language() {
        assert_eq!(yahoo_language("en"), "en");
        assert_eq!(yahoo_language("zh-CN"), "zh_chs");
        assert_eq!(yahoo_language("xx"), "any");
    }

    #[test]
    fn parses_fixture() {
        let html = r#"
<html><body>
<div class="algo-sr">
  <div class="compTitle">
    <h3><a href="https://r.yahoo.com/RU=https%3A%2F%2Frust-lang.org/RK=2" aria-label="Rust Lang">Rust Lang</a></h3>
  </div>
  <div class="compText"><p>A systems language</p></div>
</div>
<div class="AlsoTry"><table><tr><td><a>rust web</a></td></tr></table></div>
</body></html>"#;
        let res = parse_results(html, "yahoo", false);
        assert_eq!(res.results.len(), 1);
        assert_eq!(res.results[0].url, "https://rust-lang.org");
        assert_eq!(res.results[0].title, "Rust Lang");
        assert_eq!(res.suggestions, vec!["rust web"]);
    }
}
