# Ревью захода Б2′ (строка 4.1) — один `EpochTransition` на валидаторе

Ревьюер: Opus 5, свежий контекст. База — `HEAD 3a4a0fde`, объект — незакоммиченное
рабочее дерево по четырём файлам (`reader/epoch_transition.rs`, `dpos.rs`, `lib.rs`,
`node/dpos.rs`). Остальное в `git status` (`contracts/staking/**`,
`crates/staking-abi/src/lib.rs`, `target-contract/`, `.dpos-study/history/E4-ORCHESTRATOR.md`)
— чужое, как часть захода не читалось.

Все якоря `file:line` — по ИТОГОВОМУ дереву, кроме помеченных `HEAD:`.

---

## §0. Прямые ответы

### 1. Ворота — прогнал сам, verbatim в §4. [KNOWN]

Все зелёные, все цифры совпали с журналом: consensus lib **668/0**, стенд с фичей
**40/0**, node **57/0**, staking-reader **60/0** (58 базовых + 2 новых),
`slasher_integration` **16/0**, clippy обе формы — одна чужая `large_enum_variant` в
`fluentbase-node`, `cargo fmt --all --check` — **ноль** `Diff in` по всему воркспейсу
(то есть и чужие правки в `contracts/` fmt-чистые), `unresolved link` = **8**.
Чужой `crates/staking-abi/src/lib.rs` сборку не ронял — ни один из девяти прогонов на
нём не упал.

Одна оговорка по методике: моя первая форма clippy с фичей
(`--features fluentbase-consensus/dpos-devnet-byzantine`) упала `E0063 missing field
byzantine` в `node/dpos.rs:2146` — это дефект МОЕЙ команды (фича включена для
consensus, но не для node), а не захода; форма из журнала
(`--features dpos-devnet-byzantine`) зелёная. Записано, чтобы цифра «07 exit=101» в
моих логах никого не сбила.

### 2. Один инстанс — да, ровно один. [KNOWN]

`git grep -n "EpochTransition::new" -- '*.rs'`: в продакшне **одно** вхождение —
`crates/node/src/dpos.rs:1377`. Остальные: `testbed/stand.rs:1563` (стенд, за фичей) и
16 в `#[cfg(test)]` самого `epoch_transition.rs`. На `HEAD` продакшн-вхождений было
два (`HEAD:crates/dpos/consensus/src/dpos.rs:2058`, `HEAD:crates/node/src/dpos.rs:1355`).

Тот же `Arc` доходит до всех трёх потребителей — проследил по клонам, не по журналу:
`et_arc` создаётся в `node/dpos.rs:1399`, клонируется в поллер (`:1484`) и **уезжает**
в `PlaneEpochTransition { transition: et_arc, .. }` (`:1888`) → `BeaconPlane.epoch_transition`
(`:1038`) → `launch_dpos_layer(… plane.epoch_transition …)` (`:762`, параметр `:1978`) →
`DposLayer::launch(…, epoch_transition, …)` (`:2213`) → деструктуризация
`dpos.rs:2083-2086` → `et_arc`, от которого клоны идут в `cold_start` (`:2096`),
`enter_boundary`/`on_finalized` (`:2151` `et_for_hook`, `:2252`) и в
`read_floor_boundary`/`raise_anchor_height` (`:2352-2357`). На `HEAD` `et_arc` в
`node/dpos.rs` клонировался ТОЛЬКО в поллер (`HEAD:node/dpos.rs:1377`, `:1454`) — это
важно для находки B2p-01.

`bridge_tx` — `Some(bridge_tx)` (`node/dpos.rs:1381`), канал рождён на `:1371-1372`,
ёмкость 64 (как была у слоя). `bridge_rx` едет вниз в той же структуре и сливается
ровно одним форвардером `epoch_bridge` (`dpos.rs:2644-2656`) в
`outer.boundary_sender()` (`:2634`). Второго форвардера нет.

### 3. Драйвер границ и шкала. [KNOWN, кроме отмеченного]

- Цепочка хука по коду: `FluentApp::report` зовёт `(self.boundary_hook)(block.clone())`
  на КАЖДОМ `Update::Block` (`application.rs:1027-1028`) → `dpos.rs:2341-2344`
  (`enter(block.height)`) → `enter_boundary` (`:2205`) → `on_finalized(number)`
  (`:2251-2253`). `block` — `OrderBlock`, высота ORDERING-цепи. По одной высоте.
- Контигуозность `Update::Block` — подтверждена НЕ рассуждением: `.claude/COMMONWARE_INTERNALS.md:197-198`
  («`Block(B, A)` — ordered, contiguous, at-least-once»; `try_dispatch_blocks` ассертит
  `block.height() == next_height`, `marshal/core/actor.rs:1248-1276`; «A gap stops
  dispatch — no skipping»). Разрыва вне прыжка нет; в checkout commonware за этим не
  ходил — док покрывает вопрос с якорями. Дубликат после краха возможен
  (`at-least-once`), но `apply_at` идемпотентен по эпохе (`:544` write-once гейт).
- Шов приземления — `executor.rs:2635` `(self.boundary_read_floor)(floor)` затем
  `:2636-2643` `enter(terminal_at_or_below(landing))`. Это единственный источник высот,
  которые хук пропускает **в стационаре**. Для ХОЛОДНОГО прыжка (до движка, `dpos.rs:1836-1866`)
  такого шва нет — его роль играл `cold_start` слоя, и именно это ломает B2p-02.
