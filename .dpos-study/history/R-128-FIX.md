# R-128 — постоянный отказ чтения комитета: классификация, отравление слота, fail-stop

Ветка `djadjka/dpos-reth-2.2-squashed`. Стартовый HEAD `27485df6`; к моменту коммита
другая сессия положила сверху `5240ed08` и `02a0e535` (только `.dpos-study`), так что
код лёг на `02a0e535`.

Решение владельца 2026-09-12: поведение как у классических блокчейнов — fail-stop.
Реализована строка 555 проекта (`history/E4-CORE-DESIGN.md` §5.4), строка 554 (ревёрт)
оставлена как была.

## §0 Прямые ответы

1. **sha.** Код и тесты — `26609cff` (`fix(consensus)!: halt the node when the contract
   answers an impossible committee`); документы — коммит, несущий этот
   журнал (`docs(dpos): close R-128`; его sha нельзя записать внутрь него самого). Ворота — §1 ниже, verbatim до и после.
2. **Таблица категорий.** 14 вариантов `ReadError`: 3 транзиентных, 2 «ревёрт/локальное»
   (`Permanent`), 9 «невозможное» (`Impossible`). Спорных два, оба названы в §3:
   `Backend` (постоянный, но это СВОЁ хранилище узла, а не заявление цепи — отнесён к
   `Permanent`, узел не встаёт) и `ZeroEpochInterval` (ответ контракта, который не может
   быть легальным — `Impossible`, но через модуль комитета недостижим: `Geometry::new`
   отказывает на нулевом интервале до всякого чтения).
3. **Ex-4.1e.** Красный на HEAD — **да**, verbatim первой упавшей строки:
   `not every node halted: halted=[] heights=[63, 63, 63, 63]`
   (`committee_tests.rs:679`, прогон на непра́вленом проде). После правки пинит:
   маркер halt'а (`out.halted` — четыре узла, причина `ContractFork`), замирание
   marshal-tip'а (последние 10 отсчётов серии равны), ровно один снимок контракта на
   эпоху (`== 1`), отсутствие записи и схемы, ровно 3n строк ERROR (отказ модуля, halt
   менеджера, парковка исполнителя) и живая цепь до конца эпохи 1 по ordering-плоскости.
4. **Тест (б), ревёрт.** `FakeStaking` НЕ умел — добавлен режим `reverts_for`
   (`StandConfig::reverts_for` → `FakeStaking::with_revert`, счётчик
   `StakingReads::reverted`). «Был ли красным»: **нет, честно** — на непра́вленом проде он
   зелёный, потому что до правки НИКАКОЙ постоянный отказ не ронял узел. Его квитанция —
   мутация: заменил условие halt'а `e.is_contract_impossible()` на `!e.is_transient()`,
   прогнал только этот тест — красный
   (`heights [62, 62, 62, 63]` на `!out.timed_out`), мутация откачена, тест снова зелёный.
   То есть тест различает ИМЕННО разделение, а не наличие halt'а вообще.
5. **Число staticcall'ов после отравления — `==`.** `module_snapshot[2] == 1` на каждом
   узле, и это прошло с первого прогона. Недетерминизм Д-64 (дрожание 4/5) снят самим
   отравлением: гонка «два потребителя промахнулись мимо карты» в стенде невозможна —
   `committee()` синхронна целиком, детерминированный рантайм не переключает задачи внутри
   неё, а первый же отказ кладёт вердикт в карту.
6. **Что делает менеджер после `engage`** — по коду: плечо `_ = &mut halt_n =>`
   (`epoch_manager.rs:778`) снимает все движки (`:780`), все инстансы согласования ключа
   (`:790`), чистит `deferred_spawns` (`:794`) и переводит все роли в `Verifier` (`:796`).
   Сверх этого не потребовалось ничего: `engage` вызывается из `reconcile_roles`, то есть
   из другого плеча того же `select!`, а `Notify::notify_one` хранит пермит, поэтому
   следующая итерация цикла плечо получает.
