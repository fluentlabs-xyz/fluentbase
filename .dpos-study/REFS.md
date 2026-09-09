# Проверка ссылок реестра REGISTER.md

Дата: 2026-09-03. Проверены записи R-006..R-100 (R-001..R-005 пропущены — их ссылки установлены в `VERIFY-BLOCKERS.md`). Проверено 312 уникальных ссылок в 95 записях.

Что проверялось для каждой ссылки: файл существует; диапазон в пределах файла; код в диапазоне показывает именно тот механизм, о котором говорит запись. Каждый диапазон открывался и читался; правдоподобное соседнее место засчитывалось как промах.

Разрешение путей: без префикса — `crates/dpos/consensus/src/`; `beacon/`, `slasher/` — подкаталоги там же; `bls/`, `p2p/`, `staking-reader/` — `crates/dpos/<крейт>/`; `node/` — `crates/node/src/`; `CW:` — commonware v2026.4.0 (`~/.cargo/git/checkouts/monorepo-27b478c9bb41d208/3c4e02c`, tag сверен с `Cargo.toml`); `RETH:` — `~/.cargo/git/checkouts/reth-9084db4313ec21c5/c9ae365`.

Ссылки-продолжения вида `` `:1234` `` разрешались по последнему явно названному файлу — так их читает человек. Четыре промаха ниже возникли именно так: в перечислении между явным именем файла и продолжением вклинилось имя другого файла.

## Сводка

| Вердикт | Ссылок |
|---|---|
| верна | 304 |
| файла нет | 0 |
| за границей файла | 4 |
| код не тот | 4 |

По записям промахи затрагивают шесть записей: R-007, R-031, R-038 (две ссылки), R-064, R-080 (две ссылки), R-088.

По файлам (где промахи): `node/dpos.rs` — 1 из 4; `plane_upstream.rs` — 1 из 8; `beacon/ceremony.rs` — 2 из 4; `cold_start_jump.rs` — 1 из 4; `slasher/ingress.rs` — 2 из 3; `beacon/seed_journal.rs` — 1 из 3. Файлы с наибольшим числом ссылок — `dpos.rs` (27), `executor.rs` (26), `beacon/actor.rs` (25), `cert_inlet.rs` (19), `application.rs` (16) — промахов не дали ни одного.

## Промахи по существу

**R-007, `node/dpos.rs:2026-2029`** — «node всегда даёт `Some(upstream)`». По ссылке лежит объявление `enum ValidatorUpstream` с его doc-комментарием, то есть тип, а не место, где upstream заворачивается в `Some`. Само заворачивание — `node/dpos.rs:2242-2258` (`let upstream = Some(if cfg.follower_upstreams.is_empty() { …Plane… } else { …Ws… })`). Это промах того же класса, что R-002: по ссылке правдоподобный код о том же предмете.

**R-031, `:1962-1997`** — `fcu_retrying_transport`. По последнему названному перед ним файлу (`plane_upstream.rs`, 404 строки) диапазон за границей. Механизм — `executor.rs:1962-1997`.

**R-038, `:2178-2235`** — `try_recompute_pending`. По последнему названному файлу (`beacon/ceremony.rs`, 2088 строк) диапазон за границей. Механизм — `beacon/actor.rs:2178-2235`.

**R-038, `:837-841`** — `derive_pinned` на каждый verify/propose. `beacon/ceremony.rs:837-841` — это `can_finalize` и doc-комментарий к хэшу лога, другой код. Механизм — `beacon/actor.rs:837-841` (ветка `select!`, отвечающая на `PinnedRequest` через `self.derive_pinned`).

**R-064, `:1257-1323`** в блоке «Уверенность» — `cold_start_jump_self_heal`. По последнему названному файлу (`cold_start_jump.rs`) диапазон попадает в тестовый модуль (`FakeUpstream` и соседние фикстуры). Механизм — `dpos.rs:1257-1323`, уже названный в блоке «Код».

