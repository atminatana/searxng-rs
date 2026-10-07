# PROJECT_MAP.md

## Проект
`searxng-rs` — метапоисковик на Rust, порт ядра SearXNG.

## Структура
```
src/
  lib.rs                — wiring модулей (api, config, engine, mcp, query, search)
  main.rs               — CLI entry point (search / serve / sse / mcp / engines / config) + init_tracing
  config.rs             — TOML конфигурация + дефолты
  query.rs              — Парсинг !bang / :filter / <timeout>
  engine/
    mod.rs              — Engine trait, EngineError, registry
    http.rs             — общий reqwest::HttpClient
    xpath_engine.rs     — custom engines (XPath)
    json_engine.rs      — custom engines (JSON-path)
    engines/            — 21 встроенных движка: google, google_news, google_images, bing,
                          bing_news, bing_images, duckduckgo, wikipedia, brave, yandex, yahoo,
                          baidu, naver, startpage, github, gitlab, npm, pypi, docker_hub,
                          crates, hackernews (в searxng-rs.toml включены 13 — см. Статус)
  search/
    mod.rs              — Orchestration: parallel engine jobs, timeouts, merging, ranking;
                          per-engine guards (suspension → cache → rate limit → request)
    suspension.rs       — Приостановка движка после ошибок (порт SuspendedStatus из SearXNG)
    cache.rs            — Кэш результатов движков (ADR 0002)
    rate_limit.rs       — Минимальный интервал между запросами к движку (ADR 0003)
    models.rs           — SearchQuery types
    results.rs          — ResultContainer, dedup, scoring
  api/
    mod.rs              — axum JSON API (/healthz, /search) + серверный логинг
  mcp/
    mod.rs              — MCP server (stdio + Streamable HTTP) + серверный логинг
docs/
  server-logging.md     — Описание серверного логирования (tracing)
  adr/                  — Architecture Decision Records (индекс: adr/README.md)
  spec|plans|done|test|verify/ — этапы значительных задач (TL); id = YYYY-MM-DD-slug
```

## Сборка / тесты
```bash
cargo build                  # warning-free
cargo clippy --all-targets    # 0 warnings / 0 errors
cargo test                    # 115 unit + 1 integration (tests/mcp_stdio.rs), all offline
cargo run -- search "query"
cargo run -- serve            # JSON API + Streamable HTTP MCP on 127.0.0.1:8888
cargo run -- sse              # MCP only, Streamable HTTP on 127.0.0.1:3001
cargo run -- mcp              # MCP stdio server
```

Живые проверки MCP (сеть, реальные движки; stdlib Python):
```bash
python test_mcp.py                              # sse: http://127.0.0.1:3001/mcp
python test_mcp.py http://127.0.0.1:8888/mcp    # serve
python test_mcp_stdio.py --config searxng-rs.toml [--engines bing]   # mcp (stdio), exit 0/1
```

## Серверный логирование (2026-09-23)
- Библиотека: `tracing` + `tracing-subscriber` (уже были в Cargo.toml)
- Базовая конфигурация: `init_tracing(debug, log_path, is_stdio_mcp)` в `main.rs`
- Вывод идёт в **две цели** одновременно: консоль (с ANSI-цветом) и файл `searxng-rs.log` (без ANSI)
- Консоль = stdout для всех команд, **кроме `mcp` (stdio)**: там stdout — канал JSON-RPC, поэтому логи идут в stderr без ANSI (2026-10-07, фикс «bing не отвечает через MCP»; регрессия — `tests/mcp_stdio.rs`)
- Файл создаётся через `File::create` — **перезаписывается (truncate) при каждом запуске**
- Фильтр детерминированный: `EnvFilter::new("searxng_rs={level},reqwest=warn")`, level = trace при `general.debug = true`, иначе info (`RUST_LOG` не влияет)
- Логируются: `api::search`, `api::config`, `mcp::search`, `mcp::engine_status`, `search::SearchEngine::search` (TRACE: исходы движков, кэш, ожидание слота), движки (`[CALL]/[RESP] engine::<name>`), `WARN [ENGINE] <name> suspended`
- Формат консоли: `{LEVEL}  [{PREFIX}] {message}` — INFO (синий), WARN (жёлтый), ERROR (красный)
- Детали в `docs/server-logging.md`

## Статус
- `searxng-rs.toml`: в `[engines]` 13 рабочих движков (секция заменяет встроенный список целиком; движок вне её не регистрируется и в MCP даёт `unknown engine`). По умолчанию опрашиваются движки категории general (bing, duckduckgo, wikipedia, yandex, naver), it/news/images — только по явному запросу.
- 2026-10-07 из конфига убраны нерабочие из текущей сети: google, google_news, google_images (403), baidu, startpage (CAPTCHA), pypi (JS-challenge), brave, yahoo (тайм-ауты). Код движков сохранён — вернуть строкой в `[engines]`.
- bing: локаль передаётся только как `setlang=<основной язык>`, без `mkt` и `cc` — отступление от SearXNG, см. [ADR 0001](docs/adr/0001-bing-locale-setlang-only.md).
- Язык `all` (любой регистр, MCP `language`, API или `:all` в запросе) и пустой язык = язык не задан: `EngineParams::language()` возвращает `None`, `:all` вырезается из текста запроса.
- MCP `engine_status`: описание больше не перечисляет все модули, а говорит, что список берётся из конфига и только эти имена допустимы в `search`. Вывод отсортирован по популярности сервиса (`ENGINES_BY_POPULARITY` в `src/mcp/mod.rs`, все 21 встроенный модуль); движки вне списка (custom) — в конце по алфавиту.
- Сборка чистая, clippy чистая, 116 тестов проходят (115 unit + интеграционный `tests/mcp_stdio.rs`).
- Доверие поисковиков (задача `2026-10-07-engine-trust`): движок после CAPTCHA/403/429/тайм-аута приостанавливается на время из `[search.suspended_times]` / `ban_time_on_fail` (порт SearXNG; успех сбрасывает счётчик; клиент MCP видит `engine suspended: <причина>; Ns left`); результаты движков кэшируются (`cache_ttl`, `cache_max_entries`, ADR 0002); запросы к одному движку разделены `engine_min_interval` (ADR 0003); HTTP-клиент хранит cookie сайтов и сливает их с явными cookie движков; заголовки навигации Firefox и User-Agent без приписки (как `gen_useragent()` в SearXNG).
- MCP `search` без результатов: `no results[; unknown engine: <имена не из реестра>][; engine errors: <engine>: <причина>; ...]` (ошибки отсортированы по имени движка); без проблем — просто `no results`. При наличии результатов ответ — JSON-массив, как раньше. Параметр `engines`: явный список имеет приоритет; пустые имена (лишние запятые) отбрасываются; отсутствие, пустая/пробельная строка и список без имён равнозначны — берутся `!bang` из запроса, иначе включённые движки general (`select_engine_refs` в `src/mcp/mod.rs`).
- MCP транспорты: stdio (`mcp`) и Streamable HTTP (`sse`). Streamable HTTP также смонтирован в `serve` на `/mcp`.
- Логирование: все серверные функции логируют вызов с аргументами и ответ через `tracing`.

## Бэклог движков
См. `NEW_ENGINES.md` — оригинальные SearXNG движки, не связанные с видео/аудио, ещё не портированы.