7. **Что оказалось неверным по коду.** (а) R-128 говорит «`CommitteeStore::failed` …
   отказ намеренно не кэшируется (`:76-88`)» — это доксрока поля `reported`, а не код
   кэша; кода, который бы отказ кэшировал, не было вовсе, так что «намеренно» держалось
   только на комментарии (владелец это и отверг). (б) Проект §5.4 строка 555 перечисляет
   «`AbiDecode` / порядок / дубликаты / пол», но НИ ОДИН из вариантов
   `CommitteeOutOfOrder` / `CommitteeDuplicatePeerKey` / `CommitteeTooSmall` модулем не
   производится: reader ловит их раньше, а сам модуль строит для своих трёх случаев
   (`weights: None`, длина весов, неуникальные ключи) `ReadError::AbiDecode`. Классификация
   их всё равно покрывает — но «пол `MIN_COMMITTEE_LENGTH`» в модуле не живёт.
   (в) Строка 556 проекта («повторное чтение с другим значением ⇒ `error!` + отказ эпохи»)
   в коде до правки давала отказ ТОЛЬКО второму читателю, а следующий `committee(E)`
   отвечал сохранённой первой записью — «отказа эпохи» не было; теперь есть (Д-135).
8. **Что принял без чтения кода.** Три вещи. (а) Что `SyncReason` нигде не сериализуется
   в чужой формат, кроме маркера datadir и метрики, — читал `as_str`/`from_label`/
   `persist_marker`, но не искал внешних потребителей лейбла (дашборды, алерты вне репо).
   (б) Что «marshal продолжает раздавать» после halt'а — из доки `SafetyHalt` и из
   поведения стенда (tip замер, узлы живы), кода раздачи marshal'а под halt'ом не читал.
   (в) Что коммит `26609cff` не ломает e2e/devnet — их не гонял (см. §6).
9. **Чем читал длинные строки.** `sed -n 'A,Bp'` и `cat -n`; `grep -n` для поиска якорей.
   `cut -c` / `head -c` не применял.

## §1 Ворота

### До (HEAD `27485df6`, чистое дерево)

```
cargo test -p fluentbase-consensus --lib
test result: ok. 685 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 169.54s

cargo test -p fluentbase-consensus --lib --features dpos-devnet-byzantine testbed::
test result: ok. 47 passed; 0 failed; 0 ignored; 0 measured; 646 filtered out; finished in 215.32s

cargo test -p fluentbase-node --lib
test result: ok. 55 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 57.11s

cargo test -p fluentbase-staking-reader
test result: ok. 63 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
test result: ok. 0 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s

cargo clippy -p fluentbase-consensus -p fluentbase-node -p fluentbase-staking-reader --all-targets
warning: this `MutexGuard` is held across an await point
    --> crates/dpos/staking-reader/src/epoch_transition.rs:3017:17
warning: large size difference between variants
    --> crates/node/src/dpos.rs:1989:1
(две штуки, обе чужие)

cargo clippy -p fluentbase-consensus --all-targets --features dpos-devnet-byzantine
(ни одного warning)

cargo fmt --check
(ни одной строки "Diff in"; вывод — только предупреждения о nightly-опциях rustfmt.toml)

cargo doc -p fluentbase-consensus --no-deps 2>&1 | grep -c "unresolved link"
6
```

### После (`26609cff`, то же дерево после `cargo fmt -p fluentbase-consensus`)

```
cargo test -p fluentbase-consensus --lib
test result: ok. 686 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 194.70s

cargo test -p fluentbase-consensus --lib --features dpos-devnet-byzantine testbed::
test result: ok. 48 passed; 0 failed; 0 ignored; 0 measured; 646 filtered out; finished in 240.16s

cargo test -p fluentbase-node --lib
test result: ok. 55 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 58.83s

cargo test -p fluentbase-staking-reader
test result: ok. 64 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
test result: ok. 0 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s

cargo clippy -p fluentbase-consensus -p fluentbase-node -p fluentbase-staking-reader --all-targets
warning: this `MutexGuard` is held across an await point
    --> crates/dpos/staking-reader/src/epoch_transition.rs:3017:17
warning: large size difference between variants
    --> crates/node/src/dpos.rs:1989:1
(те же две, роста нет)

cargo clippy -p fluentbase-consensus --all-targets --features dpos-devnet-byzantine
(ни одного warning)

cargo fmt --check
(ни одной строки "Diff in")

cargo doc -p fluentbase-consensus --no-deps 2>&1 | grep -c "unresolved link"
6
```

Дельты: `+1` юнит в `fluentbase-consensus` (`a_reverting_committee_read…` в
`committee_tests` виден и без фичи), `+1` стенд-тест под фичей, `+1` юнит в
`fluentbase-staking-reader` (классификация). Чужие два clippy-предупреждения на тех же
якорях, doc-unresolved не вырос.

Порядок прогонов: ворота «до» сняты на чистом HEAD (правки на это время были вынуты из
дерева и возвращены `git show HEAD:<path>`); «красный до правки» для Ex-4.1e снят
на дереве, где переписаны ТОЛЬКО тесты и фикстура; ворота «после» — дважды, потому что
`cargo fmt` тронул два места уже после первого прогона.

