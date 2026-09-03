//! Raw text query parsing: `!bang` (engine/category), `!!` (feeling lucky),
//! `!!bang` (external bang), `:lang`, `<timeout`.
//!
//! Port of `searx/query.py` from SearXNG.

use serde::Serialize;

/// Reference to an engine, optionally restricted to a category.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct EngineRef {
    pub name: String,
    pub category: String,
}

/// A known language code the user can force with `:lang`. Simplified subset of
/// SearXNG's `sxng_locales` plus a generic ISO-639-1 acceptance fallback.
pub const KNOWN_LANGUAGE_CODES: &[&str] = &[
    "auto",
    "en",
    "en-US",
    "en-GB",
    "en-CA",
    "en-AU",
    "zh",
    "zh-CN",
    "zh-TW",
    "zh-Hans",
    "zh-Hant",
    "ho",
    "es",
    "es-ES",
    "es-MX",
    "es-AR",
    "fr",
    "fr-FR",
    "fr-CA",
    "fr-BE",
    "de",
    "de-DE",
    "de-AT",
    "de-CH",
    "it",
    "it-IT",
    "pt",
    "pt-BR",
    "pt-PT",
    "ru",
    "ru-RU",
    "ja",
    "ja-JP",
    "ko",
    "ko-KR",
    "nl",
    "nl-NL",
    "pl",
    "sv",
    "sv-SE",
    "no",
    "da",
    "fi",
    "cs",
    "hu",
    "ro",
    "uk",
    "tr",
    "ar",
    "he",
    "hi",
    "id",
    "ms",
    "vi",
    "th",
    "el",
    "bg",
    "hr",
    "sk",
    "sr",
];

pub fn is_valid_language_code(value: &str) -> bool {
    if value == "auto" {
        return true;
    }
    // Accept any well-formed xx or xx-YY tag.
    let parts: Vec<&str> = value.split('-').collect();
    if parts.len() > 2 {
        return false;
    }
    let primary = parts[0];
    primary.len() == 2 && primary.chars().all(|c| c.is_ascii_alphabetic())
        && parts[1..].iter().all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_alphabetic()))
}

/// Trait for the individual query-part parsers (port of `QueryPartParser`).
trait QueryPartParser {
    fn check(raw_value: &str) -> bool;
    fn parse(&mut self, raw_value: &str, rq: &mut RawTextQuery) -> bool;
}

struct TimeoutParser;
struct LanguageParser;
struct ExternalBangParser;
struct BangParser;
struct FeelingLuckyParser;

impl QueryPartParser for TimeoutParser {
    fn check(raw_value: &str) -> bool {
        raw_value.starts_with('<')
    }

    fn parse(&mut self, raw_value: &str, rq: &mut RawTextQuery) -> bool {
        let value = &raw_value[1..];
        if value.is_empty() {
            return false;
        }
        let Some(parsed) = value.parse::<u32>().ok() else {
            return false;
        };
        if parsed < 100 {
            // below 100, the unit is the second (<3 = 3 seconds timeout)
            rq.timeout_limit = Some(parsed as f32);
        } else {
            // 100 or above, the unit is the millisecond (<850 = 850ms)
            rq.timeout_limit = Some(parsed as f32 / 1000.0);
        }
        true
    }
}

impl QueryPartParser for LanguageParser {
    fn check(raw_value: &str) -> bool {
        raw_value.starts_with(':')
    }

    fn parse(&mut self, raw_value: &str, rq: &mut RawTextQuery) -> bool {
        let value = raw_value[1..].to_lowercase().replace('_', "-");
        if value.is_empty() {
            return false;
        }
        // Match against known codes first (case-insensitive, with country
        // name fallback for the small known set).
        for lc in KNOWN_LANGUAGE_CODES {
            let lc_lower = lc.to_lowercase();
            if value == lc_lower || lc_lower.replace('-', " ") == value.replace('-', " ") {
                if !rq.languages.contains(&lc.to_string()) {
                    rq.languages.push(lc.to_string());
                }
                return true;
            }
        }
        // Generic well-formed tag fallback.
        if is_valid_language_code(&value) {
            let normalized = normalize_language_tag(&value);
            if !rq.languages.contains(&normalized) {
                rq.languages.push(normalized);
            }
            return true;
        }
        false
    }
}

