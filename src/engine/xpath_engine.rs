//! Configurable XPath engine. Port of `searx/engines/xpath.py` from SearXNG,
//! powered by `sxd-document` + `sxd-xpath` so the original XPath selector
//! syntax works unchanged.

use async_trait::async_trait;
use sxd_document::parser;
use sxd_xpath::nodeset::Node;
use sxd_xpath::{Context, Factory, Value};

use crate::config::CustomEngine;
use crate::engine::http::HttpClient;
use crate::engine::{Engine, EngineError, EngineParams, EngineResults, SearchResult};

pub struct XpathEngine {
    custom: CustomEngine,
}

impl XpathEngine {
    pub fn new(custom: CustomEngine) -> Self {
        Self { custom }
    }

    fn eval<'a>(document: &sxd_document::dom::Document<'a>, xpath: &str) -> Option<Value<'a>> {
        let factory = Factory::new();
        let xp = factory.build(xpath).ok()??;
        let context = Context::new();
        xp.evaluate(&context, document.root()).ok()
    }

    fn eval_strings(document: &sxd_document::dom::Document, xpath: &str) -> Vec<String> {
        let Some(value) = Self::eval(document, xpath) else {
            return vec![];
        };
        match &value {
            Value::Nodeset(nodes) => nodes
                .document_order()
                .iter()
                .map(|n| n.string_value())
                .collect(),
            Value::String(s) => vec![s.clone()],
            other => vec![other.string()],
        }
    }
}

#[async_trait]
impl Engine for XpathEngine {
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

impl XpathEngine {
    fn parse(&self, body: &str) -> Result<EngineResults, EngineError> {
        let package = parser::parse(body).map_err(|e| EngineError::Parse(e.to_string()))?;
        let document = package.as_document();

        let mut out = EngineResults::default();

        let results_xpath = self
            .custom
            .results_xpath
            .clone()
            .ok_or_else(|| EngineError::Parse("results_xpath is required".into()))?;

        let factory = Factory::new();
        let xp = factory
            .build(&results_xpath)
            .map_err(|e| EngineError::Parse(format!("bad results_xpath: {e}")))?
            .ok_or_else(|| EngineError::Parse("empty results_xpath".into()))?;
        let context = Context::new();
        let value = xp
            .evaluate(&context, document.root())
            .map_err(|e| EngineError::Parse(format!("xpath eval failed: {e}")))?;

        let nodes: Vec<Node> = match value {
            Value::Nodeset(set) => set.document_order(),
            _ => return Ok(out),
        };

        for node in nodes {
            let mut result = SearchResult {
                engine: self.custom.name.clone(),
                ..Default::default()
            };

            // url
            if let Some(xp) = &self.custom.url_xpath {
                result.url = self.eval_first(node, xp).unwrap_or_default();
            }
            if let Some(xp) = &self.custom.title_xpath {
                result.title = self.eval_first(node, xp).unwrap_or_default();
            }
            if let Some(xp) = &self.custom.content_xpath {
                result.content = self.eval_first(node, xp).unwrap_or_default();
            }

            if result.url.is_empty() && result.title.is_empty() {
                continue;
            }
            if let Some(prefix) = self.custom.search_url.split('?').next() {
                if !result.url.starts_with("http") {
                    if let Ok(abs) = url::Url::parse(prefix).and_then(|b| b.join(&result.url)) {
                        result.url = abs.to_string();
                    }
                }
            }
            out.add(result);
        }

        if let Some(xp) = &self.custom.suggestion_xpath {
            for s in Self::eval_strings(&document, xp) {
                if !s.is_empty() {
                    out.suggestions.push(s);
                }
            }
        }
        Ok(out)
    }

    fn eval_first(&self, node: Node, xpath: &str) -> Option<String> {
        let factory = Factory::new();
        let xp = factory.build(xpath).ok()??;
        let context = Context::new();
        let value = xp.evaluate(&context, node).ok()?;
        match value {
            Value::Nodeset(set) => set.document_order_first().map(|n| n.string_value()).filter(|s| !s.is_empty()),
            Value::String(s) if !s.is_empty() => Some(s),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    }
}

fn build_url(template: &str, query: &str, pageno: u32) -> String {
    let mut url = template.replace("{query}", &percent_encoding::utf8_percent_encode(query, percent_encoding::NON_ALPHANUMERIC).to_string());
    url = url.replace("{pageno}", &pageno.to_string());
    url
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CustomEngine;
    use crate::config::CustomEngineKind;

    fn engine(results_xpath: &str) -> XpathEngine {
        XpathEngine::new(CustomEngine {
            name: "test".into(),
            enabled: true,
            weight: 1.0,
            categories: vec![],
            kind: CustomEngineKind::Xpath,
            search_url: "https://example.com/?q={query}".into(),
            results_xpath: Some(results_xpath.into()),
            url_xpath: Some(".//a/@href".into()),
            title_xpath: Some(".//a".into()),
            content_xpath: Some(".//p".into()),
            suggestion_xpath: None,
            results_query: None,
            url_query: None,
            title_query: None,
            content_query: None,
            suggestion_query: None,
        })
    }

    #[test]
    fn parses_html_with_xpath() {
        let html = r#"
<html><body>
<div class="result">
  <h3><a href="/cool">Cool Page</a></h3>
  <p>Description here</p>
</div>
<div class="result">
  <h3><a href="/other">Other Page</a></h3>
  <p>Other description</p>
</div>
</body></html>"#;
        let e = engine("//div[contains(@class, 'result')]");
        let res = e.parse(html).unwrap();
        assert_eq!(res.results.len(), 2);
        assert_eq!(res.results[0].url, "https://example.com/cool");
        assert_eq!(res.results[0].title, "Cool Page");
        assert_eq!(res.results[0].content, "Description here");
    }

    #[test]
    fn builds_query_url() {
        let u = build_url("https://e.com/q?q={query}&p={pageno}", "rust lang", 2);
        assert!(u.contains("q=rust%20lang"));
        assert!(u.contains("p=2"));
    }
}