## §2 Что сделано

### 2.1 Классификация — `crates/dpos/staking-reader/src/error.rs`

`ReadError::class()` (`:202`) — один исчерпывающий `match` на три класса
(`ReadClass`, `:51`); `is_transient()` (`:231`) и новый `is_contract_impossible()`
(`:245`) — виды на него, поэтому «ровно одна категория» есть свойство типа, а не
утверждение теста (Д-132). `ReadClass` ре-экспортирован из корня крейта.
`ReadError` получил `Clone` — вердикт надо хранить в сторе.

| Вариант `ReadError` | Категория | Почему |
|---|---|---|
| `StateNotMaterialized` | транзиентный | состояние до-материализуется само |
| `TransientStorage` | транзиентный | рваное чтение static-file, устаканивается |
| `BlockNotFound` | транзиентный | заголовок ещё может приехать |
| `CallReverted` | ревёрт (`Permanent`) | контракт ОТКАЗАЛСЯ отвечать; чинится оператором |
| `Backend` | ревёрт (`Permanent`) | своё хранилище узла, не заявление цепи (спорный — §3 Д-134) |
| `AbiDecode` | невозможное | байты не разбираются; модуль строит его же для `weights: None`, длины весов, неуникальных ключей и повторного чтения с другим значением |
| `BlsKey` | невозможное | ключ не в подгруппе |
| `PeerKey` | невозможное | не 32-байтный ed25519 |
| `CommitteeMemberKeyless` | невозможное | он-чейн инвариант нарушен |
| `PeerSetTooLarge` | невозможное | комитеты больше предела |
| `CommitteeOutOfOrder` | невозможное | нарушен порядок = индексное пространство консенсуса |
| `CommitteeDuplicatePeerKey` | невозможное | нарушена он-чейн уникальность |
| `CommitteeTooSmall` | невозможное | ниже пола, который `commitEpochCommittee` ревёртит |
| `ZeroEpochInterval` | невозможное | интервал 0 нелегален (через модуль недостижим — §3 Д-134) |

Итог: 3 / 2 / 9.

### 2.2 Отравление — `crates/dpos/consensus/src/committee/store.rs`

`Poison { error, reason }` (`:84`), карта `State::poisoned` (`:126`). Пишется в двух
местах: `failed()` (`:244`, ветка `class == Impossible`, `:251`) и форк-плечо `install()`
(`:425`). Читается в `committee()` шагом 3 (`:567`) — ПЕРЕД записью, поэтому форкнутая
эпоха перестаёт отвечать первой записью (Д-135). Ответ из отравленного слота: тот же
`CommitteeError::Read(err)`, счётчик `dpos_committee_read_permanent_total{reason}`
тикает как раньше, `error!` НЕ повторяется (его гасит прежний `reported`). Пруним тем же
полом окна, что и записи (`prune`, третья строка). Ревёрт и `Backend` не отравляются —
починенный модуль должен читаться без рестарта.

Сообщение `error!` разведено по классам с общим префиксом
`committee read failed PERMANENTLY inside the read window — …` (оператор грепает префикс).

### 2.3 Halt — `crates/dpos/consensus/src/epoch_manager.rs`

`reconcile_roles`, ветка `Err(e)`: транзиентная — как была (`deferred_reconciles` +
`debug!`); `e.is_contract_impossible()` (`:1185`) ⇒ `error!` с эпохой, ошибкой и
`reason = contract_fork` + `safety_halt.engage(SyncReason::ContractFork)` (`:1199`) +
выход; остальное постоянное — `debug!` + выход. `error!` под `!is_engaged()`, иначе
повторные reconcile'ы той же эпохи печатали бы строку на каждый тик (Д-133).
`CommitteeError::is_contract_impossible()` (`committee/mod.rs:281`) — `false` для
`NotReadable`/`OutOfWindow` по построению: только вернувшееся чтение может нести
заявление цепи. `SyncReason::ContractFork` (`sync_metrics.rs:67`, лейбл `contract_fork`,
`ALL` 10 → 11). Фасад не тронут.

### 2.4 Тесты

- Стенд (а): `a_weightless_committee_inside_the_window_stops_every_node_that_must_enter_it`
  (`testbed/committee_tests.rs`) — предикат `halted_for(10)` держит прогон 10 тиков после
  последнего halt'а, чтобы «и дальше ничего не двигалось» было наблюдением, а не
  допущением.
