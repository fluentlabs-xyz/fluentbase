# Ревью строки 4.1, заход Б2 — одна карта «запись + схема», один продюсер, `CertInlet` без второй таблицы. 2026-09-11

Ревьюер — Opus 5, свежий контекст. Объект — незакоммиченное рабочее дерево
`/home/djadjka/Work/fluentbase`, ветка `djadjka/dpos-reth-2.2-squashed`, HEAD на момент
ревью `5449a314` (ушёл вперёд базы захода `89046c93` чужими коммитами по
`contracts/staking/**` и `.dpos-study/**`; по `crates/` `git diff HEAD` = `git diff 89046c93`).
Пути без префикса — `crates/dpos/consensus/src/`; `node/` = `crates/node/src/`;
`reader/` = `crates/dpos/staking-reader/src/`. `HEAD:` = база захода.

Агентов не запускал. Правок в дереве не делал: единственный записанный файл — этот.
Все прогоны — мои, в этой сессии.

**Расхождение с постановкой ревью:** постановка говорит «20 файлов под `crates/`»;
`git status --short | grep crates/` даёт **19** (`committee/facade.rs` и
`testbed/tests.rs` не тронуты). Двадцатым, видимо, считался журнал.

---

## §0. Прямые ответы

### 1. Ворота — прогнал сам, verbatim по одной строке

Все зелёные; расхождений с журналом §5 нет ни в одной цифре. [KNOWN]

| команда | результат |
|---|---|
| `cargo test -p fluentbase-consensus --lib` | `test result: ok. 665 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 23.56s` |
| `cargo test -p fluentbase-consensus --features dpos-devnet-byzantine --lib testbed::` | `test result: ok. 40 passed; 0 failed; 0 ignored; 0 measured; 633 filtered out; finished in 32.63s` |
| `cargo test -p fluentbase-node --lib` | `test result: ok. 57 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 56.71s` |
| `cargo test -p fluentbase-staking-reader` | `test result: ok. 58 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s` |
| `cargo test -p fluentbase-consensus --test slasher_integration` | `test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s` |
| `cargo clippy -p fluentbase-consensus -p fluentbase-node --all-targets` | `warning: large size difference between variants` ×1 (`--> crates/node/src/dpos.rs:1866:1`, `ValidatorUpstream`) |
| `… --features dpos-devnet-byzantine` | то же одно предупреждение |
| `cargo fmt --check \| grep -c "Diff in"` | `0` |
| `cargo doc -p fluentbase-consensus --no-deps 2>&1 \| grep -c "unresolved link"` | `9` |

Счёт сходится с базой Б1: 668 − 5 (удалённые в консенсус-крейте) + 2 (новые) = 665;
staking-reader 60 → 58. Единственное предупреждение clippy — на `node/dpos.rs:1866`,
файл заход трогал, но диффа в этом месте нет (`git diff HEAD -- crates/node/src/dpos.rs`
не содержит ханков около `:1866`) — предупреждение действительно из базы.

Одна оговорка про ворота с фичей: строку из постановки надо писать ровно как
`--features dpos-devnet-byzantine` (фича прокидывается из `fluentbase-node`,
`crates/node/Cargo.toml:137`). Форма `--features fluentbase-consensus/dpos-devnet-byzantine`
ЛОМАЕТ сборку — `error[E0063]: missing field 'byzantine' in initializer of DposLayerConfig`
(проверено мной). Это свойство базы, не захода, но оркестратору стоит знать.

### 2. Одна карта — да, одна; `EpochSchemeProvider` — чистый вид; ретенция расширилась

**Вторых таблиц «эпоха → схема» в продакшн-коде нет.** [KNOWN] `grep -rn "Map<[^>]*Scheme\|CachedScheme\|schemes:"`
по `crates/dpos/consensus/src` + `crates/node/src` даёт: `committee/mod.rs:632`
(`SchemeCommittee.entries`, под `#[cfg(test)] pub(crate) mod testing`, `:608-609`),
одну строку ДОКА в `outer.rs:233` про удалённую, и три карты голосов слэшера
(`slasher/actor.rs:359-361` — не схемы). Продакшн-хранилище одно:
`EpochEntry { record, scheme }` (`committee/store.rs:64-67`), слот один.

**`impl CertProvider` ровно один** — `outer.rs:269-276`. [KNOWN]

**`EpochSchemeProvider` (`outer.rs:240-276`) — одно поле `committee: Arc<dyn Committee>`
и ТРИ делегирующих метода**, собственного состояния нет: `verifier_epochs()` (`:253-259`)
→ `committee.verifier_epochs()`, `latest_scheme()` (`:264-266`) → `committee.latest_scheme()`,
`scoped()` (`:273-275`) → `committee.scheme(scope.get())`. `register` и `cold_start_register`
удалены. [KNOWN]

**Ретенция.** HEAD: ПО СЧЁТУ и только внутри `register` —
`while map.len() > SCHEME_RETENTION_EPOCHS { map.pop_first(); }`
(`HEAD:outer.rs:373-375`), то есть 8 самых верхних ЗАРЕГИСТРИРОВАННЫХ эпох, и отсчёт
шёл от того, что зарегистрировали. Дерево: пол ОКНА,
`epoch(anchor) − SCHEME_RETENTION_EPOCHS` (`store.rs:172-175`, окно `:178-184`), прогон
на каждом `anchor_advanced` (`:612-616`) и перед каждым `install` (`:361`), ширина
карты `8 + 2 + 1 = 11`. То есть отсчёт переехал со счётчика регистраций на ЯКОРЬ.

Путь «marshal спросил эпоху ниже пола и раньше получал схему, а теперь `None`»
существует, но узкий: HEAD прунил ТОЛЬКО при регистрации, так что узел, который
долго ничего не регистрировал, держал старые записи неограниченно долго; дерево
роняет всё ниже `epoch(anchor)−8` на каждом продвижении якоря. Сверху регрессии нет
и быть не может: HEAD регистрировал из чтений, привязанных к EL-финализированной
высоте (`soft_enter_span` читал на `read_height_for(fin)` = `fin−K`), а модуль читает
на `ordering_finalized` — то есть на ~K блоков ВЫШЕ. Находка B2-09 (MINOR).

### 3. Продюсеры — два; гонки разобраны; ни одного отказа не потеряно

**Пишут в слот `scheme` ровно два места.** [KNOWN] `grep -rn "upgrade_scheme\|\.scheme("`:
`CommitteeStore::install` (`store.rs:332` — `(self.verifier)(&record)`, вне лока) и
`CommitteeStore::resolve_scheme` (`store.rs:387-390` — ТОТ ЖЕ `EpochVerifier`, отложенный
повтор), плюс `Committee::upgrade_scheme` (`store.rs:526-575`). Больше `entry.scheme = …`
не встречается.

**(а) Может ли отложенный `resolve_scheme` перезаписать signer-схему? НЕТ.** [KNOWN]
`resolve_scheme` берёт лок, и если `entry.scheme` уже `Some` — возвращает его
(`store.rs:376-381`); при повторном взятии лока пишет через
`entry.scheme.get_or_insert(built)` (`:390`), который existing НЕ трогает. Так что
последовательность «install(None) → upgrade_scheme(signer) → отложенный resolve_scheme»
оставляет signer на месте, а построенный verifier выбрасывается.

**(б) Два потока в `resolve_scheme` одновременно — безвредно.** [KNOWN] Оба строят,
один выигрывает `get_or_insert`, второй дропается. Важнее, что проигравший НЕ может
быть слабее: единственный продюсер — `epoch_verifier` (`committee/mod.rs:480-494`), и
он либо возвращает `None` (слот бикона пуст / бикон умер — `beacon.get()?.upgrade()?`,
`:486`), либо схему С оракулом `beacon.oracle_for(record.epoch)` (`:491`). Oracle-less
схемы этот продюсер выдать не может вообще, поэтому отсутствие гвардии
`is_beacon_active` на пути `get_or_insert` (в отличие от `upgrade_scheme`) ничего не
стоит. Пытался опровергнуть, ища путь, где `oracle_for` вернёт `None` для beacon-active
эпохи и результат закэшируется: `oracle_for` для до-бутстрап-эпох законно `None`, но это
одинаково для обоих потоков (функция эпохи, не времени) — расхождения между двумя
вызовами быть не может.

**(в) Отказы `upgrade_scheme` против `HEAD:outer.rs::register` — ни одного не потеряно,
добавлен четвёртый.** [KNOWN]

| HEAD `register` (`HEAD:outer.rs:332-376`) | дерево `upgrade_scheme` (`store.rs:526-575`) |
|---|---|
| вакантный слот ⇒ insert без проверок | нет записи ⇒ **отказ + `error!`** (`:529-541`) — НОВЫЙ, четвёртый |
| — | слот схемы пуст ⇒ insert без проверок (`:550-553`) — точный аналог «вакантного» HEAD |
| `existing.participants() != scheme.participants()` ⇒ отказ + `error!` | `entry.record.participants != *scheme.participants()` ⇒ отказ + `error!` (`:542-549`) |
| signer → verifier ⇒ отказ + `error!` | то же (`:554-561`) |
| `is_beacon_active` → не-active ⇒ отказ + `error!` + `ORACLE_DROP_REFUSED` | то же (`:562-572`), метрика под тем же именем (`store.rs:53`) |

Сравнение «другого комитета» переехало с `existing.participants()` на
`entry.record.participants` — то есть теперь сверяется с ЗАПИСЬЮ, а не с ранее
установленной схемой. Это строго сильнее.

**Зовёт `upgrade_scheme` ровно один продакшн-сайт** — `epoch_manager.rs:1367-1376`,
ДО `spawn_engine` (`:1377`). [KNOWN] Остальные вхождения — `epoch_manager.rs:2155`,
`beacon/surface.rs:1529`, `committee/tests.rs` — тесты.

