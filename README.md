# searxng-rs

Metasearch engine written in Rust. A port of the core functionality of [SearXNG](https://github.com/searxng/searxng):

- Metasearch across multiple engines in parallel
- Single TOML configuration file
- JSON API server
- MCP (Model Context Protocol) server
- CLI

## Quick start

```bash
cargo run -- search "fox jumps over the dog" --json
cargo run -- serve            # JSON API + MCP (Streamable HTTP) on 127.0.0.1:8888
cargo run -- mcp              # MCP over stdio
```

## Configuration

By default the configuration lives in `./searxng-rs.toml` or is given via
`SEARXNG_RS_CONFIG` / `--config`. All fields are optional; sensible defaults
are used otherwise.

```toml
[general]
instance_name = "SearXNG-rs"
default_lang = "auto"

[server]
bind_address = "127.0.0.1"
port = 8888
max_request_timeout = 10.0

[search]
safesearch = 0
autocomplete = false
cache_ttl = 300.0            # engine results cache, seconds (0 = off)
engine_min_interval = 1.0    # min. interval between requests to one engine (0 = off)
ban_time_on_fail = 5         # engine suspension after a timeout / network error

[search.suspended_times]     # engine suspension after typed errors, seconds (0 = off)
access_denied = 180
captcha = 3600
too_many_requests = 180

[outgoing]
user_agent = "Mozilla/5.0 (X11; Linux x86_64; rv:157.0) Gecko/20100101 Firefox/157.0"
proxy = ""
verify = true

[mcp]
enabled = true

[engines]                    # replaces the built-in list: unlisted engines are not registered
bing = { enabled = true, weight = 1.0 }
duckduckgo = { enabled = true, weight = 1.0 }
wikipedia = { enabled = true, weight = 1.0 }

[[engines.custom]]
name = "my_site"
type = "xpath"
search_url = "https://www.example.com/search?q={query}"
results_xpath = "//div[contains(@class,'result')]"
url_xpath = ".//a/@href"
title_xpath = ".//a"
content_xpath = ".//p"
```

## CLI

```text
searxng-rs search "query"                    # run a search, text output
searxng-rs search "query" --json             # JSON output
searxng-rs search "query" -e bing,duckduckgo # limit engines
searxng-rs serve [--bind A] [--port N]       # JSON API server (axum) + MCP at /mcp
searxng-rs mcp                               # MCP server (stdio)
searxng-rs engines                           # list engines and their status
searxng-rs config show                       # show merged configuration
```

## License

AGPL-3.0-or-later, same as SearXNG.