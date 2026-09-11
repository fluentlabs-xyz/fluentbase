# Ревью строки 4.1, заход Б1 — модуль `committee/` в узле, follower'е, слэшере и стенде

Ревьюер: Opus 5, свежий контекст, 2026-09-11. База — `bf6d9f5e` (`git rev-parse HEAD`),
объект — незакоммиченное рабочее дерево (`git status --short`: 20 `M`, из них
`.dpos-study/history/E4-ORCHESTRATOR.md` — правка оркестратора, игнорируется; под
`devnet/` изменений нет). Пути без префикса — `crates/dpos/consensus/src/`;
`node/` = `crates/node/src/`; `reader/` = `crates/dpos/staking-reader/src/`.
Номера строк без пометки — ДЕРЕВО; `HEAD:` — `bf6d9f5e`.
Теги: `[KNOWN]` — прочитано/прогнано мной в этой сессии; `[ГИПОТЕЗА]` — вывод по коду
без прогона.

---

## §0. Прямые ответы

### 1. Ворота — прогнаны мной, все зелёные [KNOWN]

Verbatim — §4. Сводка по одной строке:

| ворота | ожидание постановки | результат |
|---|---|---|
| `cargo test -p fluentbase-consensus --lib` | 664 база + новое | **665 passed; 0 failed** |
| `… --features dpos-devnet-byzantine testbed::` | 40/0 | **40 passed; 0 failed** (633 filtered out) |
| `cargo test -p fluentbase-node --lib` | 57/0 (59 − 2) | **57 passed; 0 failed** |
| `cargo test -p fluentbase-staking-reader` | 60/0 | **60 passed; 0 failed** (+ 0/0/1 ignored — doc-тесты) |
| `cargo test -p fluentbase-consensus --test slasher_integration` | — | **13 passed; 0 failed** |
| `cargo clippy -p fluentbase-consensus -p fluentbase-node --all-targets` | — | exit 0, одно предупреждение |
| то же `--features dpos-devnet-byzantine` | — | exit 0, то же одно предупреждение |
| `cargo fmt --check` | — | exit 0, `grep -c "Diff in"` = **0** |
| `cargo doc … \| grep -c "unresolved link"` | 9 | **9** |

**Расхождения с журналом — два, оба косметические [KNOWN]:**

1. Журнал `.dpos-study/history/E4-1-B1.md:329` даёт verbatim фичевого clippy как
   `cargo clippy -p fluentbase-consensus --all-targets --features dpos-devnet-byzantine`
   с результатом «0» — пакет `fluentbase-node` из команды выпал, а ворота (постановка
   Б1.7, `4.1-B1-impl-1.md:74`) требуют «и с фичей» от той же пары пакетов. Я прогнал
   пару: результат тот же (одно чужое `large_enum_variant`), скрытой поломки нет, но
   verbatim журнала не соответствует названным воротам.
2. Единственное предупреждение clippy — `large_enum_variant` на
   `pub(crate) enum ValidatorUpstream` (`node/dpos.rs:1873`), вариант
   `Plane(PlaneUpstreamHandle<Context>)` ≥ 232 байт. Этот enum диффом не тронут
   (`git diff HEAD -- crates/node/src/dpos.rs` не содержит ханка около `:1873`), так что
   предупреждение чужое [ГИПОТЕЗА: что оно было и на HEAD — по отсутствию правки, не по
   прогону HEAD].

Счёт тестов сходится ровно: consensus 664 → 665 (+1 новый в `committee/tests.rs`),
node 59 → 57 (−2 удалённых теста курсоров). См. §0.7.

### 2. Один якорь [KNOWN]

**Сколько `FinalizedCursor` рождается.** В продакшн-коде ровно два вызова
`FinalizedCursor::default()` — по одному на класс узла:
`node/dpos.rs:1432` (валидатор, в `build_beacon_plane`) и
`node/cert_follow/mod.rs:127` (follower). Третье вхождение,
`application.rs:1237`, — внутри `#[cfg(test)] mod tests` (`application.rs:1210`).
Конструктор `ProviderExecutedChain::new` УДАЛЁН; остался единственный
`with_cursor(provider, finalized)` (`node/ordering.rs:39`), так что второй курсор
больше нечем завести — проверено `git grep "ProviderExecutedChain::new"`: ноль
вхождений вне доков.

**Держат ли `RethAnchor` и `ProviderExecutedChain` ОДИН курсор — да, прослежено по
клонам, не по журналу.** Валидатор: `finalized_cursor` (`node/dpos.rs:1432`) →
`.clone()` в `RethAnchor::new` (`:1446-1449`) → оригинал уезжает полем
`BeaconPlane::finalized_cursor` (`:1857`, объявление `:1039`) → `.clone()` на вызове
`launch_dpos_layer` (`:792`) → параметр `:1936` → `ProviderExecutedChain::with_cursor`
(`:2031`). `FinalizedCursor` — `Clone` над `Arc<AtomicU64>` (`application.rs:151-153`),
так что все клоны — один атомик. Follower: создаётся `node/cert_follow/mod.rs:127`,
`.clone()` в `with_cursor` (`:128-131`), оригинал — поле
`FollowerLayerConfig::finalized_cursor` (`:235`, объявление `dpos.rs:2756`), в
`launch_follower` перемещается в `RethAnchor::new` (`dpos.rs:3126-3129`). Тот же
атомик.

**Три вызова `anchor_advanced` строго ПОСЛЕ `advance_finalized` — открыл все три
[KNOWN]:**

| # | `advance_finalized` | `anchor_advanced` | между ними |
|---|---|---|---|
| init (посев) | `executor.rs:1109-1110` | `executor.rs:1115` | только комментарий |
| посадка прыжка | `executor.rs:2482` | `executor.rs:2485` | только комментарий |
| финализированный derive | `executor.rs:3600` | `executor.rs:3605` | только комментарий |

Ни в одном нет `?`, `return` или ветвления между парой.

**Четвёртой точки нет [KNOWN].** `git grep "advance_finalized"` по `crates/` даёт ровно
эти три вызова плюс определения трейта (`application.rs:136`), продакшн-реализацию
(`node/ordering.rs:71-73` → `self.finalized.advance(height)`) и стендовую
(`testbed/fakes.rs:670`). `FinalizedCursor::advance` вызывается ТОЛЬКО из этих двух
реализаций плюс `executor.rs:4238` (тестовый фейк `ExecutedChain`, определён `:4193`) и
`application.rs:1240/1249/1253` (тест). Продакшн-курсор валидатора и follower'а
двигает только `ProviderExecutedChain::advance_finalized`, который зовёт только
executor, и каждый его вызов озвучен.

### 3. Стартовое окно [KNOWN + ГИПОТЕЗА]

**До `executor::Actor::init` якорь = 0.** Курсор засевается `cfg.last_consensus_finalized_height`
(`executor.rs:1109-1110`), который возвращает `MarshalActor::init` внутри
`OuterBuilder::build` (`outer.rs:904`). До этого `FinalizedCursor::height()` = 0.

**Кто в этот момент читает через модуль и что получает:**

| потребитель | читает ли через модуль | что получает при якоре 0 |
|---|---|---|
| beacon-плоскость: `committee_for`/`committee_pair_for`/`committee_source` (`beacon/plane.rs:586-596`) | ДА (фасад) | `read_at()` = `executed_state_hash(0)` = хэш ГЕНЕЗИСА (не `None`!); затем `committee(E)` ⇒ `OutOfWindow{lo:0,hi:2}` для любой эпохи > 2 ⇒ фасад `None` (`committee/facade.rs:47-54`) |
| beacon: `frozen_dkg_qual` (`beacon/plane.rs:598-607`, `beacon/follower.rs:124-134`) | ДА | `probe(...)` ⇒ `None` ⇒ мемо НЕ пишется (`beacon/carry.rs:236-237`: `let (bit, committed) = probe(epoch, at)?;` стоит ДО `cache.insert`) |
| `evidence_committee_for` (`node/dpos.rs:1718-1725`) | ДА | `.ok()` ⇒ `None`, батч республикации пропускается |
| слэшер `resolve_committee` (`slasher/actor.rs:710-724`) | ДА | `OutOfWindow{epoch > hi}` ⇒ `is_transient() == true` (`committee/mod.rs:216`) ⇒ `Transient` ⇒ повтор |
| tombstone-поллер (`node/dpos.rs:1620-1655`) | НЕТ — свой `tombstone_reader` | не затронут |
| ET (`reader/epoch_transition.rs`) | НЕТ — свой reader | не затронут |
| громкий отказ старта (`dpos.rs:1995`) | НЕТ — свой reader | не затронут |

