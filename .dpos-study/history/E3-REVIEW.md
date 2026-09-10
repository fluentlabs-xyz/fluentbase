<!-- Оценка закрытого этапа Э3 (стенд). Дерево /home/djadjka/Work/fluentbase, ветка djadjka/dpos-reth-2.2-squashed, HEAD 26d291b7. Написано 2026-09-10 одной сессией Fable 5.1; всё, что помечено [KNOWN], открыто или запущено в этой сессии. -->

# Э3 — оценка закрытия: стенд как регрессия для Э4/Э5

## 0. Состояние дерева и прогоны

- [KNOWN] Четырёх коммитов из `E3-CLOSEOUT.md` §7 (`refactor(staking-abi)`, `test(devnet)`, `test(consensus)`, `docs(dpos)`) в истории нет. `git log --oneline -8`: HEAD `26d291b7 test(devnet): bring the smoke harness up…`, ниже `38c546d4 docs(dpos): journal the first byzantine-roles session`, `3e3e9ff9 test(consensus): apply the counter-review findings…` — это коммиты сессии A и коммит пользователя, а не закрытия. Работа закрытия лежит в дереве: `git diff --stat HEAD -- . ':!.dpos-study'` — 27 файлов, +3612/−542; плюс правки четырёх файлов `.dpos-study/`.
- [KNOWN] `cargo test -p fluentbase-consensus --features dpos-devnet-byzantine --lib testbed -- --nocapture --test-threads=1` — 35 passed, 0 failed, 0 ignored, 192,93 с.
- [KNOWN] `cargo test -p fluentbase-consensus` (без фичи) — 636/0 lib; 3/0; 5/0 + 1 ignored; 13/0; doc-тест 0 passed + 1 ignored. Совпадает со строкой ворот `PLAN.md` §1 («636 + 3 + 5 + 13 / 0, 2 ignored»).
- [KNOWN] `cargo clippy -p fluentbase-consensus --all-targets --features dpos-devnet-byzantine -- -D warnings` — чисто. Без фичи не запускал.
- [KNOWN] `python3 devnet/local-dpos-smoke/scripts/xp/agreement_check.py` — 15 checks, 0 disagree, 0 unread; G3 «24 shared declarations»; G16 «17 shared + 16 contract-only … across 113 sites (54 more admitted…)».
- Длинные строки читал целиком: `sed -n A,Bp` и Read tool; никаких `cut -c`/`head -c`.

## 1. Карта фейков

Каждый шов: что заменяет, где обе стороны, известное расхождение, какие тесты опираются на него в ассертах. Нумерация тестов — по `testbed/tests.rs` (35 функций с `#[test]`; ярлыки (1), (3b), B2, C7, R-002/a… — из doc-комментариев тестов).

| Шов | Стенд (file:line) | Прод (file:line) | Известное расхождение | Тесты, у которых ассерт читает этот шов |
|---|---|---|---|---|
| `FakeChain` + `FakeDeriver` + `FakeBeacon` (EL) | `fakes.rs:324-347` (дерево + канон), `:405-448` `canonicalize`, `:464-486` `land_canonical`, `:517-549` `land_jump`, `:612-683` `ExecutedChain`, `:696-744` deriver, `:761-793` engine | `ProviderExecutedChain` `node/src/ordering.rs:37-64` (`last_block_number`, `block_hash`, `FinalizedCursor`); `RethImporter` `node/src/importer.rs:72-134` (`InsertExecutedBlock`, FCU через `ConsensusEngineHandle`); `RethBlockDeriver` `node/src/derive.rs:98-140` | [KNOWN] Объявлено блоком «Divergences from reth» `fakes.rs:207-322`: нет INVALID, нет backfill-SYNCING, safe/finalized не читаются, нет eager `MakeCanonical`, нет engine-fatal `Err`, нет лага by-hash индекса, `executed_tip` = канон-tip вместо `last_block_number`, нет персистенции. [KNOWN] Не объявлено: док-комментарий `fakes.rs:203-205` («FCU на предка сбрасывает суффикс, `retain(h <= head_h)`») противоречит коду `fakes.rs:411-413` — голова, уже каноническая на своей высоте, возвращает `true` без `retain`; `retain` выполняется только когда голова НЕ каноническая (`:435`). Код совпадает с reth (память проекта: reth пропускает backward-FCU), док — нет. [KNOWN] Блоки без тел и состояния: хэш = `keccak(digest ‖ prev_randao)`, `sealed_at` `fakes.rs:58-72` | Все, кто читает `heights`/`hashes`/`diverged`/`el_events`/`seeds`: (1), (2), (3), (3b), (4a)–(4d), (5), (5′), (6), (eq), (7), (7′), B1–B5, B4′, C2, C3, C4, C7, C8, C9, R-002/a, R-002/b, R-008, R-004, R-001, R-009, unit `land_jump` — 33 из 35. Не читают: (A) и C1 |
| `FakeStaking` | `fakes.rs:861-1026`; `committed_at` `:925-933`; `committee(epoch)` `:944-954`; `dkg_qual` `:960-978` | `RethStakingStateReader` `staking-reader/src/reader.rs:644-704` (снимок по хэшу), трейт-дефолт `scheduled_dpos_activation` `:779-781`, переопределение `:810-812` | [KNOWN] Состав комитета — чистая функция эпохи (`(self.members)(epoch)`, `:945`); `at_hash` влияет только на «закоммичен ли» (`:987-991`). `tombstoned: false` всегда (`:892`), веса `1` (`:1005`), реестр не мутирует (`:866-867`). `committed_at` считает эпоху с литералом `0` вместо `DPOS_ACTIVATION_BLOCK` (`:929`). `scheduled_dpos_activation` — трейт-дефолт `Ok(Some(0))` (`reader.rs:779-781`), настоящий ридер свернул бы `0` в `None` (`:806-812`) ⇒ арм `Intra` в `apply_at` (`epoch_transition.rs:453-455`) недостижим | Через `EpochTransition` (`stand.rs:1474-1488`) — все 33 многоузловых; напрямую в ассертах: C2 (`et_boundaries`), C3 (`staking_reads`), C4 (`et_steps`, `geometry`), C9/R-001 (`jump_committee_reads`), R-002/a (`committee_seats`), B2/C7/C8 (`artifacts` через расписание) |
| `ElNetwork` | `fakes.rs:107-135`; публикация в `advance_finalized` `:649-660`; чтение только в `land_jump` | reth devp2p backfill внутри `RethElSync::sync_to` `cold_start_jump.rs:442-613` | [KNOWN] Список хэшей без тел; `land_canonical` коммитит безусловно (`:464-486`), никакой загрузки/исполнения/вердикта. Публикуется tier-F (после `advance_finalized`), т. е. пир «знает» только финализированное | B2 (косвенно — B2 зелёный только с прыжком), C9, R-008, R-004, R-001, unit `land_jump` — 6 |
| `JumpElSync` | `fakes.rs:1209-1320` | `RethElSync` `cold_start_jump.rs:437-621`: `local_landing` `:423-431` (`best_block_number`), гейт «уже исполнено» `:455-465`, ожидание FCU `Valid` `:478-613`, `holds` `:615-620` | [KNOWN] `Unservable` → сон `EL_SYNC_STALL_ESCAPE` (300 вирт. с) → `StalledWithPeers` (`:1267-1274`); `ConflictingPrefix` → `SyncFailure::Invalid` с пометкой `[ГИПОТЕЗА]` прямо в коде (`:1280-1290`) — какой вердикт даст reth, не проверено; нет watchdog по `peer_count`, нет SYNCING-навсегда; `local_landing` читает канон-tip, прод — `best_block_number` (сопоставимо только пока канон == best) | Ассерт на `jump_calls`: C9, R-004, R-001 (3); на `rejump_calls`: B2 (`>= 1`), C8/4c/R-009 (`== 0`) |
| `JumpCommittees` | `fakes.rs:1113-1182` | `RethCommitteeSource` `cert_inlet.rs:236-262` (`scheme_at` → `build_at` над `epoch_committee_snapshot`) | [KNOWN] Форма вызова та же (`build_verifier` над `epoch_committee_from_snapshot`), но состав не зависит от `at_hash` (см. `FakeStaking`) — чтение «на хэше посадки» есть, семантики «состояние на этом хэше» нет | C9 (`reads.len()==calls.len()`, `at == landing_hash`), R-001 (`reads.any(at == result)`); R-004/B2/R-008 — не ассертят |
| Рандомность: `Beacon::Static` против `Beacon::Live` | `stand.rs:1781-1786` (`StaticRandomness::build` над `all_validators_snapshot`); `:1788-1909` (`beacon::build`) | `node/src/dpos.rs::build_beacon_plane` (`:1163`, по стендовому комментарию `stand.rs:1751-1755`; сам не открывал — [LIKELY]) | [KNOWN] `Static`: фиксированная раздача по всем узлам, σ на любую эпоху, DKG-плоскости нет — эпоха «beacon-INACTIVE» невозможна, R-002/R-008/DKG-дедлайны неприменимы. `Live`: настоящая плоскость, но `committee_for`/`dkg_qual_probe` идут в `FakeStaking` по хэшу `committee_read_hash` (`stand.rs:1764-1780`) — tier-F хэш, не header-проба прода (`node/src/dpos.rs:1396-1403`) ⇒ R-123 структурно невидим | Static: (1), (2), (3), (3b), (4a)–(4d), (5), (5′), (6), (eq), (7), (7′), R-009 — 15 (+ (A), C1, unit `land_jump` без стенда = 18). Live: B1, B2, B3, B4, B4′, B5, C2, C3, C4, C7, C8, C9, R-002/a, R-002/b, R-008, R-004, R-001 — 17. Итого 35 |
| Отсутствующий `CertInlet` | нет; `live_height` пишется в пробе ДО проверки (`stand.rs:1598-1610`, `:1639`); `upstream_frontier` пишет только проба (`executor.rs:1936-1938`); `ReJump.rotate: None` (`stand.rs:1746`) | `node/src/cert_inlet.rs:110-114` (`CertInlet::new(...).with_tee(tee).with_rotate(rotate)`); `cert_inlet.rs:607` `ingest`, `:621-651` бинд эпоха↔высота (только follower, `epoch_bind`), `:654-657` `upstream_frontier` до verify, `:693` `ensure_key(epoch, PinEffort::Local)`, `:866` `capture_certificate_seed`, `:896-898` `live_height` после verify, `:945-962` `record_data_fault` | [KNOWN] В стенде нет ни одного вызова `ensure_key(…, Local)` на пути сертификата (единственный прод-вызов — `cert_inlet.rs:693`; в `testbed/` его нет); `record_data_fault` не существует; ротация ненаблюдаема. Комментарий `stand.rs:1607-1609` «a Byzantine-upstream role (Э3.3) has to move this behind a verify before it means anything» — роли добавлены, tee не перенесён | R-008 (`deliveries_rejected`, путь через `UpstreamResolver::spawn_finalized` `cert_inlet.rs:3102`, не через `ingest`); R-004 (`peak`, `probe_calls`); R-009 (`rejump_calls == 0`); R-001; C9 |
| Захват логов на WARN | `capture.rs:50-52` (`<= Level::WARN`); исключение `SIMULATOR_ACK_DROP` `stand.rs:540-558` | tracing прода | [KNOWN] DEBUG-строки невидимы: отказ маршала по высоте (`CW:consensus/src/marshal/core/actor.rs:987-993` — `send_lossy(false)`, без лога), `stale delivery`, `frontier fetch delivered` (`plane_upstream.rs:298-302`). Один глобальный подписчик на процесс (`capture.rs:35-45`); `log_capture_live` может быть `false`, если другой модуль тестов установил подписчика раньше — только (3b) ассертит `live` (`tests.rs:329-332`), (3) делает лог-проверку условной (`:184-190`) | `errors()` пуст: (1), (2), B1, B2, B3, B4, C3, R-002/a, R-002/b, R-009 и др.; строки: (3) `SafetyHalt`, (3b) `guard #2 at` / `result divergence at height`, R-008 `quarantined seed does not verify`, B4 `re-agreeing` |
| `NoSink` слэшера + пустые тумбстоуны | `fakes.rs:1323-1333`; `stand.rs:1929` (`TombstoneSet::default()`), `:1963` (`NoSink`), `:1965` (`slasher_evidence: None`) | sink в `consensus/src/dpos.rs` (не открывал — [LIKELY]) | [KNOWN] Ни одна улика не доходит до «контракта»; `FakeStaking.tombstoned` всегда `false`; шаг 5b не сделан | Ни один тест не ассертит слэшинг; (eq) только печатает |
| `PeerSet` / `upstream_only_link` / `TrackSink` | `stand.rs:174-193`, `:752-835` (один симулированный Oracle на N узлов), `:889-892` (реестр пуст под `Committee*`), `:956-973` (`upstream_only_link`), `:1071-1130` (ручная резка линков) | authenticated transport рвёт соединение с неотслеживаемым пиром (комментарий `stand.rs:180-182`; сам транспорт не открывал — [LIKELY]); `EpochTransition::track_and_trigger` `epoch_transition.rs:615-660` | [KNOWN] Регистрацию эпохи получает только первый узел (`:813-826`); линки режутся руками по последнему `tracked`; `upstream_only_link` снимает только ИСХОДЯЩИЕ линки source/victim (`:961-973`: `remove_link(pks[a], pks[b])` для `a ∈ {source, victim}`), входящие `1→3`, `2→3` остаются — док `stand.rs:129-136` («every other upstream link touching either is removed») неточен; безвредно, пока резолвер отвечает только на запросы | (4b), (4c), (4d) (`heights == [16,16,16,15]`), C2 (`tracked_mismatches`, `tracked_forwarded`), R-009, R-004, R-001 (изоляция), R-008 (`CommitteeTrackedOnly`) |

