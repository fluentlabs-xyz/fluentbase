# E5-4-B-REVIEW — финальное ревью захода Б строки 5.4 (бикон владеет инстансами согласования; латч SafetyHalt многоадресный)

Ревьюер Opus 5, свежий контекст, 2026-09-15. База `HEAD = 79dd18f9`, объект — незакоммиченное дерево
(10 файлов под `crates/`, `git diff HEAD --stat -- crates`: 1477+/729−). Пути без префикса — от
`crates/dpos/consensus/src/`. Агентов не запускал; git — только чтение; cargo — три точечные мутации
(§0.5) с откатом и md5. `md5sum -c gates/w2.md5` до ревью и после мутаций — все 10 файлов `ЦЕЛ`, т.е.
читал то дерево, на котором прогнаны ворота `w2`. Второе dsh-ревью (`E5-4-B-DSH-2.md`, E-01…E-10)
есть — прочитано.

Теги: [KNOWN] — файл/строка открыт или команда прогнана в этой сессии; [ГИПОТЕЗА] — вывод.

## §0 Прямые ответы

### 0.1 Ворота `w2` (логи оркестратора, verbatim) [KNOWN]

`w2-status.txt` (строка `DONE` есть; `standf` первый прогон `exit=101` — OOM, перепрогон `exit=0 | 225s`):

| Ворота | Результат |
|---|---|
| `cargo test -p fluentbase-consensus --lib` | `test result: ok. 732 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 243.74s` |
| `… --lib --features dpos-devnet-byzantine testbed::` | `test result: ok. 58 passed; 0 failed; 0 ignored; 0 measured; 683 filtered out; finished in 222.61s` |
| `cargo test -p fluentbase-node --lib` | `test result: ok. 57 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 82.76s` |
| `cargo test -p fluentbase-staking-reader` | `test result: ok. 64 passed; 0 failed; …` + doc-tests `0 passed; 0 failed; 1 ignored` |
| `… --test slasher_integration` | `test result: ok. 16 passed; 0 failed; …` |
| clippy (3 крейта, `--all-targets`) | exit 0; ровно два чужих: `MutexGuard is held across an await point` → `staking-reader/src/epoch_transition.rs:3024`, `large size difference between variants` → `node/src/dpos.rs:2007`; своих 0 |
| clippy `--features dpos-devnet-byzantine` | exit 0; 0 строк `warning`/`error` |
| `cargo fmt --check` | exit 0 (только 5 `Warning: can't set … unstable` от rustfmt.toml) |
| `cargo doc --no-deps` | exit 0; `unresolved` = 6 (`base-doc.txt` — тоже 6), всего 57 warnings, все `links to private item` / `unresolved` — прежние |

Прирост 725 → 732 = +7 по именам (`git diff` по `fn `, `#[test]`-счётчики: dkg_engine 8→16,
epoch_manager 8→5, sync_metrics 11→12, actor 94→95) [KNOWN]:
- перенос ±0: `epoch_manager::tests::{agreement_instances_prune_on_the_engine_cutoff_and_are_aborted, pruning_reclaims_the_journal_partitions_an_abort_left_behind, a_repeat_prune_at_a_cutoff_already_swept_does_not_touch_storage}` → `dkg_engine::tests` (`:1326`, `:1378`, `:1432`; первый переименован `…_on_the_clock_cutoff_…`);
- +5 `dkg_engine::tests`: `a_halted_node_starts_no_agreement_instance_and_aborts_the_ones_it_has` (`:2238`), `a_latch_that_engages_during_the_spawn_retires_the_instance_it_started` (`:2309`), `the_launcher_task_starts_on_a_request_prunes_on_the_clock_edge_and_aborts_on_exit` (`:2368`), `a_halt_edge_aborts_the_running_instance_without_a_clock_tick_and_the_task_keeps_answering` (`:2436`), `a_latch_engaged_before_the_launcher_is_built_refuses_the_first_request` (`:2504`);
- +1 `sync_metrics::tests::every_engaged_edge_waiter_resolves_and_a_late_one_resolves_at_once` (`:855`);
- +1 `actor::clock_tests::the_agreement_clock_moves_on_the_epoch_edge_and_never_per_tick` (`:7933`).

