//! Shared HTTP client construction and helpers. Port of `searx/network` from
//! SearXNG — one configured `reqwest::Client` used by all engines.

use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use reqwest::cookie::{CookieStore, Jar};
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
    /// Cookies the sites set; kept here as well as in the client so they can
    /// be merged with an engine's explicit cookies (reqwest skips the jar
    /// when a request sets its own `Cookie` header).
    jar: Arc<Jar>,
}

impl HttpClient {
    pub fn from_config(cfg: &Config) -> Result<Arc<HttpClient>> {
        Self::new(&cfg.outgoing)
    }

    pub fn new(out: &Outgoing) -> Result<Arc<HttpClient>> {
        // One cookie jar for the client: cookies a site sets (consent,
        // session) go back to it, as a browser would.
        let jar = Arc::new(Jar::default());
        let mut builder = reqwest::Client::builder()
            .user_agent(out.user_agent.clone())
            .cookie_provider(jar.clone())
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
            jar,
        }))
    }

    /// `Cookie` header for `url`: the jar's cookies for that site, with the
    /// engine's explicit cookies overriding same-named ones. `None` when
    /// there is nothing to send.
    fn cookie_header(&self, url: &str, explicit: &[(String, String)]) -> Option<String> {
        let stored = url::Url::parse(url).ok().and_then(|parsed| self.jar.cookies(&parsed));
        let stored = stored.as_ref().and_then(|value| value.to_str().ok()).unwrap_or("");
        let merged = merge_cookies(stored, explicit);
        (!merged.is_empty()).then_some(merged)
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
        if let Some(cookie) = self.cookie_header(url, cookies) {
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
        if let Some(cookie) = self.cookie_header(url, cookies) {
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

/// Merge a stored `Cookie` header value (`a=1; b=2`) with explicit cookies;
/// an explicit cookie replaces a stored one of the same name.
pub fn merge_cookies(stored: &str, explicit: &[(String, String)]) -> String {
    let mut pairs: Vec<(String, String)> = stored
        .split(';')
        .filter_map(|pair| {
            let (name, value) = pair.trim().split_once('=')?;
            Some((name.to_string(), value.to_string()))
        })
        .collect();
    for (name, value) in explicit {
        match pairs.iter_mut().find(|(stored_name, _)| stored_name == name) {
            Some(pair) => pair.1 = value.clone(),
            None => pairs.push((name.clone(), value.clone())),
        }
    }
    pairs
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("; ")
}

/// Navigation headers a browser sends on a top-level page load, in addition
/// to `Accept` and `Accept-Language`.
const NAVIGATION_HEADERS: [(&str, &str); 5] = [
    ("Upgrade-Insecure-Requests", "1"),
    ("Sec-Fetch-Dest", "document"),
    ("Sec-Fetch-Mode", "navigate"),
    ("Sec-Fetch-Site", "none"),
    ("Sec-Fetch-User", "?1"),
];

/// A set of default headers mimicking a real browser's page navigation,
/// similar to what SearXNG sets per request.
pub fn default_headers(lang: Option<&str>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT_LANGUAGE, lang.unwrap_or("en-US,en;q=0.9").parse().unwrap());
    headers.insert(ACCEPT, "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8".parse().unwrap());
    for (name, value) in NAVIGATION_HEADERS {
        headers.insert(name, value.parse().unwrap());
    }
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

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;

    /// Minimal loopback HTTP/1.1 server: every response sets a cookie and
    /// echoes the request headers in its body (one `name: value` per line).
    async fn spawn_echo_server() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                tokio::spawn(async move {
                    let mut raw = Vec::new();
                    let mut chunk = [0u8; 1024];
                    while let Ok(n) = socket.read(&mut chunk).await {
                        if n == 0 {
                            break;
                        }
                        raw.extend_from_slice(&chunk[..n]);
                        if raw.windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                    let request = String::from_utf8_lossy(&raw);
                    let header_lines: Vec<&str> =
                        request.lines().skip(1).take_while(|l| !l.is_empty()).collect();
                    let body = header_lines.join("\n");
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nSet-Cookie: consent=yes; Path=/\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                });
            }
        });
        addr
    }

    fn header_value<'a>(echoed: &'a str, name: &str) -> Option<&'a str> {
        echoed.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case(name).then(|| value.trim())
        })
    }

    async fn body_of(result: Result<reqwest::Response, reqwest::Error>) -> String {
        result.expect("response").text().await.expect("body")
    }

    #[test]
    fn explicit_cookies_override_stored_ones_by_name() {
        let explicit = [("yp".to_string(), "2".to_string()), ("new".to_string(), "x".to_string())];
        assert_eq!(merge_cookies("consent=yes; yp=1", &explicit), "consent=yes; yp=2; new=x");
        assert_eq!(merge_cookies("", &explicit), "yp=2; new=x");
        assert_eq!(merge_cookies("consent=yes", &[]), "consent=yes");
        assert_eq!(merge_cookies("", &[]), "");
    }

    #[tokio::test]
    async fn sends_browser_navigation_headers_and_user_agent() {
        let addr = spawn_echo_server().await;
        let client = HttpClient::new(&Outgoing::default()).expect("client");

        let echoed = body_of(client.get(&format!("http://{addr}/"), Some("de")).await).await;

        assert_eq!(header_value(&echoed, "user-agent"), Some(crate::config::DEFAULT_USER_AGENT));
        assert_eq!(header_value(&echoed, "accept-language"), Some("de"));
        assert_eq!(header_value(&echoed, "upgrade-insecure-requests"), Some("1"));
        assert_eq!(header_value(&echoed, "sec-fetch-dest"), Some("document"));
        assert_eq!(header_value(&echoed, "sec-fetch-mode"), Some("navigate"));
        assert_eq!(header_value(&echoed, "sec-fetch-site"), Some("none"));
        assert_eq!(header_value(&echoed, "sec-fetch-user"), Some("?1"));
        assert_eq!(header_value(&echoed, "cookie"), None, "first request carries no cookie");
    }

    #[tokio::test]
    async fn returns_cookies_set_by_the_site() {
        let addr = spawn_echo_server().await;
        let client = HttpClient::new(&Outgoing::default()).expect("client");
        let url = format!("http://{addr}/");

        body_of(client.get(&url, None).await).await;
        let echoed = body_of(client.get(&url, None).await).await;

        assert_eq!(header_value(&echoed, "cookie"), Some("consent=yes"));
    }

    #[tokio::test]
    async fn explicit_cookies_are_merged_with_the_jar() {
        let addr = spawn_echo_server().await;
        let client = HttpClient::new(&Outgoing::default()).expect("client");
        let url = format!("http://{addr}/");

        body_of(client.get(&url, None).await).await;
        let cookies = [("yp".to_string(), "1".to_string())];
        let echoed = body_of(client.get_with(&url, None, &cookies).await).await;

        let cookie = header_value(&echoed, "cookie").unwrap_or_default();
        assert!(cookie.contains("yp=1"), "explicit cookie missing: {cookie}");
        assert!(cookie.contains("consent=yes"), "jar cookie missing: {cookie}");
    }
}
