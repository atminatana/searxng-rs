//! CLI entry point. Commands: `search`, `serve`, `mcp`, `engines`, `config`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::{Parser, Subcommand};
use tracing_subscriber::fmt::writer::BoxMakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

use searxng_rs::config::Config;
use searxng_rs::engine::http::HttpClient;
use searxng_rs::engine::EngineRegistry;
use searxng_rs::search::{parse_query, SearchEngine, SearchQuery};

/// Metasearch engine (Rust port of SearXNG).
///
/// Queries several web search engines in parallel, merges and ranks their
/// results. Usable from the command line, as an HTTP JSON API compatible with
/// SearXNG, and as an MCP server for LLM clients.
///
/// Logs go to the console and to ./searxng-rs.log (truncated on every start).
#[derive(Parser)]
#[command(
    name = "searxng-rs",
    version = searxng_rs::VERSION,
    after_help = "Examples:\n  \
        searxng-rs search \"rust programming\" -e bing,duckduckgo\n  \
        searxng-rs search \"!wp rust :de\" --json\n  \
        searxng-rs serve --bind 0.0.0.0 --port 8080\n  \
        searxng-rs mcp --config /path/to/searxng-rs.toml"
)]
struct Cli {
    /// Path to the TOML config file.
    ///
    /// Default: $SEARXNG_RS_CONFIG, then ./searxng-rs.toml if it exists,
    /// otherwise built-in defaults. Note: an [engines] section replaces the
    /// built-in engine list entirely.
    #[arg(long, global = true, value_name = "FILE")]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run one search and print the results to stdout.
    ///
    /// Prints title, URL and snippet per result, or `redirect: <url>` for a
    /// redirecting !bang. Engines that did not answer are listed on stderr.
    Search {
        /// Search query. Supports `!bang` (engine or category selection,
        /// external redirects), `:lang` (e.g. `:de`, `:all`) and `<timeout`
        /// (e.g. `<3` seconds).
        query: String,
        /// Query only these engines (comma-separated names, see `engines`).
        /// Ignored when the query selects engines with a !bang.
        /// Default: the enabled `general` engines.
        #[arg(short = 'e', long, value_name = "NAMES")]
        engines: Option<String>,
        /// Print the full response as JSON instead of plain text.
        #[arg(long)]
        json: bool,
        /// Safe search level: 0 = off, 1 = moderate, 2 = strict.
        #[arg(long, default_value_t = 0, value_name = "LEVEL")]
        safesearch: u8,
        /// Result page number, starting at 1.
        #[arg(long, default_value_t = 1, value_name = "N")]
        pageno: u32,
    },
    /// Run the HTTP server: JSON API and MCP over Streamable HTTP.
    ///
    /// Routes: GET /healthz, GET /search?q=... (SearXNG-compatible JSON),
    /// GET /config and /mcp (MCP Streamable HTTP endpoint for networked MCP
    /// clients). Runs until interrupted.
    Serve {
        /// Address to listen on. Default: config `server.bind_address` (127.0.0.1).
        #[arg(long, value_name = "ADDR")]
        bind: Option<String>,
        /// Port to listen on. Default: config `server.port` (8888).
        #[arg(short, long, value_name = "PORT")]
        port: Option<u16>,
    },
    /// Run the MCP server over stdio (for MCP clients that spawn the process).
    ///
    /// stdout carries only MCP JSON-RPC; console logs go to stderr. Tools:
    /// `search` and `engine_status`. For an MCP server reachable over the
    /// network use `serve` (endpoint /mcp).
    Mcp,
    /// List the registered engines, whether they are enabled, and their categories.
    Engines,
    /// Print the effective configuration (config file merged with defaults) as TOML.
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Print the effective configuration (the default action).
    Show,
}

/// `is_stdio_mcp`: stdout is the MCP JSON-RPC channel, and the MCP spec
/// forbids anything else there, so console logs go to stderr without ANSI
/// colors (MCP clients capture stderr into their log files).
fn init_tracing(debug: bool, log_path: &Path, is_stdio_mcp: bool) -> anyhow::Result<()> {
    let level = if debug { "trace" } else { "info" };
    let filter = EnvFilter::new(format!("searxng_rs={level},reqwest=warn"));

    let console_writer = if is_stdio_mcp {
        BoxMakeWriter::new(std::io::stderr)
    } else {
        BoxMakeWriter::new(std::io::stdout)
    };
    let console_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .with_ansi(!is_stdio_mcp)
        .with_level(true)
        .with_writer(console_writer);

    let file = std::fs::File::create(log_path)?;
    let file_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .with_ansi(false)
        .with_level(true)
        .with_writer(std::sync::Mutex::new(file));

    tracing_subscriber::registry()
        .with(console_layer)
        .with(file_layer)
        .with(filter)
        .init();

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let config = Arc::new(Config::load(cli.config.as_deref())?);
    let is_stdio_mcp = matches!(cli.command, Command::Mcp);
    init_tracing(config.general.debug, Path::new("searxng-rs.log"), is_stdio_mcp)?;

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