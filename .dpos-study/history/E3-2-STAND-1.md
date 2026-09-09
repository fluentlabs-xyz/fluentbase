# Э3.2 — стенд, сессия 1: шаги 1, 6, 2 оценки разведки (2026-09-09)

Дерево `~/Work/fluentbase`, ветка `djadjka/dpos-reth-2.2-squashed`, старт с `213f1db1`.
Commonware `v2026.4.0` (`3c4e02c`); в `~/.cargo/git/checkouts/` этот коммит лежит ДВАЖДЫ — `monorepo-27b478c9bb41d208/3c4e02c` (его читала разведка и я, далее `CW:`) и `monorepo-9732103c47eb4665/3c4e02c` (его подставляет cargo в паниках и ошибках компиляции); содержимое по одному тегу, различий не искал.
Теги: `[KNOWN]` — прочитано/выполнено в этой сессии, `[LIKELY]` — согласуется, не проверено, `[ГИПОТЕЗА]` — вывод.
Компакции контекста за сессию не было; `TASK.md` в scratchpad перечитывать не пришлось. Длинные строки читал `sed -n`/`cat`, обрезал только вывод grep через `cut -c1-N` (в отчётах ниже — никогда для чтения кода).

## 1. Итог

[KNOWN] Три шага сделаны, три коммита, все по явным путям, без трейлеров:

| Шаг | Коммит | Что |
|---|---|---|
| A (шаг 6 оценки) — снять `external` | `3e37d505` `refactor(consensus): drop the external runtime feature by pacing FCU locally` | `Cargo.toml:33`, `Cargo.lock` (−1 строка: `pin-project` у commonware-runtime), `executor.rs` (импорт, баунд, ШЕСТЬ `.pace(...)` → `.pace_el_call(...)`, локальный трейт `PaceElCall`), `outer.rs` (импорт + пять баундов) |
| B (шаг 2) — префикс раздела по узлу | `19e677cf` `refactor(consensus): prefix journal partitions per node` | `engine.rs` (`EpochEngineConfig.partition_prefix`, `engine_partition(prefix, epoch)`), `beacon/dkg_engine.rs` (`agreement_partition(prefix, epoch)`, `AgreementConfig`/`AgreementPlaneConfig.partition_prefix`, тест литералов), `beacon/plane.rs` (`BeaconConfig.partition_prefix`), `epoch_manager.rs` (`Config.partition_prefix`, `prune_agreements(.., prefix)`), `outer.rs` (`OuterBuilder.engine_partition_prefix`), `dpos.rs` (два конструктора — `""`), `crates/node/src/dpos.rs` (`BeaconConfig` — `""`) |
| C (шаг 1) — скелет стенда | `87d9e2ba` `test(consensus): add the deterministic multi-node testbed` | `crates/dpos/consensus/src/testbed/{mod,fakes,capture,stand,tests}.rs`, `lib.rs` (`#[cfg(test)] mod testbed;`), `beacon/surface.rs` (гвард `no_file_outside_the_beacon_names_the_static_implementation` — исключение для `testbed/`) |

[KNOWN] Не сделано и почему:
- Шаги 3 (upstream-плоскость), 4 (beacon с живым DKG), 5 (фейк-стейкинг как машина состояний) — вне сессии по заданию; стенд крутит эпохи как beacon-INACTIVE со `StaticRandomness`, комитеты — снимками из расписания теста.
- Правки `.claude/dpos_architecture/` (00, 03, 08, 13, 15, TOC) сделаны в дереве в тех же изменениях, но в коммиты НЕ вошли: каталог `.claude/` в `.gitignore` (`.gitignore:46`), `git ls-files .claude/dpos_architecture` — 0 файлов. Это состояние дерева, не моё решение; так же было у всех предыдущих правок доков.
- Этот отчёт и строка в `PLAN.md` не закоммичены: в списке разрешённых коммитов их нет.
- Тест-заготовка (4c) `a_node_outside_the_tracked_peer_set_keeps_following_through_the_upstream_plane` — `#[ignore]` с причиной (шаг 3).

