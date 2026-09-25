//! Shared HTTP client construction and helpers. Port of `searx/network` from
//! SearXNG — one configured `reqwest::Client` used by all engines.

use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use reqwest::header::{HeaderMap, ACCEPT, ACCEPT_LANGUAGE, CONTENT_TYPE, COOKIE};

use crate::config::{Config, Outgoing};

#[allow(dead_code)]
pub(crate) const TRACE_BODY_PREVIEW: usize = 2048;

#[allow(dead_code)]
pub(crate) fn body_preview(body: &str) -> String {
    if body.len() <= TRACE_BODY_PREVIEW {
        body.to_string()
    } else {
        let truncated: String = body.chars().take(TRACE_BODY_PREVIEW).collect();
        format!("{}…(truncated, total {} bytes)", truncated, body.len())
    }
}

/// Wrapper around a configured `reqwest::Client` plus request helpers shared
/// by all engines.
#[derive(Clone)]
pub struct HttpClient {
    pub client: reqwest::Client,
    pub proxy: Option<String>,
    pub verify: bool,
    pub user_agent: String,
}

impl HttpClient {
    pub fn from_config(cfg: &Config) -> Result<Arc<HttpClient>> {
        Self::new(&cfg.outgoing)
    }

    pub fn new(out: &Outgoing) -> Result<Arc<HttpClient>> {
        let mut builder = reqwest::Client::builder()
            .user_agent(out.user_agent.clone())
            .tcp_keepalive(std::time::Duration::from_secs(30))
            .pool_max_idle_per_host(out.max_connections_per_host)
            .gzip(true)
            .brotli(true)
            .deflate(true);

        if !out.http2 {
            builder = builder.http1_only();
        }
        if !out.verify {
            builder = builder.danger_accept_invalid_certs(true);
        }
        if let Some(proxy) = &out.proxy {
            builder = builder.proxy(reqwest::Proxy::all(proxy)?);
        }

        let client = builder.build()?;
        Ok(Arc::new(HttpClient {
            client,
            proxy: out.proxy.clone(),
            verify: out.verify,
            user_agent: out.user_agent.clone(),
        }))
    }

    /// Get with a browser-ish header set.
    pub async fn get(&self, url: &str, lang: Option<&str>) -> Result<reqwest::Response, reqwest::Error> {
        tracing::trace!(target: "searxng_rs::http", method = "GET", url, lang = ?lang, cookies = 0, "request");
        let start = Instant::now();
        let result = self.client.get(url).headers(default_headers(lang)).send().await;
        let elapsed = start.elapsed();
        match &result {
            Ok(resp) => tracing::trace!(target: "searxng_rs::http", method = "GET", url, lang = ?lang, status = resp.status().as_u16(), final_url = %resp.url(), content_length = ?resp.content_length(), content_type = ?resp.headers().get(CONTENT_TYPE).and_then(|h| h.to_str().ok()), elapsed = ?elapsed, "response"),
            Err(err) => tracing::trace!(target: "searxng_rs::http", method = "GET", url, lang = ?lang, error = %err, elapsed = ?elapsed, "response error"),
        }
        result
    }

    /// Get with extra cookies (used by engines that need a cookie header,
    /// e.g. Brave's `safesearch`/`ui_lang` and Yandex's `yp`).
    pub async fn get_with(
        &self,
        url: &str,
        lang: Option<&str>,
        cookies: &[(String, String)],
    ) -> Result<reqwest::Response, reqwest::Error> {
        tracing::trace!(target: "searxng_rs::http", method = "GET", url, lang = ?lang, cookies = cookies.len(), "request");
        let mut headers = default_headers(lang);
        if !cookies.is_empty() {
            let cookie = cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; ");
            headers.insert(COOKIE, cookie.parse().unwrap());
        }
        let start = Instant::now();
        let result = self.client.get(url).headers(headers).send().await;
        let elapsed = start.elapsed();
        match &result {
            Ok(resp) => tracing::trace!(target: "searxng_rs::http", method = "GET", url, lang = ?lang, status = resp.status().as_u16(), final_url = %resp.url(), content_length = ?resp.content_length(), content_type = ?resp.headers().get(CONTENT_TYPE).and_then(|h| h.to_str().ok()), elapsed = ?elapsed, "response"),
            Err(err) => tracing::trace!(target: "searxng_rs::http", method = "GET", url, lang = ?lang, error = %err, elapsed = ?elapsed, "response error"),
        }
        result
    }

    pub async fn post(
        &self,
        url: &str,
        form: &[(&str, String)],
        lang: Option<&str>,
    ) -> Result<reqwest::Response, reqwest::Error> {
        tracing::trace!(target: "searxng_rs::http", method = "POST", url, lang = ?lang, form = ?form, "request");
        let start = Instant::now();
        let result = self.client.post(url).headers(default_headers(lang)).form(form).send().await;
        let elapsed = start.elapsed();
        match &result {
            Ok(resp) => tracing::trace!(target: "searxng_rs::http", method = "POST", url, lang = ?lang, status = resp.status().as_u16(), final_url = %resp.url(), content_length = ?resp.content_length(), content_type = ?resp.headers().get(CONTENT_TYPE).and_then(|h| h.to_str().ok()), elapsed = ?elapsed, "response"),
            Err(err) => tracing::trace!(target: "searxng_rs::http", method = "POST", url, lang = ?lang, error = %err, elapsed = ?elapsed, "response error"),
        }
        result
    }

    /// Post a form with extra cookies (used by e.g. Startpage's `preferences`
    /// cookie).
    pub async fn post_with(
        &self,
        url: &str,
        form: &[(String, String)],
        lang: Option<&str>,
        cookies: &[(String, String)],
    ) -> Result<reqwest::Response, reqwest::Error> {
        tracing::trace!(target: "searxng_rs::http", method = "POST", url, lang = ?lang, cookies = cookies.len(), form = ?form, "request");
        let mut headers = default_headers(lang);
        if !cookies.is_empty() {
            let cookie = cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; ");
            headers.insert(COOKIE, cookie.parse().unwrap());
        }
        let form_refs: Vec<(&str, &str)> = form.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let start = Instant::now();
        let result = self.client.post(url).headers(headers).form(&form_refs).send().await;
        let elapsed = start.elapsed();
        match &result {
            Ok(resp) => tracing::trace!(target: "searxng_rs::http", method = "POST", url, lang = ?lang, status = resp.status().as_u16(), final_url = %resp.url(), content_length = ?resp.content_length(), content_type = ?resp.headers().get(CONTENT_TYPE).and_then(|h| h.to_str().ok()), elapsed = ?elapsed, "response"),
            Err(err) => tracing::trace!(target: "searxng_rs::http", method = "POST", url, lang = ?lang, error = %err, elapsed = ?elapsed, "response error"),
        }
        result
    }
}

/// A set of default headers mimicking a real browser, similar to what SearXNG
/// sets per request.
pub fn default_headers(lang: Option<&str>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT_LANGUAGE, lang.unwrap_or("en-US,en;q=0.9").parse().unwrap());
    headers.insert(ACCEPT, "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8".parse().unwrap());
    headers
}

/// Join query params into an encoded query string. Kept simple on purpose:
/// engines build their own URL templates.
pub fn encode_query(q: &str) -> String {
    q.replace(' ', "+")
}

pub fn normalize_user_agent(ua: &str, _proxy: Option<&str>) -> String {
    ua.to_string()
}