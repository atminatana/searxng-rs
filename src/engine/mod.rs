//! Engine framework: the `Engine` trait, typed results, errors, and the
//! engine registry. Port of `searx/enginelib` + `searx/result_types` from
//! SearXNG.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::Serialize;

use crate::config::{Config, CustomEngine, EngineTuning};
use crate::engine::http::HttpClient;

pub mod engines;
pub mod http;
pub mod json_engine;
pub mod xpath_engine;

/// A single search result as produced by an engine. Mirrors SearXNG's
/// `MainResult` (title, content, url, engines, score, positions, ...).
#[derive(Debug, Clone, Default, Serialize)]
pub struct SearchResult {
    pub url: String,
    pub title: String,
    pub content: String,
    pub engine: String,
    #[serde(default)]
    pub engines: Vec<String>,
    #[serde(default)]
    pub positions: Vec<u32>,
    pub score: f32,
    pub category: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub img_src: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_date: Option<String>,
}

impl SearchResult {
    pub fn empty(engine: &str) -> Self {
        Self {
            engine: engine.to_string(),
            ..Default::default()
        }
    }
}

/// Everything an engine returns besides plain results.
#[derive(Debug, Clone, Default)]
pub struct EngineResults {
    pub results: Vec<SearchResult>,
    pub suggestions: Vec<String>,
    pub corrections: Vec<String>,
    pub answers: Vec<String>,
}

impl EngineResults {
    pub fn add(&mut self, r: SearchResult) {
        self.results.push(r);
    }
    pub fn push(&mut self, r: SearchResult) {
        self.results.push(r);
    }
}

/// Errors an engine can raise. Port of `searx/exceptions.py`.
#[derive(Debug, Clone, thiserror::Error)]
pub enum EngineError {
    #[error("HTTP error {0}")]
    Http(u16),
    #[error("CAPTCHA required")]
    Captcha,
    #[error("Cloudflare CAPTCHA required")]
    CloudflareCaptcha,
    #[error("Access denied")]
    AccessDenied,
    #[error("Too many requests")]
    TooManyRequests,
    #[error("engine returned no results")]
    NoResults,
    #[error("engine suspended")]
    Suspended,
    #[error("request timeout")]
    Timeout,
    #[error("request error: {0}")]
    Request(String),
    #[error("parse error: {0}")]
    Parse(String),
}

impl EngineError {
    /// Match a HTTP status code to a typed error (like SearXNG's
    /// `raise_for_httperror`).
    pub fn from_status(status: u16) -> Option<EngineError> {
        match status {
            402 | 403 => Some(EngineError::AccessDenied),
            429 => Some(EngineError::TooManyRequests),
            401 => Some(EngineError::Http(401)),
            200..=399 => None,
            404 => Some(EngineError::NoResults),
            500..=599 => Some(EngineError::Http(status)),
            _ => Some(EngineError::Http(status)),
        }
    }
}

/// Parameters passed to every engine search. Built from a `SearchQuery`.
#[derive(Debug, Clone)]
pub struct EngineParams {
    pub query: String,
    pub languages: Vec<String>,
    pub safesearch: u8,
    pub pageno: u32,
    pub category: String,
}

impl EngineParams {
    pub fn language(&self) -> Option<&str> {
        self.languages.first().map(String::as_str)
    }
}

/// How one engine is parametrised in the config.
#[derive(Debug, Clone)]
pub struct EngineSpec {
    pub name: String,
    pub enabled: bool,
    pub weight: f32,
    pub timeout: Option<f32>,
    pub categories: Vec<String>,
    pub custom: Option<CustomEngine>,
}

impl EngineSpec {
    pub fn from_config(cfg: &Config) -> Vec<EngineSpec> {
        let mut specs = Vec::new();

        for (name, tuning) in &cfg.engines.named {
            let (weight, timeout) = real_tuning(tuning);
            let categories = engine_categories(name);
            specs.push(EngineSpec {
                name: name.clone(),
                enabled: tuning.enabled,
                weight,
                timeout,
                categories,
                custom: None,
            });
        }

        for custom in &cfg.engines.custom {
            specs.push(EngineSpec {
                name: custom.name.clone(),
                enabled: custom.enabled,
                weight: if custom.weight > 0.0 { custom.weight } else { 1.0 },
                timeout: None,
                categories: if custom.categories.is_empty() {
                    vec!["general".to_string()]
                } else {
                    custom.categories.clone()
                },
                custom: Some(custom.clone()),
            });
        }
        specs
    }
}

fn real_tuning(t: &EngineTuning) -> (f32, Option<f32>) {
    let weight = if t.weight > 0.0 { t.weight } else { 1.0 };
    (weight, t.timeout)
}