fn normalize_language_tag(value: &str) -> String {
    let parts: Vec<&str> = value.split('-').collect();
    if parts.len() > 1 {
        format!(
            "{}-{}",
            parts[0].to_lowercase(),
            parts[1..].join("-").to_uppercase()
        )
    } else {
        parts[0].to_string()
    }
}

/// A tiny built-in external bang table (`!!g` → Google). Users can extend this
/// via config later; the `!!` alone (feeling lucky) is handled separately.
pub const EXTERNAL_BANGS: &[(&str, &str)] = &[
    ("g", "https://www.google.com/search?q={query}"),
    ("ddg", "https://duckduckgo.com/?q={query}"),
    ("bing", "https://www.bing.com/search?q={query}"),
    ("yt", "https://www.youtube.com/results?search_query={query}"),
    ("wt", "https://en.wikipedia.org/wiki/Special:Search?search={query}"),
    ("gh", "https://github.com/search?q={query}"),
    ("so", "https://stackoverflow.com/search?q={query}"),
    ("npm", "https://www.npmjs.com/search?q={query}"),
    ("crates", "https://crates.io/search?q={query}"),
    ("rstdocs", "https://docs.rs/releases/search?query={query}"),
];

impl QueryPartParser for ExternalBangParser {
    fn check(raw_value: &str) -> bool {
        raw_value.starts_with("!!") && raw_value.len() > 2
    }

    fn parse(&mut self, raw_value: &str, rq: &mut RawTextQuery) -> bool {
        let value = raw_value[2..].to_lowercase();
        if value.is_empty() {
            return false;
        }
        for (name, _) in EXTERNAL_BANGS {
            if *name == value {
                rq.external_bang = Some(value);
                return true;
            }
        }
        false
    }
}

impl QueryPartParser for BangParser {
    fn check(raw_value: &str) -> bool {
        raw_value.starts_with('!') && (raw_value.chars().nth(1) != Some('!'))
    }

    fn parse(&mut self, raw_value: &str, rq: &mut RawTextQuery) -> bool {
        let value = raw_value[1..]
            .replace(['-', '_'], " ")
            .to_lowercase();
        if value.is_empty() {
            return false;
        }
        // engine shortcut ?
        if let Some(engine) = rq.engine_shortcuts.get(&value) {
            let engine = engine.clone();
            rq.enginerefs.push(EngineRef {
                name: engine,
                category: "none".to_string(),
            });
            rq.specific = true;
            return true;
        }
        // engine name ?
        if rq.available_engines.contains(&value) {
            rq.enginerefs.push(EngineRef {
                name: value,
                category: "none".to_string(),
            });
            rq.specific = true;
            return true;
        }
        // category ?
        if let Some(engines) = rq.categories.get(&value) {
            for engine in engines {
                if !rq.disabled_engines.contains(&(engine.clone(), value.clone())) {
                    rq.enginerefs.push(EngineRef {
                        name: engine.clone(),
                        category: value.clone(),
                    });
                }
            }
            rq.specific = true;
            return true;
        }
        false
    }
}

impl QueryPartParser for FeelingLuckyParser {
    fn check(raw_value: &str) -> bool {
        raw_value == "!!"
    }

    fn parse(&mut self, _raw_value: &str, rq: &mut RawTextQuery) -> bool {
        rq.redirect_to_first_result = true;
        true
    }
}

/// A parsed raw text query (port of `RawTextQuery`).
#[derive(Debug, Clone, Default)]
pub struct RawTextQuery {
    pub query: String,
    pub disabled_engines: Vec<(String, String)>,
    /// Engines that may be referenced by `!bang`.
    pub available_engines: Vec<String>,
    /// engine name → engine name (shortcut map).
    pub engine_shortcuts: std::collections::HashMap<String, String>,
    /// category name → engine names.
    pub categories: std::collections::HashMap<String, Vec<String>>,

    pub enginerefs: Vec<EngineRef>,
    pub languages: Vec<String>,
    pub timeout_limit: Option<f32>,
    pub external_bang: Option<String>,
    pub specific: bool,
    pub redirect_to_first_result: bool,

    query_parts: Vec<String>,
    user_query_parts: Vec<String>,
}

