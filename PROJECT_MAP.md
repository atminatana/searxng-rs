# PROJECT_MAP.md

## Проект
`searxng-rs` — метапоисковик на Rust, порт ядра SearXNG.

## Структура
```
src/
  lib.rs                — wiring модулей (api, config, engine, mcp, query, search)
  main.rs               — CLI entry point (search / serve / mcp / engines / config)
  config.rs             — TOML конфигурация + дефолты
  query.rs              — Парсинг !bang / :filter / <timeout>
  engine/
    mod.rs              — Engine trait, EngineError, registry
    http.rs             — общий reqwest::HttpClient
    xpath_engine.rs     — custom engines (XPath)
    json_engine.rs      — custom engines (JSON-path)
    engines/
      google.rs         — Google WEB
      bing.rs           — Bing WEB
      duckduckgo.rs     — DuckDuckGo HTML
      wikipedia.rs      — Wikipedia
      brave.rs          — Brave WEB (search category)
      yandex.rs         — Yandex (site category)
  search/
    mod.rs              — Orchestration: parallel engine jobs, timeouts, merging, ranking
    models.rs           — SearchQuery types
    results.rs          — ResultContainer, dedup, scoring
  api/
    mod.rs              — axum JSON API (/healthz, /search) + серверный логинг
  mcp/
    mod.rs              — MCP server (stdio + Streamable HTTP) + серверный логинг
docs/
  server-logging.md     — Описание серверного логирования (tracing)
```

## Сборка / тесты
```bash
cargo build                  # warning-free
cargo clippy --all-targets    # 0 warnings / 0 errors
cargo test                    # 80 unit tests, all pass
cargo run -- search "query"
cargo run -- serve            # JSON API + Streamable HTTP MCP on 127.0.0.1:8888
cargo run -- sse              # MCP only, Streamable HTTP on 127.0.0.1:3001
cargo run -- mcp              # MCP stdio server
```

## Серверный логирование (2026-09-23)
- Библиотека: `tracing` + `tracing-subscriber` (уже были в Cargo.toml)
- Базовая конфигурация: `init_tracing(debug, log_path)` в `main.rs`
- Вывод идёт в **две цели** одновременно: stdout (с ANSI-цветом) и файл `searxng-rs.log` (без ANSI)
- Файл создаётся через `File::create` — **перезаписывается (truncate) при каждом запуске**
- Фильтр детерминированный: `EnvFilter::new("searxng_rs={level},reqwest={level}")` (`RUST_LOG` не влияет)
- Логируются: `api::search`, `api::config`, `mcp::search`, `mcp::engine_status`, `search::SearchEngine::search`
- Формат консоли: `{LEVEL}  [{PREFIX}] {message}` — INFO (синий), WARN (жёлтый), ERROR (красный)
- Детали в `docs/server-logging.md`

## Статус
- Сборка чистая, clippy чистая, 80 тестов проходят.
- MCP транспорты: stdio (`mcp`) и Streamable HTTP (`sse`). Streamable HTTP также смонтирован в `serve` на `/mcp`.
- Логирование: все серверные функции логируют вызов с аргументами и ответ через `tracing`.

## Бэклог движков
См. `NEW_ENGINES.md` — оригинальные SearXNG движки, не связанные с видео/аудио, ещё не портированы.
