use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const DEFAULT_USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64; rv:109.0) Gecko/20100101 Firefox/119.0 searxng-rs/0.1";

/// Complete, validated application configuration. All fields have defaults, so
/// a config file is optional.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub general: General,
    pub server: Server,
    pub search: Search,
    pub outgoing: Outgoing,
    pub mcp: Mcp,
    pub engines: EngineConfig,
}

impl Config {
    pub fn load(path: Option<&Path>) -> Result<Config> {
        let config_path = resolve_config_path(path)?;
        let mut cfg: Config = match &config_path {
            Some(p) => {
                let raw = std::fs::read_to_string(p)
                    .with_context(|| format!("failed to read config file {}", p.display()))?;
                toml::from_str(&raw).with_context(|| format!("failed to parse config {}", p.display()))?
            }
            None => Config::default(),
        };
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&mut self) -> Result<()> {
        self.engines.validate()?;
        Ok(())
    }

    /// Build the list of enabled engine names (both named and custom engines).
    pub fn enabled_engine_names(&self) -> Vec<String> {
        self.engines
            .enabled_names()
            .into_iter()
            .map(|s| s.to_string())
            .collect()
    }
}

/// Resolve the config path: `--config` fleg, then $SEARXNG_RS_CONFIG,
/// then `./searxng-rs.toml` if present. Returns None to mean "defaults only".
fn resolve_config_path(explicit: Option<&Path>) -> Result<Option<std::path::PathBuf>> {
    if let Some(p) = explicit {
        return Ok(Some(p.to_path_buf()));
    }
    if let Ok(env) = std::env::var("SEARXNG_RS_CONFIG") {
        return Ok(Some(std::path::PathBuf::from(env)));
    }
    let local = Path::new("./searxng-rs.toml");
    if local.exists() {
        return Ok(Some(local.to_path_buf()));
    }
    Ok(None)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    pub instance_name: String,
    pub default_lang: String,
    pub debug: bool,
}