- Стенд (б): `a_reverting_committee_read_inside_the_window_refuses_the_epoch_and_leaves_the_node_up`
  + фикстура `StandConfig::reverts_for`.
- Юниты модуля: `absent_weights_inside_the_window_are_permanent_and_poison_the_epoch`
  (переименован из `…_and_cache_nothing`), `a_second_answer_for_one_epoch_…` дописан под
  отравление, `a_transient_read_error_caches_nothing_and_is_retried` дописан половиной
  «ревёрт НЕ отравляет».
- Юнит классификации: `every_variant_falls_into_exactly_one_of_the_three_classes`
  (`staking-reader/src/error.rs`). «Красный без предиката» — тривиально: без `class()` он
  не компилируется. Полноту списка держит не он, а исчерпывающий `match` в `class()`.

## §3 Отклонения от задания (Д-132…Д-138)

- **Д-132.** Вместо второго булева предиката — enum `ReadClass` + два предиката-вида на
  нём. Причина: «каждый вариант ровно в одной категории» становится свойством типа, и два
  исчерпывающих `match` не могут разойтись. Имя и место: `staking-reader/src/error.rs`,
  рядом с `is_transient`.
- **Д-133.** `error!` и `engage` в менеджере закрыты проверкой `!is_engaged()`. Задание
  просило «один `error!`»; без проверки каждая последующая reconcile той же эпохи (а их
  много — tip, wake-up, boundary) печатала бы строку заново.
- **Д-134.** Двух вариантов в списке задания не было. `Backend` отнесён к «ревёрту»
  (постоянный, но это СВОЁ хранилище: header-index на материализованной высоте; останавливать
  узел за свой диск — не решение владельца, там речь про «контракт ОТДАЛ невозможное»).
  `ZeroEpochInterval` отнесён к «невозможному», но через модуль недостижим: геометрия
  приходит из `watch`, а `Geometry::new` отказывает на нулевом интервале до чтения.
- **Д-135.** Отравление ПЕРЕКРЫВАЕТ запись. Касается только форк-плеча `install`, где
  запись уже есть: до правки следующий `committee(E)` отвечал первой записью, теперь —
  отказом. Цена названа в §5. Тест `a_second_answer_for_one_epoch_is_refused_and_the_first_record_stands`
  переписан (имя оставлено — первая запись всё ещё «стоит» как значение, с которым
  сверяется write-once, но потребителю не отдаётся).
- **Д-136.** Премиса Ex-4.1e переставлена с «исполненный tier дошёл до `last(1)`» на
  «ordering-плоскость дошла до `last(1)`, граница эпохи 2 доставлена, исполнение отстало не
  более чем на `K`». Причина по прогону: при fail-stop исполнение замирает на `K` блоков
  ниже границы (наблюдено `[62, 62, 62, 63]` при `last(1) = 63`), так что старая премиса
  стала бы утверждением про лаг, а не про живую цепь.
- **Д-137.** ERROR-набор прогона (а) — `3n`, не `2n`: третья строка на узел — собственная
  парковка исполнителя (`executor SafetyHalt — parking`). Она в ассерте названа отдельно:
  это и есть доказательство, что halt ДОШЁЛ до исполнения.
- **Д-138.** Ворота «clippy с фичей» в форме задания (три крейта сразу) не собираются и
  на HEAD: `fluentbase-node` не знает поля `byzantine` в `DposLayerConfig`
  (`crates/node/src/dpos.rs:2239`, `error[E0063]`). Прогонял и записывал
  консенсус-только: `cargo clippy -p fluentbase-consensus --all-targets --features
  dpos-devnet-byzantine` — 0 предупреждений до и после. Дефект не мой, в код не лез.
- Новые `state.lock().unwrap()` в сторе — тот же идиом, что во всех остальных строках
  этого файла; `#[allow]`, `todo!` не добавлял.

## §4 Оставлено как есть

- **Фасад `committee/facade.rs:47-54`** — по заданию: бикон по-прежнему видит `None` на
  любом отказе. Остановку делает менеджер.
- **R-129 и R-130** — не тронуты. `CertProvider::scoped` по-прежнему отвечает `None` на
  отравленную эпоху, то есть после форка контракта marshal-резолвер будет отвечать
  `deliver == false` и исключать честных пиров (R-129) — при том что узел уже стоит.
- **`deliver`, inlet, слэшер** — ветки `Read(_)` не менялись; слэшер по-прежнему
  маппит постоянный отказ в `Permanent` и отпускает charge.