### 0.2 Перенос как есть [KNOWN]

- `prune_agreements` — `diff` тел HEAD `epoch_manager.rs` ↔ `dkg_engine.rs:332-379`: ровно два слова в комментариях (`run_agreement`→`spawn_agreement`, `adopted`→`started`). `AGREEMENT_SWEEP_SPAN` (`:315`) = `SCHEME_RETENTION_EPOCHS`, как на HEAD. Три теста — клетка в клетку (diff: имя первого, `use` подняты на уровень модуля, три комментария).
- `on_tick` (`:957-966`) — только `prune_agreements`. `on_halt` (`:973-982`) — abort+join всех, `mem::take`.
- Момент sweep. HEAD: cutoff = `epoch` из `reconcile_roles` = `live_epoch_of(tip)` (`epoch_manager.rs:1754-1759`: `last(e) == tip ⇒ e+1`), `abort_below(epoch)` `:1020`. Новое: `now = self.epoch_of(height)` (`actor.rs:2226`) над монотонным `max` фидеров (`:2218`), публикуется `send_if_modified` (`:2347`). Для одного и того же tip: cutoff `E` у HEAD на `last(E−1)`, у нового на `first(E)` = +1 высота. **Согласен** для фидеров tip/poller (`fin+K` ≤ marshal tip). Tee — Д-5.4Б-3, принято.
- Частота. HEAD прунил per reconcile, но cutoff меняется только на ребре эпохи, а sweep гейтился `cutoff > swept_to || aborted_one` — те же две кромки, что и сейчас. Инстанс с target < cutoff аборт-ится на первом ребре, где cutoff его перерос — момент тот же (+1 высота). **Единственная разница по времени жизни** — инстанс, стартовавший ПОСЛЕ того, как cutoff прошёл его target (`on_request(e)` при `e < *clock`): HEAD снимал его на следующем reconcile (≈блок), новое — на следующем ребре часов (≤ 1 эпоха, не «более чем на эпоху»). Когда это возможно: только после рестарта (`Launcher.started` пуст), для эпохи `e < now` в состоянии `Agreed` (`recover` `actor.rs:3276-3283`: `(dealing_closed, Some(set), _) ⇒ Agreed`; `Sealed` для `e < now` недостижим — `past_boundary` `:3231` даёт `Acquiring`). Такой инстанс проигрывает свой журнал с сертификатом, берёт тело из `ArtifactStore` (`dkg_engine.rs:602-608`) и завершается сам — [ГИПОТЕЗА] практически мгновенно. Актор это оговаривает (`actor.rs:2340-2345`). F-09, оставить.
- Партиций, снесённых раньше HEAD (не через tee), не нашёл: band `[cutoff−8, cutoff)` тот же, кромка +1 высота.

### 0.3 Латч [KNOWN]

