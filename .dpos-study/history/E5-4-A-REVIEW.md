# E5-4-A-REVIEW — финальное ревью захода А строки 5.4 (один клок бикона на `watch` marshal-tip; три фидера и tee сняты)

Ревьюер Opus 5, свежий контекст, 2026-09-15. База `HEAD = 08764870` (docs-only поверх кода `a1337ab2`),
объект — незакоммиченное дерево: 14 файлов под `crates/` (`git diff HEAD --stat -- crates`: 827+/919−).
Пути без префикса — от `crates/dpos/consensus/src/`. Агентов не запускал; git — только чтение; cargo —
три точечные мутации (§0.6) с откатом и md5. `md5sum -c gates/x2.md5` до ревью и после мутаций — все
14 файлов `ЦЕЛ`: читал то дерево, на котором прогнаны ворота `x2`.

**dsh-ревью `E5-4-A-DSH-1.md` в `history/` НЕТ** (`ls history/ | grep E5-4` → только `E5-4-A.md`,
`E5-4-B*.md`). Ревью сделано без него. `E4-ORCHESTRATOR.md` не открывал.

Теги: [KNOWN] — файл/строка открыт или команда прогнана в этой сессии; [ГИПОТЕЗА] — вывод.

## §0 Прямые ответы

### 0.1 Ворота `x2` (логи оркестратора, verbatim) [KNOWN]

`x2-status.txt` — все девять `exit=0`, `DONE` в конце.

| Ворота | Результат |
|---|---|
| `cargo test -p fluentbase-consensus --lib` | `test result: ok. 730 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 45.64s` |
| `… --lib --features dpos-devnet-byzantine testbed::` | `test result: ok. 57 passed; 0 failed; 0 ignored; 0 measured; 682 filtered out; finished in 52.53s` |
| `cargo test -p fluentbase-node --lib` | `test result: ok. 57 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.64s` |
| `cargo test -p fluentbase-staking-reader` | `test result: ok. 64 passed; 0 failed; …` (+ doc-test `0 passed; 0 failed; 1 ignored`) |
| `cargo test … --test slasher_integration` | `test result: ok. 16 passed; 0 failed; …` |
| clippy (3 крейта, `--all-targets`) | 2 чужих: `large size difference between variants` `crates/node/src/dpos.rs:1985` (`ValidatorUpstream`), `MutexGuard is held across an await point` `crates/dpos/staking-reader/src/epoch_transition.rs:3024`; те же два в `base-clippy.txt`. Своих 0 |
| clippy consensus `--features dpos-devnet-byzantine` | `exit=0`, предупреждений нет |
| `cargo fmt --check` | `exit=0`; в логе только nightly-шум `can't set wrap_comments…` |
| `cargo doc -p fluentbase-consensus --no-deps` | 57 warnings (HEAD `w3-doc.txt` — тоже 57), `unresolved link` — 6 (`Actor::enter` 1, `E` 2, `FakeMarshal` 2, `Scheme::verify_attestation` 1); единственная дельта против HEAD: ссылка `partition_prefix` → `crate::dpos::AGREEMENT_JOURNAL_PARTITION_PREFIX` стала `super::AGREEMENT_JOURNAL_PARTITION_PREFIX` (private-item link, как и была) |

**Дельта тестов по именам.** База: гейт `w3` — по `w3.md5` десять файлов побайтно равны `HEAD:` (проверил
`git show HEAD:<f> | md5sum` для каждого), `w3-lib.txt` = **733**, `w3-standf.txt` = 58. (`w2` = 732 —
это дерево ДО `a1337ab2`: `dkg_engine.rs`/`epoch_manager.rs` в `w2.md5` не совпадают с HEAD.)
Дифф по `#[test]`: снято 3 — `cert_inlet::tests::verified_cert_advances_the_dkg_deal_clock_tee`,
`sync_metrics::tests::dropped_height_ticks_are_counted`, стенд
`a_cert_inlet_is_refused_on_a_node_whose_beacon_drops_the_height_channel` (`#[should_panic]`);
`the_production_tee_wiring_feeds_the_dkg_clock_with_no_drain_of_ours` заменён 1:1 на
`a_catching_up_validator_deals_its_first_epoch_on_the_live_frontier_without_a_tee` (атрибут не в диффе);
переименованы без изменения счёта `the_tip_feeds_…` → `the_tip_is_published_on_the_watch_handed_in_…`,
`the_dkg_clock_gauge_is_the_actors_max_over_every_feeder` → `…_clamp_not_the_last_height_handed_in`,
`an_inlet_on_a_healthy_member_…_and_tees_it` → `…_and_hands_it_to_the_marshal`. Пофайлово `#[test]`:
`cert_inlet.rs` 27→26, `sync_metrics.rs` 12→11, `cert_inlet_tests.rs` 8→7. `--lib`: 733 − 3 = **730 ✓**;
стенд: 58 − 1 = **57 ✓** (`a_forged_seed_slot_…` под `cfg(feature)` в обеих версиях). Счёт сходится;
названное оркестратором «732 → 730» — база взята с `w2`, не с HEAD.

