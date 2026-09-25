//! CLI entry point. Commands: `search`, `serve`, `mcp`, `engines`, `config`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::{Parser, Subcommand};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

use searxng_rs::config::Config;
use searxng_rs::engine::http::HttpClient;
use searxng_rs::engine::EngineRegistry;
use searxng_rs::search::{parse_query, SearchEngine, SearchQuery};

#[derive(Parser)]
#[command(name = "searxng-rs", version = searxng_rs::VERSION, about = "Metasearch engine (SearXNG port)")]
struct Cli {
    /// Path to the TOML config file (default: $SEARXNG_RS_CONFIG or ./searxng-rs.toml).
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run a search and print results.
    Search {
        query: String,
        /// Limit to these engines (comma-separated).
        #[arg(short = 'e', long)]
        engines: Option<String>,
        /// Output as JSON.
        #[arg(long)]
        json: bool,
        /// Safe search level 0..2.
        #[arg(long, default_value_t = 0)]
        safesearch: u8,
        /// Page number.
        #[arg(long, default_value_t = 1)]
        pageno: u32,
    },
    /// Serve the JSON API + Streamable HTTP MCP on an HTTP port.
    Serve {
        /// Bind address override (default: config `server.bind_address`).
        #[arg(long)]
        bind: Option<String>,
        /// Port override (default: config `server.port`).
        #[arg(short, long)]
        port: Option<u16>,
    },
    /// Run the MCP server (stdio transport).
    Mcp,
    /// Run the MCP server over HTTP (Streamable HTTP transport).
    Sse {
        /// Port to listen on (default: config `mcp.sse_port` or 3001).
        #[arg(short, long)]
        port: Option<u16>,
        /// Bind address (default: 127.0.0.1).
        #[arg(long, default_value = "127.0.0.1")]
        bind: String,
    },
    /// List engines and their status.
    Engines,
    /// Show the merged configuration.
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    Show,
}

fn init_tracing(debug: bool, log_path: &Path) -> anyhow::Result<()> {
    let level = if debug { "trace" } else { "info" };
    let filter = EnvFilter::new(format!("searxng_rs={level},reqwest=warn"));

    let stdout_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .with_ansi(true)
        .with_level(true);

    let file = std::fs::File::create(log_path)?;
    let file_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .with_ansi(false)
        .with_level(true)
        .with_writer(std::sync::Mutex::new(file));

    tracing_subscriber::registry()
        .with(stdout_layer)
        .with(file_layer)
        .with(filter)
        .init();

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let config = Arc::new(Config::load(cli.config.as_deref())?);
    init_tracing(config.general.debug, Path::new("searxng-rs.log"))?;

    let client = HttpClient::from_config(&config)?;
    let registry = Arc::new(EngineRegistry::from_config(&config, &client));
    let search = Arc::new(SearchEngine::new(registry.clone(), client, config.clone()));

    match cli.command {
        Command::Search {
            query,
            engines,
            json,
            safesearch,
            pageno,
        } => {
            let rq = parse_query(&registry, &query, safesearch, pageno);
            let mut sq = SearchQuery::simple(rq.get_query(), safesearch);
            sq.languages = rq.languages.clone();
            sq.pageno = pageno;
            sq.redirect_to_first_result = rq.redirect_to_first_result;
            sq.external_bang = rq.external_bang.clone();
            sq.timeout_limit = rq.timeout_limit;
            if !rq.enginerefs.is_empty() {
                sq.enginerefs = rq.enginerefs;
            } else if let Some(filter) = engines {
                sq.enginerefs = filter
                    .split(',')
                    .map(|name| searxng_rs::query::EngineRef {
                        name: name.trim().to_string(),
                        category: "none".to_string(),
                    })
                    .collect();
            }

            let resp = search.search(&sq).await?;

            if let Some(url) = &resp.redirect_url {
                println!("redirect: {url}");
                return Ok(());
            }

            if json {
                println!("{}", serde_json::to_string_pretty(&resp)?);
                return Ok(());
            }

            for r in &resp.results {
                println!("{}", r.title);
                println!("  {}", r.url);
                if !r.content.is_empty() {
                    println!("  {}", r.content);
                }
                println!();
            }
            if !resp.unresponsive_engines.is_empty() {
                eprintln!(
                    "note: no answer from engines: {}",
                    resp.unresponsive_engines.join(", ")
                );
            }
            Ok(())
        }
        Command::Serve { bind, port } => {
            let bind = bind.unwrap_or_else(|| config.server.bind_address.clone());
            let port = port.unwrap_or(config.server.port);
            searxng_rs::api::serve(config, registry, search, &bind, port).await
        }
        Command::Mcp => {
            searxng_rs::mcp::serve_stdio(search).await
        }
        Command::Sse { port, bind } => {
            let port = port.or(config.mcp.sse_port).unwrap_or(3001);
            searxng_rs::mcp::serve_http(search, &bind, port).await
        }
        Command::Engines => {
            println!("engine                enabled  categories");
            println!("--------------------- -------  --------------------");
            for spec in &registry.specs {
                let (name, enabled) = (&spec.name, spec.enabled);
                let stat = if enabled { "yes" } else { "no" };
                let cats = spec.categories.join(", ");
                println!("{name:<20} {stat:<7}  {cats}");
            }
            println!();
            println!("loaded via registry: {}", registry.names().len());
            Ok(())
        }
        Command::Config { action } => {
            let show = matches!(action, Some(ConfigAction::Show) | None);
            if show {
                println!("{}", toml::to_string_pretty(config.as_ref())?);
            }
            Ok(())
        }
    }
}