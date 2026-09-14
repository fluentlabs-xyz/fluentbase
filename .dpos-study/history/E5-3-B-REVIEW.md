# Финальное ревью захода Б строки 5.3 — идентичность DKG-лога `(dealer, hash)`

Ревьюер: Opus 5, свежий контекст, 2026-09-14. База `HEAD = 18573e76`, объект — незакоммиченное дерево (13 файлов под `crates/dpos/consensus/src/` + доки `.claude/dpos_architecture/`). md5 на входе сверены: `actor.rs 47760d6e…`, `artifact.rs 231da641…`, `ceremony.rs e74979f2…`, `log_resolver.rs e119b3a6…` — совпали. На выходе (после всех мутаций и откатов) те же значения, `git status` — те же 13 файлов + два файла оркестратора в `.dpos-study/history/`. Агенты не запускались, git только на чтение. Пути без префикса — `crates/dpos/consensus/src/beacon/`.

Постановка: `history/E5-prompts/5.3-B-review-final.md`. Журналы/dsh-отчёты читал только чтобы не повторить чужие мутации; свидетельство ниже — код и мои прогоны.

## §0 — прямые ответы

### 0.1 Ворота (мои прогоны, verbatim; скрипт `scratchpad/rev/run.sh base`, `CARGO_BUILD_JOBS=6`, после `DONE` в `gates/m1-status.txt`)

| Ворота | Результат |
|---|---|
| `cargo test -p fluentbase-consensus --lib` | `test result: ok. 681 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 178.56s` |
| `--lib --features dpos-devnet-byzantine testbed::` | `test result: ok. 56 passed; 0 failed; 0 ignored; 0 measured; 634 filtered out; finished in 230.51s` |
| `--lib testbed::` | `test result: ok. 47 passed; 0 failed; 0 ignored; 0 measured; 634 filtered out; finished in 179.38s` |
| `cargo test -p fluentbase-node --lib` | `test result: ok. 57 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 55.75s` |
| `cargo test -p fluentbase-staking-reader` | `test result: ok. 64 passed; 0 failed; …` + doc-tests `0 passed; 0 failed; 1 ignored` |
| `cargo test -p fluentbase-consensus --test slasher_integration` | `test result: ok. 16 passed; 0 failed; …` |
| `cargo clippy -p fluentbase-consensus -p fluentbase-node -p fluentbase-staking-reader --all-targets` | exit 0; ровно два чужих предупреждения: `staking-reader/src/epoch_transition.rs:3017:17` (MutexGuard across await), `node/src/dpos.rs:1989:1` (large size difference); своих 0 |
| `cargo clippy -p fluentbase-consensus --all-targets --features dpos-devnet-byzantine` | exit 0, 0 warnings |
| `cargo fmt --check` | exit 0 (stderr — только «unstable features» про `wrap_comments`/`comment_width`/`normalize_comments`, диффа нет) |
| `cargo doc -p fluentbase-consensus --no-deps` | exit 0, `grep -c "unresolved link"` = 6 |
| `(cd devnet/local-dpos-smoke && make harness-test)` | `2 failed, 2086 passed, 10 skipped in 4.94s` — те же две: `test_every_grepped_token_is_a_string_the_product_ACTUALLY_EMITS[EL-sync fast-forwarded the anchor]`, `test_the_two_jump_gates_are_the_values_the_product_uses` |

[KNOWN] Все одиннадцать совпадают с базой исполнителя (681/56/47/57/64/16, два чужих clippy, 0 с фичей, fmt 0, doc 6, harness 2/2086/10).

### 0.2 Безопасность ключа

[KNOWN] Пути, где лог выбирается по дилеру без хэша, есть ровно три, и ни один не влияет на то, над чем финализируется share:

- `first_log` (`ceremony.rs:210`, пишется только в `insert_log:531`) — читается `signed_log_hash:1109-1111` (индекс `recorded_dkg_logs` → предложение/`ShareConfirm`), `own_log_recorded:798-800` (гейт «свой лог записан») и resume-ребродкаст своего лога (`:1064-1068`). Это то, что узел *заявляет*, не то, над чем он финализирует.
- `Player::resume`'s `log_map` (`ceremony.rs:951, :976, :995`, `entry().or_insert` — первое записанное тело дилера). [KNOWN] По коду commonware `dkg.rs:1726-1758` `logs` используется в `Player::resume` ТОЛЬКО для проверки целостности (`MissingPlayerDealing`: лог с моим валидным ack, для которого нет `ReceivedDealing`); `view` из логов не строится. На share не влияет — только на Err/Ok резюма. Это записанный остаток D-10, запись верна.
- `publish_recorded_logs` (`actor.rs:1510-1528`): `signed_log_hash` + `Entry::Vacant` first-wins по seat — индекс для предложения, не для финализации.

[KNOWN] Всё, что доходит до `Player::finalize`/`observe`, идёт через `scoped_pinned_logs` (`ceremony.rs:1135-1175`), где lookup — точный `signed_logs.get(&(committee[idx], hash))` (`:1146`), а `finalize_over_pinned:1268-1276` и `derive_pinned:1223-1240` берут только его результат. `recompute_scoped:1300-1335` фильтрует тела по `pinned.get(&pk) == Some(&log_hash(&signed))` (`:1326`) из `logs_in` (обе половины пары включены, но берётся только пришпиленная). `checked_serve_map:318-334` — только раздача, keyed by `LogId`. Честный узел share над непришпиленным логом финализировать не может: набор — функция агреед-набора и точных хэшей. Подтверждено мутацией MUT-2 (§0.7): при by-dealer lookup оба стенда краснеют.

[KNOWN, вне рамки, до изменения так же — `git show HEAD:…actor.rs:1697`] На live-пути `adopt_share(e, &committee, out, share)` (`actor.rs:1783`) самопроверка `validate_share_on_poly` идёт против ЛОКАЛЬНО вычисленного `out`, а не против `group_key` артефакта; heal-путь (`:2719-2720`) — против пришпиленного `Output`. Пока `scoped_pinned_logs` точный, разницы нет; но под MUT-2 жертва приняла share над непришпиленным набором с `dkg_ceremony_ok=1` и без demote — второй заслонки нет. См. F-02.

### 0.3 Бан

[KNOWN] Что отвергается: только gossip-`Reveal` дилера, у которого уже есть пара (`ceremony.rs:504-506`: `Some((pk, _)) if self.equivocations.contains_key(&pk) => Step::default()`), `pk` — из `check`, не отправитель. Что принимается: `ingest_signed_log:842-857` (целевой фетч по `(dealer, hash)` — бана нет, есть проверка `pk == expected.0 && hash == expected.1`), replay (`resume` через `insert_log`), heal-путь `ingest_recompute_log` (`actor.rs:2895-2933`, точный хэш, бана нет), раздача `serve_log:2801-2811` (по точному id, `equivocations` не читает).

[KNOWN] Один поддельный лог бан запустить не может: в `signed_logs` попадает только то, что прошло `check` (`insert_log` вызывается из `handle:505-506`, `ingest_signed_log:853`, `seal_dealings:714`, `resume` через `checked_logs_in`); пара требует двух записей под одним `pk` с разными `log_hash` (`insert_log:533-545`). Подпись — ed25519-consensus (commonware `ed25519/scheme.rs:15,153`), неканонический `S` отвергается; хэш считается по `signed.encode()` (`log_hash:66-68`), т.е. по каноническим байтам, а не по wire-байтам — переупаковка третьей стороной второй хэш не даёт.

[KNOWN] Пришпиленный лог банённого дилера доходит до жертвы: `fetch_missing_logs:2271-2330` спрашивает `pinned − holds` по хэшу → `ingest_log:2835` → `ingest_signed_log` (без бана); `dkg_agree::fetch_bodies:1382-1410` — по хэшу предложения → тот же `ingest_log`. Единственный закрытый путь — gossip-ретрансляция, но он и не адресный.