**R-080, `ingress.rs:165`, `:290`** — «ревью-комментарии в коде». `slasher/ingress.rs` — 137 строк, обеих строк нет. Ревью-комментарии (маркер `****`) во всём `crates/dpos` ровно три: `slasher/ingress.rs:8`, `slasher/ingress.rs:133`, `slasher/actor.rs:652`. Последний в записи уже назван; первые два и есть верные ссылки. Исходный AUDIT A-39 давал здесь `ingress.rs` без номеров строк — номера появились в реестре.

**R-088, `beacon/seed_journal.rs:445-452`** — «`seed_journal::append` для раунда ниже pruned-floor пересоздаёт blob, следующий prune его удаляет». По ссылке — вторая половина механизма: вызов `prune_to_window` в `spawn_writer` при `rolled`. Сам `append` с безусловным `store.put(index)` и `rolled = false` для старой эпохи — `beacon/seed_journal.rs:181-190`. Ссылка на prune оставлена, ссылка на `append` добавлена.

## Таблица: запись → ссылка → вердикт

| Запись | Ссылка | Вердикт | Верная ссылка |
|---|---|---|---|
| R-006 | `executor.rs:3131-3152` | верна | — |
| R-006 | `executor.rs:3039-3041` | верна | — |
| R-006 | `executor.rs:3635-3645` | верна | — |
| R-006 | `executor.rs:3289` | верна | — |
| R-006 | `executor.rs:2684-2732` | верна | — |
| R-006 | `executor.rs:2975-3007` | верна | — |
| R-006 | `node/ordering.rs:45-47` | верна | — |
| R-006 | `order_block.rs:188-198` | верна | — |
| R-006 | `RETH:crates/engine/tree/src/tree/mod.rs:1551-1573` | верна | — |
| R-006 | `executor.rs:3215` | верна | — |
| R-006 | `executor.rs:3121-3130` | верна | — |
| R-006 | `executor.rs:7430-7460` | верна | — |
| R-006 | `executor.rs:4199-4209` | верна | — |
| R-007 | `spec_exec.rs:52` | верна | — |
| R-007 | `spec_exec.rs:99` | верна | — |
| R-007 | `spec_exec.rs:107` | верна | — |
| R-007 | `cert_inlet.rs:863` | верна | — |
| R-007 | `cert_inlet.rs:3152` | верна | — |
| R-007 | `outer.rs:1426-1500` | верна | — |
| R-007 | `node/dpos.rs:2026-2029` | код не тот | `node/dpos.rs:2242-2258` |
| R-007 | `dpos.rs:2447-2454` | верна | — |
| R-007 | `executor.rs:1764-1794` | верна | — |
| R-007 | `executor.rs:2845-2871` | верна | — |
| R-007 | `executor.rs:1388-1411` | верна | — |
| R-007 | `executor.rs:1391-1397` | верна | — |
| R-007 | `dpos.rs:2458` | верна | — |
| R-007 | `voter/actor.rs:454-462` | верна | — |
| R-007 | `voter/actor.rs:630-654` | верна | — |
| R-007 | `batcher/actor.rs:568` | верна | — |
| R-007 | `executor.rs:2283-2535` | верна | — |
| R-008 | `bls/src/combined_scheme.rs:428-444` | верна | — |
| R-008 | `cert_inlet.rs:690` | верна | — |
| R-008 | `cert_inlet.rs:824-863` | верна | — |
| R-008 | `cert_inlet.rs:917-930` | верна | — |
| R-008 | `beacon/certify.rs:288-327` | верна | — |
| R-008 | `beacon/log_resolver.rs:122-138` | верна | — |
| R-008 | `executor.rs:1764-1794` | верна | — |
| R-008 | `dpos.rs:2458` | верна | — |
| R-009 | `plane_upstream.rs:198-212` | верна | — |
| R-009 | `cert_inlet.rs:3151-3153` | верна | — |
| R-009 | `CW:marshal/core/actor.rs:988-996` | верна | — |
| R-009 | `CW:resolver/src/p2p/fetcher.rs:233-250` | верна | — |
| R-009 | `outer.rs:94-105` | верна | — |
| R-010 | `application.rs:496-509` | верна | — |
| R-010 | `application.rs:817-1010` | верна | — |
| R-010 | `application.rs:273` | верна | — |
| R-010 | `application.rs:344` | верна | — |
| R-010 | `application.rs:368` | верна | — |
| R-010 | `application.rs:397` | верна | — |
| R-010 | `application.rs:663` | верна | — |
| R-010 | `order_block.rs:104` | верна | — |
| R-011 | `staking-reader/src/epoch_transition.rs:44-65` | верна | — |
| R-011 | `staking-reader/src/epoch_transition.rs:465-483` | верна | — |
| R-011 | `dpos.rs:1651-1654` | верна | — |
| R-012 | `beacon/actor.rs:1616-1653` | верна | — |
| R-012 | `node/dpos.rs:1410-1430` | верна | — |
| R-012 | `beacon/carry.rs:158-174` | верна | — |
| R-012 | `resolve.rs:84-131` | верна | — |
| R-012 | `oracle.rs:115-132` | верна | — |
| R-013 | `outer.rs:823-833` | верна | — |
| R-013 | `dpos.rs:2650` | верна | — |
| R-013 | `order_block.rs:31` | верна | — |
| R-013 | `order_block.rs:357-371` | верна | — |
| R-013 | `p2p/src/constants.rs:128` | верна | — |
| R-013 | `p2p/src/constants.rs:161` | верна | — |
| R-013 | `CW:broadcast/src/buffered/engine.rs:321-360` | верна | — |
| R-014 | `slasher/actor.rs:486-538` | верна | — |
| R-014 | `slasher/actor.rs:599-610` | верна | — |
| R-014 | `CW:batcher/round.rs:115-135` | верна | — |
| R-015 | `executor.rs:3370-3462` | верна | — |
| R-016 | `dpos.rs:2458` | верна | — |
| R-016 | `executor.rs:2172-2185` | верна | — |
| R-017 | `beacon/share_state.rs:677-683` | верна | — |
| R-017 | `beacon/actor.rs:224-227` | верна | — |
| R-017 | `beacon/carry.rs:158-174` | верна | — |
| R-017 | `actor.rs:220-223` | верна | — |
| R-018 | `epoch_manager.rs:1131-1161` | верна | — |
| R-018 | `epoch_manager.rs:638-874` | верна | — |
| R-018 | `executor.rs:3241` | верна | — |
| R-019 | `dpos.rs:1959-1973` | верна | — |
| R-019 | `plane_upstream.rs:173-174` | верна | — |
| R-019 | `beacon/artifact.rs:325-329` | верна | — |
| R-019 | `slasher/evidence.rs:132-135` | верна | — |
| R-019 | `application.rs:471-476` | верна | — |
| R-020 | `beacon/key_journal.rs:262-300` | верна | — |
| R-020 | `beacon/seed_journal.rs:410-456` | верна | — |
| R-020 | `beacon/certify.rs:191-242` | верна | — |
| R-020 | `dpos.rs:629-706` | верна | — |
| R-020 | `dpos.rs:846-858` | верна | — |
| R-021 | `beacon/actor.rs:1006-1016` | верна | — |
| R-021 | `beacon/actor.rs:2093-2101` | верна | — |
| R-021 | `beacon/mod.rs:106` | верна | — |
| R-022 | `slasher/actor.rs:1174-1181` | верна | — |
| R-022 | `slasher/actor.rs:742-744` | верна | — |
| R-022 | `slasher/actor.rs:1144-1153` | верна | — |
| R-023 | `beacon/actor.rs:1409-1434` | верна | — |
| R-023 | `beacon/actor.rs:1852-1855` | верна | — |
| R-023 | `p2p/src/constants.rs:132` | верна | — |
| R-024 | `beacon/actor.rs:115` | верна | — |
| R-024 | `beacon/actor.rs:1183-1200` | верна | — |
| R-025 | `beacon/mod.rs:106` | верна | — |
| R-025 | `beacon/share_state.rs:668-674` | верна | — |
| R-025 | `beacon/actor.rs:2093-2101` | верна | — |
| R-026 | `beacon/dkg_engine.rs:376-403` | верна | — |
| R-026 | `beacon/dkg_engine.rs:580-581` | верна | — |
| R-026 | `beacon/dkg_engine.rs:602-623` | верна | — |
| R-026 | `beacon/actor.rs:944-974` | верна | — |
| R-026 | `beacon/actor.rs:2099-2137` | верна | — |
| R-027 | `slasher/actor.rs:830-837` | верна | — |
| R-027 | `slasher/actor.rs:828` | верна | — |
| R-028 | `slasher/actor.rs:324-345` | верна | — |
| R-028 | `application.rs:630-652` | верна | — |
| R-028 | `application.rs:703-749` | верна | — |
| R-028 | `node/evm.rs:1597-1603` | верна | — |
| R-028 | `node/evm.rs:1213-1256` | верна | — |
| R-029 | `p2p/src/lib.rs:362-369` | верна | — |
| R-029 | `dpos.rs:2631` | верна | — |
| R-029 | `dpos.rs:3511` | верна | — |
| R-029 | `beacon/plane.rs:155` | верна | — |
| R-029 | `beacon/dkg_engine.rs:339` | верна | — |
| R-029 | `p2p/src/lib.rs:360-362` | верна | — |
| R-030 | `staking-reader/src/reader.rs:52-69` | верна | — |
| R-030 | `staking-reader/src/error.rs:23-41` | верна | — |
| R-031 | `executor.rs:1517-1533` | верна | — |
| R-031 | `plane_upstream.rs:70` | верна | — |
| R-031 | `plane_upstream.rs:280` | верна | — |
| R-031 | `plane_upstream.rs:1962-1997` | за границей файла | `executor.rs:1962-1997` |
| R-031 | `application.rs:1159-1187` | верна | — |
| R-032 | `outer.rs:1536-1582` | верна | — |
| R-033 | `cert_inlet.rs:93-102` | верна | — |
| R-033 | `cert_inlet.rs:811` | верна | — |
| R-033 | `dpos.rs:3915-3931` | верна | — |
| R-034 | `application.rs:840` | верна | — |
| R-034 | `slasher/tombstone.rs:40-49` | верна | — |
| R-034 | `node/dpos.rs:1748` | верна | — |
| R-035 | `staking-reader/src/reader.rs:700-713` | верна | — |
| R-035 | `weighted_vrf.rs:86-103` | верна | — |
| R-035 | `epoch_manager.rs:1461-1464` | верна | — |
| R-036 | `beacon/actor.rs:1660-1661` | верна | — |
| R-036 | `beacon/actor.rs:1669-1670` | верна | — |
| R-036 | `beacon/actor.rs:1183-1200` | верна | — |
| R-036 | `ceremony.rs:235-240` | верна | — |
| R-036 | `share_state.rs:545-551` | верна | — |
| R-037 | `beacon/dkg_transport.rs:118-128` | верна | — |
| R-037 | `beacon/artifact.rs:146-152` | верна | — |
| R-037 | `CW:broadcast/src/buffered/engine.rs:312-360` | верна | — |
| R-038 | `beacon/actor.rs:1470` | верна | — |
| R-038 | `ceremony.rs:875` | верна | — |
| R-038 | `ceremony.rs:2178-2235` | за границей файла | `beacon/actor.rs:2178-2235` |
| R-038 | `ceremony.rs:837-841` | код не тот | `beacon/actor.rs:837-841` |
| R-039 | `beacon/actor.rs:1530-1557` | верна | — |
| R-039 | `beacon/actor.rs:2210-2213` | верна | — |
| R-039 | `CW:cryptography/src/bls12381/dkg.rs:1830-1846` | верна | — |
| R-039 | `combined_scheme.rs:348-351` | верна | — |
| R-040 | `cert_follow.rs:200-202` | верна | — |
| R-040 | `cold_start_jump.rs:669-673` | верна | — |
| R-040 | `dpos.rs:444-466` | верна | — |
| R-040 | `CW:marshal/core/actor.rs:567-605` | верна | — |
| R-040 | `CW:marshal/core/actor.rs:1404-1430` | верна | — |
| R-040 | `dpos.rs:3880` | верна | — |
| R-040 | `epoch_manager.rs:1343-1354` | верна | — |
| R-041 | `engine.rs:267` | верна | — |
| R-041 | `epoch_manager.rs:318-362` | верна | — |
| R-042 | `dpos.rs:2996` | верна | — |
| R-042 | `dpos.rs:3040` | верна | — |
| R-042 | `dpos.rs:3094-3105` | верна | — |
| R-043 | `executor.rs:2305-2345` | верна | — |
| R-043 | `executor.rs:2004-2055` | верна | — |
| R-044 | `application.rs:840-849` | верна | — |
| R-044 | `node/dpos.rs:1748` | верна | — |
| R-045 | `engine.rs:195` | верна | — |
| R-045 | `engine.rs:258` | верна | — |
| R-045 | `epoch_manager.rs:1402-1410` | верна | — |
| R-045 | `epoch_manager.rs:1461-1464` | верна | — |
| R-045 | `outer.rs:392-401` | верна | — |
| R-045 | `outer.rs:349-358` | верна | — |
| R-046 | `staking-reader/src/epoch_transition.rs:423-429` | верна | — |
| R-047 | `staking-reader/src/epoch_transition.rs:739` | верна | — |
| R-047 | `staking-reader/src/epoch_transition.rs:324-326` | верна | — |
| R-048 | `staking-reader/src/epoch_transition.rs:647-648` | верна | — |
| R-049 | `dpos.rs:3230-3233` | верна | — |
| R-049 | `dpos.rs:3129-3135` | верна | — |
| R-049 | `dpos.rs:3622-3629` | верна | — |
| R-050 | `sync_metrics.rs:556-580` | верна | — |
| R-050 | `sync_metrics.rs:542-554` | верна | — |
| R-051 | `beacon/share_state.rs:575-621` | верна | — |
| R-051 | `beacon/actor.rs:1673-1686` | верна | — |
| R-052 | `beacon/share_state.rs:266-271` | верна | — |
| R-053 | `beacon/keys.rs:333-342` | верна | — |
| R-053 | `beacon/artifact.rs:452-461` | верна | — |
| R-054 | `beacon/ceremony.rs:364-371` | верна | — |
| R-054 | `beacon/actor.rs:1916-1933` | верна | — |
| R-055 | `beacon/ceremony.rs:226-240` | верна | — |
| R-056 | `beacon/carry.rs:74-76` | верна | — |
| R-056 | `beacon/carry.rs:131-147` | верна | — |
| R-056 | `beacon/carry.rs:158-174` | верна | — |
| R-056 | `resolve.rs:84` | верна | — |
| R-056 | `surface.rs:1952` | верна | — |
| R-056 | `surface.rs:1985` | верна | — |
| R-056 | `surface.rs:1792` | верна | — |
| R-056 | `carry.rs:158-174` | верна | — |
| R-057 | `weighted_vrf.rs:207-245` | верна | — |
| R-057 | `epoch_manager.rs:158-188` | верна | — |
| R-058 | `engine.rs:267` | верна | — |
| R-059 | `slasher/gossip.rs:49-51` | верна | — |
| R-059 | `slasher/gossip.rs:137-150` | верна | — |
| R-060 | `beacon/actor.rs:2315-2324` | верна | — |
| R-060 | `beacon/log_store.rs:113-121` | верна | — |
| R-060 | `beacon/log_store.rs:136-158` | верна | — |
| R-061 | `executor.rs:184-208` | верна | — |
| R-061 | `feed_sink.rs:21` | верна | — |
| R-061 | `slasher/ingress.rs:59` | верна | — |
| R-061 | `beacon/keys.rs:174` | верна | — |
| R-061 | `beacon/certify.rs:84` | верна | — |
| R-062 | `p2p/src/config.rs:72-78` | верна | — |
| R-063 | `cert_inlet.rs:766-775` | верна | — |
| R-063 | `cert_inlet.rs:806-810` | верна | — |
| R-063 | `cert_inlet.rs:3166-3172` | верна | — |
| R-064 | `dpos.rs:192-232` | верна | — |
| R-064 | `dpos.rs:1257-1323` | верна | — |
| R-064 | `dpos.rs:2191-2268` | верна | — |
| R-064 | `cold_start_jump.rs:183` | верна | — |
| R-064 | `cold_start_jump.rs:1257-1323` | код не тот | `dpos.rs:1257-1323` |
| R-065 | `dpos.rs:2149-2283` | верна | — |
| R-065 | `dpos.rs:2234-2243` | верна | — |
| R-066 | `staking-reader/src/epoch_transition.rs:545-579` | верна | — |
| R-066 | `dpos.rs:2244-2266` | верна | — |
| R-067 | `CW:resolver/src/p2p/engine.rs:415-442` | верна | — |
| R-067 | `beacon/log_resolver.rs:385-401` | верна | — |
| R-067 | `beacon/actor.rs:2057-2064` | верна | — |
| R-067 | `beacon/plane.rs:74` | верна | — |
| R-068 | `beacon/keys.rs:280-297` | верна | — |
| R-068 | `beacon/key_journal.rs:170-181` | верна | — |
| R-069 | `beacon/keys.rs:416-431` | верна | — |
| R-069 | `beacon/certify.rs:281-327` | верна | — |
| R-069 | `beacon/log_resolver.rs:122-138` | верна | — |
| R-069 | `cert_inlet.rs:3032-3038` | верна | — |
| R-070 | `beacon/metrics.rs:1-3` | верна | — |
| R-070 | `keys.rs:332` | верна | — |
| R-070 | `surface.rs:1754-1758` | верна | — |
| R-070 | `resolve.rs:104-128` | верна | — |
| R-070 | `key_journal.rs:215-292` | верна | — |
| R-070 | `seed_journal.rs:249-439` | верна | — |
| R-070 | `artifact.rs:613-678` | верна | — |
| R-071 | `beacon/actor.rs:1495-1498` | верна | — |
| R-071 | `beacon/actor.rs:1677-1683` | верна | — |
| R-072 | `beacon/actor.rs:1742-1751` | верна | — |
| R-072 | `beacon/actor.rs:1255` | верна | — |
| R-072 | `share_state.rs:593-621` | верна | — |
| R-072 | `actor.rs:1673-1686` | верна | — |
| R-073 | `epoch_manager.rs:1192-1200` | верна | — |
| R-073 | `epoch_manager.rs:1041-1074` | верна | — |
| R-074 | `executor.rs:2329-2351` | верна | — |
| R-074 | `executor.rs:2028-2039` | верна | — |
| R-075 | `outer.rs:1188-1192` | верна | — |
| R-075 | `outer.rs:339-369` | верна | — |
| R-075 | `epoch_manager.rs:1792` | верна | — |
| R-076 | `dpos.rs:1464-1482` | верна | — |
| R-076 | `CW:marshal/core/actor.rs:963-972` | верна | — |
| R-077 | `executor.rs:2857-2871` | верна | — |
| R-077 | `sync_metrics.rs:565` | верна | — |
| R-078 | `CW:marshal/core/actor.rs:1450` | верна | — |
| R-078 | `outer.rs:1575-1581` | верна | — |
| R-079 | `p2p/src/bootstrappers.rs:157-163` | верна | — |
| R-080 | `slasher/ingress.rs:134-137` | верна | — |
| R-080 | `slasher/actor.rs:652` | верна | — |
| R-080 | `ingress.rs:165` | за границей файла | `slasher/ingress.rs:8` |
| R-080 | `ingress.rs:290` | за границей файла | `slasher/ingress.rs:133` |
| R-081 | `bls/src/combined_scheme.rs:80-91` | верна | — |
| R-082 | `beacon/plane.rs:491` | верна | — |
| R-083 | `bls/src/keys.rs:109-119` | верна | — |
| R-083 | `bls/src/keystore.rs:222-235` | верна | — |
| R-084 | `staking-reader/src/reader.rs:149` | верна | — |
| R-084 | `staking-reader/src/reader.rs:210` | верна | — |
| R-084 | `staking-reader/src/reader.rs:434` | верна | — |
| R-085 | `beacon/metrics.rs:174-218` | верна | — |
| R-086 | `carry.rs:227` | верна | — |
| R-086 | `carry.rs:205-217` | верна | — |
| R-087 | `beacon/ceremony.rs:372-386` | верна | — |
| R-088 | `beacon/seed_journal.rs:445-452` | код не тот | `beacon/seed_journal.rs:181-190` |
| R-089 | `beacon/plane.rs:753-762` | верна | — |
| R-090 | `beacon/follower.rs:513-518` | верна | — |
| R-091 | `dkg_oracle.rs:149-152` | верна | — |
| R-091 | `share_state.rs:545-551` | верна | — |
| R-091 | `outcome.rs:79-95` | верна | — |
| R-091 | `seed.rs:57-58` | верна | — |
| R-091 | `keys.rs:104-106` | верна | — |
| R-091 | `actor.rs:2652-2654` | верна | — |
| R-091 | `actor.rs:3334-3337` | верна | — |
| R-092 | `dkg_agree.rs:3175-3178` | верна | — |
| R-092 | `dkg_engine.rs:935-938` | верна | — |
| R-093 | `epoch_manager.rs:1069-1070` | верна | — |
| R-093 | `outer.rs:351-359` | верна | — |
| R-093 | `outer.rs:349-358` | верна | — |
| R-094 | `application.rs:947-948` | верна | — |
| R-094 | `application.rs:735` | верна | — |
| R-095 | `cert_inlet.rs:3284-3288` | верна | — |
| R-095 | `cert_inlet.rs:2974-2989` | верна | — |
| R-096 | `epoch_manager.rs:908-914` | верна | — |
| R-096 | `cert_inlet.rs:3259-3268` | верна | — |
| R-097 | `order_block.rs:353-359` | верна | — |
| R-098 | `cert_inlet.rs:3102` | верна | — |
| R-098 | `cert_inlet.rs:3286` | верна | — |
| R-098 | `cert_inlet.rs:3291` | верна | — |
| R-098 | `cert_inlet.rs:3295` | верна | — |
| R-098 | `plane_upstream.rs:206` | верна | — |
| R-098 | `plane_upstream.rs:273` | верна | — |
| R-098 | `plane_upstream.rs:291` | верна | — |
| R-098 | `outer.rs:188-189` | верна | — |
| R-099 | `bls/src/secret_store.rs:79-82` | верна | — |
| R-100 | `extra_data.rs:123` | верна | — |
| R-100 | `application.rs:227-235` | верна | — |

## Замечание по существу находки

R-088: `append` возвращает `rolled = false` для раунда старой эпохи, то есть prune после такой записи не вызывается вовсе — blob, пересозданный ниже floor, удалит только следующий подъём эпохи. На вердикт NIT это не влияет, но формулировка «следующий prune его удаляет» точнее звучала бы как «удалит следующий prune, а он запускается только при подъёме эпохи». Находку не пересматривал.
