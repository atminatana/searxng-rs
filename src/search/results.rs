//! Thread-safe collection, dedup and scoring of results. Port of
//! `searx/results.py::ResultContainer` from SearXNG.

use std::collections::HashSet;

use crate::engine::SearchResult;

/// Result container that merges results from multiple engines.
#[derive(Debug, Default)]
pub struct ResultContainer {
    pub results: Vec<SearchResult>,
    pub suggestions: Vec<String>,
    pub corrections: Vec<String>,
    pub answers: Vec<String>,
    seen: HashSet<String>,
}

/// Compute the dedup key, mirroring SearXNG's `MainResult.__hash__`.
/// Ordinary URL results are equal when (scheme-less) host, path, params,
/// query and fragment match.
fn dedup_key(r: &SearchResult) -> Option<String> {
    if r.url.is_empty() {
        return None;
    }
    let parsed = url::Url::parse(&r.url).ok()?;
    // Drop scheme & leading "www." to merge http/https duplicates.
    let mut host = parsed.host_str().unwrap_or("").to_string();
    if host.starts_with("www.") {
        host = host[4..].to_string();
    }
    Some(format!(
        "{host}|{}|{}|{}|{}",
        parsed.path(),
        parsed.query().unwrap_or(""),
        parsed.fragment().unwrap_or(""),
        r.img_src.as_deref().unwrap_or("")
    ))
}

impl ResultContainer {
    pub fn add(&mut self, mut r: SearchResult) {
        // Every result must carry the engine it came from.
        if !r.engines.contains(&r.engine) {
            r.engines.push(r.engine.clone());
        }
        // If a result with the same url exists already, merge engine refs.
        if let Some(key) = dedup_key(&r) {
            if let Some(existing) = self.results.iter_mut().find(|e| dedup_key(e) == Some(key.clone())) {
                if !existing.engines.contains(&r.engine) {
                    existing.engines.push(r.engine.clone());
                }
                if existing.title.is_empty() {
                    existing.title = r.title.clone();
                }
                if existing.content.is_empty() {
                    existing.content = r.content.clone();
                }
                if r.title.is_empty() {
                    r.title = existing.title.clone();
                }
                existing.score = existing.score.max(r.score);
                return;
            }
            self.seen.insert(key);
        }
        self.results.push(r);
    }

    pub fn extend(&mut self, iter: impl IntoIterator<Item = SearchResult>) {
        for r in iter {
            self.add(r);
        }
    }

    /// Sort results by score descending; scores were set by `merge_outcomes`.
    pub fn get_ordered_results(&mut self) -> &[SearchResult] {
        self.results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        &self.results
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(url: &str, engine: &str, score: f32) -> SearchResult {
        SearchResult {
            url: url.to_string(),
            title: String::new(),
            content: String::new(),
            engine: engine.to_string(),
            score,
            ..Default::default()
        }
    }

    #[test]
    fn dedups_by_scheme_less_url() {
        let mut c = ResultContainer::default();
        c.add(result("https://example.com/a?x=1", "google", 10.0));
        c.add(result("http://www.example.com/a?x=1", "bing", 5.0));
        assert_eq!(c.results.len(), 1);
        assert_eq!(c.results[0].engines, vec!["google".to_string(), "bing".to_string()]);
        assert_eq!(c.results[0].score, 10.0);
    }

    #[test]
    fn keeps_distinct_urls() {
        let mut c = ResultContainer::default();
        c.add(result("https://example.com/a", "google", 1.0));
        c.add(result("https://example.com/b", "bing", 2.0));
        assert_eq!(c.results.len(), 2);
    }

    #[test]
    fn orders_by_score() {
        let mut c = ResultContainer::default();
        c.add(result("https://e.com/low", "bing", 2.0));
        c.add(result("https://e.com/high", "google", 10.0));
        c.get_ordered_results();
        assert_eq!(c.results[0].url, "https://e.com/high");
    }
}