Мутация MUT-1 (бан распространён на `ingest_signed_log`; md5 `ceremony.rs` `e74979f2…` → `f20f42e6…` → откат `e74979f2…`): `a_dealers_second_valid_log_is_evidence_and_a_ban_but_never_a_replacement … FAILED` (`ceremony.rs:2487: assertion failed: ok`); `a_held_body_that_is_not_the_pinned_one_… ok`; ОБА стенд-теста (`a_two_log_dealers_victim_…`, `a_two_log_dealer_that_also_…`) — `ok`. Постановка ждала красного стенда; стенд зелёный, потому что жертва в нём банит дилера только НА фетче (пара создаётся доставкой пришпиленного тела), а не до него. Свойство «бан не закрывает целевой фетч» держит только юнит церемонии. См. F-03.

### 0.4 Журнал

[KNOWN] Пара — один рекорд `JournalRecord::DealerEquivocation(first, second)` (`share_state.rs:390`, тег 4 `:397`, кодек `:497-503` — два `DealerReveal::read_cfg`, ничего не проверяет). Пишется ТОЛЬКО для второго тела на live-путях (`record_checked_log:575-579`), для третьего и далее — `PeerLog` (`:572-574`). Whole-or-nothing — `checked_logs_in:278-315`: обе половины должны `check`-иться как ОДИН дилер под РАЗНЫМИ хэшами, иначе WARN и пусто; одна функция для `checked_serve_map:329-331` и `resume:948-999` (три arm'а). Юнит `the_serve_map_takes_a_journaled_pair_under_the_rule_the_resume_applies … ok` (мой прогон lib) — четыре формы, serve-map == resume.

[KNOWN] Журнал старого формата (второе тело как `PeerLog` без пары): `resume` arm `PeerLog:993-998` → `insert_log` → `first_log[pk]` уже есть, первое тело в `signed_logs` ⇒ `Inserted::Equivocation`, пара и бан восстанавливаются тем же правилом. Crash между `PeerLog(first)` и парой: пара содержит `first` (`journal_record_for:588-600`, `record_checked_log:575-579`), replay: `PeerLog(first)` → First; пара → Duplicate + Equivocation. Если `PeerLog(first)` не лёг, а пара легла — пара сама даёт `first_log = first`. Порядок ретрая `nondurable_logs` (BTreeSet по `(pk, hash)`, `retry_nondurable_journals:1456-1485`) на результат не влияет: носитель порядка — сама пара. Replay ≡ live: `a_replayed_journal_rebuilds_the_recorded_set_and_the_equivocation_pair … ok` (мой прогон lib; `signed_logs.keys()` до == после, `first_log` ==, пара ==, бан после рестарта).

### 0.5 Wire

[KNOWN] `DkgLogKey{epoch, dealer, hash}` (`log_resolver.rs:61-65`): `write` = `u64 ‖ PeerPubkey(32) ‖ put_slice(hash)` (`:84-88`), `encode_size` = 8+32+32 (`:92-94`), `read` = `u64 ‖ PeerPubkey ‖ <[u8;32]>::read` (`:100-109`). `BeaconFetchKey::Log(Box<DkgLogKey>)` (`:133`), тег `TAG_LOG` первым (`:227`). Тотальность декода на коротком входе — по кодеку: commonware `codec/src/types/primitives.rs:158-167` `impl Read for [u8; N]` → `at_least(buf, N)?` → `util.rs:8-14` `Err(EndOfBuffer)` при `remaining < len`; паники нет. Юнит `dkg_log_key_round_trips_and_orders_by_epoch_then_dealer … ok` пинит 72/73 байта, раскладку, `decode(&bytes[..71]).is_err()` (`:624, :639, :651`). Отдельную мутацию не ставил — кодек прочитан.

### 0.6 `ArtifactPull`

[KNOWN] `minter_to_ask` (`artifact.rs:1725-1750`): `others = committee.bimap \ me`; пустой `others` → `None` (`:1732`); нечитаемый комитет → `None` (`:1726`) → нетаргетный `fetch` (`:1796`); не-член — никого не пропускает, ×n; индекс `others[cursor % len]`, `cursor.wrapping_add(1)` (`:1746-1747`). `forget` на трёх выходах: стор до throttle (`:1773`), стор после throttle (`:1781`, E-07), ответ `Have` (`:1804`). Рост `slots`: `throttle:1856` `retain(next_allowed + PULL_TIMEOUT > now)` — один map, одно правило; `minter_to_ask` вставляет слот только если `throttle` его не создал (в `pull` throttle всегда раньше). Юнит `repeated_pulls_are_rate_bounded_per_epoch … ok` (ротация, wrap, `me` пропущен, слоты 2→1→0, стор после throttle).

Причинная цепочка C7 — мой прогон. MUT-4: `pull` всегда `resolver.fetch(key)` (нетаргетный), `minter_to_ask` вызывается и игнорируется; md5 `artifact.rs` `231da641…` → `94613fed…` → откат `231da641…`. `a_zero_overlap_boundary_is_crossed_by_acquiring_the_other_halfs_key … FAILED` (`tests.rs:2410`), `(C7) heights=[95, 95, 95, 95, 400, 400, 400, 400] artifacts=[[2],[2],[2],[2],[2,3],…]` — уходящая половина не получает артефакт 3, `latest_calls: 498`. [KNOWN] На ЭТОМ дереве таргетный round-robin — несущая часть C7. [LIKELY] На HEAD тест проходил при нетаргетном фетче за счёт побочного эффекта pre-agreement фетча логов (по журналу и по доке `artifact.rs:1711-1724`); сам HEAD не собирал — git только на чтение, worktree создать нельзя.

### 0.7 Приёмка

[KNOWN] Оба стенда зелёные в моём прогоне (`base-standf.txt`). Витнессы: `reveals_swapped >= 1`, `reveals_seen == reveals_swapped` (оба теста, `tests.rs:3134-3138`, `:3335`), `log1_hash != log2_hash`, `both_logs_check` (`byzantine_roles.rs:207-229` — оба лога проходят `check` получателя ДО подмены), `claimed(0) == log2_hash` — `ShareConfirm` жертвы называет поддельный хэш (`:3174-3179`), `[1,0,0,0]` по `dpos_dkg_dealer_equivocation_total` (`:3225-3231`, `:3387-3393`), WARN с `first=h2 second=h1 evidence="journaled"` (`:3232-3248`). Вердикт до/после для второго: под MUT-2 он же печатает `(b1) … heights=[63, 63, 63, 63]`, на дереве — `(b2) … heights=[72, 72, 72, 72]` (видно под MUT-3, где меняется только счётчик).

Мои мутации (не совпадают с M1/M2/M2′/M3/M3′, D-07, E-02, E-07):

- **MUT-2** — `scoped_pinned_logs` берёт лог по ДИЛЕРУ (первое тело) вместо точного `(dealer, hash)` (`ceremony.rs:1146`); md5 `ceremony.rs` `e74979f2…` → `f42c6852…` → откат `e74979f2…`. Юниты: `a_dealers_second_valid_log_… FAILED` (`:2526`), `a_held_body_… FAILED` (`actor.rs:4953`). Стенд 1: `FAILED` на `tests.rs:3230` — при этом строка вердикта: `branch = (a) the victim … kept its share | heights=[72;4] equivocations=[0,0,0,0]`: жертва ПРИНЯЛА share над непришпиленным набором (`ok=1`, без demote), красным тест сделал только счётчик эквивокации (фетч по хэшу дошёл после финализации → `ingest_log` без живой церемонии → «moot»). Стенд 2: `FAILED` на `:3365` — `(b1) REPRODUCED … heights=[63;4] … victim ok=Some(1.0)`: share жертвы на чужом полиноме ⇒ её партиалы никто не принимает ⇒ 2 < quorum(4), цепь стоит. Вывод: тест 1 при сломанном свойстве держится на витнессе-счётчике, не на `victim_minted` (F-02); тест 2 держится на арифметике кворума.
- **MUT-3** — на пути фетча (`actor.rs:2864`) `note_equivocation` не вызывается; md5 `actor.rs` `47760d6e…` → `67c767eb…` → откат `47760d6e…`. `a_held_body_… FAILED` (`actor.rs:5002`), стенд 1 `FAILED` (`tests.rs:3230`), стенд 2 `FAILED` (`tests.rs:3392`) — оба на `[1,0,0,0]`. Витнесс улики живой в обоих стендах.
- MUT-1 (бан на фетч) и MUT-4 (C7) — выше.

Может ли стенд быть зелёным при сломанном свойстве? [KNOWN] Да для одного свойства: «бан не закрывает пришпиленный фетч» (MUT-1) — стенд не ставит бан ДО фетча; держит юнит. Для «выбор по хэшу» и «улика считается» — нет (MUT-2, MUT-3 красные).

### 0.8 Доки

[KNOWN] `00_preamble.md:6-60` (verified-against 2026-09-14), `13:543-600` (S2), `12:62-63` (тег 4, 72-байтный ключ), `15:422-445` (две строки инвертированы), `08:1609, :1657, :2256, :2286, :2571` — соответствуют коду. Устаревшее осталось в `08`: `:1231` «the resolver fetches per-DEALER»; `:1741-1746` «`committee[E] \ recorded_dealers`», «one unsatisfiable `{e,me}` fetch», «pinned `dealers()` logs off `recompute_pending`» — на дереве `recorded_dealers` нет, фетч — `pinned − holds` по хэшу и только при артефакте, `want` — `LogId`; `:1930` «`Step::recorded_dealer`» — символ переименован в `recorded_log` (`ceremony.rs:109`). Внутреннее противоречие: `00_preamble.md:16-19` «a journal without the record holds neither body nor ban» против `13:566-568` «two plain `PeerLog`s of one dealer prove the pair again» и `share_state.rs:378-384` — верно второе (§0.4). В доккомментариях кода старых формулировок не нашёл (`git grep` по `recorded_dealer`, `{epoch, dealer}` без hash, «by dealer» — только доки). См. F-01.

### 0.9 Граница и гигиена

[KNOWN] `git status`: ровно 13 файлов под `crates/` + `.dpos-study/history/E4-ORCHESTRATOR.md`, `E5-ORCHESTRATOR.md` (не код). `#[allow]` в диффе: 0. Новых `unwrap`/`expect`/`panic!` в добавленных строках вне `mod tests`/`mod clock_tests` (`actor.rs:2946+`, `ceremony.rs:1352+`, `artifact.rs:1897+`) — нет; в прод-коде только `unwrap_or_else(PoisonError::into_inner)`. Новых `pub` наружу нет: все подмодули `beacon/*` приватные (`beacon/mod.rs:68-93`), реэкспорт только `plane::{build, …}`; `pub(crate) LogId/log_hash/DealerEquivocation`, новые `pub fn` на `pub(crate) DkgCeremony`. Мёртвого кода после удаления `Logs`-поля/`recorded` нет: clippy 0 своих; `recorded_log_count` под `#[cfg(test)]` (`ceremony.rs:757`); `Logs` по-прежнему строится в `scoped_pinned_logs:1140`.

### 0.10 Hard-stop

[KNOWN] `DECISIONS.md`: Д-6 (defer, `:39-43`) соблюдён — на цепь ничего не идёт, только пара в журнале/RAM, WARN, счётчик; П-3 — артефакт остаётся единственным фактом о ключе, `agreed_pinned` = набор артефакта, удержание после отказа adopt (`actor.rs:1784-1793`) — тот же набор, не второй источник; Д-1 не затронут. Менять решения не нужно. BLOCKER-ов нет.

### 0.11 Вердикт

**КОММИТИТЬ** — после правки трёх устаревших мест в `08_node_integration…md` (F-01, доки, минуты; по правилу проекта док-дрифт чинится в том же изменении). В коде блокеров нет: share только над точным пришпиленным набором, пришпиливание не тронуто, декод тотален, живость DKG на стендах и C7 подтверждена.

### 0.12 Где проверка слабее всего (по убыванию)

1. HEAD не собирал: «на HEAD C7 держался на pre-agreement фетче» — [LIKELY] по доке/журналу, не мой прогон.
2. Heal-путь с эквивокацией (второе тело только в журнале, пришпиленное — через `ingest_recompute_log`) не гонял — по коду там нет ни счётчика, ни WARN (F-05).
3. Потеря артефакта на `adopt.try_send` (`artifact.rs:957-958`) → `drive_recompute` без `agreed_pinned` — рассуждение по коду, без прогона (F-04).
4. Wire — без мутации, по кодеку и юниту.
5. Только стенд, без живого devnet.

## §1 Находки

| id | серьёзность | file:lines | что не так | чем пытался опровергнуть | уверенность |
|---|---|---|---|---|---|
| F-01 | SERIOUS (док-дрифт; по `.claude/CLAUDE.md` — блокер ревью, чинится в том же изменении) | `.claude/dpos_architecture/08_node_integration_crates_node_bins_fluent.md:1231, :1741-1746, :1930`; `00_preamble.md:16-19` | Три места описывают по-дилерный фетч как текущий (`per-DEALER`, `committee[E] \ recorded_dealers`, `{e,me}` fetch, `pinned dealers() logs`) и ссылаются на удалённый символ `Step::recorded_dealer`; преамбула утверждает «journal without the record holds neither body nor ban», что противоречит `13:566-568` и коду (`resume` собирает пару из двух `PeerLog`) | `git grep` по старым формулировкам во всех секциях; сверил с `actor.rs:2293-2330`, `ceremony.rs:109, :993-998` | [KNOWN] |
| F-02 | SERIOUS (вне рамки идентичности, ДО изменения так же — `HEAD:actor.rs:1697`) | `actor.rs:1783` vs `:2719-2720` | Live-путь `adopt_share` самопроверяет share против ЛОКАЛЬНО вычисленного `out`, не против `group_key` артефакта; heal-путь — против пришпиленного `Output`. Сегодня недостижимо (`scoped_pinned_logs` точный), но заслонки нет: под MUT-2 жертва приняла share над непришпиленным набором с `ok=1`, без demote, без ERROR, и её партиалы молча отвергаются (стенд 2 → `[63;4]`); стенд 1 красный только по счётчику улики | MUT-2 (§0.7); чтение `adopt_share:1080-1144`, `drive_finalization:1690-1793` | [KNOWN] |
| F-03 | MODERATE | `testbed/tests.rs:3127-3255`, `ceremony.rs:2478-2487` | Приёмочный стенд не покрывает «бан не закрывает пришпиленный фетч»: жертва банит дилера только на фетче, порядок «бан → фетч» стенд не проходит; свойство держит один юнит церемонии | MUT-1: стенды `ok`, юнит `FAILED` `ceremony.rs:2487` | [KNOWN] |
| F-04 | MINOR | `actor.rs:2493-2495`, `artifact.rs:957-958` | Heal теперь требует `agreed_pinned`, т.е. доставки артефакта в актор; для артефакта, пришедшего pull-ом, единственный путь — `adopt.try_send` (канал 16, `plane.rs:732`); при потере `drive_recompute` пропускает эпоху каждый тик (`outcome_at` есть, `pull` не переспрашивается) до рестарта (`restart_replay:1287-1298`). До изменения heal брал scope из `outcome.dealers()` без этой зависимости | Прочитал `on_artifact:938-975`, `spawn_write_back` (`plane.rs:486` — `send().await`), `drive_recompute:2474-2495`; практически 16 недоставленных артефактов недостижимы; не гонял | [LIKELY] |
| F-05 | MINOR | `actor.rs:2895-2933` | На heal-пути (`ingest_recompute_log`) эквивокация, доказанная приходом пришпиленного тела к журналу с другим телом, не считается и не логируется (`note_equivocation` не вызывается, пары в `DkgActor::equivocations` нет); диагностика только | Чтение; на стенде heal-путь не задействован | [KNOWN] |
| F-06 | NIT | `actor.rs:1510-1516`, `ceremony.rs:575-579` | Если `PeerLog(first)` не лёг, а пара `(first, second)` легла, seat не публикуется, хотя `first` уже durable внутри пары; лечится ретраем `PeerLog(first)` (избыточной записью). Консервативно, не ошибка | Чтение `publish_recorded_logs`, `retry_nondurable_journals` | [KNOWN] |
| F-07 | NIT | `ceremony.rs:547` | Arm `_ => Inserted::Another` покрывает и «первое тело не в `signed_logs`» (после `take_signed_logs` — но церемония тогда уже вне map), и «уже эквивокатор»; комментарий второе называет, первое нет | Чтение `take_signed_logs:818`, `drive_finalization:1770-1778` | [KNOWN] |

## §2 Оставить как есть

- Снятие pre-agreement фетча логов (`fetch_missing_logs` только при `agreed_pinned`, `actor.rs:2293-2295`): до артефакта тела предложения тянет `dkg_agree::fetch_bodies` по хэшу предложения (`dkg_agree.rs:1382-1410`), после — `pinned − holds`; шорт-хэндед узел не подтверждает ниже кворума, но паркуется на `Missing` и лечится. Живость на стендах (47+56) и C7 подтверждена.
- `agreed_pinned` удерживается после отказа adopt (`actor.rs:1784-1793`) — heal должен уметь назвать тело; удаляется на adopt или на sweep (`:1222`).
- `first_log` как источник `signed_log_hash` + `Entry::Vacant` в `publish_recorded_logs` — двойная страховка first-wins per seat, на которой стоит `Confirmations`/widest-wins (`confirmations.rs:16-20`, `dkg_agree.rs:375-379`).
- `ingest_signed_log` без бана и `serve_log` без бана — единственный путь агреед-тела банённого дилера.
- Пара только для второго тела, третье и далее — `PeerLog`: улика — первая пара, replay даёт ту же пару (`insert_log:533-545`).
- `DealerEquivocation` в `recorded_a_log` (`ceremony.rs:133-141`) — второе тело может завершить пришпиленный набор.
- `pinned_by_dealer` (`actor.rs:261-270`) с тем же детерминированным skip немаппируемого idx, что и `scoped_pinned_logs`.
- Отклонение (1) — переписанный вердикт второго стенда: арифметика `n − 1 = 3 = quorum(4)` верна, витнесс `withhold_probe`/`schemes_withheld` сохранён; под MUT-2 тот же тест возвращается к `[63;4]`, т.е. вердикт привязан к share жертвы, а не к кворуму с участием молчащего.
- Отклонение (2) — `minter_to_ask`: реализация корректна (§0.6), причинная цепочка на дереве подтверждена MUT-4.
- Записанные остатки D-09, D-10, D-11, D-18, E-04, E-09, E-10, E-14 — записи верны в части, которую проверял (D-09: MUT-1 показал, что стенд лечится не одним `fetch_missing_logs`; D-10: `Player::resume` логи не использует для `view`).

## §3 Вне рамок

- Дилер с двумя РАЗНЫМИ dealings (не логами): игрок, чей `view[d]` построен от dealing A, а пришпилен лог с commitment B, получает share не на полиноме → отказ adopt → heal с тем же `view` → тот же отказ → узел без share; идентичность лога это не закрывает — свойство DKG-протокола, не этого изменения.
- F-02 как отдельный тикет: гейт live-adopt против `group_key` артефакта (П-3 «`validate_share_on_poly` перед любым `adopt_share` против пришпиленного `Output`» на live-пути выполняется только транзитивно).

## md5 на выходе

`actor.rs 47760d6eb3e8945eaa529692190325a0`, `artifact.rs 231da6415a91c9306009cbc5eb7d383d`, `ceremony.rs e74979f25c154c6b780e9aa6023e113b`, `log_resolver.rs e119b3a6cfad99af01d05e36b6efb44b` — как на входе; каждая мутация откатана копией из `scratchpad/rev/*.orig` с проверкой md5.