**Необратимого потребителя на `NotReadable` при старте НЕТ [KNOWN].** Три кандидата
проверены по коду: (а) мемо `frozen_dkg_qual` замерзает только при `Some` от probe —
фасад отдаёт `Some` только при наличии записи (`committee/facade.rs:88-90`), так что
`false` замёрзнуть не может; (б) отказ старта идёт мимо модуля (см. §0.10); (в) у
слэшера `Permanent` даёт только `OutOfWindow` НИЖЕ окна и `Read(permanent)` — при
якоре 0 эпоха всегда ВЫШЕ окна ⇒ `Transient`. Граница ET не пропускается: она
собирается ET'шным reader'ом, не модулем.

**`GeometryRx` (Д-15).** Пока `geometry_tx` не послал `Some`, `committee()` возвращает
`NotReadable{ready_at: 0}` на шаге 0 (`committee/store.rs:369-376`) — без лока, без
якоря, без EVM. Публикует геометрию ровно одна точка: поллер плоскости, ветка
`cold_start` (`node/dpos.rs:1563-1571`). Проверил, что дырки «заморозили мимо
поллера» нет: `et_arc` создаётся свежим (`node/dpos.rs:1390-1412`), и
`cold_start(` в `node/dpos.rs` вызывается только из поллера (`git grep` по файлу:
`:1563` — единственный вызов), так что `frozen_before` на первом тике всегда `false`.
[KNOWN]

**Кто зависел от чтения комитета ДО заморозки геометрии — сравнение с
`HEAD:node/dpos.rs:1383-1500`.** На HEAD `PlaneCommitteeReads::read_at`
(`HEAD:node/dpos.rs:1422-1435`) читал `block_hash(max(fin, live))`, где
`fin = provider.finalized_block_number()` — ПЕРСИСТЕНТНЫЙ reth-тег, доступный сразу
после старта процесса, без всякой геометрии и без executor'а. В дереве та же
плоскость слепа, пока не выполнены ОБА условия: (а) ET заморозил геометрию,
(б) executor засеял курсор. Разбор последствий — находка **B1-02** (§1): узел
восстанавливается сам на первом финализированном derive, но окно новое и журналом
названо только наполовину (§8.4 журнала говорит про геометрию, про курсор — нет).

### 4. Поведение по каждому переведённому носителю против HEAD [KNOWN]

**`PlaneCommitteeReads` → фасад.** Теряется: (а) `live`-курсор — по проекту принято
(§5.1 «Якорь после 3-го прохода», полоса (ii)); (б) **сверх проектного — персистентный
reth-тег как источник высоты**: HEAD брал `finalized_block_number()` (переживает
рестарт), дерево берёт `FinalizedCursor` (0 до `Actor::init`). Это не «полоса (ii)», это
другой класс — см. B1-02. (в) `read_at()` больше никогда не `None` при живом провайдере
на высоте 0: `executed_state_hash(provider, 0)` отдаёт генезис-хэш
(`executed.rs:45-58`: `0 > best` ложно, `block_hash(0)` = `Some`). Безвредно, потому что
значение `at` фасад игнорирует, а окно отсекает эпоху раньше.

**`FollowerCommitteeReads` → фасад.** Ушла дыра `committee() => None`
(`HEAD:dpos.rs:3446-3452`) — инертно: `build_follower` проецирует только
`committee_bls` и `dkg_qual` (`beacon/follower.rs:120-133`), `committee` не зовёт никто.
Курсор был `get_finalized_num_hash().hash` (тоже персистентный) — та же смена, что у
валидатора, тот же B1-02.

**`evidence_committee_for` → модуль, memo снят.** Одна строка
(`node/dpos.rs:1724`). Memo одной эпохи заменён картой модуля — строго лучше
(HEAD мемоизировал одну эпоху, модуль держит всё окно). Регресса не нашёл: оба
складывают ошибку в `None`.

**Tombstone-поллер: `executed_state_hash(fin)` вместо `block_hash(fin)`.** Регресса на
бэкфилле НЕТ [KNOWN]. HEAD: `block_hash(fin)` на бэкфилле отдавал заголовок (reth пишет
заголовки впереди состояния), затем `epoch_committee_snapshot(epoch, hash)` падал
`StateNotMaterialized` каждый тик → ветка `Err(e) => debug!` (`HEAD:node/dpos.rs:1741-1745`).
Дерево: `executed_state_hash` отдаёт `Ok(None)` при `fin > best` (`executed.rs:49-51`),
тик пропускается молча. Наблюдаемое поведение одинаково (тумбстоун не обнаружен ни там,
ни там), EVM-вызовов меньше. «Паркуется» — да, но парковался и раньше, только шумнее.
Одно ухудшение молчания — NIT B1-11.

**Слэшер `resolve_committee`.** Маршрутизация — по `CommitteeError::is_transient()`
(`slasher/actor.rs:712-724`), а тот (`committee/mod.rs:213-219`):
`NotReadable ⇒ true`; `OutOfWindow ⇒ epoch > hi` (ВЫШЕ окна — транзиентно, НИЖЕ —
постоянно); `Read(e) ⇒ e.is_transient()`. То есть ровно как спрашивает вопрос:
`NotReadable`/`Read(transient)` ⇒ `Transient`, `OutOfWindow` ниже / `Read(permanent)` ⇒
`Permanent`, `OutOfWindow` ВЫШЕ окна ⇒ `is_transient() == true` ⇒ `Transient`
(Д-12 захода А). **Но** `Read(e)` расширил класс постоянных против HEAD — B1-01, и
транзиентность пустого комитета сделала голову очереди блокируемой — B1-03.

**Watchdog удалён — ничего его не звало и не ждало [KNOWN].** На HEAD это был
`drop(ctx.with_label("committee_watchdog").spawn(...))` (`HEAD:dpos.rs:2344-2408`) —
хэндл сразу выброшен, никто его не держал. `git grep "committee_watchdog"` по дереву —
ноль. Потеря — единственная операторская диагностика «финализация стоит, и этого ключа
нет в комитете»; проектом (§5.5) её удаление предписано, замены нет — B1-09.

**Follower `committee_at` из записи (Д-18): найден КАЖДЫЙ читатель `tombstoned` и
`activation_epoch` на пути `boundary_rx` → `reconcile_roles` → … [KNOWN].**
`git grep "tombstoned"` по `crates/dpos/consensus/src` + `crates/node/src`: единственный
читатель поля — `slasher/tombstone.rs:45` (`TombstoneSet::observe`,
`.filter(|member| member.tombstoned)`), и его продакшн-вызывающий ровно один —
tombstone-поллер плоскости (`node/dpos.rs:1627`), которого у follower'а нет
(`dpos.rs:3439` отдаёт `TombstoneSet::default()`). `git grep "activation_epoch"`:
шесть вхождений, все — конструкции, и все шесть в `#[cfg(test)]` (`beacon/artifact.rs:1092`
под `:1013`, `beacon/follower.rs:673` под `:575`, `beacon/surface.rs:1365` под `:1252`,
`slasher/tombstone.rs:88`, `testbed/fakes.rs:920`, `committee/tests.rs:49`) — проверил
принадлежность к тест-модулям по ближайшему `#[cfg(test)]` выше. Читатели снимка на пути
границы: `epoch_manager.rs:1057` (`snap.validators.len()`), `soft_enter_verifier`
(`scheme.rs:57-68` — `epoch_committee_from_snapshot`, то есть `validators`),
`constant_fallback_seed` (`beacon/seed.rs:93-105` — `epoch` + `peer_pubkey`),
`WeightedVrf::try_new` (`weighted_vrf.rs:87-110` — `validators` + `weights`).
`weights` запись несёт (`dpos.rs:3576`), `tombstoned`/`activation_epoch` — не несёт и
не нужны. **Д-18 подтверждаю по коду.** Одно уточнение: у follower'а `WeightedVrf` вообще
не строится (движка нет), так что нога `weights` там избыточна, но безвредна.

**`FluentApp::randomness` снят — читателей не было [KNOWN].**
`git show HEAD:crates/dpos/consensus/src/application.rs | grep -n randomness` даёт ровно
пять мест: док (`:275-278`), поле (`:278`), `Clone` (`:356`), инициализация `:398`,
сеттер `:435-440`, плюс два вызова сеттера в тестах (`:1544`, `:1967`). Ни одного
чтения. Единственный продакшн-вызов сеттера — `HEAD:outer.rs:1150`, снят.
`git grep "with_randomness"` по дереву оставляет только `CertInlet::with_randomness`
(`cert_inlet.rs:537`) — другой тип. Удаление чистое.

### 5. Стенд: посылка `sum > 50` → «каждая пройденная эпоха прочитана ≥ 1 раз» [KNOWN]

