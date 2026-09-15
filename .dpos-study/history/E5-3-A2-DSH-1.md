# Независимое ревью (раунд 1): заход 5.3-А2 — `Option`-швы → `Wiring<R>` + пять точечных правок

Предмет: `git diff HEAD` (HEAD `f71f4d52`), ровно 6 файлов под `crates/dpos/consensus/src/beacon/`
(`actor.rs`, `artifact.rs`, `ceremony.rs`, `outcome.rs`, `plane.rs`, `share_state.rs`). Cargo не запускался
(по постановке); всё ниже — чтение кода. Проверялись и заявления из `dsh-input-journal.md`.

Обозначения уверенности: **confirmed by code** — видел строку; **inferred** — вывод из прочитанного.
Ни один пункт не подпал под определение BLOCKER (production change вне пяти правок / подпись возобновляется
под `Conflict` / честный дилер отвергнут как `equivocator` / доля принята с чужим индексом). Найденные
дефекты — ниже.

## Таблица находок

| id | severity | file:lines | what is wrong | refutation attempt | confidence |
|---|---|---|---|---|---|
| D-01 | MODERATE | `actor.rs:2889-2912` (сверка `has_seat` :2904; источник evidence :1333-1341; буфер :3241-3258) | **E-10 обходится на start-race drain.** Буферизованные `Commitment`/`Share` сливаются в `c.handle` с единственной проверкой `c.has_seat(&from)`; `equivocator` из `on_message` (:3419-3429) здесь не повторяется. `enter` копирует `ceremony.equivocations()` в `slot.evidence` (:1333-1341), поэтому после рестарта с журналом, где уже есть `DealerEquivocation`, доказанный эквивокатор может быть потреблён на drain-пути. Сценарий: рестарт, `Commitment` дилера D для эпохи E приходит ДО первого тика (слот пуст, `is_bufferable` :3241 разрешает) → буфер; первый тик решает E, `recover` возобновляет `Dealing` и `enter` поднимает evidence из журнала; drain (:2889) отдаёт D-дилинг в `handle`, бана нет. | Возможно, evidence не может существовать при пустом слоте в момент буферизации — да (`is_bufferable` требует отсутствия слота), но после рестарта evidence приходит из журнала и появляется ровно к моменту drain; сценарий достижим. | confirmed by code |
| D-02 | MODERATE | `actor.rs:2919-2927` (ср. починку на живом пути :3445-3469) | **DB-08 не покрывает drain-путь.** `append_journal(epoch, step.journal)` при неудаче только `withhold_acks`; `ReceivedDealing` из слитых чужих дилингов не кладётся в `nondurable_dealings` и не ретраится, в отличие от `on_message` (:3459-3468). Это ровно тот класс, который DB-08 обязан чинить (доля peer-дилинга не перевыводима локально; проверка «церемония есть» в ретрае :2381 здесь как раз прошла бы). | Возможно, drain-путь всегда с `share_dir` из `Wiring` и потому надёжен — но это тот же диск и та же `journal_failures`; отказ достижим (bad dir), а тест :13801 проверяет только `on_message`-путь. | confirmed by code |
| D-03 | MINOR | `actor.rs:4232-4238` (`changed`/`share_dir`/`recorded_dkg_logs`/`confirms`), `:5066-5076` (`fresh_share_dir`), `:4222-4228` (`mem::forget`) | **`Wiring::standalone()` тихо меняет семантику фикстур.** HEAD-сайты имели `share_dir: None`, `recorded_dkg_logs: None`, пул не выставлен (диф `git show HEAD:actor.rs` :908-923 и :983-1110). Теперь каждый standalone-актор (а) реально пишет share/journal в `std::env::temp_dir()`-каталог, который никогда не удаляется, — тесты, проверявшие ветку «нет каталога ⇒ durable тривиально», идут по диску; (б) `recorded_dkg_logs` + `ConfirmPool` реальны, поэтому `Confirmations::mint` (`confirmations.rs:156-206`) на кворуме записанных логов действительно подписывает и рассылает `ShareConfirm` в симулированную сеть. Примеры затронутых: `restart_midwindow_recovers_via_journal` (:5086), `a_restart_reads_the_stored_artifact_back_into_the_actor` (:11480), новые :13451/:13548/:13661/:13715/:13801. | Все `beacon::`-тесты зелёные, и ни один ассерт явно не ослаблен; но ни на диск-ветку, ни на «изменился ли трафик фикстур» ассерта не добавлено — это собственный weakest point №1 исполнителя. | confirmed by code (эффект — inferred) |
| D-04 | MINOR | `log_store.rs:85,97,141`; `confirmations.rs:95,103`; вызов `actor.rs:1011` и `:1022-1023` | **Д-А2-2 оставляет внутренние `Option`-швы.** `DealerLogStore::new(.., share_dir: Option<PathBuf>)` и `Confirmations::{pool,recorded}: Option` за актором всегда `Some` (`Some(share_dir.clone())`, `set_recorded`/`set_pool`), т.е. это тот же класс, который этажом выше устранён; будущий вызывающий может снова передать `None` и потерять durability/минт без изменения типа. | Файлы не в списке записываемых, и оба `Option` всегда `Some` из актора; формально поведение не меняется. | confirmed by code |
| D-05 | NIT | `actor.rs:2644-2690` (комментарий «exclusive» :2682-2684) | **Обоснование F-03 логически неверно.** При `!all_held && !ready` выбранный `reason` — `BodyMissing`, и `clear_stall(QuorumMissing)` (:2689) снимает защёлку, хотя кворума тоже нет. Ущерба нет только потому, что `all_held`/`ready` монотонны (тело, один раз удержанное, не теряется), поэтому оба-ложны бывают лишь до первой `QuorumMissing`-защёлки. | Монотонность действительно делает снятие безвредным; комментарий «sub-conditions are exclusive» всё равно неверен как утверждение о предикатах. | confirmed by code |
| D-06 | NIT | `actor.rs:4222-4228` | **`mem::forget` забывает и Receiver.** В кортеже пять значений (`resolver_tx`, `pinned_tx`, `artifacts_tx`, `body_lost_tx`, `agreement_rx`); докстрокой :4209 сказано «четыре хендла». Из-за забытого `agreement_rx` `try_send` после заполнения ёмкости 1 даёт `Full`, а не `Closed` (первый `try_send` при этом УСПЕВАЕТ и ставит `announced`, в отличие от `None` в HEAD) — на ассерты не влияет (`:1941-1947` оба исхода debug). Утечка пяти хендлов на тестовый актор. | Тесты — не прод; различие Full/Closed нигде не ассертится. | confirmed by code |
| D-07 | NIT | `actor.rs:1592` (`apply_artifact`), `:4222` (забытые сендеры), `:4232` (`changed`), `outcome.rs:220` | **Пробелы покрытия, названные исполнителем и подтверждённые кодом:** (а) прямой `on_artifact`-путь F-01 не тестирован — тест :13451 идёт через `reconcile_with_store`; обе ветки зовут один `apply_artifact`, но контекст вызова другой; (б) parking `recv_or_park` на ЗАКРЫТЫХ `pinned_rx`/`artifacts_rx`/`body_lost_rx` не тестируется (сендеры забыты), тестируется только `resolver_rx`-break (`:7305`-era); (в) `standalone().changed = \|_\| None` (:4232) не зафиксирован ассертом об «эпоха ≠ 2 не решена». | (а) одна и та же функция; (б) в HEAD это тоже не тестировалось; (в) эквивалентность HEAD `None` — по построению. | confirmed by code |
| D-08 | NIT | `actor.rs:737-820`; `mod.rs:68` | **Новый `pub`.** `pub struct Wiring<R>` с 13 `pub`-полями. Модуль `actor` приватный и `Wiring` не реэкспортирован, т.е. «наружу модуля» ничего не ушло; `pub` нужен `plane.rs` (сосед по `beacon`). | Требование «никаких новых pub наружу модуля» выполнено по границе видимости. | confirmed by code |
| D-09 | NIT | `git show HEAD:actor.rs:983,993,1002,1015,1029,1079,1092,1110` | **Арифметика постановки.** «Девять `with_*`» — на HEAD восемь методов, покрывающих девять рёбер (`with_agreement_plane` — обе половины). На код не влияет. | Не влияет на корректность. | confirmed by code |
| D-10 | NIT | `actor.rs:6645` (и док :6617-6631) | **`standalone_actor_at` безусловно перезаписывает `wiring.changed`** правилом над `cf`, молча теряя `changed`, переданный через `standalone_actor_wired`. Сейчас таких вызывающих нет, но это ловушка при добавлении теста с собственным битом. | Задокументировано в доке хелпера; живых зависимостей нет. | confirmed by code |