- Свойства HEAD сохранены: идемпотентность и «ровно один раз» — `latch` `sync_metrics.rs:544-548` (`send_if_modified(|e| !mem::replace(e, true))`, модификация под write-lock watch-а, `tokio-1.52.3/src/sync/watch.rs:1171-1208`); первый reason — `self.reason.set(reason).is_ok()` не тронут; синхронный restore из маркера — `restoring` → `restore_marker` → `latch` (диф не трогает `restore_marker`); gauge `safety_halt_engaged.set(1)` `:545`.
- `borrow()` через `await` — нигде: `is_engaged` = `*self.engaged.borrow()` (`:609-611`, копия bool, guard умирает в том же выражении); `engaged_edge` — `let _ = edge.wait_for(..).await` (`:622-626`): `Ref` дропается сразу, после него нет await.
- Клоны делят один канал: `#[derive(Clone)]` → `watch::Sender::clone` = `shared.clone()` + `ref_count_tx` (`watch.rs:201-209`). `Default` для `Sender<T: Default>` есть (`watch.rs:211-215`) — derive корректен.
- Циклы не крутятся вхолостую и не поллят завершённый future: `tokio::select!` — precondition `false` ⇒ бит в `disabled`, будущее «still evaluated but never polled» (`tokio-1.52.3/src/macros/select.rs:28-31`, `:618-630`, `:691-692`). `&mut halt_edge` при `halt_seen` — только reborrow. Менеджер `epoch_manager.rs:601-603`, `:621-622`; лаунчер `dkg_engine.rs:757-759`, `:780-783`. Мутация M4 (арм снят) — два теста красные по виртуальному таймауту/stall (§0.5).
- `engaged_edge` при взведённом до постройки латче: `subscribe()` берёт текущую версию (`watch.rs:1387-1394`), но `wait_for_inner` (`watch.rs:896-914`) проверяет предикат на текущем значении ДО ожидания (`if !closed || has_changed`) ⇒ резолв с первого poll. Тест `:2504` + `sync_metrics.rs:855`.
- Окно `start_one` (четыре `register_dkg_subchannel(..).await`, `:1023`): закрыто перечитыванием в ветке `Started::Running` `:942-944`; инстанс к этому моменту **уже в карте** — `instances.insert` `:929` стоит выше перечитывания, `on_halt` берёт `mem::take(&mut self.instances)`. M7 (перечитывание снято) — тест `:2309` красный.
- После `on_halt` новый запрос не спавнит: `start` `:898-905` читает латч до чтения комитета и оседает target в `started`. Тесты `:2436` (events=3, reads=1), `:2504` (reads=0).
- Все выходы лаунчера abort-ят инстансы: `requests.recv() → None` и `clock.changed() → Err` ⇒ `break` (`:765`, `:774`) ⇒ `abort_all` (`:794`, `:986-991`); внешний abort — инстансы спавнятся из `self.ctx` = ctx задачи лаунчера (`Launcher::new(ctx, …)` `:751`, `start_one(&self.ctx, …)` `:914-915`), потомки ⇒ каскад (`monorepo-…/3c4e02c/runtime/src/lib.rs:187-206`).
- Остаточный зазор (E-01 dsh, согласен, не BLOCKER): латч, взведённый между перечитыванием `:942` и следующим `select!`, либо во время `on_tick`/ветки `NotAMember|Failed`, обслуживается на первой итерации, где выигрывает арм `halt_edge`; выбор ветки случайный (`select.rs:669-741`), так что при очереди запросов — конечное число холостых итераций (каждая — отказ спавна или прунинг). F-01.

### 0.4 Проводка [KNOWN]

- `git grep "agreement_intake\|AgreementClock\|Wiring.safety_halt\|with_agreement_intake\|recv_agreement" -- crates bins` — пусто (кроме `halt_marker` follower-пути, ниже). `Notify`/`AtomicBool` в `sync_metrics.rs` — 0 вхождений.
- `SyncMetrics::register` — валидатор: один вызов `node/dpos.rs:1460-1461` (в `build_beacon_plane`, `ctx` тот же, что уходит в `launch_dpos_layer` `:736-740`); `DposLayer::launch` больше не регистрирует (`consensus/dpos.rs:2139-2147` — только комментарий). Follower: свой один вызов `consensus/dpos.rs:3312-3313` — путь не изменён (`halt_marker` `FollowerLayerConfig` `:3155`, `cert_follow/mod.rs:227-230`). Стенд: `stand.rs:2412-2413`, по узлу.
- Один латч на процесс: `node/dpos.rs:1462` → `ValidatorInputs.safety_halt` `:1926` и `SharedBeaconPlane.{sync_metrics, safety_halt}` `:1984-1985` → `DposLayer::launch` деструктурирует `:2125-2126` → executor/менеджер/супервизор (прежние места, `safety_halt` та же переменная). Стенд: `halt` `stand.rs:2414` → `ValidatorInputs` `:2939` и `OuterBuilder.safety_halt` `:3025` — тот же.
- `plane.rs`: `Tasks{supervised, drain}` `:174-183`, `epoch_clock` `watch::channel(0u64)` `:679`, лаунчер получает `epoch_clock_rx, safety_halt` `:930-931`, `#[cfg(test)] probe: None` `:919-920`; лаунчер по-прежнему в `supervised` (`:1012`). `build_follower` латча не получает — Д-5.4Б-2.

### 0.5 Тесты и мутации [KNOWN]

