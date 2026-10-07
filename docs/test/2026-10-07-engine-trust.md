# Отчёт о тестах 2026-10-07-engine-trust

## Состав (25 новых тестов, все офлайн)

| Модуль | Тесты | Что проверяют |
|---|---|---|
| `engine/http.rs` | `explicit_cookies_override_stored_ones_by_name`, `sends_browser_navigation_headers_and_user_agent`, `returns_cookies_set_by_the_site`, `explicit_cookies_are_merged_with_the_jar` | слияние cookie; заголовки навигации и User-Agent; cookie сайта во втором запросе; слияние с явными cookie движка. Три последних — через loopback-сервер `127.0.0.1:0` внутри теста (эхо заголовков), без внешней сети |
| `search/suspension.rs` | `suspension_expires_and_counts_errors`, `zero_duration_counts_the_error_without_suspending`, `registry_tracks_engines_separately`, `suspension_times_follow_the_config_table`, `disabled_suspension_still_counts` | истечение паузы, счётчик, сброс; реестр по движкам; таблица пауз по типам ошибок; значение 0 |
| `search/cache.rs` | `key_covers_every_result_shaping_parameter`, `entries_live_for_ttl`, `oldest_entries_are_evicted_when_full`, `reinsert_refreshes_the_entry`, `expired_entries_make_room_before_eviction`, `zero_ttl_or_zero_capacity_disables_the_cache` | ключ; TTL; вытеснение; обновление; значение 0 |
| `search/rate_limit.rs` | `consecutive_requests_are_spaced_by_the_interval`, `engines_have_independent_slots`, `wait_beyond_max_wait_is_refused_without_reserving`, `zero_interval_disables_the_limit` | интервал; независимость движков; отказ без резервирования; значение 0 |
| `search/mod.rs` (`guard_tests`) | `failed_engine_is_suspended_and_not_queried_again`, `success_resets_the_error_count`, `repeated_query_is_served_from_the_cache`, `errors_are_not_cached`, `requests_to_one_engine_are_spaced_by_the_interval`, `wait_longer_than_the_timeout_is_refused` | сквозной путь оркестратора с подставным движком (`ScriptedEngine`, счётчик вызовов). Два теста интервала — на виртуальном времени tokio (`start_paused`): без реального ожидания |

Время в тестах модулей подставляется явно (`Instant` в аргументах), в тестах
оркестратора — виртуальное время tokio; `sleep` для синхронизации не используется.

## Результат прогона

```
cargo test
  unittests src/lib.rs:   115 passed; 0 failed   (0.02 s)
  tests/mcp_stdio.rs:       1 passed; 0 failed   (1.05 s)
cargo build / cargo clippy --all-targets: 0 warnings
```

Первый прогон: 2 падения (`failed_engine_is_suspended_and_not_queried_again` —
остаток 3599 вместо 3600; `explicit_cookies_are_merged_with_the_jar` — cookie
хранилища отсутствовала). Оба исправлены в коде (округление вверх; слияние cookie),
asserts не менялись.

## Живые проверки (сеть, не входят в `cargo test`)

| Проверка | Результат |
|---|---|
| bing «rust programming» → повтор | 10 результатов за 0.56 с → 10 из кэша за 0.002 с |
| bing «rust async» сразу после | ожидание слота 0.44 с, 10 результатов |
| google ×2 (временный конфиг) | 403 → «Access denied»; второй вызов → «engine suspended: Access denied; 180s left», без HTTP |
| duckduckgo | CAPTCHA (HTTP 202), движок приостановлен на 3600 с — см. ⚠ WARNINGS в done |

## Ограничения

- Живое поведение DDG воспроизводимо проверить нельзя: ответ DDG на одинаковые запросы
  с этого адреса непостоянен.
- Отпечаток TLS/HTTP2 клиента тестами не покрыт и в ТЗ не входил.