## Ответы по вопросам

### 1. WIRING EQUIVALENCE

`Wiring` — `actor.rs:737-820`, 13 обязательных полей; `new` деструктурирует его целиком (`:979+`), ни одного
`if let Some(шов)`/`recv_or_never(None)` на путях `run`/тика не осталось (`recv_or_never` отсутствует по
`git grep`). Продакшн (`plane.rs:855-882`) и HEAD (`git show HEAD:plane.rs:859-884`):

| # | поле (`Wiring`) | HEAD (production) | new (`plane.rs`) | HEAD default в тестах | new default (`Wiring::standalone`/`inert`, `actor.rs:4216-4251`) |
|---|---|---|---|---|---|
| 1 | `resolver` | `Some(logs)` | `resolver: logs` (:868) | `None` ⇒ gossip-only, `fetch_missing_logs` early-return (`HEAD:3478`) | `NoopResolver`; `fetch_missing_logs` зовёт no-op `retain`/`fetch_targeted` (`:3637-3641`) — поведенчески то же |
| 2 | `resolver_rx` | `Some(log_resolver_rx)` | `:869` | `None` ⇒ park | канал, сендер забыт ⇒ park; закрытие даёт `ERROR + break` (`:1208-1216`) как в HEAD |
| 3 | `changed` | `.with_changed_bit(changed.clone())` | `:870` | `None` | `Arc::new(\|_e\| None)` (:4232) |
| 4 | `share_dir` | `Some(share_dir)` | `:871` | `None` | `fresh_share_dir("standalone")` (:4233) — **отличие тестов, D-03** |
| 5 | `plane_clock` | `.with_plane_clock(plane_clock)` | `:872` | `None` ⇒ gauge молчит | `PlaneClock::default()` (:4234) — незарегистрированные gauge, `record_dkg_clock` inert (`sync_metrics.rs:309-313`) |
| 6 | `outcome_at` | `Some(outcome_at)` | `:873` | `None` ⇒ `stored()==None` | `Arc::new(\|_e\| None)` (:4235) — эквивалентно |
| 7 | `pull_artifact` | `.with_artifact_pull(pull_artifact)` | `:874` | `None` ⇒ inert | no-op closure (:4236) |
| 8 | `recorded_dkg_logs` | `.with_recorded_logs(recorded)` | `:875` | `None` | свежий `Arc<RwLock<BTreeMap>>` (:4237) — **отличие тестов, D-03** |
| 9 | `confirms` | `.with_share_confirms(confirms)` | `:876` | без пула ⇒ `mint` инертен | `ConfirmPool::new(b"FLUENT_TEST_STANDALONE")` (:4238) — **отличие тестов, D-03** |
| 10 | `pinned_rx` | `.with_pinned_requests(pinned_rx)` | `:877` | `None` ⇒ park | канал, сендер забыт (:4218,4224) |
| 11 | `agreement_tx` | `.with_agreement_plane(agreement_request_tx, ..)` | `:878` | `None` ⇒ `announce` early-return | канал cap 1, RECEIVER забыт (:4221,4227) |
| 12 | `artifacts_rx` | `.with_agreement_plane(.., artifacts_rx)` | `:879` | `None` ⇒ park | канал, сендер забыт |
| 13 | `body_lost_rx` | `.with_body_lost(body_lost_rx)` | `:880` | `None` ⇒ park | канал, сендер забыт |