- Режим падения `runtime timeout`/`runtime stalled` — виртуальное время `deterministic::Runner::timed(600s)` (`deterministic.rs:567` / `:458`), реальное — доли секунды (M4: 0.29 s wall). Настоящий горячий цикл (форма M5′ журнала) — зависание в реальном времени, ловится только внешним `timeout`; тесты этой формы не пришпиливают (E-10 dsh, согласен, NIT).
- `#[cfg(test)] probe` в feature-сборку стенда не течёт иначе как под `cfg(test)`: `testbed` — `#[cfg(test)] mod testbed;` (`lib.rs:73-74`), стенд гоняется как `--lib`. `dkg_engine` — приватный модуль (`beacon/mod.rs:73`), `LauncherProbe` наружу не виден.
- Может ли новый тест быть зелёным при сломанном свойстве: `a_latch_engaged_before…` считает ровно 2 события (внутренний счётчик) — хрупко, но свойство «reads == 0, инстанса нет» держится и без него. `the_launcher_task_…` exit-половина прошла бы и без `abort_all` (супервизия добивает потомков) — E-02 dsh, согласен, NIT.
- Мутации (по одной, `CARGO_BUILD_JOBS=12`, `cargo test -p fluentbase-consensus --lib beacon::dkg_engine`; pristine md5 `dkg_engine.rs` = `f106c86d6ca67f12f4e642d882bd2cf1`, после каждого отката `ЦЕЛ`; итоговый `md5sum -c w2.md5` — все `ЦЕЛ`; логи `scratchpad/rev-mut-{M4,M7,M1p}.txt`):

| # | Мутация | Результат (verbatim) |
|---|---|---|
| M4 | `:780` `if !halt_seen => {` → `if false => {` | `test result: FAILED. 14 passed; 2 failed` — `a_latch_engaged_before_the_launcher_is_built_refuses_the_first_request ... FAILED` (`panicked at …/runtime/src/deterministic.rs:458:9: runtime stalled`), `a_halt_edge_aborts_the_running_instance_without_a_clock_tick_and_the_task_keeps_answering ... FAILED` (`deterministic.rs:567:25: runtime timeout`) |
| M7 | `:942` `if self.safety_halt.is_engaged() {` → `if false && …` | `test result: FAILED. 15 passed; 1 failed` — `a_latch_that_engages_during_the_spawn_retires_the_instance_it_started ... FAILED`, `panicked at …/dkg_engine.rs:2343:13: an instance spawned under a latch that engaged during its registrations survived the request that started it` |
| M1′ | `:366` band `..cutoff` → `..=cutoff` | `test result: ok. 16 passed; 0 failed` — **ВЫЖИЛА**. Верхняя граница band-а не пришпилена (унаследовано от HEAD: тесты перенесены клетка в клетку). F-02 |

### 0.6 Гигиена [KNOWN]

- `#[allow]` новых нет (единственный — прежний `too_many_arguments` на `spawn_agreement_launcher` `:726`). `unwrap`/`expect` вне тестов в диффе — нет (`expect("four routes")` `:1036-1039` — HEAD). Новых `pub` наружу `beacon/` нет: `agreement_partition` стал приватным, `pub(crate) use` снят (`mod.rs`), `AGREEMENT_JOURNAL_PARTITION_PREFIX` — `pub(crate)` (`dpos.rs:230`). `cfg(test)`-поле в продакшн-структуре — `AgreementPlaneConfig.probe` (`:691-692`), по образцу `Wiring.fixture` (F-06). Мёртвый код: `abort_below` остался `async` без единого `await` (`epoch_manager.rs:1460-1484`) — F-03. Уровни логов: `warn!` на отказ спавна — один раз на target (оседает в `started`), `warn!` на abort по халту, `info!` на прунинг — уместно.

### 0.7 Доки [KNOWN]