- **Ревёрт не мемоизируется** — два staticcall'а на каждый `committee(E)` реверта
  остаются (B3-12 закрыт только для «невозможного»). Это решение, а не недоделка:
  починенный модуль обязан читаться без рестарта.
- **Повтор спавна при `WeightsUnavailable`** (остаток R-035) — не трогал. Перепроверено
  по коду: `epoch_manager.rs:1414` по-прежнему `error!` + `soft_enter` + `return` без
  очереди повтора. Но ветка стала практически мёртвой: `WeightedVrf::try_new` строится из
  `record.snapshot_view()`, где `weights` всегда `Some` и длина сверена в `build()`.
- **`cargo fmt`** переформатировал ровно два моих места (цепочки вызовов) — принял как есть.

## §5 Всплыло

- После обнаруженного форка контракта эпоха теряет и СХЕМУ (`scheme(E)` идёт через
  `committee(E).ok()?`), то есть marshal перестаёт верифицировать её сертификаты. Для
  fail-stop это ожидаемо (узел встал), но обещание §5.4 «marshal продолжает раздавать»
  для ЭТОЙ эпохи не выполняется. Отдельная строка, не трогал.
- `ZeroEpochInterval` производится только `epoch_transition.rs` (`:520`, `:576`), то есть
  живёт на другом читателе; через модуль комитета его нельзя получить вовсе — кандидат на
  ревизию «где вообще живёт геометрия» (П-10, R-130).
- `SyncReason` теперь 11 вариантов; дашборды и алерты вне репозитория про `contract_fork`
  не знают.
- В §5.4 проекта строки 554 и 555 перечисляют варианты, которых модуль не производит
  (`CommitteeOutOfOrder` / `CommitteeDuplicatePeerKey` / `CommitteeTooSmall` ловит reader
  раньше) — текст проекта описывает не тот слой; правка проекта не делалась, он история.

## §6 Где проверка слабее всего

1. **Живого прогона не было.** Всё — стенд и юниты. Ни девнет, ни e2e не гонялись;
   `cargo test -p fluentbase-node --lib` и clippy — единственное, что покрывает узел
   целиком.
2. **Halt проверен на одном классе причины.** Стенд умеет `weights: None`; варианты
   `CommitteeOutOfOrder` / `PeerKey` / `BlsKey` до halt'а не доводились — они дойдут той
   же веткой, но это вывод, а не прогон. Форк-плечо (повторное чтение с другим значением)
   проверено ЮНИТОМ стора, не стендом: что именно сделает менеджер, получив его, не
   наблюдалось.
3. **«Marshal продолжает раздавать»** не проверено ничем: стенд смотрит, что tip ЗАМЕР, а
   не что архив ещё отвечает пирам.
4. **Тест (б) не был красным до правки** (см. §0.4); его различающая сила держится на
   одной мутации, которую я прогнал один раз.
5. **Премиса «граница эпохи 2 доставлена»** в (а) читает `et_boundaries`, то есть лог
   `EpochTransition` стенда, а не то, с какой именно кромки пришёл reconcile. Что halt
   случился именно на boundary-плече (а не на tip-плече или на wake-up), не доказано —
   доказано только, что эпоху узлу вручали.
6. **Счётчики markdown-контроля** этого документа и правленых `.md` — §7.

## §7 Контроль правленых `.md`

Прогнано по четырём правленым файлам. Счётчики (журнал / REGISTER / EXPERIMENTS /
DECISIONS):

- строки с многоточием: 6 / 22 / 6 / 1 — все осмысленные (сокращённые цитаты и
  многоточие внутри сообщений кода);
- строки с нечётным числом обратных кавычек: 8 / 0 / 0 / 0 — из восьми четыре суть
  строки-заборы кода (` ``` `), остальные четыре — две пары перенесённых по строкам
  инлайн-кодов (13–14 и 248–249), что валидный markdown;
- `,,`: 1 / 0 / 0 / 0 — единственное вхождение это сам символ в описании этой проверки;
- пустые пары обратных кавычек: 0 / 0 / 0 / 0 (совпадения `grep '``'` — те же четыре
  забора);
- пустые `()` вне кода: 0 / 1 / 0 / 0 — единственное в `REGISTER.md:626` есть `Ok(())`
  внутри инлайн-кода, чужое и правильное.

Проверки прогонялись пофайлово: общий вызов `grep -c` по списку файлов печатает ОДНО
число и выглядит как «ноль везде» — на нём я один раз чуть не подписался.