**Продакшн-эквивалентность:** все 13 значений — те же объекты, что и в HEAD (HEAD `plane.rs:859-884`);
`agreed_tx` перестал клонироваться только потому, что replay удалён (`new plane.rs:925`). Ни одно `None`
поведение HEAD в продакшне не воспроизводилось (HEAD всегда всё проводил), поэтому отличий продакшна, кроме
пяти правок, нет. Единственные тихие отличия — в тестах: `share_dir`, `recorded_dkg_logs`, `confirms`
(D-03); `resolver: None ⇒ NoopResolver`, `plane_clock: None ⇒ default`, `outcome_at: None ⇒ |_| None`,
`pull_artifact: None ⇒ no-op` эквивалентны. Тест, державший «отсутствие резолвера» как свойство, не найден
(`restart_midwindow_recovers_via_resolver` строился на `Some(NoopResolver)`), — согласен с журналом.

`recv_or_park` (`:655-659`) на закрытом plane-канале: парковка — то же, что HEAD делал через `None => rx = None`
(`HEAD:1238-1241` и др.), только без одного лишнего витка цикла; `resolver_rx` закрывается в `break`
(`:1208-1216`) — это смерть supervised-сиблинга (LogHandler), и это тоже сохранённая семантика Г1. Решение
последовательное. **Оговорка:** сама ветка «закрытый plane-канал» не тестируется (D-07б).