### 0.2 Живость клока — каждый путь постройки [KNOWN]

Конструкторы всех трёх типов перечислены `git grep`: `OuterBuilder {` — `dpos.rs:2912` (validator),
`:4074` (follower), `stand.rs:2962`; `ValidatorInputs {` — `node/dpos.rs:1868`, `stand.rs:2900`;
`FluentApp::new(` вне тестов — только `outer.rs:989`.

| Путь | sender создан | receiver актора | sender в `FluentApp` |
|---|---|---|---|
| Узел, validator | `node/dpos.rs:1323` `Arc::new(watch::Sender::new(0))` — ДО `beacon::build` | `:1899` `clock: beacon_tip.subscribe()` — единственный `subscribe()` на этом sender-е в дереве | `:1962` `SharedBeaconPlane.beacon_tip` (поле не-`Option`, `dpos.rs:1476`) → `plane.shared.clone()` (`:740-756`) → `launch` деструктурирует `:2069-2082` → `OuterBuilder.beacon_tip: Some(beacon_tip)` `:2936` → `outer.rs:1002-1005` `with_beacon_tip` → `marshal_reporter_app = app.clone()` `:1006` (клон ПОСЛЕ, Arc общий) → marshal `Reporters` `:1320/:1480` |
| Узел, follower (`launch_follower`) | — | нет плоскости, нет актора | `dpos.rs:4116` `beacon_tip: None`; app пишет только `ordering_tip` |
| Стенд `Beacon::Live` | `stand.rs:2785`, до `beacon::build` | `:2914` `clock: beacon_tip.subscribe()` | `:2957` `Some` iff `Beacon::Live` → `:2979` в `OuterBuilder` |
| Стенд `Beacon::Static` / `Role::AbsentBeacon` | `:2785` | актора нет | `Static` → `None`; `Live+AbsentBeacon` → `Some`, но приёмника нет — `send_replace` в канал без приёмников безвреден (tokio doc `send_modify`: «permits sending values even when there are no receivers», `tokio-1.52.3/src/sync/watch.rs:1073-1079`) |
| Рестарт (узел = новый процесс; стенд = `build_node` заново) | новая пара | новый | `Update::Tip` из архива marshal-а на старте: CW `marshal/core/actor.rs:397-402` (`get_latest` → `application.report(Update::Tip)`), открыл в чекауте `3c4e02c` |
| Промоушен follower→validator в процессе, verifier-режим | `FluentApp` один на процесс (`outer.rs:989-1006`), marshal — синглтон; промоушен клонирует app (`epoch_manager` `cfg.app`) — Arc тот же | — | тот же `:1110` |

Семантика `subscribe()`: «The most recent message is considered seen» (`watch.rs:1361-1362`) — приёмник
взят ДО первого `Update::Tip`, любой последующий `send_replace` (безусловный, `:1073`) будит первый
`changed()`. **Пути с неписаным клоком нет.**

### 0.3 Коалесцирование [KNOWN]

`on_height` (`actor.rs:2217-2394`) — всё, что он двигает, и от чего зависит:

| Шаг | Строки | Зависит от | Пропуск высот теряет действие? |
|---|---|---|---|
| clamp + гейдж | `:2222`, `:2228` | уровень `height` | нет |
| `reconcile_journals` (one-shot) | `:2237-2240` | `now` | нет |
| `decide_window(now)` | `:2251` → `decidable_epochs(now)` `:628-633` = `[max(BOOTSTRAP, now−R)..=now] ∪ {now+1}` | уровень `now` | нет: пропущенная эпоха попадает в trailing-окно и идёт в `recover` (acquire артефакта, R-121/R-122) |
| seal | `:2256-2263` `height >= epoch_start(e) − DKG_MARGIN_BLOCKS` | уровень | нет |
| `pending.retain(e > now)` | `:2310` | уровень | нет |
| `drive_finalization`, `confirmations.mint`, `announce_agreement_targets` | `:2325-2335` | состояние | нет |
| `sweep_epoch_state(now)` | `:2341` → `:2160-2188` `e + R >= now` | уровень | нет |
| `epoch_clock.send_if_modified(now)` | `:2350` → лаунчер `dkg_engine.rs:780-787` `on_tick(cutoff)` → `prune_agreements` `:339-343` `e < cutoff`, band `cutoff−SPAN..cutoff` `:366` | уровень | нет (см. «вне рамки» про band при прыжке ≥ SPAN — довзаходное) |
| `retransmit()` не-акнутых dealings | `:2369-2380` | «на каждом тике» | **действие не теряется, теряется частота**: в бурсте тиков меньше — это темп, а не пропуск; seal — по дедлайну |
| `drive_acquisition`, `fetch_missing_logs` | `:2389-2393` | состояние | нет |

Точечных проверок (`height == …`, `% interval`) в акторе нет (`grep` пуст). **Два ребра в одном
прыжке**: `now` уходит с E на E+2; `decide_window` решает `[E+2−R, E+2] ∪ {E+3}` — E+1 (был `Dealing`)
силится шагом seal, E+2 (никогда не стартовал) идёт в `recover` → `Acquiring`. Ровно то же было на
HEAD: фид уже был коалесцирован у источников (поллер `borrow_and_update` — HEAD `node/dpos.rs`, marshal
`Tip` только при `height > self.tip` — CW `:1454`, mpsc `try_send` с дропом), а clamp съедал
промежуточные. Регрессии по коалесцированию нет.

### 0.4 Tee снят — дедлайн и путь клока догоняющего [KNOWN]

Дилинг E+1 стартует в `decide_window` → `recover(now+1)` при входе актора в E; seal — `on_height`
шаг 1 `height >= epoch_start(E+1) − DKG_MARGIN_BLOCKS` (`:2261`, `DKG_MARGIN_BLOCKS = 20`, `:123`).
Клок догоняющего = tip его marshal-а, который двигает его же инлет:
`CertInlet::ingest` → `verify_block` (`cert_inlet.rs:756`/`:764`) → `report_finalization` (`:761`/`:765`)
→ marshal `Message::Finalization` (CW `actor.rs:567-604`: `find_block_by_commitment` — тело уже локально
после `verified` — → `store_finalization`) → `Update::Tip` при `height > self.tip` (CW `:1454-1458`) →
`FluentApp::report` (`application.rs:1096-1111`) → `beacon_tip.send_replace` (`:1110`). Цена против tee —
один оборот mailbox-а marshal-а плюс его `store_finalization` I/O; `report(Tip)` НЕ блокируется на
исполнителе (`self.executor.send` — не-`await` unbounded, `:1117`), так что при остановившемся
исполнении tip продолжает идти (до потолка окна чтения комитета — как и tee, тот же verify-гейт).

Обязательный тест `a_catching_up_validator_…` (`cert_inlet_tests.rs:1509-1687`): `CUT_AT = 8` (`:1516`),
`HEAL_ABOVE = 36` (`:1523`), окно `[32, 44)`; узел 3 с инлетом `CertInletSource::NextAboveTier` —
собственная frontier-плоскость узла (`stand.rs`, `CountingUpstream` над `PlaneUpstreamHandle`);
`partition(&[0,1,2], &[3]).after_height(8).heal_above(36)` (`:1538-1541`). Догон — через инлет
(plane-upstream) И marshal gap-repair одновременно, оба в один marshal; тест этого не разделяет и сам
это говорит (`:1478-1484`). Ассерции: премисса `:1560-1573` (`lag_tip < 32`, `36 ∈ [32,44)`); «дилил» —
`pinned_seats.contains(&seat)` `:1597`; share `dkg_ceremony_ok_total == 1` `:1614`; `dkg_clock <= ordering`
`:1627`, `== ordering` `:1632`, `>= 64` `:1636`; `!in_window.is_empty()` `:1649`.
**Чем он красный без замены** — см. §0.6: все три мутации валят его на `:1544` (`!out.timed_out`,
`heights [63,63,63,63]`), т.е. на живости всей сети, а не на ассерциях (2)–(5). Это находка F-01.

### 0.5 Два канала [KNOWN]