- Поллер больше НЕ зовёт `on_finalized`: `grep -n "on_finalized" crates/node/src/dpos.rs`
  — пусто (на `HEAD` было `:1538`). Ветка `frozen_before` теперь даёт `None`
  (`node/dpos.rs:1568-1570`).
- Что поллер ещё делает: (а) `dkg_height` = `fin + K` (`:1546-1550`); (б) `cold_start` до
  заморозки + публикация геометрии + `committee_wake.anchor_advanced()` (`:1571-1587`);
  (в) тумбстоуны (`:1638-1700`).
- **Тумбстоуны по `epoch_at(fin)` (`node/dpos.rs:1638`) — НЕ регресс**: на `HEAD` ровно
  та же строка (`HEAD:node/dpos.rs:1607`, `let epoch = et.lock().await.epoch_at(fin);`).
  Заход её не трогал. То, что в окне шириной K после границы это ПРЕДЫДУЩАЯ эпоха, —
  поведение `HEAD`; после слияния это просто единственное место, где плоскость ещё
  считает эпоху сама. В рамки захода не входит, в §6.

### 4. Два `cold_start` на одном инстансе. [KNOWN]

- **По полу** — идемпотентно: `cold_start` теперь `raise_anchor_height(head_number)`
  (`epoch_transition.rs:700`), обе точки записи берут `max` (`:322`). Прогнал тест сам
  (§4), плюс мутацию.
- **По эпохе** — идемпотентно, но НЕ безобидно: гейт бутстрапа
  `if self.last_tracked_epoch.is_none()` (`epoch_transition.rs:513`) — write-once.
  Побеждает ПЕРВЫЙ cold start. Кто первый — поллер (EL-шкала, `fin`) или слой
  (ordering-шкала, `latest_finalized`, ПОСЛЕ прыжка) — решает гонка. Это находка
  **B2p-02**.
- **По мосту** — дважды не уйдёт: второй `cold_start` попадает в `apply_at` мимо
  бутстрап-ветки, а граничная ветка требует `is_boundary(number)`; дедупликации на
  стороне `epoch_manager` нет и не нужно — `boundary_rx` просто зовёт
  `reconcile_roles(epoch)` на каждое сообщение (`epoch_manager.rs:783-790`).
- **Гонка «поллер трекнул ДО `DposLayer::launch`»** — проследил по коду: `cold_start`
  слоя уходит в граничную ветку `apply_at:543-596`. Да, стартовая парковка там
  возможна (`:584` `self.pending_boundary = Some(number)`, без `debug_assert`) — это
  ТРЕТИЙ парковщик в смысле комментария `:398-417` («only the in-order delivery path
  may park»). Столкнуться с парковкой хука она может только пережив целую эпоху
  (парковка слоя — на `latest_finalized`, первая доставка хука — `latest_finalized+1`,
  и replay идёт ПЕРЕД новой высотой, `:361-389`), то есть нужно, чтобы исполненный
  хвост целый интервал не дотягивал до `B − K`. Наблюдаемый исход в релизе, где
  `debug_assert` молчит: клоббер слота ⇒ ровно один вход в эпоху теряется навсегда
  (гейт `last_tracked_epoch < Some(next)` пропустит СЛЕДУЮЩУЮ границу, `:544`), без
  единой строки выше `debug`. Находка **B2p-03**.

### 5. `raise_anchor_height` ∧ `cold_start` через `max`. [KNOWN]

Пути, где пол ОБЯЗАН опуститься, **нет**:
- перезапуск процесса — новый `EpochTransition`, `anchor_height: None`
  (`epoch_transition.rs:221`); понижать нечего;
- follower-переключение — у `--cert-follow` follower'а `EpochTransition` нет вовсе
  (см. п. 7), а роль Verifier внутри процесса валидатора движок/хук не сносит
  (`OuterEngine` живёт до конца процесса; «promotion» в `epoch_manager` — это
  ПОЭПОШНЫЙ движок, `epoch_manager.rs:685-688`, а не перезапуск `DposLayer::launch`);
- стенд — `resume_from` лишь задаёт якорь холодного старта
  (`testbed/stand.rs:1584`), ET строится заново на каждый узел (`:1563`).

Тест `cold_start_after_a_raise_does_not_lower_the_floor` (`epoch_transition.rs:2290`)
пинает ровно шов: `cold_start(500)` → пол 500, `raise_anchor_height(999_997)` →
`cold_start(400)` не опускает → `cold_start(1_000_000)` двигает вперёд. Что он НЕ
пинает: что пол, поставленный `cold_start`'ом слоя, — это ORDERING-высота
(`latest_finalized`), а поднятый швом — RESULT-высота (`landing − K`,
`executor.rs:2635` + `:2632-2634`); `max` между двумя разными шкалами — это поведение
`HEAD` (`HEAD:dpos.rs:2069` присваивал ту же ordering-высоту), заход его не менял, но
тест этого различия не касается.

**Мутацию повторил САМ.** Мутация (журнал §4.2): в `apply_at` заменить
`let is_boundary = is_epoch_boundary(number, activation, interval);`
(`epoch_transition.rs:495`) на `… || self.last_tracked_epoch < Some(epoch_e)`.
Файл предварительно сохранён побайтово и восстановлен после
(`md5 54e1bc9d7c05f930bb3b367d249c87b8` до и после; `cargo test -p
fluentbase-staking-reader --lib` = 60/0 после восстановления). Вывод:

~~~
thread 'epoch_transition::tests::a_coalesced_driver_skips_the_boundary_a_stepping_one_enters' (3820257) panicked at crates/dpos/staking-reader/src/epoch_transition.rs:2385:13:
assertion `left == right` failed: the coalesced driver tracked only the cold-start epoch — 2 is lost
  left: [2]
 right: [1]

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 59 filtered out; finished in 0.00s
~~~