fn engine_categories(name: &str) -> Vec<String> {
    // Default categories for built-in engines. Custom engines declare their
    // own. Kept small — mirrors the `categories` attribute of each engine
    // module in SearXNG.
    let cat = match name {
        "google" | "bing" | "brave" | "yandex" | "duckduckgo" | "startpage" | "qwant"
        | "yahoo" => "general",
        "baidu" | "naver" => "general",
        "wikipedia" => "general",
        "wikcommons" => "images",
        "google_news" | "bing_news" => "news",
        "google_images" | "bing_images" => "images",
        "github" | "gitlab" => "it",
        "npm" | "pypi" | "docker_hub" | "crates" => "it",
        "hackernews" => "it",
        _ => "general",
    };
    vec![cat.to_string()]
}

/// A concrete, runnable engine.
#[async_trait]
pub trait Engine: Send + Sync {
    fn name(&self) -> &str;
    fn weight(&self) -> f32 {
        1.0
    }
    fn timeout(&self) -> Option<f32> {
        None
    }
    async fn search(
        &self,
        params: &EngineParams,
        client: &HttpClient,
    ) -> Result<EngineResults, EngineError>;
}

/// Registry of all loaded engines, pre-built from the `Config`.
pub struct EngineRegistry {
    engines: HashMap<String, Arc<dyn Engine>>,
    /// engine name → engine shortcuts (`!g` → google).
    pub shortcuts: HashMap<String, String>,
    /// category → engine names.
    pub categories: HashMap<String, Vec<String>>,
    /// Specs in load order (enabled + disabled).
    pub specs: Vec<EngineSpec>,
}

impl EngineRegistry {
    /// Empty registry for tests.
    #[cfg(test)]
    pub(crate) fn empty() -> Self {
        Self {
            engines: HashMap::new(),
            shortcuts: HashMap::new(),
            categories: HashMap::new(),
            specs: vec![],
        }
    }

    pub fn from_config(cfg: &Config, _client: &HttpClient) -> Self {
        let specs = EngineSpec::from_config(cfg);
        let mut engines: HashMap<String, Arc<dyn Engine>> = HashMap::new();
        let mut categories: HashMap<String, Vec<String>> = HashMap::new();
        let shortcuts = default_shortcuts();

        for spec in &specs {
            let engine: Option<Arc<dyn Engine>> = if let Some(custom) = &spec.custom {
                match custom.kind {
                    crate::config::CustomEngineKind::Xpath => {
                        Some(Arc::new(xpath_engine::XpathEngine::new(custom.clone())))
                    }
                    crate::config::CustomEngineKind::Json => {
                        Some(Arc::new(json_engine::JsonEngine::new(custom.clone())))
                    }
                }
            } else {
                engines::builtin(spec)
            };
            if let Some(engine) = engine {
                for cat in &spec.categories {
                    categories
                        .entry(cat.clone())
                        .or_default()
                        .push(spec.name.clone());
                }
                engines.insert(spec.name.clone(), engine);
            }
        }

        Self {
            engines,
            shortcuts,
            categories,
            specs,
        }
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Engine>> {
        self.engines.get(name).cloned()
    }

    pub fn names(&self) -> Vec<String> {
        self.engines.keys().cloned().collect()
    }

    pub fn is_loaded(&self, name: &str) -> bool {
        self.engines.contains_key(name)
    }

    pub fn categories_for(&self, name: &str) -> Vec<String> {
        self.specs
            .iter()
            .find(|s| s.name == name)
            .map(|s| s.categories.clone())
            .unwrap_or_default()
    }
}

fn default_shortcuts() -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert("g".to_string(), "google".to_string());
    m.insert("gg".to_string(), "google".to_string());
    m.insert("b".to_string(), "bing".to_string());
    m.insert("ddg".to_string(), "duckduckgo".to_string());
    m.insert("brave".to_string(), "brave".to_string());
    m.insert("wp".to_string(), "wikipedia".to_string());
    m.insert("wt".to_string(), "wikipedia".to_string());
    m.insert("ya".to_string(), "yandex".to_string());
    m.insert("yh".to_string(), "yahoo".to_string());
    m.insert("gh".to_string(), "github".to_string());
    m.insert("gl".to_string(), "gitlab".to_string());
    m.insert("hn".to_string(), "hackernews".to_string());
    m.insert("sp".to_string(), "startpage".to_string());
    m.insert("dh".to_string(), "docker_hub".to_string());
    m
}

/// Normalize whitespace like SearXNG's `WHITESPACE_REGEX` (collapse runs of
/// whitespace to a single space and trim).
pub fn normalize_text(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_whitespace() {
        assert_eq!(normalize_text("  a   b\nc\t "), "a b c");
    }
}