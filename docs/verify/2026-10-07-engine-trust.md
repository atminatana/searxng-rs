# Верификация 2026-10-07-engine-trust

Второй проход по критериям приёмки ТЗ (BP-09): каждый критерий сверен с кодом,
тестом или живым прогоном. Перепроверено 10 критериев, открыто 9 файлов
(5 исходников, 3 отчёта/ADR, лог живого прогона); расхождение одно (критерий 9).

| # | Критерий | Подтверждение | Статус |
|---|---|---|---|
| 1 | После Captcha / AccessDenied / TooManyRequests / тайм-аута движок не опрашивается; клиент видит `engine suspended` | `suspension_time` в `src/search/suspension.rs`; тест `suspension_times_follow_the_config_table`; оркестрация `failed_engine_is_suspended_and_not_queried_again` (1 вызов движка на 2 поиска); живой прогон google 403 → «engine suspended: Access denied; 180s left» | ✅ |
| 2 | Успех сбрасывает счётчик | `run_engine` → `suspensions.resume`; тесты `success_resets_the_error_count`, `suspension_expires_and_counts_errors` | ✅ |
| 3 | Повтор в пределах TTL без HTTP, те же результаты; ошибки не кэшируются | `repeated_query_is_served_from_the_cache` (1 вызов на 2 поиска, тот же URL), `errors_are_not_cached`; живой прогон 0.002 с | ✅ |
| 4 | Два запроса к движку разделены `engine_min_interval` | `requests_to_one_engine_are_spaced_by_the_interval` (виртуальное время ≥ 1 с), `consecutive_requests_are_spaced_by_the_interval`; живой прогон: ожидание 0.44 с | ✅ |
| 5 | Cookie сайта уходит в следующем запросе | `returns_cookies_set_by_the_site`; для движков с явными cookie — `explicit_cookies_are_merged_with_the_jar` | ✅ |
| 6 | Заголовки навигации и User-Agent по Р1(а) | `sends_browser_navigation_headers_and_user_agent`: UA = `DEFAULT_USER_AGENT` (Firefox 157, без приписки), 5 заголовков | ✅ |
| 7 | Значение 0 отключает механизм | `zero_ttl_or_zero_capacity_disables_the_cache`, `zero_interval_disables_the_limit`, `disabled_suspension_still_counts` (0 → ошибка считается, пауза не ставится, как в SearXNG) | ✅ |
| 8 | Офлайн-тесты на 1–7; build/clippy чистые; тесты зелёные | 25 новых тестов без внешней сети (loopback-сервер для HTTP); `cargo test` 116/116; 0 warnings | ✅ |
| 9 | Живая проверка через MCP: bing и duckduckgo отдают результаты | bing — 10 результатов (×2 запроса); **duckduckgo — CAPTCHA (HTTP 202) в 4 из 4 попыток через бинарник и 4 из 8 через curl с теми же заголовками** | ⚠ расхождение: `[data needed: ответ DDG без challenge — повтор позже / другая сеть]` |
| 10 | ADR на отступления; описание в PROJECT_MAP | `docs/adr/0002-engine-result-cache.md`, `docs/adr/0003-engine-rate-limit.md`, индекс обновлён; `PROJECT_MAP.md` — структура и статус | ✅ |

## Замечания проверочного прохода

- Отчёт done и `PROJECT_MAP.md` не дублируют ПОЧЕМУ — ссылки на ADR (AR-02.3); grep по
  `docs/` на отложенные формулировки про ADR — совпадений нет.
- Один `<id>` прослеживается по `docs/spec|plans|done|test|verify/` (TL-05).
- LOC: `src/search/mod.rs` 388 строк без тестов (< 500); `src/config.rs` 520 —
  унаследованное превышение, вынесено в ⚠ WARNINGS отчёта done.
