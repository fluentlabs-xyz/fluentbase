<!-- Независимая оценка закрытия Э4 (строки 4.1, 4.2, 4.3). Дерево /home/djadjka/Work/fluentbase,
ветка djadjka/dpos-reth-2.2-squashed, HEAD fca897c1, диапазон git log 1b11b61f..HEAD (73 коммита).
Написано 2026-09-12 одной сессией Opus 5 в свежем контексте; всё, помеченное [KNOWN], открыто или
запущено в ЭТОЙ сессии. Агентов не запускал. Git только на чтение. -->

# Э4 — оценка закрытия: комитет как значение, один фронтир, членство на границе

## 0. Состояние дерева и мои прогоны

- [KNOWN] `git rev-parse HEAD` = `fca897c1fbb1136d059435aa174b3cc6bbc3ad4e`; `git log --oneline 1b11b61f..HEAD | wc -l` = **73**. Из них по `git show --stat` мои объекты оценки — коммиты узла/ридера/p2p/стенда и их `docs(dpos)`; параллельная работа владельца (`contracts/staking/**`, `crates/staking-abi/**`, `E1-*`, `D4-BRIEF`, `DECISIONS.md`) отделена и не оценивается.
- [KNOWN] `git status --short` на входе и на выходе: одна запись — ` M .dpos-study/history/E4-ORCHESTRATOR.md` (+2 строки, чужой файл, не трогал). Кроме неё дерево совпадает с HEAD. `git diff --stat` = `1 file changed, 2 insertions(+)`.
- [KNOWN] Мои прогоны (`CARGO_BUILD_JOBS=4`):
  - `cargo test -p fluentbase-consensus --lib committee::tests` — **25 passed; 0 failed** (660 filtered out).
  - `cargo test -p fluentbase-consensus --lib plane_upstream::tests` — **13 passed; 0 failed**.
  - `cargo test -p fluentbase-consensus --lib dpos::gated_receiver_tests` — **2 passed; 0 failed**.
  - `cargo test -p fluentbase-staking-reader` — **63 passed; 0 failed** (+ doctest 0/0/1 ignored).
  - `cargo test -p fluentbase-p2p` — **32 passed; 0 failed**, `+ 1 passed; 0 failed; 3 ignored`, `+ 0/0`.
  - Три мутации прод-кода с откатом байт-в-байт (§4).
- Полные наборы не перегонял (запрет постановки; идёт живой docker-прогон). Числа финальных ворот беру из `gates/a3v-*.txt` и сверяю их содержимое сам (§4).
- Длинные строки читал целиком: `git show HEAD:<path> | cat -n | sed -n 'A,Bp'`, `sed -n`, Read tool. Ни одного `cut -c`/`head -c`.
- Чекауты: commonware `~/.cargo/git/checkouts/monorepo-27b478c9bb41d208/3c4e02c` (после `.claude/COMMONWARE_INTERNALS.md`), reth не открывал.

**Определение «свойств 1–6».** Отдельного нумерованного списка в `.dpos-study` нет — я его искал (`grep 'св\. '` по `E4-CORE-DESIGN.md`, `history/D4-BRIEF.md`, `DECISIONS.md`; `grep 'свойств'` по `E4-prompts/*`). Свойства реконструируются однозначно из мест, где на них ссылаются: `E4-CORE-DESIGN.md:197` («где EL синхронизируется к хэшу до аутентификации (св. 2)»), `:225` («где не-член может вызвать EVM-чтение или аллокацию до проверки членства (св. 3)»), `:370` («категория (св. 6)» = таксономия Д/Г/С1/С2/С3/О/Н), `:418` («явный retry-forever с gauge — приемлемо по св. 4»), `:616` («второй путь рядом с первым (св. 5)»), `:587` («`L1Fork ⇒ SafetyHalt` … (св. 6)»), плюс постановка вопроса 9 контр-рецензии (`E4-DESIGN-CRITIQUE.md:18`). Итого:

| св. | формулировка | соответствие §3 |
|---|---|---|
| 1 | Один комитет: одно значение из одного якоря, без второй таблицы и без живого чтения там, где нужно замороженное | I1 |
| 2 | Ничто не синхронизирует EL к хэшу, чей сертификат не проверен под локально читаемым комитетом | I2 |
| 3 | Членство до ресурсов: не-член не покупает EVM-чтение, аллокацию тел и доставку | I3 |
| 4 | Детерминизм рестарта: у каждого состояния явный вход и выход; транзиентно ⇒ повтор, постоянно ⇒ громко | I4 |
| 5 | Один путь, а не второй рядом с первым (нет параллельного механизма без общей рамки) | — (сквозное) |
| 6 | Границы названы честно: каждый отказ отнесён к классу, BFT-граница записана, а не заклеена | — (сквозное) |

---

## 1. Свойства 1–6 и инварианты I1–I5 после кода

### св. 1 / I1 — «Один комитет» — **ЧАСТИЧНО**

Что стало правдой [KNOWN]:

- Модуль есть и он write-once: `committee/store.rs:420-504` — шесть шагов (`geometry` → окно → `commit_height` → кэш → якорь → два staticcall'а на ОДНОМ хэше → `build` → `install`); `install` при повторном чтении с другим значением оставляет первую запись и отказывает эпохе (`:334-355`). Равенство между узлами доказано арифметикой окна, а не предположением: `WINDOW_FITS_THE_WEIGHT_RING` — `const_assert` на `WEIGHT_RING_EPOCHS − MAX_COMMITTEE_LOOKAHEAD_EPOCHS > SCHEME_RETENTION_EPOCHS` (`committee/mod.rs:98-102`; 16 − 2 > 8 по `crates/types/src/staking_protocol.rs:38,73,94` и `consensus/src/lib.rs` `SCHEME_RETENTION_EPOCHS`).
- Второй таблицы `epoch → scheme` нет: `EpochEntry{record, scheme}` — один слот (`store.rs:64-67`), `EpochSchemeProvider` стал ВИДОМ на карту (`outer.rs:255-290`), `CertInlet.schemes`/`CachedScheme`/`scheme_at_finalized_tip` удалены (grep пуст), единственный продюсер verify-схемы — `epoch_verifier` (`committee/mod.rs:521-535`), второй писатель — только `upgrade_scheme` с тремя монотонными отказами (`store.rs:526-575`).
- `tombstoned` в запись не входит и `is_member` его не читает (`mod.rs:58-68`, `store.rs:510-515`) — это ровно Д-1(а) с уточнением.
- Один `FinalizedCursor` на процесс: создаётся один раз и отдаётся и исполненной цепи, и якорю модуля (`node/dpos.rs:1387`, `:1408-1411`; follower — `node/cert_follow/mod.rs:127-131`).

Где ломается — **три оставшихся носителя и вторая копия геометрии** [KNOWN]:

1. **`EpochTransition` читает комитеты мимо модуля.** `staking-reader/src/epoch_transition.rs:615`, `:643`, `:805`, `:865` — `self.reader.epoch_committee_snapshot(...)` на СВОЁМ `at`. Именно из этих чтений собирается `TrackedPeers` (`:767-790`), то есть весь предмет строки 4.3 — маска `Ingress`, primary для commonware, адресаты ступени. Код это признаёт открытым текстом (`:729-734`: «The records come from the reader, not from the `committee/` module … making the module the single source is a Cargo-level move»). Якоря РАЗНЫЕ: модуль читает на `executed_state_hash(max(cursor, EL-finalized тег))` (`store.rs:713-725`), ET — на `executed_state_hash(fin)` плоскостного поллера (`node/dpos.rs:1626-1631`) и на `executed_hash(number − K)` в граничном контуре. У ET нет ни предиката окна, ни write-once, ни классификации `permanent`. Записано как Д-122; но §5.3 требовал «из записей модуля», и по коду это не так.
2. **`RethCommitteeSource::scheme_at`** — чтение комитета на ПРОИЗВОЛЬНОМ хэше (`cert_inlet.rs:127-140`, `:167-186`, `:173`). Осталось для двух by-height швов (`cold_start_jump.rs:738-759`, вызывающие — `dpos::refetch_verified_archive_hole` и `cert_follow::fetch_verified_boundary`). Сужено честно и задокументировано, но это второй авторитет с собственным курсором.
3. **Тумбстоун-поллер** читает полный снимок комитета (`node/dpos.rs:1694`) — по проекту это отдельный liveness-слой, не нарушение.
4. **Геометрия процесса — не одна.** `committee/mod.rs:388-398` утверждает буквально: потребитель `epoch_of(height)` «must not be able to reach a SECOND copy of it. That is the whole defect class this module exists to close». По коду копий три, из них ДВА независимых чтения цепи: (а) ET замораживает пару и публикует её в `geometry_tx` → модуль, beacon (`beacon/plane.rs:523`, `actor.rs:596`), `plane_upstream`; (б) слой консенсуса читает `dposActivationBlock`/`epochBlockInterval` САМ, на своём `cs_finalized_hash` (`consensus/dpos.rs:1745-1748`) и строит из них `OriginEpocher` для executor'а и движков (`outer.rs:722`) и ещё один на месте (`consensus/dpos.rs:1848`). Формула общая (`epoch_at_block`), значения — из двух разных чтений на двух разных высотах. Сеттеры `setEpochBlockInterval`/`setDposActivationBlock` в контракте ещё есть (их снимает П-10/Э2.4), поэтому расхождение латентно возможно, и его последствие теперь дороже, чем до Э4 (§9.3).

Вывод: значение записи одно и заморожено; «один якорь» и «нет второй таблицы» держатся для схем и для потребителей внутри `consensus`; «один источник» НЕ держится для peer-set'а и для геометрии.

### св. 2 / I2 — «Фронтир только по проверенной финализации» — **ДА, с одним названным исключением**

Полный перечень `sync_to`/`sync_to_checkpoint` на HEAD (grep по `crates/`, вычтены `mod tests` и `FakeElSync`) [KNOWN]:

| # | место | вход | проверен ли |
|---|---|---|---|
| 1 | `cold_start_jump.rs:818` `el.sync_to(&latest)` внутри `jump_to_target` | `latest` — параметр | да: единственные два вызывающих ниже |
| 2 | `consensus/dpos.rs:2960` `mk_el_sync(0).sync_to_checkpoint(l1_hash)` | операторский `--dpos.l1-checkpoint` | да (человек, не пир) |
| 3 | `consensus/dpos.rs:2984` `mk_el_sync(0).sync_to(&latest)` | `up.get_latest()` | **НЕТ** — единственный оставшийся TOFU |

Пути к `jump_to_target` — **ровно два**, оба через `executor::maybe_re_jump`: `consensus/dpos.rs:2421` (валидатор) и `:3180` (follower). Цель обоих — `self.marshal.pair_at(height)` из СОБСТВЕННОГО архива на высоте `Update::Tip` (`executor.rs:2558-2569`), а `Update::Tip` marshal шлёт только из `store_finalization` после `verify_delivered`. Ни один вызывающий не передаёт ответ пира. `verify_jump_structural`/`verify_jump_authenticated` как стадии сняты (`cold_start_jump.rs:712-733`), `JumpOutcome::{BadTarget, AuthFailed}` удалены (`:85`), после посадки остаётся `holds(latest.block.result)` ⇒ `InvalidTarget` ⇒ `Fault::corruption` (`:846-865`) и `holds(l1)` ⇒ `L1Fork` (`:868-888`), ошибка пробы ⇒ `Stalled`.

Точка доверия одна и она реализована как в проекте: `FrontierHandler::deliver`, пять шагов (`plane_upstream.rs:405-510`) — декод с cap `MAX_COMMITTEE_SIZE` (`:302-309`), `block.height == asked` (`:418-422`), payload↔digest (`:423-425`), `epoch_of(height) == round.epoch` над ЕДИНСТВЕННОЙ геометрией модуля (`:432-438`), `committee(epoch)` из модуля + BLS 2f+1 под verify-only схемой БЕЗ оракула (`:445-465`, `verifier_for` `:371-390`); `false` только на четырёх сигналах лжи, «не могу проверить» ⇒ отброс + `true` (`:486-490`). Живая эпоха тоже переехала на проверенный tip: `ordering_tip` пишется одним армом `Update::Tip` в `FluentApp::report` (`application.rs:1072-1086`), правило — `live_epoch_of`/`is_live_epoch_at` (`epoch_manager.rs:1919-1937`), `highest_observed_epoch`/`corroborate_frontier`/`sender_pins`/`latest_live` в `crates/` отсутствуют (grep пуст).

Исключение #3 ограничено предикатом: `fresh_follower_entry` (`consensus/dpos.rs:1393-1411`) на `deployed_network` отказывает при старте, а `is_deployed_network` (`node/dpos.rs:2364-2372`) считает деплойнутыми devnet/testnet/mainnet — то есть TOFU остаётся только для чужого chain_id (локальный харнесс), с `warn!` и явным текстом «REFUSED on a deployed network (E4-05)». Побочный эффект, который проект не называет: на этом пути reth'овский тег `finalized` становится полом якоря модуля (`store.rs:713-721` `cursor.height().max(tag)`), то есть ВСЕ записи комитета и все схемы процесса читаются из состояния, которое назвал один пир. Для прода это закрыто отказом при старте; записываю как расширение радиуса, а не как новую дыру.

### св. 3 / I3 — «Членство до ресурсов» — **ДА для названных каналов, ЧАСТИЧНО в целом**

По каждому входу [KNOWN]:

- **BEACON.** `GatedReceiver(members_only=true)` (`node/dpos.rs:1892-1897`), `admits` по `TrackedWindow::classify` до любого декода (`consensus/dpos.rs:112-143`, `p2p/lib.rs:376-392`); тумбстоун бьёт членство и не ждёт peer-set'а (`p2p/lib.rs:377-379`). Внутри актора — заголовочный пик `u64::read_cfg` до `DkgMsg::read_cfg` (`beacon/actor.rs:1963-1985`), окно `epoch_is_actionable` (`:1923-1929`) и `beacon_member` (`:1943-1945`) ДО тела. `on_confirm`: окно `[now, now+2]` до `committee_for` (`:1476-1491`).
- **EVIDENCE.** `GatedReceiver(members_only=true)` (`node/dpos.rs:1818-1823`); `ingest_batch` — тир до декода (`slasher/gossip.rs:114-123`), кап батча `2×MAX_COMMITTEE_SIZE` (`:54-56`), одна эпоха/view на батч (`:138-148`), `retains` (`:149-157`), `member_of(epoch)` (`:158-169`) — и только потом `committee_for` (`:174`).
- **VOTE/CERT/RESOLVER/BROADCAST/MARSHAL backup.** Потребителя больше нет: все пять backup-приёмников уходят в `observe_route_misses` — счётчик и ничего больше (`node/dpos.rs:1079-1088`, проводка `:1266-1285`). `pipeline_catchup_span`, `handle_msg_for_unregistered_epoch`, `forward_vote_backup` в `crates/` отсутствуют (grep пуст). Это закрывает E4-04/R-003 удалением, а не фильтром (Д-124) — сильнее, чем требовал проект.
- **FRONTIER.** `deliver` зовёт `committee(epoch)` с эпохой, которую выбрал ОТВЕЧАЮЩИЙ. Вне окна — отказ без EVM (`store.rs:437-442`); в окне это одна из ≤ 11 эпох и она write-once ⇒ ≤ 2 staticcall'а на эпоху за процесс. Ответ приходит только на наш собственный fetch. Ограничено.
- **MARSHAL-резолвер (не назван в §5.3, а это самый горячий вход).** `CertProvider::scoped(epoch)` (`outer.rs:287-289`) → `Committee::scheme` → `Committee::committee` (`store.rs:517-524`). Эпоха тоже выбрана отвечающим, окно то же, кэш тот же — НО на постоянном отказе кэша нет (`store.rs:76-88`: «A permanent failure is re-derived on every call … by design»), и два блокирующих staticcall'а выполняются на задаче актора marshal'а. См. §9.1.
- Аллокация тел: `deque_size` BROADCAST 4 (`consensus/dpos.rs:2563`, `:3422`), DKG bodies 2 (`beacon/dkg_transport.rs:135`), и то и другое — только primary (CW). Байтового лимита нет, и код это говорит прямо (`consensus/dpos.rs:2558-2562`): бюджет на пира = `4 × MAX_ORDER_BLOCK_SIZE` = 16 МиБ (`order_block.rs:31`). При нулевом пересечении трёх комитетов по 51 (`staking_protocol.rs:38`) верхняя граница primary — 153 пира ⇒ до ~2,4 ГиБ. Против прежнего (64 × 4 МиБ × весь реестр) это 16-кратное улучшение, но R-013 закрыт по механизму, а не по абсолютной величине.

### св. 4 / I4 — «Детерминизм рестарта» — **ЧАСТИЧНО**

Улучшилось [KNOWN]: `CommitteeError` — три варианта с направленной транзиентностью (`mod.rs:210-268`); `NotReadable` имеет явный выход — `Committee::subscribe` публикует на КАЖДОМ продвижении якоря, а не только на росте значения (`store.rs:609-647`), четыре вызывающих `anchor_advanced` (`executor.rs:1225`, `:2753`, `:3873`, `node/dpos.rs:1593`); `deferred_reconciles` — явный долг, снимаемый пробуждением (`epoch_manager.rs:1133-1157`); фатал inlet'а на нетранзиентном чтении СНЯТ — `None`-схема теперь одно отложение без ротации и без смерти узла (`cert_inlet.rs:618-639`); слэшер маршрутизирует по `is_transient` и отпускает застрявший charge (`slasher/actor.rs:712-725`); `soft_enter_span`/`register_soft_entered`/`cold_start_register` удалены, у эпохи один продюсер схемы.

Не держится:

- **Класс «постоянный отказ чтения» не имеет исхода, который требует §5.4.** Проект: «`AbiDecode`/порядок/дубликаты/пол/`weights: None` в окне ⇒ `Read(permanent)` ⇒ `Fault::corruption` — громко, узел стоит». Код: `failed()` увеличивает счётчик и пишет `error!` ОДИН раз на эпоху (`store.rs:192-207`), дальше фасад сворачивает любую ошибку в `None` (`facade.rs:47-54`), `epoch_manager` пишет `debug!` и выходит (`:1141-1157`), `deliver` отбрасывает, inlet откладывает, слэшер отпускает. Ни один потребитель не производит `Fault` — `git grep CommitteeError` по `crates/` даёт только эти пять мест. Узел остаётся «здоровым» и молча перестаёт участвовать в эпохе. §9.1.
- **«Повтора нет» из §5.4 тоже не реализовано, и в обратную сторону**: постоянный отказ НЕ кэшируется намеренно (`store.rs:76-88`), то есть повтор происходит на каждом вызове, включая горячий путь marshal'а.
- `ordering_tip` пишется `send_replace` без монотонного клампа (`application.rs:1080`); живая эпоха — функция текущего значения, поэтому откат tip'а откатил бы и роль. В процессе marshal tip монотонен, так что это запас прочности, а не наблюдаемый дефект.

### св. 5 — «второй путь рядом с первым» — **ДА в основном**

Удалены именно параллельные пути, а не добавлены фильтры поверх них: один `EpochTransition` (`479b60b3`), один продюсер схемы, один писатель фронтира, один потребитель backup-канала (его нет), один курсор финализации. Остатки второго пути — те же, что в I1: ET-чтения комитета и `OriginEpocher` слоя.

### св. 6 — «границы названы честно» — **ДА**

Отклонения зафиксированы поимённо (Д-1…Д-131, `E4-ORCHESTRATOR.md:263-277`), открытые приёмки записаны в `PLAN.md:102-104` («живые прогоны лестницы/E4-28 НЕ сделаны», «дозвон secondary — приёмка»), новые находки заведены как R-126/R-127 в тот же день. Ни одного места, где отказ переименован в успех, я не нашёл. Единственное, что названо слабее, чем есть: §5.4 обещает `Fault::corruption` там, где кода нет (выше), и это не помечено как отклонение.

### I5 — «Живучесть при ≤ f» — **ЧАСТИЧНО**

Что может один tracked-пир ПОСЛЕ этапа [KNOWN]: (а) на BEACON/EVIDENCE — ничего, если он не член нужной эпохи; (б) на фронтире — соврать один раз и потерять канал навсегда (`deliver=false` ⇒ `block!` + `fetcher.block` ⇒ `excluded`, никогда не очищается: CW `resolver/p2p/engine.rs:425-441`, `fetcher.rs:509-517`); (в) удержать до 16 МиБ тел, если он в primary; (г) не отдать ступень — узел продолжает контигуозный догон (`executor.rs:2249-2271`, метрика `frontier_step_unserved`). Раздутый `Latest` больше не двигает ничего: `upstream_frontier` удалён, триггер прыжка читает только собственный tip (`executor.rs:2524-2542`).

Что может один ЧЕСТНЫЙ пир — новое и хуже: отдать на MARSHAL-резолвере финализацию эпохи, схему которой этот узел построить не может, и быть исключённым навсегда (§9.2). И что может ОДИН ЧЕСТНЫЙ КОМИТЕТ: при отставании `last_tracked_epoch` от часов beacon'а DKG-кадры входящего комитета `now+2` не проходят транспортный шов (§9.3, R-126).

Остаются открытыми из прежнего: `VoteStore` не ограничен (R-014, перенесён в Э7); лестница требует живого члена `committee(T+1)` (B2-01, escape `--dpos.follower-upstream`); нулевое пересечение на границе (R-121) — граница beacon'а.

---

## 2. Три самых опасных места — см. §9

---

## 3. Где оркестратор принял чужое утверждение без квитанции

Раздел «Квитанции» state-файла (`E4-ORCHESTRATOR.md:109-231`) дисциплинирован: релейные утверждения помечены («по реляции», «не открывал», «[LIKELY]»). Но решения на них построены. Места, где ДЕЙСТВИЕ есть, а квитанции нет или она проверяет не то:

1. **Э4.1-А, находки R-05…R-14, R-16…R-18** — решения FIX/RECORD приняты и позже засчитаны как FIXED дважды подряд без единого открытого якоря: `:113` («сами якоря не открывал … по реляции ревьюера») и `:115` («Остальные FIXED (R-05…R-14, R-16…R-18) — по реляции F1 … сам не открывал»). 13 находок закрыты релеем.
2. **Э4.1-Б1, B1-07…B1-15** — то же: `:120` («решения … сами якоря не открывал (реляция)») и `:122` («Остальные FIXED (B1-08, B1-10…B1-15) — реляция F2»). 8 находок.
3. **«Красный до правки» за весь заход Б2** — `:135`: «Тесты `committee/tests.rs:1352`, `:1409`, `:1458`, `epoch_manager.rs:3599`, `:3640` существуют. «Красный до правки» — реляция F3, не проверял.» Квитанция проверяет СУЩЕСТВОВАНИЕ тестов, а не то, что они падали до правки, — то есть проверяет не то. Фальсифицируемость пяти новых тестов не подтверждена ничем.
4. **A2-05 REJECTED** — `:161`: «не открывал (`cert_inlet.rs:2872-2894` `debug!`, `outer.rs:121-146` — реляция)». Находка ревью ОТКЛОНЕНА на релейном якоре; это действие без квитанции в чистом виде.
5. **Измерение, обосновавшее REJECT A2-01** — там же: «Само измерение (красный `a_forged_seed_slot_…` без `servable`) — реляция, не повторял.» BLOCKER оставлен открытым на релейном измерении.
6. **Детерминизм ×3 после F7** — `:160`: «×3 не повторял (реляция F7: три прогона байт-в-байт)».
7. **Э4.3, мутации фикс-прохода** — `:224`: «Реляция F12: … мутации М1/М2 красные с откатом.» Мутации третьего прохода 4.3 оркестратором не воспроизведены (мутации ревьюера R10 — воспроизведены им же, не оркестратором: `:226` «две мутации «красный до правки» воспроизведены» — это реляция R10).
8. **Э4.3, «Ревью §5 «оставить как есть» — не открывал»** (`:222`), при том что решения по §5 вошли в фикс-таблицу.

Итого мест, где действие опирается на непроверенное утверждение: **8** (из них 2 — прямые решения по находкам: п. 4 и п. 5; 3 — закрытие пачек находок: п. 1, 2, 7; 3 — свойства прогонов: п. 3, 6, 8).

**[KNOWN] опровергнутые/неподтверждённые.**

- **Опровергнуто 1.** `E4-ORCHESTRATOR.md:218`, помечено «**[KNOWN — открыто мной]**», перечисляет `TrackedPeers::is_secondary_only()` по якорю `epoch_transition.rs:169`. На HEAD такого символа в дереве НЕТ: `git grep -n is_secondary_only HEAD -- crates/` пусто; `epoch_transition.rs:169` — середина `epochs_of`. `git log -S is_secondary_only --all` даёт единственный коммит `fca897c1` — то есть символ существовал только в незакоммиченном дереве и был снят фиксом P-18 (`E4-3-A.md:522`). Квитанция была верна в момент написания, но в закрывающем документе она стоит как `[KNOWN]` про дерево, которого нет; тот же якорь вписан в постановку ревьюеру (`E4-prompts/4.3-A-review-1.md:13`).
- **Неподтверждено (дрейф якорей).** Та же квитанция `:218`: `on_confirm` окно «`:1432-1442` до `committee_for` `:1443`» — на HEAD это `beacon/actor.rs:1476-1488` и `:1489`; `epoch_is_actionable` «`:1858-1864`» — на HEAD `:1923-1929`; `beacon_member` «`:1878-1880`» — `:1943-1945`; `assemble_tracked_peers` «`:739-760`» — `:767-790`; `check_peer_set_size` «`:760`» — `:788`; `deque_size` стенда «`stand.rs:2342`» — `:2367`. Свойства при этом держатся (я их прочитал), но ни один из этих `[KNOWN]`-якорей не разрешается на HEAD.
- **Подтверждено, а не опровергнуто:** Д-124 (потребитель backup удалён — проверил: `node/dpos.rs:1079-1088`), Д-122 (ET через reader — `epoch_transition.rs:729-734`), Д-123 (два источника классификации — `GatedReceiver` по `TrackedWindow`, `beacon_member` по `committee_for`), числа ворот `a3v` (§4), clippy 2 / doc 6 / fmt 0.

---

## 4. Стенд и ворота

### 4.1 Три мутации (verbatim, по одной на строку)

Каждая: `md5sum` до, правка, прогон, откат копией, `md5sum` после, `git status --short`.

**М1 — строка 4.1, окно модуля.** `crates/dpos/consensus/src/committee/store.rs:438` — предикат окна обезврежен:

~~~
-        if epoch < lo || epoch > hi {
+        if false && (epoch < lo || epoch > hi) {
~~~

md5 до `c1953bf72b58565deaae8d83ec2044e6` → после мутации `2cdca272646951f994c4e85073598d3e` → после отката `c1953bf72b58565deaae8d83ec2044e6`.

~~~
test committee::tests::the_facade_folds_every_committee_error_to_none ... FAILED
test committee::tests::a_persisted_finalized_tag_anchors_the_window_before_the_cursor_is_seeded ... FAILED
test committee::tests::an_epoch_above_the_window_is_worth_a_retry_and_one_below_never_is ... FAILED
test committee::tests::the_map_keeps_every_epoch_the_window_still_admits ... FAILED
test committee::tests::an_epoch_outside_the_window_is_refused_without_a_single_read_and_the_borders_are_inside ... FAILED
test result: FAILED. 20 passed; 5 failed; 0 ignored; 0 measured; 660 filtered out; finished in 0.01s
~~~

Окно — самый несущий ассерт 4.1 (на нём держится необязательность `weights`), и он различает включённый и выключенный предикат пятью тестами.

**М2 — строка 4.2, `deliver`.** `crates/dpos/consensus/src/plane_upstream.rs:419` — снят бинд высоты (шаг 2):

~~~
-            if height != asked {
+            if false && height != asked {
~~~

md5 до `6232a651ba698ab2a91906c5fec08829` → после `db72516987038af78b2e14940ae7a345` → после отката `6232a651ba698ab2a91906c5fec08829`.

~~~
---- plane_upstream::tests::a_foreign_height_under_a_by_height_key_is_a_lie stdout ----
thread '…::a_foreign_height_under_a_by_height_key_is_a_lie' (1634848) panicked at crates/dpos/consensus/src/plane_upstream.rs:1094:13:
a foreign height must be refused as a lie
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 684 filtered out; finished in 0.01s
~~~

(В полном прогоне `plane_upstream::tests`: `12 passed; 1 failed`.) Различает, но **ровно одним** юнитом — стенд R-009 на эту мутацию не отвечает.

**М3 — строка 4.3, тир `Ingress`.** `crates/dpos/consensus/src/dpos.rs:118` — реестровый тир допущен на комитетский канал:

~~~
-            fluentbase_p2p::Ingress::Tracked(_) => !self.members_only,
+            fluentbase_p2p::Ingress::Tracked(_) => true,
~~~

md5 до `361b8da8a42cc388c81d6a238274ba47` → после `0850a872a14ed8e188d99dc5fc4d7184` → после отката `361b8da8a42cc388c81d6a238274ba47`.

~~~
thread '…::a_committee_channel_admits_only_members_and_a_registry_channel_admits_the_tier' (1642081)
panicked at crates/dpos/consensus/src/dpos.rs:5047:9:
assertion `left == right` failed: a committee channel must admit the member and nobody else: the registry tier, the untracked peer and the tombstoned member all stop before `recv` returns
  left: [478b8e50…5266, 5925ba86…428c]
 right: [478b8e50…5266]
test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 683 filtered out; finished in 0.00s
~~~

Компилятор при этом ещё и сказал `warning: field members_only is never read` — мутация наблюдаема дважды.

Дерево после всех трёх: `git status --short` = ` M .dpos-study/history/E4-ORCHESTRATOR.md` (чужой файл), `git diff --stat` = 1 файл / +2.

### 4.2 Числа ворот: `PLAN.md` §1 против `a3v-*`

`a3v-status.txt` [KNOWN, прочитан]: `lib/stand/stand-nofeat/node/reader/p2p/slasher/fmt/clippy` все exit 0; `clippy exit 0 warnings 5`, `clippy-feat 0`, `doc unresolved 6`, `DONE`.

| ворота | `PLAN.md` §1 (`:34-37`) | `a3v-*` (финальное дерево 4.3) | сходится |
|---|---|---|---|
| `fluentbase-consensus` lib | **679**/0 «на `48bf62ed`» | **685** passed; 0 failed | **нет** (+6) |
| стенд с фичей | **46**/0 | **47** passed; 0 failed | **нет** (+1) |
| стенд без фичи | **38**/0 | **39** passed; 0 failed | **нет** (+1) |
| `fluentbase-node` | **55**/0 | **55** passed; 0 failed | да |
| `fluentbase-staking-reader` | **63**/0 | **63** passed; 0 failed | да (и мой прогон — 63) |
| `slasher_integration` | 16/0 | 16 passed; 0 failed | да |
| clippy | «одна чужая `large_enum_variant`, с фичей 0» | **2** различных предупреждения (`large_enum_variant` в node + `MutexGuard` across await в reader), с фичей 0 | **нет** |
| `cargo doc --no-deps` | unresolved 6 | 6 (`a3v-doclinks.txt` — шесть строк) | да |
| `cargo fmt --check` | — | `Diff in` = 0 | — |
| `fluentbase-p2p` | «зелёные», 09-09 | 32/0 + 1/0 (3 ignored) | числа в PLAN нет; мой прогон — те же 32+1 |

Расхождения объяснимы и помечены: числа `PLAN.md` §1 явно подписаны «на `48bf62ed`» и совпадают байт-в-байт с квитанцией ворот v1v (`E4-ORCHESTRATOR.md:208`), а коммит закрытия 4.3 прямо говорит «PLAN §1 unchanged» (`git show fca897c1`). То есть это осознанная несинхронизация, а не ошибка; но читатель `PLAN.md` §1 сегодня получает ворота не того HEAD'а, а строка про clippy стала неверной (reader вошёл в набор и принёс второе предупреждение).

---

## 5. Дрейф: удалённые символы, всё ещё описанные как живые

Проверено grep'ом по `.claude/dpos_architecture/`, `.dpos-study/{PLAN,REGISTER,EXPERIMENTS}.md`, `E4-CORE-DESIGN.md` [KNOWN].

- **`.claude/dpos_architecture/` — чисто по существу.** Все большие блоки про `cold_start_jump_self_heal`, `classify_jump_outcome`, `JumpDisposition`, `SyncReason::{AwaitingUpstream, AuthRotate}`, `BadTarget`/`AuthFailed` стоят под явными маркерами: `04_cold_start…:37-42` («DELETED with this row»), `:44-45` («[HISTORICAL, pre-Б2 record — everything below in this section describes the pre-engine jump that no longer exists]»), `:433-441` («[REVISED 2026-09-12, Э4 row 4.2-Б2] THIS WHOLE SECTION IS HISTORICAL»). `06_staking_layer.md:240-250`, `:372-380`, `:585-595` — `[REVISED]`/`[DELETED]` про `cold_start_register`, `register_soft_entered`, `soft_enter_span`, `qual_read_at`. `15_smoke_cases…:260-275`, `:318-328`, `:438-446` — `[REVISED 2026-09-12]`/`[4.2-Б1]` про `Scripted::BadTarget`, `upstream_frontier_series`, `soft_enter_committees`. `03_epoch_machinery.md:804` — «(HISTORICAL, pre-В.)» про `corroborate_frontier`. `13_invariants…:267-278` — `[2026-09-12, Э4 row 4.2-Б1]`. Непомеченных мест, где символ описан как ЖИВОЙ, я не нашёл; единственный остаток — `00a_errata_2026_08_10_full_audit.md:92`, `:162` (описание аудита 08-10 про `upstream_frontier`/`highest_observed_epoch`), но у файла историческая роль по имени и рядом стоит правка `:96` «**[2026-09-12,…]**».
- **`REGISTER.md` — чисто.** R-003 (`:54`), R-004 (`:65`), R-034/R-064 (`:398`, `:638`), R-075 (`:712`) имеют статусы «2026-09-12/09-11 (закрыта/смягчена)» с sha и механизмом. Новые R-126/R-127 заведены.
- **`EXPERIMENTS.md` — исторические строки не помечены.** `:96` («R-004 частично: раздутый `Latest` поднимает `upstream_frontier` …»), `:99` (три мутации Э3, одна из них — «снять BLS-`ensure!` в `verify_jump_authenticated`», которая как стадия больше не существует), `:146` («Ex-4 … немонотонность `upstream_frontier`») читаются как текущее состояние. Ex-4.2c (`:114`) поверх них говорит «`upstream_frontier` удалён», но сами строки не помечены REVISED/HISTORICAL. Это единственный документ с реальным дрейфом.
- **`PLAN.md`** — символы упоминаются только внутри описаний строк 4.1/4.2/4.3 как «удаляются», что верно.
- **`E4-CORE-DESIGN.md`** — это проект «как было и как станет»; упоминания удалённых символов там уместны по жанру, дрейфом не считаю.
- **В коде** остатки — только комментарии «как было» (`executor.rs:2527`, `:10300-10320`, `epoch_manager.rs:492-493`, `cold_start_jump.rs:23`, `:85`, `:683`, `:719`, `cert_inlet.rs:264`), плюс одна стале-строка в док-комментарии теста: `testbed/tests.rs:3715` описывает `JumpOutcome::AuthFailed`, варианта нет.

---

## 6. Что всплыло для Э5/Э6/Э7

- **Э5 5.1.** `CommitteeReadsFacade` — единственная реализация `beacon::CommitteeReads` (`committee/facade.rs:40-54`), но каждая проекция гейтится `read_at()?`, то есть неисполненный якорь прячет запись, которую модуль уже держит (записано в `06_staking_layer.md:376-380` как «Known residual»). Снятие трейта в 5.1 это закрывает.
- **Э5.** Д-123/R-127: два источника классификации на BEACON (`TrackedWindow` от ET vs `committee_for` от модуля) и Д-126/R-126 (`now+3`) — механизм шире, чем записано: гонятся не `last_height`, а `last_tracked_epoch` ET (эпоха ordering-финализированного блока, `consensus/dpos.rs:2241`) против часов beacon'а (`epoch_of(marshal tip)`, `beacon/actor.rs:1907-1929`). См. §9.3.
- **Э5/Э7.** Д-122: чтобы `TrackedPeers` собирались из записей модуля, нужна правка `Cargo.toml` (крейт `staking-reader` ниже `consensus`). Пока это не сделано, св. 1 на peer-set не распространяется.
- **Э7 (или раньше).** Нет отрицательного кэша у постоянного отказа модуля (`store.rs:76-88`) — два блокирующих staticcall'а на КАЖДЫЙ `Committee::scheme` такой эпохи, и они выполняются на задаче актора marshal'а (`outer.rs:287-289` → CW `marshal/core/actor.rs:1092`). B3-12 записан; после 4.2/4.3 у него появился горячий вызывающий.
- **Э7.** `VoteStore` без границы (R-014); байтовый лимит BROADCAST — граница библиотеки CW (`buffered::Config` несёт только `deque_size`), то есть либо апстрим, либо форк.
- **Э6 6.3.** Класс «`SyncFailure` фатален в `launch_follower`» остался (R-034 смягчена, не закрыта).
- **Новое для любого из трёх.** (а) Исчезла единственная граница размера РЕЕСТРА: `check_peer_set_size` теперь считает только `primary().len()` (`epoch_transition.rs:788`), а до этапа считал объединение реестра и комитетов (`1b11b61f:epoch_transition.rs:654`); commonware secondary не проверяет вовсе — я открыл чекаут: `p2p/.../tracker/actor.rs:155-164`, комментарий «Secondary peers are not checked here». (б) `deliver=false` на MARSHAL-резолвере (§9.2). (в) Вторая копия геометрии (§9.3).

---

## 7. Где моя проверка была слабее всего

- Я не гонял полный `lib` (запрет постановки) — числа 685/47/39 беру из `a3v-*` как чужое измерение, сверив только их внутреннюю непротиворечивость и три файла ворот (clippy/doc/fmt) построчно.
- `executor.rs` (12 тыс. строк) читал выборочно: `probe_frontier` (`:2146-2289`), `maybe_re_jump` (`:2505-2590`), `anchor_advanced`-сайты. Трёх-тирную логику derive/спекуляции не перечитывал — если 4.2-В что-то сломал там, я бы этого не увидел.
- Комитетский путь beacon'а (`actor.rs`, 8 тыс. строк) читал только вокруг `on_message`/`on_confirm`/`epoch_is_actionable`/`epocher`.
- §9.2 и §9.3 — механизмы, доказанные чтением кода узла И чекаута commonware, но НЕ воспроизведённые ни тестом, ни прогоном. Помечаю как [ГИПОТЕЗА по коду], а не как наблюдение.
- Живого прогона (девнет/харнесс) не делал вовсе — приёмки 4.2 (лестница, E4-28) и 4.3 (дозвон secondary) остаются открытыми, как и записано в `PLAN.md`.

## 8. Оставлено как есть

- Отклонения Д-1…Д-131: прочитал таблицу (`E4-ORCHESTRATOR.md:263-277`), ни одно не меняет П-1/П-4/П-5 по существу — согласен с оценкой оркестратора, кроме двух формулировок: §5.3 «primary из записей модуля» (Д-122) и §5.4 «⇒ `Fault::corruption`» (не отмечено отклонением вовсе).
- Тумбстоун-поллер как второй читатель комитета — по проекту (liveness-слой).
- `verify_jump_authenticated` как функция (не стадия) с двумя by-height вызывающими — по проекту.
- TOFU свежего follower'а вне трёх известных chain_id — по проекту (E4-05), с `warn!` и отказом на деплойнутой сети.

---

## 9. Три самых опасных пункта

1. **Постоянный отказ чтения комитета не производит `Fault` и не кэшируется — узел молча перестаёт участвовать, а операторская сигнализация — одна строка.** Механизм: `weights: None` в окне / неуникальные ключи / `AbiDecode` / revert / header-index inconsistency на материализованной высоте (`committee/store.rs:225-294`, `:467-477`, `executed.rs:65-70`) дают `CommitteeError::Read(permanent)`. `failed()` пишет `error!` ровно один раз на эпоху и возвращает ошибку (`store.rs:192-207`). Дальше НИ ОДИН потребитель не превращает её в отказ: фасад сворачивает в `None` (`facade.rs:47-54`), `epoch_manager` — `debug!` + `return` без постановки в `deferred_reconciles` (`epoch_manager.rs:1141-1157`), `deliver` — отброс с `true` (`plane_upstream.rs:445-449`, `:486-490`), inlet — отложение без ротации (`cert_inlet.rs:618-639`), слэшер — release charge (`slasher/actor.rs:712-725`). Проект требовал прямо противоположного: §5.4 «`Read(permanent)` ⇒ `Fault::corruption` — громко, узел стоит; форк контракта». Отказ читается одинаково всеми валидаторами (они читают один контракт), то есть это коррелированная тихая остановка целой сети под зелёным liveness-чеком. Плюс §5.4 обещало «повтора нет», а код намеренно не кэширует (`store.rs:76-88`) — повтор идёт на каждом вызове, включая два блокирующих staticcall'а на задаче актора marshal'а. **Что сделал бы:** в `epoch_manager` (и/или в `CommitteeStore` через явный «отравленный» слот) маршрутизировать `!is_transient()` для эпохи В ОКНЕ в `Fault::corruption`, как написано в §5.4, и заодно закэшировать отказ, чтобы горячий путь не повторял staticcall. Тест: `committee/tests.rs` уже строит `absent_weights_inside_the_window_are_permanent_and_cache_nothing` — к нему нужен потребительский тест «эпоха в окне отказала постоянно ⇒ узел встал громко», которого сейчас нет.

2. **Правило «`deliver == false` только на сигнале лжи» реализовано ТОЛЬКО на FRONTIER; на MARSHAL-резолвере — том канале, который двигает tip — «я не могу аутентифицировать эту эпоху» по-прежнему исключает честного пира навсегда.** [ГИПОТЕЗА по коду, все звенья прочитаны] Цепочка: `verify_delivered` берёт схему через `self.provider.scoped(*epoch)`, и при `None` просто `continue` — элемент остаётся `verified[idx] == false` (CW `consensus/src/marshal/core/actor.rs:1085-1100`), дальше `response.send_lossy(false)` (`:1109-1115`); `Handler::deliver` возвращает это наружу (CW `marshal/resolver/handler.rs:62-78`); резолвер на `false` делает `block!` + `fetcher.block(peer)` (CW `resolver/p2p/engine.rs:425-441`), а `fetcher.block` кладёт пира в `excluded`, который не очищается ничем (`resolver/p2p/fetcher.rs:509-517`). `scoped` = `Committee::scheme` (`outer.rs:287-289`), а он отдаёт `None` на КАЖДОМ отказе модуля: вне окна, ниже `commit_height`, постоянный отказ (п. 1), и на несобранной verify-схеме (`store.rs:517-524`, `:374-391`). То есть п. 1 не просто тихо гасит эпоху — он ещё и последовательно выбивает из by-height-догона каждого честного пира, который попробует помочь. На FRONTIER этот случай закрыт явно и с комментарием почему (`plane_upstream.rs:467-490`), на MARSHAL — нет, и §5.2/§5.4 про этот канал вообще не говорят. **Что сделал бы:** прежде чем что-то менять — поставить стенд-тест (фикстура «узел получает финализацию эпохи, схемы которой у него нет» + ассерт на `requests_created`/`excluded`), он решит, достижимо ли это вне контрактного форка; если достижимо — либо `CertProvider::scoped` не должен молчать, либо fluentbase нужен свой `Consumer`-адаптер поверх marshal-handler'а, который отличает «не могу проверить» от «ложь», как это уже сделано на фронтире.

3. **Peer-set стал единственным пропуском на BEACON/EVIDENCE, но собирается вторым читателем комитета и по второму экземпляру геометрии — и обе расхождения теперь наказываются.** Три звена: (а) `TrackedPeers` строятся `EpochTransition`'ом из СВОИХ чтений `epoch_committee_snapshot` (`epoch_transition.rs:615/643/805/865`, признано в `:729-734`), на якоре, отстающем от якоря модуля на K, без окна и без write-once — при этом именно эти записи дают маску `Ingress` (`p2p/lib.rs:376-392`) и решают, чьи кадры вообще декодируются (`consensus/dpos.rs:112-143`); (б) beacon считает «сейчас» по своим часам `epoch_of(marshal tip)` (`beacon/actor.rs:1907-1929`) и готов работать с `[now, now+2]`, тогда как primary — это `C[T−1..T+1]` при `T = last_tracked_epoch` (эпоха ordering-финализированного блока, `consensus/dpos.rs:2241`); когда `T` отстаёт (припаркованная граница, `Full` мост, догон), дилеры входящего комитета `now+2`, которых нет в трёх записях, классифицируются `Ingress::Tracked` и роняются `members_only`-шлюзом — до Э4 реестр был primary и они проходили. Это R-126, но записан он как «риск при разбросе `last_height`», а гонка шире; цена — незакрытая DKG-церемония, то есть эпоха без ключа; (в) геометрия процесса не одна: модуль/beacon/фронтир берут замороженную пару ET через `geometry_rx`, а executor и движки — вторую, прочитанную слоем самостоятельно (`consensus/dpos.rs:1745-1748` → `outer.rs:722`, `:1848`), при живых сеттерах `setEpochBlockInterval`/`setDposActivationBlock` в контракте; при расхождении шаг (3) `deliver` (`plane_upstream.rs:432-438`) начнёт считать честные ответы ложью и исключать пиров навсегда. Док модуля утверждает обратное буквально (`committee/mod.rs:388-398`). Побочно этап потерял границу размера реестра: `check_peer_set_size` теперь считает только primary (`epoch_transition.rs:788`) против объединения до этапа (`1b11b61f:epoch_transition.rs:654`), а commonware secondary не проверяет (чекаут `p2p/.../tracker/actor.rs:155-164`). **Что сделал бы:** (а) — перенести `TrackedPeers` на модуль (Д-122; это правка `Cargo.toml`, не архитектуры) или, до неё, пинануть тестом равенство «маска ET == записи модуля» на двух якорях; (б) — брать `now` для `epoch_is_actionable` из того же источника, что и `T` (или расширить primary до `C[T+2]` на время церемонии), с фикстурой «`T` отстаёт на эпоху ⇒ церемония `now+2` всё равно собирается»; (в) — довести П-10 (геометрия из одного источника) или, минимально, отдать слою ту же `geometry_rx` и снять `OriginEpocher` из `outer.rs`; и вернуть проверку размера для secondary.

---

## Прямые ответы

1. **Свойства 1–6 держатся?** Не все. св. 2 — **да** (единственный неаутентифицированный `sync_to` остался за предикатом «не деплойнутая сеть», `consensus/dpos.rs:2984` + `:1400-1409` + `node/dpos.rs:2364-2372`). св. 3 — **да** для BEACON/EVIDENCE/backup/FRONTIER (backup закрыт удалением потребителя), **частично** в целом: MARSHAL-вход в §5.3 не назван, байтового лимита тел нет (`consensus/dpos.rs:2558-2562`). св. 5 — **да**. св. 6 — **да**, с одной непомеченной подменой (§5.4 обещает `Fault::corruption`, которого нет). св. 1 — **частично**: один комитет по ЗНАЧЕНИЮ и одна таблица схем, но peer-set читает второй читатель (`epoch_transition.rs:729-734`, Д-122) и геометрия существует в двух независимо прочитанных копиях (`consensus/dpos.rs:1745-1748` против `geometry_rx`), вопреки `committee/mod.rs:388-398`. св. 4 — **частично**: класс «постоянный отказ» без исхода (§9.1).
2. **I1–I5?** I1 — частично (см. выше). I2 — **да**: `sync_to` три входа, два через проверенную архивную пару (`executor.rs:2558`, `cold_start_jump.rs:818`, вызывающие `consensus/dpos.rs:2421`, `:3180`), третий — операторский checkpoint (`:2960`); четвёртый (`:2984`) отказывает на деплойнутой сети. I3 — да/частично (см. св. 3). I4 — **частично**: выходы типизированы и пробуждение есть (`store.rs:609-647`), но permanent не имеет исхода и повторяется на каждом вызове. I5 — **частично**: один tracked-пир больше не уводит EL, не раздувает фронтир и не переводит узел в verify-only; зато честный пир может быть навсегда исключён с marshal-резолвера (§9.2), а отставание `last_tracked_epoch` роняет DKG-трафик входящего комитета (§9.3); `VoteStore` (R-014) и лестница без живого `committee(T+1)` (B2-01) остаются.
3. **Три опасных места.** (1) Постоянный отказ чтения комитета — `error!` один раз, `debug!` у потребителя, ни `Fault`, ни кэша (`store.rs:192-207`, `:76-88`, `facade.rs:47-54`, `epoch_manager.rs:1141-1157`), при том что §5.4 требует `Fault::corruption`. (2) `deliver == false` на MARSHAL-резолвере при неаутентифицируемой эпохе исключает честного пира навсегда (CW `marshal/core/actor.rs:1085-1115`, `resolver/p2p/engine.rs:425-441`, `fetcher.rs:509-517`) — правило §5.2 доведено только до FRONTIER. (3) Peer-set — единственный пропуск на комитетские каналы, но собран вторым читателем комитета и сверяется против второй копии геометрии; плюс потеряна граница размера реестра (`epoch_transition.rs:788` против `1b11b61f:…:654`).
4. **Сколько мест без квитанции и какие.** **8**: (1) R-05…R-14/R-16…R-18 закрыты релеем дважды (`E4-ORCHESTRATOR.md:113`, `:115`); (2) B1-07…B1-15 — то же (`:120`, `:122`); (3) «красный до правки» за весь Б2 не проверен, квитанция проверяет существование тестов, а не их падение (`:135`); (4) A2-05 REJECTED на релейном якоре (`:161`); (5) измерение под REJECT A2-01 не повторено (`:161`); (6) детерминизм ×3 — реляция (`:160`); (7) мутации фикс-прохода 4.3 не воспроизведены (`:224`); (8) §5 ревью R10 не открыт при принятых по нему решениях (`:222`).
5. **Сколько [KNOWN] опровергнуто.** **Один** опровергнут: `TrackedPeers::is_secondary_only()` `epoch_transition.rs:169` в квитанции `E4-ORCHESTRATOR.md:218` («[KNOWN — открыто мной]») — символа нет в дереве (`git grep` пуст; `git log -S` даёт только docs-коммит `fca897c1`), и тот же якорь ушёл в постановку ревьюеру. Плюс **шесть** `[KNOWN]`-якорей той же квитанции не разрешаются на HEAD (дрейф 20–65 строк: `actor.rs:1432-1442/1858-1864/1878-1880`, `epoch_transition.rs:739-760/760`, `stand.rs:2342`) — свойства при этом на месте, я их прочитал. Ни одного содержательного `[KNOWN]` в `REGISTER.md`/`EXPERIMENTS.md` опровергнуть не удалось.
6. **Мутации verbatim.** Три, по одной на строку, все различают включённую и выключенную проверку, все откачены байт-в-байт (md5 совпали): 4.1 — окно `store.rs:438` ⇒ `FAILED. 20 passed; 5 failed`; 4.2 — бинд высоты в `deliver` `plane_upstream.rs:419` ⇒ `FAILED. 12 passed; 1 failed` («a foreign height must be refused as a lie», `plane_upstream.rs:1094`); 4.3 — `Ingress::Tracked` на комитетском канале `dpos.rs:118` ⇒ `FAILED. 1 passed; 1 failed` (`dpos.rs:5047`). Слабое место — 4.2: единственный тест на весь бинд высоты.
7. **Числа ворот сходятся с `a3v`?** Частично. Совпадают: node 55, reader 63, `slasher_integration` 16, doc unresolved 6, fmt 0. Не совпадают: consensus lib 679 против **685**, стенд 46/38 против **47/39**, clippy «одна чужая» против **двух** (второе — `MutexGuard` в reader, который в набор ворот вошёл только на 4.3); p2p в `PLAN.md` числом не назван (a3v и мой прогон — 32/0 + 1/0). Расхождение объяснено и честно подписано: числа `PLAN.md:34-37` помечены «на `48bf62ed`» и совпадают с квитанцией v1v (`E4-ORCHESTRATOR.md:208`), а коммит `fca897c1` прямо говорит «PLAN §1 unchanged».
8. **«Этап закрыт как регрессия для П-1/П-4/П-5» — вердикт.** **Да, с оговорками.** П-4 закрыт полностью и буквально: один фронтир = marshal tip, `deliver` — единственная точка доверия, `sync_to` больше не получает неаутентифицированного хэша нигде, кроме явно названного devnet-TOFU. П-5 закрыт по механизму: два тира, `Ingress` до декода, штрафа нет, backup-потребитель удалён, `deque_size` 4/2 — и сверх П-5 сняты `pipeline_catchup_span` и корроборация. П-1 закрыт **наполовину по объёму**: модуль, окно, write-once, одна карта «запись+схема», один продюсер, один якорь и один курсор — да; но «один источник комитета» не распространён на peer-set (Д-122) и «одна геометрия на процесс» по коду неверна, хотя документ модуля её утверждает. Регрессией для П-1 этап является в той части, где есть тесты (25 юнитов модуля + стенд), и НЕ является для двух вещей, которые §5.4 обещал и которых в коде нет: исхода `Fault::corruption` на постоянном отказе и правила «`false` только на лжи» на marshal-канале. Обе — не переделка сделанного, а дописывание; ни одна не требует пересмотра П-1/П-4/П-5.