### 2. F-01 — `Conflict{key}`

- `needs_artifact()` включает `Conflict { key: None, .. }` (`:426-437`, строка :435) ⇒ `drive_acquisition`
  тянет и ставит `NoArtifact` (`:3708-3719`); при `key: Some` — не тянет.
- `apply_artifact` на `Conflict` возвращает `false` всегда, а при `key.is_none()` лишь переводит в
  `Conflict { held, second, key: Some(digest) }` (`:1606-1639`, установка :1629-1636): фаза не меняется,
  `Keyed`/`KeyOnly` не достигаются (все конструкторы `Conflict` — `:1839-1852`, `:3009`, `:3019-3023`;
  ни один не переходит из `Conflict` в `Keyed`).
- `reconcile_with_store` фильтрует ровно `Conflict { key: Some }` (`:3741`) и для keyless-`Conflict` зовёт
  `apply_artifact` (:3752-3754), не сравнивая (вердикт финален).
- `conflict()` ставит `key: Some(held)` (`:1845-1852`), и `held` — значение, которое актор уже держит или
  прочитал из стора; `recover` берёт ключ из стора (`:3008`), ветка `stored.divergent` — `key: Some(held)`
  (`:3019-3023`).
- Подпись остановлена: `drop_share`/`stop_signing` (`:1870-1903`) и `carries(Conflict)` (:516). Ветка
  `Conflict` в `apply_artifact` не зовёт `drive_finalization` (возврат `false`), `on_artifact` при `false`
  ничего не финализирует (`:1572-1575`).

Вывод: **подпись не возобновляется ни на одном пути**; после появления ключа `needs_artifact()` перестаёт
спрашивать (тест :13451 подтверждает `asked.len()==1` и пустой store/share-файлы). Блокера нет.

### 3. E-10

- Отвергаются `Commitment | Share | Ack` от `from`, если `epochs[epoch].evidence.contains_key(&from)`
  (`:3419-3429`), с `refuse(&from, Some(epoch), "equivocator")`. Он-чейн ничего.
