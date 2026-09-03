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
cargo run -- serve
cargo run -- mcp
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

[outgoing]
user_agent = "Mozilla/5.0 (X11; Linux x86_64; rv:109.0) Gecko/20100101 Firefox/119.0"
proxy = ""
verify = true

[mcp]
enabled = true

[engines]
google = { enabled = true, weight = 1.0, timeout = 6.0 }
bing = { enabled = true, weight = 1.0 }
duckduckgo = { enabled = true, weight = 1.0 }

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
searxng-rs search "query" -e google,bing     # limit engines
searxng-rs serve                             # JSON API server (axum)
searxng-rs mcp                               # MCP server (stdio)
searxng-rs engines                           # list engines and their status
searxng-rs config show                       # show merged configuration
```

## License

AGPL-3.0-or-later, same as SearXNG.