- Писатели: `ordering_tip.send_replace` — только `application.rs:1105`; `beacon_tip.send_replace` — только
  `:1110`, тем же `if let Update::Tip` (`:1096`). Оба в одном арме, порядок фиксирован. Расхождения
  значений быть не может: один `height.get()`.
- `beacon_tip == None` на валидаторном пути невозможно: `SharedBeaconPlane.beacon_tip` не `Option`
  (`dpos.rs:1476`), `launch` кладёт `Some` (`:2936`); `None` только `launch_follower` (`:4116`) и стенд
  `Beacon::Static` (`stand.rs:2957`).
- Паркующиеся приёмники (`git grep 'changed()\|wait_for('`): `ordering_tip` — ровно один,
  `epoch_manager.rs:683` `tip_wake.changed()` (`self.tip` только `borrow()`, `:860`, `:882`; подписка
  `:531`, клон `:591`); `beacon_tip` — ровно один, `actor.rs:1244`; других `subscribe()` на этих sender-ах
  нет (`git grep beacon_tip\|ordering_tip`). Инвариант «один паркующийся приёмник на канал» держится для
  обоих каналов захода.
- Механизм круга 3 подтверждён в реестре: `tokio-1.52.3/src/sync/watch.rs:420-424`
  `notified()` → `thread_rng_n(8)`; `notify_waiters` `:405-409` по индексу.

### 0.6 Тесты и мутации [KNOWN]

Таблица §0.4 журнала (16 строк) сверена с диффом `cert_inlet_tests.rs`/`cert_inlet.rs`/`stand.rs` —
каждая строка соответствует коду. Снятые утверждения и что потеряно:

| Снято | Что было свойством | Потеряно? |
|---|---|---|
| `dpos_dkg_height_drops_total == 0` ×3 (стр. 3, 7, 10) | «форвард в lossy mpsc ничего не потерял» | нет — `watch::send_replace` не может отказать; «актор перестал брать» теперь виден как `dkg_clock < ordering` и ассертится `== ordering` |
| `verified_cert_advances_the_dkg_deal_clock_tee` (стр. 14) | «верифицированный кормит канал, отклонённый — нет» | нет: отклонённый → 0 marshal-вызовов (`wrong_signature_cert_skips…` `cert_inlet.rs:1197-1199`), верифицированный → `["verified","report"]` (`matching_epoch_cert_…` `:1108`, позитивный контроль `boundary_cert_defers…` `:1175-1179`). Нюанс: снятая негативная половина была про ЧУЖОЙ комитет на большей высоте, замена — про плохую подпись; тот же fault-арм [ГИПОТЕЗА] — F-10 |
| `the_production_tee_wiring…` (стр. 15) | развилка `Observed/Production` | развилки нет — тика нет; свойство «порядок тика против marshal» стало «клок = tip», ассертится гейджами |
| `a_cert_inlet_is_refused…` (стр. 16) + `assert!` в `stand.rs` | честность drop-счётчика на `Static`/`AbsentBeacon` | счётчика нет; `Static`+инлет теперь допустимая конфигурация; `clock_pair` (`:88-99`) паникует `expect`-ом, а не врёт |

Тавтологий не нашёл; единственная избыточность — `dkg_clock <= ordering` перед `dkg_clock == ordering`
(F-02). `clock_pair` читает `dpos_ordering_finalized_height`/`dpos_dkg_clock_height` из реестра узла;
в стенде это ОДИН `PlaneClock` — `stand.rs:2915` (`ValidatorInputs.plane_clock`) и `:2978`
(`OuterBuilder.plane_clock`) клоны одного значения; гейдж DKG пишет только clamp актора
(`record_dkg_clock` — один продакшн-вызов `actor.rs:2228`).

Мутации (`CARGO_BUILD_JOBS=12`, последовательно, точечный `cargo test … --features dpos-devnet-byzantine`,
откат `cp` из бэкапа, md5 до/после = `x2.md5`, все 14 `ЦЕЛ`; логи `scratchpad/mut/M{1,2,3}.txt`):