Совпадает с журналом (строка 2385 против 2379 — моя мутация заняла две строки вместо
одной). Тест не вакуумен.

### 6. `track` — да, только через `track_and_trigger`. [KNOWN]

Единственный продакшн-вызов `PeerSetSink::track` — `epoch_transition.rs:655`
(`self.sink.track(epoch, Set::from_iter_dedup(tracked)).await`), внутри
`track_and_trigger` (`:615-682`); в неё ведут ровно два места: бутстрап холодного
старта (`:526-529`) и граничная ветка (`:587`). Разделять «границу» и «track» нечего —
вывод Д-44 верен.

Потерян ли `track` после снятия поллерного `on_finalized`? В стационаре — нет:
граничное ребро хука надмножество (каждый ordering-финализированный блок ⊇ каждая
граница), и по новому тесту коалесцирующий поллер границу всё равно чаще ТЕРЯЛ, чем
видел. Отдельно проверил «поллер видел границу ДО того, как хук существует»: окно от
постройки плоскости до `outer.start()`. Всё, что ниже `latest_finalized`, покрыто
`cold_start`'ом слоя (включая случай «якорь ровно на границе» ⇒ `cold_epoch = epoch_e + 1`,
`:519`); всё, что выше, хук доставит сам, начиная с `latest_finalized + 1`. Так что
ни один `track` не потерян — **но** в этом окне (а при холодном прыжке оно может
длиться часами, `EL_SYNC_BACKSTOP_CEILING`) `oracle.track` теперь не обновляется
вообще, тогда как на `HEAD` поллер его иногда (ненадёжно) обновлял. Находка
**B2p-04**.

### 7. Follower — подтверждаю, ET нет и ничего не изменилось. [KNOWN]

`run_node_stack` ветвится один раз на `is_validator` (`node/dpos.rs:489-507`);
follower идёт в `cert_follow::launch_follower_overlay` (`:500`) →
`DposLayer::launch_follower` (`node/cert_follow/mod.rs:252`). `build_beacon_plane`
зовётся ровно из одного места — `node/dpos.rs:737`, валидаторская ветка. Границы
follower'у даёт `boundary_hook` → `follower_finalized`/`Notify` (`dpos.rs:3265-3272`) →
`enter_finalized_epoch` → `follower_boundary_tx` (`:3500`). `track` он не делает и на
`HEAD` не делал (`EpochTransition::new` в `HEAD` тоже отсутствовал на follower-пути).
Диффом этот участок не тронут. Утверждение постановки Б2′.2 «плоскостной ET у
follower'а остаётся ЕДИНСТВЕННЫМ» было неверно по коду — журнал §0.3 это ловит верно.

### 8. Граница типов. [KNOWN]

- `BeaconPlane<Provider, EvmConfig>` (Д-43): тип `pub(crate)` (`node/dpos.rs:941`),
  наружу не течёт; цена — три площадки. Оправдано: иначе поле не выражается.
- `DposLayer::launch` пятым параметром (Д-41): аргумент журнала проверил — у
  `DposLayerConfig<D, XC, A, U>` (`dpos.rs:914`) генериков `Provider`/`EvmConfig` нет,
  а `launch_follower` берёт ту же конфигурацию и ET не имеет. Отдельный параметр
  честнее `Option`-поля. Согласен.
- `PlaneEpochTransition` vs «трейт на три глагола»: трейт был бы ХУЖЕ. Три глагола —
  `cold_start`, `on_finalized`, `raise_anchor_height` — два из них `async`, то есть
  трейт потребовал бы боксированных фьюч, а обмен всё равно идёт через
  `Arc<Mutex<_>>`. Плюс `bridge_rx` в трейт не заворачивается — он половина канала, а
  не глагол. Структура правильная.
- Реэкспорт `PlaneEpochTransition` в `lib.rs:91` — **не используется**: единственный
  потребитель адресует его как `fluentbase_consensus::dpos::PlaneEpochTransition`
  (`node/dpos.rs:1038`, `:1887`, `:1978`), а `pub mod dpos` (`lib.rs:41`) и так открыт.
  Находка **B2p-07**.
- `#[allow(clippy::too_many_arguments)]` — оба уже стояли на `HEAD`:
  `HEAD:dpos.rs:1528` и `HEAD:node/dpos.rs:1915`. Новых `#[allow]`, `todo!`, таймеров,
  поллинга-вместо-события в диффе нет; все восемь `unwrap(` в добавленных строках — в
  двух новых тестах. Подтверждаю §0.7 журнала.

### 9. Стенд. [KNOWN]

ET один на узел (`stand.rs:1563`), драйвер — `boundary_feed`, читающий `hook_rx` и
зовущий `on_finalized(block.height)` на каждый финализированный блок (`:2128`, `:2156`).
После правки это ТОТ ЖЕ драйвер, что у продакшна. `tracked_mismatches` по-прежнему
МЕЖДУ узлами: поле документировано как «two nodes' transitions tracked DIFFERENT peer
sets for the same epoch» (`stand.rs:431-433`), снимается из общего `tracked`
(`:1269-1272`), ассерт `== 0` в `tests.rs:1779`. Стенд не менялся, 40/0.

