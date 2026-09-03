//! Search query models. Port of `searx/search/models.py` from SearXNG.

use crate::query::EngineRef;

/// Immutable parameters of a single search (port of `SearchQuery`).
#[derive(Debug, Clone, Default)]
pub struct SearchQuery {
    pub query: String,
    pub enginerefs: Vec<EngineRef>,
    pub languages: Vec<String>,
    pub safe_search: u8,
    pub pageno: u32,
    pub timeout_limit: Option<f32>,
    pub external_bang: Option<String>,
    pub redirect_to_first_result: bool,
    pub disabled_engines: Vec<(String, String)>,
}

impl SearchQuery {
    pub fn simple(query: impl Into<String>, safe_search: u8) -> Self {
        Self {
            query: query.into(),
            safe_search,
            pageno: 1,
            ..Default::default()
        }
    }

    pub fn with_engines(mut self, refs: Vec<EngineRef>) -> Self {
        self.enginerefs = refs;
        self
    }

    pub fn with_languages(mut self, languages: Vec<String>) -> Self {
        self.languages = languages;
        self
    }
}

/// Underscore alias to satisfy `RawTextQuery` integration.
#[allow(unused)]
pub type EngineRefList = Vec<EngineRef>;