impl RawTextQuery {
    pub fn new(
        query: &str,
        disabled_engines: &[(String, String)],
        available_engines: &[String],
        engine_shortcuts: &std::collections::HashMap<String, String>,
        categories: &std::collections::HashMap<String, Vec<String>>,
    ) -> Self {
        let mut rq = RawTextQuery {
            query: query.to_string(),
            disabled_engines: disabled_engines.to_vec(),
            available_engines: available_engines.to_vec(),
            engine_shortcuts: engine_shortcuts.clone(),
            categories: categories.clone(),
            ..Default::default()
        };
        rq._parse_query();
        rq
    }

    fn _parse_query(&mut self) {
        let tokens: Vec<String> = self.query.split_whitespace().map(|s| s.to_string()).collect();
        let last_index = tokens.len().saturating_sub(1);

        for (i, part) in tokens.iter().enumerate() {
            let mut special_part = false;

            if TimeoutParser::check(part) {
                special_part = TimeoutParser.parse(part, self);
            } else if LanguageParser::check(part) {
                special_part = LanguageParser.parse(part, self);
            } else if ExternalBangParser::check(part) {
                special_part = ExternalBangParser.parse(part, self);
            } else if BangParser::check(part) {
                special_part = BangParser.parse(part, self);
            } else if FeelingLuckyParser::check(part) {
                special_part = FeelingLuckyParser.parse(part, self);
            }

            let _ = i == last_index; // autocomplete position (unused for CLI/MCP)
            let list = if special_part {
                &mut self.query_parts
            } else {
                &mut self.user_query_parts
            };
            list.push(part.clone());
        }
    }

    /// The pure user query without the special `!`, `:`, `<` parts.
    pub fn get_query(&self) -> String {
        self.user_query_parts.join(" ")
    }

    /// Full query including special parts (port of `getFullQuery`).
    pub fn get_full_query(&self) -> String {
        let mut parts = self.query_parts.clone();
        parts.push(self.get_query());
        parts.retain(|s| !s.is_empty());
        parts.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn test_rq(query: &str) -> RawTextQuery {
        let shortcuts: HashMap<String, String> =
            [("g".to_string(), "google".to_string())].into_iter().collect();
        let cats: HashMap<String, Vec<String>> =
            [("general".to_string(), vec!["google".to_string(), "bing".to_string()])]
                .into_iter()
                .collect();
        RawTextQuery::new(
            query,
            &[],
            &["google".to_string(), "bing".to_string(), "duckduckgo".to_string()],
            &shortcuts,
            &cats,
        )
    }

    #[test]
    fn parses_plain_query() {
        let rq = test_rq("hello world");
        assert_eq!(rq.get_query(), "hello world");
        assert!(rq.enginerefs.is_empty());
        assert!(rq.languages.is_empty());
        assert!(rq.timeout_limit.is_none());
    }

    #[test]
    fn parses_engine_bang() {
        let rq = test_rq("!google rust");
        assert_eq!(rq.enginerefs.len(), 1);
        assert_eq!(rq.enginerefs[0].name, "google");
        assert!(rq.specific);
        assert_eq!(rq.get_query(), "rust");
    }

    #[test]
    fn parses_engine_shortcut() {
        let rq = test_rq("!g rust");
        assert_eq!(rq.enginerefs[0].name, "google");
    }

    #[test]
    fn parses_category_bang() {
        let rq = test_rq("!general rust");
        assert_eq!(rq.enginerefs.len(), 2);
    }

    #[test]
    fn parses_language() {
        let rq = test_rq(":fr bonjour");
        assert_eq!(rq.languages, vec!["fr"]);
        let rq = test_rq(":en_us hello");
        assert_eq!(rq.languages, vec!["en-US"]);
    }

    #[test]
    fn parses_timeout_seconds() {
        let rq = test_rq("<3 test");
        assert_eq!(rq.timeout_limit, Some(3.0));
    }

    #[test]
    fn parses_timeout_millis() {
        let rq = test_rq("<850 test");
        assert_eq!(rq.timeout_limit, Some(0.85));
    }

    #[test]
    fn parses_feeling_lucky() {
        let rq = test_rq("!! hello");
        assert!(rq.redirect_to_first_result);
    }

    #[test]
    fn parses_external_bang() {
        let rq = test_rq("!!ddg hello");
        assert_eq!(rq.external_bang.as_deref(), Some("ddg"));
    }

    #[test]
    fn full_query() {
        let rq = test_rq("!google :fr hello world");
        assert!(rq.get_full_query().contains("hello world"));
        assert!(rq.get_full_query().contains("google"));
        assert_eq!(rq.get_query(), "hello world");
    }
}