**M1** — `actor.rs:1245-1248`: `Ok(()) => { let _mutant_m1 = *clock.borrow_and_update(); }` (берёт
значение, `on_height` не зовёт; форма журнала «тело → `{}`» без `borrow_and_update` крутила бы
`select!` на неснятой версии, поэтому взял «пометить прочитанным, не применить»).
```
test application::tests::the_tip_is_published_on_the_watch_handed_in_and_on_every_later_subscription ... ok
test testbed::cert_inlet_tests::a_catching_up_validator_deals_its_first_epoch_on_the_live_frontier_without_a_tee ... FAILED
panicked at crates/dpos/consensus/src/testbed/cert_inlet_tests.rs:1543:5:
heights [63, 63, 63, 63] halted [] errors []
test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 737 filtered out; finished in 3.52s
```
**M2** — `stand.rs:2914`: `clock: { let (mutant_m2_tx, rx) = tokio::sync::watch::channel(0u64);
std::mem::forget(mutant_m2_tx); rx }` (третий канал, sender жив, никто не пишет).
```
test testbed::tests::restart_replays_key_and_seed_journals ... FAILED   (tests.rs:1840:5)
test testbed::cert_inlet_tests::a_catching_up_validator_deals_its_first_epoch_on_the_live_frontier_without_a_tee ... FAILED
panicked at crates/dpos/consensus/src/testbed/cert_inlet_tests.rs:1543:5:
heights [63, 63, 63, 63] halted [] errors []
test result: FAILED. 0 passed; 2 failed; 0 ignored; 0 measured; 737 filtered out; finished in 3.52s
```
**M3** — `application.rs:1109-1111`: `if let Some(_mutant_m3) = &self.beacon_tip {}` (запись в
`beacon_tip` снята).
```
test application::tests::the_tip_is_published_on_the_watch_handed_in_and_on_every_later_subscription ... FAILED
   panicked at crates/dpos/consensus/src/application.rs:2746:13: the receiver taken from the sender handed in was not woken
test testbed::cert_inlet_tests::an_inlet_on_a_healthy_member_verifies_every_height_it_is_fed_and_hands_it_to_the_marshal ... FAILED
   panicked at crates/dpos/consensus/src/testbed/cert_inlet_tests.rs:168:5: heights [63, 63, 63, 63]
test testbed::cert_inlet_tests::a_catching_up_validator_deals_its_first_epoch_on_the_live_frontier_without_a_tee ... FAILED
   panicked at crates/dpos/consensus/src/testbed/cert_inlet_tests.rs:1543:5: heights [63, 63, 63, 63] halted [] errors []
test result: FAILED. 0 passed; 3 failed; 0 ignored; 0 measured; 736 filtered out; finished in 3.57s
```
Все три красные. Зелёных мутаций нет; непокрытого свойства по этим мутациям нет. Что видно из
трёх прогонов: во всех случаях стенд гибнет на живости ВСЕЙ сети (клок стоит у всех четырёх), и
дискриминирующие ассерции обязательного теста (место в pinned-set, `dkg_clock == ordering`, окно
догона) ни разу не срабатывают — F-01.

### 0.7 Остаток поллера [KNOWN]

`node/dpos.rs:1465-1700`: тело цикла — `finalized_rx.borrow_and_update()` (`:1508`); geometry freeze
+ единственная публикация `geometry_tx.send_replace(Some(frozen))` (`:1572`; актор паркуется на первом
`Some`, `beacon/plane.rs:863-869`); первая `track_peers` (`:1618`); tombstone-watch
`tombstones.observe(&snap)` (`:1683`). Ни `beacon_tip`, ни `plane_clock`, ни ссылки на актора в теле нет.
Д-5.4А-1 держится: остаток поллера не кормит клок бикона ни одним путём. Дизайн это и предписывал
(`E5-BEACON-DESIGN.md:627` — «поллер `fin + K` как фидер (задача остаётся — она морозит геометрию)»).
`grep -rn --exclude-dir=.claude 'dkg_height\|note_height_drop\|LiveFrontierTee\|fin + K' crates devnet bins`
→ только комментарии (`cert_inlet.rs:1121` — историческая проза теста про §5.2; `plane.rs:539`,
`node/dpos.rs:1412-1417`, `cert_inlet_tests.rs:23,1454,1460` — описания снятого) и
`devnet/local-dpos-smoke/dpos_harness/cases/smoke/verdicts_fault.py:624-625`: **stale-атрибуция
подтверждена** — «`dkg_height = finalized + K` (`crates/node/src/dpos.rs`, the finalized-height
poller)»; значение `DKG_CLOCK_LEAD = RESULT_LAG_K` по-прежнему верно на узле в локстепе
(`fin + K == tip`), источник — теперь `FluentApp::report` (F-06).

### 0.8 E5-40 [KNOWN]