Комментарий `stand.rs:2100-2107` («the OTHER production transition … the beacon plane's
peer-set tracker (`node/src/dpos.rs:1695`)») **протух** — второго инстанса нет, и
плоскость больше не потребляет EL-курсор для `on_finalized`. Плюс его соседние якоря
(`:1556-1560` → `consensus/src/dpos.rs:2025-2041` и `:2739-2749`; `:1579-1581` →
`:2044-2047`) указывают на посторонний код — часть дрейфа уже была на `HEAD`, часть
добавил заход. Находка **B2p-06**.

### 10. Тесты. [KNOWN]

Счёт `#[test]`/`#[tokio::test]` по четырём файлам, HEAD → дерево:
`epoch_transition.rs` 29 → **31**; `dpos.rs` 31 → 31; `lib.rs` 0 → 0;
`node/dpos.rs` 8 → 8. Два новых — оба в `epoch_transition.rs` (`:2290`, `:2329`).
Удалённых нет (в диффе по тестовым модулям только `+`).

### 11. Где журнал вводит в заблуждение, и что неверно в §5.1/E4-20/B27.

Пять пунктов журнала §0.6 — проверил, **все пять верны**:
(а) §5.1 (`E4-CORE-DESIGN.md:502`) молчит о драйвере — да, и молчание здесь ошибка
(инстанс, которому §5.1 велит отдать `bridge_tx`, ведётся коалесцирующим вотчем);
(б) «ET делает две вещи — границы и `track`» — по коду одна (`:655` внутри
`track_and_trigger`);
(в) якоря §5.1/B27 (`node/dpos.rs:1530-1537`, `consensus/dpos.rs:2040-2053`) дрейфнули
от базы — на `HEAD` это `:1355` и `:2058`;
(г) E4-20 (`:410`) «стенд считает `tracked_mismatches` именно потому, что они [два ET]
могут разойтись» — неверно, счётчик межузловой (`stand.rs:431-433`);
(д) §5.1 не называет, что вместе с signer-плоскостным ET исчезает место рождения
моста — верно, `bridge_rx` пришлось везти вниз.

**Где журнал вводит в заблуждение — своё:**

1. **§7 журнала утверждает, что `.claude/dpos_architecture/**` не обновлялся. Это не
   так.** `06_staking_layer.md:422-444` и `08_node_integration…md:70-76` уже несут
   блоки «[REVISED 2026-09-11 … Б2′]» с описанием захода (`.claude/` в `.gitignore`,
   поэтому в `git status` их не видно — `git check-ignore -v` подтверждает). Но
   обновление НЕПОЛНОЕ и правило проекта нарушено — см. **B2p-05**.
2. **§0.4 журнала: «`cargo fmt --check` 0 диффов».** Верно, но проверено на всём
   воркспейсе; постановка просила ограничиться четырьмя файлами. Разницы нет (ноль
   везде), фиксирую только методику.
3. **§8.3 журнала: «хук не пропускает границ проверено на двух опорах, не на трёх; в
   commonware не ходил».** Третья опора существует и лежала под рукой —
   `.claude/COMMONWARE_INTERNALS.md:197-198` (контигуозность `Update::Block` с якорями
   в `marshal/core/actor.rs`). Не находка в коде, но самооценка «слабее всего» здесь
   занижена зря.
4. **§0.2/Д-42 журнала: «Все три [вещи, ради которых `cold_start` оставлен в слое]
   идемпотентны против поллера, который мог их уже сделать».** Это главная неверная
   фраза захода. Идемпотентна только ПЕРВАЯ (пол). Вторая (заморозка геометрии) не
   идемпотентна на уровне ПУБЛИКАЦИИ — **B2p-01**. Третья (стартовая эпоха в мосту) не
   идемпотентна вовсе — она write-once и достаётся тому, кто пришёл первым —
   **B2p-02**.
5. **Док `raise_anchor_height` (`epoch_transition.rs:309-310`): «both writers of
   `anchor_height` reach ONE instance».** Писателей три пути, не два: `cold_start` из
   поллера (`node/dpos.rs:1576`), `cold_start` из слоя (`dpos.rs:2096-2099`) и
   `raise_anchor_height` из шва (`dpos.rs:2357`). Формально «два МЕТОДА» — но фраза
   читается как «две точки вызова» и прячет именно ту третью, которая создаёт B2p-02.

---

## §1. Находки