Тест `a_committee_not_yet_committed_is_not_read_early` (`testbed/tests.rs:1838-1881`;
`HEAD:testbed/tests.rs:1838-1872`) держит три посылки. Две не тронуты:
`reads.unknown_state == 0` и `reads.uncommitted[&1] >= 1`. Третья заменена.

Что она теперь доказывает: для каждой эпохи `0..=epoch(max height)` счётчик
`reads.committed[&epoch] >= 1` — то есть журнал чтений покрывает каждую эпоху, через
которую прошёл прогон. Что перестала: любое утверждение о ВЕЛИЧИНЕ трафика чтений
(`> 50` в сумме).

Ответ: **(а) равносильная защита от вакуумности, с оговоркой.** Ровно одна из двух
сохранённых посылок вакуумна на пустом журнале — `unknown_state == 0`; вторая
(`uncommitted[&1] >= 1`) сама требует реального чтения. Старый порог `> 50` защищал
первую только суммой, которая могла набраться на одной эпохе; новая посылка требует
покрытия каждой эпохи и в этом смысле по ФОРМЕ строже. По ОБЪЁМУ она слабее на порядок
(минимум 3 чтения против 51), но объём тест и не утверждал — он его использовал как
прокси. Так что «(а) с элементом (в)»; чистого ослабления нет. Замечание к якорю:
`epoch_at_block(max_height, 0, EPOCH_LEN)` (`:1875`) хардкодит `activation = 0`, а модуль
стенда получает `DPOS_ACTIVATION_BLOCK` (`stand.rs:1889`); они совпадают
(`testbed/fakes.rs:53`: `DPOS_ACTIVATION_BLOCK = 0`), так что расхождения сегодня нет —
но связи между ними тоже нет (B1-12).

**Другие старые ассерты стенда — не менялись [KNOWN].**
`git diff HEAD -- crates/dpos/consensus/src/testbed/ | grep "^[-+].*assert"` даёт только
ханк этого теста: три удалённые строки `assert!(reads.committed.values().sum::<u64>() > 50, …)`
и добавленный цикл. В `stand.rs`/`fakes.rs` ни одной строки с `assert` не тронуто.

### 6. Граница [KNOWN]

- `beacon::CommitteeReads` — **ровно одна реализация**:
  `git grep "CommitteeReads for"` даёт единственную строку
  `committee/facade.rs:57: impl beacon::CommitteeReads for CommitteeReadsFacade`.
- `qual_read_at` **удалён полностью**: `git grep "qual_read_at"` по `crates/` даёт одно
  вхождение — в прозе дока трейта (`beacon/plane.rs:109`), объясняющей, почему метода
  больше нет. Из трейта (`HEAD:beacon/plane.rs:134`) и обеих проекций
  (`beacon/plane.rs:601`, `beacon/follower.rs:128` — обе на `read_at()`) ушёл.
- `pub` в `committee/`: **появился** `pub type GeometryRx`
  (`committee/mod.rs:310`), реэкспортирован (`lib.rs:83`). **Пропало**: ничего из
  `committee/`; у фасада ушёл метод `qual_read_at` (метод трейта, не собственный
  `pub`). Сигнатура `CommitteeStore::new` сменила третий параметр
  `Geometry` → `GeometryRx` (`committee/store.rs:108`). Побочно: тип `Geometry` после
  этого не используется НИГДЕ вне `committee/` (`git grep "Geometry\b"` по
  `crates/dpos/consensus/src` + `crates/node/src` + `bins` минус `committee/` — пусто),
  но остаётся в `pub use` (`lib.rs:82`) — B1-13.
- `ProviderExecutedChain::new` **удалён**, вызывающих ноль; оба перевода на
  `with_cursor` — `node/dpos.rs:2031` и `node/cert_follow/mod.rs:128`. Третьего
  вызывающего на HEAD не было (`git show HEAD:… | grep`), проверено.
- Побочно снят параметр `R` у `OuterBuilder`/`OuterEngine` и `slasher::{Config, Actor}`;
  удалён `pub type slasher::actor::LatestFinalizedHash` (`HEAD:slasher/actor.rs:79`).

### 7. Тесты: `#[test]` HEAD vs дерево по каждому из 19 файлов [KNOWN]

Счёт снят скриптом (`grep -c "^\s*#\[test\]\|^\s*#\[tokio::test"` на `git show HEAD:$f`
против дерева):

| файл | HEAD | дерево |
|---|---|---|
| `committee/tests.rs` | 19 | **20** |
| `node/dpos.rs` | 10 | **8** |
| `application.rs` | 31 | 31 |
| `beacon/follower.rs` | 7 | 7 |
| `beacon/plane.rs` | 1 | 1 |
| `committee/{mod,store,facade}.rs` | 0 | 0 |
| `dpos.rs` | 31 | 31 |
| `executor.rs` | 120 | 120 |
| `lib.rs` | 0 | 0 |
| `outer.rs` | 5 | 5 |
| `slasher/actor.rs` | 7 | 7 |
| `testbed/{fakes,stand}.rs` | 0 | 0 |
| `testbed/tests.rs` | 35 | 35 |
| `tests/slasher_integration.rs` | 13 | 13 |
| `node/cert_follow/mod.rs` | 0 | 0 |
| `node/ordering.rs` | 1 | 1 |

**Удалённые — два, оба в `node/dpos.rs`, оба оправданы (предмет исчез):**
`the_qual_cursor_refuses_the_window_where_the_committee_cursor_falls_back_to_genesis`
(`HEAD:node/dpos.rs:2502`) и `with_a_finalized_marker_the_two_legs_read_at_one_cursor`
(`HEAD:node/dpos.rs:2522`) — оба вызывали `committee_cursor`/`qual_cursor`
(`HEAD:node/dpos.rs:1090`, `:1107`), функций в дереве нет. Свойство, которое они
пиновали («qual-нога не читает у генезиса»), пинуется структурно и тестом
`an_epoch_below_its_commit_height_is_not_readable_without_a_single_read`
(`committee/tests.rs:332`): `commit_height(1) = commit_height(2) = 1`
(`committee/mod.rs:373-379`), так что при якоре 0 эпохи 1 и 2 недостижимы без EVM.
Дырка, которую эта замена НЕ закрывает: эпоха 0 при якоре 0 читается на генезис-хэше и
пишется write-once — на genesis-committed девнете это верный ответ, но тестом это не
пинуется [ГИПОТЕЗА].