Четыре константы — `beacon/mod.rs:161-190`, все приватные (`const`, без `pub`). Читатели: `plane.rs:51-53`
(импорт `super::{ARTIFACT_JOURNAL_PARTITION, MINT_MEMO_PARTITION, SEED_JOURNAL_PARTITION}`),
`dkg_engine.rs:90` (`AGREEMENT_JOURNAL_PARTITION_PREFIX`). `git grep` по четырём именам и трём on-disk
строкам под `crates bins devnet` вне `beacon/` — пусто (tracked-файлы; `.claude/` в `.gitignore` — не
входит, там только доки). Реэкспорт из `dpos.rs` не нужен; `pub ARTIFACT_JOURNAL_PARTITION` снят
законно — внешнего читателя не было и на HEAD. Rustdoc-ссылки на приватные константы из pub-доков
`partition_prefix` (`plane.rs:574-577`) — были и на HEAD, счёт 57 = 57.

### 0.9 Гигиена [KNOWN]

Добавленных строк с `#[allow]` нет; `unwrap`/`expect` — только в `#[cfg(test)]`/`testbed`
(`application.rs:2739-2769`, `cert_inlet_tests.rs:88-99,:1585-1595`, `stand.rs:365`). Наружу `beacon/`:
`pub(crate) use super::actor::DKG_MARGIN_BLOCKS` в `#[cfg(test)] pub(crate) mod testing`
(`mod.rs:219-230`) — тестовый ярус; `pub clock` заменяет `pub heights`. Публичная поверхность крейта:
`with_beacon_tip` вместо `with_dkg_heights`, поля `OuterBuilder.beacon_tip`,
`SharedBeaconPlane.beacon_tip` вместо `dkg_height_tx`; `LiveFrontierTee`/`with_tee` сняты (в `bins/`
не использовались — `git grep` пуст). Мёртвых импортов нет (clippy 0 своих). Новых логов нет.
Док `ValidatorInputs.clock` — `plane.rs:527-548`, полный. Комментарии-change-notes с номером захода в
продакшн-коде — F-04.

### 0.10 Доки [KNOWN]

`grep -rn "dkg_height_tx\|LiveFrontierTee\|three feeders\|три фидера\|note_height_drop\|with_ordering_tip"
.claude/dpos_architecture .claude/DPOS_ARCHITECTURE_CHANGELOG.md` → все вхождения в записях с пометкой
REVISED/DELETED/REMOVED/HISTORICAL или в исторических записях `verified-against`/CHANGELOG (`00_preamble.md:668,978,2388-2394`,
`09_followers.md:646-656,811-834`, `08_…md:1331-1346,2264-2311`, CHANGELOG `:2253+`). Блок
`verified-against` обновлён (`00_preamble.md:7-63`, дата 2026-09-15, база `08764870`, круг 3 отражён).
Якоря: `beacon/plane.rs:544` → поле на `:548` (в доке поля); `beacon/actor.rs:1244-1250`, `:2222`, `:2228` ✓;
`epoch_manager.rs:531` ✓; `node/dpos.rs:1569` → `:1572`; `consensus/dpos.rs:2476` → `:2477`; CW
`:397-402`, `:1454-1458` ✓. Дрейф ≤ 4 строк (F-07). Одно завышение в `08…md:1336-1340` — F-05.
Файлы 06/12/00a — по `grep` содержат по одной записи 5.4-А; целиком не читал.

### 0.11 Hard-stop

`DECISIONS.md` править не нужно: клок как `watch` от marshal-tip и сохранение поллера как задачи уже
записаны в дизайне (`E5-BEACON-DESIGN.md:627`, строка PLAN `:723`); правило «один паркующийся приёмник
на `watch`» — инженерное, записано в `03_epoch_machinery.md:77-90` и в коде. BLOCKER-ов, на которые
§5.3 не отвечает, нет.

### 0.12 Вердикт: **КОММИТИТЬ**

Не читал/не проверял: dsh-отчёта нет; `E4-ORCHESTRATOR.md` (запрещён); `06/12/00a` доки целиком;
`epoch_manager.rs` кроме сайтов подписки (`git diff` по нему пуст); `bins/fluent` не собирается ни
одними воротами (зависит только от `fluentbase-node`, прямых ссылок на снятое API нет — `git grep`);
смоук `devnet/local-dpos-smoke` не гонялся; follower-путь (`launch_follower`) — только типами и
`node --lib` 57/0, живого сценария в стенде нет (журнал §0.9 п.5 — верно).

### 0.13 Слабее всего (ранжировано)