## 2. Гипотезы разведки, закрытые сессией

**§2 #1 — ускорение от снятия `external`: подтвердилась.** [KNOWN] Замеры `cargo test -p fluentbase-consensus` до/после шага A (один и тот же набор: 631 строка `test … ok/ignored`, `diff` пустой):

| Что | до (с `external`) | после |
|---|---|---|
| `--lib beacon::dkg_engine::tests` (8 тестов) | 21,44 с | 0,17 с (×126) |
| весь `--lib` (608 тестов) | 25,67 с | 13,07 с |
| `tests/slasher_integration.rs` (13) | 0,65 с | 0,02 с |
| стенд N=4, 6 блоков (разведка под `external`: 7,3 с) | — | 0,71–1,04 с реальных при 6 с виртуальных |

Причина по коду: `CW:runtime/src/deterministic.rs:392-399` — под `external` `advance_time` делает `std::thread::sleep(self.cycle)`; `:413-416` — `skip_idle_time` не пропускает простой; `:453-458` — `assert_liveness` не паникует. `cargo tree -e features -i commonware-runtime -p fluentbase-consensus` до правки: фича `external` включалась только `fluentbase-consensus` и тянула `pin-project`; после — не включается никем (grep по `Cargo.toml`/`*.rs` всего дерева: единственное упоминание было `consensus/Cargo.toml:33`).

**§2 #2 — коллизия разделов Storage: подтвердилась, и она НЕ маскируется.** [KNOWN] Тест `replay_over_shared_journals_records_the_collision`: N=4 с `shared_engine_partitions = true` (все узлы пишут в один `consensus_epoch_0`) проходит фазу 1 чисто (`[6,6,6,6]`, без ошибок, без halt), а фаза 2 (`Runner::from(checkpoint)`, те же ключи, все узлы заново) ПАНИКУЕТ в commonware: `replaying notarize from another signer` (`CW:consensus/src/simplex/actors/voter/round.rs:531`), затем `voter should not finish` (`CW:consensus/src/simplex/engine.rs:236`) — паника валит весь runtime. Механизм: журнал voter’а — append-only, живые движки его не перечитывают, поэтому пока все писатели живы коллизия невидима; при `init` каждый voter реплеит ОБЪЕДИНЕНИЕ голосов и находит чужую подпись. С префиксом (`replay_over_prefixed_journals_resumes_every_node`) все четыре поднимаются и идут дальше 6 → 12, хэши фазы 1 — префикс фазы 2. То есть «разные имена blob’ов» ничего не маскируют — реплей читает раздел целиком.

**§2 #3 — `tokio::time::timeout` в `plane_upstream.rs:280`: не дошли.** [KNOWN] Стенд передаёт `upstream: None::<NoUpstream>` в `OuterEngine::start`, `PlaneUpstreamHandle::fetch_one` не вызывается; ни одной паники «no reactor running» за сессию не было — потому что путь не достигнут, а не потому что он безопасен. Остаётся [ГИПОТЕЗА] разведки; это первая работа шага 3.

**§9 п.4 — детерминизм по view: подтвердилась.** [KNOWN] `the_same_seed_reproduces_the_view_leader_hash_trace_byte_for_byte`: seed=1 трижды — байтово одна последовательность `(height, view, leader_index, order digest, executed hash)` по узлу 0: `[(1,1,L3),(2,2,L2),(3,3,L3),(4,4,L1),(5,5,L1),(6,6,L3)]` — нуллификаций в ней нет (ИСПРАВЛЕНО после контр-ревью: первая редакция писала «блок 6 во view 7», это была трасса другого прогона — теста (3) с расходящимся узлом). seed=2 — другая: `[(1,1,L3),(2,2,L3),(3,3,L1),(4,4,L1),(5,5,L0),(6,6,L3)]`. Оговорка: ключи узлов тоже выводятся из seed (`stand.rs::keys`), так что различие seed=2 — и расписание, и материал ключей; сильное утверждение — тройное совпадение. Хэши фейка теперь зависят от консенсуса: derive = `keccak(order.digest() ‖ prev_randao(seed))`, замечание разведки §4 снято.

