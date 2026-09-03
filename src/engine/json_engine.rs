//! Configurable JSON engine. Port of `searx/engines/json_engine.py` from
//! SearXNG. The config lists dot-paths into the response JSON.

use async_trait::async_trait;
use serde_json::Value;

use crate::config::CustomEngine;
use crate::engine::http::HttpClient;
use crate::engine::{Engine, EngineError, EngineParams, EngineResults, SearchResult};

pub struct JsonEngine {
    custom: CustomEngine,
}

impl JsonEngine {
    pub fn new(custom: CustomEngine) -> Self {
        Self { custom }
    }
}

#[async_trait]
impl Engine for JsonEngine {
    fn name(&self) -> &str {
        &self.custom.name
    }

    fn weight(&self) -> f32 {
        if self.custom.weight > 0.0 {
            self.custom.weight
        } else {
            1.0
        }
    }

    async fn search(
        &self,
        params: &EngineParams,
        client: &HttpClient,
    ) -> Result<EngineResults, EngineError> {
        let url = build_url(&self.custom.search_url, &params.query, params.pageno);

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

        self.parse(&body)
    }
}

impl JsonEngine {
    fn parse(&self, body: &str) -> Result<EngineResults, EngineError> {
        let json: Value =
            serde_json::from_str(body).map_err(|e| EngineError::Parse(e.to_string()))?;

        let mut out = EngineResults::default();

        // results_query selects the array of result objects.
        let results_array = match &self.custom.results_query {
            Some(path) => get_path(&json, path),
            None => Some(&json),
        };
        let Some(array) = results_array.and_then(|v| v.as_array()) else {
            return Ok(out);
        };

        for item in array {
            let mut result = SearchResult {
                engine: self.custom.name.clone(),
                ..Default::default()
            };
            if let Some(p) = &self.custom.url_query {
                result.url = get_path(item, p).and_then(Value::as_str).unwrap_or("").to_string();
            }
            if let Some(p) = &self.custom.title_query {
                result.title = get_path(item, p).and_then(Value::as_str).unwrap_or("").to_string();
            }
            if let Some(p) = &self.custom.content_query {
                result.content = get_path(item, p).and_then(Value::as_str).unwrap_or("").to_string();
            }
            if result.url.is_empty() && result.title.is_empty() {
                continue;
            }
            out.add(result);
        }

        if let Some(p) = &self.custom.suggestion_query {
            if let Some(arr) = get_path(&json, p).and_then(|v| v.as_array()) {
                for s in arr {
                    if let Some(s) = s.as_str() {
                        if !s.is_empty() {
                            out.suggestions.push(s.to_string());
                        }
                    }
                }
            }
        }
        Ok(out)
    }
}

/// Dot-path lookup into arbitrary JSON values (e.g. `data.results` or
/// `results.0.title`).
fn get_path<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = root;
    for part in path.split('.') {
        if let Ok(idx) = part.parse::<usize>() {
            cur = cur.get(idx)?;
        } else {
            cur = cur.get(part)?;
        }
    }
    Some(cur)
}

fn build_url(template: &str, query: &str, pageno: u32) -> String {
    let mut url = template.replace("{query}", &percent_encoding::utf8_percent_encode(query, percent_encoding::NON_ALPHANUMERIC).to_string());
    url = url.replace("{pageno}", &pageno.to_string());
    url
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CustomEngineKind;

    fn engine() -> JsonEngine {
        JsonEngine::new(CustomEngine {
            name: "crates".into(),
            enabled: true,
            weight: 1.0,
            categories: vec![],
            kind: CustomEngineKind::Json,
            search_url: "https://crates.io/api/v1/crates?q={query}".into(),
            results_xpath: None,
            url_xpath: None,
            title_xpath: None,
            content_xpath: None,
            suggestion_xpath: None,
            results_query: Some("crates".into()),
            url_query: Some("crate.name".into()),
            title_query: Some("crate.description".into()),
            content_query: None,
            suggestion_query: None,
        })
    }

    #[test]
    fn parses_nested_json() {
        let e = engine();
        let body = r#"{
            "crates": [
                {"crate": {"name": "serde", "description": "Serialization framework"}},
                {"crate": {"name": "tokio", "description": "Async runtime"}}
            ]
        }"#;
        let res = e.parse(body).unwrap();
        assert_eq!(res.results.len(), 2);
        assert_eq!(res.results[0].title, "Serialization framework");
        assert_eq!(res.results[0].url, "serde");
    }

    #[test]
    fn dotpath_lookup() {
        let v: Value = serde_json::json!({"a": {"b": [{"c": "x"}]}});
        assert_eq!(get_path(&v, "a.b.0.c").and_then(Value::as_str), Some("x"));
    }
}