1. F-01 — обязательный тест под всеми тремя мутациями умирает на живости сети, не на своих
   дискриминирующих ассерциях; «догоняющий дилит благодаря marshal-tip» стендом не отделяется от
   «клок жив вообще».
2. F-03 — `dkg_clock == ordering` «в покое» — свойство планировщика (актор успел взять последний tip до
   снимка после `pred`), не инвариант; детерминированный раннер его держит (x2, 5/5 круга 3), но §3.7
   остаточная случайность `select!` в этом же тесте существует.
3. Порядковая регрессия «на один marshal-вызов позже» не измерена в блоках (журнал §0.9 п.4), а док
   `08` называет её измеренной (F-05).
4. Follower-путь и смоук — не прогнаны живьём.
5. F-08 — актор теперь выходит по закрытию канала вслед за последним `FluentApp`; на HEAD sender
   поллера жил всю жизнь узла. Не наблюдаемо в живом узле (совпадает с падением слоя), но это
   изменение против HEAD.

## §1 Находки

| id | серьёзность | file:lines | что не так | чем пытался опровергнуть | уверенность |
|---|---|---|---|---|---|
| F-01 | LOW | `testbed/cert_inlet_tests.rs:1543-1550`, `:1597`, `:1627-1636`, `:1649` | Под M1/M2/M3 тест красный только на `!out.timed_out` (`heights [63,63,63,63]`): клок стоит у ВСЕХ узлов, эпоха 2 не минтится, цепь встаёт на 63. Ассерции «место в pinned-set», «клок = tip», «окно догона» ни в одной мутации не срабатывают. Тест пинит живость клока через marshal у члена, разрезанного внутри окна, — реальное свойство, но не «tee заменён без потери» (в стенде и на HEAD Tip-фидер уже был; `fin + K`-фидера в стенде не было никогда). Мутация «клок стоит только у отстающего» в стенде сегодня не строится | три прогона §0.6; поиск per-node переключателя клока в `stand.rs` — нет | высокая |
| F-02 | NIT | `cert_inlet_tests.rs:201-213`, `:1626-1634` | `assert!(dkg_clock <= ordering)` сразу перед `assert_eq!(dkg_clock, ordering)` — первое следует из второго; отличается только текст сообщения («вернулся второй фидер») | — | высокая |
| F-03 | LOW | `cert_inlet_tests.rs:206-213`, `:1631-1634`; `stand.rs:1926-1936` | Равенство «в покое» читается сразу после `pred(&progress)` без settle-паузы: гейдж ordering пишется в `report` (`application.rs:1098`) до `send_replace`, актор берёт tip своим тиком позже. Сегодня стабильно (x2 зелёный; круг 3 — 5/5 одного бинарника), но это порядок планирования, а не инвариант; при остаточной случайности `select!` (§3.7) может однажды дать `dkg_clock = ordering − 1` | прочитал `drive` `stand.rs:1926-1945`: снимок после `break`, `ctx.sleep(POLL)` только между итерациями | средняя [ГИПОТЕЗА] |
| F-04 | NIT | `cert_inlet.rs:730-736`; `node/src/cert_inlet.rs:29-35`; `node/src/dpos.rs:765-773, :931-940, :1412-1418`; `stand.rs:302-303`; `cert_inlet_tests.rs:124-134, :1454-1461` | Change-notes с номером захода в коде («No beacon-clock tee here any more (5.4-А)», «It tees nothing any more (5.4-А)», «Since 5.4-А it feeds no clock», «5.4-А removed …») — описывают, чего больше нет, а не «почему код такой». Репозиторная норма это допускает (R-126, B1-14, §5.2 повсюду), поэтому NIT | — | высокая |
| F-05 | NIT | `.claude/dpos_architecture/08_node_integration…md:1336-1340` | «What is lost is ORDER — the clock now moves one marshal call later — and that is measured, not assumed: the stand test …» — тест не измеряет порядковую дельту (ни одной ассерции про задержку; журнал §0.9 п.4 прямо говорит «числа нет»). Завышение | прочитал ассерции теста | высокая |
| F-06 | NIT | `devnet/local-dpos-smoke/dpos_harness/cases/smoke/verdicts_fault.py:624-625` | Атрибуция «`dkg_height = finalized + K` (… the finalized-height poller)» устарела; значение верно (`fin + K == tip` в локстепе), источник — `FluentApp::report`. Файл вне списка записи захода — фиксирую как долг | `grep` по `DKG_CLOCK_LEAD` — используется в арифметике окон `:631+`, менять не надо | высокая |
| F-07 | NIT | `.claude/dpos_architecture/00_preamble.md:11-12, :44-45, :47` | Дрейф якорей: `plane.rs:544` (поле `:548`), `node/dpos.rs:1569` (`:1572`), `consensus/dpos.rs:2476` (`:2477`) | `sed -n` по каждой | высокая |
| F-08 | LOW | `beacon/actor.rs:1250` `Err(_) => break`; `node/src/dpos.rs:851` `drop(plane.shared)` | На HEAD sender `dkg_height_tx` жил в поллере всю жизнь узла — актор не видел закрытия канала. Теперь после `drop(plane.shared)` единственные sender-ы — клоны `FluentApp` внутри слоя (`outer.rs:1006`, `epoch_manager` `cfg.app`, marshal `Reporters`); уйдут они — актор выходит, `("beacon", …)` резолвится. Совпадает с гибелью слоя (супервизор всё равно валит узел), поэтому не наблюдаемо; но семантика «актор живёт, пока жив узел» стала «пока жив слой» | проследил владельцев `beacon_tip` (§0.2); в стенде sender — в `OuterBuilder` узла | средняя |
| F-09 | INFO | `E5-prompts/5.4-A-review-final.md` §0.1 | База `--lib` = 733 (`w3` = HEAD по md5), не 732 (`w2` — дерево до `a1337ab2`); дельта −3 сходится по именам | `w2.md5`/`w3.md5` против `git show HEAD:` | высокая |
| F-10 | NIT | `cert_inlet.rs:1185-1200` vs снятый `verified_cert_advances_the_dkg_deal_clock_tee` | Снятая негативная половина использовала сертификат ЧУЖОГО комитета (`committee(2)`) на большей высоте; названная замена (`wrong_signature_cert_skips…`) — испорченную подпись. Оба должны падать на verify под схемой эпохи в один арм [ГИПОТЕЗА]; отдельного теста «чужой комитет → 0 marshal-вызовов» в `cert_inlet::tests` я не нашёл | `grep 'committee(2)' cert_inlet.rs` — другие тесты используют для иных свойств | средняя |

