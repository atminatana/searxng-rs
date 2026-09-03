# AGENTS.md

Metasearch engine written in Rust — a port of the core functionality of
[SearXNG](https://github.com/searxng/searxng). This file documents the
architecture, the building/testing commands, and the work that has been done
so far.

## Project layout

```
src/
  lib.rs                - module wiring (api, config, engine, mcp, query, search)
  main.rs               - CLI entry point (search / serve / mcp / engines / config show)
  config.rs             - typed TOML configuration + defaults
  query.rs              - RawTextQuery: parsing of !bang / :filter / <timeout> syntax
  engine/
    mod.rs              - Engine trait, EngineError, EngineParams, EngineSpec, registry
    http.rs             - shared reqwest::HttpClient (get / get_with / post, browser headers)
    xpath_engine.rs     - custom engines driven by XPath expressions
    json_engine.rs      - custom engines driven by JSON-path expressions
    engines/
      google.rs         - Google WEB engine (port of searx/engines/google.py)
      bing.rs           - Bing WEB engine (port of searx/engines/bing.py)
      duckduckgo.rs     - DuckDuckGo HTML engine (port of searx/engines/duckduckgo.py)
      wikipedia.rs      - Wikipedia engine
      brave.rs          - Brave WEB engine, "search" category (port of brave.py)
      yandex.rs         - Yandex engine, "site" WEB category (port of yandex.py)
  search/
    mod.rs              - SearchEngine orchestration, parallel engine jobs, timeouts,
                          !bang redirects, merging + ranking
    models.rs           - SearchQuery / EngineRef types
    results.rs          - ResultContainer, SearchResult
  api/
    mod.rs              - axum JSON API server (/healthz, /search) - SearXNG-compatible
  mcp/
    mod.rs              - Model Context Protocol server (stdio + optional SSE)
```

## Build / test commands

```bash
cargo build                  # must be warning-free
cargo clippy --all-targets    # must be 0 warnings / 0 errors
cargo test                    # 47 unit tests, all pass
cargo run -- search "rust programming"
cargo run -- serve            # JSON API + Streamable HTTP MCP on 127.0.0.1:8888
cargo run -- serve --bind 0.0.0.0 --port 8080
cargo run -- sse              # MCP only, Streamable HTTP on 127.0.0.1:3001 (or mcp.sse_port)
cargo run -- mcp              # MCP stdio server
```

The `serve` command now also mounts the MCP Streamable HTTP endpoint at
`/mcp` and prints `MCP (Streamable HTTP) endpoint: http://<addr>/mcp` to the
console. The `sse` command serves MCP over Streamable HTTP on its own port.

Live-but-unreliable network verification should be done with a short `timeout`
(engines and the test network block rapid repeated scraping).

## Engine backlog (`NEW_ENGINES.md`)

`NEW_ENGINES.md` tracks original SearXNG engines whose search is **not related
to video/audio** and that have not yet been ported to this project. This is the
candidate backlog for future ports.

**Rule**: whenever an engine from `NEW_ENGINES.md` is implemented in this
project (`src/engine/engines/<name>.rs` + registration in the engine registry),
**remove that engine's entry from `NEW_ENGINES.md`** at the same time.

## Configuration

`searxng-rs.toml` (or `SEARXNG_RS_CONFIG` / `--config`). Every field is
optional and falls back to built-in defaults. Key sections: `[general]`,
`[server]`, `[search]`, `[outgoing]`, `[mcp]`, `[engines]`, and
`[[engines.custom]]` for XPath/JSON engines configured entirely in TOML.
Engine tuning supports `enabled`, `weight`, `timeout`.

## Engines

All built-in engines scrape plain HTML/JSON — no API keys required. Each is a
Rust port of the corresponding SearXNG engine (same URL templates, selectors,
and option handling).

- **google** (`google`): relies on the SearXNG-style search endpoint;
  `market_code` maps `en` -> `en-US` etc.
- **bing** (`bing`): `q` plus count/setlang; supports custom override.
- **duckduckgo** (`duckduckgo`): POST to the HTML endpoint, parse of the
  result `link` anchors.
- **wikipedia** (`wikipedia`): full-text API; extracts `SearchResponse`
  hits, plus a "Did you mean" correction.
- **brave** (`brave`): categories `general` (always enabled); scrapes
  `search.brave.com/search`. Sets cookies (`safesearch` off/moderate/strict,
  `useLocation`, `summarizer`, `country`, `ui_lang`), parses organic
  `div[class~="snippet"]` results, filters ad offsets by checking for a
  network location (`has_netloc`), normalizes `published_date` (ISO,
  `d.m.Y`, and short month-word formats) from `span.t-secondary`, extracts a
  thumbnail from `a[class*="thumbnail"] img[src]`, and surface "related
  query" suggestions.
- **yandex** (`yandex`): hits `https://yandex.com/search/site/?...` with the
  `yp` cookie set; detected CAPTCHA via the `x-yandex-captcha: captcha`
  response header maps to `EngineError::Captcha`; extracts
  `li[class*="serp-item"]` items, title from the
  `a.b-serp-item__title-link` text, content from
  `div.b-serp-item__content div.b-serp-item__text`. Language only from
  `SUPPORTED_LANGS` (ru/en/be/fr/de/id/kk/tt/tr/uk); pagination via `p`.
  Note: the yandex.com edge is intermittently unreachable/slow from some
  networks; when it responds it redirects to the yandex.ru edge which is
  reliable.

## Status / notes

- Build clean, `cargo clippy` clean, 47 tests pass.
- MCP transports: stdio (`mcp` command) and Streamable HTTP (`sse` command, or
  mounted at `/mcp` inside `serve`). The Streamable HTTP transport uses
  `rmcp::transport::streamable_http_server::StreamableHttpService` with the
  local in-memory `LocalSessionManager`; sessions are created per `initialize`
  and identified via the `Mcp-Session-Id` header (legacy sessionful mode).
- Live e2e verified: CLI `search` (text + JSON), `/healthz`, `/search`
  (SearXNG-compatible JSON), MCP stdio `initialize` / `tools/list`, and MCP
  Streamable HTTP `initialize` / `tools/list` / `tools/call` (both integrated
  with `serve` and standalone via `sse`).
- The network used during development is unstable (search engines block
  rapid scraping; yandex.com edge flaky). Engine code paths are covered by
  unit tests against fixtures, so a failed live run is usually environmental.

## Recent work

- Repaired the initial build: fixed 34 errors + 6 warnings (lifetime of a
  const `AsciiSet`, non-object-safe trait object in query parsing, rewrite of
  the XPath engine onto the real sxd API, `impl Default for Config`, serde
  defaults on `EngineTuning`, HTTP/1.1 fallback, E0614 in main, test fixes).
- Implemented the **brave** and **yandex** engines (see above) and wired them
  into the engine registry + config defaults.
- Extended `HttpClient` with `get_with(url, lang, cookies)`.
- Added `EngineError::Captcha`.
- Driving clippy back to zero warnings (incl. a `QueryContext` type alias for
  the registry lookup tuple and derivable `Default`s).
- Discovered yandex.com intermittently stalls before connecting; confirmed
  via direct curl probes (0.2 s vs >15 s connect time). No code change fixes
  a broken edge — keeps the upstream `yandex.com` endpoint.
- Added the Streamable HTTP MCP transport: rmcp `transport-streamable-http-server`
  feature, `mcp::serve_http` + `mcp::mcp_router`, `/mcp` route mounted inside
  `serve`, new `sse` CLI command, `--bind`/`--port` overrides for both `serve`
  and `sse`, and `Mcp.allowed_hosts` (DNS-rebinding guard, default
  localhost/127.0.0.1/::1).