- Источник evidence — `EpochSlot.evidence` (`:450`), заполняемый `note_equivocation` из пары церемонии
  (`:2571-2597`) и `enter` из `ceremony.equivocations()` (`:1333-1341`), т.е. это механизм Б/А1, а не
  `Conflict{evidence}` (последнего в коде нет).
- Честный дилер отвергнут быть не может: пара создаётся только `handle`-ом по двум валидным логам с
  проверенной подписью (`note_equivocation` :2575-2584); пара привязана к эпохе (`epochs.get(&epoch)`);
  после sweep слота evidence исчезает вместе со слотом (`:2113-2119`), на другую эпоху не влияет.
- **Но:** отверждение не покрывает start-race drain (D-01) — достижимо; и `Confirm` от эквивокатора не
  банится (вне заявленного списка Commitment/Share/Ack).
- Liveness: церемония может не набрать дилинги банированного, но recompute из pinned-логов
  (`try_recompute` → `recompute_scoped`, `:3827-3874`) восстанавливает долю без Player-дилинга, поэтому
  seal/quorum не блокируется окончательно. Принятый трейд; отдельного теста на это нет.

### 4. `validate_share_on_poly(outcome, committee, me, share)`

- `outcome.rs:106-123`: после формы — `committee.position(me) != Some(usize::from(my_share.index))` ⇒ `false`
  (:116). Тип `Share.index` — `commonware_utils::Participant(u32)` (`utils/src/lib.rs:53`) с
  `impl From<Participant> for usize { p.0 as Self }` (`:76-79`), а `Set::position` даёт `usize`
  (`utils/src/ordered.rs:83`); на 32/64-битных платформах усечения нет (`Participant::from_usize` паникует
  свыше `u32::MAX`, но здесь обратное направление).
- Все вызывающие передают **ключ узла, а не отправителя**: `share_on_artifact` → `&self.me_key.public_key()`
  (`:1786`); тесты — владелец перебираемой доли (`outcome.rs:198,203,212,283,287,344`), `ceremony.rs:2403`
  (`&key0.public_key()`, владелец `recomputed_share`).
- Единственный продовый путь принятия доли — `adopt_share` → `share_on_artifact` (:2018), `store.insert`
  только после гейта (:2051-2055). «Доля принята с чужим индексом» невозможна; блокера нет.

### 5. Удаление `restart_replay`

Что делает рестарт с артефактом на диске теперь, по шагам:
1. `plane.rs` больше не читает стор на старте (удалены выборка :759-771 HEAD и push :927-945 HEAD);
   `ArtifactStore::epochs` переведён в `#[cfg(test)]` (`artifact.rs:671`), `share_state::journal_epochs`
   удалён.
2. Первый `on_height` (тика из буфера `heights`, `plane.rs:823,841`) зовёт `decide_window` → `recover(E)`:
   `stored = self.stored(E)` (`:2985`) читает стор, `load_journal` — журнал; при `share_held=false` и
   `mints` артефакт становится `agreed`, а состояние — `Agreed` при закрытом дилинге (`:3122-3129`).
3. Дальше `reconcile_with_store` перечитывает стор каждый тик (`:3737-3774`) и до-применяет потерянный
   hand-off.
4. Rewritten-тест `a_restart_reads_the_stored_artifact_back_into_the_actor` (`:11480`): журнал минус одно
   тело + артефакт в сторе ⇒ первый тик даёт `agreed` при пустом `ceremony_store` (финализация ждёт тело),
   приход тела ⇒ `keyed` и доля в сторе, следующий тик ⇒ по-прежнему `keyed`. Это ровно свойство
   store-владельца; фальсификатор (`recover` игнорирует стор) тест красит.