### Оставить как есть

- Clamp `max` в `on_height` (`actor.rs:2222`): при одном монотонном фидере он избыточен как слияние,
  но тесты зовут `on_height` напрямую с убывающими высотами (`:7498-7540`), а гейдж должен быть clamp-ом
  — постановка это и требовала.
- Два гейджа `record_ordering_tip`/`record_dkg_clock` над одним значением (`sync_metrics.rs:338-348`):
  видимость вставшего актора — теперь единственная диагностика, которую давал drop-счётчик.
- Поллер как задача (Д-5.4А-1): три работы, ни одна не кормит клок (§0.7); дизайн §5.3 это предписывал.
- `Beacon::Static` + инлет допустимы (снят `assert!` в `stand.rs`): счётчика, который бы врал, больше
  нет; `clock_pair` `expect`-ом отказывает честно.
- Остаточный ~2–3% разброс `in_window` (§3.7 журнала): ассерций, зависящих от него, нет — проверил
  `:1649` (`!in_window.is_empty()`) и печать `:1671-1687`; сайт `select!` по коду не назову.
- F-08: изменение семантики выхода актора не наблюдаемо в живом узле; исправление (держать sender в
  плоскости) вернуло бы «sender, который никто не пишет» в узел — хуже, чем есть.
- Rustdoc private-item links `partition_prefix` → константы (`plane.rs:574-577`): были на HEAD, счёт
  не вырос.

## Вне рамки (одной строкой)

- Заявленный в `03_epoch_machinery.md:77-90` как глобальный инвариант «один паркующийся приёмник на
  `watch`» уже нарушен на HEAD латчем `SafetyHalt.engaged`: два `engaged_edge()` —
  `epoch_manager.rs:601` и `dkg_engine.rs:757` (`sync_metrics.rs:599-603` `subscribe()` +
  `wait_for`) — при срабатывании латча будятся в процессно-случайном порядке; либо сузить формулировку
  инварианта, либо это следующая цель того же класса (5.4-Б, не 5.4-А).
- `prune_agreements` band `cutoff − SPAN .. cutoff` (`dkg_engine.rs:366`): прыжок клока на ≥ SPAN эпох
  оставляет партиции ниже band-а невыметенными — довзаходное, коалесцирование его не меняет.
- `bins/fluent` не собирается ни одними воротами.