- `verified-against` в `00_preamble.md:7-40` — запись круга 2 добавлена, запись круга 1 помечена «[round 2: …]». Якоря круга 2 проверены `sed -n`: `sync_metrics.rs:442/547/609/621(±1)/854`, `epoch_manager.rs:601/621`, `dkg_engine.rs:757/780/957/973/942/890/692/2309/2368/2436/2504`, `actor.rs:818/2347/7933`, `plane.rs:544`, `dpos.rs:230/1533`, `node/dpos.rs:1462` — все на месте. Якоря круга 1 (тот же коммит) дрейфуют: `prune_agreements :329`→332, `agreement_partition :281`→284, `Launcher :767`→802, `on_request :844`→879, `on_tick :904`→957, `:850`→898; `08:2659` `spawn_agreement_launcher :708` (док-коммент; fn на 727), `13:614` `on_request :850`. F-05.
- `grep -rn "agreement_intake\|AgreementClock\|notify_one\|Wiring.safety_halt" .claude/dpos_architecture` — все хиты либо явные «DELETED/used to» записи (`00:21,66-69,1304`, `08:1237,2663-2667`), либо ДРУГИЕ `notify_one` (`03:286`, `08:1211,2417,2875`, `09:226`, `13:739`). `.claude/DPOS_ARCHITECTURE_CHANGELOG.md:16-54` — две записи 5.4-Б. `.claude/` в gitignore (`.gitignore:47`) — в коммит не входит, drift-правило выполняется по дереву.

### 0.8 Hard-stop

`DECISIONS.md` править не требуется: halt-постура (R-128: исполнение стоит, marshal раздаёт, метрики видны) не изменена — изменена только доставка ребра. BLOCKER-ов, на которые не отвечает дизайн, нет.

### 0.9 Вердикт: **КОММИТИТЬ**

Не читал: `E4-ORCHESTRATOR.md` (запрещено), `E5-ORCHESTRATOR.md` (карта), `E5-BEACON-DESIGN.md` §5.3, `E5-prompts/5.4-B-{impl-1,fix-2}.md`, тело `spawn_agreement` `:398-590` целиком, `executor.rs` места `is_engaged` (только `grep`), §1 ответов DSH-1 (только таблица §2), разделы 03/09 доков целиком (только строки с `2026-09-15`). Cargo — только три мутации; полные ворота — по логам `w2` оркестратора.

### 0.10 Слабее всего (ранжировано)

1. F-01 — зазор случайного выбора ветки `select!` после халта (согласен с E-01 dsh, не BLOCKER); один `biased;` закрыл бы.
2. F-02 — верхняя граница band-а не пришпилена (M1′ выжила); при регрессии `..=cutoff` sweep снёс бы партицию живого-но-ещё-не-перезапущенного инстанса cutoff-эпохи после рестарта (журнал simplex ⇒ риск двойного голоса). Унаследовано от HEAD.
3. F-09 — инстанс для `Agreed`-эпохи ниже cutoff после рестарта живёт до ребра часов, а не до следующего блока; [ГИПОТЕЗА] завершается сам по журналу, живьём не проверял.
4. F-03/F-04/F-05 — доковый и тестовый drift внутри самого коммита.

## §1 Находки