**§9 п.6 — `Pacer` через `Cell<C>`: подтвердилась, свойство не потеряно.** [KNOWN] `CW:runtime/src/utils/cell.rs:190-201` — `impl Pacer for Cell<C>` только делегирует `self.as_present().pace(latency, future)`; `CW:runtime/src/tokio/runtime.rs:773-785` — tokio-`pace` возвращает future как есть (комментарий «Execute the future immediately»). Значит в проде пейсинг FCU не делал ничего, и локальная `PaceElCall::pace_el_call` (identity) ему эквивалентна. Что теряется только в тестах: deterministic-`Waiter` (`CW:deterministic.rs:1462-1530`) сначала опрашивает future noop-waker’ом, ждёт `target = now + latency`, а если future всё ещё Pending — БЛОКИРУЕТ ОС-поток до его готовности; тесты executor’а звали его с `fcu_pace = 0` и готовыми фейками, набор и результаты тестов до/после идентичны.

**§2 #8 — «выбывший стоит из-за `upstream: None`»: опроверглась в этой форме.** [KNOWN] `epoch_boundaries_pass_with_a_shrinking_committee_and_a_tracked_dropped_node_follows`: `epoch_len=5`, комитет 4→3 с эпохи 1, все четыре узла остаются в tracked peer set (индекс 0) — четыре границы проходят, узел 3 ДОГОНЯЕТ цепь без upstream, `[24,24,24,24]` в lockstep (он верификатор без движка: `epoch_manager.rs` `is_member` по `peer_pubkey` снимка ⇒ `Role::Verifier`). [LIKELY] механизм — by-height resolver marshal’а; broadcast-плоскость (`buffered::Engine` на `BROADCAST_CHANNEL`, `outer.rs:833`) не исключена — метрик/логов по этому не снимал. Форма разведки `[29,29,29,4]` воспроизводится только когда peer set сужен до `committee[E]` И линки узла 3 сняты (`PeerSet::Committee`, тест `a_node_outside_the_tracked_peer_set_stands_at_the_boundary`: `[16,16,16,4]`); с суженным set, но живыми линками simulated-сеть доставляет по линку независимо от tracked set — узел 3 шёл за цепью (`[16,16,16,14]`). [ГИПОТЕЗА] проба разведки трекала комитет по эпохам без узла 3. Следствие для шага 3: upstream-плоскость нужна НЕзарегистрированному узлу (нет линков), а не выбывшему из комитета зарегистрированному.

## 3. Тесты стенда

Все — `crates/dpos/consensus/src/testbed/tests.rs`, deterministic без `external`, seed 1, латентность 10 мс, потерь 0. Реальное время — `Outcome.real_elapsed` одного прогона (`--test-threads=1`, debug-профиль; при параллельном прогоне 1,0–2,4 с).