impl Default for General {
    fn default() -> Self {
        Self {
            instance_name: "SearXNG-rs".to_string(),
            default_lang: "auto".to_string(),
            debug: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Server {
    pub bind_address: String,
    pub port: u16,
    pub max_request_timeout: f32,
    pub allowed_formats: Vec<String>,
}

impl Default for Server {
    fn default() -> Self {
        Self {
            bind_address: "127.0.0.1".to_string(),
            port: 8888,
            max_request_timeout: 10.0,
            allowed_formats: vec!["json".to_string()],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Search {
    pub safe_search: u8,
    pub default_timeout: f32,
    pub max_page: u32,
    pub autocomplete: bool,
    pub ban_time_on_fail: u32,
    pub max_ban_time_on_fail: u32,
}

impl Default for Search {
    fn default() -> Self {
        Self {
            safe_search: 0,
            default_timeout: 5.0,
            max_page: 50,
            autocomplete: false,
            ban_time_on_fail: 5,
            max_ban_time_on_fail: 120,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Outgoing {
    pub user_agent: String,
    pub proxy: Option<String>,
    pub verify: bool,
    pub http2: bool,
    pub max_connections_per_host: usize,
}

impl Default for Outgoing {
    fn default() -> Self {
        Self {
            user_agent: DEFAULT_USER_AGENT.to_string(),
            proxy: None,
            verify: true,
            http2: true,
            max_connections_per_host: 8,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Mcp {
    pub enabled: bool,
    /// If Some(port), the MCP SSE server binds to this port too.
    /// None (default) means stdio transport only.
    pub sse_port: Option<u16>,
    /// Allowed Host values for the Streamable HTTP MCP server (DNS-rebinding
    /// protection). Matches "host" or "host:port". Empty = allow any.
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
}

impl Default for Mcp {
    fn default() -> Self {
        Self {
            enabled: false,
            sse_port: None,
            allowed_hosts: vec![
                "localhost".to_string(),
                "127.0.0.1".to_string(),
                "::1".to_string(),
            ],
        }
    }
}

/// Named engines are configured by a map `name -> EngineTuning`.
/// Whether they are built-in (google, bing, ...) or custom xpath/json engines
/// depends on the `custom` list.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EngineConfig {
    pub custom: Vec<CustomEngine>,
    #[serde(flatten)]
    pub named: HashMap<String, EngineTuning>,
}

impl Default for EngineConfig {
    fn default() -> Self {
        let mut named = HashMap::new();
        named.insert(
            "google".to_string(),
            EngineTuning {
                enabled: true,
                weight: 1.0,
                timeout: Some(6.0),
            },
        );
        named.insert(
            "bing".to_string(),
            EngineTuning {
                enabled: true,
                weight: 1.0,
                timeout: None,
            },
        );
        named.insert(
            "duckduckgo".to_string(),
            EngineTuning {
                enabled: true,
                weight: 1.0,
                timeout: None,
            },
        );
        named.insert(
            "yandex".to_string(),
            EngineTuning {
                enabled: true,
                weight: 1.0,
                timeout: None,
            },
        );
        named.insert(
            "brave".to_string(),
            EngineTuning {
                enabled: true,
                weight: 1.0,
                timeout: None,
            },
        );
        named.insert(
            "yahoo".to_string(),
            EngineTuning {
                enabled: true,
                weight: 1.0,
                timeout: None,
            },
        );
        named.insert(
            "baidu".to_string(),
            EngineTuning {
                enabled: true,
                weight: 1.0,
                timeout: None,
            },
        );
        named.insert(
            "naver".to_string(),
            EngineTuning {
                enabled: true,
                weight: 1.0,
                timeout: None,
            },
        );
        named.insert(
            "startpage".to_string(),
            EngineTuning {
                enabled: true,
                weight: 1.0,
                timeout: None,
            },
        );
        named.insert(
            "google_news".to_string(),
            EngineTuning {
                enabled: true,
                weight: 1.0,
                timeout: None,
            },
        );
        named.insert(
            "bing_news".to_string(),
            EngineTuning {
                enabled: true,
                weight: 1.0,
                timeout: None,
            },
        );
        named.insert(
            "google_images".to_string(),
            EngineTuning {
                enabled: true,
                weight: 1.0,
                timeout: None,
            },
        );
        named.insert(
            "bing_images".to_string(),
            EngineTuning {
                enabled: true,
                weight: 1.0,
                timeout: None,
            },
        );
        named.insert(
            "github".to_string(),
            EngineTuning {
                enabled: true,
                weight: 1.0,
                timeout: None,
            },
        );
        named.insert(
            "gitlab".to_string(),
            EngineTuning {
                enabled: true,
                weight: 1.0,
                timeout: None,
            },
        );
        named.insert(
            "npm".to_string(),
            EngineTuning {
                enabled: true,
                weight: 1.0,
                timeout: None,
            },
        );
        named.insert(
            "pypi".to_string(),
            EngineTuning {
                enabled: true,
                weight: 1.0,
                timeout: None,
            },
        );
        named.insert(
            "docker_hub".to_string(),
            EngineTuning {
                enabled: true,
                weight: 1.0,
                timeout: None,
            },
        );
        named.insert(
            "crates".to_string(),
            EngineTuning {
                enabled: true,
                weight: 1.0,
                timeout: None,
            },
        );
        named.insert(
            "hackernews".to_string(),
            EngineTuning {
                enabled: true,
                weight: 1.0,
                timeout: None,
            },
        );
        Self {
            custom: vec![],
            named,
        }
    }
}

impl EngineConfig {
    pub fn validate(&self) -> Result<()> {
        if self.named.contains_key("custom") {
            anyhow::bail!("'custom' is a reserved engine name");
        }
        // Ensure custom engine names are unique and don't collide with named ones.
        for custom in &self.custom {
            if self.named.contains_key(&custom.name) {
                anyhow::bail!("custom engine '{}' conflicts with a named engine", custom.name);
            }
            if custom.name.is_empty() {
                anyhow::bail!("custom engine name must not be empty");
            }
        }
        Ok(())
    }

    pub fn enabled_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self
            .named
            .iter()
            .filter(|(_, t)| t.enabled)
            .map(|(k, _)| k.as_str())
            .collect();
        names.extend(self.custom.iter().filter(|c| c.enabled).map(|c| c.name.as_str()));
        names
    }

    pub fn tuning(&self, name: &str) -> EngineTuning {
        self.named
            .get(name)
            .map(EngineTuning::enabled_checked)
            .unwrap_or_default()
    }

    pub fn is_enabled(&self, name: &str) -> bool {
        if let Some(t) = self.named.get(name) {
            return t.enabled;
        }
        self.custom.iter().any(|c| c.enabled && c.name == name)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineTuning {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub weight: f32,
    #[serde(default)]
    pub timeout: Option<f32>,
}

impl EngineTuning {
    fn enabled_checked(&self) -> EngineTuning {
        Self {
            enabled: true,
            ..self.clone()
        }
    }
}

impl Default for EngineTuning {
    fn default() -> Self {
        Self {
            enabled: true,
            weight: 1.0,
            timeout: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum CustomEngineKind {
    #[default]
    Xpath,
    Json,
}

/// A configurable engine defined in TOML without any Rust code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomEngine {
    pub name: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub weight: f32,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub kind: CustomEngineKind,
    /// URL template with `{query}` / `{pageno}` placeholders, e.g.
    /// `https://example.com/search?q={query}&page={pageno}`
    pub search_url: String,
    // XPath selectors (for type = "xpath")
    pub results_xpath: Option<String>,
    pub url_xpath: Option<String>,
    pub title_xpath: Option<String>,
    pub content_xpath: Option<String>,
    pub suggestion_xpath: Option<String>,
    // JSON selectors (for type = "json")
    pub results_query: Option<String>,
    pub url_query: Option<String>,
    pub title_query: Option<String>,
    pub content_query: Option<String>,
    pub suggestion_query: Option<String>,
}

impl CustomEngine {
    pub fn is_xpath(&self) -> bool {
        matches!(self.kind, CustomEngineKind::Xpath)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid() {
        let mut cfg = Config::default();
        assert!(cfg.validate().is_ok());
        assert!(cfg.enabled_engine_names().iter().any(|n| n == "google"));
    }

    #[test]
    fn parses_toml() {
        let raw = r#"
[general]
instance_name = "Test"

[engines]
google = { enabled = false }
custom_pet = { enabled = true, timeout = 3.0 }

[[engines.custom]]
name = "my_site"
type = "xpath"
search_url = "https://e.com/s?q={query}"
results_xpath = "//div"
enabled = true
"#;
        let mut cfg: Config = toml::from_str(raw).unwrap();
        assert_eq!(cfg.general.instance_name, "Test");
        assert!(!cfg.engines.is_enabled("google"));
        assert!(cfg.engines.is_enabled("custom_pet"));
        assert!(cfg.engines.is_enabled("my_site"));
        cfg.validate().unwrap();
    }

    #[test]
    fn rejects_custom_named_collision() {
        let raw = r#"
[engines]
google = { enabled = true }

[[engines.custom]]
name = "google"
type = "xpath"
search_url = "x"
"#;
        let mut cfg: Config = toml::from_str(raw).unwrap();
        assert!(cfg.validate().is_err());
    }
}