| id | серьёзность | file:lines | HEAD-якорь | что не так | чем пытался опровергнуть | уверенность |
|---|---|---|---|---|---|---|
| **B2p-01** | **BLOCKER** | `node/dpos.rs:1568-1587` (гейт `frozen_before` + единственный `geometry_tx.send_replace` на `:1580`); `dpos.rs:2096-2099` (второй замораживатель); `beacon/plane.rs:828-838` (парковка `DkgActor`) | `HEAD:node/dpos.rs:1377`, `:1454` — `et_arc` клонировался ТОЛЬКО в поллер, т.е. поллер был ЕДИНСТВЕННЫМ, кто мог заморозить геометрию, и публикация была безусловной | Публикация `(activation, interval)` в `geometry_tx` живёт ВНУТРИ ветки `else if let Ok(Some(hash)) = provider.block_hash(fin)`, то есть исполняется только когда геометрию заморозил САМ поллер. Заход добавил второго замораживателя того же инстанса — `cold_start` слоя (`dpos.rs:2096`). Если он успевает первым, следующий тик поллера видит `frozen_before == true` (`:1568`) и уходит в `None` — `geometry_tx` НИКОГДА не публикуется. `DkgActor` вечно висит в `loop { if let Some(frozen) = *geometry.borrow_and_update() … geometry.changed().await }` (`beacon/plane.rs:829-838`): без ошибки, без лога, до конца процесса. Нет DKG ⇒ нет share ⇒ share-гейт демотит узел в verify-only навсегда. Заодно не срабатывает `committee_wake.anchor_advanced()` (`:1586`) | Искал второго писателя `geometry_tx` — `grep -n "geometry_tx" crates/node/src/dpos.rs` даёт `:1411` (создание), `:1487` (клон), `:1580` (единственный `send_replace`). Искал, может ли `DkgActor` стартовать без геометрии — нет, `plane.rs:829-838` это буквально парковка на первом `Some`. Искал, гарантирован ли выигрыш поллера: нет — поллер при `latest == None` уходит в `changed().await` (`node/dpos.rs:1525-1533`), а слой берёт якорь НЕ из этого вотча, а из `provider.block_hash(archive_finalized)` / `wait_for_activation_block` (`dpos.rs:1729-1736`, `:1725-1727`), то есть может стартовать, пока вотч пуст; комментарий `node/dpos.rs:1385-1392` сам признаёт, что маркер может быть не выставлен на старте процесса | высокая по коду; конкретная вероятность гонки не измерена (живого прогона не делал) |
| **B2p-02** | **BLOCKER** | `epoch_transition.rs:513` (write-once бутстрап), `dpos.rs:2096-2099` (`cold_start` слоя на ordering-якоре), `node/dpos.rs:1576` (`cold_start` поллера на EL-`fin`) | `HEAD:dpos.rs:2058-2069` — signer-плоскостной ET был СВЕЖИМ, его `cold_start(latest_finalized)` всегда попадал в бутстрап-ветку и всегда ставил стартовую эпоху в мост | Стартовая эпоха, попадающая в мост (= единственное ребро, по которому `epoch_manager` вообще узнаёт новую эпоху, `epoch_manager.rs:783-790`), достаётся тому `cold_start`'у, который пришёл ПЕРВЫМ, потому что бутстрап-ветка гейтится `last_tracked_epoch.is_none()`. Первым почти всегда приходит поллер, и он работает в EL-шкале. Два следствия. (i) Рестарт в окне шириной K после границы: `epoch(fin) = E−1`, `epoch(latest_finalized) = E` ⇒ в мост уходит `E−1`, `cold_start` слоя возвращает `Intra`, и движок эпохи `E` не спавнится до конца `E` (следующая граница даст `E+1`, `:544`). (ii) ХУДШЕЕ: холодный прыжок. `latest_finalized` переписывается посадкой (`dpos.rs:1863-1864`, `cold_start_jump_self_heal` возвращает `(landing, hash, floor)`, `:1290`), а поллер к этому моменту давно заморозился и трекнул ДОпрыжковую эпоху; `cold_start(landing)` уходит в граничную ветку и молчит. Эпоха посадки не входит вовсе. Шов `executor.rs:2635-2643` тут не спасает — он для СТАЦИОНАРНОГО ре-прыжка, а холодный идёт до движка | Искал второй путь входа в стартовую эпоху: `outer.boundary_sender()` имеет ровно двух отправителей — форвардер `dpos.rs:2644-2656` и follower-ветка `:3500`; `boundary_rx` — «The only edge carrying a fresh epoch» (`epoch_manager.rs:785-786`). Проверил, что `committee.committee(initial_epoch_u64)` на `dpos.rs:2036` только РЕГИСТРИРУЕТ схему, движка не спавнит (его собственный комментарий `:2057-2063` прямо говорит, что вход даёт `cold_start` через мост). Проверил порядок: `build_beacon_plane` (`node/dpos.rs:737`) спавнит поллер ДО `launch_dpos_layer` (`:747`); поллер берёт значение вотча сразу, без ожидания `changed()` (`node/dpos.rs:1523`, комментарий `:1509-1515`) | высокая по коду для (ii); для (i) — высокая, но окно узкое (≈K/interval рестартов) |
| **B2p-03** | MODERATE | `epoch_transition.rs:398-424` (аргумент `debug_assert` «only the in-order delivery path may park»), `:584` (парковка в граничной ветке), `dpos.rs:2096-2099` | `HEAD:dpos.rs:2069` — `cold_start` на СВЕЖЕМ инстансе всегда шёл в бутстрап-ветку, которая парковку СБРАСЫВАЕТ (`epoch_transition.rs:515`) и никогда не ставит | Появился третий продюсер парковки: `cold_start` слоя может уйти в граничную ветку и припарковать `latest_finalized` (`:584`) — этот сайт `debug_assert`'а не имеет, а сам `debug_assert` (`:419-424`) по-прежнему утверждает, что парковщик один. Наблюдаемый исход клоббера в релизе (assert молчит): ровно один вход в эпоху теряется навсегда, без лога выше `debug` — гейт `last_tracked_epoch < Some(next)` (`:544`) спокойно пропустит следующую границу | Пытался опровергнуть достижимостью: столкновение требует, чтобы парковка слоя пережила целый интервал (replay идёт первым, `:361-389`, а первая доставка хука — `latest_finalized + 1`), то есть чтобы исполненный хвост интервал не дотягивал до `B − K`. Это не невозможно в глубоком бэкфилле, но узко. Комментарий `:398-417` при этом не обновлён и теперь просто неверен | средняя (механизм — высокая, достижимость — средняя) |
| **B2p-04** | MODERATE | `node/dpos.rs:1568-1570` (снятая ветка `on_finalized`), `epoch_transition.rs:655` (единственный `track`) | `HEAD:node/dpos.rs:1538` `Some(et.lock().await.on_finalized(fin).await)` | В окне «процесс поднялся — движок ещё не стартовал» (для узла с холодным прыжком это до `EL_SYNC_BACKSTOP_CEILING` = 6 ч, `dpos.rs:1836-1866` + цикл повтора `:1855-1880`) `oracle.track` теперь не обновляется НИ РАЗУ после первого `cold_start`. Именно в этом окне крутится цикл «жду, пока кто-то из комитета отдаст фронтир» — а комитет за часы сменится | Смягчение нашёл и оставляю в силе: трекаемое множество — `active_registry_peers ∪ committee[E] ∪ committee[E+1]` (`epoch_transition.rs:621-653`), т.е. ВСЕ активированные валидаторы, а не только комитет; устаревает оно не со сменой комитета, а только с оборотом реестра. Поэтому не BLOCKER. Но на `HEAD` это окно всё же имело (ненадёжный) апдейтер, а теперь не имеет никакого | средняя |
| **B2p-05** | MINOR | `.claude/dpos_architecture/06_staking_layer.md:465-470`; `08_node_integration_crates_node_bins_fluent.md:61-63`; `00_preamble.md` (блок `verified-against`) | — | Док-дрейф в обход правила проекта («update the affected section files **and** the `verified-against` block IN THE SAME change»). Секции 06 и 08 ОБНОВЛЕНЫ (блоки «[REVISED 2026-09-11 … Б2′]»), но: (а) `06:465-470` двумя абзацами ниже по-прежнему пишет «once frozen it switches to the steady `on_finalized` walk» и «The engine's OWN EpochTransition (`consensus/dpos.rs:2386` `EpochTransition::new`, cold-started at `:2396-2399`)» — оба утверждения теперь ложны и прямо противоречат новому блоку в том же файле; (б) `08:61-63` «Wires: staking reader + cache + EpochTransition» — `DposLayer::launch` ET больше не строит; (в) в `00_preamble.md` записи `verified-against` для Б2′ **нет** — есть для проходов A, Б1 и Б2 (`:138`, `:119`, `:93`), для Б2′ ничего | Искал запись грепом по `Б2′`/`B2p` в `00_preamble.md` — пусто. Проверил, что `.claude/` в `.gitignore` (`git check-ignore -v` → `.gitignore:46`), поэтому отсутствие файлов в `git status` не означает «не трогали» | высокая |
| **B2p-06** | MINOR | `testbed/stand.rs:2098-2107`; `:1556-1560`; `:1579-1581` | те же строки на `HEAD` (стенд не менялся) | `:2100-2107` описывает «the OTHER production transition … the beacon plane's peer-set tracker (`node/src/dpos.rs:1695`)» — второго инстанса больше нет, и плоскость EL-курсор для `on_finalized` не потребляет; комментарий читается как объяснение, ПОЧЕМУ стенд кормит ordering-высоту, и теперь объясняет несуществующее. Соседние якоря (`consensus/src/dpos.rs:2025-2041`, `:2739-2749`, `:2044-2047`) указывают на посторонний код | Проверил, что часть дрейфа досталась от базы: `HEAD:dpos.rs:2739-2749` — это поля `CertInlet`, т.е. якорь врал уже на `HEAD`; а вот `:2025-2041`/`:2044-2047` на `HEAD` указывали на область моста и стали неверны именно из-за захода. Постановка Б2′ разрешала править стенд — правка одной строки была доступна | высокая |
| **B2p-07** | NIT | `lib.rs:89-92` | `HEAD:lib.rs:88-92` | Реэкспорт `PlaneEpochTransition` в `pub use dpos::{…}` никем не используется: единственный потребитель берёт его по полному пути `fluentbase_consensus::dpos::PlaneEpochTransition` (`node/dpos.rs:1038`, `:1887`, `:1978`), а `pub mod dpos` открыт (`lib.rs:41`). Два имени для одного типа в публичном API | `git grep -n "PlaneEpochTransition"` — шесть вхождений, ни одно не импортирует через корень крейта. Clippy молчит (реэкспорт `pub`, «не используется» не ловится) | высокая |
| **B2p-08** | NIT | `dpos.rs:2096-2101` (`.wrap_err("epoch_transition cold_start failed")?`), `epoch_transition.rs:551`, `:654` | `HEAD:dpos.rs:2066-2072` | После слияния `cold_start` слоя может уйти в ГРАНИЧНУЮ ветку, где `?` пропускает наружу `epoch_committee_snapshot(next, at)` (`:551`, эпоха `E+1`, не `E`) и `check_peer_set_size` (`:654`) — и любой из них валит СТАРТ узла, тогда как бутстрап-ветка на пустом снапшоте отвечала `Ok(Intra)` (`:521-523`). Разница узкая (обе ветки и так `?`-ят чтение комитета), но набор читаемых эпох сдвинулся на одну вверх | Проверил, что `commitEpochCommittee` идёт с запасом в 2 эпохи (`epoch_transition.rs:557-566` со ссылкой на `node/src/evm.rs:895-918`), поэтому `E+1` обычно читаемо. Понижаю до NIT | средняя |