| Тест | Что проверяет | Что упало бы при нарушении | Вирт. | Реал. |
|---|---|---|---|---|
| (1) `four_honest_nodes_finalize_six_blocks_in_lockstep` | N=4, 6 блоков, одна цепь | другой финализированный хэш на любой высоте (`assert_lockstep_except`), `timed_out`, любая строка ERROR | 6 с | 0,71 с |
| (2) `eight_honest_nodes_…` | N=8, то же | то же | 5,8 с | 2,03 с |
| (3) `one_divergent_deriver_is_isolated_and_safety_halts` | узел 2 деривит другой блок на h=3; `diverged == Some((2,3))`, `halted == [(2, ResultDivergence)]` (defect на h=6=3+K, `executor.rs:3247`), 0/1/3 lockstep до 9 | halt не сработал, halt на честном узле, честные разошлись, нет строки `SafetyHalt` в логе | 9,9 с | 0,91 с |
| (4a) `epoch_boundaries_pass_with_a_shrinking_committee_and_a_tracked_dropped_node_follows` | `epoch_len=5`, 4→3, все tracked: 4 границы (первые блоки эпох 1–4 во view 1), `[24,24,24,24]` | граница не пройдена, узел 3 не дошёл, две цепи | 24,2 с | 1,75 с |
| (4b) `a_node_outside_the_tracked_peer_set_stands_at_the_boundary` | то же с `PeerSet::Committee`: члены 16, узел 3 стоит на 4 | узел 3 идёт без единого пира | 16 с | 1,27 с |
| (4c) `…keeps_following_through_the_upstream_plane` | `#[ignore]` — цель шага 3 | — | — | — |
| (5) `a_two_two_partition_stalls_finalization_and_heals_into_one_chain` | `[0,1]│[2,3]` после h=3 на 5 leader-таймаутов: высоты на срезе и на восстановлении РАВНЫ `[3,3,3,3]` (срез 3 с, восстановление 11,8 с), потом одна цепь до 12 | половина 2-из-4 финализировала; две цепи после восстановления | 20 с | 1,17 с |
| (6) `the_same_seed_reproduces_the_view_leader_hash_trace_byte_for_byte` | seed 1 ×3 — байтово одна трасса (без нуллификаций); seed 2 — другая | любой недетерминизм, дошедший до голосования | 4×6 с | 0,72/0,55/0,55/0,55 с |
| (7) `replay_over_prefixed_journals_resumes_every_node` | стоп на 6 → `Checkpoint` → все узлы заново → 12, префикс хэшей совпал | узел не поднялся, вторая цепь | 6+6 с | 0,72 + 0,86 с |
| (7′) `replay_over_shared_journals_records_the_collision` | без префикса фаза 2 паникует `replaying notarize from another signer` | чистый реплей | 6 с | ~1 с |
| (eq) `a_vote_equivocator_does_not_stop_the_honest_majority` (`--features dpos-devnet-byzantine`) | узел 1 = `ByzantineMode::Equivocate`: честные три — 6 в lockstep, узел 1 остаётся на 0, halt нет, в логе одна строка `BYZANTINE: equivocating votes` | честные встали | 6 с | 0,72 с |

`Equivocate` подключён без прод-правок (`OuterBuilder.byzantine` за фичей уже был); `cargo clippy --all-targets --features dpos-devnet-byzantine` — чисто.

## 4. Прод-правки и доказательство нейтральности

| Файл | Изменено | Нейтральность |
|---|---|---|
| `consensus/Cargo.toml:33`, `Cargo.lock` | снята фича `external`; из lock ушла одна строка `pin-project` у commonware-runtime | `cargo test -p fluentbase-consensus`: те же 631 строка результатов (diff пустой); `cargo tree -e features -i commonware-runtime` — `external` больше никем не включается |
| `executor.rs` | импорт без `FutureExt`/`Pacer`; баунд `E` без `Pacer`; трейт `PaceElCall` (identity); шесть `.pace(&self.context, self.fcu_pace)` → `.pace_el_call(self.fcu_pace)` (`:1970, 2031, 2337, 2687, 3698, 3779` до вставки трейта) | tokio-`Pacer::pace` = identity (`CW:tokio/runtime.rs:773-785`), `Cell` делегирует (`cell.rs:190-201`); `grep -rn 'Pacer\|\.pace('` по `consensus/src` — 0 в коде (одно упоминание в doc-комментарии `PaceElCall`, `executor.rs:90`) |
| `outer.rs` | импорт и пять баундов без `Pacer` | то же |
| `engine.rs`, `beacon/dkg_engine.rs`, `beacon/plane.rs`, `epoch_manager.rs`, `outer.rs`, `dpos.rs`, `node/src/dpos.rs` | `partition_prefix` сквозь конфиги; `engine_partition(prefix, e)`, `agreement_partition(prefix, e)`; прод передаёт `""` (`dpos.rs:2660,3559`, `node/src/dpos.rs:1969`) | `grep -rn 'consensus_epoch_\|dkg_epoch_' consensus/src` — только два `format!("{prefix}…")` и тест литералов; тест `dkg_engine::tests::partition_is_disjoint_from_the_ordering_plane`: `engine_partition("",7) == "consensus_epoch_7"`, `agreement_partition("",7) == "dkg_epoch_7"`; полные ворота B зелёные (§ ниже) |
| Остальные `partition:` в `consensus/src` (grep) | НЕ трогал: архивы marshal (`outer.rs:433-504` — `{partition_prefix}-v2/v3-…`, префикс — аргумент `OuterBuilder.partition_prefix`), `slasher_wal_partition` (`outer.rs:664`), seed/key/artifact-журналы (`plane.rs:517,555,574` — константы `dpos.rs:116-125` передаются в `beacon::build` из прод-кода; стенд beacon-плоскость не поднимает), `{prefix}-application-metadata` (`dpos.rs:256`, только `launch`) — все уже строки от вызывающего, стенд передаёт свои `node{i}-…` | — |
| `beacon/surface.rs` (тест-гвард) | `testbed/` исключён из `no_file_outside_the_beacon_names_the_static_implementation` с обоснованием в doc-комментарии | прод-код не тронут; гвард по-прежнему падает на core-файле |

