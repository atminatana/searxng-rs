//! searxng-rs: a metasearch engine in Rust.
//!
//! Ports the core of [SearXNG]: parallel multi-engine search, a single
//! configuration file, JSON API and an MCP server.
//!
//! [SearXNG]: https://github.com/searxng/searxng

pub mod api;
pub mod config;
pub mod engine;
pub mod mcp;
pub mod query;
pub mod search;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");