**BLOCKER'ов два — B2p-01 и B2p-02.** Оба — регрессии ровно на том пути, ради которого
заход делался (вход в эпоху / старт плоскости), и ни одна не предписана §5.1/B27:
проект говорит «ET строится в плоскости, движок получает тот же `Arc` и `bridge_rx`», но
НЕ говорит, что делать с тем, что у одного инстанса теперь два холодных старта в разных
шкалах. Общий корень у них один: **заход слил ИНСТАНС, но не слил СОБЫТИЯ «геометрия
заморожена» и «стартовая эпоха выбрана» — обе остались привязаны к конкретному
вызывающему.**

---

## §2. Поведение по ханкам против HEAD

| ханк | до (`HEAD`) | после | поведенческая разница |
|---|---|---|---|
| `dpos.rs:13` | `use crate::executed::executed_state_hash` | удалён | нулевая: единственным потребителем был конструктор удалённого ET; `executed_state_hash` для ЕДИНСТВЕННОГО ET теперь строится в `node/dpos.rs:1382` над `node.provider` — тем же провайдером, что `RethHandle.provider` (`node/dpos.rs:2051`). Проверил: оба `node.provider.clone()` |
| `dpos.rs:36` | `use fluentbase_p2p::NoopBlocker` | `{NoopBlocker, OracleHandle}` | нулевая; `OracleHandle` нужен для сигнатуры `PlaneEpochTransition`. Тип sink'а совпадает с `HEAD` (`HEAD:dpos.rs:2060` `oracle.clone()`, `node/dpos.rs:1379` `handles.oracle.clone()` — тот же plane-oracle) |
| `dpos.rs:1055-1080` | — | новый `pub struct PlaneEpochTransition` | новая публичная форма; см. §3 |
| `dpos.rs:1545-1566` | `launch(ctx, reth, cfg, shutdown)` | `+ epoch_transition` пятым | ломающая сигнатура; поведение то же |
| `dpos.rs:2077-2086` | локальные `(bridge_tx, bridge_rx)` + `EpochTransition::new` | деструктуризация `PlaneEpochTransition` | **инстанс перестал быть свежим.** Отсюда растут B2p-02 (бутстрап-гейт `last_tracked_epoch.is_none()` уже мог быть закрыт) и B2p-03 (парковка в граничной ветке). Побочно: единственный живой `Sender` моста теперь лежит ВНУТРИ ET (на `HEAD` слой держал ещё и свой клон) — на практике безразлично, `Arc` живёт до конца процесса |
| `dpos.rs:2088-2101` | `epoch_transition.cold_start(...)` на свежем | `et_arc.lock().await.cold_start(...)` на разделяемом | см. B2p-01/02/08 |
| `dpos.rs:2139-2141` | `let et_arc = Arc::new(Mutex::new(epoch_transition))` | удалён | нулевая |
| `lib.rs:89-92` | — | `+ PlaneEpochTransition` | см. B2p-07 |
| `epoch_transition.rs:295-319` | док: монотонность — свойство ПРОВОДКИ | док: свойство ТИПА | док теперь ближе к правде, но «both writers … reach ONE instance» прячет третий путь вызова (§0.11 п. 5) |
| `epoch_transition.rs:687-701` | `self.anchor_height = Some(head_number)` | `self.raise_anchor_height(head_number)` | **единственная чисто-поведенческая правка в машине**, и она правильная: повторный `cold_start` с более низкой высоты больше не возвращает пол в отрезанное прыжком окно. Закрывает пункт (д) ревью Б2. Замечу, что она НИЧЕГО не делает для эпохи и для геометрии — а именно они и сломались |
| `epoch_transition.rs:2282-2415` | — | два теста | см. §0.10, §0.5 |
| `node/dpos.rs:762`, `:941`, `:1030-1038`, `:1129-1135`, `:1887-1890`, `:1974-1981`, `:2213` | — | проводка ET вниз | нулевая сама по себе; носитель B2p-01/02 |
| `node/dpos.rs:1365-1386` | ET с `None` | мост рождается тут, ET с `Some(bridge_tx)` | ёмкость 64 та же; **начиная с этой строки поллерный `cold_start` ставит стартовую эпоху В МОСТ**, чего на `HEAD` не делал (`None` глушил `try_send`). Это и есть механизм B2p-02(i) |
| `node/dpos.rs:1453-1473`, `:1556-1570`, `:1601` | `frozen_before ⇒ on_finalized(fin)` | `frozen_before ⇒ None` | поллер перестал вести границы — цель захода, выполнена. Но ветка `frozen_before` теперь ГЛУШИТ и публикацию геометрии, потому что публикация лежит в соседней ветке (B2p-01) |