Ворота (все [KNOWN], команды из задания):

| Ворота | A | B | C |
|---|---|---|---|
| `cargo test -p fluentbase-consensus` | 608+3+5+13 / 0 (набор = база) | 608+3+5+13 / 0 (набор = база) | 617+3+5+13 / 0, 1 ignored (4c); lib 13,4 с; с `--features dpos-devnet-byzantine` стенд 10/0/1 |
| `cargo test -p fluentbase-node -p fluentbase-staking-reader -p fluentbase-p2p -p fluentbase-bls` | EXIT 0, 0 failed | EXIT 0 | EXIT 0 |
| `cargo check --workspace` | Finished | Finished | Finished |
| `cargo clippy -p fluentbase-consensus --all-targets` | 0 warnings | 0 warnings (+ `-p fluentbase-node`) | 0 warnings, и с `--features dpos-devnet-byzantine` |
| `rustfmt --check` затронутых файлов | чисто, кроме чужого хунка `executor.rs:4712` (есть на HEAD) | чисто, кроме чужого хунка `plane.rs:815` (есть на HEAD) | чисто (`testbed/*.rs`, `lib.rs`, `surface.rs`) |

Доки: `grep -rn 'external\|Pacer\|fcu_pace\|consensus_epoch_\|dkg_epoch_\|partition' .claude/dpos_architecture/` — `Pacer`/`fcu_pace`/`external`(runtime) не упоминались нигде; исправлены `03_epoch_machinery.md:12` (раздел `{prefix}consensus_epoch_{N}`, откуда префикс), `08_…:2211-2214` (сметание по имени через `agreement_partition(prefix, epoch)`; заодно мёртвые якоря `epoch_manager.rs:199`/`:1307` заменены на символы), `13_…` правило 23 (виртуальное время реально, `PaceElCall`), новый §15.a в `15_smoke_cases…` (таблица тестов стенда), три записи в `verified-against` (`00_preamble.md`), счётчики строк в `TOC.md` для 00/03/08/13/15. Остальные попадания grep (`12_…:64` artifact-partition, `04_…:342-345` архивы v2/v3, `09_…` «partition a peer», `05_…` hostname) — верны как были.

## 5. Что стенд НЕ показывает после этой сессии