| id | серьёзность | file:lines | что не так | чем пытался опровергнуть | уверенность |
|---|---|---|---|---|---|
| F-01 | LOW | `beacon/dkg_engine.rs:760-783`, `:942-944`, `:947-951` | Латч, взведённый после перечитывания `:942` (или во время `on_tick`, или в ветке `NotAMember`/`Failed`), снимается на первой итерации, где `halt_edge` ВЫИГРЫВАЕТ случайный выбор `select!`; при очереди запросов инстансы живут конечное число холостых итераций. Тот же паттерн у менеджера (HEAD). | `select.rs:669-741` — порядок ветвей случайный; каждая проигранная итерация — отказ спавна (`:898`) или прунинг; ребро не теряется (`wait_for` резолвит с текущего значения). | высокая (код), низкая (значимость) |
| F-02 | LOW | `beacon/dkg_engine.rs:366`; тесты `:1378`, `:1432` | Верхняя граница band-а `..cutoff` не пришпилена: M1′ (`..=cutoff`) — 16/0 зелёный. | Прогнал мутацию (§0.5). Оба теста держат cutoff-эпоху в карте или не создают её партицию. HEAD-тесты те же. | высокая |
| F-03 | NIT | `epoch_manager.rs:1460-1484`, `:1410-1412` | `abort_below` остался `async fn` без `.await` (единственный await был `prune_agreements`); док `:1411` «`abort_below(...).await` that does file I/O» — устарел. E-06 dsh. | `awk`/`grep await` по телу. | высокая |
| F-04 | NIT | `beacon/dkg_engine.rs:1415-1416`, `:1458-1459` | Док перенесённого теста: «prunes on EVERY tick of the actor's epoch clock — which is every finalized height», сообщение assert-а «…removals per block» — противоречит часам «только на смене эпохи» (`actor.rs:2347`, `Wiring::epoch_clock` `:807-818`). | Прочитал `send_if_modified` и тест `:7933`. | высокая |
| F-05 | NIT | `.claude/dpos_architecture/00_preamble.md:42-55`, `08_…:2659`, `13_…:614` | Якоря круга 1 дрейфуют на 3…36 строк внутри того же коммита (`:329/:281/:767/:844/:904/:850/:708`). Запись круга 2 точна. | `sed -n` по каждому. | высокая |
| F-06 | NIT | `beacon/dkg_engine.rs:688-704`, `:747-750`, `:785-793`; `plane.rs:919-920` | `#[cfg(test)]` поле и блок публикации в продакшн-структуре/задаче. По образцу `Wiring.fixture`; наружу не виден (`mod dkg_engine` приватный). E-05 dsh. | `lib.rs:73-74`, `beacon/mod.rs:73`. | высокая |
| F-07 | NIT | `sync_metrics.rs:609-611`; `executor.rs:1366,1666,1844` | `is_engaged` — read-lock watch-а вместо atomic load; executor читает на каждом dispatch. Не измерялось. E-04 dsh. | Uncontended `RwLock::read` — один CAS; писатель пишет один раз за жизнь. | средняя (не измерял) |
| F-08 | NIT | `beacon/dkg_engine.rs:310` | Строка комментария 129 символов (rustfmt stable комментарии не переносит). | `awk length>100`. | высокая |
| F-09 | NIT | `beacon/dkg_engine.rs:879-905`; `actor.rs:2340-2345`, `:3276-3283` | После рестарта инстанс для `Agreed`-эпохи `e < now` спавнится и до ребра часов не прунится (HEAD снимал на следующем reconcile). | Только после рестарта (`started` пуст); `Sealed` для `e<now` недостижим (`past_boundary`); инстанс сам завершается по журналу — [ГИПОТЕЗА]. Одна строка `if target < *clock.borrow() { return }` закрыла бы, но это редизайн запроса. | средняя |
| F-10 | NIT | `sync_metrics.rs:544-548` | По журналу К2.6 (не перепрогонял) мутация M6 `send_replace(true)` зелёная: «публикуется ровно один раз» не пришпилено. Безвредно — оба ждущих обезоруживаются после первого срабатывания. | Relayed из журнала; код `send_if_modified` прочитан. | средняя (relayed) |

### Оставить как есть

- Д-5.4Б-1/-2/-3 — не переоткрываю; круг 2 хуже не сделал: D-01/D-06/D-12/D-13 касаются только фидера tee, который круг 2 не трогал (`actor.rs:2218-2226` без изменений).
- `on_halt`/prune join-ят инлайн в задаче лаунчера (D-09/E-07): аборт-нутый инстанс ничего не ждёт от лаунчера, join нужен для корректности sweep-а (`:352-361`).
- `abort_all` без join на выходе (D-08): за ним нет sweep-а, потомки добиваются супервизией.
- Повторный `on_halt` на пустой карте после перечитывания `:942` (E-08): идемпотентен, и именно он обезоруживает арм.
- `Default` derive на `SafetyHalt` с `watch::Sender<bool>` — корректен (`watch.rs:211-215`).
- `a_latch_engaged_before…` считает `events == 2` — хрупко к рефакторингу счётчика, но не к поведению.

## Вне рамки (одной строкой)

`biased;` в `select!` лаунчера с `halt_edge` первым (F-01) и тест на верхнюю границу band-а (F-02) — оба мелкие, в отдельный follow-up вместе с 5.4-А; `abort_below` → синхронная (F-03).