---

## §3. Граница (`pub`)

Новое публичное:
- `pub struct PlaneEpochTransition<Provider, EvmConfig>` (`dpos.rs:1071`) с
  `pub transition: Arc<Mutex<EpochTransition<RethStakingStateReader<P, E>, OracleHandle>>>`
  (`:1075-1076`) и `pub bridge_rx: mpsc::Receiver<(u64, ValidatorSetSnapshot)>` (`:1078`).
  Публичные поля здесь ОБЯЗАТЕЛЬНЫ: `node/dpos.rs:1887-1890` конструирует структуру
  литералом из другого крейта. Конструктора нет — согласен, он бы ничего не добавил.
- Реэкспорт `lib.rs:91` — лишний (B2p-07).

Изменённое публичное (ломающее):
- `DposLayer::launch` — пятый параметр (`dpos.rs:1565`). `!` в сообщении коммита
  журнала (§6) оправдан.
- `EpochTransition::cold_start` — сигнатура та же, КОНТРАКТ изменился (пол
  поднимается, а не назначается); док обновлён (`epoch_transition.rs:688-696`).

`pub(crate)`, наружу не течёт:
- `BeaconPlane` → `BeaconPlane<Provider, EvmConfig>` (`node/dpos.rs:941`),
  `build_beacon_plane` → `BeaconPlane<<N as FullNodeTypes>::Provider, <N as
  FullNodeComponents>::Evm>` (`:1135`), поле `epoch_transition` (`:1038`),
  `launch_dpos_layer` (`:1978`). `git grep BeaconPlane` за вычетом `SharedBeaconPlane`
  — только эти площадки.