### 1.1 «Divergences from reth» — что подтверждено, что записано с чужих слов

Проверял по `.claude/RETH_INTERNALS.md` и по pinned checkout `~/.cargo/git/checkouts/reth-9084db4313ec21c5/8cc96a3` (rev из `Cargo.lock:11124`, ветка `v2.2-patched-tree-escrow`, `8cc96a37`).

| Утверждение блока `fakes.rs:207-322` | RETH_INTERNALS | Pinned checkout | Статус |
|---|---|---|---|
| derive/import не канонизируют; уже известный по хэшу блок — тихий no-op (`f6fb181`, «§11») | `RETH_INTERNALS.md:210-213`, `:56-60`, `:116` | [KNOWN] `crates/engine/tree/src/tree/mod.rs:1551-1573`: `already_known = tree_state.contains_hash(hash) || (number <= last_persisted && sealed_header_by_hash(hash).is_some())` → `Continue`; блок кладётся в дерево, канон не трогается | подтверждено (якорь доки `:1551-1576` на три строки шире, чем гейт) |
| FCU на непривязанную голову → SYNCING (ветка 5, `handle_missing_block`) | `RETH_INTERNALS.md:194` | [KNOWN] `fn handle_missing_block` `mod.rs:1320`; `validate_forkchoice_state` `:1158`, гейт `!backfill_sync_state.is_idle()` `:1173` | место подтверждено, семантику ветки читал только в RETH_INTERNALS — [LIKELY] |
| `executed_tip` в проде = `last_block_number` (DB-only), `best_block_number` — другой ярус | `RETH_INTERNALS.md:262-264`, `:284` | [KNOWN] обе функции на `blockchain_provider.rs:254` и `:258`; тела не читал | [LIKELY] |
| «ACCEPTED never produced» | `RETH_INTERNALS.md:185` | не открывал | с чужих слов |
| INVALID: `InvalidHeaderCache`, эвикция после 128 попаданий | `RETH_INTERNALS.md:225` | не открывал | с чужих слов |
| `invalid_state` при неразрешимом safe/finalized | `RETH_INTERNALS.md:288-292`; `node/src/importer.rs:87-104` (обработка `ForkchoiceUpdateError` → `anchor_inconsistent`) [KNOWN] | не открывал | с чужих слов на стороне reth, подтверждено на стороне узла |
| eager `MakeCanonical` по sync-target, verdict (a) | `RETH_INTERNALS.md:200`, `:185` | не открывал | с чужих слов |
| «reth declares the branch canonical AND executed before `sync_to` returns» (`fakes.rs:452-453`, `:504-505`) | — (это `cold_start_jump.rs`, не reth) | [KNOWN] хвост цикла `cold_start_jump.rs:596-613`: после выхода из ожидания — `block_hash(landing)`; сам цикл ожидания `Valid` (`:478-595`) не читал | [LIKELY] |
| `ConflictingPrefix` → `Invalid` | — | — | сам код помечает `[ГИПОТЕЗА]` (`fakes.rs:1280-1285`); никем не проверено |

Итог: из 12 пунктов блока по pinned checkout подтверждён один (already-known гейт) и локализованы два (missing-block ветка, две функции провайдера); остальное — пересказ RETH_INTERNALS без перепроверки в этой работе. Это не ошибка блока (он честно даёт якоря), но «подтверждено чекаутом» про него сказать нельзя.

## 2. Что каждый тест «до правки» пинует на самом деле

Формат: (а) несущий ассерт и что он пропустил бы; (б) что выполняется и на честном прогоне; (в) какое наблюдение записи не показано и почему; (г) одна мутация прод-кода, которая должна уронить тест, и упадёт ли по чтению; (д) годится ли как тест «после». Числа — из моего прогона (`scratchpad/testbed_run.log`).

### R-001 — `a_lying_upstream_lands_a_divergent_branch_and_authentication_refuses_it` (`tests.rs:3159-3371`; роль `Role::LyingUpstream`, `ForgeMode::LyingLatest`)