Регресса не вижу: HEAD-replay всё равно отбрасывался `on_artifact`-guard-ом до первого тика
(`on_artifact`: `last_height.is_none()` ⇒ return, :1555-1563), а A1-`reconcile_with_store` уже покрывал
этот случай; удаление лишь убирает дублирующий push.

### 6. DB-08 / F-02 / F-03

- **DB-08.** `journal_failures` (`:1144-1160`) отдаёт нелегшие записи; `on_message` кладёт
  `ReceivedDealing` в `nondurable_dealings` (`:3445-3469`, очередь :928); `retry_nondurable_journals`
  (`:2376-2390`) переписывает их из записи и оставляет нелегшие, sweep `retain` — `:2133`; Ack остаётся
  удержанным. Lifecycle верен. **Пробел:** drain-путь не кладёт записи (D-02).
- **F-02.** `EpochSlot.body_lost` (`:452-458`), `on_body_lost` запоминает на `Dealing{..}` (`:1472-1486`),
  шаг печати `on_height` применяет при `agreed: None` (`:2218-2242`), pull — тем же тиком в
  `drive_acquisition` (:2312, :3718). При `agreed: Some` флаг не читается (печать идёт в `Agreed`) — применение
  ровно один раз, без повторной печати. **Д-А2-5: исполнитель прав** — при `agreed: Some` артефакт уже
  есть, «применить body-lost» значило бы форсировать acquisition поверх известного набора; постановка
  «`Dealing{agreed: Some}`» — описка.
- **F-03.** `clear_stall` (`:1456-1463`) снимает «прошедшую» половину пары в `drive_finalization`
  (`:2682-2690`). Оговорка: пара не взаимоисключающая (D-05), но монотонность предикатов делает снятие
  безвредным; gauge не завышается.

### 7. Тесты и мутации

- 6 новых тестов и 1 переименованный присутствуют: `actor.rs:13451,13548,13661,13715,13801,11480`,
  `outcome.rs:220`.
- Мутации M2–M6 по коду действительно убивают соответствующие тесты: M2 ломает `asked` в :13514/:13489; M3 —
  счётчик `equivocator` :13610; M4 — последний assert `outcome.rs:248`; M5 — множество защёлок :13765; M6 —
  `phase`/`asked` :13675. M1 — компиляционная ошибка (`Option` vs `Arc<dyn Fn>`) в `drive_acquisition`.
  Зелёными при сломанном свойстве они быть не могут: каждый ассертит именно артефакт свойства, а не
  промежуточный факт.
- Слабое место тестов E-10: drain-путь не покрыт (D-01), DB-08 — только `on_message` (D-02), F-01 — только
  store-ветка (D-07а).
- Переписывание 15 конструкторов + 15 builder-цепочек через 4 хелпера: семантика фикстур изменилась ровно в
  трёх местах — `share_dir`, `recorded_dkg_logs`, `confirms` (D-03). `outcome_at: None` и `changed`-дефолт
  эквивалентны HEAD по построению; тестов, полагавшихся на `share_dir: None`/`outcome_at: None` как на
  проверяемое свойство, не нашёл (все проверки диска/артефакта шли через явные `Some(dir)`/`store_reader`).

### 8. Гигиена

- Новых `#[allow]` нет: единственный `#[allow(clippy::too_many_arguments)]` на `new` (`:978`) — существующий
  (в HEAD он же). Новых `unwrap`/`expect` на продакшн-пути нет: все добавленные `expect` — в тестах
  (`a2-actor.diff`, строки 2155-2913); наоборот, удалён prod-`expect("checked Some above")` в
  `fetch_missing_logs` (`HEAD:3569`).
- Новый `pub` — только `Wiring` (D-08), внутри приватного `mod actor` (`mod.rs:68`), не реэкспортирован.
- Мёртвого кода в рамках захода не осталось: `recv_or_never` и `with_*` отсутствуют, `journal_epochs` удалён,
  `ArtifactStore::epochs` гейтнут `#[cfg(test)]` (все читатели — `artifact.rs`/`surface.rs`/`cert_inlet.rs`
  под `cfg(test)`; `cert_inlet.rs` использует его внутри тест-модуля, начатого на :885).
