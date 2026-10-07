# AGENTS.md

Metasearch engine written in Rust — a port of the core functionality of
[SearXNG](https://github.com/searxng/searxng). This file documents the
architecture, the building/testing commands, and the work that has been done
so far. `PROJECT_MAP.md` (Russian) is the detailed, always-current map;
architectural decisions live in `docs/adr/`.

## Project layout

```
src/
  lib.rs                - module wiring (api, config, engine, mcp, query, search)
  main.rs               - CLI entry point (search / serve / mcp / engines / config show)
                          + tracing setup (console + searxng-rs.log)
  config.rs             - typed TOML configuration + defaults (incl. [search] trust guards)
  query.rs              - RawTextQuery: parsing of !bang / :filter / <timeout> syntax
  engine/
    mod.rs              - Engine trait, EngineError, EngineParams, EngineSpec, registry
    http.rs             - shared reqwest::HttpClient (get / get_with / post / post_with),
                          browser headers, cookie jar
    xpath_engine.rs     - custom engines driven by XPath expressions
    json_engine.rs      - custom engines driven by JSON-path expressions
    engines/            - 21 built-in engines (google, google_news, google_images, bing,
                          bing_news, bing_images, duckduckgo, wikipedia, brave, yandex,
                          yahoo, baidu, naver, startpage, github, gitlab, npm, pypi,
                          docker_hub, crates, hackernews)
  search/
    mod.rs              - SearchEngine orchestration: parallel engine jobs, per-engine
                          guards (suspension -> cache -> rate limit -> request), timeouts,
                          !bang redirects, merging + ranking
    suspension.rs       - engine suspension after errors (port of SearXNG SuspendedStatus)
    cache.rs            - short-lived cache of engine results (ADR 0002)
    rate_limit.rs       - minimum interval between requests to one engine (ADR 0003)
    models.rs           - SearchQuery / EngineRef types
    results.rs          - ResultContainer, SearchResult
  api/
    mod.rs              - axum JSON API server (/healthz, /search) - SearXNG-compatible
  mcp/
    mod.rs              - Model Context Protocol server (stdio + Streamable HTTP)
tests/
  mcp_stdio.rs          - integration test: stdio MCP transport (stdout = JSON-RPC only)
docs/
  adr/                  - Architecture Decision Records (index: adr/README.md)
  spec|plans|done|test|verify/ - lifecycle artifacts of significant tasks (id = YYYY-MM-DD-slug)
  server-logging.md     - tracing setup
test_mcp.py             - live MCP check over Streamable HTTP (stdlib only)
test_mcp_stdio.py       - live MCP check over stdio (stdlib only)
```

## Build / test commands

```bash
cargo build                   # must be warning-free
cargo clippy --all-targets    # must be 0 warnings / 0 errors
cargo test                    # 116 tests (115 unit + 1 integration), all offline
cargo run -- search "rust programming" [-e bing,duckduckgo] [--json]
cargo run -- serve            # JSON API + Streamable HTTP MCP on 127.0.0.1:8888
cargo run -- serve --bind 0.0.0.0 --port 8080
cargo run -- mcp              # MCP stdio server
cargo run -- engines          # registered engines and their status
```

Live checks (real network, not part of `cargo test`; exit code 0/1):

```bash
python test_mcp_stdio.py --config searxng-rs.toml [--engines bing] [--query q] [--language en]
python test_mcp.py                              # serve endpoint http://127.0.0.1:8888/mcp
python test_mcp.py http://host:port/mcp         # serve on another address
```

Tests are offline: HTTP behaviour is tested against a loopback echo server, the
rate limiter on tokio virtual time (`start_paused`, dev-dependency feature
`test-util`). Live-but-unreliable network verification should be done sparingly:
engines block rapid repeated scraping, and a blocked engine is then suspended by
the server itself (see "Trust guards").

## Logging

`tracing` to the console and to `searxng-rs.log` in the working directory
(truncated on every start; `general.debug = true` switches to TRACE). In
`mcp` (stdio) mode the console log goes to **stderr without ANSI** — stdout is
the JSON-RPC channel and must carry nothing else (`tests/mcp_stdio.rs` guards
this). Details: `docs/server-logging.md`.

## Configuration

`searxng-rs.toml` (or `SEARXNG_RS_CONFIG` / `--config`). Every field is
optional and falls back to built-in defaults. Sections: `[general]`,
`[server]`, `[search]`, `[search.suspended_times]`, `[outgoing]`, `[mcp]`,
`[engines]`, and `[[engines.custom]]` for XPath/JSON engines configured
entirely in TOML. Engine tuning supports `enabled`, `weight`, `timeout`.

**`[engines]` replaces the built-in engine list entirely**: an engine missing
from the section is not registered, and MCP reports it as `unknown engine`.
The shipped `searxng-rs.toml` registers 13 engines that work from the
development network; google, google_news, google_images (HTTP 403), baidu,
startpage (CAPTCHA), pypi (JavaScript challenge), brave and yahoo (timeouts)
were removed on 2026-10-07 — the code is kept, add a line to re-enable one.

## Engines

All built-in engines scrape plain HTML/JSON — no API keys required. Each is a
Rust port of the corresponding SearXNG engine (same URL templates, selectors,
and option handling). Engines read the language via `EngineParams::language()`:
`all` (any case), `:all` in the query text and a blank language mean "no
language".

- **bing**: locale is sent only as `setlang=<primary language>` — no `mkt`,
  no `cc` (deviation from SearXNG, [ADR 0001](docs/adr/0001-bing-locale-setlang-only.md):
  with a market/country Bing returns an empty "There are no results" page).
- **brave**, **yandex**, **yahoo**, **startpage** send explicit cookies; these
  are merged with the cookies the site itself set (cookie jar).
- **duckduckgo**: HTTP 202 / "anomaly" page is mapped to `EngineError::Captcha`.
  DDG challenges this IP intermittently after heavy use.

### Trust guards (task `2026-10-07-engine-trust`)

Per engine, in `SearchEngine::run_engine`, before every request:

1. **Suspension** — after `Captcha` (3600 s), `AccessDenied`/403 (180 s),
   `TooManyRequests`/429 (180 s), Cloudflare CAPTCHA (15 days), timeout or
   network error (`min(max_ban_time_on_fail, ban_time_on_fail)` = 5 s) the
   engine is not queried until the time is over; a successful reply resets the
   state. Values: `[search.suspended_times]`, `ban_time_on_fail`; 0 disables.
   Port of SearXNG `SuspendedStatus` + `settings.yml`.
2. **Result cache** — successful engine results are reused for
   `search.cache_ttl` seconds (300), at most `cache_max_entries` (1000); errors
   are never cached. [ADR 0002](docs/adr/0002-engine-result-cache.md).
3. **Rate limit** — at least `search.engine_min_interval` seconds (1.0) between
   two requests to the same engine; a wait longer than the request timeout is
   refused with `rate limited`. [ADR 0003](docs/adr/0003-engine-rate-limit.md).
4. **HTTP client** — cookie jar, Firefox navigation headers
   (`Upgrade-Insecure-Requests`, `Sec-Fetch-*`), User-Agent = current Firefox
   without a product suffix (as SearXNG's `gen_useragent()`).

All guard state is per process (reset on restart, not shared between
instances).

## MCP server

Tools (`tools/list`):

- **search** — `query` (required; supports `!bang`, `:lang`, `<timeout`),
  `engines` (comma-separated names from `engine_status`; empty names from
  stray commas are ignored; omitted/blank = `!bang` selection or the enabled
  `general` engines), `language`, `safesearch`, `pageno`. Reply: JSON array of
  results, or `redirect: <url>`, or
  `no results[; unknown engine: a, b][; engine errors: <engine>: <reason>; ...]`
  (reasons include `CAPTCHA required`, `Access denied`, `request timeout`,
  `engine suspended: <reason>; Ns left`, `rate limited ...`; sorted by engine).
- **engine_status** — registered engines, one per line
  `<name>: enabled|disabled (<categories>)`, sorted by service popularity
  (`ENGINES_BY_POPULARITY`), custom engines last alphabetically; optional
  `engine` filter.

Transports: stdio (`mcp`) and Streamable HTTP (mounted at `/mcp` inside
`serve`; the separate `sse` command was removed on 2026-10-07) via `rmcp` `StreamableHttpService` + `LocalSessionManager`
(sessions per `initialize`, `Mcp-Session-Id` header). `Mcp.allowed_hosts` is a
DNS-rebinding guard (default localhost/127.0.0.1/::1).

## Engine backlog (`NEW_ENGINES.md`)

`NEW_ENGINES.md` tracks original SearXNG engines whose search is **not related
to video/audio** and that have not yet been ported to this project.

**Rule**: whenever an engine from `NEW_ENGINES.md` is implemented in this
project (`src/engine/engines/<name>.rs` + registration in the engine registry),
**remove that engine's entry from `NEW_ENGINES.md`** at the same time.

## Working rules for this repository

- Significant tasks go through `docs/spec -> plans -> done -> test -> verify`
  with one id; architectural decisions get an ADR in `docs/adr/` in the same
  change set.
- Keep `PROJECT_MAP.md` current on every code change; update this file when
  the architecture, commands or engine set change.
- Do not commit on behalf of the owner.

## Status / notes

- Build clean, `cargo clippy` clean, 116 tests pass.
- Live e2e verified 2026-10-07: CLI `search`, `/healthz`, `/search`, MCP stdio
  and Streamable HTTP (`serve`) `initialize` / `tools/list` /
  `tools/call` for both tools; cache hit, rate-limit wait and suspension
  observed in the server log.
- The development network is unstable towards search engines (403, CAPTCHA,
  timeouts — see the removed engines above). Engine code paths are covered by
  unit tests against fixtures, so a failed live run is usually environmental;
  the server log names the reason per engine.

## Recent work

- 2026-10-07, CLI: `sse` command, `mcp::serve_http` and `mcp.sse_port`
  removed — networked MCP is served only by `serve` at `/mcp`; `--help`
  texts expanded (routes, defaults, query syntax, examples).
- 2026-10-07, trust guards: engine suspension, result cache, per-engine rate
  limit, cookie jar + merged cookies, browser headers / User-Agent
  (`docs/done/2026-10-07-engine-trust.md`, ADR 0002, ADR 0003).
- 2026-10-07, MCP: logs to stderr in stdio mode (fixes "bing returns nothing
  via MCP" — log lines corrupted the JSON-RPC stream); `search` reports unknown
  engines and per-engine error reasons; `engines` param semantics; language
  `all`; `engine_status` sorted by popularity; tool descriptions corrected.
- 2026-10-07, engines: bing locale via `setlang` only (ADR 0001); non-working
  engines removed from `searxng-rs.toml`.
- Earlier: initial build repair (34 errors + 6 warnings), brave and yandex
  engines, `HttpClient::get_with`, `EngineError::Captcha`, Streamable HTTP MCP
  transport (`sse` command, `/mcp` route, `--bind`/`--port`, `Mcp.allowed_hosts`),
  server logging via `tracing`.