**Новый — один:** `without_a_frozen_geometry_every_epoch_is_not_readable_without_a_single_read`
(`committee/tests.rs:276-331`). Утверждает: при `geometry = None` — `NotReadable{ready_at: 0}`
и `Calls::default()` (ни одного staticcall'а); `subscribe()` стартует с 0;
`anchor_advanced()` до заморозки не будит; после `tx.send_replace(Some(...))` ТОТ ЖЕ стор
отвечает и первый `anchor_advanced()` публикует `epoch(400)+2`. Красным быть не мог —
ветка новая, конструктор не принимал watch; журнал (§4) утверждает проверку мутацией с
verbatim падения. Я мутацию не воспроизводил (правки файлов запрещены) — принимаю как
РЕЛЕЙ, не как `[KNOWN]`.

**Изменённые — один тест** (стенд, §0.5) и один комментарий
(`tests/slasher_integration.rs:761-771` — переписан под R-027, ассерт
`!wait_for_sink_calls(&calls, 1)` не тронут, проверено по диффу).

### 8. Продакшн-путь: запрещённое [KNOWN]

`git diff HEAD -- crates/ | grep "^+" | grep -E "unwrap\(|expect\(|#\[allow|todo!|unimplemented!|dbg!|sleep|Duration::from|interval\("`
даёт ровно четыре строки, все в тестах:
`committee/tests.rs:305` (`has_changed().expect("sender alive")`),
`committee/tests.rs:310` (`.expect("readable once the geometry is frozen")`),
`testbed/tests.rs:1875-1876` (`.expect("heights")`, `.expect("non-zero epoch length")`).
Новых `#[allow]`, `todo!`, `dbg!`, таймеров и поллинга — ноль.
Наоборот, УДАЛЁН один таймер: `c.sleep(Duration::from_secs(60))` watchdog'а
(`HEAD:dpos.rs:2358`). Единственный новый сигнал — `(self.anchor_advanced)()`, событие,
а не тик.

### 9. Где журнал вводит в заблуждение [KNOWN]

Журнал (`.dpos-study/history/E4-1-B1.md`) точен почти везде; я перепроверил его
загружающие утверждения и нашёл три места, где он неточен, и одно, где он неполон.

1. **§5, verbatim фичевого clippy** (`E4-1-B1.md:329`) — команда без
   `-p fluentbase-node`, ворота требуют пары. Результат от этого не меняется
   (проверил), но verbatim не тот, что заявлен. См. §0.1.
2. **§0.5, вторая причина** (`E4-1-B1.md:97-107`) сформулирована как «громкий отказ
   старта не может идти через модуль, потому что `OuterBuilder::build` вызывается ПОСЛЕ
   этой проверки». Механически проверено и верно (`dpos.rs:1995` против `.build(…)` на
   `dpos.rs:2624`; `last_consensus_finalized_height` рождается в `MarshalActor::init`,
   `outer.rs:904`). Но формулировка «невозможно» скрывает, что `Arc<dyn Committee>` в
   этой точке УЖЕ в области видимости (деструктуризация `DposLayerConfig` —
   `dpos.rs:1558-1565`): невозможно не «дотянуться до модуля», а «получить от него `Ok`».
   Разница существенна для Б2: чинить надо порядок посева курсора, а не проводку.
3. **§8.2** («стенд теперь читает эпохи чуть раньше, чем читал») верно, но не говорит
   главного: продакшн-якорь сдвинулся ВВЕРХ ровно на те же K, так что стенд не
   «разошёлся с продакшном», а снова с ним сошёлся (`HEAD` стенда читал
   `max(tip−K, live)` — `HEAD:testbed/stand.rs:1678-1680` + `:1858-1866`; продакшн HEAD
   читал `max(fin, live)`, `fin = ordering_finalized − K`). Формулировка журнала читается
   как деградация вернности стенда, тогда как это восстановление.
4. **§7 неполон в одном пункте**: список «что Б2 обязан сделать» называет
   `cert_inlet.rs:213/:252` и `node/cert_inlet.rs::committee_source`, но НЕ называет
   follower-овский источник inlet'а `dpos.rs:3678-3730` — замыкание с собственным
   курсором `max(fin, live)` над `executed_state_hash` (`dpos.rs:3693-3730`), который
   §5.1 проекта перечисляет отдельным носителем («inlet-источник `:3809-3887`»).
   Сайт жив, второй курсор держит; в §1 журнала он не фигурирует (в 18 он не попал,
   потому что `epoch_committee_snapshot` там вызывает `RethCommitteeSource`, а не
   само замыкание) — B1-08.

Утверждений журнала, ОПРОВЕРГНУТЫХ деревом, я не нашёл. Проверил выборочно и подтвердил:
счёт «18 → 9» (`git grep "epoch_committee_snapshot(" -- crates/dpos/consensus/src
crates/node/src crates/dpos/staking-reader/src` минус `committee/`, минус `testbed/`,
минус внутренности `reader.rs` и тесты `epoch_transition.rs` даёт ровно девять:
`cert_inlet.rs:213`, `:252`; `dpos.rs:1995`, `:3169`; `epoch_transition.rs:522`, `:550`,
`:639`, `:723`; `node/dpos.rs:1625`); «одна реализация `CommitteeReads`»; Д-18;
«читателей `FluentApp::randomness` не было»; три вызова `anchor_advanced` после
`advance_finalized`.

### 10. Что из §5.1 оказалось неверным против кода [KNOWN]

Журнал называет два места. **Оба подтверждаю по коду**, третье добавляю.

1. **Д-15, ленивая геометрия — подтверждено.** §5.1 («геометрия — `frozen_geometry()`
   этого ET») предполагает, что геометрия известна там, где строится стор. Стор строится
   `node/dpos.rs:1439` — сразу за курсором (`:1432`) и на 130 строк раньше поллера,
   который единственный её замораживает (`:1563-1571`). Взять по значению можно было бы
   только отложив стор — то есть заведя второй курсор. `GeometryRx` — правильная правка.
2. **Невозможность громкого отказа старта через модуль до `OuterBuilder::build` —
   подтверждено с уточнением.** См. §0.9 п.2: препятствие — незасеянный курсор
   (`executor.rs:1109-1110` ← `outer.rs:904` ← `.build()` на `dpos.rs:2624` ПОСЛЕ
   `dpos.rs:1995`), а не отсутствие хэндла. Проверка через модуль в этой точке дала бы
   `OutOfWindow` (окно при якоре 0 — `[0, 2]`) и `bail!` на каждом старте выше эпохи 2.
3. **Третье, журналом не названное: §5.1 «Кто читает через `CommitteeReads`» обещает,
   что для follower'а «равенство держится» и «ничего не теряется», потому что его курсор
   `get_finalized_num_hash` state-gated.** По коду это неверно в одном измерении, которое
   таблица не рассматривает: `get_finalized_num_hash()` читается из reth-состояния,
   ПЕРЕЖИВШЕГО рестарт, а `FinalizedCursor` рождается нулём и засевается из marshal'а
   (`outer.rs:904`). На свежем datadir'е follower'а (marshal пуст, cold-start прыжок уже
   посадил EL на высоту N) якорь = 0 при `get_finalized_num_hash() = N`. Таблица §5.1
   сравнивает только ВЫСОТЫ курсоров в установившемся режиме и этого окна не покрывает.
   Это и есть B1-02.

### 11. Правка вне списка файлов (Д-16, `outer.rs`) [KNOWN]

Ханков шесть; пять — чистая проводка, один вводит новое поведение (и должен).

| # | место (дерево) | что | поведение? |
|---|---|---|---|
| 1 | `:521`, `:702`, `:711`, `:793`, `:801`, `:1303`, `:1312` | снят параметр `R: slasher::StakingStateRead` с `OuterBuilder`/`OuterEngine`/обоих `impl`; `slasher: slasher::Actor<E>` вместо `Actor<E, R>` | нет — типовая |
| 2 | `:665-670` | поля `slasher_reader` + `slasher_latest_finalized_hash` → одно `committee: Arc<dyn Committee>` | нет — источник тот же контракт, курсор другой (см. B1-01) |
| 3 | `:1113-1119` | `executor::Config.anchor_advanced` = `Arc::new(move \|\| committee.anchor_advanced())` | **ДА** — executor начал звать модуль в трёх точках. Это и есть обязательная работа Б1.1 |
| 4 | `:1157` (снято `HEAD:outer.rs:1150`) | удалена строка `let app = app.with_randomness(randomness.clone());` | нет — поле без читателей (проверено, §0.4) |
| 5 | `:1224-1227` | `slasher::Config { committee }` вместо `{ reader, latest_finalized_hash }` | нет напрямую; косвенно — маппинг ошибок сменился внутри слэшера (B1-01) |
| 6 | — | локальная `randomness` осталась живой (уходит в `executor::Config.randomness`, `:1112`) | нет |

Вывод: правка минимальна и неизбежна — `slasher::Config` строится только здесь
(`git grep "slasher::Config"` → `outer.rs:1223` и `tests/slasher_integration.rs:431`),
`FluentApp::with_randomness` звался только здесь. Ни один элемент списка Б2
(`EpochSchemeProvider` как вид, продюсеры, `CertInlet.schemes`, `latest_live`) не тронут
— проверил: `EpochSchemeProvider` в `outer.rs` фигурирует так же, как на HEAD
(`:1289` и `:733`), `cold_start_register` жив (`dpos.rs:2638`).

---

## §1. Находки

| id | серьёзность | file:lines (дерево) | HEAD-якорь | что не так | чем пытался опровергнуть и почему не вышло | уверенность |
|---|---|---|---|---|---|---|
| **B1-01** | **SERIOUS** | `slasher/actor.rs:712-724`; `committee/store.rs:411-421` (`REASON_ANCHOR_FAULT`), `:168-183`; `executed.rs:45-62`; `committee/mod.rs:217` | `HEAD:slasher/actor.rs:730-742` — `Err(e) => HandleError::transient("committee read failed: {e:?}")` | Класс ПОСТОЯННЫХ отказов слэшера расширен молча. HEAD считал ЛЮБУЮ ошибку reader'а транзиентной и повторял 30 × 2 с. Дерево маршрутизирует по `ReadError::is_transient()`, где транзиентны ровно три варианта (`staking-reader/src/error.rs:158-166`), а `Backend` и `CallReverted` — постоянны. Зонд якоря `Anchor::executed_hash` → `executed_state_hash` заворачивает ЛЮБУЮ ошибку провайдера (`best_block_number`, `block_hash`) в `ReadError::Backend(e.to_string())` — включая рваное чтение static-file, которое сам крейт в другом месте классифицирует как `TransientStorage` (`error.rs:60-72`). Итог: один сбой стораджа на промахе кэша ⇒ `Read(Backend)` ⇒ `Permanent` ⇒ `warn!` + **улика равновесия выброшена навсегда** (simplex сообщает о конфликте ровно один раз, реплея нет — `slasher/actor.rs:631-634`). Плюс спурьозный `error!` «контракт ответил невозможное» на дисковую икоту (`committee/store.rs:173-181`) | (а) Может, `Permanent` там недостижим? Нет: `resolve_committee` зовётся на каждом charge (`:785`, `:812`), шаг 4 выполняется при промахе карты (`store.rs:404-411` — кэш проверяется раньше, но при первом обращении к эпохе промах гарантирован). (б) Может, §5.4 это и предписывает? Нет: §5.4 перечисляет `CallReverted` (эта половина СПРОЕКТИРОВАНА), но `Backend`/рваное чтение в таблице отказов отсутствуют, а `Anchor::executed_hash` в доке обещает `Err` только для «real header-index fault» (`committee/mod.rs:295-297`) — `executed_state_hash` шире этого обещания. (в) Может, `best_block_number()` не падает? `block_hash()` падает: reth читает static-files, и весь `is_transient_torn_static_file_read` существует ровно для этого класса | высокая (по коду; вживую не воспроизводил) |
| **B1-02** | **MODERATE** | `node/dpos.rs:1432` + `committee/store.rs:518-534`; `executor.rs:1109-1115`; `outer.rs:904`; `dpos.rs:3123-3135` | `HEAD:node/dpos.rs:1422-1435` (`finalized_block_number()`), `HEAD:dpos.rs:3431-3442` (`get_finalized_num_hash()`) | Обе плоскости лишились ПЕРСИСТЕНТНОГО источника высоты. HEAD брал высоту из reth-тега/канонического состояния, которые `BlockchainProvider::with_latest` поднимает с диска до старта консенсуса. Дерево берёт `FinalizedCursor`, который рождается нулём и засевается `executor::Actor::init` из marshal'ова acked-курсора внутри `OuterBuilder::build`. Окно слепоты: от построения плоскости до `build` (валидатор) и до первого финализированного derive при пустом marshal'е (свежий datadir, follower после cold-start-прыжка, восстановление узла с целым reth и стёртым консенсус-datadir'ом). В окне `committee(E)` для середины цепи ⇒ `OutOfWindow{lo:0,hi:2}`; beacon не видит роспись, `evidence_committee_for` отдаёт `None`, follower не проверяет артефакты | (а) Дедлок? Нет: регистрация схемы стартовой эпохи идёт мимо модуля (`dpos.rs:1995` + `cold_start_register` `:2638`), выборщик — из ET'шного снимка, так что консенсус доезжает до первого derive сам и окно закрывается. (б) Окно нулевое? Нет: у валидатора оно ≥ времени `OuterBuilder::build` (инициализация marshal'а + сторадж), у follower'а со свежим marshal'ом — до первого derive. (в) §5.1 это покрывает? Нет — таблица «Кто читает через `CommitteeReads`» сравнивает только высоты установившихся курсоров, случай «курсор ещё не засеян, а тег уже есть» в ней отсутствует. (г) Журнал это называет? §8.4 называет только окно ГЕОМЕТРИИ | высокая (по коду) |
| **B1-03** | **MODERATE** | `slasher/actor.rs:640-668` (цикл повторов) + `:712-724`; `tests/slasher_integration.rs:761-775` | `HEAD:slasher/actor.rs:733-737` — пустой комитет ⇒ `HandleError::permanent(… "unrecoverable")` | Пустой комитет стал ТРАНЗИЕНТНЫМ (это R-027 и верно), но цикл повторов продюсера блокирует голову очереди: ветка `if let Some((entry, attempts)) = retry.take()` спит `SLASHER_RETRY_BACKOFF` и делает `continue`, НЕ трогая `mailbox_rx.recv()`. Значит одна улика о нечитаемой эпохе теперь держит весь актор до 30 × 2 с = 60 с, где раньше отбрасывалась на первом ответе. На догоне (много `NotReadable` подряд) задержки складываются | (а) Теряются ли улики? Нет — мейлбокс `unbounded_channel` (`:592`), только задержка. (б) Задержка важна? Слэшинг не латентно-критичен; но 60 с на элемент × N на догоне — режим, которого раньше не было, и он новый именно из-за Б1. (в) Может, §5.4 это предписывает? §5.4 предписывает транзиентность пустого комитета, но ничего не говорит о голове очереди — это следствие, которое ни журнал, ни проект не называют | средняя |
| **B1-04** | **MODERATE** | `committee/store.rs:466-471` (`anchor_advanced` ранний выход) + `node/dpos.rs:1569-1571` (`geometry_tx.send_replace`) | новое поведение (на HEAD модуля в проводке не было) | Заморозка геометрии — переход из «ничего не читается» в «читается всё окно» — НЕ публикует пробуждение. `anchor_advanced()` при `geometry == None` выходит до `readable.send_if_modified`, а в точке `send_replace(Some(frozen))` модуль не дёргается вовсе. Контракт `Committee::subscribe` («A consumer parked on `NotReadable` wakes here … instead of polling on a timer», `committee/mod.rs:247-253`) в этот момент не выполняется: `readable` остаётся 0 до следующего `advance_finalized` | (а) Есть ли сегодня жертва? Нет: `git grep` по `subscribe()` даёт только `store.rs:463` и два теста — продакшн-подписчиков ещё нет (они приходят с Б2). (б) Значит, безвредно? Сегодня да; но дырка в СОБСТВЕННОМ контракте модуля, и Б2 подписчика заведёт. (в) Закроется само? Да, первым же финализированным derive (1 блок/с) — то есть на живой сети окно доли секунды; но это свойство темпа, не кода | высокая (по коду), низкая по влиянию сегодня |
| **B1-05** | **MODERATE** | `.claude/dpos_architecture/06_staking_layer.md:280`; `09_followers.md:242-246`; `00_preamble.md:34-38` | символы `qual_read_at` (`HEAD:beacon/plane.rs:134`), `FluentApp::with_randomness` (`HEAD:outer.rs:1150`), `HEAD:node/dpos.rs:1082-1110` | Доковый дрейф в `.claude/dpos_architecture/`, который проектный CLAUDE.md называет блокером ревью, а не follow-up'ом. `06_staking_layer.md:280` описывает фасад как «`read_at` = the anchor hash, `qual_read_at` = `read_at`» — метода нет. `09_followers.md:243-246` перечисляет `FluentApp::new` среди трёх вызывающих `absent_unregistered` и утверждает, что «`with_randomness` at `outer.rs:1150` runs before the first `app.clone()`» — и поля, и сеттера, и строки нет. `00_preamble.md:34-38` держит якоря на удалённый `qual_read_at` и на `node/dpos.rs:1082-1110` (там теперь `record_route_miss`). Блок `verified-against` в `00_preamble.md` не обновлён | (а) Может, это исторический раздел? `00_preamble.md` — да, датированная запись прохода В; но `06_staking_layer.md:280` и `09_followers.md:246` — описание ТЕКУЩЕГО кода. (б) Может, доки обновляются в конце строки 4.1? Правило CLAUDE.md говорит «IN THE SAME change» и «grep the doc for the old symbol» — грепа явно не было. (в) Может, дифф их трогает? `git status --short` не содержит ни одного файла под `.claude/` | высокая |
| **B1-06** | **MODERATE** | `dpos.rs:3145-3176` (`soft_enter_committees`), `dpos.rs:3678-3730` (`inlet_committees` + замыкание курсора), `dpos.rs:1995`, `cert_inlet.rs:213`, `:252` | `HEAD:dpos.rs:3225-3256`, `HEAD:dpos.rs:3809-3887`, `HEAD:dpos.rs:1999` | 4.1 стоит ровно в состоянии, о котором §7 проекта пишет «наполовину, хуже нынешнего, потому что расхождение станет невидимым за общим типом»: рядом с модулем живут ЧЕТЫРЕ независимых комитетных курсора (span-читатель follower'а на `get_finalized_num_hash`, inlet-источник follower'а на `max(fin, live)`, `initial_snapshot` на `latest_finalized_hash`, `RethCommitteeSource` валидатора). Это сознательный раздел Б1/Б2 по постановке, а не ошибка исполнителя — но пока Б2 не закрыт, «две авторитетные карты» существуют, и §7 их прямо запрещает оставлять | (а) Постановка Б1 это разрешает? Да — `4.1-B1-impl-1.md:19` явно выносит `cert_inlet.rs` и `epoch_manager.rs` в Б2, а Б1.3 не называет span-читатель. (б) Значит не находка? Находка о СОСТОЯНИИ строки, а не о заходе: коммитить Б1 отдельно = зафиксировать в истории состояние, которое §7 называет хуже исходного. Решение — оркестратора. (в) Журнал это признаёт? Да, §7, но неполно (не назван `dpos.rs:3678-3730`) — B1-08 | высокая |
| **B1-07** | **MINOR** | `beacon/plane.rs:586-596`, `beacon/follower.rs:120-122` | `HEAD:beacon/plane.rs:589-607` | Проекции устроены как `reads.committee(epoch, reads.read_at()?)` — `read_at()` остался ГЕЙТОМ. `CommitteeReadsFacade::read_at` = `anchor_hash()` = `executed_state_hash(anchor.height())` (`facade.rs:62-64`), и при `Ok(None)` (высота выше материализованной головы) вся проекция даёт `None` — **даже если запись уже лежит в карте модуля**. Модуль сам ответил бы из кэша (`store.rs:404-406` стоит ДО зонда якоря). То есть фасад слабее, чем модуль, ровно на этот случай | (а) Достижимо? Окно узкое: якорь — ordering-финализированная высота, она ≤ best почти всегда; достижимо сразу после посадки прыжка и при откате материализации. (б) Вредно? Ещё один цикл ожидания, не потеря. (в) Уходит само? Да, с формой трейта в Э5 5.1 — но пока трейт жив, это регресс относительно контракта модуля | средняя |
| **B1-08** | **MINOR** | `.dpos-study/history/E4-1-B1.md` §7 (список «что Б2 обязан сделать») | `dpos.rs:3678-3730` (дерево) | Журнал не называет follower-овский inlet-источник как оставшийся носитель. §5.1 проекта перечисляет его отдельной строкой («inlet-источник `:3809-3887`, `executed_state_hash(max(fin, live))` с откатом на `fin` — единственное state-gated чтение»). В §1 журнала он тоже не фигурирует, потому что `epoch_committee_snapshot` там зовёт `RethCommitteeSource`, а не само замыкание — так что счёт «18 → 9» его не видит вовсе | (а) Может, он ушёл? Нет: `dpos.rs:3693-3730` в дереве, курсор `p.finalized_block_number().max(live_frontier)` на месте. (б) Может, пункт 1 §7 его покрывает? Он называет `cert_inlet.rs:213/:252` и `node/cert_inlet.rs::committee_source` — валидаторскую половину; follower'ская строится в `dpos.rs`. Для Б2 это разные сайты | высокая |
| **B1-09** | **MINOR** | удалено: `HEAD:dpos.rs:2325-2408` | `HEAD:dpos.rs:2385-2400` (`warn!` «no finalized progress and this key is NOT in the current committee») | Вместе с watchdog'ом ушла единственная операторская диагностика «финализация стоит, и этот ключ не в комитете» — та самая silent-verifier ловушка, ради которой он и писался. Проект §5.5 удаление предписывает, замены не называет; в дереве замены нет | (а) Может, что-то ещё её даёт? `git grep "NOT in the current committee"` — ноль. Метрики `dpos_committee_*` (`store.rs:24-32`) считают отказы ЧТЕНИЯ, а не «я не член»; `is_member` через модуль никто не зовёт (проверил `git grep "is_member"`). (б) Может, кто-то ждал watchdog? Нет — `drop(ctx…spawn(…))`, хэндл выброшен на HEAD | высокая |
| **B1-10** | **MINOR** | `testbed/stand.rs:1670-1685` | — | Комментарии стенда протухли на той же правке. `:1670-1677` объясняет, почему якорь НЕ `chain.tip()` («Reading at the ordering cursor would sit K blocks HIGHER than production») — а модуль стенда (`:1861-1863`) теперь читает ровно `chain.tip()`, потому что продакшн туда и переехал. `:1682-1685` утверждает «the `dkgQual` reads take THIS one (`node/src/dpos.rs:1519-1524` — `finalized_block_number` only, no live cursor), while the committee reads take the teed cursor below» — ни отдельного qual-курсора, ни teed committee-курсора больше нет, а `node/dpos.rs:1519-1524` теперь тело поллера | (а) Может, они всё ещё про `el_finalized`? `:1670-1677` — да, для ET; но фраза про «ordering cursor … K blocks HIGHER than production» прямо ложна. `:1682-1685` ложна целиком. (б) Комментарии не свидетельство (правило 4) — верно, и именно поэтому ЛОЖНЫЙ комментарий рядом с правильным кодом это находка: следующий читатель поверит | высокая |
| **B1-11** | **MINOR** | `cert_inlet.rs:322-333`; `node/dpos.rs:806-813`; `beacon/surface.rs:630-634` | `HEAD` те же места | Три протухших дока в продакшн-коде. `cert_inlet.rs:322-323`: «`live_height` is the `beacon::actor::CommitteeFor` read cursor (`committee_for` reads `committee[E]` at `max(EL-finalized, live_height)`)» — для валидатора уже нет (для follower'а ещё да, `dpos.rs:3693`), так что док стал ПОЛУверным без указания, какая половина. `node/dpos.rs:806-813`: «the inlet also tees the LIVE upstream cert frontier into the beacon plane's `committee_for` read cursor» — ложно. `beacon/surface.rs:632`: «`FluentApp::new` and `CertInlet::new` both have `with_randomness` called on them before first use» — у `FluentApp` ни поля, ни сеттера | грепом проверил каждое утверждение против дерева; ни одно не воспроизводится | высокая |
| **B1-12** | **NIT** | `testbed/tests.rs:1875` | — | Новая посылка считает `epoch_at_block(max_height, 0, EPOCH_LEN)` — хардкод `activation = 0`, тогда как модуль стенда получает `(DPOS_ACTIVATION_BLOCK, cfg.epoch_len)` (`stand.rs:1889`). Сегодня совпадает (`fakes.rs:53`: `DPOS_ACTIVATION_BLOCK = 0`), но связи нет: сдвиг активации в фейке сделает границу цикла неверной молча | (а) Может, `EPOCH_LEN` тоже разойдётся с `cfg.epoch_len`? `tests.rs:1021` = 32, `StandConfig::live(4, 1)` — не проверял её epoch_len, так что риск двойной. (б) Тест зелёный — да, поэтому NIT | средняя |
| **B1-13** | **NIT** | `lib.rs:82`; `committee/mod.rs:314-380` | `HEAD:lib.rs:82` | После Д-15 тип `Geometry` не используется нигде вне `committee/` (`git grep "Geometry\b"` по `crates/dpos/consensus/src` + `crates/node/src` + `bins` минус `committee/` и минус `GeometryRx`/`GeometryUnfrozen` — пусто), но остаётся в `pub use committee::{… Geometry …}`. Мёртвая публичная поверхность крейта | (а) Может, его используют тесты вне модуля? `committee/tests.rs` — внутри модуля. (б) Может, `GeometryRx` его требует? Нет, `GeometryRx` = `watch::Receiver<Option<(u64,u64)>>`, сырая пара | высокая |
| **B1-14** | **NIT** | `testbed/stand.rs:1862-1872` (`StandAnchor::executed_hash`) | — | Стендовый якорь отдаёт `Ok(self.chain.hash_at(height))` — ветка `Err` (header-index fault) не моделируется, хотя ВЕРНЫЙ двойник уже существует: `FakeChain::executed_state_hash` (`testbed/fakes.rs:609-619`) воспроизводит все три исхода продакшн-`executed_state_hash`. Взять его было бы бесплатно; комментарий (`stand.rs:1852-1858`) осознанно отказывается, но ссылается на «нет header/state split», хотя фейк этот split как раз и эмулирует через `executed_tip()` | (а) Меняет ли это ответы сегодня? Нет: якорь читается на `chain.tip()` (tier-F tip), там `hash_at` и `executed_state_hash` совпадают. (б) Значит эквивалентно? По значению да, по ПОКРЫТИЮ нет — и именно ветка `Err` — вход находки B1-01 | средняя |
| **B1-15** | **NIT** | `tests/slasher_integration.rs:203-211` (`StubAnchor`) | — | `StubAnchor::height()` — константа в середине `EPOCH`, `executed_hash` — всегда `Ok(Some(ZERO))`. Значит ни `OutOfWindow` (ни вверх, ни вниз), ни `NotReadable` по незасеянному якорю, ни `REASON_ANCHOR_FAULT` в интеграционных тестах слэшера не проходят ни разу — а новый маппинг исходов (§0.4) держится ровно на них. Журнал признаёт это в §8.5 | (а) Может, юниты модуля покрывают? `committee/tests.rs:896` (`an_epoch_above_the_window_is_worth_a_retry_and_one_below_never_is`) покрывает ПРЕДИКАТ, но не то, что слэшер на него правильно маршрутизирует. (б) Значит нет теста на «кто как отреагирует» — верно, и это ровно тот стык, где B1-01 живёт | высокая |

---

## §2. Поведение по ханкам против HEAD

**`committee/mod.rs`** — добавлен `pub type GeometryRx` (`:300-310`) с доком; из дока
`Geometry` убран абзац «A value type rather than a live read». Поведения нет.

**`committee/store.rs`** — `new` берёт `GeometryRx` (`:108`), seed `readable` = 0 при
`None` (`:109-112`); `geometry_of`/`geometry()` (`:122-130`); `window`/`highest_readable`
берут геометрию параметром; `install` берёт `lo` параметром (Д-20 — пол ТОГО чтения, а
не более свежий: направление безопасное, лишний хвост карты чистит следующий
`anchor_advanced`); шаг 0 в `committee()` (`:369-376`) и ранний выход в
`anchor_advanced` (`:466-471`). Единственное новое поведение — «нет геометрии ⇒
`NotReadable{ready_at:0}` без EVM» и непубликация пробуждения на заморозке (B1-04).

**`committee/facade.rs`** — снят `qual_read_at` и абзац о нём. Остальные четыре метода
байт-в-байт.

**`beacon/plane.rs`** — `qual_read_at` из трейта удалён (`HEAD:134`), док трейта
переписан; проекция `dkg_qual_for` резолвит `read_at()` (`:601`). Поведение: qual-нога
теперь читает на том же курсоре, что и комитет — что у фасада тождество, потому что
`at` игнорируется; защита от «замёрзшего `false`» перенесена в структуру модуля
(`store.rs:390-396`).

**`beacon/follower.rs`** — то же (`:128`).

**`node/ordering.rs`** — `new` → `with_cursor`. Поведения нет, кроме невозможности
завести второй курсор.

**`node/dpos.rs`** — удалены `PlaneCommitteeReads` (~110 строк), `committee_cursor`,
`qual_cursor`, два их теста; добавлены `finalized_cursor` (`:1432`), `CommitteeStore`
(`:1439-1451`), фасад (`:1454-1456`), два поля `BeaconPlane` (`:1039`, `:1044`), два
параметра `launch_dpos_layer` (`:1936-1937`), `with_cursor` (`:2031`),
`DposLayerConfig.committee` (`:2117`). `evidence_committee_for` (`:1718-1725`) — одна
строка, memo снят. Tombstone-поллер (`:1621-1623`) — state-gated курсор. Поведение:
всё чтение комитета переехало на якорь модуля (B1-02); поллер перестал делать заведомо
провальные EVM-вызовы на бэкфилле.

**`node/cert_follow/mod.rs`** — курсор создаётся здесь (`:127`), уезжает и в `executed`,
и в `FollowerLayerConfig` (`:235`). Поведения нет.

**`dpos.rs` (consensus)** — `DposLayerConfig.committee` (`:923-928`);
удалён `reader_for_slasher` (`HEAD:1626-1634`); удалён watchdog целиком
(`HEAD:2325-2408`) вместе с `slasher_latest_finalized_hash` (`HEAD:2413-2421`);
`OuterBuilder` получает `committee` вместо двух полей (`:2610`, `:3483`);
`FollowerLayerConfig.finalized_cursor` (`:2756`); follower строит `CommitteeStore`
(`:3123-3135`) с уже замёрзшим watch (`tokio::sync::watch::Sender::new(Some(...)).subscribe()`
— Sender роняется сразу, но `borrow()` читает последнее значение, а стор только
`borrow()` и делает, `store.rs:123`); `FollowerCommitteeReads` заменён фасадом (`:3337`);
`committee_at` проецирует запись в `ValidatorSetSnapshot` (`:3558-3580`).
Поведение: (а) слэшер на обоих путях сменил источник (B1-01); (б) follower читает
комитет на своём ordering-финализированном курсоре (B1-02); (в) ушла операторская
диагностика (B1-09); (г) `committee_at` теперь отдаёт `tombstoned: false` /
`activation_epoch: 0` — читателей нет (§0.4).

**`slasher/actor.rs`** — `Config<R,E>` → `Config<E>`, `Actor<E,R>` → `Actor<E>`, поля
`reader`+`latest_finalized_hash` → `committee`; `LatestFinalizedHash` удалён;
`resolve_committee` возвращает `Arc<CommitteeRecord>`; `enqueue_fallback` берёт жертву из
`record.members` (поиск по ключу, не по индексу — семантика та же, что у
`snap.validators.iter().find` на HEAD). Поведение: маппинг исходов (§0.4) — одна
улучшенная ветка (пусто ⇒ Transient, R-027) и одна расширенная (B1-01), плюс
head-of-line (B1-03).

**`outer.rs`** — §0.11.

**`application.rs`** — снято поле `randomness`, сеттер, ветка `Clone`, инициализация;
снят хелпер `test_randomness`/`test_group_keys`, параметры `witness_app`/`propose_app`.
Ни один ассерт не тронут (проверил по диффу: изменены только сигнатуры вызовов).
Поведения нет.

**`executor.rs`** — `AnchorAdvancedFn` (`:529-544`), поле `Config` (`:774-777`), поле
актора (`:1037-1039`), три вызова. Две тестовые конструкции `Config` получают
`Arc::new(|| {})`. Логика не тронута.

**`testbed/fakes.rs`** — `impl committee::EpochReads for FakeStaking` (`:1012-1031`).
Только добавление.

**`testbed/stand.rs`** — `StandAnchor` (`:1856-1872`), `CommitteeStore` (`:1878-1890`),
roster-closure через запись (`:1903-1907`), фасад (`:1947-1949`), `OuterBuilder.committee`
(`:2052`); удалены `committee_read_hash` и `StandCommitteeReads`. Поведение стенда:
комитет читается на K блоков выше, чем читался, — ровно как переехал продакшн (§0.9 п.3).

**`testbed/tests.rs`** — одна посылка (§0.5).

**`tests/slasher_integration.rs`** — `EpochReads` для `StubReader`, `StubAnchor`,
`stub_committee`, `EPOCH_INTERVAL`; `Config` заполняется модулем; один комментарий
переписан. Ассерты не тронуты.

---

## §3. Граница — таблица имён `pub`

| имя | HEAD | дерево | примечание |
|---|---|---|---|
| `committee::GeometryRx` | — | **`pub type`** (`committee/mod.rs:310`), реэкспорт `lib.rs:83` | новое |
| `committee::Geometry` | `pub struct` | `pub struct` | больше не используется вне `committee/` (B1-13) |
| `CommitteeStore::new` | `(R, Arc<dyn Anchor>, Geometry)` | `(R, Arc<dyn Anchor>, GeometryRx)` | сигнатура |
| `committee::{Anchor, Committee, CommitteeError, CommitteeReadsFacade, CommitteeRecord, CommitteeStore, EpochReads, Member, RethAnchor}` | `pub` | `pub` | без изменений |
| `WINDOW_FITS_THE_WEIGHT_RING` | `pub const` | `pub const` | без изменений |
| `beacon::CommitteeReads::qual_read_at` | метод трейта | **удалён** | одна реализация осталась — фасад |
| `slasher::actor::LatestFinalizedHash` | `pub type` | **удалён** | |
| `slasher::actor::Config<R, E>` | | `Config<E>` | поля `reader`+`latest_finalized_hash` → `committee` |
| `slasher::Actor<E, R>` | | `Actor<E>` | |
| `OuterBuilder<B,P,BE,D,XC,A,R>` / `OuterEngine<…,R>` | | без `R` | поля `slasher_reader`+`slasher_latest_finalized_hash` → `committee` |
| `DposLayerConfig.committee` | — | **новое поле** (`dpos.rs:928`) | |
| `FollowerLayerConfig.finalized_cursor` | — | **новое поле** (`dpos.rs:2756`) | |
| `BeaconPlane.finalized_cursor`, `.committee` | — | **новые поля** (`node/dpos.rs:1039`, `:1044`) | `pub(crate)` |
| `executor::AnchorAdvancedFn`, `Config.anchor_advanced` | — | **новые** (`executor.rs:544`, `:777`) | |
| `ProviderExecutedChain::new` | `pub fn` | **удалён** → `with_cursor` (`node/ordering.rs:39`) | все вызывающие (2) переведены |
| `FluentApp::with_randomness` | `pub fn` | **удалён** | поле `randomness` тоже |
| `node/dpos.rs::committee_cursor`, `::qual_cursor` | приватные | **удалены** | вместе с двумя тестами |

---

## §4. Ворота (verbatim)

Все команды прогнаны мной в этой сессии из `/home/djadjka/Work/fluentbase` на
незакоммиченном дереве. [KNOWN]

~~~
$ cargo test -p fluentbase-consensus --lib
test result: ok. 665 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 28.94s

$ cargo test -p fluentbase-consensus --lib --features dpos-devnet-byzantine testbed::
test result: ok. 40 passed; 0 failed; 0 ignored; 0 measured; 633 filtered out; finished in 34.60s

$ cargo test -p fluentbase-node --lib
test result: ok. 57 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 57.66s

$ cargo test -p fluentbase-staking-reader
test result: ok. 60 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
test result: ok. 0 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s

$ cargo test -p fluentbase-consensus --test slasher_integration
test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s

$ cargo clippy -p fluentbase-consensus -p fluentbase-node --all-targets
warning: large size difference between variants
    --> crates/node/src/dpos.rs:1873:1
     |
1873 | / pub(crate) enum ValidatorUpstream {
1874 | |     Ws(crate::cert_follow::upstream::UpstreamHandle),
     | |     ------------------------------------------------ the second-largest variant contains at least 8 bytes
1875 | |     Plane(fluentbase_consensus::PlaneUpstreamHandle<Context>),
     | |     --------------------------------------------------------- the largest variant contains at least 232 bytes
1876 | | }
     | |_^ the entire enum is at least 232 bytes
warning: `fluentbase-node` (lib) generated 1 warning
warning: `fluentbase-node` (lib test) generated 1 warning (1 duplicate)
### EXIT=0

$ cargo clippy -p fluentbase-consensus -p fluentbase-node --all-targets --features dpos-devnet-byzantine
warning: large size difference between variants
    --> crates/node/src/dpos.rs:1873:1
warning: `fluentbase-node` (lib) generated 1 warning
warning: `fluentbase-node` (lib test) generated 1 warning (1 duplicate)
### EXIT=0

$ cargo fmt --check
### EXIT=0
$ cargo fmt --check 2>&1 | grep -c "Diff in"
0

$ cargo doc -p fluentbase-consensus --no-deps 2>&1 | grep -c "unresolved link"
9
~~~

Расхождения с журналом — §0.1 (две косметические позиции: пакет `fluentbase-node`
выпал из фичевого clippy журнала; всё остальное совпадает цифра в цифру).

---

## §5. Оставить как есть

- **Д-17 (`AnchorAdvancedFn` вместо `Arc<dyn Committee>` в executor'е).** Постановка
  Б1.1 оставляла выбор; замыкание даёт executor'у ровно один глагол и не открывает ему
  `committee()`/`changed()`/`is_member()`. Это строго лучше буквы §5.1 («executor держит
  `Arc<dyn Committee>`») — не трогать.
- **Д-20 (`install` берёт `lo` параметром).** Направление безопасное: устаревший (более
  низкий) пол оставляет в карте лишнее, которое подметёт следующий `anchor_advanced`;
  перечитанный (более свежий) пол мог бы выкинуть только что прочитанную запись.
  Не трогать.
- **Фасад игнорирует `at`.** Правильно по write-once; уходит вместе с формой трейта в
  Э5 5.1. Не чинить здесь.
- **`dkg_qual` фасада возвращает `(changed, true)`.** Второй страж
  `!(bit || committed) ⇒ None` (`beacon/carry.rs:238-240`) становится мёртвой ветвью, но
  снимать его до 5.1 не нужно: он ничего не стоит и держит контракт трейта на случай
  второй реализации.
- **Follower'ский watch, созданный из немедленно роняемого `Sender`**
  (`dpos.rs:3134`, `committee/tests.rs:266`, `stand.rs:1888`). Выглядит подозрительно,
  но корректно: `watch::Receiver::borrow()` читает последнее значение и после смерти
  отправителя, а стор ничего, кроме `borrow()`, не делает. Переписывать на живой
  `Sender` — лишнее поле ради вида.
- **Удаление `FluentApp::randomness`.** Проверено независимо от журнала (§0.4):
  читателей на HEAD не было. Правка чистая.
- **`live_height` оставлен живым write-only атомиком** (`node/dpos.rs:1360`, `:1860`).
  Постановка выносит его снятие в 4.2 вместе с `upstream_frontier`; удалять сейчас —
  лезть в `cert_inlet.rs`.
- **Две тестовые конструкции `executor::Config` с `Arc::new(|| {})`.** Считать вызовы —
  не свойство `executor.rs`; порядок относительно `advance_finalized` читается в
  исходнике и прочитан (§0.2).

---

## §6. Замечено вне рамок Б1

**Для Б2 (`EpochSchemeProvider` как вид, `CertInlet.schemes`, один ET, `latest_live`):**

1. **Порядок посева курсора — корневая причина двух «невозможностей» Б1.** И громкий
   отказ старта (`dpos.rs:1995`), и слепое стартовое окно (B1-02) упираются в одно:
   `FinalizedCursor` засевается внутри `OuterBuilder::build` (`outer.rs:904` →
   `executor.rs:1109`), тогда как высота УЖЕ известна из reth до всякого консенсуса
   (`dpos.rs:2010` — `latest_finalized`/`latest_finalized_hash` читаются на `:1990`).
   Б2, снимая `cold_start_register` как продюсера, должен решить именно это, а не
   переписывать проверку: засеять курсор из `latest_finalized` до построения плоскости
   (или дать `RethAnchor` fallback на reth-тег, пока курсор нулевой) закрывает обе
   позиции сразу. Без этого Б2 упрётся в ту же стену.
2. **`epoch_manager.rs::latest_live`** (`:511`, пишется `:752`) — кэш
   `(Epoch, ValidatorSetSnapshot)`. Это и есть «вторая авторитетная карта», которую §7
   4.1 запрещает оставлять рядом с модулем. Замена по §5.1 — `committee(live_epoch)`.
3. **Канал `boundary_rx` несёт `ValidatorSetSnapshot`** (`outer.rs:735`,
   `epoch_manager.rs:463`). Follower уже проецирует его из записи (Д-18), валидатор шлёт
   ET'шный снимок. Пока канал не станет `epoch`-only, у одной границы два формата с
   разной полнотой (`tombstoned` живой у одного, `false` у другого) — сейчас безвредно
   (читателей нет), но это ровно та невидимость за общим типом, о которой §7.
4. **Четыре оставшихся комитетных курсора** — B1-06: `dpos.rs:3145-3176` (span),
   `dpos.rs:3678-3730` (inlet follower'а), `dpos.rs:1995` (`initial_snapshot`),
   `cert_inlet.rs:213`/`:252` + `node/cert_inlet.rs::committee_source`
   (`node/dpos.rs:751-768`).
5. **`EpochSchemeProvider` как вид:** `EpochEntry` уже структура с одним полем
   (`committee/store.rs:55-57`) с доком «adding a `scheme` field here» — заготовка на
   месте, менять модуль под это не придётся.
6. **Метрика без потребителя:** `Committee::subscribe` сегодня не вызывает никто
   (`git grep ".subscribe()"` по `committee/` — только `store.rs:463` и два теста).
   Б2 — первый подписчик; до него не забыть B1-04 (заморозка геометрии не будит).

**Для Б3 (стенд `FakeStaking` по ветке хэша):**

7. `FakeStaking::dkg_qual` внутри себя зовёт `epoch_committee_snapshot` ещё один-два
   раза (`testbed/fakes.rs:992`, `:1001`), так что `StakingReads` растёт быстрее, чем
   «два staticcall'а на эпоху» модуля — на счётчиках теста §0.5 это 4–5 чтений на эпоху
   вместо 2. Тест «ровно два чтения на эпоху» над этим фейком будет ложным, пока
   `dkg_qual` не перестанет дочитывать снимок.
8. `StandAnchor` (`stand.rs:1856-1872`) стоит заменить на `FakeChain::executed_state_hash`
   (`fakes.rs:609-619`) — ветка `Err` уже смоделирована, и она вход находки B1-01
   (B1-14).
9. Тест «два узла на разных высотах — одна запись» (§7 проекта) над сегодняшним общим
   `FakeStaking` будет тавтологией; ветвление по хэшу — предусловие его осмысленности.
   Юнит `two_anchors_on_different_branches_produce_the_same_record`
   (`committee/tests.rs:445`) это уже пинует НА УРОВНЕ МОДУЛЯ — стендовый тест должен
   пинать другое: что два УЗЛА приходят к одной записи при разных якорях.

**Для 4.2:**

10. `RethAnchor` держит собственный клон провайдера рядом с тем, что держит
    `RethStakingStateReader` (`node/dpos.rs:1440-1450`) — два клона одного провайдера,
    не два источника, но при переписывании `node/dpos.rs` их стоит свести.
11. `live_height` и `upstream_frontier` (`node/dpos.rs:1360`, `:778`) — первый теперь
    write-only на валидаторском пути; удаляются вместе.

**Для 4.3:**

12. `reader/epoch_transition.rs:639` (`track_and_trigger`, `committee[E+1]` для
    peer-set'а) — единственный оставшийся сайт, который читает комитет РАДИ маски
    tracked-окна. Он вход и Б2 (один ET), и 4.3 (`TrackedPeers`); разводить эти две
    работы по разным заходам опасно — обе меняют одно чтение.

**Процессное:**

13. B1-05 — доковый дрейф в `.claude/dpos_architecture/` (три якоря на удалённые
    символы + необновлённый `verified-against`). По проектному CLAUDE.md это блокер
    коммита, а не follow-up.
