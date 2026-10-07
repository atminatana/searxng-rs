# Отчёт о выполнении 2026-10-07-engine-trust

ТЗ: [spec](../spec/2026-10-07-engine-trust.md) · план: [plans](../plans/2026-10-07-engine-trust.md) ·
ПОЧЕМУ: [ADR 0002](../adr/0002-engine-result-cache.md), [ADR 0003](../adr/0003-engine-rate-limit.md).

## WHAT

| Файл | Изменение |
|---|---|
| `src/config.rs` | `DEFAULT_USER_AGENT` = Firefox 157 без приписки (как `searx/data/useragents.json`); `[search]`: `cache_ttl`, `cache_max_entries`, `engine_min_interval`, таблица `suspended_times` (значения из `searx/settings.yml`) |
| `src/engine/http.rs` | cookie jar клиента (`cookie_provider`), слияние cookie сайта с явными cookie движка (`merge_cookies`), заголовки навигации Firefox; тесты с loopback-сервером |
| `src/engine/mod.rs` | `EngineError::Suspended { reason, remaining_secs }`, `EngineError::RateLimited(f32)`; тестовый конструктор реестра `with_engines` |
| `src/search/suspension.rs` (новый) | порт `SuspendedStatus` + реестр по движкам + `suspension_time(error, config)` |
| `src/search/cache.rs` (новый) | `ResultCache` (TTL, лимит записей, вытеснение старых), `CacheKey` |
| `src/search/rate_limit.rs` (новый) | `RateLimiter::reserve` — слот на движок, отказ при ожидании дольше тайм-аута |
| `src/search/mod.rs` | `run_engine`: приостановлен? → кэш? → слот → запрос (тайм-аут за вычетом ожидания) → сброс и кэш / приостановка; тесты оркестрации с подставным движком |
| `Cargo.toml` | dev-dependency `tokio` с фичей `test-util` (виртуальное время в тестах интервала) |
| `searxng-rs.toml`, `README.md`, `PROJECT_MAP.md` | новые ключи, актуальный User-Agent, описание механизмов; у `searxng-rs.toml` добавлен завершающий перевод строки |

Сверх ТЗ по необходимости: слияние cookie сайта с явными cookie движков. Без него
критерий 5 не выполнялся для brave, yandex, yahoo и startpage: reqwest не добавляет
cookie из хранилища, если запрос задаёт свой заголовок `Cookie`.

Отличие от SearXNG, сохранённое намеренно: пауза после тайм-аута/сетевой ошибки
равна `min(max_ban_time_on_fail, ban_time_on_fail)` = 5 с, счётчик `continuous_errors`
на длину паузы не влияет — ровно как в `abstract.py` эталона.

## COMPLETENESS

Все 5 пунктов ТЗ реализованы; каждый критерий приёмки 1–8 и 10 закрыт кодом и
тестом (таблица — в [verify](../verify/2026-10-07-engine-trust.md)). Критерий 9
закрыт для bing, для duckduckgo — расхождение (см. ⚠ WARNINGS).

## CORRECTNESS

- Приостановка: живой прогон — google 403 → «engine suspended: Access denied; 180s left»
  на втором вызове без HTTP-запроса; duckduckgo CAPTCHA → `WARN [ENGINE] duckduckgo suspended for 3600s`.
- Кэш: повтор запроса к bing — 0.002 с, в логе `cached results`, HTTP-запроса нет.
- Интервал: второй реальный запрос к bing ждал 0.44 с (`rate limit wait`), интервал 1 с выдержан.
- Cookie и заголовки: loopback-сервер эхом вернул User-Agent Firefox 157, все 5 заголовков
  навигации, cookie `consent=yes` во втором запросе, слияние с явной cookie `yp=1`.
- Два теста упали при первом прогоне и были исправлены по причине, не по assert'у:
  остаток приостановки усекался вниз (3599 вместо 3600 — теперь округление вверх);
  явный `Cookie` вытеснял cookie хранилища (теперь слияние).

## EVIDENCE

- `cargo build`, `cargo clippy --all-targets` — 0 warnings; `cargo test` — 116 passed
  (115 unit за 0.02 с + 1 интеграционный за 1.05 с), 0 failed. Новых тестов 25.
- Живые прогоны через MCP stdio (один процесс): bing «rust programming» ×2, bing «rust async»,
  duckduckgo, google ×2 (конфиг с google); curl-матрица к DDG — 8 запросов.
- Эталон сверен по исходникам SearXNG master: `processors/abstract.py` (SuspendedStatus),
  `processors/online.py` (какие исключения приостанавливают), `settings.yml`
  (suspended_times), `data/useragents.json`, `utils.py::gen_useragent`.
- Временные файлы (`searxng-rs.log`, скрипты) в проект не попали; процессов сервера не осталось.

## ⚠ WARNINGS

- **duckduckgo в живой проверке отдаёт CAPTCHA (HTTP 202, страница «anomaly»)** — 4 попытки
  через бинарник (новый UA, старый UA через конфиг, HTTP/1.1, повтор) и 4 из 8 curl-запросов
  с теми же заголовками. Одинаковые curl-запросы получают то выдачу, то капчу, поэтому
  заголовки и UA не причина. `[data needed: ответ DDG с адреса, не попавшего под проверку, —
  решается повтором позже или из другой сети]`. Утром того же дня DDG через бинарник отдавал
  10 результатов. Приостановка это и должна гасить: после капчи DDG не опрашивается 1 час.
- Кэш, интервал и приостановки живут в процессе: перезапуск сервера их сбрасывает, несколько
  экземпляров на одном IP их не разделяют.
- `src/config.rs` — 520 строк без тестов при ориентире 150 для конфигурации/DTO; превышение
  унаследовано (525 строк до задачи). Предлагаю отдельную задачу по разбиению по секциям.
- `AGENTS.md` по-прежнему называет 47 тестов и не описывает новые ключи; файл не в списке
  BP-10.1, не трогал — предлагаю обновить отдельно.
- Коммит не делал (BP-08.4). Закрытие задачи — за владельцем (BP-08.5).
