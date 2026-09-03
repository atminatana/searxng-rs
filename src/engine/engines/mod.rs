//! Built-in search engines, each a `Engine` implementation.
//!
//! Port directory: `searx/engines/` from SearXNG.

use std::sync::Arc;

use super::EngineSpec;

pub mod baidu;
pub mod bing;
pub mod bing_images;
pub mod bing_news;
pub mod brave;
pub mod crates;
pub mod docker_hub;
pub mod duckduckgo;
pub mod gitlab;
pub mod github;
pub mod google;
pub mod google_images;
pub mod google_news;
pub mod hackernews;
pub mod naver;
pub mod npm;
pub mod pypi;
pub mod startpage;
pub mod wikipedia;
pub mod yahoo;
pub mod yandex;

/// Build a built-in engine for `spec`, or `None` if the name is unknown.
pub fn builtin(spec: &EngineSpec) -> Option<Arc<dyn crate::engine::Engine>> {
    let engine: Arc<dyn crate::engine::Engine> = match spec.name.as_str() {
        "google" => Arc::new(google::GoogleEngine::new(spec)),
        "google_news" => Arc::new(google_news::GoogleNewsEngine::new(spec)),
        "google_images" => Arc::new(google_images::GoogleImagesEngine::new(spec)),
        "bing" => Arc::new(bing::BingEngine::new(spec)),
        "bing_news" => Arc::new(bing_news::BingNewsEngine::new(spec)),
        "bing_images" => Arc::new(bing_images::BingImagesEngine::new(spec)),
        "duckduckgo" => Arc::new(duckduckgo::DuckDuckGoEngine::new(spec)),
        "wikipedia" => Arc::new(wikipedia::WikipediaEngine::new(spec)),
        "brave" => Arc::new(brave::BraveEngine::new(spec)),
        "yandex" => Arc::new(yandex::YandexEngine::new(spec)),
        "yahoo" => Arc::new(yahoo::YahooEngine::new(spec)),
        "baidu" => Arc::new(baidu::BaiduEngine::new(spec)),
        "naver" => Arc::new(naver::NaverEngine::new(spec)),
        "startpage" => Arc::new(startpage::StartpageEngine::new(spec)),
        "github" => Arc::new(github::GithubEngine::new(spec)),
        "gitlab" => Arc::new(gitlab::GitlabEngine::new(spec)),
        "npm" => Arc::new(npm::NpmEngine::new(spec)),
        "pypi" => Arc::new(pypi::PypiEngine::new(spec)),
        "docker_hub" => Arc::new(docker_hub::DockerHubEngine::new(spec)),
        "crates" => Arc::new(crates::CratesEngine::new(spec)),
        "hackernews" => Arc::new(hackernews::HackernewsEngine::new(spec)),
        _ => return None,
    };
    Some(engine)
}

/// Common data for all built-in scraped engines.
pub struct ScrapeEngineBase {
    pub name: String,
    pub weight: f32,
    pub timeout: Option<f32>,
}

/// Join all descendant text nodes of an element with single spaces.
pub(crate) fn text_of(element: &scraper::ElementRef) -> String {
    element.text().collect::<Vec<_>>().join(" ")
}

pub(crate) fn attr(element: &scraper::ElementRef, name: &str) -> Option<String> {
    element.value().attr(name).map(|s| s.to_string())
}