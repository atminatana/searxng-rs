# Серверное логирование — реализация

## Дата
2026-09-23

## Описание
Добавлено цветное логирование работы сервера через `tracing` + `tracing-subscriber` (уже были зависимостями проекта). Все серверные функции теперь логируют вызов с аргументами и ответ со сводкой.

Логи выводятся **одновременно в две цели**:
- **stdout** — с ANSI-цветом (для терминала);
- **файл `searxng-rs.log`** — без ANSI-кодов (рабочая директория процесса).

Файл открывается через `std::fs::File::create` при старте, поэтому **перезаписывается (truncate) при каждом новом запуске** — старые логи не накапливаются.

## Формат вывода

```
{УРОВЕНЬ}  {префикс} {содержание}
```

- **Уровень** — цветной: INFO (синий), WARN (жёлтый), ERROR (красный)
- **Префикс** — `[CALL]` (вызов), `[RESP]` (ответ), `[ENGINE]` (внутренний)
- Префикс модуля `searxng_rs >` убран (`.with_target(false)`)

### Пример

```
 INFO  [CALL] api::search(q="hello", format=None, safesearch=0, pageno=1, engines=None, language=None)
 INFO  [RESP] api::search -> 200, results=5, time=0.342s
WARN  [RESP] api::search -> 400, time=0.001s
 INFO  [CALL] mcp::search(query="rust", engines=None, safesearch=0, language=None, pageno=1)
 INFO  [RESP] mcp::search -> results=3, time=0.512s
 INFO  [CALL] search::search(query="hello", engines=[google,bing], safesearch=0, pageno=1)
 INFO  [RESP] search::search -> results=8, unresponsive=[], time=0.234s
```

## Изменённые файлы

### `src/main.rs`
- `init_tracing(debug, log_path)` — переписан на композицию слоёв через `tracing_subscriber::registry()`:
  - слой stdout — `.with_ansi(true)`, `.with_level(true)`, `.with_target(false)`;
  - слой файла — `.with_writer(Mutex::new(File::create(log_path)?))`, `.with_ansi(false)`, `.with_level(true)`, `.with_target(false)`;
  - фильтр — `EnvFilter::new("searxng_rs={level},reqwest={level}")` (детерминированный, `RUST_LOG` больше не перекрывает);
  - импорты `SubscriberExt` / `SubscriberInitExt` для `.with()` / `.init()`.
- Вызов в `main()`: `init_tracing(config.general.debug, Path::new("searxng-rs.log"))?`.
- `.gitignore` — добавлен `searxng-rs.log`.

### `src/api/mod.rs`
- `search()` — `tracing::info!` при входе (все параметры запроса); `tracing::info!`/`tracing::warn!` при каждом return (статус, кол-во результатов, время, ошибки)
- `config()` — `tracing::info!` при входе и выходе (кол-во движков)
- Сообщение MCP endpoint — заменено `println!` на `tracing::info!`

### `src/mcp/mod.rs`
- `SearchMcpServer::search()` — `tracing::info!` при входе (query, engines, safesearch, language, pageno); `tracing::info!`/`tracing::warn!` при выходе (результаты, ошибки, время)
- `SearchMcpServer::engine_status()` — `tracing::info!` при входе и выходе (фильтр engine, кол-во движков)
- Сообщение MCP endpoint — заменено `println!` на `tracing::info!`

### `src/search/mod.rs`
- `SearchEngine::search()` — `tracing::info!` при входе (query, enginerefs, safesearch, pageno); `tracing::info!` при выходе (результаты, нерабочие движки, общее время)

### `Cargo.toml`
- Без изменений (tracing и tracing-subscriber уже были зависимостями)

## Что НЕ изменено
- `src/main.rs` — CLI-вывод (поиск результатов, список движков, конфиг) остался через `println!`/`eprintln!`, т.к. это не серверное логирование
- Все бизнес-логики и алгоритмы без изменений

## Верификация
- `cargo build` — OK
- `cargo clippy --all-targets` — 0 warnings
- `cargo test` — 80 passed, 0 failed
- Live: `searxng-rs serve` пишет стартовые логи и `[CALL]`/`[RESP] api::config` одновременно в консоль и в `searxng-rs.log`; при повторном запуске старые строки файла исчезают (проверено маркером).