- `mem::forget` — только тестовый `Wiring::inert` (Д-А2-7, D-06).
- Замечание: `confirmations.rs:95,103` и `log_store.rs:85` оставляют `Option` (Д-А2-2, D-04).

### 9. Где ревью слабее всего (по убыванию)

1. **Динамика.** Cargo не запускался: mutate-красные и зелёные прогоны беру из журнала; сам убедился только
   в логической неизбежности (M2–M6) и в том, что ассерты метят именно свойство.
2. **Фикстурный трафик D-03.** Не могу перечислить все тесты, чьи наблюдения изменились от реального
   `ShareConfirm`-минта; анализировал только те, что читают `pool.covering`/счётчики.
3. **D-01/D-02 (drain-путь).** Достижимость D-01 вывел из комбинации `is_bufferable`/`enter`; не проверял,
   что `DealerEquivocation` из журнала действительно переживает `resume` в `DkgCeremony` (проверял по
   `enter`-коду и по тесту :13548, а не по `ceremony.rs`).
4. **Внешние крейты.** Тип `Participant`/`Set::position` смотрел в `~/.cargo/git/checkouts`; не проверял
   иные реализации/фичи.
5. **Доки/CHANGELOG.** Утверждения журнала об отсутствии живых `gossip-only`/builder-упоминаний в доках
   проверял только `git grep` по `crates/`+`bins/`; `.claude/`-доки не открывал (вне диффа).

## Leave as is

1. `recv_or_park` vs `break` для plane-каналов (`:655-659`, :1221-1235) — оставить: это сохранённая
   семантика HEAD, смерти отправителей — забота супервизора, а `break` убил бы актор на локальном закрытии
   канала.
2. Д-А2-2 (`log_store.rs:85`, `confirmations.rs:95,103`) — оставить: `Option` всегда `Some` за актором,
   файлы вне списка; отдельная правка.
3. Д-А2-3 (`standalone().changed = |_| None`, :4232) — оставить: эквивалентно HEAD-дефолту `changed: None`;
   тест-ассерт на «эпоха ≠ 2 не решена» — полезное, но не блокирующее улучшение.
4. Д-А2-4 (F-03 через `clear_stall`, а не `carries`) — оставить: `carries` видит только фазу, а оба
   условия — под-условия `Agreed`; текущая форма снимает ровно «прошедшую» защёлку.
5. Д-А2-5 (F-02 для `Dealing{agreed: None}`) — оставить: при `agreed: Some` применять body-lost некуда.
6. Д-А2-6 (ключом `Conflict` становится любой quorum-заверенный артефакт, включая «третий» и `Malformed`) —
   оставить: цель — `PK_E` для верификации, арбитраж между тремя сертифицированными значениями
   невозможен.
7. Д-А2-7 (`mem::forget` в `Wiring::inert`, :4222) — оставить как тестовый приём; учёл бы в доке, что
   забыто пять значений, а не четыре (D-06).
8. Д-А2-8 (закрытые plane-каналы паркуются) — оставить (см. п.1).
9. Д-А2-10 (`ArtifactStore::epochs` → `#[cfg(test)]`) — оставить: продакшн-читателей нет, `cert_inlet`-
   использование внутри тест-модуля.
10. `#[allow(clippy::too_many_arguments)]` на `new` (`:978`) — оставить: существующий, 12 параметров.
11. `Conflict` на не-членских терминалах (`KeyOnly`/`SatOut`/`Unrecoverable`) — не расширять в этом заходе.
12. `share_state::persist_conflict` без tmp+rename (А1 §3) и порядок marker-first (F-01 глубже: писать
    маркер ПОСЛЕ sync артефакта) — не трогать в этом заходе.