- Reth: R-006 (guard #2 читает канонический хэш до FCU — фейк канонизирует при derive), R-015, R-031, R-043, R-074 — только 3.1. `FakeBeacon` всегда `Valid`: SYNCING/INVALID/transport-ошибки engine API не моделируются (у `executor::tests::FakeBeacon` ручки есть — в стенд не перенесены).
- Контракт: путь «контракт → снимок → граница» подменён ретранслятором (`stand.rs::build_node`, задача `boundary_relay`): cold-start `(0, committee[0])`, на последнем блоке эпохи E — `(E+1, committee[E+1])` из расписания; настоящий `EpochTransition` (пометка Full/парк на пустом снимке, `soft_enter_span` по реальному состоянию) не работает — шаг 5.
- Beacon: DKG, `AgreedArtifact`, σ через границу, `dkgQual`, дедлайны 30/45 с — `StaticRandomness` даёт σ по запросу для любой эпохи (`surface.rs:492-537` — «UNCONDITIONAL» арм); R-002, R-008 не проверяемы — шаг 4.
- Догон верификатора/незарегистрированного через upstream-плоскость (`FRONTIER_CHANNEL`, `plane_upstream`) — шаг 3; (4c) `#[ignore]`.
- R-020 в части write-behind на реальной ФС: `Checkpoint` deterministic — потеря страниц не моделируется, только «все задачи убиты + реплей».
- Всё через `authenticated::discovery`: срыв линков при выпадении из tracked set в стенде моделируется вручную (`PeerSet::Committee`), simulated-сеть сама этого не делает.
- Слэшинг живьём: `slasher_evidence: None` — улика `Equivocate` до слэшера вообще не доходит (в проде `Some(evidence)`, `dpos.rs:2699`); `NoSink` — лишь последний хоп; тумбстоуны пусты.
- Нагрузку/ресурсы при n=51 — не мерил (N=8 — 2 с реальных на 6 блоков).
- Метка узла в логах: захваченные WARN/ERROR без атрибуции (см. §8).

## 6. Оставлено как есть

- `fcu_pace: Duration` остался в `executor::Config`, `OuterBuilder`, `dpos.rs` (20 мс) — теперь мёртвая ручка (identity). Не удалял: задание просило обёртку «с той же семантикой», и удаление трогает `dpos.rs` и `crates/node`.
- Разведка §2 #1/#6 говорит «два вызова `.pace`» — их шесть (`executor.rs`); отчёт разведки не правил (архив).
- Два чекаута одного коммита commonware (см. шапку); `COMMONWARE_INTERNALS.md` ссылается на первый, cargo компилирует второй.
- Каталоги `.claude/session-reads/*.jsonl` внутри `crates/dpos/consensus/src/`, `.claude/dpos_architecture/`, `.dpos-study/` — попадают в grep по коду; исключал `--exclude-dir=.claude`.
- Чужие fmt-хунки `executor.rs:4712`, `plane.rs:809` (в списке шести файлов `PLAN.md` §1).
- `OuterBuilder.soft_enter_committees` doc-комментарий обещает «`None` ⇒ no catch-up span», а тип — `Arc<dyn Fn…>` без `Option` (`outer.rs:533-547,199-200`); стенд передаёт замыкание над расписанием.
- `TOC.md` счётчики строк были неверны и до меня (00: 793 при факте 1135); обновил только затронутые пять.
- `StaticRandomness::deal` раздаёт по `self.snap` (конструктор), а `signer_scheme` берёт место по переданному `snap` (`surface.rs:325-354,480-520`): при комитете 3 из 4 узлы используют 4-стороннюю раздачу с тремя местами — работает, потому что стенд даёт всем узлам ОДИН полный снимок. При шаге 4/5 это место надо помнить.
- `Runner::start_and_recover` возвращает `Checkpoint`; после паники в фазе 2 (7′) runtime-состояние потока не восстанавливается, следующий тест в том же потоке прошёл — не исследовал, что осталось в thread-local’ах.

## 7. Найденное вне объёма

- [KNOWN] Выбывший из комитета, но отслеживаемый узел догоняет цепь без upstream (§2 #8) — переоценка шага 3: он нужен для незарегистрированного узла и для крайнего случая «все линки старого комитета сняты».
- [KNOWN] `beacon::surface::tests::no_file_outside_the_beacon_names_the_static_implementation` — гвард против утечки фикстуры; любой будущий тестовый модуль вне `beacon/`, использующий `StaticRandomness`, его уронит (теперь есть исключение только для `testbed/`).
- [KNOWN] Узел-эквивокатор (`Inner::Equivocate`) не финализирует ничего сам (высота 0 при 6 у честных) — `VoteEquivocator` подменяет весь движок; для сценариев 3.3 с эквивокатором, который должен ещё и следовать цепи, нужна другая точка подмены.
- [KNOWN] На реплее (7) фаза 2 стартует из того же `StandConfig`: `last_execution_finalized_height: 0`, `initial_finalized`/`initial_head` = генезис, `marshal_floor: Some(0)`, `CanonicalInMemoryState::empty()` — то есть «процесс, забывший свою высоту», а не честный рестарт; поднимается всё равно: executor передеривает 1..6 из архива marshal в свежий `FakeChain` (хэши совпали). Resume-семантика проверена слабее, чем читается из §3. Фидельность к reth здесь нулевая — resume-семантика executor’а против реального reth — тема 3.1.
- [LIKELY] Панику `replaying notarize from another signer` можно получить и в проде при двух процессах над одним datadir — но на диске раздел = каталог, второй процесс упрётся раньше в блокировку reth; не проверял.
- [KNOWN] Tier-S (`spec_executed_hash`) на высоте, которую узел ещё не финализировал, легитимно хранит нуллифицированного сиблинга (первый прогон (3): узлы 1/3 держали спекулятивный блок 9 из view 10, финализирован блок 9 из view 11) — любой межузловой чекер (и `agreement_check.py`, и soak-батарея `prev_randao byte-identity`) обязан сравнивать tier-F или высоты ≤ min finalized.
- [KNOWN] `commonware_p2p::simulated` доставляет по линку независимо от `track` — тест на «выпадение из peer set» без `remove_link` не моделирует authenticated-транспорт.
- [KNOWN] `Ed25519PrivateKey::random` требует `commonware_math::algebra::Random` в scope; `MuxerBuilderWithBackup::build` — трейт `commonware_p2p::utils::mux::Builder` (не реэкспортирован под другим именем).

## 8. Где проверка была самой слабой

1. Захват логов без метки узла: commonware вешает tracing-span только на `traced` spawn и не наследует его дочерними задачами (`CW:deterministic.rs:1157-1173`), поэтому `halted` берётся с типизированной защёлки `SafetyHalt` каждого узла, а строки WARN/ERROR — общим списком. Строка «SafetyHalt» в (3) проверена как «есть хотя бы одна», не «от узла 2».
2. (5): «нет финализации в партиции» — по выборке драйвера каждые 100 мс виртуальных и по tier-F; сообщение в полёте, финализировавшее блок ровно в момент среза, не наблюдалось при seed 1 (равенство строгое), при других seed не гонял.
3. (6): различие seed=2 — в основном материал ключей; чистое «то же ключи, другой seed runtime» не измерял.
4. (eq): проверяет только, что честные идут; что делает slasher с уликой (кроме одной строки лога) — не смотрел.
5. Реплей (7): фаза 2 поднимает свежий `FakeChain`, т. е. проверяет реплей ЖУРНАЛОВ консенсуса, а не согласованность journal↔EL-состояние.
6. `#[ignore]` (4c) никогда не исполнялся; его тело — ожидание, не наблюдение.
7. Замер dkg-тестов — один прогон до и один после; разброс не оценивал (на 21 с против 0,17 с он не важен).
8. Ворота C гонялись дважды: первый раз — с красным гвардом `surface.rs`; второй, полный (тесты крейта, четыре крейта, `check --workspace`, clippy с фичей и без) — пока в дереве ещё лежали пять чужих файлов, переформатированных моей ошибкой (`rustfmt lib.rs` форматирует и дочерние модули; правки только whitespace, файлы восстановлены из HEAD через `git show HEAD:… >`), а третий, на точном закоммиченном дереве — только `cargo test -p fluentbase-consensus` + clippy. Итог `git status`: чужие `devnet/local-dpos-smoke/**` не тронуты, других изменений нет.

## 9. Handoff — с чего начинать шаг 3

- Точки в `testbed/` под upstream: `stand.rs::build_node` — `outer.start(.., None::<NoUpstream>)`; `fakes.rs::NoUpstream` — заменяется на `PlaneUpstreamHandle` поверх `FRONTIER_CHANNEL` (в стенд добавить шестой канал и `plane_upstream::new_bridge` на каждом узле). `StandConfig.peer_set = PeerSet::Committee` даёт «незарегистрированный» узел без линков — под него и (4c).
- Первое, что задеть: `plane_upstream.rs:280` `tokio::time::timeout` под deterministic без tokio-рантайма (§2 #3 разведки) — ожидаемая паника при первом poll; замена на `select!{ ctx.sleep(..), rx }` (образец `beacon/artifact.rs:952-954`).
- Временное в стенде (уходит с шагом 5): `boundary_relay` (`stand.rs::build_node`), `Committees::Schedule`, `SnapshotReader`; с шагом 4: `StaticRandomness::build(CHAIN_ID, full_snapshot)` и отсутствие `beacon::build` (`dkg_height_tx: None`, `agreement_intake: None`).
- Сравнение узлов — только tier-F (`FakeChain::tip/hash_at`); не переключать на tier-S.
- Драйвер (`stand.rs::drive`) — единственное место, где можно менять сеть/peer set во времени: партиции и `PeerSet::Committee` уже там; роли задаются до старта (`Stand::node(i).role(..)`). Роли 3.3 (двойной `Reveal`, ложный `Latest`, пара h−1, вздутая проба) — через `fakes.rs` или обёрточный `Sender` в `build_node`.
- Прогон: `cargo test -p fluentbase-consensus --lib testbed -- --nocapture --test-threads=1` (≈13 с с компиляцией); с эквивокатором — `--features dpos-devnet-byzantine`.

## 10. Контр-ревью (агент на Opus, 2026-09-09; его отчёт — пересказ, отмечено, что я перепроверил)

Подтверждено агентом своими прогонами (я не перепроверял, кроме отмеченного): гигиена коммитов (нет трейлеров, `devnet/**` не тронут, пять чужих файлов байт в байт), `cargo check --workspace --all-targets` зелёный, все числа §3 воспроизвелись, `diverged` в (3) считается по tier-F, узел 3 в (4a) — верификатор без движка, в (7′) архивы marshal остаются per-node (коллизия — именно voter-журнал).
Находки агента, принятые (правки в этом файле/доках сделаны, в коде — ждут отдельного коммита):
1. [KNOWN, перепроверил по своим логам] трасса seed=1 заканчивается `(6,6,L3)`, не `(6,7,L3)` — нуллификации нет; исправлено в §2, §3, доке §15.a.
2. [KNOWN] тест (4a) проверяет `views.len() >= 3` при заявленных четырёх границах; doc-комментарий говорит «three». Код не менял.
3. [KNOWN] «через by-height resolver» стояло как `[KNOWN]` без проверки — понижено до `[LIKELY]` здесь и в доке.
4. §8.2 названа не та слабость (5): срез/восстановление снимаются до `remove_link`/`add_link`, равенство строгое; настоящая оговорка — `FakeChain::tip()` это исполненный ярус, а не консенсусная финализация.
5. [KNOWN] реплей (7) стартует из `StandConfig` с нулевыми стартовыми высотами (поле `last_execution_finalized_height`, не `last_consensus_finalized_height`) — исправлено в §7.
6. [KNOWN] `slasher_evidence: None` — улика до слэшера не доходит; исправлено в §5.
7. `first_divergence` считает большинство через `HashMap` — при ничьей результат зависит от `RandomState`. Код не менял.
8. Два независимых пути к одному префиксу агрегата (`BeaconConfig.partition_prefix` создаёт, `epoch_manager::Config.partition_prefix` подметает) ничем не связаны — в проде обе `""`.
9. §5 не перечислял: `QUOTA` без лимита, `disconnect_on_block: false`, `active_registry_peers → []`, `Address::ZERO`, `boundary_fetch/re_jump/feed: None`.
10. `for_views(k)` — k leader-таймаутов, не k view (doc-комментарий теста (5) неточен).
11. `pin-project = "1.1"` в `consensus/Cargo.toml:35` никем не используется (и до шага A).
12. Наблюдение `[16,16,16,14]` (суженный set при живых линках) не закреплено тестом.
Не принято/уточнено: п.15 агента (разведка §2 #8 сама называла альтернативу `peer_set(1)` без узла 3) — верно, вывод отчёта от этого не меняется.