**Путь «движок E запущен, а в карте verifier-схема» существует и безвреден.**
Отказ `upgrade_scheme` только предупреждает (`epoch_manager.rs:1371-1376`), спавн идёт
дальше. Достижим ровно один отказ — oracle-drop, — и он СОХРАНЯЕТ более сильную
beacon-active схему; движок держит свой signer-экземпляр отдельно
(`engine.rs:225`, `simplex::Config.scheme = cfg.scheme`). Собственный голос не страдает.
Это ровно поведение HEAD (там `register` тоже только логировал). Обратный порядок —
схема поднята, а спавн провалился (`spawn_engine` возвращает `false` в четырёх местах,
`epoch_manager.rs:1629-1663`) — у HEAD тоже был: `(cfg.register_scheme)` стоял в
`EpochEngine::new` на `HEAD:engine.rs:225`, ПЕРЕД тремя `mux.register` и перед
`WeightedVrf::try_new` на `HEAD:engine.rs:288`. Дерево в этой части строго лучше:
`try_new` поднят ДО подъёма схемы, так что отказ электора больше не оставляет signer'а
в карте.

### 4. Слабая ссылка на бикон (Д-25) — заполняется ровно один раз; бикон в процессе не пересобирается

`BeaconSlot = Arc<OnceLock<Weak<dyn Beacon>>>` (`committee/mod.rs:469`). Заполняется
тремя `let _ = slot.set(Arc::downgrade(&…))`: валидатор — `node/dpos.rs:1800` (сразу за
`beacon::build`, `:1785-1794`); follower — `dpos.rs:3282` (за `build_follower`, `:3270`);
стенд — `testbed/stand.rs:1971`. [KNOWN]

**Бикон за жизнь процесса не пересобирается.** `run_node_stack` (`node/dpos.rs:489-509`)
ветвится на `is_validator` РОВНО ОДИН раз и вызывает либо `launch_validator_overlay`
(→ `build_beacon_plane` `:737`, → `DposLayer::launch` `:2170`), либо
`launch_follower_overlay` (→ `DposLayer::launch_follower`, `cert_follow/mod.rs:252`).
Ни один из двух не вызывается повторно, `build_follower` — один раз. Переключение
follower↔signer «в процессе» сегодня не реализовано (это цель, а не факт), и
`build_beacon_plane` сам документирует, что плоскость строится ОДИН раз и переживает
переключение (`dpos.rs:959-967`).

**Если бы бикон когда-нибудь пересобрался** — `OnceLock` уже занят МЁРТВЫМ `Weak`,
`set` вернёт `Err`, который повсюду отбрасывается (`let _ =`), `upgrade()` навсегда
`None`. Тогда `epoch_verifier` возвращает `None`, и это НЕ «vote-only схема», а
**отсутствие схемы вообще**: `Committee::scheme` даёт `None`, marshal перестаёт
проверять сертификаты новых эпох, inlet дефёрит всё подряд. Уже установленные схемы
остаются. Находка B2-06 (MINOR — сегодня недостижимо, но ловушка стоит на прямой
дороге заявленной цели «валидаторы не рестартуют»).

**Что делает verifier при `upgrade() == None` — `None`, повтор позже.** [KNOWN]
`committee/mod.rs:486` `beacon.get()?.upgrade()?`; `resolve_scheme` отсутствие НЕ
кэширует (`store.rs:387` — `?` выходит, в карту ничего не пишется), так что эпоха,
прочитанная до появления бикона, подхватит схему на следующем `Committee::scheme`.
Запина на vote-only нет — потому что oracle-less схему этот продюсер построить не умеет.

### 5. Отложенный повтор (Д-26) и цена на пути marshal'а

`Committee::scheme` (`store.rs:517-524`) = `self.committee(epoch).ok()?` + `resolve_scheme`.
`committee()` на ПРОМАХЕ внутри окна и при исполненном якоре делает
**два блокирующих staticcall'а** — `epoch_committee_snapshot` (`store.rs:482-485`) и
`dkg_qual` (`:496-499`). [KNOWN] Это не только «BLS-агрегация из записи»: на промахе
платится полное чтение.

`CertProvider::scoped` (`outer.rs:273-275`) зовёт ровно это. HEAD:
`self.map.lock().unwrap().get(&scope).cloned()` (`HEAD:outer.rs:416`) — чистый поиск в
карте, ни одного блокирующего чтения. Значит **актор marshal'а теперь может
заблокироваться на двух reth-staticcall'ах там, где раньше брал мьютекс**. Поток —
консенсусный рантайм commonware, в котором marshal и живёт (`outer.rs` `MarshalActor::init`,
запуск в `build`); отдельного пула для этого нет.

Промах ограничен: вне окна и ниже `commit_height` ответ арифметический без EVM
(`store.rs:437-451`), попадание — один лок. Пытался опровергнуть тем, что все эпохи,
о которых спрашивает marshal, уже прочитаны `soft_enter` (`epoch_manager.rs:1412`) или
`register_span` (`:1870`) — не вышло: marshal скоупит эпоху сертификата, пришедшего от
пира, и такой эпохи никакой предварительный проход не гарантирует. Находка B2-03
(MODERATE). Замера нет ни у исполнителя (§8.4 журнала признаёт), ни у меня.

### 6. Пробуждение на каждом `anchor_advanced` (Д-28) — подтверждено, и до него класс парковки повтора не имел

`store.rs:637-640`:
~~~
self.readable.send_if_modified(|current| {
    *current = (*current).max(highest);
    true
});
~~~
Замыкание возвращает `true` БЕЗУСЛОВНО, а `tokio::sync::watch::Sender::send_if_modified`
уведомляет ровно по возврату замыкания — значит уведомление на каждом вызове, при
равном значении тоже. Значение держится монотонным через `max`. HEAD (`89046c93`)
публиковал только при росте: `if highest > *current { *current = highest; true } else { false }`
— видно в диффе `store.rs`. [KNOWN]

**Подписчик один** — `epoch_manager.rs:707` (`let mut committee_readable = self.cfg.committee.subscribe();`),
ветка `select!` на `:803-805` → `reconcile_live`. Больше `subscribe()` у модуля никто
не зовёт (grep по `crates/`). Зовущих `anchor_advanced` двое: executor через стёртую
до одного глагола замыкалку (`outer.rs:959-962` → `executor.rs:1115`, `:2485`, `:3605`)
и поллер плоскости сразу после заморозки геометрии (`node/dpos.rs:1557`).

**Что стоит одно пробуждение.** `reconcile_live` (`:1485-1497`) → `reconcile_roles`:
чтение записи (попадание = лок + клон `Arc`, `store.rs:458-460`), `highest_entered_epoch.max`,
`randomness.observe_epoch`, `prune_resolved`, `abort_below`, `is_live_epoch`, затем
для живого signer'а ранний выход через `active_epochs.get_mut` + `can_participate`
(два чтения стора бикона). EVM-чтения на этом пути нет, пока запись в карте.
При ~1 событии/с это тот же класс, что уже несёт `spawn_unblocked`.

**Класс «якорь есть, состояние не исполнено» до Д-28 повтора не имел — подтверждаю.**
`executed_hash(anchor) == Ok(None)` (`store.rs:469-475`) даёт `NotReadable{ready_at: anchor_height}`,
и открывается он, когда исполнение догоняет на ТОЙ ЖЕ высоте: `highest_readable`
(`:157-159`) считается из `anchor.height()` и не двигается, значит публикация «по росту»
этот класс не будила вовсе. Единственный другой повторитель — сам `anchor_advanced`,
который тогда молчал. Свидетель исполнителя (стенд, `[70,70,70,70]`) я не воспроизводил
— это релей; но механизм по коду закрыт.

### 7. Громкий отказ старта → `warn!` (Д-27) — отклонение от §5.1, и проверка почти вырожденная

**Что защищал `bail!`.** `HEAD:dpos.rs:1993-2013`: прямое
`reader.epoch_committee_snapshot(initial_epoch_u64, latest_finalized_hash)?` (ошибка
чтения ⇒ фатал через `?`) плюс `eyre::bail!` на пустом комитете. Дерево
(`dpos.rs:1991-2013`): `match committee.committee(initial_epoch_u64)` → любой `Err` ⇒
`warn!` с полным операторским текстом, выполнение продолжается. То же на follower'е
(`dpos.rs:3408-3421`) и на стенде (`testbed/stand.rs:2051-2062`). [KNOWN]

**Что ИЗ этого классов теперь виден только как `warn!`:**
- «свежий datadir / цепь не дошла до коммита / DPoS ещё не активирован» — `NotReadable`;
- «геометрия ещё не заморожена» — `NotReadable { ready_at: 0 }` (`store.rs:428-431`);
- постоянный класс (`Read(permanent)`, `weights: None`, форк) — тоже `warn!` ЗДЕСЬ,
  но модуль сам печатает `error!` один раз и тикает `dpos_committee_read_permanent_total`
  (`store.rs:192-207`), так что громкость сохраняется.

Неверный `staking_address` до этой точки не доходит: `interval`/`dpos_activation_block`
и `active_validators_length` читаются выше и остаются фатальными
(`dpos.rs:1936-1959` — `?` и `return Err`). Это сильно сужает потерю.

**Но проверка стала почти вырожденной на валидаторе.** Геометрия стора приходит с
`geometry_tx` (`node/dpos.rs:1389`), который заполняет ПОЛЛЕР плоскости
(`:1549`) — задача, которую `build_beacon_plane` только СПАВНИТ (`:1452`), — а
`DposLayer::launch` вызывается заметно позже (`:2170`). Пока геометрия `None`,
`committee()` отвечает `NotReadable` БЕЗ единого чтения. То есть ветка `warn!` на
холодном старте валидатора — ожидаемая, а не исключительная, и она забивает собой тот
самый операторский текст, ради которого сохранена.