Ничего не стало публичным зря; ничего нужного не осталось приватным.

---

## §4. Ворота (verbatim, прогон ревьюера, `CARGO_BUILD_JOBS=6`)

~~~
$ cargo test -p fluentbase-consensus --lib
test result: ok. 668 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 25.38s

$ cargo test -p fluentbase-consensus --lib --features dpos-devnet-byzantine testbed::
test result: ok. 40 passed; 0 failed; 0 ignored; 0 measured; 636 filtered out; finished in 35.77s

$ cargo test -p fluentbase-node --lib
test result: ok. 57 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 57.50s

$ cargo test -p fluentbase-staking-reader
test result: ok. 60 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
test result: ok. 0 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s

$ cargo test -p fluentbase-consensus --test slasher_integration
test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s

$ cargo clippy -p fluentbase-consensus -p fluentbase-node --all-targets
warning: large size difference between variants
warning: `fluentbase-node` (lib) generated 1 warning
warning: `fluentbase-node` (lib test) generated 1 warning (1 duplicate)

$ cargo clippy -p fluentbase-consensus -p fluentbase-node --all-targets --features dpos-devnet-byzantine
warning: large size difference between variants
warning: `fluentbase-node` (lib) generated 1 warning (1 duplicate)
warning: `fluentbase-node` (lib test) generated 1 warning
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1m 36s

$ cargo fmt --all --check | grep "^Diff in" | sort -u
(пусто — ноль диффов по всему воркспейсу, включая четыре файла захода)

$ cargo doc -p fluentbase-consensus --no-deps 2>&1 | grep -c "unresolved link"
8
~~~

Плюс мутационный прогон и восстановление файла — verbatim в §0.5.

Отдельно: моя НЕВЕРНАЯ форма clippy-с-фичей
(`--features fluentbase-consensus/dpos-devnet-byzantine`) упала
`error[E0063]: missing field byzantine … crates/node/src/dpos.rs:2146:21` — это
дефект команды (фича не включена для `fluentbase-node`), не захода; правильная форма
выше зелёная.

---

## §5. Оставить как есть

1. **Структура `PlaneEpochTransition` вместо трейта.** Обе половины — один объект
   (`bridge_rx` сливается только там, где есть `boundary_sender()`), два из трёх
   глаголов `async`. Трейт добавил бы боксированные фьючи и ничего не купил. Д-41/Д-43
   согласованы.
2. **`cold_start` через `raise_anchor_height` (`:700`).** Правильная правка, красный
   тест до неё, тест не вакуумен. Единственное место захода, которое делает машину
   строго лучше.
3. **Тест `a_coalesced_driver_skips_the_boundary_a_stepping_one_enters`.** Пинит
   свойство МАШИНЫ, а не проводки, поэтому переживёт 4.2/4.3. Мутацию воспроизвёл сам —
   не вакуумен. Хороший тест, оставить.
4. **`cold_start` слоя оставлен (Д-42).** Сам факт правильный — движку нужны геометрия,
   пол на послепрыжковом якоре и стартовая эпоха. Ломается не решение, а его
   обоснование («всё три идемпотентны»). Чинить надо публикацию геометрии и выбор
   стартовой эпохи, а не выкидывать вызов.
5. **Поллер не ведёт границы.** Цель захода, выполнена, доказана тестом. Не трогать.
6. **`#[allow(clippy::too_many_arguments)]` не размножен**, `unwrap` на продакшн-пути
   ноль, таймеров и поллинга не добавлено. Дисциплина выдержана.
7. **`tracked_mismatches` и стендовый драйвер** — менять нечего: стендовый ET и так
   один на узел, и его драйвер теперь ровно продакшновый.
8. **`epoch_at(fin)` в тумбстоун-ветке** — поведение `HEAD`, не заход. Чинить в 4.2/4.3
   вместе со шкалами, а не здесь.

---

## §6. Вне рамок

**Б3** (из `E4-1-B2.md` §7.2): `FakeStaking` с составом по ветке хэша, весами и
тумбстоунами; три стенд-теста; e2e-пин `commit_height(E) = start(E−2)`.

**4.2** (`is_live_epoch` по tip'у, один фронтир, `deliver` как единственная точка
доверия). Заход оставляет ей готовым ровно одно: `T = last_tracked_epoch`
(`E4-CORE-DESIGN.md:512`) теперь существует в единственном экземпляре и ведётся
драйвером, который его не пропускает. **Но** — с оговоркой: пока B2p-02 не закрыт,
`T` на старте может оказаться на эпоху ниже правильного (или на много эпох ниже
после холодного прыжка), а лестница фронтира ступает от `T`. То есть 4.2 унаследует
неверный `T`, если B2p-02 не починить ДО неё.

**4.3** (`track`): сайтов правки стало на один меньше — продакшн-вызывающий теперь один
(`epoch_transition.rs:655`). Сюда же — переезд `epoch_at(fin)` тумбстоун-поллера
(`node/dpos.rs:1638`) на ordering-шкалу: после слияния это последнее место, где
плоскость считает эпоху сама, и K-сдвиг вокруг границы там остаётся.

**Не для строки 4.1, но перед вливанием:** блок `verified-against` в
`.claude/dpos_architecture/00_preamble.md` без записи Б2′ и протухшие абзацы
`06_staking_layer.md:465-470` / `08_…:61-63` — по правилу проекта это блокер ревью, а
не follow-up (B2p-05).
