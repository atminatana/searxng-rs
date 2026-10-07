# План 2026-10-07-engine-trust

ТЗ: [docs/spec/2026-10-07-engine-trust.md](../spec/2026-10-07-engine-trust.md).
Открытых решений нет: Р1–Р4 приняты по рекомендациям.

## Рамки

Меняются: `src/config.rs`, `src/engine/http.rs`, `src/engine/mod.rs`
(варианты `EngineError`), `src/search/mod.rs` и новые модули
`src/search/{suspension,cache,rate_limit}.rs`, `searxng-rs.toml`, ADR, документы.
Движки (`src/engine/engines/*`) и MCP/API-слои не меняются.

## Шаги

1. **Конфиг и клиент.** `[search]`: `cache_ttl`, `cache_max_entries`,
   `engine_min_interval`, `suspended_times`; новый `DEFAULT_USER_AGENT`
   (Firefox 157, как в `searx/data/useragents.json`). `HttpClient`: cookie jar,
   заголовки навигации. Проверка: юнит-тест с локальным loopback-сервером
   (эхо заголовков и cookie).
2. **Приостановка** (`search/suspension.rs`): порт `SuspendedStatus`;
   таблица пауз из конфига. Проверка: юнит-тесты с подставным временем.
3. **Кэш** (`search/cache.rs`): TTL + лимит записей, ключ по движку и параметрам.
   Проверка: юнит-тесты с подставным временем.
4. **Частота** (`search/rate_limit.rs`): резервирование слота на движок,
   отказ, если ждать дольше тайм-аута. Проверка: юнит-тесты с подставным временем.
5. **Оркестрация** (`search/mod.rs`): порядок на движок — приостановлен? →
   кэш? → слот частоты → запрос → кэш/сброс или приостановка. Новые варианты
   `EngineError::Suspended {..}` и `RateLimited`. Проверка: офлайн-тест
   оркестрации с подставным движком (капча → второй поиск без вызова; успех →
   из кэша; сброс после успеха).
6. **Документы.** ADR 0002 (кэш), ADR 0003 (частота), `PROJECT_MAP.md`,
   `searxng-rs.toml`, отчёты `docs/done|test|verify`. Живая проверка через MCP.

Сборка и прогон тестов — после шагов 1–4 как одной единицы и после шага 5.