**Ребро повтора для этого пути.** Единственный потребитель пробуждения —
`epoch_manager.rs:803` → `reconcile_live` (`:1485-1497`), который начинается с
`if let Some(epoch) = self.latest_live` и `if !self.is_live_epoch(epoch) { return; }`.
`latest_live` заполняется ТОЛЬКО доставкой границы (`epoch_manager.rs:756-758`).
Значит **до первой границы стартовую эпоху не перечитывает никто**: ни
`spawn_unblocked` (тоже через `reconcile_live`), ни `vote_backup`. Узел не «висит до
рестарта» — он входит в эпоху на первой же границе, — но окно между стартом и первой
границей до целой эпохи, и в нём marshal не имеет схемы стартовой эпохи. На HEAD этого
окна не было: `cold_start_register` регистрировал схему синхронно.

Находка B2-01 (SERIOUS). Проект (§5.1 «громкий отказ запуска с операторским
сообщением, как сегодня», PLAN.md:100 та же формулировка) предписывал обратное.

### 8. Inlet без фатала (Д-34) — вердикт: MODERATE, не SERIOUS

`HEAD:node/cert_inlet.rs:117-124` ронял узел на `Err` из `ingest`; `Err` брался из
`scheme_at_finalized_tip`. Дерево: `cert_inlet.rs:619-639` — `let Some(scheme) = self.committee.scheme(epoch) else { … return Ok(()) }`. [KNOWN]

**Что делает `ingest` на `None` для эпохи с ПОСТОЯННОЙ ошибкой:** тикает
`dpos_cert_inlet_committee_read_deferred_total{reason="committee_not_committed"}`,
один раз за эпизод пишет `warn!`, **пропускает этот сертификат и возвращает `Ok(())`**.
Сертификат ТЕРЯЕТСЯ (нигде не откладывается), `consecutive_faults` не трогается
(`:635-638`). Очередь НЕ блокируется — цикл продолжает дренаж, и `upstream_frontier`
продолжает расти (`:562-566`), то есть re-jump-триггер follower'а живой. Но marshal
навсегда стоит на дыре: cert-follow follower новых тел не получает.

**Где виден постоянный класс.** `store.rs:192-207` — счётчик
`dpos_committee_read_permanent_total{reason}` инкрементируется на КАЖДОМ вызове, а
`error!` печатается один раз на эпоху, пока эпоха не выпала из окна (`reported` чистится
тем же полом, `store.rs:174`). Открыл и подтвердил: этот путь идёт через
`CommitteeStore::committee`, то есть через `Committee::scheme`, то есть его проходит
именно inlet — не только слэшер. [KNOWN] Причины покрыты все шесть
(`REASON_WEIGHTS_NONE`, `_LEN`, `_NON_UNIQUE_KEYS`, `_FORK`, `_READ_ERROR`, `_ANCHOR_FAULT`).

**Вердикт — приемлемая замена, MODERATE, не SERIOUS.** Обоснование: (1) корректный
класс НЕ стал невидимым — он громкий, просто печатает его модуль, а не супервизор;
(2) исчезнувшее поведение («узел падает на ревёрте контракта, который он читает
на бэкфилле») — как раз тот дефект E4-16, ради которого модуль и различает
транзиентное от постоянного, а `eyre::Result` не различал; (3) постановка Б2.2
предписывала ровно этот дефер-путь дословно. Что ОСТАЁТСЯ потерей и в цену не
записано: у оператора больше нет ОДНОГО сигнала «узел умер» — есть метрика и одна
строка `error!`, которую надо было заметить; сквозного теста «ревёрт контракта на
пути inlet'а виден и не роняет узел» не существует (§8.5 журнала честно это признаёт).
Плюс отдельно — мёртвая фатальная ветка, B2-04.

### 9. Б2.3 не сделан — обе причины подтверждены по коду

**(а) Коалесценция плоскостного ET против точечной детекции границы — ПОДТВЕРЖДАЮ.** [KNOWN]
`node/dpos.rs:1494` — `let latest = finalized_rx.borrow_and_update().as_ref().map(|h| h.number);`,
то есть берётся ПОСЛЕДНЕЕ значение вотча, промежуточные высоты не проходят.
`node/dpos.rs:1538` — `et.lock().await.on_finalized(fin).await` только для этого `fin`.
Детекция границы точечная: `reader/epoch_transition.rs:495` `is_epoch_boundary(number, activation, interval)`
и `:546` `if is_boundary && self.last_tracked_epoch < Some(next)` — требуется РОВНО
граничная высота. Значит на догоне плоскостной ET границу пропускает.

**Это ДЕФЕКТ HEAD, и он не компенсируется.** Диффа в этом месте `node/dpos.rs` нет —
`git diff HEAD -- crates/node/src/dpos.rs` не содержит ханков между `:1440` и `:1560`.
`pending_boundary` тут не спасает (он паркует УЖЕ ОБНАРУЖЕННУЮ границу, а не находит
пропущенную), `cold_start`-ветка перевзводится только пока `last_tracked_epoch.is_none()`
(`:518`). Компенсация частичная и другой природы: следующая ОБНАРУЖЕННАЯ граница
трекает `epoch_e + 1` сразу, перепрыгивая пропущенные — то есть peer-set сходится, но
промежуточные составы не трекаются никогда.

**Сегодня последствие ограничено peer-set'ом:** плоскостной ET строится с `None`
вместо `bridge_tx` (`node/dpos.rs:1355-1362`, четвёртый аргумент), то есть в
epoch-manager границы НЕ шлёт. Триггеры границ даёт signer-плоскостной ET, ведомый
хуком доставки по КАЖДОМУ финализированному блоку (`dpos.rs:2291-2294`, `boundary_hook`
→ `enter(block.height)`). Именно это и делает слияние опасным без правки: отдать
`bridge_tx` коалесцирующему инстансу — обменять дефект peer-set'а на потерю входа в
эпоху.

**(б) `cold_start` присваивает `anchor_height` безусловно — ПОДТВЕРЖДАЮ.** [KNOWN]
`reader/epoch_transition.rs:694` — `self.anchor_height = Some(head_number);` без `max`.
И плоскостной инстанс cold-start'ится на каждом тике до заморозки:
`node/dpos.rs:1533-1552` — `if frozen_before { on_finalized } else if block_hash ok { cold_start }`.
Док `raise_anchor_height` (`reader/epoch_transition.rs:311-318`) говорит это прямым
текстом.

**Оценка: отложить в отдельный заход — разумно.** Оба свойства лежат в
`reader/epoch_transition.rs` и в поллере `node/dpos.rs`, то есть в файлах, которые
постановка Б2 либо запрещает, либо открывает узко. Минимум, названный журналом
((а)–(г)), по коду полный, с одним уточнением к (в): типы двух инстансов совпадают
(`RethStakingStateReader<node.provider, node.evm_config>` + `OracleHandle` в обоих —
`node/dpos.rs:1349-1362` и `dpos.rs:2036-2044`), так что проводка `Arc<Mutex<…>>`
действительно возможна без трейта. Я бы добавил (д): удалить `raise_anchor_height`-seam
или доказать, что он не конфликтует с повторным `cold_start`, потому что после слияния
единственный инстанс будет и cold-start'иться поллером, и получать `raise` от
executor'а (`dpos.rs:2302-2308`).

### 10. Канал границ по `Epoch` (Д-33) и снимок из записи — повтор Д-18 проходит

`CommitteeRecord::snapshot_view()` (`committee/mod.rs:174-195`) синтезирует
`tombstoned: false`, `activation_epoch: 0`, `weights: Some(self.weights.clone())`;
`block_hash`/`block_number` — из `record.snapshot`, то есть высота ЯКОРЯ этого узла, а
не граничного блока.

**Кто читает ниже — три потребителя, ни один не читает синтезированные ноги.** [KNOWN]

| потребитель | что берёт из снимка | читает `tombstoned`/`activation_epoch`? |
|---|---|---|
| `seedless_base` → `constant_fallback_seed` (`beacon/seed.rs:93-105`) | `snap.epoch` + `peer_pubkey` всех членов | нет |
| `WeightedVrf::try_new` (`weighted_vrf.rs:86-113`) | `snap.weights` + `peer_pubkey` | нет |
| `Beacon::signer` (`beacon/surface.rs:881-913`) → `epoch_committee_from_snapshot` | peer + bls ключи | нет |
| `EpochEngineConfig.snapshot` → `EpochEngine::new` (`engine.rs:199-207`, `:259-262`) | то же `epoch_committee_from_snapshot` | нет |

Полный grep `tombstoned` по `crates/dpos/consensus/src` + `crates/node/src` даёт
единственного продакшн-читателя — `slasher/tombstone.rs:45`, и он работает НАД ЖИВЫМ
снимком tombstone-поллера (`node/dpos.rs:1612`), не над проекцией. `activation_epoch`
в продакшне не читает никто. Д-18 для нового пути держится.

**`latest_live: Option<Epoch>` — все читатели переведены, проверки «тот же состав» не
потеряно.** [KNOWN] На HEAD `reconcile_live` (`HEAD:epoch_manager.rs:1414-1419`) брал
КЭШИРОВАННЫЙ снимок и передавал его дальше — никакого сравнения снимков там не было
ни на HEAD, ни где-либо ещё (grep по `latest_live`). Перечитывание записи строго
сильнее: кэш мог отстать от записи, запись write-once не может.

Одно НЕ-тождество, которое стоит назвать: на HEAD снимок границы читался ET на
`read_height_for(boundary)` = `boundary − K`, теперь запись читается на
`executed_state_hash(ordering_finalized)`. Значение записи в окне от якоря не зависит
(инвариант §5.1), так что состав, ключи, бит и веса те же; отличаются только
диагностические `block_number`/`block_hash`, которых никто не читает.