- Прогон [KNOWN]: 41 вызов прыжка, все `AuthFailed`, `outcome_detail` «FAILED BLS verification against committee[4|5|6] read at the synced landing …»; `committee_reads[0]` — 41 запись на хэшах посадки; `heights=[95,194,194,194]`, `halted=[]`; контроль 2 вызова, `[192×4]`.
- (а) Несущие: `calls.all(outcome == "AuthFailed")` (`:3229-3232`); текст ошибки содержит «FAILED BLS verification against committee[» и не содержит «is unreadable» (`:3239-3252`); `consumed.result == divergent_hash(tip − K)` и `reads.any(at == result)` (`:3267-3283`); `canon_div` непуст (`:3302-3315`). Пропустили бы: проверку, ПРОХОДЯЩУЮ на подменённом состоянии (механизм записи) — недостижимо, потому что `FakeStaking::committee` не зависит от хэша (`fakes.rs:944-954`); отсутствие сверки `round.epoch()` с `epoch_of(block.height)` в `verify_jump_authenticated` (`cold_start_jump.rs:683-685` берёт эпоху из раунда и больше ничего) — нигде не ассертится.
- (б) На честном прогоне держатся: `control.all(Landed)`, `halted.is_empty()`, `diverged == None`, lockstep, `reads` непуст (честный прыжок тоже читает комитет).
- (в) Не показано: (а) записи — захват через подменённый комитет (фейк); (б) — `SafetyHalt` через K блоков (гейт отвергает раньше, но только для варианта Б; вариант А не ставился); (в) — снята Ex-2; посадка — модель `land_canonical` без тел (`fakes.rs:464-486`), а не reth.
- (г) Мутация: `cold_start_jump.rs:687-692` — заменить `ensure!(latest.finalization.verify(ctx, &scheme, &Sequential), …)` на `let _ = latest.finalization.verify(ctx, &scheme, &Sequential);`. По чтению: `verify_jump_authenticated` вернёт `Ok`, `cold_start_jump_with_threshold` вернёт `Landed` (`:842-871`), `calls.all(AuthFailed)` на `:3229` упадёт, контроль останется зелёным. Упадёт — [KNOWN] по коду.
- (д) После П-4 (аутентификация до `sync_to`, комитет из локального finalized): посадки не будет ⇒ `canon_div` пуст (`:3311` красный), `reads.any(at == result)` (`:3279`) красный — читаться будет локальный finalized-хэш; для эпох 5–6 локальное состояние узла 0 на 95 (эпоха 2) их не коммитит ⇒ сработает плечо «unreadable» и ассерт на текст (`:3244-3251`) тоже красный. Тест станет красным по правильной причине, но переписывать придётся блоки (3) и (4) целиком; несущей «после»-проверки (отказ без посадки, чтение на локальном finalized) в нём нет.
- Вердикт: частично (посадка модельная, проверка на подменённом состоянии недостижима) — совпадает со статусом `REGISTER.md` R-001 от 09-10. Переоценки нет.

### R-002 — `a_dealer_with_two_logs_leaves_the_addressed_victim_without_a_share` (`tests.rs:2295-2423`; `Role::TwoReveals { withhold_partials: false }`)

- Прогон [KNOWN]: ветка (b), `heights=[72×4]`, `log1=0x547383a1…`, `log2=0xe4698147…`.
- (а) Несущие: confirm жертвы называет `log2_hash` на месте дилера (`:2339-2345`), `victim_demoted` (`:2383-2389`), `dkg_ceremony_ok(0) == 0` (`:2400-2404`). Пропустили бы: что `recorded.insert(pk)` (`ceremony.rs:412`) вернул `true` на L2 и `false` на L1 — счётчика нет, это вывод; что refetch не пошёл (`actor.rs:1951 fetch_missing_logs`) — тоже вывод.
- (б) На честном: `halted`/`errors` пусты, lockstep, `ok == 1` и `demoted == 0` у 1–3, `seed_agreed_at` для 1–3, `assert_ne!(victim_minted, victim_demoted)` (на честном minted=true, demoted=false).
- (в) Не показано: f жертв (роль адресует одну); carry-forward эпохи (прогон до 72); «лидер пинует набор с H(L2)» — опровергнуто (пин — L1); «обнаружение нигде не выполняется» — только отсутствие ERROR при захвате на WARN.
- (г) Мутация: `ceremony.rs:628` `ingest_signed_log` — принимать валидный лог того же дилера с ДРУГИМ хэшем, если он совпадает с пиннутым (заменять запись, а не отбрасывать). По записи реестра сейчас там `(true, empty)`; тело функции я не читал — [ГИПОТЕЗА], что этой одной мутации хватит, чтобы жертва собрала share и `victim_demoted` на `:2383` стал красным. Мутация в `record_checked_log` (ключ по хэшу) сама по себе тест не уронит: L1 до жертвы по проводу вообще не доходит (`byzantine_roles.rs:305-315` шлёт L1 только `others`).
- (д) Как «после» для П-9: ветка (2) `assert!(victim_demoted)` и `ok(0) == 0` должны инвертироваться; блок (1) с `claimed(0) == log2_hash` при hash-идентичности может как сохраниться, так и нет (жертва записывает L2 первым в любом случае, но после refetch по пину индекс `recorded_dkg_logs` может назвать L1) — блок (1) придётся переписать.
- Вердикт: воспроизведено — совпадает с реестром; статус честно говорит «в прогоне пин — L1».

### R-002 — `a_two_log_dealer_that_also_withholds_its_partial_stops_the_chain_silently` (`tests.rs:2471-2547`; `withhold_partials: true`)

- Прогон [KNOWN]: `heights=[63×4]`, таймаут 200 вирт. с, `halted`/`errors` пусты.
- (а) Несущие: `withhold_probe == Some((true, false))` (`:2490-2495`), `!crossed` (`:2510-2517`), `heights == [63;4]`. Пропустили бы: что узел 1 действительно не отправил ни одного голоса эпохи 2 на `VOTE_CHANNEL` — проба проверяет схему, не провод (журнал сессии A §8 п. 2 это признаёт).
- (б) На честном: `halted`/`errors` пусты, `ok == 1` у 1–3. Ядро (`!crossed`) на честном не выполняется.
- (в) Не показано: раздельность порога seed и кворума голосов — невозможно любым прогоном (`combined_scheme.rs:286` — нет partial ⇒ нет голоса; `:387` порог = `M::quorum`); «n−1−f = t−1» реализуется как 2 < 3.
- (г) Мутации прод-кода, роняющей этот тест, не существует без изменения кворума: цепь не может пройти 64 с двумя голосующими из четырёх. Тест пинует арифметику BFT плюс раскол. Это нормально для «до»: красным его сделает только правка, возвращающая жертве share (П-9), — тогда 3 подписанта, `!crossed` на `:2510` падает.
- (д) Годится как «после» для П-9 с инверсией `!crossed` → `crossed`; текст ветки (b2) в доке придётся переименовать (сейчас (b2) описано как «голос без partial'а засчитан», после П-9 это будет штатная ветка).
- Вердикт: воспроизведено — совпадает с реестром.

### R-004 — `an_inflated_latest_probe_wedges_a_rotated_out_validator_in_re_jumps` (`tests.rs:2898-3080`; `Role::InflatedProbe`, `ForgeMode::InflatedLatest`)

- Прогон [KNOWN]: `peaked=1000194`, 14 вызовов все `Stalled`, `probes[0]=437`, `heights=[95,194,194,194]`; контроль 2 вызова, `probes=293`, `[192×4]`.
- (а) Несущие: `Some(peaked) == byz.inflate_to` (`:2961-2967`); роль `all(Stalled)` при контроле `all(Landed)` (`:2981-2988`); `heights[0] == 95` при контроле `> 95` (`:3005-3015`); `role_canon_max > heights[0] + 32` (`:3029-3034`); `role_probes > ctrl_probes` (`:3046-3054`). Пропустили бы: блокировку derive на 300 с (роль сохраняет настоящий хэш, `StalledWithPeers` не достигается — сказано в доке `:2839-2848`); «`upstream_frontier` не убывает» — вакуумно (единственный писатель `fetch_max`, `executor.rs:1936-1938`).
- (б) На честном: `control.byz[3].latest_inflated == 0`, lockstep, `halted` пуст, `diverged == None`; `:2989-2992` (`all != Landed`) дублирует `:2981`.
- (в) Не показано: здоровая жертва — константы `BLOCK_INTERVAL = 1 s` (`application.rs:75`) и `FRONTIER_PROBE_INTERVAL = 1 s` (`executor.rs:392`) в детерминированном рантайме идут в ногу, `probe_frontier` выходит на `advanced && probe_fast_left == 0` (`executor.rs:1927-1930`) [KNOWN]; `StalledWithPeers` + 300 с derive-freeze; «один пир, обработав > f валидаторов, останавливает сеть»; ротация (`rotate: None`); подъём `upstream_frontier` через inlet (нет inlet).
- (г) Мутация: `executor.rs:1936-1938` — удалить `rj.upstream_frontier.fetch_max(frontier.get(), …)`. По чтению: `peaked` останется 0 ⇒ `:2961` красный. Упадёт — [KNOWN] по коду. Мутация в `maybe_re_jump` (`executor.rs:2196-2198`, игнорировать `upstream_frontier`) — исход неясен [ГИПОТЕЗА]: пик всё равно наберётся в пробе (`:2961` останется зелёным), а спавнится ли прыжок, зависит от того, поднимает ли маршал узла 0 свой `last_tip_height` при подсказке на несуществующую высоту 1 000 194 (`executor.rs:1940` и далее); если нет — `calls.is_empty()` и красный на `:2977`. Не проверял.
- (д) После П-4 (`upstream_frontier` только от проверенного сертификата): `:2961` красный — это регрессия. Но `heights[0] == 95` (`:3005`) останется ИСТИННЫМ и после правки: единственный источник фронтира узла 0 — лжец (`upstream_only_link = (3, 0)`, `tests.rs:2809`), честной альтернативы у него нет, `get_latest` отвергнутого ответа даст `Lagging`. Половину «после» (жертва отвергает лжеца И догоняет от честного пира) эта фикстура показать не может.
- Вердикт: частично — совпадает со статусом реестра; статус честно называет гейт `awaiting_seed` и «клин по курсору». Заголовок записи («блокирует исполнение живого валидатора») стендом не показан и статус этого не скрывает.

### R-006 сценарий 1 — `guard_two_on_the_catch_up_path_reads_a_pre_fcu_height` (`tests.rs:277-445`; без фичи)

- Прогон [KNOWN]: `heights=[24,24,24,8]`, `halted=[(3, ResultDivergence)]`, `guard2=[]`, `backward` — «result divergence at height 9 … executor.rs:3247:17»; `el_events[3]` около 6: `Derived(4) Canonicalized(4) Derived(5) Canonicalized(5) Derived(6, 0x96c835…) Canonicalized(6) Derived(7) Canonicalized(7) Derived(8) Derived(8) Derived(8) Derived(8) Canonicalized(8) Derived(9)`.
- (а) Несущие: `derived_at(3,6) < canonicalized_at(3,6)` (`:376-383`); `guard2.is_empty()` (`:408-412`); `backward` содержит «at height 9» (`:413-420`); `heights[3] == 8` (`:421-427`); `canonicalized_at(3,9).is_none()` (`:399-404`). Пропустили бы: что guard #2 вообще выполнился и прочитал `None` — положительного наблюдения нет (guard пишет лог только на `Some(false)`, `executor.rs:3154-3170`); «вооружён» выводится из свидетеля разреза (`:337-341`).
- (б) На честном: lockstep трёх, структура `halted`; свидетель разреза держится и на честном разрезе.
- (в) Не показано: сценарий 2 (ложный halt по спекулятивному сиблингу); поведение настоящего `provider.block_hash(h)` до FCU — смоделировано по RETH_INTERNALS, против reth не гонялось. Сверх записи: `Derived(8)` четыре раза подряд до `Canonicalized(8)` — по коду это парк `NeedAttestation` с повторным derive (`executor.rs:3171-3180`, комментарий «the park CARRIES σ, so the re-poke re-derives»), т. е. guard #2 на высоте 8 был вооружён и ждал тело 11 — [ГИПОТЕЗА] по причине, [KNOWN] по факту четырёх записей. Док-строка `15_…` §15.a (строка таблицы (3b): «deriving each height ONCE») этому прогону не соответствует.
- (г) Мутация: `executor.rs:3154-3159` — заменить замыкание `|h| self.executed.spec_executed_hash(h)` на `|_| Some(derived_hash)` (это и есть П-6 п. 3: сравнивать с локально деривленным хэшем). По чтению: `result_matches` вернёт `Some(false)` на 6, guard выбросит `Fault::fork_safety` до ack ⇒ `guard2.is_empty()` на `:408` красный, `heights[3]` станет 5. Упадёт — [KNOWN] по коду.
- (д) После П-6 п. 3 — красный по (г); ветка «not reproduced» уже описана в доке (`:235-238`), переписать нужно ассерты (`heights[3]`, `backward`, `canonicalized_at(9)`). К Э4 (П-1/П-4) не относится.
- Вердикт: сценарий 1 воспроизведён — совпадает с реестром.

### R-008 — `a_forged_seed_slot_is_admitted_with_no_key_and_refused_when_the_key_lands` (`tests.rs:2635-2791`; `Role::ForgedSeedUpstream`, `ForgeMode::SeedSlot`)

- Прогон [KNOWN]: `heights=[140,140,140,127,127]`, `forged=[65..70]`, `keyless=310`, `refusals=12`, `replays=[[],[],[],[66,67,68,69,70],[]]`.
- (а) Несущие: `refused_heights == forged_sorted` (`:2763-2766`); `!refusals.is_empty()` (`:2725-2729`); `!relayed.is_empty()` (`:2785-2790`); только ERROR-строки отказов (`:2735-2740`). Пропустили бы: от кого узел 4 получил подделку (не атрибутировано); что сертификат лежит в архиве маршала (выводится из того, что узел 3 его отдал); ротацию (inlet нет).
- (б) На честном: `rejected == 0`, `keyless > 0`, `verify_ok > 0`, lockstep, `halted` пуст — все держатся (док теста это перечисляет, `:2617-2623`).
- (в) Не показано: плечо `CertInlet::ingest` / `ensure_key(Local)` (`cert_inlet.rs:607`, `:693`) — путь стенда идёт через `UpstreamResolver::spawn_finalized` (`cert_inlet.rs:3102`); `record_data_fault` (`:945`) в стенде не существует; «другие видят data fault и ротируют» — не достигнуто; R-069 (carry-forward); «executor держит блок» — маскируется включённым прыжком (`:2641`), показано только как «не деривил 64..70».
- (г) Мутация: `bls/src/combined_scheme.rs`, ветка `verify_certificate` (около `:437`, `!matches!(o.verify_seed(…), SeedCheck::Invalid)`) — заменить на `matches!(…, SeedCheck::Valid)` (fail-closed при `NoKey`, вариант (а) решения Д-3). По чтению: follower отвергнет сертификат в маршале, в карантин ничего не ляжет, `promote_epoch` (`certify.rs:288-317`) нечего отвергать ⇒ `refusals` пуст ⇒ `:2725` красный с печатью «(c) PATH NOT REACHED — nothing was admitted keyless, or no key ever landed». Упадёт — [KNOWN] по коду, но ярлык ветки будет вводить в заблуждение: исправленная система напечатает «путь не достигнут».
- (д) После П-2 (σ из `get_finalization`, `certify.rs`/`SeedStore` удаляются): ERROR-строка `certify.rs:315` исчезает вместе с файлом ⇒ тот же красный с тем же ярлыком; `served_seed_replays` останется непустым, пока `NoKey`-приём не закрыт (Д-3). Как «после» — переписывать блок (2)–(3) под выбранный вариант Д-3.
- Вердикт: воспроизведено, включая отравление архива. Статус реестра говорит «воспроизведено целиком» и тут же перечисляет три недостигнутых звена (ротация, R-069, плечо inlet). «Целиком» — переоценка формулировки: из семи звеньев механизма записи (таблица журнала сессии A §5) не достигнуты два, третье («executor держит блок») показано косвенно.

### R-009 — `a_wrong_height_answer_satisfies_the_fetch_and_starves_the_by_height_gap` (`tests.rs:3441-3583`; `Role::WrongHeightFinalized`)

- Прогон [KNOWN]: роль — `served=42`, пары `(6, 5)`, `victim_h=5`, `finalized_calls=43 = finalized_delivered=43`, `deliveries_rejected=0`; контроль — `victim_h=15`, `finalized 11/11`.
- (а) Несущие: свидетель подмены (`:3451-3474`); `role.heights[3] == 5` (`:3543-3548`); `role < control` (`:3549-3554`); `control >= 14` (`:3536-3540`); `finalized_calls == finalized_delivered` и `>= wrong_height_served` (`:3520-3529`). Пропустили бы: сам отказ маршала — он на DEBUG (`CW …/marshal/core/actor.rs:987-993`, `send_lossy(false)` без лога [KNOWN]), стенд ловит WARN ⇒ отказ выведен дифференциально; что `deliver` вернул `true` именно на этих парах (`deliveries_decoded` считает всё).
- (б) На честном: `deliveries_rejected == 0`, `finalized_delivered > 0`, `rejump_calls == 0`, lockstep, `errors` пуст.
- (в) Не показано: гонка «пока пир самый быстрый» (единственный источник); штраф/ротация пира резолвером (`[LIKELY]` реестра не снят и не может быть снят этой фикстурой — второго пира нет); плечо `CertInlet::ingest`; выход через re-jump.
- (г) Мутация: `plane_upstream.rs:206-214` — после `decode_frontier` добавить `if let FrontierKey::Finalized { height } = key { if uf.block.height != height { return false; } }` (это П-4 для `deliver`). По чтению: `CountingHandler::deliver` (`fakes.rs:1585-1594`) увеличит `deliveries_rejected` ⇒ `:3503` красный. Упадёт — [KNOWN] по коду.
- (д) После П-4: красный только по `deliveries_rejected`; `role.heights[3] == 5` (`:3543`) с большой вероятностью ОСТАНЕТСЯ истинным — резолвер после `false` повторит запрос тому же единственному источнику, честного пира в фикстуре нет. Чтобы тест показал «после» (жертва догоняет), нужен второй, честный источник — ручка `upstream_only_link` даёт ровно одну пару.
- Вердикт: воспроизведено — совпадает с реестром; `[LIKELY]` про резолвер оставлен корректно.

### R-121 — `a_zero_overlap_boundary_halts_the_chain_verify_only` (C7, `tests.rs:1953-2017`)

- Прогон [KNOWN]: `heights=[95×4, 63×4]`, `boundaries(out)=[(0,0),(1,28),(2,60),(3,92)]`, `boundaries(in)=[(0,0),(1,28),(2,60)]`, 400 вирт. с.
- (а) Несущие: `timed_out` + точный вектор высот (`:1963-1982`), `halted.is_empty()` (`:1983-1987`), `artifacts` `[2]`/`[3]` (`:1989-2002`). Пропустили бы: ПОЧЕМУ стоят — механизм («σ без backfill, ключа не подтянуть») выведен из кода, в тесте нет наблюдения попытки/отказа pull'а.
- (б) На честном: `diverged == None`.
- (в) Не показано: on-chain ротация через контракт и пол `MIN_COMMITTEE_LENGTH` (расписание — замыкание); R-007 при > f, R-018, R-026 (grep по `testbed/` — ни одного упоминания, подтверждаю контрагента 3.4 [KNOWN]); «второго пути доставки `PK_E`/σ нет» — `[LIKELY]` реестра: 400 вирт. с без движения — свидетельство, не доказательство.
- (г) Мутация: `epoch_manager.rs:1677` — `filter(|e| **e < frontier)` → `**e <= frontier` (sweep рассматривает эпоху фронтира). [ГИПОТЕЗА]: уходящая половина (верификаторы эпохи 3, фронтир 3) подтянет артефакт 3 у входящей и уйдёт с 95 ⇒ вектор высот на `:1968` красный. Уверенности нет: не читал, попадает ли эпоха фронтира в `verifier_epochs()` и есть ли у sweep'а будильник в этой конфигурации (те же будильники `:730`/`:817`, что в R-122).
- (д) Для П-1/П-4 не «после»; ближайшая правка — Э5 рядом с П-2 (`PLAN.md` R-121 «кандидат»). Как регрессия годится только после того, как правка названа.
- Вердикт: воспроизведено (факт остановки) — совпадает с реестром; уверенность `[KNOWN]` по воспроизведению корректна, механизм по-прежнему `[LIKELY]`.

### R-122 — `a_rotated_out_node_without_the_rejump_parks` (C8, `tests.rs:2040-2089`)

- Прогон [KNOWN]: `heights=[168,168,168,95]`, `rejumps=[0,0,0,0]`, `ack_drops=0`.
- (а) Несущие: `heights[3] == 95` (`:2053-2058`), `artifacts[3] == [2]` (`:2061-2065`), `latest_delivered > 0 && finalized_delivered > 0` (`:2071-2074`), `rejump_calls == 0`. Пропустили бы: сам механизм (будильники sweep'а) — нет счётчика «sweep не разбужен»; тест не отличает «пути к pull'у нет» от «pull был и не удался».
- (б) На честном: lockstep трёх, `halted` пуст.
- (в) Не показано: «узел без upstream'а выхода не имеет» (`[LIKELY]` реестра); tee живого фронтира «не помогает» — в тесте не ставится (это отдельный прогон журнала `E3-2-STAND-4.md` §11).
- (г) Мутация: `epoch_manager.rs:753-756` — после `handle_msg_for_unregistered_epoch`, когда пара `(observed, entered)` сдвинулась, добавить `sweep_wake.send_replace((self.highest_observed_epoch, self.highest_entered_epoch))` (минимальная правка из самой записи R-122). По чтению [ГИПОТЕЗА]: догоняющий span поднимает `highest_entered_epoch` (`:1811`), эпоха 3 становится под-фронтирной, sweep её чинит, узел 3 уходит с 95 ⇒ `:2053` красный.
- (д) Хороший «до»; после правки инвертировать `:2053` и `:2061` (узел следует без прыжка).
- Вердикт: воспроизведено («до»-тест есть) — совпадает по сути. [KNOWN] Статус `REGISTER.md` R-122 называет тест `a_rotated_out_node_without_the_live_tee_parks` — такого теста нет, актуальное имя `a_rotated_out_node_without_the_rejump_parks` (`tests.rs:2040`).

### Сводка по §2

| Запись | Вердикт этой оценки | Статус `REGISTER.md` | Переоценка |
|---|---|---|---|
| R-001 | частично (посадка — модель, проверка на подменённом состоянии недостижима) | воспроизведено частично | нет |
| R-002 | воспроизведено (оба звена) | воспроизведено | нет |
| R-004 | частично (пик и `Stalled`; клин только курсора; жертва ротированная) | воспроизведено частично | нет |
| R-006 сц. 1 | воспроизведено | воспроизведено | нет; дока §15.a «each height ONCE» неверна |
| R-008 | воспроизведено, 2 звена из 7 не достигнуты | «воспроизведено целиком» | да, словом «целиком» |
| R-009 | воспроизведено (одноисточниковый голод) | воспроизведено | нет |
| R-121 | воспроизведён факт, механизм по коду | воспроизведено (C7) | нет |
| R-122 | «до»-тест есть | «before»-тест есть | нет; имя теста в статусе неверно |

## 3. Решения об объёме

### 3.1 урезано до «расхождения фейка — в коде» вместо conformance против in-process reth

- Что теряет Э4/Э5, если П-4/П-1 проверять только стендом. П-4 меняет порядок `sync_to`/аутентификации и вводит `Frontier`. Стенд проверит это против `JumpElSync` (`fakes.rs:1209-1320`) — ручной модели `RethElSync`, в которой: нет SYNCING-навсегда при FCU на неотдаваемый хэш (Ex-2 показал его живьём), нет INVALID, `holds` никогда не зовётся, а вердикт для конфликтующего префикса помечен `[ГИПОТЕЗА]` в самом коде (`fakes.rs:1280-1285`). То есть П-4 будет проверен тем же классом фейк-оракула, который R-006 и обнажил; блок «Divergences» честно это перечисляет, но перечисление не делает тесты зелёными по правильной причине. Для П-1 потеря меньше: комитет читается через `StakingStateRead`, а не через engine API; фейк здесь `FakeStaking`, не reth.
- Цена альтернативы. Полный conformance (`BeaconEngineLike`/`ExecutedChain`/`DerivedBlockBuilder` против in-process reth с rWasm-EVM) журнал оценивает в 3–4 недели при неподключённом `reth-e2e-test-utils`. Дешевле — двухступенчато: (1) одноузловой conformance без devp2p: поднять reth-провайдер + engine tree в тесте (`genesis-bootstrap` уже строит genesis-состояние), гонять `RethImporter::import_derived` → `fork_choice_updated` → `block_hash(n)` и сверять с `FakeChain` на пяти сценариях (import без FCU; FCU на голову; FCU на предка; FCU на неизвестный хэш; сиблинг той же высоты) — закрывает ярусы R-006, INVALID/`invalid_state`, `best`/`last`; [ГИПОТЕЗА] ~2 недели, основание — нужен `tree_sender_escrow` и `EngineNodeLauncher` внутри теста, чего в workspace никто не делал. (2) Двухузловой devp2p backfill для `sync_to` — ещё ~2 недели [ГИПОТЕЗА]. Итого те же 3–4 недели, но первая половина полезна сама по себе.
- Был ли выбор правильным. Для закрытия Э3 — да: без урезания Э3 не закрылся бы, а урезанная форма закрыла реальную ложь (R-006). Для Э4 — с одним условием: до правки `sync_to` в П-4 нужна либо ступень (1), либо живой Ex-2-подобный прогон на девнете для трёх исходов `sync_to` (неотдаваемый хэш, конфликтующая ветка, честная ветка). Без этого П-4 проверяется моделью.

### 3.4 отложено до после Э4/Э5

- Что теряет Э4/Э5. Плечо `CertInlet::ingest` целиком: `ensure_key(Local)` (`cert_inlet.rs:693`), `record_data_fault`/ротация (`:945`), бинд эпоха↔высота (`:621-651`), `upstream_frontier` через inlet (`:654-657`). Это ровно то, что П-2/Д-3 (R-008) и П-4 (R-004: «`upstream_frontier` только проверенным сертификатом») меняют. Плюс on-chain ротация и пол `MIN_COMMITTEE_LENGTH`, R-007 при > f, R-018, R-026 — стенд их не касается (grep подтверждаю).
- Цена альтернативы. Девнет Ex-21/Ex-19 — 4–8 ч + 3–4 ч по `EXPERIMENTS.md` §3 плюс третий режим `cert-mitm-proxy.py`. Стендовая альтернатива — шов inlet'а в `build_node`: `CertInlet::new(marshal, committees, ctx).with_tee(tee).with_rotate(rotate)` (`node/src/cert_inlet.rs:110-114`) над каналом `UpstreamFinalized`, который кормит `PlaneUpstreamHandle::get_finalization`/`get_latest` в цикле; [ГИПОТЕЗА] 1–2 дня, основание — `CertInlet` уже generic по `CommitteeSource` и `Randomness`, а `JumpCommittees` и `plane.randomness` в стенде есть.
- Был ли выбор правильным. Да — девнет здесь приёмка, а не «до». Но откладывать заодно и стендовый inlet — нет: без него R-008 «до» для Д-3 и R-004 «до» для П-4 проверяют не тот вход, что прод.

### Вариант А для R-001 и здоровая жертва для R-004 не ставились

- Что теряет Э4. Механизм записи R-001 — «`verify_jump_authenticated` читает `committee[E]` из подменённого состояния и проверка ПРОХОДИТ» — это единственная вещь, которую П-4 («комитет из локального finalized») и П-1 («комитет как значение») чинят напрямую. Сегодня в стенде НЕТ теста, который проходит на подменённом состоянии и станет красным после правки; C9 и R-001 проверяют только ФОРМУ вызова (`at == landing_hash`). Значит правка П-4 в этой части не имеет свидетеля red→green. Здоровая жертва для R-004: механизм тот же (проба → `fetch_max` → прыжок), различие в достижимости; потеря — только утверждение «даже здоровый узел».
- Цена. Вариант А: (1) `FakeStaking` с составом, зависящим от ветки хэша — `Members` должен принимать `(epoch, at_hash)`, а роль — публиковать «свой» комитет под дивергентными хэшами (~0,5 дня); (2) переподпись: роль генерирует 2f+1 своих BLS-ключей, кладёт их в комитет под своим хэшем и собирает `Finalization` над переставленным `proposal` (`build_signer` + `assemble` уже используются в `WithholdingRandomness`, `byzantine_roles.rs:437-443`) — ~1–1,5 дня; итого ~2 дня [ГИПОТЕЗА]. Здоровая жертва: развести темп пробы и блока — либо `StandConfig`-ручка на латентность одного узла (`Link { latency }` задаётся per-link, `stand.rs:927-931`, сейчас одна на всех), либо сделать `FRONTIER_PROBE_INTERVAL` параметром (`executor.rs:392`, прод-правка) — ~0,5–1 день [ГИПОТЕЗА].
- Был ли выбор правильным. Здоровую жертву — можно было отложить. Вариант А — нет: это «до» для центральной части П-4, и его отсутствие делает 4.2 непроверяемым стендом в той части, ради которой стенд строился (`PLAN.md` §2: «стенд раньше … доказательства П-2/П-4»).

## 4. Готовность к Э4 (и коротко к Э5)

### 4.2 П-4 — `Frontier`, `deliver` сверяет высоту и эпоху, аутентификация до `sync_to`, комитет из локального finalized

Станут красными при правильной реализации (это регрессия):
| Тест | Ассерт | Почему красный |
|---|---|---|
| R-009 | `tests.rs:3503` `deliveries_rejected == 0` | `deliver` вернёт `false` на пару чужой высоты |
| R-004 | `tests.rs:2961` `Some(peaked) == inflate_to` | `upstream_frontier` перестанет подниматься непроверенным `Latest` |
| R-001 | `tests.rs:3279-3283` чтение комитета на хэше посадки; `:3311` `canon_div` непуст | комитет из локального finalized; посадки до аутентификации нет |
| C9 | `tests.rs:3210-3213` `*at == landing_hash` | то же чтение из локального finalized |
| R-004/R-001 (косвенно) | `outcome == "Stalled"` / `"AuthFailed"` | если П-4 отвергает раздутый/подменённый `Latest` уже в `deliver` (эпоха раунда ≠ `epoch_of(1_000_194)`), прыжок получит `Lagging`, а не `Stalled`/`AuthFailed` |

Останутся зелёными, и это плохо:
- R-004 `heights[0] == 95` и R-009 `heights[3] == 5`: жертва в обеих фикстурах прикована к лжецу (`upstream_only_link`), честного пира нет — правильная П-4 отвергнет лжеца и оставит жертву стоять. Половина «после» (отверг → догнал от честного) не проверяется ни одним тестом.
- B2, R-008, C8: посадки честные, комитет для эпох 4/5 закоммичен и в локальном finalized узла 3 (95 — эпоха 2 — коммитит до 4; посадка 125 — эпоха 3 — до 5), так что «комитет из локального finalized» здесь неотличим от «на хэше посадки».
- Все тесты со `StaticRandomness` — к П-4 отношения не имеют, зелёные по построению.

Каких тестов нет и они нужны до начала правки:
1. Вариант А R-001: проверка ПРОХОДИТ на подменённом состоянии до правки — красный после. Без него П-4 в части «комитет из локального finalized» не имеет свидетеля.
2. Двухисточниковая фикстура для R-009/R-004: жертва с лжецом И честным пиром; до правки — голод/раздутие, после — отказ лжецу и догон. Требует `upstream_only_link` → список пар.
3. Роль «эпоха раунда ≠ `epoch_of(height)`» на `Latest` и на `Finalized{h}` (R-040/R-001 «эпоха не сверяется», `cold_start_jump.rs:683`): сегодня прод сверяет это только в follower-inlet'е (`cert_inlet.rs:621-651`, `epoch_bind`), стенд не ставит.
4. Cold-start прыжок с фиксированным порогом (`cold_start_jump`, `cold_start_jump.rs:761`) — в стенде не зовётся вовсе (grep по `testbed/` — только `_with_threshold`) [KNOWN]; Д-2 («прыжок только в пределах 2 эпох от локального finalized») нечем проверить.
5. `holds(l1)` / Ex-23 (`block_hash(number) == hash`) — `l1_checkpoint = None` на пути стенда; нужна фикстура с чекпоинтом, иначе замена `holds` не тестируется.
6. R-003 `corroborate_frontier` — в `testbed/` не упоминается [KNOWN]; П-4 закрывает R-003 по плану — без теста.
7. Inlet-плечо (§3): `upstream_frontier` через `ingest` и ротация.

### 4.1 П-1 — комитет как значение

Красные при правильной реализации: C2 (`et_boundaries` — форма записи границы и якорь `read_height_for` сменятся на «первый финализированный блок с высотой ≥ start(E−2)», `tests.rs:1683-1707` пересчёт нужно переписать); C3 (`uncommitted[1] >= 1` на генезисе `tests.rs:1844-1848` — при едином якоре чтение на генезисе может исчезнуть); C4 (если `TransitionOutcome::EpochAdvanced` переживёт — зелёный, иначе компиляционно красный); C9/R-001 блок (4) (чтение на хэше посадки). Останутся зелёными, и это плохо: всё, что читает расписание, — состав комитета в `FakeStaking` не зависит ни от хэша, ни от якоря (`fakes.rs:944-954`), веса всегда `1` (`:1005`), тумбстоуны `false` (`:892`) — «значение `{epoch, members, keys, weights@anchor, read_at}`» неотличимо от «функция эпохи»; класс R-075 (две авторитетные таблицы) непроверяем. Нет и нужно: `FakeStaking` с состоянием по ветке хэша и мутируемым реестром (шаг 5b: `recordProduction`, тумбстоун, изменение веса); тест «два узла читают эпоху E на разных хэшах и получают одно значение»; типизация ошибок `NotYetCommitted` / `ReadFailed{transient}` — `FakeStaking` умеет только `Ok`-пусто и `Backend` на неизвестном хэше (`fakes.rs:935-940`).

### 5.2 П-9 — коротко

Красные: R-002/a (`tests.rs:2383` `victim_demoted`), R-002/b (`:2510` `!crossed`). Зелёные-плохо: ничего про улику (Д-6) — `NoSink`; R-036 (`NoFile` после дедлайна) не поставлен (журнал сессии A §5 говорит «постановимо прямо сейчас»); f жертв. Нужно до правки: тест на улику через `slasher_sink` → `FakeStaking` (шаг 5b) и R-036.

### 5.3 П-2 — коротко

Красные: R-008 (`tests.rs:2725` — ERROR-строка `certify.rs:315` исчезнет вместе с `certify.rs`; ярлык печати скажет «PATH NOT REACHED»). Зелёные-плохо: R-007 не имеет теста (ветка «финализация без нотаризации» не поставлена; by-height половина идёт живьём в R-008, но не ассертится); `hint_finalized` как выход — нигде. Нужно: фикстура «финализация без локально виденной нотаризации» (разрыв, при котором узел получает `Finalization`, не собрав нотаризацию), inlet-плечо для Д-3, и переписанный R-008 под выбранный вариант Д-3.

## 5. Дрейф доков

Проверено против дерева в этой сессии. «Девяти записей за 09-10» в `00_preamble.md` нет: [KNOWN] блок `verified-against` содержит шесть записей с датой `2026-09-10` (строки 7, 36, 94, 149, 189, 204) и восемь с `2026-09-09` (240, 272, 309, 328, 348, 361, 378, 390); плюс маркер `[REVISED 2026-09-10, Э3.9]` на строке 397.

| Претензия | Где | file:line в дереве | Верно/неверно |
|---|---|---|---|
| `FakeBeacon` строится «at `testbed/stand.rs:1803`» | `00_preamble.md:105-106` (запись Э3.1) | `stand.rs:1941` `FakeBeacon::new(chain.clone())` | неверно (якорь протух после переработки T2) |
| Гейт клина — `awaiting_seed` «`executor.rs:1418`» | `00_preamble.md` (запись B1), `15_…` строка R-004, `tests.rs:2999` | `executor.rs:1417` `&& self.awaiting_seed.is_none()` | сдвиг на одну строку |
| BLS-`ensure!` «`cold_start_jump.rs:688`» | запись B1, `tests.rs:3108`, `:3236` | `cold_start_jump.rs:687-692`, `verify` на `:688` | верно |
| guard #2 `executor.rs:3140`, FCU `:3308`, обратная проверка `:3247`, тест `:7448` | записи Э3.1, правило 42, `15_…` | `:3140` (начало комментария guard #2), `:3150` `if behind_by_k`, `:3308` `fcu_retrying_transport`, `:3245-3247` `fork_safety(ResultDivergence…)`, `:7448` `fn guard2_convergence_mismatch_engages_safety_halt` | верно |
| `node/src/ordering.rs:45-46`, `node/src/importer.rs:112` | правило 42, `fakes.rs:613-637` | `ordering.rs:45-47`, `importer.rs:112` | верно |
| reth already-known гейт «`engine/tree/src/tree/mod.rs:1551-1576`» | запись Э3.1, `15_…:40`, `fakes.rs:213` | pinned `8cc96a3` `mod.rs:1551-1573` (return на 1572-1573) | почти верно (хвост шире на 3 строки) |
| Прыжок стенда вызван «как `consensus/src/dpos.rs:2492-2536`» | запись «step 1», `stand.rs:1647` | `dpos.rs:2492-2536` (`cb` closure), вызов `cold_start_jump_with_threshold(` на `:2525`, `rotate: Some(rotate)` `:2552`, `probe: Some(frontier_probe)` `:2559` | верно; но прод передаёт `rotate: Some`, стенд — `None` (`stand.rs:1746`) — в доке названо |
| «`JUMP_THRESHOLD.min(interval)` `consensus/src/dpos.rs:2466`» | `15_…:101`, `tests.rs:1251` | `dpos.rs:2466` | верно |
| `MAX_COMMITTEE_LOOKAHEAD_EPOCHS` «`staking_protocol.rs:73`» | запись «soundness», `cold_start_jump.rs:42-46` | `staking_protocol.rs:73` | верно |
| Посадки C9 «(125, 0xecf0792f…), (157, 0xca925388…)» | запись «step 1» | прогон: `(125, 0xecf0792f…)`, `(157, 0xca925388…)` | верно |
| Посадки R-008 «(93, 0xb9a09898…), (125, 0x99e03837…)» | запись «step 1» | тест R-008 их больше не печатает; не проверено | [LIKELY] |
| «`commitEpochCommittee` selector … `0xfecaf0f1` / `0xad36f42f` pinned as before» | `00_preamble.md:236-238` (запись Э3.9) | `consts.rs:178` `0xad36f42f` = `SIG_GET_CONSENSUS_KEYS`; `consts.rs:184` и `staking-abi/src/lib.rs:315-318` `0xe505b249` = `commitEpochCommittee()` | неверно: селектор `commitEpochCommittee` — `0xe505b249` (журнал `E3-CLOSEOUT.md` §2 говорит верно) |
| «24 calls now», «G16 … 17 shared signatures» | запись Э3.9 | `agreement_check.py`: G3 24, G16 17+16 | верно |
| «35/0/0 in 187 s», «lib suite 636+3+5+13» | записи B2, Э3.1 | 35/0/0 за 192,93 с; 636/3/5+1 ign/13 | верно |
| «all 30 pre-existing stand tests pass UNCHANGED» | Э3.1, правило 42, `15_…:72` | `git diff HEAD -- tests.rs` — +1240 строк; проверить «ни один старый ассерт не менялся» diff'ом я не стал (объём); по чтению тестов (1)–(7′), B*, C2–C8 ассерты соответствуют докам | [LIKELY] |
| `committee_read_hash` прода «`node/src/dpos.rs:1382-1404`» и «`:1441-1457`» | `15_…:95` и `:86` (две разные ссылки на одно) | `node/src/dpos.rs:1382-1404` (`committee_read_hash`), `:1450-1457` — вторая копия того же паттерна | обе существуют; §15.a ссылается на две разные строки как на одно место |
| `live_height` пишется «past the cert inlet's BLS gate, `cert_inlet.rs:896-898`» | `15_…:96`, `stand.rs:1604` | `cert_inlet.rs:896-898` `tee.live_height.fetch_max` после `capture_certificate_seed` (`:866`) | верно |
| «`RethElSync::sync_to` waits for reth to call the landing canonical AND EXECUTED, `cold_start_jump.rs:447-455`» | `15_…:111` | `:447-455` — это ветка pre-K (`tip_hash == ZERO`); ожидание `Valid` — `:478-613` | якорь не туда |
| (3b): «the catch-up node deriving each height ONCE while the honest nodes derive twice» | `15_…` строка таблицы (3b) | прогон: `Derived(8)` ×4 у узла 3 до `Canonicalized(8)` | неверно для этого прогона |
| (3b): `Derived(6, 0x96c8…)` | там же | прогон: `0x96c835e6…` | верно |
| (4c) «47 Latest, 12 by-height, node 0 served all 59»; (4d) «[16,16,16,15]» | `15_…` | прогон (4c): 47/47, 12/12, served 59; (4d): `[16,16,16,15]`, node 3 47/47, 12/11, served 58 | верно |
| R-004 строка: «14 calls», «heights[0]=192» у контроля | `15_…` | прогон: 14, `[192×4]` | верно |
| R-009 строка: «43», «5 против 15», «~:987-993» | `15_…`, `REGISTER.md` | прогон 43/43, 5/15; `CW …/marshal/core/actor.rs:987-993` (checkout `monorepo-9732103c47eb4665/3c4e02c`) | верно |
| Статус R-122: тест `a_rotated_out_node_without_the_live_tee_parks` | `REGISTER.md` R-122 | `tests.rs:2040` `a_rotated_out_node_without_the_rejump_parks`; теста с `live_tee` нет | неверно (имя) |
| `testbed/mod.rs:33` «`fakes::SnapshotReader` — committees come from a per-epoch schedule … the on-chain path is step 5 and NOT here» | `mod.rs:33-37` | `grep SnapshotReader testbed/` — только эта строка; типа нет; шаг 5 сделан (`stand.rs:1474`) | неверно, протухло с шага 5 |
| `testbed/mod.rs:77` «The byzantine wrappers of Э3.3 (`Role::TwoReveals`, `Role::ForgedSeedUpstream`)» | `mod.rs:77-81` | ролей пять (`stand.rs:250-289`) | неполно |
| `fakes.rs:1350` «`deliver` calls that did NOT decode (`false` — the R-009 arm)» | `fakes.rs:1350` | R-009 как раз НЕ трогает этот счётчик (`tests.rs:3503` ассертит `== 0`); T3 это пометил как «неверно», не исправлено | неверно |
| `fakes.rs:203-205` «Naming an ANCESTOR as head … the suffix above it is dropped» | `fakes.rs:203-205` | код `:411-413`: голова, каноническая на своей высоте, — no-op; `retain` только при походе к точке развилки | неверно (противоречит `:223-227` того же блока и коду) |
| `stand.rs:1607-1609` «a Byzantine-upstream role (Э3.3) has to move this behind a verify before it means anything» | `stand.rs:1598-1610` | роли добавлены, `live_height` по-прежнему пишется в пробе до проверки (`:1639`) | протухло / невыполненное обещание |
| `stand.rs:129-136` «every other upstream link touching either is removed» | `stand.rs:961-973` | снимаются только исходящие линки source/victim | неточно |
| `executor.rs:3140-3148` «a wrong derive is caught IMMEDIATELY, not K blocks downstream … the ONLY code-proven result-divergence detector on the catch-up path» | прод-комментарий | (3b): guard молчит, halt через K на `:3247` | неверно; журнал переписал `02_…`, а прод-комментарий и комментарий теста `:7443-7446` («immediately») — нет |
| Счётчики `TOC.md` «Lines» | `TOC.md:14-32` | `wc -l`: `00a_errata` 169 (в TOC 155), `05_` 186 (180), `06_` 585 (557), `07_` 454 (440), `10_` 138 (134), `11_` 92 (87); остальные 13 совпадают | 6 из 19 устарели |
| `EXPERIMENTS.md` §1 09-10 и `PLAN.md` 3.3 (числа R-009 «42 пары… 5 против 15», R-004 «`real+10^6`») | живой слой | прогон | верно |
| `E3-CLOSEOUT.md` §7 «Коммиты… 1–4» | журнал | `git log` — не сделаны | описание намерения, не факта; в §7 это сказано («после “ок” пользователя») |

## 6. Числа

| Величина | Заявлено | Пересчитано [KNOWN] | Источник |
|---|---|---|---|
| Тестов стенда | 35 | 35 `#[test]` в `testbed/tests.rs`, 0 в остальных файлах `testbed/`; 8 за `#[cfg(feature = "dpos-devnet-byzantine")]` (eq, R-002/a, R-002/b, R-008, R-004, R-001, R-009, unit `land_jump`), 27 без фичи; `#[ignore]` — 0 | `grep -c '#\[test\]'`, список `#[cfg(feature` над `#[test]` |
| Прогон стенда | 35/0/0, 187–188 с | 35/0/0, 192,93 с | `testbed_run.log` |
| lib без фичи | 636+3+5+13/0, 2 ignored | 636/0; 3/0; 5/0 + 1 ign; 13/0; doc 0 + 1 ign | `lib_run.log` |
| Деклараций общего ABI | 24 | G3: «24 shared declarations, each imported by consts.rs and present exactly once in the blob» | `agreement_check.py` |
| Мест харнесса | 113 | G16: «17 shared + 16 contract-only signatures agree across 113 sites (54 more admitted as off-contract or not-called)» | там же |
| Прод-правок кода в Э3 | «ноль» (поведение); видимость `EL_SYNC_STALL_ESCAPE` + комментарий; `consts.rs` | `git diff HEAD`: `cold_start_jump.rs` 32 строки диффа (док-комментарий `:42-56`, `:648-656`; `pub(crate) const EL_SYNC_STALL_ESCAPE` `:219`); `contracts/staking/src/consts.rs` 41 строка (вывод `SIG_*` через `sig::<abi::…>()`); `crates/staking-abi/src/lib.rs` +155 (9 новых деклараций + пины); `genesis-bootstrap/{Cargo.toml,src/bootstrap.rs,tests/bootstrap_smoke.rs}`; `e2e/{Cargo.toml,src/staking*.rs}` ×4; `Cargo.lock` +2; харнесс: `writes.py`, `shadow.py`, `compose_gen.py`, `production_path.py`, три `tests/*.py` + новый `test_protocol_constant_transcriptions.py`; `scripts/xp/{agreement_check.py,README.md}`; `STAKING_ARTEFACT.md` | `git diff --stat HEAD -- . ':!.dpos-study'` — 27 файлов, +3612/−542 |
| Совпадает ли с журналом §7 | — | Да: список файлов коммитов 1–3 в `E3-CLOSEOUT.md:140-142` покрывает все 27 (плюс `.dpos-study/` — коммит 4). Уточнение к формуле «ноль прод-правок»: `staking-abi` и `consts.rs` — код поставляемых крейтов; значения селекторов не менялись (G3 зелёный против блоба), поведение — нет | — |
| Стенд 26 → 35, «ни один старый ассерт не менялся» | журнал §7 | 26 → 35 по именам сходится (9 новых: (3b), C9, R-002 ×2, R-008, R-004, R-001, R-009, `land_jump`); неизменность старых ассертов diff'ом не проверял | [LIKELY] |

## 7. Где предыдущие проверки были слабы

Все шесть контр-ревью — реляции агентов; по журналу ни один не запускал мутаций прод-кода и ни один не ссылается на pinned checkout reth (только на `RETH_INTERNALS.md`). Ниже — что именно каждый не открыл или принял на слово, с якорями.

- **V1 (T1, прод-прыжок в стенде).** Сильное: F5 (ложный довод про эпоху посадки) и F15 (невидимость не-`Landed`). Слабое: F3 — назвал `FakeStaking` чистой функцией эпохи и отложил «для T2»; T2 это не закрыл (вариант А не построен), оркестратор принял «F3 — для T2» и больше к нему не вернулся — дыра пережила закрытие (`fakes.rs:944-954`). Не проверил «посадки те же, что у модели» прогоном — принял по арифметике (сам журнал это фиксирует). Не заметил, что `live_height` пишется до проверки (`stand.rs:1598-1610`) — противоречие с `cert_inlet.rs:896-898`, названное в доке фейка, но так и оставленное.
- **V4 (T4, ярусы reth).** Самое сильное из шести (F3 — гэп при разрыве дерева, F12 — пустой захват не доказательство). Не открыл: pinned reth — все якоря приняты из RETH_INTERNALS (§1.1 выше); противоречие док-комментария `fakes.rs:203-205` с кодом `:411-413`; в прогоне (3b) — четырёхкратный `Derived(8)`, который расходится с текстом §15.a «each height ONCE». Утверждение «ни один `Syncing` не сработал в 33 тестах» — реляция исполнителя, никто не проверил счётчиком (в `Outcome` его нет).
- **V2 / V2b (T2, роли R-004/R-001).** Хорошо разобрали вакуумные ассерты (F3, F8, F12). Слабое: приняли `heights[0] == 95` как свидетеля клина в фикстуре, где 95 следует и из того, что лжец — единственный источник; никто не спросил, что эта фикстура покажет ПОСЛЕ П-4 (ответ — то же 95, §4). V2b F8 (состав `FakeStaking` не зависит от `at_hash`) второй раз назвал главную дыру — оркестратор снова принял «в доку/статусы» вместо кода. Это единственная находка, которую отклонили зря: она стоит ~2 дней и без неё П-4 не имеет свидетеля.
- **V3 (T3, R-009).** Верно поймал, что отказ маршала выведен дифференциально, а `rejump_calls == 0` — тавтология фикстуры. Не поставил вопрос об одноисточниковой фикстуре как о препятствии для «после» (§4 п. 2). Принял `finalized_calls == finalized_delivered` как «цикл consume-and-refuse» — равенство показывает только, что каждый pull завершился до следующего, не что маршал что-то отверг.
- **V5 / V6 (T5/T6, ABI и чекер).** К стенду не относятся; по своему предмету — добротные (F1 V6 про 78 пропущенных мест — существенная). Оставленный F7 (только арность, не типы ног) — честно назван.
- **Самопроверка сессии A (CR-1–CR-4) и её контрагент (A-1–A-8).** Сильные по механике R-002 (confirm жертвы как наблюдение). Слабое: (1) «R-008 воспроизведено целиком» при двух недостигнутых звеньях — формулировка перекочевала в `REGISTER.md`; (2) маршрут доставки `PK_2` follower'ам так и не атрибутирован (журнал §6) — это влияет на всё, что стенд говорит про «поздний ключ»; (3) единственный прогон каждого теста и «ветки “не воспроизведено” ни разу не сработали живьём» — сессия сама это записала, дальнейшие сессии не закрыли.
- **Оркестратор.** Квитанции [KNOWN] честные и по делу (номера строк, свои прогоны). Систематическая слабость: находки класса «фейк не может выразить прод-механизм» (T1 F3, V2b F8, V4 F5 про `holds`) уходили «в доку», а не в план работ; в результате `PLAN.md` 3.3 закрыт с формулировкой «R-001 вариант Б — частично», но нигде не записано, что вариант А — предусловие 4.2. Второе: 3.4 отложено с перечнем непокрытого, но стендовый inlet (1–2 дня) в этот перечень как работа не попал.

## 8. Оставлено как есть

Рассматривалось как проблема, признано нормальным — с причиной.

- Захват логов на WARN (`capture.rs:50-52`). DEBUG-строки маршала невидимы, отказ по высоте в R-009 выведен дифференциально. Понижать до DEBUG — менять объём для всех 35 тестов; дифференциальный вывод в доке назван. Нормально, пока (3b) единственный тест, ассертящий `log_capture_live`.
- `executed_tip` = канон-tip, а не `last_block_number` (`fakes.rs:616-624`). Объявлено; ни один гвард executor'а в стендовых сценариях на лаг персистенции не опирается.
- `FakeStaking::committed_at` с литералом `0` (`fakes.rs:929`) при общей константе `DPOS_ACTIVATION_BLOCK = 0`. Безвредно, пока активация нулевая; станет ловушкой при первой фикстуре с ненулевой активацией.
- `upstream_only_link` снимает только исходящие линки (`stand.rs:961-973`). Резолвер отвечает только на запросы, входящие линки к жертве ничего ей не приносят; док неточен, поведение верное.
- `EL_SYNC_STALL_ESCAPE` 300 вирт. с в `JumpElSync` без бюджета в фикстурах (`fakes.rs:1203-1205`). Ни одна текущая фикстура `Unservable` не достигает (после переработки R-004 хэш настоящий).
- `FORGE_WINDOW`/`WRONG_HEIGHT_WINDOW`/`LATEST_INFLATION`/`LYING_DIVERGE_AT` — константы модуля, не параметры роли (`byzantine_roles.rs:479-502`). `Role: Copy + PartialEq`; для новой фикстуры — вынести в `StandConfig`.
- Первая высота окна R-008 — донор σ и уходит нетронутой (`byzantine_roles.rs:659-671`): подделок `|окно| − 1`. Признано в журнале сессии A §6.
- `TwoRevealChecked::send` пропускает rate-check внутреннего Sender'а (`byzantine_roles.rs:327-341`) — квота `NonZeroU32::MAX` (`stand.rs:82`), фильтр не срабатывает.
- `SplitSendError` схлопывает ошибку в текст (`byzantine_roles.rs:272-278`) — единственный вызывающий отбрасывает результат.
- Каталоги `fluent-testbed-<pid>-<n>` в temp копятся (`stand.rs:219-221` удаляет только при создании). Известно с сессии 3.
- Один симулированный `Oracle` на N узлов (`TrackSink`, `stand.rs:752-835`): регистрацию эпохи получает первый узел, расхождения считаются (`tracked_mismatches`). Объявлено в §15.a.
- Точный вектор `[16, 16, 16, 15]` в (4d) (`tests.rs:648`) — хрупкий пин по замыслу (граница «симулированная сеть / транспорт»).
- `assert_ne!(victim_minted, victim_demoted)` и печать веток в ролевых тестах — ярлыки веток вводят в заблуждение после правки (§2 R-008), но как «до» — читаемы.
- `Derived(8)` ×4 в (3b) — парк `NeedAttestation` с повторным derive; не дефект стенда, только расхождение с текстом доки.
- `executor.rs:7448` `guard2_convergence_mismatch_engages_safety_halt` зелёный на `land_on_import = false` (`executor.rs:4093-4097`, `:4209-4216`) — прод-тест, Э6 П-6; R-006 это называет.
- `capture.rs` глобальный `INSTALLED` (`:35-45`): если другой модуль тестов первым поставит подписчика, `log_capture_live == false` и (3) молча пропустит лог-проверку. В `--lib testbed` прогоне такого модуля нет; при полном `cargo test` порядок не гарантирован — (3b) упадёт громко, (3) нет.
- Тестов стенда без фичи 27 — они гоняются в `make pr`; 8 за фичей — только по явной команде. Нормально, но регрессия Э4 по R-001/R-004/R-009 не входит в `make pr`.

## 9. Три самых опасных пункта

1. Нет теста, в котором аутентификация прыжка ПРОХОДИТ на подменённом состоянии (вариант А R-001; `fakes.rs:944-954`). Если не закрыть до 4.2 — «комитет из локального finalized» будет реализован без единого red→green свидетеля, а C9/R-001 останутся зелёными по форме вызова.
2. `JumpElSync` — ручная модель `sync_to` с `[ГИПОТЕЗА]` в коде (`fakes.rs:1280-1285`) и без SYNCING-навсегда. Если не закрыть (одноузловой reth-conformance или живой Ex-2-подобный прогон трёх исходов) — П-4 переставит `sync_to` и аутентификацию, проверив это против модели, то есть повторит R-006 на новом месте.
3. Фикстуры с жертвой, прикованной к лжецу (R-004, R-009, R-001) держат `heights == 95/5` и после правильной П-4; ярлыки веток R-008/R-001 напечатают «PATH NOT REACHED» на исправленной системе. Если не переписать до начала — исполнитель Э4 увидит частично красный набор с вводящими в заблуждение печатями и «починит» тесты, а не прочитает их.

## Прямые ответы

- **Держится ли «Э3 закрыт, стенд готов быть регрессией для Э4/Э5»?** Да с оговорками. Стенд — регрессия для: П-9 (R-002 оба звена, ассерты инвертируются), R-006 сц. 1 (П-6), `deliver`-половины П-4 (R-009 `:3503`), фронтир-половины П-4 (R-004 `:2961`), Д-3/П-2 (R-008 `:2725`, с плохим ярлыком). Стенд НЕ регрессия для: центральной части П-4 (комитет из локального finalized / аутентификация до `sync_to`) — нет варианта А; половины «после» для R-004/R-009 (отверг → догнал) — одноисточниковые фикстуры; П-1 — состав комитета не зависит от хэша и якоря, веса и тумбстоуны фиксированы; всего, что идёт через `CertInlet::ingest`.
- **Сколько тестов из 35 опираются на фейк, который может отличаться от прода в ассерте?** 33 читают `FakeChain` (все, кроме (A) и C1). Уже: у 5 тестов несущий ассерт стоит на модели прыжка (`JumpElSync`/`ElNetwork`/`JumpCommittees`): B2, C9, R-008, R-004, R-001; у 2 — на ярусе канона фейка: (3), (3b); у 17 Live-тестов — на времени коммита `FakeStaking` (`committed_at`), которое не читалось против контракта.
- **Сколько статусов реестра переоценены?** Один по существу — R-008 («целиком»); один с неверным якорем — R-122 (имя теста). Остальные семь из проверенных десяти (R-001, R-002, R-004, R-006, R-007 — без изменений, R-009, R-121, R-123) — соответствуют.
- **Какой мутацией прод-кода начать проверять стенд?** `cold_start_jump.rs:687-692`: заменить `ensure!(latest.finalization.verify(ctx, &scheme, &Sequential), …)` на `let _ = latest.finalization.verify(ctx, &scheme, &Sequential);`. Ожидание: `a_lying_upstream_lands_a_divergent_branch_and_authentication_refuses_it` падает на `tests.rs:3229` (`all AuthFailed`), C9 и контроль остаются зелёными. Если R-001 не упал — стенд не различает включённую и выключенную аутентификацию, и всё сказанное про П-4 надо пересматривать. Вторая мутация — `plane_upstream.rs:206`: бинд высоты в `deliver` → R-009 падает на `:3503`.
- **Компакций:** 0.
- **Команда для длинных строк:** `sed -n A,Bp <файл>` и Read tool; для журналов `.dpos-study/` — `cat`/`sed -n`, ни одной `cut -c`/`head -c`.