### 11. `register_span` останавливается на первой нечитаемой эпохе (Д-32) — как и HEAD

`epoch_manager.rs:1870-1878`: цикл `from..=to`, `if committee.scheme(epoch).is_none() { break; }`.
HEAD: `HEAD:reader/epoch_transition.rs:705-735` — тот же `break` на первом пустом /
нечитаемом / ошибочном чтении, с тем же обоснованием «коммит-курсор — контигуозный
префикс». Регрессии «эпохи за дырой не регистрируются никогда» нет — её не было и на HEAD. [KNOWN]

На `Read(transient)` в середине диапазона остаток повторяет тот же
`pipeline_catchup_span` на следующем backup-vote-хинте (`epoch_manager.rs:1021-1027`),
с той же супрессией `catchup_no_progress` — механика не изменилась. `OutOfWindow`
сверху — ожидаемо и теперь бесплатно (отказ без EVM, `store.rs:437-442`).

Одно расхождение с HEAD, узкое: HEAD'овский обёрточный цикл в `outer.rs`
(`HEAD:outer.rs:1183-1207`) на неудаче ПОСТРОЕНИЯ схемы (`soft_enter_verifier` вернул
`None`) НЕ ломал цикл, а шёл дальше; `register_span` ломает, потому что `scheme()`
склеивает «запись не читается» и «verifier не построился». В продакшне недостижимо
(слот бикона заполнен задолго до epoch-manager'а). Находка B2-08 (MINOR).

### 12. `WeightedVrf` поднят до `upgrade_scheme` (Д-30) — поведение при отказе стало ЛУЧШЕ

HEAD: `WeightedVrf::try_new(&cfg.snapshot, cfg.fallback_seed)?` внутри `EpochEngine::new`
(`HEAD:engine.rs:288`), то есть ПОСЛЕ `(cfg.register_scheme)(cfg.epoch, scheme.clone())`
на `HEAD:engine.rs:225`. `?` выходил из `new` → `spawn_engine` ловил
`Err(e) => warn!("skipping epoch spawn — invalid committee snapshot"); return false`
(и сегодня `epoch_manager.rs:1628-1633`). Итог на HEAD: схема эпохи УЖЕ signer в
реестре, движка нет, `verifier_epochs()` эпоху не видит ⇒ repair-sweep её не чинит.

Дерево: `epoch_manager.rs:1298-1318` — `try_new` до всего, на `Err` `error!` + `soft_enter`
+ `return`, схема остаётся verify-only. Свойство E4-10 сохранено и усилено. [KNOWN]

Окно «схема поднята, спавн провалился» осталось для трёх `mux.register`
(`epoch_manager.rs:1635-1663`) — ровно как на HEAD.

### 13. `latest_scheme` может назвать будущую эпоху — да, и класс чуть шире HEAD

`store.rs:589-597`: `records.values().rev().find_map(|e| e.scheme.clone())` — самая
высокая эпоха карты со схемой. Карта держит до `epoch(anchor) + MAX_COMMITTEE_LOOKAHEAD_EPOCHS`
= `+2` (`store.rs:178-184`). Потребитель — `peers_for_finalization` (`outer.rs:914-922`),
цели fetch'а финализаций для executor'а. [KNOWN]

Значит да: в `latest_scheme` может попасть `epoch(anchor)+2`, чей комитет закоммичен,
но не живой, и fetch будет адресован его участникам. Вред ограничен: это настоящие
валидаторы закоммиченной эпохи, а `track` держит `C[E−1] ∪ C[E] ∪ C[E+1]`, так что
часть целей может быть вне peer-set'а и просто не ответить — потеря раунда fetch'а,
не безопасность.

Регрессией это не назову: на HEAD `soft_enter_span` регистрировал контигуозный
закоммиченный префикс, читая на `fin − K`, то есть тоже доходил до `epoch(fin−K)+2`.
Дерево читает на `ordering_finalized`, то есть потолок на ~K блоков выше, и ВДОБАВОК
в карту теперь попадает любая эпоха, которую кто-нибудь просто ПРОЧИТАЛ (`is_member`,
`changed`, `scheme` из inlet'а) — раньше в реестр попадали только зарегистрированные.
Находка B2-15 (MINOR).

### 14. Тесты

**Счёт `#[test]`/`#[tokio::test]`, HEAD против дерева, по всем 19 файлам** (посчитано
мной, `grep -c`): [KNOWN]

| файл | HEAD → дерево |
|---|---|
| `committee/tests.rs` | 22 → **24** |
| `cert_inlet.rs` | 28 → **25** |
| `outer.rs` | 5 → **3** |
| `reader/epoch_transition.rs` | 31 → **29** |
| `beacon/surface.rs` 14→14, `cold_start_jump.rs` 23→23, `dpos.rs` 31→31, `epoch_manager.rs` 27→27, `executor.rs` 120→120, `fault.rs` 4→4, `tests/slasher_integration.rs` 16→16, `node/dpos.rs` 8→8, остальные 0→0 | без изменений |

Совпадает с журналом §0.5 до цифры.

**Удалённые семь, поимённо, с вердиктом:**

| тест | оправдано? |
|---|---|
| `outer::…::register_refuses_a_signer_to_verifier_downgrade_but_accepts_the_upgrade` | ДА — **перенесён** в `committee/tests.rs:1352` |
| `outer::…::register_refuses_a_replacement_that_drops_the_beacon_oracle` | ДА — **перенесён** в `committee/tests.rs:1409` |
| `cert_inlet::…::committee_read_fault_defers_transient_and_blocknotfound_else_corruption` | ДА — предмет исчез (см. ниже) |
| `cert_inlet::…::state_not_materialized_defers_non_fatally_and_ticks_counter` | ДА — исхода больше нет, у inlet'а один `None` |
| `cert_inlet::…::corrupt_committee_read_stays_fatal` | предмет исчез, НО замены нет (см. ниже) |
| `reader::…::soft_enter_span_registers_contiguous_committed_prefix` | ДА — вместе с методом |
| `reader::…::soft_enter_span_unresolvable_read_state_registers_nothing` | ДА — вместе с методом |

**`committee_read_fault_defers_transient_and_blocknotfound_else_corruption`** доказывал,
что inlet маппит ошибку комитетного чтения в `FaultClass`: транзиент и `BlockNotFound`
⇒ дефер, остальное ⇒ corruption. Свойство ЖИВО, но в другом месте и над типизированной
ошибкой: `CommitteeError::is_transient` (`committee/mod.rs:262-268`) плюс
`ReadError::is_transient`, и пинуется в `tests/slasher_integration.rs`
(`an_epoch_above_the_window_is_retried_until_the_anchor_reaches_it`,
`an_epoch_below_the_window_is_dropped_for_good`,
`a_torn_anchor_probe_costs_the_evidence_a_retry_not_its_life` — все три в дереве есть,
`grep` подтверждает). Замена адекватная и сильнее (типы вместо строк).

**`corrupt_committee_read_stays_fatal`** доказывал, что коррумпированное чтение
комитета ронит inlet. Свойства больше НЕТ и быть не должно (Д-34). Замены —
«узел на форке контракта остаётся жив, класс виден» — **нет ни одного теста**: ни на
стороне inlet'а, ни на стороне модуля (в `committee/tests.rs` есть тесты на
`REASON_FORK`/`reported_epochs`, но связка «через `Committee::scheme` до
`CertInlet::ingest`» не пинуется). Назвать замену не могу — её нет.

**Два новых** (`committee/tests.rs:1352-1394`, `:1409-1443`): утверждают, что
verify-only схема приходит ВМЕСТЕ с записью без всякого регистратора
(`assert!(!is_signer(), "the module's own entry is verify-only")`), что signer садится
сверху, что signer→verifier и чужой комитет отклонены, что beacon-active не заменяется
oracle-less и что обратный апгрейд садится. Против оригиналов — **усиление**: слот
теперь заполняет модуль, а не сам тест.

**Мутацию я НЕ прогонял, и это отказ по правилам, а не пропуск.** Правило 2 постановки
ревью — «Никаких правок файлов вообще, кроме одного» — запрещает временно править
исходник, а правило 7 предупреждает, что параллельно идут прогоны оркестратора, так
что окно «мутировал → прогнал → откатил» реально рискует попасть в чужую сборку.
Поэтому чувствительность двух новых тестов у меня — **[ГИПОТЕЗА]** по коду (обе гвардии
`store.rs:554-561` и `:562-572` стоят ровно между ассертами и возвратом `true`, так что
их снятие обязано покрасить `assert!(!store.upgrade_scheme(…))`), а утверждение
журнала §4 о прогнанных мутациях — **релей, мной не проверен**.

Одно упущение в покрытии: у `upgrade_scheme` ЧЕТЫРЕ отказа, а перенесённые тесты
пинуют три. Четвёртый — «нет записи» (`store.rs:529-541`) — единственный новый в этом
заходе, и его не проверяет ничто. Находка B2-10 (MINOR).

**Изменённый `the_wake_up_carries_the_highest_readable_epoch_as_the_anchor_grows`
(`committee/tests.rs:797`) — УСИЛЕНИЕ.** Снята посылка «якорь не двинулся ⇒ ничего не
публикуется» (она была утверждением об условии публикации, которое Д-28 и меняет); на
её место встали две: событие приходит и на продвижении, не поднявшем потолок, И
значение не убывает — причём проверено против ПОНИЖЕННОГО якоря
(`anchor.advance(geometry().start(1), hash(13))`), а не против равного. Это строго
больше утверждений, чем было.

**`git diff HEAD -- crates/dpos/consensus/src/testbed/tests.rs` — пуст.** [KNOWN]
(файла нет в `git status`, `--stat` ничего не печатает).

### 15. Продакшн-путь: запрещённого не добавлено

`git diff HEAD -- crates/ | grep "^+"` по `unwrap()|expect(|#[allow|todo!|unimplemented!|dbg!|sleep|Duration::from`: [KNOWN]

- `#[allow]`, `todo!`, `unimplemented!`, `dbg!`, таймеры/`sleep`/`Duration::from` — **ноль**;
- `unwrap`/`expect` на продакшн-пути — **ровно пять**, все `self.state.lock().unwrap()`
  в `committee/store.rs`: `resolve_scheme` (`:376`, `:388`), `upgrade_scheme` (`:528`),
  `verifier_epochs` (`:581`), `latest_scheme` (`:591`). Та же конвенция лока, что у
  уже существовавших в этом файле;
- всё остальное — под `#[cfg(test)]` (`committee/mod.rs` testing-двойник,
  `committee/tests.rs`, `cert_inlet.rs` тесты, `testbed/`).

Совпадает с журналом §0.8.

### 16. Файлы вне списка постановки (Д-31) — четыре, все ханки перечислены

| файл | ханки | что это |
|---|---|---|
| `executor.rs` | 2 (`:11833-11843`, `:12006-12016`) | только `mod tests`: `EpochSchemeProvider::new()` + `provider.register(…)` → `SchemeCommittee::new(…)` + `EpochSchemeProvider::new(module)` |
| `cold_start_jump.rs` | 1 (`:1260-1271`) | только `mod tests`: удалён `impl` мёртвого метода трейта `scheme_at_finalized_tip` |
| `fault.rs` | 1 (`:18`) | ТОЛЬКО док-комментарий: убрана ссылка на удалённый `cert_inlet::committee_read_fault` |
| `beacon/surface.rs` | 3 (`:1484-1536` — док+тело теста; `:1501` — `use`; `:2562-2572` — комментарий ВНУТРИ продакшн-кода `impl Randomness for LiveBeacon`) | тест + комментарии; кода не тронуто |

То есть «только тесты/доки» — ДА. Но постановка Б2 запрещала `beacon/**` «целиком» и
`executor.rs` «кроме нуля правок»; оба тронуты. Механически неизбежно (удалён метод
трейта и изменена арность конструктора), журналом признано как Д-31, но правилом
постановки это всё же нарушение. Находка B2-11 (MINOR).

### 17. Где журнал вводит в заблуждение

Содержательных искажений я не нашёл — журнал необычно честен, §8 сам перечисляет
слабые места. Нашёл три СБИТЫХ ЯКОРЯ и одну неточную формулировку: [KNOWN]

1. **Д-29 указывает `epoch_manager.rs:832`** — там комментарий; ветка
   `_ = committee_readable.changed() =>` реально на `:803`, подписка на `:707` (§2
   журнала даёт верные `:803`/`:707`, Д-29 — нет).
2. **Д-26 указывает `store.rs:349`** — там строка текста `error!`; `resolve_scheme`
   реально `:374` (§2 даёт верный).
3. **Д-33 указывает `dpos.rs:2288`** для «epoch_manager перечитывает запись сам» —
   на `:2288` закрывающая скобка; форвардер границ на `:2597-2598`.
4. **§0.7, четвёртый буллет**: «с ним пропадает исход … ФАТАЛ inlet'а». Формально
   исход не «пропал», а стал НЕДОСТИЖИМЫМ: ветка `if let Err(e) = inlet.ingest(uf).await { error!("cert-inlet fatal (committee read)"); break; }`
   стоит на месте и на валидаторе (`node/cert_inlet.rs:91-95`), и у follower'а, а
   `ingest` больше не имеет ни одного `Err`-пути (`cert_inlet.rs:511-752`, ни `?`, ни
   `bail!`, ни `return Err`). Это не искажение по сути, но мёртвый код журналом не
   назван — см. B2-04.

Отдельно: журнал §0.5 говорит «Удалены СЕМЬ», а §5 — «668 − 5 удалённых + 2 новых =
665». Обе цифры верны (семь по всем крейтам, пять по консенсус-крейту), но рядом
читаются как противоречие; стоило сказать «пять из семи — в консенсус-крейте».

### 18. Что из §5.1/§5.5 неверно против кода

Журнал §0.7 называет четыре места. **Все четыре подтверждаю по коду**, плюс нашёл пятое.

1. **«`install` кладёт запись вместе с verify-схемой атомарно» — верно, но недостаточно.**
   ПОДТВЕРЖДЕНО: бикон строится ИЗ модуля на обоих классах (`node/dpos.rs:1787-1794`
   принимает фасад, `dpos.rs:3270-3276` `build_follower` — `committees: follower_committees`),
   значит стор старше бикона и `(self.verifier)(&record)` на раннем `install` вернёт
   `None` (`committee/mod.rs:486`). Держится только вместе с Д-26 (`store.rs:374-391`)
   и Д-25 (слабая ссылка, `committee/mod.rs:469`).
2. **«`subscribe` = наивысшая читаемая эпоха выросла» — ложно как условие публикации.**
   ПОДТВЕРЖДЕНО, механизм в §0.6 выше.
3. **«громкий отказ старта» — стало `warn!`.** ПОДТВЕРЖДЕНО, `dpos.rs:1991-2013`;
   и это единственное из четырёх, где отклонение ПРОТИВОРЕЧИТ явному требованию
   проекта и PLAN.md:100, а не уточняет его. См. B2-01.
4. **§5.5 не называет исчезающий фатал inlet'а.** ПОДТВЕРЖДЕНО, `cert_inlet.rs:619-639`.

**Пятое, журналом не названное: §5.1 «`EpochSchemeProvider` становится видом на эту
карту» — верно по структуре, но молча меняет КЛАСС ОПЕРАЦИИ у `CertProvider::scoped`.**
Проект описывает вид на карту; в коде вид на карту оказался видом на ЛЕНИВОЕ ЧТЕНИЕ
(`store.rs:517-524` → `committee()` → два staticcall'а на промахе). Проект нигде не
говорит, что marshal теперь может блокироваться на EVM, и §5.1 разбирает стоимость
только для потребителей, которые и раньше читали. См. B2-03.

### 19. Граница — таблица имён

Полная таблица в §3. Ключевое: `SchemeCommittee` — `#[cfg(test)] pub(crate) mod testing`
(`committee/mod.rs:608-609`, сам тип `pub(crate) struct` `:630`), **в релизе его нет**. [KNOWN]

---

## §1. Находки

| id | серьёзность | file:lines (дерево) | HEAD-якорь | что не так | чем пытался опровергнуть и почему не вышло | уверенность |
|---|---|---|---|---|---|---|
| B2-01 | SERIOUS | `dpos.rs:1991-2013`; follower `dpos.rs:3408-3421`; стенд `testbed/stand.rs:2051-2062` | `HEAD:dpos.rs:1993-2013` (`epoch_committee_snapshot(…)?` + `eyre::bail!`) | Проект (§5.1 «громкий отказ запуска с операторским сообщением, как сегодня»; PLAN.md:100 дословно то же) предписывал ЛОУДНЫЙ отказ; стало `warn!` + продолжение. Хуже того, проверка на валидаторе почти вырожденная: геометрия стора приходит от поллера плоскости (`node/dpos.rs:1549`), который `build_beacon_plane` только спавнит (`:1452`), а `DposLayer::launch` идёт позже (`:2170`) — значит в момент чтения геометрия обычно `None` и `committee()` отвечает `NotReadable` без единого чтения (`store.rs:428-431`). Ветка `warn!` становится штатной и забивает операторский текст. До первой границы стартовую эпоху не перечитывает НИКТО: единственное ребро повтора — `epoch_manager.rs:803` → `reconcile_live` (`:1485-1497`), гейтится на `latest_live.is_some()`, а `latest_live` пишет только доставка границы (`:756-758`). На HEAD окна не было — `cold_start_register` регистрировал схему синхронно | Искал второе ребро повтора: `spawn_unblocked`, `vote_backup`, биконовый поток — все идут через `reconcile_live` с тем же гейтом; `pipeline_catchup_span` гейтится backup-vote-хинтом. Искал, не остаётся ли фатальным неверный `staking_address`: остаётся (`dpos.rs:1936-1959` читает `interval` и `active_validators_length` с `?`), это сужает класс, но не отменяет ни вырожденность проверки, ни потерю схемы стартовой эпохи до первой границы | высокая |
| B2-02 | SERIOUS | `epoch_manager.rs:1485-1497` + `:1046-1048` | `HEAD:epoch_manager.rs:1408-1421` | Ребро повтора Д-29 (`committee.subscribe()` → `reconcile_live`) гейтится на `is_live_epoch(latest_live)` = `epoch >= highest_observed_epoch`. `reconcile_roles` теперь возвращается НИЧЕГО не решив на любой ошибке чтения записи (`:1073-1083`) — включая всю бухгалтерию и `soft_enter`. Значит отложенная граница для НЕ-живой эпохи (раздутый `highest_observed_epoch`, R-003) повтора от этого ребра не получает вовсе: схема эпохи не регистрируется, marshal её сертификаты не проверяет. На HEAD граница несла снимок и ВСЕГДА доходила до `soft_enter` | Искал, нет ли отдельного множества отложенных чтений по аналогии с `deferred_spawns`: нет, `reconcile_roles` выходит до любой записи состояния. Искал альтернативный ремонт: `register_span` (`:1870`) через `pipeline_catchup_span` действительно может дочитать такую эпоху, но только по backup-vote-хинту и с супрессией `catchup_no_progress` — то есть не гарантированно и не на каждом ребре. Проверял, не гейтится ли сама ветка границы: нет, `:756-760` зовёт `reconcile_roles` напрямую — потому и SERIOUS, а не BLOCKER | высокая |
| B2-03 | MODERATE | `outer.rs:269-276`; `committee/store.rs:517-524`, `:458-504` | `HEAD:outer.rs:414-419` (`self.map.lock().unwrap().get(&scope).cloned()`) | `CertProvider::scoped` у marshal'а превратился из поиска в карте в ленивое чтение: промах внутри окна при исполненном якоре стоит ДВУХ блокирующих reth-staticcall'ов (`store.rs:482-485`, `:496-499`) прямо в акторе marshal'а консенсусного рантайма. Проект §5.1 описывает `EpochSchemeProvider` как «вид на карту» и стоимость этого перехода не разбирает. Замера нет | Пытался показать, что промах невозможен: `soft_enter` (`epoch_manager.rs:1412`) и `register_span` (`:1870`) покрывают эпохи, которые вёл менеджер, но marshal скоупит ещё и эпоху сертификата, пришедшего от пира, — её не читает заранее ничто. Пытался найти неблокирующий путь: `Committee` синхронный по контракту (`committee/mod.rs:271-277`), отдельного пула нет | высокая (факт), средняя (величина) |
| B2-04 | MODERATE | `node/cert_inlet.rs:87-95`; follower-цикл в `dpos.rs`; `cert_inlet.rs:511-752` | `HEAD:node/cert_inlet.rs:117-124` | `CertInlet::ingest` больше не возвращает `Err` ни на одном пути (в теле нет ни `?` по fallible-вызову, ни `bail!`, ни `return Err`), но обе точки вызова сохранили `if let Err(e) = … { error!("cert-inlet fatal (committee read)"); break; }` и тип `eyre::Result<()>`. Мёртвая фатальная ветка с сообщением, которое теперь не может быть напечатано, плюс вестигиальный тип возврата | Проверил весь диапазон `:511-752` построчно на `?`/`Err`; единственный `?`-подобный вызов — `self.randomness.ensure_key(...).await` (`:597`), он возвращает `bool`. Проверил, не остался ли `Err` в `record_data_fault` — она `async fn(&mut self)` без Result | высокая |
| B2-05 | MODERATE | `testbed/stand.rs:2036-2062` | `HEAD:testbed/stand.rs:2037-2060` | Ассерт стенда ОСЛАБЛЕН, а не переадресован. HEAD: `.expect("the cold-start committee reads at the anchor")` + `assert!(!snap.validators.is_empty(), …)` — два жёстких утверждения. Дерево: только `assert!(e.is_transient(), …)` на ошибке. `NotReadable` транзиентен по построению (`committee/mod.rs:264`), а собственный комментарий кода (`stand.rs:2046-2050`) прямо говорит, что якорь на этапе сборки = 0, то есть чтение штатно ПАДАЕТ транзиентно ⇒ ассерт проходит вхолостую. Постановка Б2 запрещала менять ассерты старых тестов | Проверял, не остаётся ли ассерт живым: `OutOfWindow` снизу (`epoch < lo`) даёт `is_transient() == false` (`committee/mod.rs:265`), так что буквально мёртвым он не стал. Но случай, который он раньше пинал — «пустой комитет у стартовой эпохи на якоре» — теперь `NotReadable` = транзиент = принимается | высокая |
| B2-06 | MINOR | `committee/mod.rs:453-494`; `node/dpos.rs:1410`/`:1800`; `dpos.rs:3071`/`:3282`; `testbed/stand.rs:1846`/`:1971` | новое | `BeaconSlot` — `OnceLock`, заполняется `let _ = …set(…)` (ошибка второго `set` отбрасывается). Сегодня безопасно: бикон в процессе не пересобирается. Но если заявленный in-process follower→signer переключатель когда-нибудь пересоберёт бикон, слот удержит МЁРТВЫЙ `Weak`, `upgrade()` навсегда `None`, и каждая новая эпоха останется БЕЗ СХЕМЫ ВООБЩЕ (не vote-only) — marshal тихо перестанет проверять сертификаты новых эпох | Пытался найти существующий путь пересборки: `run_node_stack` (`node/dpos.rs:489-509`) ветвится один раз; `launch`/`launch_follower` по одному вызову; `dpos.rs:959-967` документирует плоскость как «built ONCE per process». Пути нет — поэтому MINOR, а не выше | высокая |
| B2-07 | MINOR | `cert_inlet.rs:43-51` | `HEAD:cert_inlet.rs:43-60` | Док утверждает, что `probe_inconsistency` живёт в модуле «как `Read(transient)`». Это неверно: `executed_state_hash` отдаёт `ReadError::Backend` для header-index-несогласованности на материализованной высоте (`executed.rs:64-68`, док `:53-56`), `is_transient()` там `false`, и модуль печатает `error!` + тикает `dpos_committee_read_permanent_total{reason="anchor_fault"}` (`store.rs:192-207`, вызов `:476`). Комментарий занижает наблюдаемость — то есть ошибается в ту сторону, которая скрыла бы реальную потерю, если бы она была | Проверил классификацию: `classify()` (`executed.rs:76-80`) уходит в `Backend` для всего, что `classify_transient_provider_error` не узнаёт, а ветка `Ok(None)` на материализованной высоте даёт `Backend` напрямую | высокая |
| B2-08 | MINOR | `epoch_manager.rs:1870-1878` | `HEAD:outer.rs:1183-1207` | `register_span` ломает цикл на `scheme(epoch).is_none()`, склеивая «запись не читается» и «verifier не построился» (пустой слот бикона). HEAD'овский аналог на неудаче ПОСТРОЕНИЯ схемы цикл не ломал — шёл дальше без регистрации этой эпохи | Проверял достижимость: слот бикона заполняется до запуска epoch-manager'а на всех трёх путях (§0.4), так что в продакшне вторая причина не наступает. Поэтому MINOR — но у вызывающего два разных факта теперь неразличимы | высокая |
| B2-09 | MINOR | `committee/store.rs:172-175`, `:361`, `:612-616` | `HEAD:outer.rs:373-375` | Ретенция переехала со СЧЁТА регистраций (прун только внутри `register`) на пол окна от ЯКОРЯ (прун на каждом `anchor_advanced`). Обычно шире (11 записей против 8), но узел, который долго ничего не регистрировал, на HEAD держал старые записи неограниченно, а теперь теряет всё ниже `epoch(anchor)−8` | Искал регрессию сверху: её нет — HEAD регистрировал из чтений на `fin−K`, модуль читает на `ordering_finalized`, то есть на K блоков выше. Форма пола предписана проектом §5.1, так что это не отклонение, а следствие, которое стоит записать | высокая |
| B2-10 | MINOR | `committee/tests.rs:1352-1394`, `:1409-1443`; `store.rs:529-541` | — | Перенесённые тесты пинуют ТРИ отказа `upgrade_scheme` из четырёх. Четвёртый — «нет записи комитета» — единственный, который в этом заходе НОВЫЙ, и его не проверяет ни один тест | Искал покрытие в `epoch_manager.rs` тестах и в `beacon/surface.rs:1529`: там `SchemeCommittee`, чей `upgrade_scheme` всегда `true` (`committee/mod.rs:674-677`), то есть путь `CommitteeStore` не исполняется | высокая |
| B2-11 | MINOR | `executor.rs:11833-11843`, `:12006-12016`; `beacon/surface.rs:1484-1536`, `:2562-2572`; `cold_start_jump.rs:1260-1271`; `fault.rs:18` | — | Четыре файла вне списка постановки. `executor.rs` был «кроме нуля правок», `beacon/**` — запрещён «целиком»; `beacon/surface.rs:2562-2572` — комментарий ВНУТРИ продакшн-кода `impl Randomness for LiveBeacon`. Все правки тестовые/доковые, механически неизбежные, журналом признаны как Д-31 — но правило нарушено | Проверил каждый ханк на изменение продакшн-поведения: изменений кода вне `#[cfg(test)]` нет ни в одном из четырёх | высокая |
| B2-12 | NIT | `scheme.rs:57` | `HEAD:scheme.rs:57` | `pub fn soft_enter_verifier` осталась без единого продакшн-вызывающего: `grep` даёт только `epoch_manager.rs:2152` и `:3056`, оба в `mod tests`. Мёртвая публичная поверхность (память проекта: «ничего не задеплоено ⇒ удалять, а не депрекейтить») | Проверил экспорт: в `lib.rs` не реэкспортируется, так что вне крейта недостижима — поэтому NIT | высокая |
| B2-13 | NIT | `.dpos-study/history/E4-1-B2.md` Д-29, Д-26, Д-33 | — | Три якоря журнала указывают мимо: Д-29 `epoch_manager.rs:832` (реально `:803`), Д-26 `store.rs:349` (реально `:374`), Д-33 `dpos.rs:2288` (форвардер `:2597-2598`). В §2 те же места названы верно | Перечитал каждую строку `sed -n` — подтверждено | высокая |
| B2-14 | NIT | `testbed/stand.rs:1652-1656` | — | Новые комментарии несут сбитые якоря: `cert_inlet.rs:896-898` (тее-запись реально `:715`) и `cert_inlet.rs:343-347` (поле `live_height` реально `:264`). Комментарий написан ЭТИМ заходом, так что дрейф внесён им | Проверил обе цели `sed -n` — по указанным номерам другое | высокая |
| B2-15 | MINOR | `committee/store.rs:589-597`; `outer.rs:914-922` | `HEAD:outer.rs:411-413` | `latest_scheme()` = самая высокая эпоха КАРТЫ со схемой, а карта держит до `epoch(anchor)+2`. Значит `peers_for_finalization` может адресовать fetch финализаций участникам эпохи, которая закоммичена, но ещё не живая и часть которой может быть вне `track`-окна `C[E−1] ∪ C[E] ∪ C[E+1]`. Вдобавок в карту теперь попадает ЛЮБАЯ прочитанная эпоха (`is_member`, `changed`, `scheme` inlet'а), а не только зарегистрированная | Проверял, не регрессия ли: на HEAD `soft_enter_span` регистрировал контигуозный закоммиченный префикс на `fin−K`, то есть тоже доходил до `+2`. Класс тот же, потолок выше на ~K блоков и шире по источникам — отсюда MINOR, а не SERIOUS | средняя |

BLOCKER'ов нет. Ни одна находка не является непредписанной регрессией поведения на
пути голосования, финализации, границы или доставки сертификатов: B2-01 и B2-02
задевают границу и доставку, но первая — предписанное проектом требование, которое
заход осознанно переопределил и записал как Д-27, а вторая имеет вторичный путь
ремонта (`register_span`).

---

## §2. Поведение по ханкам против HEAD

**`committee/mod.rs`** (+265/−2). Трейт `Committee` получил четыре метода
(`scheme` `:308`, `upgrade_scheme` `:328`, `verifier_epochs` `:338`, `latest_scheme` `:343`)
— поверхность, а не поведение. `CommitteeRecord::snapshot_view()` (`:174-195`) —
новая проекция; поведение нового кода разобрано в §0.10. Новые типы `EpochVerifier`
(`:451`), `BeaconSlot` (`:469`), `epoch_verifier` (`:480-494`) — единственный
продюсер verify-схем; поведенчески заменяет четыре HEAD-продюсера, один из которых
(`cold_start_register`) хардкодил `oracle: None`. `#[cfg(test)] mod testing` (`:607-704`)
— только тесты.

**`committee/store.rs`** (+~130). `EpochEntry` получил `scheme: Option<Arc<BlsScheme>>`.
`install` строит схему ВНЕ лока и кладёт в тот же слот (`:332`, `:362-368`) — записи без
схемы наблюдаемо не существует, кроме окна «бикон младше стора». `resolve_scheme`
(`:374-391`) — отложенный повтор через `get_or_insert`, не перезаписывает. `scheme`
(`:517-524`) — ленивое чтение (см. B2-03). `upgrade_scheme` (`:526-575`) — четыре отказа
(§0.3). `verifier_epochs` (`:577-587`) — семантика та же, что у HEAD, но фильтр теперь
дополнительно требует `scheme.is_some()`. `latest_scheme` (`:589-597`) — `rev().find_map`
вместо `values().next_back()`: теперь пропускает записи БЕЗ схемы, что HEAD не умел
(там записей без схемы не бывало). `anchor_advanced` (`:603-641`) — публикация всегда
(Д-28, §0.6).

**`outer.rs`** (−410 нетто). `EpochSchemeProvider` из владельца карты стал видом
(`:240-276`); `register`, `cold_start_register`, поле `scheme_provider` у `OuterEngine`,
`SoftEnterCommittees`, замыкание `register_scheme`, блок `soft_enter_span` удалены.
`boundary_tx`/`boundary_sender` — `Epoch` вместо `(Epoch, ValidatorSetSnapshot)`
(`:579`, `:1106-1112`). Модуль `scheme_provider_tests` удалён (оба теста перенесены).
Поведенчески — см. B2-03 (`scoped`) и B2-09 (ретенция).

**`epoch_manager.rs`** (+~170 нетто). `Config` потерял `register_scheme` и
`soft_enter_span`, получил `committee` (`:590`). `boundary_rx: Receiver<Epoch>` (`:464`),
`latest_live: Option<Epoch>` (`:517`). `reconcile_roles` (`:1056`) читает запись первым
делом и на ЛЮБОЙ ошибке возвращается ничего не сделав (`:1073-1083`) — это новый исход,
которого на HEAD не было (снимок всегда приходил с ребром). `is_member` — через
`record.participants.position` (`:1140-1142`) вместо линейного скана. Новая ветка
`select!` `:803-805` (Д-29). `WeightedVrf` поднят (`:1298-1318`, §0.12), `upgrade_scheme`
(`:1367-1376`), `soft_enter(epoch)` (`:1401-1415`) стал чистым чтением.
`register_span` (`:1870-1878`) заменил колбэк.

**`engine.rs`** (+18/−13). `fallback_seed: [u8;32]` → `elector: WeightedVrf` (`:86`);
`(cfg.register_scheme)(…)` снят (`HEAD:engine.rs:225`); `simplex::Config.elector =
cfg.elector` (`:289`) вместо `try_new(…)?`. Поведение: движок больше не регистрирует
ничего и не может упасть на электоре.

**`cert_inlet.rs`** (−600 нетто). `CommitteeSource` с одним методом (`:114-127`);
`RethCommitteeSource` без `finalized_hash`; `CertInlet<E, M>` держит
`committee: Arc<dyn Committee>` (`:301`). Горячий путь `:619-639` — один дефер вместо
трёх исходов; evict после verify-FAIL снят (`:654-661` объясняет, почему запись
модуля не бывает stale); ретенция `{prev,cur}` снята (`:700-706`). Метрика сохранила
форму, `reason` схлопнулся до одного значения (`:52-59`). См. §0.8 и B2-04, B2-07.

**`node/cert_inlet.rs`** (−43 нетто). `committee_source` удалён, `spawn_cert_inlet`
перестал быть генериком. Фатальный цикл (`:87-104`) НЕ тронут — отсюда B2-04.

**`node/dpos.rs`** (+21/−41). `inlet_setup` сжался до `(ctx, urls)` (`:733`), inlet
берёт `plane.committee.clone()` (`:783`). `beacon_slot` (`:1410`) + четвёртый аргумент
стора (`:1423`); заполнение слота (`:1800`). ET плоскости, коалесценция поллера и
`on_finalized` НЕ тронуты — Б2.3 здесь не делался.

**`dpos.rs` (consensus)** (−310 нетто). Валидатор: `initial_snapshot`/`bail!` →
чтение модуля + `warn!` (`:1991-2013`, B2-01); `cold_start_register` и построение
`initial_scheme` сняты; форвардер шлёт `Epoch` (`:2597-2598`). Follower: `beacon_slot`
(`:3071`) + `epoch_verifier` (`:3084`); заполнение (`:3282`); cold-start —
чтение модуля + `warn!` (`:3408-3421`); `FollowerCommitteeAt` стал
`Arc<dyn Fn(u64) -> bool>` (`:1439`) — «читается ли», и это же регистрация;
inlet-источник с трёхветочной пробой удалён целиком, inlet берёт модуль (`:3544`).
`RethCommitteeSource::new` потерял третий аргумент в шести местах. Признак,
который исчез вместе с пробой, — `DEFER_PROBE_INCONSISTENCY`; он покрыт модулем
(см. B2-07).

**`reader/epoch_transition.rs`** (−179). Только удаление `soft_enter_span`, двух его
тестов и осиротевшего фейка `PrefixReader`. Поведения оставшегося кода не менялось.

**`testbed/stand.rs`** (+~50/−90). Стор получил `epoch_verifier` (`:1855`), слот
заполняется (`:1971`); `soft_enter`-замыкание удалено; пара `el_finalized`/`finalized_hash`
удалена; cold-start-ассерт ослаблен (B2-05); форвардер шлёт `Epoch` (`:2078`), снимок
по-прежнему записывается в `EtBoundary`.

**`testbed/fakes.rs`** (−12 нетто). `JumpCommittees` без `finalized_hash` и без
`scheme_at_finalized_tip`.

**`executor.rs`, `cold_start_jump.rs`, `fault.rs`, `beacon/surface.rs`,
`tests/slasher_integration.rs`, `lib.rs`** — см. §0.16 и §3; продакшн-поведения не
меняют (`lib.rs` — только реэкспорты).

---

## §3. Граница — таблица имён `pub`

| символ | файл | HEAD → дерево |
|---|---|---|
| `CommitteeRecord::snapshot_view` | `committee/mod.rs:174` | — → **pub fn** |
| `Committee::scheme` | `committee/mod.rs:308` | — → **метод трейта** |
| `Committee::upgrade_scheme` | `committee/mod.rs:328` | — → **метод трейта** |
| `Committee::verifier_epochs` | `committee/mod.rs:338` | — → **метод трейта** |
| `Committee::latest_scheme` | `committee/mod.rs:343` | — → **метод трейта** |
| `EpochVerifier` | `committee/mod.rs:451` | — → **pub type** |
| `BeaconSlot` | `committee/mod.rs:469` | — → **pub type** |
| `epoch_verifier` | `committee/mod.rs:480` | — → **pub fn** |
| `CommitteeStore::new` | `committee/store.rs:126` | 3 аргумента → **4** (`EpochVerifier`) |
| `SchemeCommittee` | `committee/mod.rs:630` | — → `pub(crate)` внутри `#[cfg(test)] pub(crate) mod testing` (`:608-609`) — **в релизе отсутствует** |
| `EpochSchemeProvider::new` | `outer.rs:249` | `()` → **`Arc<dyn Committee>`** |
| `EpochSchemeProvider::register` | `HEAD:outer.rs:332` | pub fn → **удалён** |
| `OuterEngine::cold_start_register` | `HEAD:outer.rs:1326` | pub fn → **удалён** |
| `SoftEnterCommittees` | `HEAD:outer.rs:199` | pub type → **удалён** (и из `lib.rs:101`) |
| `OuterBuilder.soft_enter_committees` | `HEAD:outer.rs:546` | pub-поле → **удалено** |
| `OuterEngine::boundary_sender` | `outer.rs:1106` | `Sender<(Epoch, ValidatorSetSnapshot)>` → **`Sender<Epoch>`** |
| `CommitteeSource::scheme_at_finalized_tip` | `HEAD:cert_inlet.rs:245` | метод трейта → **удалён** (трейт остался с одним `scheme_at`) |
| `RethCommitteeSource::new` | `cert_inlet.rs:143` | 3 аргумента → **2** (без `finalized_hash`) |
| `CertInlet::new` | `cert_inlet.rs:405` | `(marshal, C: CommitteeSource, ctx)` → **`(marshal, Arc<dyn Committee>, ctx)`**; тип стал `CertInlet<E, M>` (два параметра вместо трёх) |
| `DEFER_STATE_NOT_MATERIALIZED` | `HEAD:cert_inlet.rs:~62` | pub const → **удалён** |
| `DEFER_PROBE_INCONSISTENCY` | `HEAD:cert_inlet.rs:~70` | pub const → **удалён** |
| `EpochTransition::soft_enter_span` | `HEAD:reader/epoch_transition.rs:705` | pub async fn → **удалён** |
| `EpochEngineConfig.fallback_seed` | `HEAD:engine.rs:80` | pub-поле → **`elector: WeightedVrf`** (`engine.rs:86`) |
| `EpochEngineConfig.register_scheme` | `HEAD:engine.rs:~91` | pub-поле → **удалено** |
| `epoch_manager::Config.register_scheme` | `HEAD:epoch_manager.rs:576` | pub-поле → **удалено** |
| `epoch_manager::Config.soft_enter_span` | `HEAD:epoch_manager.rs:594` | pub-поле → **удалено** |
| `epoch_manager::Config.committee` | `epoch_manager.rs:590` | — → **pub-поле** |
| `epoch_manager::Actor::new` | `epoch_manager.rs:620` | возвращает `Sender<(Epoch, snap)>` → **`Sender<Epoch>`** |
| `lib.rs` реэкспорты | `lib.rs:84-86`, `:101` | `+epoch_verifier, +BeaconSlot, +EpochVerifier`; `−SoftEnterCommittees` |

Утечек в релизную поверхность нет: всё новое, что не должно быть публичным
(`SchemeCommittee`), сидит под `#[cfg(test)]`.

---

## §4. Ворота — вывод verbatim

~~~
$ CARGO_BUILD_JOBS=6 cargo test -p fluentbase-consensus --lib
test result: ok. 665 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 23.56s

$ CARGO_BUILD_JOBS=6 cargo test -p fluentbase-consensus --features dpos-devnet-byzantine --lib testbed::
test result: ok. 40 passed; 0 failed; 0 ignored; 0 measured; 633 filtered out; finished in 32.63s

$ CARGO_BUILD_JOBS=6 cargo test -p fluentbase-node --lib
test result: ok. 57 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 56.71s

$ CARGO_BUILD_JOBS=6 cargo test -p fluentbase-staking-reader
test result: ok. 58 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
test result: ok. 0 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s

$ CARGO_BUILD_JOBS=6 cargo test -p fluentbase-consensus --test slasher_integration
test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s

$ CARGO_BUILD_JOBS=6 cargo clippy -p fluentbase-consensus -p fluentbase-node --all-targets
warning: large size difference between variants
    --> crates/node/src/dpos.rs:1866:1
warning: `fluentbase-node` (lib) generated 1 warning
warning: `fluentbase-node` (lib test) generated 1 warning (1 duplicate)

$ CARGO_BUILD_JOBS=6 cargo clippy -p fluentbase-consensus -p fluentbase-node --all-targets --features dpos-devnet-byzantine
warning: large size difference between variants
warning: `fluentbase-node` (lib) generated 1 warning
warning: `fluentbase-node` (lib test) generated 1 warning (1 duplicate)

$ cargo fmt --check | grep -c "Diff in"
0

$ CARGO_BUILD_JOBS=6 cargo doc -p fluentbase-consensus --no-deps 2>&1 | grep -c "unresolved link"
9
~~~

Отдельной строкой, потому что это ловушка для следующего прогона:
`cargo clippy … --features fluentbase-consensus/dpos-devnet-byzantine` (форма «через
крейт») ПАДАЕТ с `error[E0063]: missing field 'byzantine' in initializer of
fluentbase_consensus::DposLayerConfig<_, _, _, _>` — фича обязана включаться как
`--features dpos-devnet-byzantine`, чтобы `fluentbase-node` включил свою
(`crates/node/Cargo.toml:137`). Это свойство базы, не захода.

---

## §5. Оставить как есть

1. **`EpochEntry { record, scheme }` и один `EpochVerifier`.** Форма ровно та, что
   предписывает §5.1 «Кэш и ретенция», и она закрывает класс «две авторитетные
   таблицы» структурно, а не дисциплиной: слот один, второго писателя нет, и
   `upgrade_scheme` не умеет СОЗДАТЬ запись, только поднять существующую
   (`store.rs:529-541`).
2. **Слабая ссылка на бикон (Д-25).** Сильная замыкает цикл
   `стор → verifier → бикон → фасад → стор`; выбор правильный, и `epoch_verifier`
   структурно не способен выдать oracle-less схему (`committee/mod.rs:486-492`), что
   снимает целый класс запинов на vote-only.
3. **`get_or_insert` в `resolve_scheme` вместо перезаписи** (`store.rs:390`). Это и
   есть то, что делает гонку «отложенный verifier против signer» безвредной без
   дополнительной гвардии. Не трогать.
4. **Публикация пробуждения на КАЖДОМ `anchor_advanced` (Д-28).** Класс «якорь есть,
   состояние не исполнено» иначе повтора не имеет вовсе; монотонность значения
   удержана вручную (`store.rs:637-640`). Цена — один реконсиль в секунду без EVM.
5. **Подъём `WeightedVrf` до `upgrade_scheme` (Д-30).** Строго лучше HEAD: отказ
   электора больше не оставляет в карте signer-схему без движка.
6. **Сравнение комитета в `upgrade_scheme` с ЗАПИСЬЮ, а не с ранее установленной
   схемой** (`store.rs:542`). Сильнее HEAD и делает отказ «чужой комитет»
   структурно недостижимым, а не просто маловероятным.
7. **Отказ от `debug_assert` на occupied-ветке `install`** (`store.rs:307-317`) —
   форк контракта отрабатывается одинаково в debug и release.
8. **Решение НЕ делать Б2.3 в этом заходе.** Обе причины (§0.9) проверены мной по
   коду и настоящие; слияние без правки `reader/epoch_transition.rs` обменяло бы
   дефект peer-set'а на потерю входа в эпоху.
9. **Форма журнала.** §8 «Где проверка слабее всего» перечисляет ровно те семь мест,
   которые я и нашёл бы сам; это то, за что ревью не приходится платить.

---

## §6. Замечено вне рамок Б2

**Для Б2.3 / Б3:**
- Плоскостной ET пропускает границы на догоне (коалесценция `node/dpos.rs:1494` против
  точечной детекции `reader/epoch_transition.rs:495`, `:546`) — ДЕФЕКТ HEAD, сегодня
  ограниченный peer-set'ом, потому что этот инстанс строится с `None` вместо
  `bridge_tx` (`node/dpos.rs:1358`). Становится дефектом входа в эпоху ровно в тот
  момент, когда Б2.3 отдаст ему `bridge_tx`.
- `EpochTransition::cold_start` присваивает `anchor_height` безусловно
  (`reader/epoch_transition.rs:694`), и док `raise_anchor_height` (`:311-318`) прямо
  называет проводку к многократно cold-start'ящемуся инстансу дефектом. Тот же док
  несёт СБИТЫЕ якоря (`node/src/dpos.rs:1298-1315` — поллер реально `:1476-1560`;
  `consensus/dpos.rs:2162`) — дрейф из базы, не из захода.
- Стенд не моделирует пол якоря по тегу reth (`testbed/stand.rs:1638-1650` это
  признаёт): `StandAnchor::height()` = `chain.tip()`, продакшн = `max(курсор, тег)`
  (`store.rs:707-715`). Именно поэтому «якорь есть, состояния нет» достижимо только
  на стенде — и именно поэтому Д-28 нашли.
- `FakeStaking` по ветке хэша, модель кольца весов (`weights: None` для
  `epoch ≤ current − 14`), тумбстоуны и три стенд-теста + e2e-пин
  `commit_height(E) = start(E−2)` — не тронуты, как и предписано.
- `tracked_mismatches` (`testbed/stand.rs:433`, `:835`, `:1269`) слияние ET не осиротит:
  он считает расхождения МЕЖДУ УЗЛАМИ, а на стенде ET и так один на узел. Проверил
  чтением — утверждение журнала §7.2 верное.

**Для 4.2:**
- `CommitteeSource` (`cert_inlet.rs:114-127`) остался с одним методом `scheme_at` и
  одним продакшн-потребителем — `cold_start_jump` (через `dpos.rs:2386`, `:2468`).
  `RethCommitteeSource` и `CommitteeSource` по-прежнему реэкспортируются из `lib.rs:75`.
- `LiveFrontierTee.live_height` write-only ВЕЗДЕ: последний читатель (follower
  inlet-источник) удалён. На стенде тоже — `testbed/stand.rs:1662` пишет через
  `FrontierProbeFn` (`:1682-1694`), читателей нет. Снимается в 4.2 вместе с
  `upstream_frontier`.
- `is_live_epoch` (`epoch_manager.rs:1046-1048`) всё ещё `epoch >= highest_observed_epoch`
  — замена на правило по tip'у (§5.2) остаётся за 4.2, и от неё зависит острота B2-02.

**Для 4.3:**
- Окно чтения модуля `[epoch(anchor)−8, epoch(anchor)+2]` — готовая маска для
  `Ingress`; `CommitteeError::OutOfWindow` уже отказывает без EVM (`store.rs:437-442`),
  то есть половина 4.3 уже стоит.

**Для Э7:**
- Голова очереди слэшера (B1-03) не тронута; транзиентных исходов не убавилось —
  `CommitteeError::is_transient` теперь направленный по стороне окна
  (`committee/mod.rs:262-268`), что для слэшера скорее плюс, но замера нет.

**Прочее:**
- `pub fn soft_enter_verifier` (`scheme.rs:57`) осталась без продакшн-вызывающих
  (B2-12) — кандидат на удаление или на `#[cfg(test)]`.
- Постановка ревью говорит «20 файлов под `crates/`»; в дереве 19.
