# E1-CONTRACT-2 — вторая контрактная сессия: остатки 1.3, 1.6, 1.8 и вестигии W6

Сессия 2026-09-11, ветка `djadjka/dpos-reth-2.2-squashed`, дерево
`/home/djadjka/Work/fluentbase`, контракт `contracts/staking/src`.
`CARGO_TARGET_DIR=/home/djadjka/Work/fluentbase/target-contract` (target делится с
другой сессией, поэтому свой каталог).

Правило свидетельства этой сессии: факт — это file:line, который я открыл, или
команда, которую я прогнал здесь. Комментарий и док-комментарий свидетельством
поведения не считаются (в этом контракте прошлый проход нашёл комментарий,
утверждавший обратное коду, — K-5). Тест — свидетельство, только если сказано, был
ли он красным до правки.

## §0 Прямые ответы

**1. Пункты П1–П8 и sha.** Все восемь сделаны.

| пункт | статус | sha кода | sha docs |
|---|---|---|---|
| П1 (1.6, K-16) пол комитета при `initialize` | сделан | `5386a635` | `0ea497da` |
| П2 (1.6, K-18) `ensure_initialized` | сделан | `548f10bd` | `bac2035c` |
| П3 (1.6, K-22) ревёрт при отказе фонда | сделан | `3c4560db` | `fb3d0030` |
| П4 (а) F-1 двухцветная фикстура | сделан | `79152ea2` | `5449a314` |
| П4 (б) F-3 вторая книга заглушки | сделан | `52c84f6d` | `5449a314` |
| П4 (в) четыре класса | сделан | `32c1f2f3` | `146cc8b1` |
| П4 (г) арифметика | сделан | `9c3c3610` | `146cc8b1` |
| П5 (W6) вестигии | сделан | `9d4fa617` | `f2336452` |
| П6 (1.3) таймлок | сделан | `b237a81e` (блоб — в П7) | `39b11300` |
| П7 (R5.2) `claimValidatorFeeAtEpoch` | сделан | `0b9daeac` (с блобом) | `39b11300` |
| П8 (1.6, K-13) строгая калдата | сделан, ветвь обёртки | `9cd156db` (с блобом) | этот коммит |
| П9 (контр-ревью) таймлок пересмотрен | сделан, не пункт задания | `0f283a82` (с блобом) | `a03181de` |

Строка П9 дописана 2026-09-11 по `E1-CONTRACT-REVIEW.md` F-4: §0 был остановлен на
П8 и не переписан после последнего коммита сессии, поэтому ответы 1, 2 и 3 ниже
описывали не сессию, а её середину. Числа в ответах 2 и 3 исправлены там же.

**2. Ворота.** Контракт ДО (verbatim, прогон мой на `89046c93`):
`test result: ok. 175 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s`;
с `--features devnet-views` — `176 passed; 0 failed`; clippy
``Finished `dev` profile [optimized] target(s) in 7.62s`` без warning'ов; `fmt --check` чисто.
ПОСЛЕ П8:
`test result: ok. 191 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s`;
`192 passed; 0 failed` с фичей; clippy
``Finished `dev` profile [optimized] target(s) in 3.53s``; `fmt --check` чисто.
ПОСЛЕ П9, то есть итог СЕССИИ (verbatim в §2 П9, `:993-1005` до этой правки):
`test result: ok. 194 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s`;
`195 passed; 0 failed` с фичей; clippy и `fmt --check` чисто.
Общие ворота после П6/П7 и, повторно, после П8 — одинаковы:
`cargo test -p fluentbase-node --lib` → `57 passed; 0 failed`;
`cargo test -p fluentbase-staking-reader` → `58 passed; 0 failed`;
`cargo test -p fluentbase-e2e --release staking` → `13 passed; 0 failed` (118 filtered out);
`agreement_check.py` → `=== 15 checks, 0 disagree, 0 unread ===`;
селекторный скан `.rwasm` → `selector scan OK`. Девять красных `builtins::*` не
наблюдались — фильтр `staking` их не берёт.

**3. Число тестов.** До: 175 без фичи / 176 с фичей (177 атрибутов `#[test]`, из них
два взаимно исключающих по фиче — §1). После П8: 191 / 192. После П9, то есть итог
сессии: 194 / 195, чистый прирост 19.
**Удалённых как избыточные — НОЛЬ.** Заменены, а не удалены, ТРИ теста — один в П3 и
два в П9:
`a_slash_survives_a_fund_that_refuses_the_seizure` пинил политику, которую П3
перевернул, и его место занял `a_fund_that_refuses_the_seizure_reverts_the_whole_slash`;
`the_address_setters_declare_now_and_land_seven_epochs_later` →
`the_reserve_setter_declares_now_and_lands_seven_epochs_later` и
`the_address_timelocks_are_governance_only_on_both_halves` →
`the_reserve_timelock_is_governance_only_on_every_mutating_half`, оба потому, что П9
вывел `setSlashFundAddress` из-под таймлока и «оба адреса» перестали быть предметом.
Одно утверждение исчезло вместе с хендлером, о котором было (окно за пределами
текущей эпохи, П7). Пар мутаций, показывающих избыточность ТЕСТА, я не строил, потому
что ни одного теста не удалял.

**4. «Не измерим на заглушке» — НОЛЬ тестов.** Ни один из четырнадцати в остатке не
получил этого вердикта: одиннадцать — «проверка не снималась» (пины селекторов,
топиков, раскладки хранилища, векторов кодировки и маршрут улик, который вне объёма),
два — «проверка снималась, тест красный, но мутацией не своего класса», один — Д-06.
Заглушка, к которой вердикт мог бы относиться, — `install_stipend_token`, и её как раз
починил П4(б), так что повода его выставить не возникло.

**5. K-13: кого ломает строгость, и выбранная ветвь.** Ломает: калдату
universal-token (`crates/sdk/src/universal_token/command.rs`), два чтения хранилища
(`crates/sdk/src/universal_token/storage.rs:81`, `crates/sdk/src/storage_legacy.rs:41`),
калдату `contracts/webauthn` (два места) и — это решающее —
`crates/sdk/src/universal_token/storage.rs:167,179`, где ДВЕ версии
`InitialSettings` различаются перебором по одному и тому же payload'у: строгость
убирает не запас прочности, а сам механизм версионирования. Ветвь выбрана
**обёртка на стороне контракта**: SDK не тронут, `util::decode` (статический путь)
сверяет пере-кодированные байты со входом. `decode_args` (динамический путь,
`initialize` и три маршрута улик) сознательно не ужесточён.

**6. Отклонений Д-nn — десять** (Д-01…Д-10). Записями Д-nn не оформлены решения
владельца по находкам 1 и 2 контр-ревью — они лежат в §2 П9 как решения, а не как
отклонения (дописано 2026-09-11, `E1-CONTRACT-REVIEW.md` F-4). Ратификации владельцем
просят два:

- **Д-08** — `STATUS_ACTIVE` оставлен и задокументирован, а не удалён, хотя W6
  числит его среди вестигий. Причина по коду: тот же терм нагружен в
  `eligible_population_at_least`, и удаление в одном месте рассогласовало бы две
  функции о том, кто пригоден.
- **Д-07** — из 45 зелёных мутаций классов закрыто тестами 18, показано избыточными
  5, а 21 оставлена открытой дырой. Это сознательный отказ расширять пункт «только
  тесты» до трёх-четырёх десятков новых тестов; если владелец считает иначе — это
  отдельная работа, и список готов.

Остальные восемь — фактические (Д-04 поправляет премису задания по замеру, Д-05
форма таблицы, Д-06 четвёртый вердикт, Д-09 объём общего крейта, Д-10 порядок
коммита и блоба, Д-01/Д-02/Д-03 — следствия правок, проверенные по коду).

**7. Что в строках плана 1.3/1.6/1.8 оказалось неверным по коду.** Три вещи, все
измерены:

- **1.6 / K-18.** Строка говорит «`ensure_initialized`: 16 вызовов в `staking.rs`, 0
  в `config.rs`» и ведёт к выводу, что чинить надо `config.rs`. По замеру чинить там
  нечего: тринадцать вьюх `config.rs` не считают ничего, двенадцать сеттеров закрыты
  `ensure_governance`. Падали ДВЕ вьюхи, обе в `staking.rs`.
- **1.3.** «`test_prod_substrate.py:981` гоняет `setBlendReserve` через governance —
  НЕ править, записать как открытую приёмку». Строка 981 — юнит-тест форматтера
  вердикта, `"setBlendReserve"` передаётся в него как `desc=`; вызова харнесс не
  делает нигде. Открытой приёмки не существует.
- **1.8 / F-3.** «показать, что они краснеют при M53 ПОСЛЕ правки заглушки и что
  были зелёными до». M53 красная и до, и после. Достаточность меряет другая мутация
  (M53b), и она действительно была зелёной против всего дерева.

Сверх плана: 1.6 подразумевала, что K-13 — «усечение целых И игнорирование хвоста»;
по замеру усечение БУФЕРА уже отвергалось, а усекались целые ВНУТРИ слова.

**8. Где я принял утверждение теста или комментария без проверки по коду.** Прямо,
четыре места:

1. **Столбец «до правки» для M65** («КРАСНАЯ (2)») взят из `E1-8-TESTS.md` F-1
   замером 09-08, а не перепрогнан в этой сессии до смены фикстур. Перепрогон
   ПОСЛЕ дал пять, и в пятёрку входят те самые два — косвенное подтверждение, не
   прямое. В таблице §4 так и помечено.
2. **Вердикт A34** («эквивалентна по значению») опирается на арифметику
   пропорционального деления и на док-комментарий `staking.rs:2162-2167`, который
   утверждает то же. Численного теста на равенство долей при сыром и расширенном
   весе я не ставил.
3. **README, разделы «Solidity parity» и таблица аудита событий** — не сверял с
   деревом вообще (§5). Не правил именно поэтому.
4. **`TIMELOCK_EPOCHS = 7` в `e2e/src/staking_reserve.rs`** — переписан рукой с
   `consts.rs`, а не разделён через крейт. Расхождение константы компилятор не
   поймает; поймает только упавший e2e.

**9. Длинные строки `.dpos-study` читал** `sed -n 'N,Mp'` и инструментом Read; `cut -c`
и `head -c` не применялись ни разу.

## §1 Инвентаризация «до»

HEAD на входе: `89046c93 docs(dpos): record the committee wiring`. В дереве —
незакоммиченные правки ЧУЖОЙ сессии под `crates/dpos/`, `crates/node/` и
`.dpos-study/history/E4-ORCHESTRATOR.md`; их я не трогаю.

Ворота контракта на HEAD, verbatim (прогон мой, 2026-09-11):

| Ворота | Вывод |
|---|---|
| `cargo test` | `test result: ok. 175 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s` |
| `cargo test --features devnet-views` | `test result: ok. 176 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s` |
| `cargo clippy --all-targets -- -D warnings` | ``Finished `dev` profile [optimized] target(s) in 7.62s`` (без единого warning) |
| `cargo fmt --check` | чисто (только шесть `Warning: can't set … unstable features are only available in nightly channel`, они на stable печатаются всегда) |

Совпадает с записью `PLAN.md` §1 (175/0 и 176/0, 09-09).

Счёт `#[test]` по файлам (`grep -c '#\[test\]' src/*.rs`):

| файл | `#[test]` |
|---|---|
| `src/tests.rs` | 151 |
| `src/evidence.rs` | 21 |
| `src/math.rs` | 4 |
| `src/util.rs` | 1 |
| остальные 11 файлов | 0 |
| **итого** | **177** |

177 атрибутов против 175/176 прогнанных — расхождение объяснено, не списано:
в `tests.rs` два теста взаимно исключающие по фиче —
`devnet_view_selectors_match_their_pinned_ids` под `#[cfg(feature = "devnet-views")]`
(`tests.rs:1342-1343`) и `the_production_shape_answers_no_view_selector` под
`#[cfg(not(feature = "devnet-views"))]` (`tests.rs:1358-1359`), плюс
`production_liveness_views_read_the_new_namespace` под фичей (`tests.rs:7848-7849`).
`cargo test -- --list` подтверждает: без фичи `tests` 149, с фичей 150; `evidence` 21,
`math` 4, `util` 1 в обоих случаях.

Строк в `src/*.rs` — 17 897 (`wc -l`), из них `tests.rs` 10 383.

## §2 По пунктам

### П1 (1.6, K-16). Пол комитета при `initialize`

**Что было.** `validate_initialization` (`config.rs:131-145` на HEAD) проверяла cap
только на `!= 0` и `<= MAX_COMMITTEE_SIZE`; комментарий над проверкой прямо объявлял
это намеренным («Deliberately only the zero check here, not the committee floor the
setter enforces»). Гейт `cap < MIN_COMMITTEE_LENGTH` стоял только в сеттере
`set_active_validators_length` (`config.rs:384-390` на HEAD), с ошибкой
`ERR_ACTIVE_VALIDATORS_LENGTH_BELOW_COMMITTEE_FLOOR` и полезной нагрузкой
`(value, MIN_COMMITTEE_LENGTH as u32)`.

**Что сделано.** Тот же гейт, тот же код ошибки, та же пара в нагрузке, перед общей
проверкой `ERR_INVALID_CHAIN_CONFIG` — `config.rs:138-151`. Дизъюнкт
`active_validators_length == 0` из общей проверки убран: после гейта он недостижим
(ноль ниже пола). Комментарий переписан по коду.

**Файлы.** `src/config.rs` (гейт), `src/consts.rs` (док `MIN_COMMITTEE_LENGTH`:
абзац утверждал «`initialize` does not» — это стало ложью в том же изменении),
`src/tests.rs` (новый тест + три места, где фикстура задавала cap ниже пола).

**Тесты.**

- `initialize_refuses_a_committee_cap_below_the_floor` — новый. **Был ли красным до
  правки: да** — прогнан против кода с гейтом, заглушённым на `if false && …`:
  `test result: FAILED. 175 passed; 1 failed`, упал ровно он.
- Тесты, которые правка сломала и которые пришлось поправить, — 14 штук, все по одной
  причине: их фикстура инициализировалась с cap ниже пола. Разобрано в §3 (Д-01).

**Ворота после П1, verbatim:**

    cargo test                        → test result: ok. 176 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
    cargo test --features devnet-views → test result: ok. 177 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
    cargo clippy --all-targets -- -D warnings → Finished `dev` profile [optimized] target(s) in 2.69s
    cargo fmt --check                 → чисто

ABI не затронут: ни одного нового/удалённого/переименованного селектора, сигнатура
`initialize` не менялась. Общие ворота по правилу Э2 не требуются; `cargo test -p
fluentbase-genesis-bootstrap` прогнан отдельно, потому что именно он строит генезис
через `initialize` (результат — ниже).

**sha:** `5386a635` (код), docs — следующим коммитом.

**Общие ворота.** ABI не тронут, но `initialize` — точка входа генезиса, поэтому
прогнан `cargo test` крейта `fluentbase-genesis-bootstrap` (он строит генезис именно
этим вызовом): `test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered
out; finished in 222.66s` (плюс `emit_pop_vectors` 0/0/2 ignored и doc-тесты 0/0).

**Документы.** `.claude/dpos_architecture/12_…md` — строка `MIN_COMMITTEE_LENGTH`
говорила «The contract refuses a cap under it» без указания, где именно; переписана
с пометкой `[REVISED 2026-09-11, Э1.6 / K-16]`, названы оба места. Блок
`verified-against` в `00_preamble.md` дополнен записью 2026-09-11. Правка
`.claude/dpos_architecture/` в коммит НЕ входит и войти не может: каталог целиком
игнорируется (`.gitignore:46 .claude`) и в индексе git отсутствует
(`git ls-files .claude/dpos_architecture/` — ноль файлов). Правка живёт на диске;
`-f` в этой сессии разрешён только для `.dpos-study/`.

### П2 (1.6, K-18). `ensure_initialized` на точках входа без гейта

**Перечень всех внешних точек входа `config.rs` по диспетчеру `lib.rs`** — 25 штук,
13 вьюх и 12 сеттеров. Построен скриптом по `lib.rs:48-76` и телам функций, не по
памяти:

| вьюха (`config.rs`) | гейты | вьюха (`config.rs`) | гейты |
|---|---|---|---|
| `get_staking_token` :190 | `ensure_non_payable` | `get_blend_stipend_per_epoch` :345 | `ensure_non_payable` |
| `get_active_validators_length` :203 | `ensure_non_payable` | `get_min_verdict_due_blocks` :553 | `ensure_non_payable` |
| `get_epoch_block_interval` :216 | `ensure_non_payable` | `get_exclusion_backoff_cap` :605 | `ensure_non_payable` |
| `get_dpos_activation_block` :229 | `ensure_non_payable` | `get_production_liveness_disabled` :640 | `ensure_non_payable` |
| `get_undelegate_period` :242 | `ensure_non_payable` | `get_blend_reserve` :672 | `ensure_non_payable` |
| `get_min_validator_stake_amount` :255 | `ensure_non_payable` | | |
| `get_min_staking_amount` :268 | `ensure_non_payable` | | |
| `get_slash_fund_address` :313 | `ensure_non_payable` | | |

Двенадцать сеттеров — `set_slash_fund_address` :326, `set_blend_stipend_per_epoch`
:358, `set_active_validators_length` :384, `set_epoch_block_interval` :424,
`set_dpos_activation_block` :454, `set_undelegate_period` :483,
`set_min_validator_stake_amount` :510, `set_min_staking_amount` :533,
`set_min_verdict_due_blocks` :576, `set_exclusion_backoff_cap` :618,
`set_production_liveness_disabled` :653, `set_blend_reserve` :693 — все начинаются с
`ensure_governance_mutation`, а она зовёт `ensure_governance` (`config.rs:19-22`),
которая ПЕРВОЙ строкой зовёт `ensure_initialized` (`util.rs:60`). Governance-гейт
покрывает; отдельный гейт ни одному сеттеру не нужен.

**Ни одна вьюха `config.rs` не падает арифметикой.** Все тринадцать — одно чтение поля
через `write_abi(… get_checked …)`, ни одного деления и ни одного вызова
`current_epoch`. Это измерено, а не вычитано: см. ниже.

**Где симптом K-18 живёт на самом деле.** `ExitCode::IntegerDivisionByZero` рождается в
одном месте — `util.rs:84`, `math::epoch_at_block(...).ok_or(ExitCode::IntegerDivisionByZero)`,
а `staking_protocol::epoch_at_block` (`crates/types/src/staking_protocol.rs:139-148`)
отдаёт `None` ровно при `interval == 0`, то есть до `initialize`. Я прогнал пробник по
всем негейченным точкам чтения на неинициализированном контракте и получил (вывод
теста, не рассуждение):

    PROBE getValidators: IntegerDivisionByZero len=0
    PROBE isValidatorActive: Ok 0x00000000
    PROBE isValidator: Ok 0x00000000
    PROBE getValidatorStatus: Ok 0x00000000
    PROBE getValidatorByOwner: Ok 0x00000000
    PROBE getValidatorDelegatedStakeAt: IntegerDivisionByZero len=0
    PROBE getValidatorDelegation: Ok 0x00000000
    PROBE getEpochBlockInterval: Ok 0x00000000
    PROBE getBlendReserve: Ok 0x00000000
    PROBE getStakingToken: Ok 0x00000000
    PROBE getActiveValidatorsLength: Ok 0x00000000
    PROBE getMinVerdictDueBlocks: Ok 0x00000000

Две вьюхи, обе в `staking.rs`: `get_validators` (:894, через `selected_validators`
→ `current_epoch`) и `get_validator_delegated_stake_at` (:1023, через
`current_epoch_at_block`). `isValidatorActive` попадает во вторую группу по
короткому замыканию `&&`, а не по устройству: `selected_validators` там стоит вторым
операндом после `validator_status(...) == STATUS_ACTIVE`, который до `initialize` ложен
для любого адреса (`staking.rs:848-853`). Пробник после замера удалён.

**Что сделано.** `ensure_initialized` первой строкой в эти две вьюхи (`staking.rs:894`,
`:1023`). В `config.rs` прод-кода не тронуто — там нечего чинить.

**Файлы.** `src/staking.rs`, `src/tests.rs`.

**Тесты.**

- `the_two_views_that_reach_the_epoch_formula_refuse_an_uninitialized_contract` —
  новый. **Был ли красным до правки: да** — снял оба `ensure_initialized`, прогон:
  `test result: FAILED. 177 passed; 1 failed`, упал ровно он. Вторая половина теста
  перечисляет ОСТАЛЬНЫЕ восемнадцать негейченных точек чтения и утверждает, что каждая
  отвечает `ExitCode::Ok` на неинициализированном контракте — это и делает «гейт не
  нужен» измерением, а не прочтением кода.
- `every_config_setter_refuses_an_uninitialized_contract_before_it_checks_anything_else`
  — новый. **Был ли красным до правки: НЕТ, он был зелёным** — он пинит поведение,
  которое уже было, и никакого прод-кода под него не писалось. Что он стоит, показано
  мутацией: снятие `ensure_initialized(sdk)?` из `ensure_governance` (`util.rs:60`)
  даёт `test result: FAILED. 177 passed; 1 failed`, и падает ровно он — до этой
  сессии ни один тест не ловил снятие этой строки.

**Ворота после П2, verbatim:**

    cargo test                         → test result: ok. 178 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
    cargo test --features devnet-views → test result: ok. 179 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
    cargo clippy --all-targets -- -D warnings → Finished `dev` profile [optimized] target(s) in 1.57s
    cargo fmt --check                  → чисто

ABI не затронут: селекторы те же, сигнатуры те же; меняется только, чем отвечают две
вьюхи до `initialize` — состояние, в котором живая цепь не бывает после генезиса.

**sha:** `548f10bd` (код), docs — следующим коммитом.

### П3 (1.6, K-22). Отказ фонда ревёртит слэш

**Что было.** `consensus.rs` (`seize_self_stake`, хвост): `if !try_transfer(sdk,
recipient, seized)? { seized = U256::ZERO; }`, затем `EquivocationStakeSeized` с нулём.
Тумбстоун, тюрьма, удаление из активного набора и штамп невидимости отбора записаны
выше и оставались записанными. Комментарий над этим местом объявлял правило «платёж не
должен ревёртить» и обосновывал его: иначе отказавший токен сделает эквивокацию
неслэшируемой.

**Решение владельца** — ревёрт при отказе фонда, не учёт замороженного. Записано в
`DECISIONS.md` §3 «K-22: ревёрт при отказе фонда — принято 2026-09-11».

**Что сделано.** `return revert(sdk, ERR_STAKING_TOKEN_CALL_FAILED);` вместо обнуления.
Ошибка выбрана из существующих: конфискация двигает именно staking-токен. Комментарий
переписан — он утверждал противоположную политику, а поверх неё лежит цена, которую
теперь платит контракт, и она названа в коде.

**Почему цена переживаема** — проверено, не предположено: `crates/node/src/evm.rs:1184-1190`
на ветке `ExecutionResult::Revert` для `slashEquivocation` пишет `tracing::warn!` и НЕ
коммитит состояние (комментарий выше, `:1155-1162`, объясняет выбор: fail-loud «convert
a lost slash into a stalled chain»). То есть ревёрт системного вызова слэша не
останавливает цепь — он теряет слэш. Три маршрута по уликам
(`slashEquivocationNotarize/Finalize/NullifyFinalize`) — обычные транзакции от EOA
слэшера, там ревёрт просто проваливает транзакцию.

**Поправка 2026-09-11 (`E1-CONTRACT-REVIEW.md` F-23, прочитано по коду обеих сторон).**
Последняя фраза неверна. Транзакции слэшера в этом дереве идут через предсимуляцию, и
любой ревёрт кроме `AlreadySlashedForEquivocation` становится `Sim::Rejected` →
`SubmitOutcome::Failed` (`crates/node/src/slasher_sink.rs:235-244`, `:281`), а
`run_consumer` на `Failed` НЕ акает запись WAL и логирует `error!` с текстом «A
simulated revert here is a deterministic bug (calldata/EIP-2537 encoding) — alert;
retrying the same bytes won't help» (`crates/dpos/consensus/src/slasher/actor.rs:1050-1057`).
`ERR_STAKING_TOKEN_CALL_FAILED` — не баг кодирования, и повтор тех же байт как раз
поможет, как только получатель примет. То есть «проваливает транзакцию» стоило бы
читать как «оставляет незакрываемую запись WAL, переигрываемую на каждом рестарте, под
алертом с неверной причиной». Плюс второй стык, которого нет ни в одной строке этого
журнала: `ChargeStore::next_charge` выбрасывает заряд только по `tombstoned(accused)`
(`actor.rs:208-229`), значит ненаказуемый эквивокатор занимает единственный слот «один
заряд на блок» своей эпохи навсегда. Оба стыка и есть причина, по которой K-22
пересмотрена 2026-09-11 на burn-fallback (`52714f62`, `DECISIONS.md` §3).

**Файлы.** `src/consensus.rs`, `src/tests.rs`.

**Тесты.**

- `a_fund_that_refuses_the_seizure_reverts_the_whole_slash` — заменил
  `a_slash_survives_a_fund_that_refuses_the_seizure`, который пинил старую политику
  (`seized = 0`, тумбстоун стоит) и стал красным от правки прод-кода. **Был ли красным
  до правки: да** — прогнан против восстановленного обнуления:
  `test result: FAILED. 177 passed; 1 failed`, упал ровно он.
- Заглушка **сама проверяет, что отказала**: `record_transfers_refusing`
  (`tests.rs:6440-6476`) записывает КАЖДУЮ попытку `transfer` в `transfers`, поэтому
  обе отказные ноги утверждают, что платёж был предъявлен на полную сумму
  (`&[(EQUIVOCATION_BURN_SINK, stake)]`), а не что его не было. Сверх этого добавлена
  контрольная нога: та же фикстура, тот же вызов, пустой список отказа — слэш проходит,
  тумбстоун встаёт, событие сообщает `stake`. Заглушка, тихо переставшая отказывать, не
  может покрасить отказные ноги зелёным при живой контрольной.
- Откат проверяется по четырём записям отдельно (тумбстоун, статус `STATUS_ACTIVE`,
  длина очереди делегации, `selection_visible_at`), плюс отсутствие события
  `EquivocationStakeSeized`. Откат в юнит-тесте не фикция: `Harness::call`
  (`tests.rs:186-197`) снимает `dump_storage()` до вызова и делает `restore_storage`
  на любом не-`Ok` выходе.

**Ворота после П3, verbatim:**

    cargo test                         → test result: ok. 178 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
    cargo test --features devnet-views → test result: ok. 179 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
    cargo clippy --all-targets -- -D warnings → Finished `dev` profile [optimized] target(s) in 2.76s
    cargo fmt --check                  → чисто

ABI не затронут. Потребителей события `EquivocationStakeSeized` вне контракта нет
(`grep` по `e2e/src/`, `crates/dpos/`, `crates/node/`, `devnet/local-dpos-smoke/dpos_harness/`
— пусто), так что общие ворота эта правка не задевает.

**sha:** `3c4560db` (код), docs — следующим коммитом.

### П4 (1.8 остаток). Только тесты

#### П4 (а). F-1 — одноцветные фикстуры трёх тестов личности

**Что было.** `compressed_key_of(byte)` (`tests.rs:427-432`) и фикстуры трёх тестов
брали ОДИН байт на все 256 байт EIP-2537 G2. По `bls.rs:315-329` `x.c0` — байты
`[16..64]`, `x.c1` — `[80..128]`, и `compress_g2_unchecked` кладёт в вывод сначала
`x_c1`, потом `x_c0`. При одинаковом заполнении перестановка половин не двигает ни
байта.

**Что сделано.** Два помощника рядом с `compressed_key_of`:
`g2_uncompressed_of_halves(c0, c1)` — 256-байтная точка с нулевыми паддингами,
`x.c0 = c0`, `x.c1 = c1`, обе половины `y` = `c0`; и `compressed_key_of_halves(c0, c1)`
— её 96-байтный эталон (`c1` впереди, флаг сжатия и знак в первом байте). Три теста
переведены на `(0x11, 0x33)`. У `register_validator_cast_calldata_registers_consensus_keys_atomically`
hex-литерал переписан пословно и рядом стоит утверждение
`&calldata[4 + 7*32 .. 4 + 15*32] == g2_uncompressed_of_halves(0x11, 0x33)`, чтобы
литерал и помощник не разъехались.

**Мутация M65** (`bls.rs`, `out[..FP_LENGTH] = x_c0; out[FP_LENGTH..] = x_c1` — перестановка
отменена), прогон после правки:

    test tests::g2_compression_swaps_the_halves_and_reads_the_sign_from_c1 ... FAILED
    test tests::get_consensus_keys_matches_dynamic_struct_return_vectors ... FAILED
    test tests::register_validator_cast_calldata_registers_consensus_keys_atomically ... FAILED
    test tests::register_validator_verifies_and_stores_consensus_keys_in_one_call ... FAILED
    test tests::the_y_sign_bit_is_strictly_above_half_the_field ... FAILED
    test result: FAILED. 173 passed; 5 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s

Пять, а не два — ровно то, что требовалось показать. **Были ли красными до правки:**
три названных теста были ЗЕЛЁНЫМИ при M65 (`history/E1-8-TESTS.md` F-1, замер 09-08;
в этой сессии M65 до правки фикстур не перепрогонялся — перепрогон после правки
показывает пять, и состав пяти включает те два, что F-1 называет единственными
красными).

**Ворота после П4(а), verbatim:**

    cargo test                         → test result: ok. 178 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
    cargo test --features devnet-views → test result: ok. 179 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
    cargo clippy --all-targets -- -D warnings → Finished `dev` profile [optimized] target(s) in 1.53s
    cargo fmt --check                  → чисто

**sha:** `79152ea2`.

#### П4 (б). F-3 — вторая книга в `install_stipend_token`

**Что было.** Заглушка вела только книгу резерва: `transfer` возвращал `true` без
единой проверки, `transferFrom` в пользу `GENESIS_STAKING` ничего не зачислял,
`balanceOf(GENESIS_STAKING)` отвечал нулём. Утверждение «депозит выходит из баланса
контракта» проверялось против баланса бесконечного и неподвижного.

**Что сделано.** Поле `contract_balance` в `StipendFunding`; `transferFrom` в адрес
контракта зачисляет, в чужой адрес — проходит мимо; `transfer` списывает и падает
(`ExitCode::Panic`), когда не хватает; `balanceOf(GENESIS_STAKING)` отвечает второй
книгой. Начальный остаток — пятый аргумент `install_stipend_token` на всех
одиннадцати вызовах: депозиты, сделанные фикстурой ДО подмены обработчика, изнутри
заглушки не видны, и назвать их может только вызывающий.

**Тесты.**

- `a_reward_and_a_matured_principal_claim_are_independent` — существующий, усилен:
  называет свой начальный остаток (`stake * 2` — генезисный самостейк и делегация) и
  утверждает баланс контракта дважды: не тронут после претензии на награду, уменьшен
  ровно на депозит после вывода.
- `a_deposit_the_contract_cannot_cover_is_not_paid_out_of_the_reserve` — новый:
  резерв полон и одобрен, у контракта на один wei меньше депозита; вывод обязан
  провалиться, резерв — не быть спрошенным, депозит — остаться забронированным за
  вкладчиком; затем контрольная нога, где контракт может покрыть, и вывод проходит.
  Против старой заглушки этот тест не «зелёный», а невыразимый: `transfer` там не мог
  отказать.

**Чем это измерено — и чем НЕ измерено.** Требование пункта было показать, что два
теста краснеют при M53 ПОСЛЕ правки заглушки и были зелёными ДО. Премиса неверна по
замеру, и `history/E1-8-TESTS.md` F-3 говорит то же самое своими словами («`M53` …
красная, то есть ИСТОЧНИК запинен, а достаточность — нет»). M53 (платить принципал с
резерва вместо `safe_transfer`) — прогон в этой сессии:

| стенд | M53 |
|---|---|
| заглушка с обеими книгами + новые тесты | КРАСНАЯ, 3 теста: `a_deposit_the_contract_cannot_cover_is_not_paid_out_of_the_reserve`, `a_reward_and_a_matured_principal_claim_are_independent`, `the_delegator_views_report_the_reward_and_the_deposit_apart` |
| заглушка с обезвреженной второй книгой + те же тесты | КРАСНАЯ, те же 3 |

То есть M53 ловится записью `pulls`/`transfers`, которая была и до правки, и правка
заглушки на неё не влияет.

Достаточность измеряет другая мутация — назову её **M53b**: `safe_transfer`
(`util.rs:204-207`) проглатывает неуспешный статус (`return Ok(())` вместо
`Err(result.status)`). Прогон:

| стенд | M53b |
|---|---|
| ВСЕ тесты дерева на `79152ea2` + старая заглушка | **ЗЕЛЁНАЯ**: `test result: ok. 178 passed; 0 failed` |
| заглушка с обеими книгами + новые тесты | **КРАСНАЯ**, 1 тест: `a_deposit_the_contract_cannot_cover_is_not_paid_out_of_the_reserve` |

Это и есть то, что покупает правка заглушки: до неё ни один тест в дереве не замечал,
что контракт платит принципал переводом, который не прошёл.

**Ворота после П4(б), verbatim:**

    cargo test                         → test result: ok. 179 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
    cargo test --features devnet-views → test result: ok. 180 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
    cargo clippy --all-targets -- -D warnings → Finished `dev` profile [optimized] target(s) in 1.88s
    cargo fmt --check                  → чисто

**sha:** `52c84f6d`.

#### П4 (в). Четыре класса, которые 1.8 не мутировала

**Как мерил.** Скрипт-прогонщик (`scratchpad/mut/run.py`): одна текстовая замена в
прод-файле → `cargo test` → запись упавших → восстановление файла из памяти (в
`finally`). Каждая мутация — снятие ровно одной охраняемой проверки, либо возврат
механизма, который проверка исключает. Полная таблица — §4.

**Объём.** Liveness 28 мутаций (`liveness.rs` целиком: `record_production`,
`close_epoch`, `release_expired`, `judge`, `stamp`); отбор 22
(`selected_committee_at`, `active_peer_key_at`, `committee_changed`,
`commit_epoch_committee`, `top_k_by_stake_at`, `selected_validators`); конфиг 48
(`config.rs` — преамбула, `validate_initialization`, все двенадцать сеттеров;
`initializer.rs` — одноразовость, `validate`, `pull_initial_stakes`); ABI-вью 16
(`ensure_non_payable`/`ensure_mutable`, `decode`, `write_abi`, `reserve_available`,
`erc20_scalar_read`, `write_validators_with_keys`, `committee_at`,
`committee_length_at`, `get_epoch_committee_with_stakes`, диспетчер `lib.rs`).
Плюс две комбинированные (K1, K2). Итого 116.

**Результат первого прогона: 45 мутаций ЗЕЛЁНЫХ** — проверка снималась, и ни один
тест этого не замечал.

**Что закрыто тестами** (выбор по тому, что зелёная мутация пропускает, а не по
счёту). Шесть тестов покрыли восемнадцать из сорока пяти:

| тест | какие мутации покрыл |
|---|---|
| `the_recorder_refuses_every_caller_but_the_system_address` | L1 |
| `the_commit_refuses_a_target_past_the_lookahead_horizon` | S12 |
| `a_verdict_pass_past_the_ring_forfeits_its_verdicts_and_says_so` | L12 |
| `initialize_refuses_every_malformed_chain_configuration` | C7, C8, C9, C10, C11, C12 |
| `the_governance_setters_refuse_every_value_they_are_supposed_to` | C15, C18, C25, C28, C30 |
| `the_activation_setter_refuses_the_past_and_a_running_chain` | C22, C24 |
| `handlers_refuse_value_and_refuse_to_mutate_inside_a_static_frame` (усилен) | C1, C2, K2 |

Каждый новый тест **был красным** против нетронутого прод-кода — это и есть
перепрогон мутации после правки (столбец «после» в §4).

**Отдельно про L1.** `record_production` — системный вызов, и его счётчики решают
каждый вердикт живучести. Гейт `caller != SYSTEM_CALLER` снимался — суита зелёная.
То есть до этой сессии любой аккаунт мог накрутить кредит любому месту комитета и
счётчик блоков эпохи, и ни один тест этого не ловил. Это самая крупная находка
пункта.

**Что осталось зелёным — 26 мутаций, каждая с вердиктом:**

| мутация | вердикт |
|---|---|
| L4 (парковка при незакоммиченном комитете) | **избыточна** — показано комбинацией: K1 (снять L4 И L5 вместе) КРАСНАЯ, падают `an_uncommitted_committee_parks_the_block_instead_of_reverting` и `the_recorder_keeps_the_block_count_equal_to_the_sum_of_its_credits`. При `committee_size == 0` пояс L5 (`leader_index >= committee_size`) ловит тот же случай |
| C6 (нулевой staking-токен при `initialize`) | **избыточна** — `pull_initial_stakes` → `safe_transfer_from` (`util.rs:154-156`) поднимает ТУ ЖЕ ошибку `ERR_ZERO_STAKING_TOKEN`, поэтому генезис с ненулевой ставкой ревёртит одинаково с гейтом и без него |
| S1, S21 (терм `STATUS_ACTIVE` в двух отборах) | **избыточна по инварианту** — R1.5a (`E1-REFLECTION.md`): в `active_validators` попадают только ACTIVE, а любой выход из ACTIVE зовёт `remove_active`; убить терм тестом нельзя в принципе, кроме как записью в хранилище мимо API (что и делает `exclusion_release_skips_…`) |
| L22 (ранний выход при `failures == 0`) | **избыточна** — `stamp` с пустым `failed` не находит `best` и выходит на первом же обороте; это короткое замыкание, не проверка |
| V9 (`visible_at` в `write_validators_with_keys`) | **избыточна** — параметр мёртв: единственный вызов передаёт `None` (R1.6). Удаляется в П5 |
| L3 (закрывать эпоху на каждом блоке) | **нет теста** — открыта |
| L10 (сентинел `readmit == 0` как истёкший) | **нет теста** — открыта |
| L11 (пустой комитет в `judge`) | **нет теста** — открыта |
| L13 (нулевой суммарный вес в `judge`) | **нет теста** — открыта |
| L19 (уже исключённый как «новый отказ») | **нет теста** — открыта |
| L25 (порядок kick-count → адрес при штампе) | **нет теста** — открыта |
| L26 (лестница двигается при отказанном исключении) | **нет теста** — открыта |
| L27 (насыщение лестницы потолком `cap`) | **нет теста** — открыта; связано с Д-13 (потолок недостижим) |
| C20 (выравнивание интервала в `setEpochBlockInterval`) | **нет теста** — открыта |
| C39, C40 (флаг `initializing` — вход и подъём) | **нет теста** — открыта; ветка достижима только вложенным `initialize` из BLS-вызова |
| C41 (нулевой владелец при `initialize`) | **нет теста** — открыта |
| C43 (пятый массив `peer_pubkeys` в проверке арности) | **нет теста** — открыта |
| C44 (потолок комиссии в `initializer::validate`) | **нет теста** — открыта; = M23 из 1.8, там тоже зелёная |
| C46 (короткое замыкание при нулевой сумме ставок) | **нет теста** — открыта |
| C48 (checked_add → saturating_add в сумме ставок) | **нет теста** — открыта |
| V3 (код ошибки декодера) | **нет теста** — открыта; мутация меняет ExitCode, а не поведение |
| V5, V6 (короткие замыкания в `reserve_available`) | **нет теста** — открыты; обе ветки дают тот же ноль другим путём |
| V10 (длина из записи, а не из индекса) | **нет теста** — открыта |
| V14 (код ошибки при калдате короче 4 байт) | **нет теста** — открыта; мутация меняет ExitCode, а не поведение |

Пять «избыточна» показаны механизмом (комбинация, общий код ошибки, инвариант,
мёртвый параметр), не заявлены. Двадцать одна «нет теста» — открытые дыры,
записанные как дыры; закрывать их все в пункте «только тесты» я не стал, объём
не тот.

**Ворота после П4(в), verbatim:**

    cargo test                         → test result: ok. 185 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
    cargo test --features devnet-views → test result: ok. 186 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
    cargo clippy --all-targets -- -D warnings → Finished `dev` profile [optimized] target(s) in 1.15s
    cargo fmt --check                  → чисто

**sha:** `32c1f2f3`.

#### П4 (г). Арифметика

**Объём.** `math.rs` целиком (6 мутаций) и денежные формулы `staking.rs` (37):
оба бинарных поиска по снимкам, лаг отбора `selection_epoch_for` /
`first_reward_epoch_for`, `snapshot_payout`, обход наград владельца и вкладчика,
созревание принципала, `available_for_redelegate`, `assign_epoch_shares`,
`accrue_epoch`, `validator_total_at`. Мутации не «снятие проверки», а сдвиг
арифметики: границы циклов, знаки сравнений, off-by-one в бинарных поисках,
порядок операндов деления.

**Плотность покрытия высокая.** Перестановка двух винтажей комиссии (`min` → `max`)
— 3 упавших; деление доли вкладчика по итогу эпохи закрытия вместо винтажа отбора —
6; перестановка операндов числителя — 9; сдвиг лага на единицу — 9 и 11.

**Одна мутация ловится ЗАВИСАНИЕМ, а не падением.** A8 — округление середины вверх
во всех пяти бинарных поисках `staking.rs` (`low + (high - low).div_ceil(2)`):
поиск перестаёт сходиться, `cargo test` не завершается, прогон пришлось убить.
Записываю как обнаружение, но отдельным видом — assert'а, который бы это назвал,
нет.

**Что закрыто тестами** (три из шести зелёных):

| тест | мутация |
|---|---|
| `the_owner_reward_walk_is_bounded_to_one_thousand_epochs_too` | A20 |
| `a_redelegation_under_the_staking_minimum_is_paid_out_instead` | A31 |
| `materializing_the_same_snapshot_epoch_twice_adds_no_second_entry` | — (см. ниже) |

**Что осталось зелёным:**

| мутация | вердикт |
|---|---|
| A9 (короткое замыкание «эпоха уже в индексе») | **избыточна** — показано по коду и перепроверено мутацией: оба вызывающих недостижимы с уже присутствующей эпохой. `touch_snapshot_at_or_before` (`staking.rs:594-596`) возвращается при `base_epoch == epoch` ДО вызова; `set_validator` (`staking.rs:139`) зовёт один раз на регистрацию. Новый тест `materializing_the_same_snapshot_epoch_twice_adds_no_second_entry` пинит инвариант индекса, но НЕ эту ветку, и говорит об этом прямо |
| A34 (`expand_balance` вместо сырого веса в делении долей) | **эквивалентна по значению** — деление пропорциональное, равномерный множитель сокращается; док-комментарий `staking.rs:2162-2167` утверждает то же и здесь совпал с кодом |
| A37 (`unwrap_or(epoch)` вместо раннего нуля) | **эквивалентна по значению** — снимок несуществующей эпохи читается нулём, `expand_balance(0) == 0` |
| A21 (обход наград владельца на эпоху дальше, `<` → `<=`) | **нет теста** — открыта; лишняя эпоха всегда пуста на фикстурах, потому что кредиты пишутся только в эпохи, которые тест назвал |
| A31 | закрыта тестом |
| A20 | закрыта тестом |

**Ворота после П4(г), verbatim:**

    cargo test                         → test result: ok. 188 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
    cargo test --features devnet-views → test result: ok. 189 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
    cargo clippy --all-targets -- -D warnings → Finished `dev` profile [optimized] target(s) in 1.38s
    cargo fmt --check                  → чисто

**sha:** `9c3c3610`.

#### П4. Тесты, не упавшие ни от одной мутации

163 теста в `tests.rs` (с фичей). 124 из них покраснели хотя бы от одной мутации
ЭТОЙ сессии; из оставшихся 39 двадцать пять названы в таблице мутаций 1.8
(`E1-8-TESTS.md` §2). Остаток — **14 тестов, не упавших ни от одной мутации
никогда**:

| тест | вердикт |
|---|---|
| `a_slash_naming_an_unregistered_key_is_rejected` | проверка не снималась — маршрут улик вне объёма П4 (ждёт Д-4) |
| `close_event_topics_match_the_shared_abi` | проверка не снималась — сверяет два ОБЪЯВЛЕНИЯ (`#[derive(Event)]` против `sol!`), охраняемой проверки в прод-коде под ним нет |
| `compact_storage_matches_solidity_struct_layouts` | проверка не снималась — пинит константы раскладки, порождённые макросом, не ветку кода |
| `contract_storage_uses_separate_erc7201_namespaces` | проверка не снималась — пинит `erc7201_slot!`, не ветку кода |
| `derived_selectors_match_independent_hex_pins` | проверка не снималась — пинит селекторы |
| `devnet_view_selectors_match_their_pinned_ids` | проверка не снималась — пинит селекторы |
| `initialize_events_report_defaults_as_previous_values` | проверка не снималась — содержимое событий инициализации я не мутировал |
| `production_liveness_event_signatures_match_the_solidity_abi` | проверка не снималась — пинит строки сигнатур |
| `production_liveness_views_read_the_new_namespace` | проверка не снималась — четыре devnet-вьюхи суть чистые чтения поля, охраняемой проверки под ними нет (замер П2: все четыре отвечают `Ok` до `initialize`) |
| `solidity_bytes_outputs_and_event_match_cast_vectors` | проверка не снималась — пинит векторы кодировки |
| `staking_is_a_genesis_rwasm_contract_not_a_system_precompile` | проверка не снималась — структурное утверждение о сборке |
| `every_config_setter_refuses_an_uninitialized_contract_before_it_checks_anything_else` | проверка СНИМАЛАСЬ и тест красный — мутация M-A (П2), не мутация «своего класса» |
| `the_two_views_that_reach_the_epoch_formula_refuse_an_uninitialized_contract` | проверка СНИМАЛАСЬ и тест красный — снятие двух `ensure_initialized` (П2) |
| `materializing_the_same_snapshot_epoch_twice_adds_no_second_entry` | проверка снималась (A9) и тест НЕ упал — но «избыточен» здесь неверно: ветка недостижима от обоих вызывающих, поэтому никакая мутация её не может покрасить ничего. Ни один из трёх вердиктов пункта не подходит; см. §3 Д-06 |

### П5 (W6). Вестигии переноса комитета

| вестигия | что сделано | опора |
|---|---|---|
| `write_ring_compact` разделён ради удалённого переноса | свёрнут в `write_ring`; проверка длины кольца (`ERR_COMMITTEE_EXCEEDS_WEIGHT_RING`) переехала ВПЕРЁД компакции, по счёту членов | док самой функции признавал причину разделения («a second caller that is now gone») и назвал препятствие к свёртке — `IntegerOverflow` внутри store-прохода; порядок проверок это препятствие снимает |
| две перепроверки «duplicated, unreachable, kept» в `store_consensus_keys` | удалены | `consensus.rs:118` и `:129` — обе ошибки поднимает `verify_consensus_keys`; оба вызывающих зовут её непосредственно перед (`initializer.rs:65-66`, `staking.rs:1086-1087`); `grep -c 'peer_pubkey_owner_accessor\|bls_pubkey_owner_accessor' src/staking.rs` = 0, то есть `set_validator` между ними ключевые карты не трогает |
| мёртвый `visible_at` в `write_validators_with_keys` | параметр и его ветка удалены | единственный вызов передавал `None` (R1.6); мутация V4-класса **V9** (снять маскировку) была ЗЕЛЁНОЙ — ветка недостижима |
| `effective_epoch` — док против кода (R1.4) | **ничего не потребовалось** | док поля (`events.rs:46-58`) уже описывает то, что пишет код: «an ANNOUNCEMENT, not a rule», и прямо говорит, что поле никто не читает. Перепроверил утверждение: `grep -rn "effectiveEpoch\|effective_epoch\|ActiveValidatorsLengthChanged" crates/ devnet/ e2e/ bins/` — пусто. R1.4 описывает состояние до правки, которой она добилась |
| `STATUS_ACTIVE` (R1.5a) | **оставлен**, задокументирован как утверждение инварианта | см. §3 Д-08 |

**README контракта** сверен с деревом. Правил только ложь — три места:

1. Раздел про эквивокацию утверждал «A recipient that refuses the transfer does not roll
   the slash back». П3 это перевернул; абзац переписан вместе с ценой.
2. «Every revert reachable inside a system call stops the chain» — по коду узла
   (`crates/node/src/evm.rs:1184-1190`, прочитано в П3) `slashEquivocation` МЯГКО
   сворачивается: warn, состояние не коммитится. Абзац теперь называет исключение —
   и это то, что делает пункт 1 переживаемым.
3. «Source Layout» перечислял шесть файлов из пятнадцати. Дополнен до полного.

Не правил: раздел «Solidity parity» (соседнее дерево, вне объёма), таблица аудита
событий (не проверял), «Verification» (команды рабочие).

**Ворота после П5, verbatim:**

    cargo test                         → test result: ok. 188 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
    cargo test --features devnet-views → test result: ok. 189 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
    cargo clippy --all-targets -- -D warnings → Finished `dev` profile [optimized] target(s) in 1.15s
    cargo fmt --check                  → чисто

Ни одного нового и ни одного удалённого теста: свойство, которое каждая правка
затрагивает, уже было запинено (или, для `visible_at`, доказано недостижимым).

**sha:** `9d4fa617`.

### П6 (1.3). Таймлок на двух адресных сеттерах

**Схема.** `setSlashFundAddress` и `setBlendReserve` больше не записывают значение —
они его ЗАЯВЛЯЮТ. Пара `(адрес, эпоха заявки)` ложится в хранилище, летит событие
заявки. Новые `applySlashFundAddress()` / `applyBlendReserve()` переносят заявленное
в живое поле, если `epoch_now >= declared + ADDRESS_SETTER_TIMELOCK_EPOCHS` (7);
иначе ревёрт. Существующее `…Changed` теперь летит на ПРИМЕНЕНИИ — там, где значение
меняется. Повторная заявка перезаписывает пару и сбрасывает срок.

**Единица — эпохи, и это проверено, а не унаследовано.** `grep -rn 'block_timestamp'
contracts/staking/src` — пусто; эпоха текущего блока берётся `util::current_epoch`,
тем же вызовом, каким её берёт `apply_production_exclusion` (`staking.rs:296`).

**Сентинел «ничего не заявлено» — НУЛЕВОЙ АДРЕС, не нулевая эпоха.** Оба заявляющих
сеттера отказывают нулевому адресу (`ERR_ZERO_VALUE`, было и до правки), поэтому
сентинел недостижим законной заявкой; эпоха 0 — обычная эпоха для заявки и сентинелом
служить не может. Тест `the_address_timelocks_are_governance_only_on_both_halves`
гоняет именно это: отказ нулю, а сразу за ним — «применить» отвечает
`ERR_NO_PENDING_CHANGE`.

**Хранилище.** Четыре поля ДОПИСАНЫ в конец `ChainConfigStorage`
(`pending_slash_fund_address`, `pending_slash_fund_epoch`, `pending_blend_reserve`,
`pending_blend_reserve_epoch`). Дописаны, а не вставлены: поле, вставленное выше,
сдвигает все слоты ниже.

**ABI.** Два селектора (`applyBlendReserve()` `0x47a9615b`,
`applySlashFundAddress()` `0x7bb69756`) и два события (`BlendReserveDeclared`,
`SlashFundAddressDeclared`, оба indexed по адресу) — в общий `sol!`; `consts.rs`
выводит селекторы через `sig::`, строк-литералов не заведено. `setSlashFundAddress`
оставлен в `consts.rs` — см. §3 Д-09.

**Тесты.** `the_address_setters_declare_now_and_land_seven_epochs_later` — все пять
требуемых свойств, по обоим сеттерам: применение раньше срока ревёртит (и ревёрт
называет обе эпохи), ровно на 7-й проходит, заявка эмитит событие и НЕ трогает живое
значение, применение без заявки ревёртит, повторная заявка сдвигает срок. Плюс
контроль: между этими шагами вьюха читается и сверяется.
`the_address_timelocks_are_governance_only_on_both_halves` — обе половины обоих
сеттеров под governance, и отказ нулю. **Были ли красными до правки: вопрос не
применим — оба теста описывают механизм, которого до этой правки не существовало
(ни одного отложенного применения в контракте не было).** Что их держит — мутации;
не ставил, см. §7.

**Правки вне контракта, вынужденные ABI.** `e2e/src/staking_reserve.rs` ротирует
резерв в трёх местах; добавлен `Fixture::rotate_reserve(value, at_epoch)` — заявить,
прыгнуть на 7 эпох вперёд по номеру блока, применить, вернуть высоту. Прыжок
безопасен: ни один из двух вызовов не пишет `last_processed_block`. Плюс два теста
контракта, которые ставили фонд/резерв напрямую, переведены на помощник
`rotate_address_setting`.

**Приёмка 1.3, которую я НЕ трогал, и почему её формулировка в задании неверна.**
`devnet/local-dpos-smoke/dpos_harness/tests/test_prod_substrate.py:981` — это
юнит-тест ФОРМАТТЕРА вердикта (`gov_wait_verdict`), а `"setBlendReserve"` в нём —
произвольная строка-описание, передаваемая как `desc=`. Никакого вызова
`setBlendReserve` через governance харнесс не делает: `grep -rn 'setBlendReserve'
devnet/` даёт ровно эти две строки и ничего больше. То есть открытой приёмки
«харнесс prod-кейса ждёт двухшагового пути» не существует — записал это в `PLAN.md`
строкой 1.3 как проверенный факт, а не как открытый пункт.

**Ворота после П6 (контракт), verbatim:**

    cargo test                         → test result: ok. 190 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
    cargo test --features devnet-views → test result: ok. 191 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
    cargo clippy --all-targets -- -D warnings → Finished `dev` profile [optimized] target(s) in 2.42s
    cargo fmt --check                  → чисто

**sha:** `b237a81e` (без блоба — см. §3 Д-10).

### П7 (R5.2). `claimValidatorFeeAtEpoch` удалён

**Что удалено.** Хендлер (`staking.rs`), `SIG_CLAIM_VALIDATOR_FEE_AT_EPOCH`
(`consts.rs`), строка диспетчера (`lib.rs`), `ValidatorEpochCommand` (`types.rs`) —
он был её единственным декодером — и `ERR_INVALID_CLAIM_EPOCH` (`consts.rs`),
поднимавшийся только им. Проверено grep'ом после удаления: ни одного вхождения не
осталось. В общем `sol!` его не было (нет вызывающих вне крейта), так что там ничего
не двигалось.

**Кого ломает вне контракта.** Никого: `grep -rn 'claimValidatorFeeAtEpoch'
crates/ e2e/ bins/ devnet/` до правки давал только `STAKING_ARTEFACT.md` (три
селекторных скана и одна строка прозы) и сам контракт.

**Тесты.** Два использовали его.
`a_commission_rise_misses_two_epochs_while_a_cut_lands_on_the_next` брал окно
аргументом, чтобы предъявлять по одной эпохе; теперь шагает НОМЕРОМ БЛОКА и зовёт
`claimValidatorFee`, который идёт до текущей эпохи — тот же обход, только окно
берётся с часов. `reward_claims_are_bounded_to_one_thousand_epochs` терял
утверждение про окно за пределами текущей эпохи — оно ушло вместе с хендлером, о
котором было.

**Ворота после П7, verbatim:**

    cargo test                              → test result: ok. 190 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
    cargo test --features devnet-views      → test result: ok. 191 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
    cargo clippy --all-targets -- -D warnings → Finished `dev` profile [optimized] target(s) in 2.19s
    cargo fmt --check                       → чисто

**Общие ворота (после П6 и П7 вместе, на собранном блобе), verbatim:**

    cargo test -p fluentbase-node --lib        → test result: ok. 57 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 56.82s
    cargo test -p fluentbase-staking-reader    → test result: ok. 58 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
                                                 (и ещё один бинарь: 0 passed; 0 failed; 1 ignored)
    cargo test -p fluentbase-e2e --release staking → test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 118 filtered out; finished in 35.15s
    python3 devnet/local-dpos-smoke/scripts/xp/agreement_check.py → === 15 checks, 0 disagree, 0 unread ===
    селекторный скан .rwasm                    → selector scan OK

`cargo test -p fluentbase-genesis-bootstrap` прогнан после П1 (8/0); после П6/П7 не
перезапускался — записано в §7 как слабое место.

**Блоб.** Собран один раз после обоих пунктов:
`cargo build --release -p fluentbase-genesis --features devnet-views`, копии руками
в `devnet/local-dpos-smoke/contracts/`. wasm 405 682 Б
(`53ce6029…`), rwasm 2 800 125 Б (`620b79bb…`), обе выросли — +3 904 и +24 788.
Новая датированная секция `STAKING_ARTEFACT.md` написана целиком по её же правилу:
HEAD на момент сборки (`b237a81e`), список грязных файлов, SHA-256 каждого исходника,
размеры, дайджесты, обновлённый селекторный скан с двумя новыми в `must_be_1` и
`claimValidatorFeeAtEpoch`, переехавшим в `must_be_0`.

**sha:** `0b9daeac` (код + блоб + артефакт).

### П8 (1.6, K-13). Строгость к калдате — обёртка на стороне контракта

**Сначала перечисление, без правок.** Все вызывающие `SolidityABI::…::decode` /
`decode_function_args` в дереве (`grep` по `crates/ contracts/ bins/ e2e/ devnet/`,
09-11), кроме тестов:

| место | что декодирует | что сломает строгость |
|---|---|---|
| `contracts/staking/src/util.rs` (7 вхождений) | калдату контракта и ответы ERC-20 | предмет этого пункта |
| `crates/sdk/src/universal_token/command.rs:26` | калдату universal-token | любой вызывающий, который добивает аргументы до кратности или шлёт широкие слова |
| `crates/sdk/src/universal_token/storage.rs:81` | ХРАНИЛИЩЕ (`Self` из буфера) | чтение слота, чей писатель не обязан был писать канонично |
| `crates/sdk/src/universal_token/storage.rs:167,179` | ХРАНИЛИЩЕ, `InitialSettingsV1` и `InitialSettingsV2` **из одного и того же payload'а перебором**, берётся та версия, что распарсилась | **строгость ломает сам механизм версионирования**: перебор работает ровно потому, что декодер не требует, чтобы буфер соответствовал типу точно |
| `crates/sdk/src/storage_legacy.rs:41` | ХРАНИЛИЩЕ, с откатом на `T::default()` при ошибке | превращает часть успешных чтений в молчаливые дефолты |
| `contracts/webauthn/src/lib.rs:44,65` | калдату другого контракта | его вызывающих |
| `crates/codec/src/evm.rs` (5) | внутренности самого кодека | — |
| `e2e/src/universal_token_solidity.rs`, `e2e/src/router.rs`, тесты | тесты | — |

**Вывод: ломает чужих**, и не гипотетически — три чтения хранилища и два чужих
контракта, из них `storage.rs:167,179` держатся на нестрогости как на механизме.
Значит, по правилу пункта — SDK не трогаем, ветвь обёртки.

**Что именно нестрого — измерено, а не вычитано.** Пробник по трём формам на
неправленом контракте:

    PROBE clean_len=36 padded_len=68
    PROBE tail: Ok
    PROBE truncated: MalformedBuiltinParams
    PROBE wide u32: Ok sel=None cap 21 -> 7

То есть: хвост 32 байта — ПРИНЯТ; буфер короче слова — уже отвергался и до правки;
слово `uint32` с установленным 33-м битом — ПРИНЯТО и молча усечено, причём
`setActiveValidatorsLength` записал 7, а его потолок `MAX_COMMITTEE_SIZE` настоящего
числа не видел. Из трёх форм K-13 одна была закрыта, две — нет. Пробник после замера
удалён.

**Обёртка.** `util::decode` (статический путь) после успешного декода ПЕРЕ-КОДИРУЕТ
значение и сверяет байты с входом. Одна проверка на обе дыры: хвост меняет длину,
усечённое целое меняет старшие байты своего слова. Знания о полях не требует — а
проверка ширины требовала бы, и на этом уровне его нет. `ExitCode::MalformedBuiltinParams`,
тот же код, каким уже отвечал короткий буфер.

**Чего обёртка НЕ покрывает, и почему** — `decode_args`: её зовут `initialize` и три
маршрута улик, все с ДИНАМИЧЕСКИМИ кортежами, у которых законные кодировки
различаются раскладкой смещений, поэтому побайтное равенство отвергало бы корректную
калдату. Эти две дыры остаются открытыми и записаны как открытые — в коде, в
`STAKING_ARTEFACT.md` и здесь.

**Тест.** `a_static_argument_tuple_refuses_a_tail_and_a_too_wide_integer` — хвост из
мусора, хвост из НУЛЕЙ (та же дыра, пройти легче: декодируется в то же значение),
короткий буфер, широкое целое; плюс два контроля — канонический вызов проходит, и те
же младшие байты с чистым старшим словом записываются. **Был ли красным до правки:
да** — прогнан против обёртки, заглушённой на `if false`:
`test result: FAILED. 190 passed; 1 failed`, упал ровно он.

**Ворота после П8, verbatim:**

    cargo test                              → test result: ok. 191 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
    cargo test --features devnet-views      → test result: ok. 192 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
    cargo clippy --all-targets -- -D warnings → Finished `dev` profile [optimized] target(s) in 3.53s
    cargo fmt --check                       → чисто
    cargo test -p fluentbase-node --lib        → test result: ok. 57 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 61.72s
    cargo test -p fluentbase-staking-reader    → test result: ok. 58 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
    cargo test -p fluentbase-e2e --release staking → test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 118 filtered out; finished in 30.79s
    python3 devnet/local-dpos-smoke/scripts/xp/agreement_check.py → === 15 checks, 0 disagree, 0 unread ===
    селекторный скан .rwasm                 → selector scan OK

Эти четыре и есть свидетельство, что строгость никого в дереве не отвергает: каждый
из них шлёт калдату ПРОТИВ этого блоба.

`cargo test -p fluentbase-codec -p fluentbase-sdk -p fluentbase-testing` НЕ прогонялся
— эти ворота задание привязывает к ветви «править SDK», а SDK не правился.

**Блоб.** Пересобран (селекторы не двигались, скан тот же и перепрогнан): wasm
409 431 Б `da3137f0…`, rwasm 2 815 430 Б `df7605b6…`. Отдельная датированная секция
`STAKING_ARTEFACT.md` — рост в 15 305 байт rwasm объяснён там как пере-кодирующий
проход, инстанцируемый по одному на каждый статический командный тип.

**sha:** `9cd156db`.

### П9 (контр-ревью). Две правки одной сессии противоречили друг другу

Не пункт задания. `/code-review` в свежем контексте по `contracts/staking` +
`crates/staking-abi` целиком, с этим журналом на вход как с набором утверждений
для опровержения. Семь замечаний; четыре его утверждения я перепроверил сам по
коду перед тем, как что-то менять, три принял и пометил как relay, пока не проверил
тестом.

**Что не удалось сломать** (его работа, мой relay): обёртка декодера П8 — все
достижимые impl'ы байт-симметричны и Solidity-канонические, все шесть вызывающих
`decode_args` действительно динамические, все внешние вызывающие в дереве кодируют
через `abi_encode`/`cast`; Д-08 про `STATUS_ACTIVE` — перечислены все записи `status`
и все они идут после `remove_active` или с push'ем; П5 — оба вызывающих, включая цикл
`initialize`; четыре новых селектора и топика пересчитаны `cast`; SHA-256 артефакта
сходятся с деревом.

#### Находка 1 (серьёзная): оправдание П3 отменено П6

**Проверено мной:** `consensus.rs:798-801` — burn sink берётся, только если ХРАНИМЫЙ
`slash_fund_address` нулевой; `config.rs:365` — сеттер отказывает нулю. Значит после
любого ненулевого фонда вернуться к сжиганию нельзя ничем.

Сцепление: `3c4560db` (П3) сделал отказ фонда ревёртом всего наказания и обосновал
цену тем, что «governance перенаправит `slashFundAddress`». `b237a81e` (П6),
закоммиченный ПОСЛЕ, поставил этот сеттер за 7 эпох. Итог, которого не видно ни в
одном из двух дифов по отдельности: токен блокирует адрес фонда → ни один эквивокатор
не наказуем, узел мягко сворачивает ревёрт и продолжает производить, лечение — неделя,
всю которую нарушитель сидит с местом, ставкой и наградами.

**Решение владельца:** вывести `setSlashFundAddress` из-под таймлока (спрошено и
получено 2026-09-11). Резерв таймлок сохраняет. Обоснование — асимметрия угрозы:
резерв это счёт, ОТКУДА тянут стипендию, и украденный ключ может направить его на
себя; фонд это адрес, КУДА уходит конфискат, и подмена его не обогащает, зато
задержка ломает единственный путь ремонта.

#### Находка 2: заявку нельзя отменить и она не протухает

**Проверено мной:** `pending_*` пишется только в заявке (`config.rs:371/374`) и
чистится только в применении (`:403/406`). Сеттер отказывает нулю, поэтому вернуться
в «ничего не заявлено» без применения было нечем. Срока годности не было.

Инверсия гарантии: заявка, сделанная и брошенная, остаётся заряженной навсегда;
ключ, украденный через полгода, приземляет её одним вызовом, и «семь эпох публичного
уведомления» давно прошли мимо всех.

**Решение владельца:** делаем отмену и срок годности. Реализовано:
`ADDRESS_SETTER_APPLY_WINDOW_EPOCHS = 7` (равно сроку — и это ВСЯ деривация:
действовать дают ровно столько же, сколько ждать), ошибка `ERR_TIMELOCK_EXPIRED`,
хендлер `cancelBlendReserve()` и событие `BlendReserveDeclarationCancelled`.

#### Находка 3: заявку не видно ни одной вьюхой

**Проверено мной:** `grep pending src/lib.rs` — в диспетчере ничего (единственное
совпадение `SIG_PENDING_EXCLUSIONS` — про исключения живучести, другое). Добавлена
`getPendingBlendReserve()` → `(адрес, эпоха заявки, эпоха применимости, эпоха
протухания)`, четыре нуля когда ничего не заряжено. Именно заброшенную заявку — ту,
про которую находки 1 и 2, — иначе не найти иначе как проигрыванием лога с генезиса.

#### Находка 4: до активации таймлок не стоит ничего

**Проверено мной тестом, а не рассуждением.** `the_timelock_is_not_a_bound_before_dpos_activates`
проводит весь путь: заявка на эпохе 0 → `setEpochBlockInterval(1)` → 
`setDposActivationBlock(51)` → на блоке 58 применение проходит. Восемь блоков
уведомления вместо семи эпох.

**Не чиню, фиксирую тестом.** До активации ничего не застейкано, ничего не
зарабатывает, а резерв governance и так задаёт прямо в `initialize`; модель угрозы
таймлока — скомпрометированный ключ на РАБОТАЮЩЕЙ цепи, и `ensure_dpos_not_active`
закрывает геометрические сеттеры в тот момент, когда она ею становится. Тест стоит
для того, чтобы правка, которая протащит эту дыру за активацию, была падением, а не
сюрпризом.

#### Находки 5–7: три дока против кода, все три созданы этой сессией

- `util.rs` — док `try_transfer` утверждал «A refusal is a value here, not a
  revert… This caller must not revert». Единственный вызывающий с П3 ревёртит на
  каждом отказе. Переписан: «значение, а не ревёрт» — про то, КТО решает, а не про
  то, что решено; решает вызывающий, и сегодня он решает ревёртить.
- `consensus.rs` — обоснование «дубликат это гонка, ревёрт превратил бы её в
  проваленный pre-execution вызов» читалось как правило, которое тот же хендлер
  нарушает двумя сотнями строк ниже. Дописано, в чём разница: отказ платежа чинится
  оператором, а дубликат — два честных предлагателя с одной уликой, чинить нечего.
- `consensus.rs` — `store_consensus_keys` после удаления двух перепроверок (П5)
  осталась без локального свидетеля для write-once карт владельцев, а
  `slash_from_evidence` резолвит личность эквивокатора именно через
  `bls_pubkey_owner`. Контракт вызывающего записан в док функции явно: третий
  вызывающий, пропустивший верификацию, не испортит регистрацию — он отправит
  будущий слэш не тому валидатору.
- `README.md` нёс ту же ложь про восстановление после отказа фонда; переписан, плюс
  добавлен абзац про асимметрию двух адресных сеттеров.

**Тесты.** Четыре новых, каждый прогнан против кода без своей правки:

| тест | мутация, против которой он красный |
|---|---|
| `the_reserve_setter_declares_now_and_lands_seven_epochs_later` | (переписан из старого; пять свойств + новая вьюха) |
| `an_abandoned_declaration_expires_and_can_be_withdrawn` | снять проверку протухания → КРАСНЫЙ; отмена, не чистящая пару → КРАСНЫЙ |
| `the_slash_fund_rotates_immediately_because_a_refused_seizure_needs_it` | сеттер, не записывающий значение → КРАСНЫЙ (плюс ещё два теста) |
| `the_reserve_timelock_is_governance_only_on_every_mutating_half` | (восстанавливает покрытие, потерянное при переписывании) |
| `the_timelock_is_not_a_bound_before_dpos_activates` | пин границы, не правки — см. находку 4 |

**Ворота после П9, verbatim:**

    cargo test                              → test result: ok. 194 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
    cargo test --features devnet-views      → test result: ok. 195 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
    cargo clippy --all-targets -- -D warnings → чисто
    cargo fmt --check                       → чисто
    cargo test -p fluentbase-staking-abi       → test result: ok. 2 passed; 0 failed
    cargo test -p fluentbase-node --lib        → test result: ok. 57 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 60.79s
    cargo test -p fluentbase-staking-reader    → test result: ok. 60 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
    cargo test -p fluentbase-e2e --release staking → test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 118 filtered out; finished in 31.53s
    python3 devnet/local-dpos-smoke/scripts/xp/agreement_check.py → === 15 checks, 0 disagree, 0 unread ===
    селекторный скан .rwasm                 → selector scan OK

`staking-reader` 58 → 60: чужая сессия закоммитила своё между прогонами
(HEAD сдвинулся с `9cd156db` на `3a4a0fde`), к этой правке отношения не имеет.

**Блоб.** Пересобран, отдельная датированная секция `STAKING_ARTEFACT.md`: wasm
410 748 Б `6caa4f7c…`, rwasm 2 825 599 Б `4040ef11…`. Два новых селектора и топик
пересчитаны `cast sig` / `cast keccak` НЕЗАВИСИМО от `alloy` и только потом
запинены с обеих сторон.

**Главный урок сессии, который стоит дороже любой отдельной находки.** Два пункта
задания были написаны как независимые, я их такими и делал, и ни один диф по
отдельности проблему не показывал. Контр-ревью нашло её ровно потому, что смотрело на
диапазон целиком, а не на пункт. При следующем многопунктовом задании стоит явно
спрашивать: какое обоснование, написанное в раннем пункте, отменяет поздний.

**sha:** `0f283a82`.

## §3 Отклонения Д-nn

### Д-01 (П1). Три способа, которыми правка задела существующие тесты

Плановая строка 1.6 обещала только «добавить тот же гейт»; по факту гейт сломал
14 тестов. Разбор по коду, не по догадке:

1. `liveness_harness` (`tests.rs:7942`) принимает `cap` параметром и пишет его прямо
   в `InitializeCommand`; 11 из 26 её вызовов просят cap 2. Правка: инициализировать
   с `cap.max(MIN_COMMITTEE_LENGTH)`, а под-половый cap дописывать в
   `chain_config_storage().active_validators_length_accessor()` уже после
   `initialize`. Причина, почему это не ослабление: cap ниже пола и до правки не давал
   коммитируемого комитета (`commitEpochCommittee` держит тот же пол), и каждый из этих
   вызовов и так сажает комитет `commit_test_committee`, а не через реальный хендлер.
   Cap в этих тестах управляет шириной среза отбора, и ничем больше.
2. `exclusion_release_skips_tombstoned_and_non_active_validators` (`tests.rs:7648`)
   задавал cap 1. Поднят до `MIN_COMMITTEE_LENGTH` — тест зелёный без других правок,
   то есть значение cap в нём было безразличным.
3. `the_verdict_floor_is_a_stake_share_at_the_production_epoch_length`
   (`tests.rs:8577`) задавал cap 2 при двух валидаторах. Поднят до
   `MIN_COMMITTEE_LENGTH` — зелёный без других правок: cap только усекает отбор, а
   усекать двух валидаторов до четырёх нечего.

Решение `DECISIONS.md` это не меняет.

### Д-02 (П1). Код ошибки для cap 0 при `initialize` сменился

До правки cap 0 давал `ERR_INVALID_CHAIN_CONFIG`, теперь даёт
`ERR_ACTIVE_VALIDATORS_LENGTH_BELOW_COMMITTEE_FLOOR`. Это прямое следствие требования
«тот же код ошибки, что у сеттера» (сеттер на 0 отвечает именно полом). Ни один тест
в дереве не пинил cap 0 (`grep 'active_validators_length = ' src/tests.rs` — 13 мест,
нуля среди них нет). Решение `DECISIONS.md` это не меняет.

### Д-08 (П5). `STATUS_ACTIVE` оставлен, а не удалён

Строка пункта — «`STATUS_ACTIVE` (R1.5a) — по рефлексии», а W6 перечисляет терм среди
вестигий, то есть подразумевает удаление. Я его оставил. Причина по коду, а не по
осторожности: терм стоит В ДВУХ местах — `selected_committee_at` (`consensus.rs`) и
`eligible_population_at_least` (`staking.rs`), и во втором он нагружен, что доказано
существующим тестом `exclusion_release_skips_tombstoned_and_non_active_validators`
(`tests.rs`), который пишет `STATUS_PENDING` прямо в хранилище и утверждает, что
популяция считает шесть, а не семь. Удалить терм в одном месте и оставить в другом —
это сделать две функции несогласными о том, кто пригоден; удалить в обоих — сломать
тест, который эту несогласованность и ловит. Вместо удаления терм назван в коде тем,
чем является: утверждением инварианта, который через API не нарушить (мутации S1 и S21
зелёные — измерено в П4в), и который ловит запись мимо API. Решение `DECISIONS.md`
это не меняет; W6 в части этого пункта считаю закрытым документированием, а не
удалением.

### Д-09 (П6). `setSlashFundAddress` не переехал в общий `sol!`

Задание говорит «два новых селектора и два события в общий `sol!`» — ровно это и
сделано. Но заявляющая половина `applySlashFundAddress` — `setSlashFundAddress` — в
общем крейте не была и не переехала: правило объёма самого крейта
(`crates/staking-abi/src/lib.rs`, шапка) — «в общий `sol!` идёт хендлер, который
кодирует БОЛЕЕ ОДНОГО места вне `contracts/staking`», а у `setSlashFundAddress`
таких мест ноль (`grep` по `crates/ bins/ e2e/ devnet/` — только сам контракт).
Перенос ради симметрии расширил бы крейт против его собственного правила. Асимметрия
(apply в крейте, set — нет) названа в доке крейта прямо, со ссылкой сюда. Решение
`DECISIONS.md` это не меняет.

### Д-10 (П6, П7). Коммит таймлока без блоба

Задание разрешает одну секцию `STAKING_ARTEFACT.md` на П6+П7, если блоб собран один
раз после обоих — «тогда П6 коммитится без блоба, и это записано в §3». Так и
сделано: `b237a81e` (таймлок) не несёт блоба, блоб собран после удаления
`claimValidatorFeeAtEpoch` и лежит в коммите П7 вместе с единственной новой секцией
артефакта. Промежуточного блоба (таймлок есть, удаления ещё нет) не существовало и в
`STAKING_ARTEFACT.md` он не описан — там это сказано прямо, чтобы дыра в датах
читалась как решение, а не как пропуск. Решение `DECISIONS.md` это не меняет.

### Д-04 (П4б). Премиса пункта про M53 неверна по замеру

Пункт требовал показать, что два теста «депозит выходит из баланса контракта»
краснеют при M53 ПОСЛЕ правки заглушки и были зелёными ДО. Замер: M53 красная и до,
и после — правка заглушки на неё не влияет; `E1-8-TESTS.md` F-3 говорит то же
своими словами. Достаточность измеряется мутацией M53b (`safe_transfer` глотает
неуспешный статус), которая была зелёной против ВСЕХ тестов дерева и красная после.
Решение `DECISIONS.md` это не меняет.

### Д-05 (П4в, П4г). Форма таблицы мутаций отличается от `E1-8-TESTS.md` §2

При более чем шести упавших тестах перечислены первые три и общее число. Причина:
у восьми мутаций упавших 20–41, и полные списки заняли бы страницы нечитаемого
текста. Полные списки лежат в
`scratchpad/mut/*.results.json` этой сессии, но это не часть репозитория и
переживёт её только в этом абзаце — то есть для будущей сессии эти восемь строк
восстановимы только перепрогоном. Решение `DECISIONS.md` это не меняет.

### Д-06 (П4в). Одному тесту не подходит ни один из трёх вердиктов

`materializing_the_same_snapshot_epoch_twice_adds_no_second_entry`: проверка его
класса снималась (A9), тест не упал, но «избыточен» неверно — ветка недостижима от
обоих вызывающих (`staking.rs:594-596` возвращается раньше; `staking.rs:139` зовёт
один раз), поэтому НИКАКОЙ тест её покрасить не может. «не измерим на заглушке» и
«проверка не снималась» тоже ложны. Записываю четвёртым вердиктом с механизмом
вместо того, чтобы подогнать под три; подгонка была бы тем самым «вероятно»,
которое пункт запрещает. Решение `DECISIONS.md` это не меняет.

### Д-07 (П4в). Пункт «только тесты» оставил 21 дыру открытой

45 зелёных мутаций классов; закрыто тестами 18, показано избыточными 5, осталось
21 «нет теста». Закрывать все в одном коммите я не стал: это было бы три-четыре
десятка новых тестов в пункте, чей заявленный объём — «снять, прогнать, записать,
вернуть». Каждая записана в §4 и в таблице вердиктов с механизмом, который она
пропускает. Решение `DECISIONS.md` это не меняет; кандидат в отдельную работу.

## §4 Мутации

Форма — как `history/E1-8-TESTS.md` §2. «До» и «после» относятся к состоянию тестов
этой сессии, а не к 09-08.

| M | что снято | до правки тестов | упавшие тесты | после правки |
|---|---|---|---|---|
| M-A | util.rs ensure_governance: drop the ensure_initialized call | — (проверка не снималась 09-08) | — | КРАСНАЯ (1): `every_config_setter_refuses_an_uninitialized_contract_before_it_checks_anything_else` |
| M65 | bls.rs compress_g2_unchecked: undo the EIP-2537↔zcash half swap | КРАСНАЯ (2) по замеру 09-08 (`E1-8-TESTS.md` F-1) | `g2_compression_swaps_the_halves_and_reads_the_sign_from_c1`, `the_y_sign_bit_is_strictly_above_half_the_field` | КРАСНАЯ (5): те же два + `get_consensus_keys_matches_dynamic_struct_return_vectors`, `register_validator_cast_calldata_registers_consensus_keys_atomically`, `register_validator_verifies_and_stores_consensus_keys_in_one_call` |
| M53 | staking.rs withdraw_delegator_principal_before: pay the principal off the reserve | КРАСНАЯ (3) | `a_deposit_the_contract_cannot_cover_is_not_paid_out_of_the_reserve`, `a_reward_and_a_matured_principal_claim_are_independent`, `the_delegator_views_report_the_reward_and_the_deposit_apart` | КРАСНАЯ (3), те же — правка заглушки на эту мутацию не влияет |
| M53b | util.rs safe_transfer: swallow a failed transfer status | **ЗЕЛЁНАЯ** (178/0 на `79152ea2`) | — | КРАСНАЯ (1): `a_deposit_the_contract_cannot_cover_is_not_paid_out_of_the_reserve` |

### Классы П4(в) и арифметика П4(г)

116 + 43 мутаций. Столбец «до правки» — первый прогон, до тестов этой сессии;
«после» заполнен только для мутаций, перепрогнанных после добавления тестов
(остальные — «—», они были красными сразу). Отличие формы от `E1-8-TESTS.md` §2:
при более чем шести упавших тестах перечислены первые три и общее число —
списки на сорок имён нечитаемы (§3, Д-05).

| M | что снято | до правки | упавшие тесты | после правки |
|---|---|---|---|---|
| A1 | compact_balance: drop the precision-remainder refusal | КРАСНАЯ (4) | `governance_updates_embedded_chain_configuration`, `initialize_refuses_every_malformed_chain_configuration`, `math::compact_balance_rejects_precision_dust`, `the_governance_setters_refuse_every_value_they_are_supposed_to` | — |
| A2 | compact_balance: multiply by the precision instead of dividing | КРАСНАЯ (41) | `a_committee_stays_readable_far_past_the_retired_pruning_horizon`, `a_delegator_who_left_after_the_selection_still_earns_it_at_the_older_rate`, `a_deposit_the_contract_cannot_cover_is_not_paid_out_of_the_reserve`, … (всего 41) | — |
| A3 | expand_balance: divide by the precision instead of multiplying | КРАСНАЯ (41) | `a_clean_run_retires_the_kick_ladder_and_one_epoch_short_does_not`, `a_committee_stays_readable_far_past_the_retired_pruning_horizon`, `a_delegator_who_left_after_the_selection_still_earns_it_at_the_older_rate`, … (всего 41) | — |
| A4 | narrow_reward: drop the uint96 overflow refusal | КРАСНАЯ (1) | `math::reward_narrowing_rejects_uint96_overflow` | — |
| A5 | epoch_at_block: drop the unarmed-activation sentinel | КРАСНАЯ (3) | `dpos_activation_at_block_zero_remains_configurable`, `math::unarmed_activation_pins_the_epoch_regardless_of_height`, `scheduling_activation_never_moves_the_epoch_backwards` | — |
| A6 | epoch_at_block: return the raw epoch instead of clamping the unarmed chain to 0 | КРАСНАЯ (3) | `dpos_activation_at_block_zero_remains_configurable`, `math::unarmed_activation_pins_the_epoch_regardless_of_height`, `scheduling_activation_never_moves_the_epoch_backwards` | — |
| A7 | insert_snapshot_epoch: lower-bound search uses <= instead of < (both copies) | КРАСНАЯ (20) | `a_commission_rise_misses_two_epochs_while_a_cut_lands_on_the_next`, `a_delegator_who_left_after_the_selection_still_earns_it_at_the_older_rate`, `a_raised_delegation_minimum_does_not_govern_the_owner_self_stake`, … (всего 20) | — |
| A9 | insert_snapshot_epoch: drop the already-present short circuit | ЗЕЛЁНАЯ |  | ЗЕЛЁНАЯ |
| A10 | insert_snapshot_epoch: shift loop stops one short (index > low + 1) | КРАСНАЯ (6) | `a_seizure_stops_the_seized_bond_counting_as_stake`, `commission_change_carries_forward_without_copying_future_stake_backward`, `future_delegation_and_noop_commission_do_not_bypass_warmup`, `sparse_snapshot_lookup_uses_sorted_materialized_epochs`, `the_accrual_path_credits_an_epoch_that_is_still_divided_two_epochs_back`, `validator_owner_cannot_drop_below_minimum_while_delegators_remain` | — |
| A11 | latest_snapshot_epoch_at_or_before: binary search uses < instead of <= | КРАСНАЯ (27) | `a_commission_rise_misses_two_epochs_while_a_cut_lands_on_the_next`, `a_committee_stays_readable_far_past_the_retired_pruning_horizon`, `a_delegator_who_left_after_the_selection_still_earns_it_at_the_older_rate`, … (всего 27) | — |
| A12 | latest_snapshot_epoch_at_or_before: answer at `low`, not `low - 1` | КРАСНАЯ (25) | `a_commission_rise_misses_two_epochs_while_a_cut_lands_on_the_next`, `a_delegator_who_left_after_the_selection_still_earns_it_at_the_older_rate`, `a_raised_delegation_minimum_does_not_govern_the_owner_self_stake`, … (всего 25) | — |
| A13 | latest_snapshot_epoch_at_or_before: drop the empty-prefix guard | КРАСНАЯ (1) | `a_seat_with_no_snapshot_at_its_selection_epoch_pays_its_whole_credit_to_the_owner` | — |
| A14 | selection_epoch_for: lag off by one | КРАСНАЯ (9) | `a_commission_rise_misses_two_epochs_while_a_cut_lands_on_the_next`, `a_delegator_who_left_after_the_selection_still_earns_it_at_the_older_rate`, `a_reward_and_a_matured_principal_claim_are_independent`, … (всего 9) | — |
| A15 | first_reward_epoch_for: drop the stake-epoch-0 saturation arm | КРАСНАЯ (3) | `future_delegation_and_noop_commission_do_not_bypass_warmup`, `reward_claims_are_bounded_to_one_thousand_epochs`, `the_owner_and_delegator_claims_split_an_accrued_epoch_by_its_commission` | — |
| A16 | first_reward_epoch_for: add one epoch too few | КРАСНАЯ (11) | `a_delegator_who_left_after_the_selection_still_earns_it_at_the_older_rate`, `a_reward_and_a_matured_principal_claim_are_independent`, `a_seat_with_no_snapshot_at_its_selection_epoch_pays_its_whole_credit_to_the_owner`, … (всего 11) | — |
| A17 | snapshot_payout: take the MAX of the two commission vintages, not the min | КРАСНАЯ (3) | `a_commission_rise_misses_two_epochs_while_a_cut_lands_on_the_next`, `a_delegator_who_left_after_the_selection_still_earns_it_at_the_older_rate`, `a_snapshot_materialized_after_its_epoch_passed_still_carries_the_old_rate` | — |
| A18 | snapshot_payout: divide the owner cut by the rate instead of the bps denominator | КРАСНАЯ (14) | `a_commission_rise_misses_two_epochs_while_a_cut_lands_on_the_next`, `a_delegator_who_left_after_the_selection_still_earns_it_at_the_older_rate`, `a_permissionless_owner_claim_pays_the_owner_off_the_reserve`, … (всего 14) | — |
| A19 | snapshot_payout: pay the whole reward to the owner instead of splitting it | КРАСНАЯ (14) | `a_commission_rise_misses_two_epochs_while_a_cut_lands_on_the_next`, `a_delegator_who_left_after_the_selection_still_earns_it_at_the_older_rate`, `a_permissionless_owner_claim_pays_the_owner_off_the_reserve`, … (всего 14) | — |
| A20 | validator_owner_rewards: drop the per-claim epoch bound | ЗЕЛЁНАЯ |  | КРАСНАЯ: `the_owner_reward_walk_is_bounded_to_one_thousand_epochs_too` |
| A21 | validator_owner_rewards: walk one epoch too far (< becomes <=) | ЗЕЛЁНАЯ |  | ЗЕЛЁНАЯ |
| A22 | reward walk: divide by the CLOSE-epoch total, not the selection vintage (both copies) | КРАСНАЯ (6) | `a_delegator_who_left_after_the_selection_still_earns_it_at_the_older_rate`, `a_reward_and_a_matured_principal_claim_are_independent`, `redelegation_takes_the_reward_and_leaves_the_withdrawal_queue_alone`, `the_accrual_path_credits_an_epoch_that_is_still_divided_two_epochs_back`, `the_delegator_split_reproduces_the_seat_weight_frozen_two_epochs_back`, `the_delegator_views_report_the_reward_and_the_deposit_apart` | — |
| A23 | reward walk: swap the numerator operands — total * pool / delegated (both copies) | КРАСНАЯ (9) | `a_delegator_who_left_after_the_selection_still_earns_it_at_the_older_rate`, `a_reward_and_a_matured_principal_claim_are_independent`, `a_tombstone_extinguishes_the_owner_commission_but_not_the_delegator_share`, … (всего 9) | — |
| A24 | reward walk: close the window at the successor's raw stake epoch, without the lag (both copies) | КРАСНАЯ (4) | `a_delegator_who_left_after_the_selection_still_earns_it_at_the_older_rate`, `a_reward_and_a_matured_principal_claim_are_independent`, `redelegation_takes_the_reward_and_leaves_the_withdrawal_queue_alone`, `the_delegator_views_report_the_reward_and_the_deposit_apart` | — |
| A25 | reward walk: take the MAX of the window bounds, not the min (both copies) | КРАСНАЯ (1) | `a_delegator_who_left_after_the_selection_still_earns_it_at_the_older_rate` | — |
| A26 | delegator_principal_claimable: mature an undelegation one epoch early (> becomes >=) | КРАСНАЯ (2) | `redelegation_takes_the_reward_and_leaves_the_withdrawal_queue_alone`, `the_delegator_views_report_the_reward_and_the_deposit_apart` | — |
| A27 | capped_delegator_principal_epoch: take the MAX of the two bounds, not the min | КРАСНАЯ (1) | `a_delegator_who_left_after_the_selection_still_earns_it_at_the_older_rate` | — |
| A28 | consume_delegator_principal: mature an undelegation one epoch early (> becomes >=) | КРАСНАЯ (4) | `a_deposit_the_contract_cannot_cover_is_not_paid_out_of_the_reserve`, `a_reward_and_a_matured_principal_claim_are_independent`, `a_slash_with_nothing_to_seize_still_tombstones`, `the_delegator_views_report_the_reward_and_the_deposit_apart` | — |
| A29 | consume_delegator_principal: do not debit the pending-undelegated total | КРАСНАЯ (1) | `a_slash_with_nothing_to_seize_still_tombstones` | — |
| A30 | available_for_redelegate: round the compact amount UP instead of down | КРАСНАЯ (1) | `redelegation_takes_the_reward_and_leaves_the_withdrawal_queue_alone` | — |
| A31 | available_for_redelegate: drop the minimum-staking floor | ЗЕЛЁНАЯ |  | КРАСНАЯ: `a_redelegation_under_the_staking_minimum_is_paid_out_instead` |
| A32 | assign_epoch_shares: drop the reserve-covers-the-pot gate (M36 repeated) | КРАСНАЯ (9) | `an_approval_over_an_empty_reserve_covers_nothing`, `an_epoch_with_a_committee_but_no_recorded_block_draws_no_pot`, `an_unreadable_reserve_zeroes_the_epoch_without_failing_the_close`, … (всего 9) | — |
| A33 | assign_epoch_shares: divide the share by the seat weight, not the total | КРАСНАЯ (7) | `an_approval_over_an_empty_reserve_covers_nothing`, `epochs_that_close_before_the_treasury_approves_burn_for_good`, `funding_the_reserve_after_the_close_does_not_revive_the_epoch`, … (всего 7) | — |
| A34 | assign_epoch_shares: expand the frozen weight before the split | ЗЕЛЁНАЯ |  | ЗЕЛЁНАЯ |
| A35 | assign_epoch_shares: sum the pot rather than the floored shares | КРАСНАЯ (1) | `the_accrued_total_is_exactly_the_sum_of_the_credits_it_wrote` | — |
| A36 | accrue_epoch: draw a full pot for an epoch with no recorded block | КРАСНАЯ (1) | `an_epoch_with_a_committee_but_no_recorded_block_draws_no_pot` | — |
| A37 | validator_total_at: drop the no-snapshot short circuit | ЗЕЛЁНАЯ |  | ЗЕЛЁНАЯ |
| C1 | ensure_governance_mutation: drop the non-payable gate | ЗЕЛЁНАЯ |  | КРАСНАЯ: `handlers_refuse_value_and_refuse_to_mutate_inside_a_static_frame` |
| C2 | ensure_governance_mutation: drop the static-frame gate | ЗЕЛЁНАЯ |  | КРАСНАЯ: `handlers_refuse_value_and_refuse_to_mutate_inside_a_static_frame` |
| C3 | ensure_governance_mutation: drop the governance caller check | КРАСНАЯ (2) | `governance_updates_embedded_chain_configuration`, `production_liveness_setters_enforce_their_bounds` | — |
| C4 | ensure_dpos_not_active: let a running chain move its epoch geometry | КРАСНАЯ (1) | `chain_config_guards_match_solidity_boundaries` | — |
| C5 | ensure_dpos_not_active: treat the unarmed sentinel as already active | КРАСНАЯ (3) | `dpos_activation_at_block_zero_remains_configurable`, `scheduling_activation_never_moves_the_epoch_backwards`, `undelegate_period_change_does_not_shorten_queued_principal` | — |
| C6 | validate_initialization: drop the zero-staking-token gate | ЗЕЛЁНАЯ |  | ЗЕЛЁНАЯ |
| C7 | validate_initialization: drop the committee-cap ceiling | ЗЕЛЁНАЯ |  | КРАСНАЯ: `initialize_refuses_every_malformed_chain_configuration` |
| C8 | validate_initialization: drop the zero-interval gate | ЗЕЛЁНАЯ |  | КРАСНАЯ: `initialize_refuses_every_malformed_chain_configuration` |
| C9 | validate_initialization: drop the zero-undelegate-period gate | ЗЕЛЁНАЯ |  | КРАСНАЯ: `initialize_refuses_every_malformed_chain_configuration` |
| C10 | validate_initialization: drop the two zero-minimum gates | ЗЕЛЁНАЯ |  | КРАСНАЯ: `initialize_refuses_every_malformed_chain_configuration` |
| C11 | validate_initialization: drop the two compact-precision gates | ЗЕЛЁНАЯ |  | КРАСНАЯ: `initialize_refuses_every_malformed_chain_configuration` |
| C12 | validate_initialization: drop the activation-block alignment gate | ЗЕЛЁНАЯ |  | КРАСНАЯ: `initialize_refuses_every_malformed_chain_configuration` |
| C13 | validate_initialization: drop the undelegate-window floor | КРАСНАЯ (1) | `stores_chain_configuration_in_its_own_namespace` | — |
| C14 | validate_initialization: drop the zero-blend-reserve gate | КРАСНАЯ (1) | `parameterized_custom_errors_use_solidity_abi` | — |
| C15 | set_slash_fund_address: accept the zero address | ЗЕЛЁНАЯ |  | КРАСНАЯ: `the_governance_setters_refuse_every_value_they_are_supposed_to` |
| C16 | set_blend_stipend_per_epoch: drop the ceiling | КРАСНАЯ (1) | `chain_config_guards_match_solidity_boundaries` | — |
| C17 | set_active_validators_length: drop the MAX_COMMITTEE_SIZE ceiling | КРАСНАЯ (1) | `chain_config_guards_match_solidity_boundaries` | — |
| C18 | set_epoch_block_interval: accept zero | ЗЕЛЁНАЯ |  | КРАСНАЯ: `the_governance_setters_refuse_every_value_they_are_supposed_to` |
| C19 | set_epoch_block_interval: drop the dpos-not-active gate | КРАСНАЯ (1) | `chain_config_guards_match_solidity_boundaries` | — |
| C20 | set_epoch_block_interval: drop the alignment gate | ЗЕЛЁНАЯ |  | ЗЕЛЁНАЯ |
| C21 | set_epoch_block_interval: drop the undelegate-window floor | КРАСНАЯ (1) | `dpos_activation_at_block_zero_remains_configurable` | — |
| C22 | set_dpos_activation_block: drop the dpos-not-active gate | ЗЕЛЁНАЯ |  | КРАСНАЯ: `the_activation_setter_refuses_the_past_and_a_running_chain` |
| C23 | set_dpos_activation_block: drop the alignment gate | КРАСНАЯ (1) | `chain_config_guards_match_solidity_boundaries` | — |
| C24 | set_dpos_activation_block: accept an activation in the past | ЗЕЛЁНАЯ |  | КРАСНАЯ: `the_activation_setter_refuses_the_past_and_a_running_chain` |
| C25 | set_undelegate_period: accept zero | ЗЕЛЁНАЯ |  | КРАСНАЯ: `the_governance_setters_refuse_every_value_they_are_supposed_to` |
| C26 | set_undelegate_period: drop the dpos-not-active gate | КРАСНАЯ (1) | `chain_config_guards_match_solidity_boundaries` | — |
| C27 | require_undelegate_window: drop the floor comparison | КРАСНАЯ (2) | `chain_config_guards_match_solidity_boundaries`, `dpos_activation_at_block_zero_remains_configurable` | — |
| C28 | set_min_validator_stake_amount: accept zero | ЗЕЛЁНАЯ |  | КРАСНАЯ: `the_governance_setters_refuse_every_value_they_are_supposed_to` |
| C29 | set_min_validator_stake_amount: drop the compact-precision gate | КРАСНАЯ (1) | `governance_updates_embedded_chain_configuration` | — |
| C30 | set_min_staking_amount: accept zero | ЗЕЛЁНАЯ |  | КРАСНАЯ: `the_governance_setters_refuse_every_value_they_are_supposed_to` |
| C31 | set_min_staking_amount: drop the compact-precision gate | КРАСНАЯ (1) | `governance_updates_embedded_chain_configuration` | — |
| C32 | set_min_verdict_due_blocks: accept zero | КРАСНАЯ (1) | `production_liveness_setters_enforce_their_bounds` | — |
| C33 | set_min_verdict_due_blocks: drop the ceiling | КРАСНАЯ (1) | `production_liveness_setters_enforce_their_bounds` | — |
| C34 | set_exclusion_backoff_cap: accept zero | КРАСНАЯ (1) | `production_liveness_setters_enforce_their_bounds` | — |
| C35 | set_blend_reserve: accept the zero address | КРАСНАЯ (1) | `parameterized_custom_errors_use_solidity_abi` | — |
| C36 | set_active_validators_length: drop the committee floor (the П1 gate) | КРАСНАЯ (1) | `the_cap_setter_refuses_a_value_below_the_committee_floor` | — |
| C37 | validate_initialization: drop the committee floor (the П1 gate) | КРАСНАЯ (1) | `initialize_refuses_a_committee_cap_below_the_floor` | — |
| C38 | initialize: drop the one-shot flag (initialized) | КРАСНАЯ (2) | `initializer_is_permissionless_for_atomic_deployment_but_one_shot`, `stores_chain_configuration_in_its_own_namespace` | — |
| C39 | initialize: drop the in-flight reentrancy flag (initializing) | ЗЕЛЁНАЯ |  | ЗЕЛЁНАЯ |
| C40 | initialize: never raise the in-flight flag | ЗЕЛЁНАЯ |  | ЗЕЛЁНАЯ |
| C41 | validate: drop the zero-owner gate | ЗЕЛЁНАЯ |  | ЗЕЛЁНАЯ |
| C42 | validate: drop the five-array arity check | КРАСНАЯ (1) | `initializer_rejects_mismatched_arrays_without_persisting_state` | — |
| C43 | validate: check four arrays instead of five (peer_pubkeys unchecked) | ЗЕЛЁНАЯ |  | ЗЕЛЁНАЯ |
| C44 | validate: drop the commission-rate ceiling (M23 repeated) | ЗЕЛЁНАЯ |  | ЗЕЛЁНАЯ |
| C45 | validate: drop the genesis minimum-stake floor | КРАСНАЯ (1) | `initializer_rejects_subminimum_active_validator` | — |
| C46 | pull_initial_stakes: drop the zero-total short circuit | ЗЕЛЁНАЯ |  | ЗЕЛЁНАЯ |
| C47 | pull_initial_stakes: pull to the sponsor instead of into this contract | КРАСНАЯ (1) | `initializer_pulls_genesis_stake_from_declared_sponsor` | — |
| C48 | initialize: sum the stakes as a saturating add instead of checked | ЗЕЛЁНАЯ |  | ЗЕЛЁНАЯ |
| K2 | COMBINED C1+C2: drop both the non-payable and the static-frame gate from the governance preamble | ЗЕЛЁНАЯ |  | КРАСНАЯ: `handlers_refuse_value_and_refuse_to_mutate_inside_a_static_frame` |
| L1 | record_production: drop the SYSTEM_CALLER gate | ЗЕЛЁНАЯ |  | КРАСНАЯ: `the_recorder_refuses_every_caller_but_the_system_address` |
| L2 | record_production: drop the idempotency gate on the block height | КРАСНАЯ (2) | `record_production_belt_holds_and_the_epoch_cursor_precedes_the_overwrite`, `the_recorder_takes_its_height_from_the_block_context` | — |
| L3 | record_production: close the epoch on EVERY block, not only at the boundary | ЗЕЛЁНАЯ |  | ЗЕЛЁНАЯ |
| L4 | record_production: drop the uncommitted-committee park | ЗЕЛЁНАЯ |  | ЗЕЛЁНАЯ |
| L5 | record_production: drop the leader-index range belt | КРАСНАЯ (1) | `the_recorder_keeps_the_block_count_equal_to_the_sum_of_its_credits` | — |
| L6 | close_epoch: expect a full interval in epoch 0 too | КРАСНАЯ (3) | `a_healthy_epoch_zero_is_complete_one_block_short_of_the_interval`, `an_epoch_zero_two_blocks_short_is_still_partial`, `the_correlation_guard_keys_on_new_failures_and_frees_the_next_epoch` | — |
| L7 | close_epoch: judge a partial epoch anyway | КРАСНАЯ (6) | `a_partial_epoch_suppresses_judging_entirely`, `an_epoch_zero_two_blocks_short_is_still_partial`, `an_uncommitted_committee_parks_the_block_instead_of_reverting`, `only_epoch_zero_gets_the_shortened_expectation`, `record_production_belt_holds_and_the_epoch_cursor_precedes_the_overwrite`, `the_close_reports_the_epoch_that_ended_not_the_one_the_block_starts` | — |
| L8 | close_epoch: ignore the kill switch | КРАСНАЯ (2) | `the_kill_switch_also_suppresses_verdicts`, `the_kill_switch_suspends_judging_but_never_releases` | — |
| L9 | release_expired: release before the term expires | КРАСНАЯ (1) | `stamps_are_bounded_per_close_and_by_the_concurrent_budget` | — |
| L10 | release_expired: treat the never-stamped sentinel as expired | ЗЕЛЁНАЯ |  | ЗЕЛЁНАЯ |
| L11 | judge: drop the empty-committee guard | ЗЕЛЁНАЯ |  | ЗЕЛЁНАЯ |
| L12 | judge: a ring miss no longer announces itself | ЗЕЛЁНАЯ |  | КРАСНАЯ: `a_verdict_pass_past_the_ring_forfeits_its_verdicts_and_says_so` |
| L13 | judge: drop the zero-total-weight guard | ЗЕЛЁНАЯ |  | ЗЕЛЁНАЯ |
| L14 | judge: drop the due-blocks floor | КРАСНАЯ (2) | `the_verdict_floor_is_a_stake_share_at_the_production_epoch_length`, `verdicts_come_from_the_frozen_weights_and_are_never_divided` | — |
| L15 | judge: never retire the backoff ladder | КРАСНАЯ (2) | `a_clean_run_retires_the_kick_ladder_and_one_epoch_short_does_not`, `the_ladder_reset_also_lands_on_the_member_failing_that_same_epoch` | — |
| L16 | judge: ladder reset one epoch late (>= becomes >) | КРАСНАЯ (2) | `a_clean_run_retires_the_kick_ladder_and_one_epoch_short_does_not`, `the_ladder_reset_also_lands_on_the_member_failing_that_same_epoch` | — |
| L17 | judge: drop the half-of-due factor (fail only at a full shortfall) | КРАСНАЯ (1) | `verdicts_come_from_the_frozen_weights_and_are_never_divided` | — |
| L18 | judge: count a chronic failer as a new failure | КРАСНАЯ (1) | `the_correlation_guard_keys_on_new_failures_and_frees_the_next_epoch` | — |
| L19 | judge: count an already-excluded member as a new failure | ЗЕЛЁНАЯ |  | ЗЕЛЁНАЯ |
| L20 | judge: drop the correlated-failure guard | КРАСНАЯ (1) | `the_correlation_guard_keys_on_new_failures_and_frees_the_next_epoch` | — |
| L21 | judge: correlated-failure guard fires one failure early (> becomes >=) | КРАСНАЯ (3) | `an_unreadable_reserve_zeroes_the_epoch_without_failing_the_close`, `stamps_are_bounded_per_close_and_by_the_concurrent_budget`, `verdicts_come_from_the_frozen_weights_and_are_never_divided` | — |
| L22 | judge: drop the no-failures early return | ЗЕЛЁНАЯ |  | ЗЕЛЁНАЯ |
| L23 | stamp: drop the concurrent-exclusion budget | КРАСНАЯ (1) | `stamps_are_bounded_per_close_and_by_the_concurrent_budget` | — |
| L24 | stamp: re-stamp a member already serving an exclusion | КРАСНАЯ (1) | `stamps_are_bounded_per_close_and_by_the_concurrent_budget` | — |
| L25 | stamp: drop the kick-count-then-address ordering | ЗЕЛЁНАЯ |  | ЗЕЛЁНАЯ |
| L26 | stamp: advance the ladder for a refused exclusion | ЗЕЛЁНАЯ |  | ЗЕЛЁНАЯ |
| L27 | stamp: drop the backoff cap saturation | ЗЕЛЁНАЯ |  | ЗЕЛЁНАЯ |
| L28 | stamp: raise the per-close stamp bound to the committee cap | КРАСНАЯ (1) | `stamps_are_bounded_per_close_and_by_the_concurrent_budget` | — |
| S1 | selected_committee_at: drop the STATUS_ACTIVE term (M1 repeated) | ЗЕЛЁНАЯ |  | ЗЕЛЁНАЯ |
| S2 | selected_committee_at: drop the selection-visibility term | КРАСНАЯ (2) | `governance_activation_does_not_cancel_a_running_exclusion`, `production_exclusion_bites_at_the_next_epoch_and_not_before` | — |
| S3 | selected_committee_at: seat keyless candidates too | КРАСНАЯ (3) | `a_refused_commit_writes_neither_committee_nor_cursor`, `keys_activating_after_the_selection_epoch_are_filtered_before_the_cut`, `the_committee_seats_the_next_eligible_validator_instead_of_a_keyless_one` | — |
| S4 | selected_committee_at: ignore the committee cap | КРАСНАЯ (1) | `production_exclusion_bites_at_the_next_epoch_and_not_before` | — |
| S5 | active_peer_key_at: drop the empty-peer-key sentinel | КРАСНАЯ (2) | `a_refused_commit_writes_neither_committee_nor_cursor`, `the_committee_seats_the_next_eligible_validator_instead_of_a_keyless_one` | — |
| S6 | active_peer_key_at: drop the activation-epoch gate | КРАСНАЯ (1) | `keys_activating_after_the_selection_epoch_are_filtered_before_the_cut` | — |
| S7 | active_peer_key_at: activation gate off by one (<= becomes <) | КРАСНАЯ (13) | `a_committee_stays_readable_far_past_the_retired_pruning_horizon`, `a_raised_minimum_does_not_empty_the_next_committee`, `a_refused_commit_writes_neither_committee_nor_cursor`, … (всего 13) | — |
| S8 | committee_changed: drop the length term (M2 repeated) | КРАСНАЯ (1) | `committee_changed_compares_positions_not_membership` | — |
| S9 | committee_changed: drop the positional comparison | КРАСНАЯ (1) | `committee_changed_compares_positions_not_membership` | — |
| S10 | committee_changed: drop the genesis short-circuit | КРАСНАЯ (12) | `a_committee_stays_readable_far_past_the_retired_pruning_horizon`, `a_raised_minimum_does_not_empty_the_next_committee`, `a_refused_commit_writes_neither_committee_nor_cursor`, … (всего 12) | — |
| S11 | commit_epoch_committee: drop the SYSTEM_CALLER gate | КРАСНАЯ (1) | `committee_commit_is_system_gated_and_returns_epoch_stakes` | — |
| S12 | commit_epoch_committee: drop the lookahead ceiling | ЗЕЛЁНАЯ |  | КРАСНАЯ: `the_commit_refuses_a_target_past_the_lookahead_horizon` |
| S13 | commit_epoch_committee: drop the committee floor | КРАСНАЯ (2) | `a_refused_commit_writes_neither_committee_nor_cursor`, `an_eligible_set_one_short_of_the_floor_is_refused` | — |
| S14 | commit_epoch_committee: drop the peer-key sort | КРАСНАЯ (2) | `committee_commit_is_system_gated_and_returns_epoch_stakes`, `the_commit_orders_by_peer_key_and_membership_changes_mint_the_dkg_bit` | — |
| S15 | commit_epoch_committee: select at the target epoch, not target-lookahead | КРАСНАЯ (1) | `leader_weights_are_frozen_at_the_selection_epoch_vintage` | — |
| S16 | commit_epoch_committee: drop the target==0 always-append arm | КРАСНАЯ (12) | `a_committee_stays_readable_far_past_the_retired_pruning_horizon`, `a_raised_minimum_does_not_empty_the_next_committee`, `a_refused_commit_writes_neither_committee_nor_cursor`, … (всего 12) | — |
| S17 | top_k_by_stake_at: drop the cap truncation | КРАСНАЯ (2) | `equal_stake_top_k_preserves_solidity_roster_order`, `production_exclusion_bites_at_the_next_epoch_and_not_before` | — |
| S18 | top_k_by_stake_at: rank ascending (> becomes <) | КРАСНАЯ (6) | `a_raised_minimum_does_not_empty_the_next_committee`, `future_delegation_and_noop_commission_do_not_bypass_warmup`, `initializes_registry_and_preserves_solidity_read_abi`, `keys_activating_after_the_selection_epoch_are_filtered_before_the_cut`, `production_exclusion_bites_at_the_next_epoch_and_not_before`, `the_committee_seats_the_next_eligible_validator_instead_of_a_keyless_one` | — |
| S19 | top_k_by_stake_at: selection sort starts one past itself (skip(index+1) -> skip(index+2)) | КРАСНАЯ (2) | `future_delegation_and_noop_commission_do_not_bypass_warmup`, `initializes_registry_and_preserves_solidity_read_abi` | — |
| S20 | top_k_by_stake_at: break ties toward the later candidate (> becomes >=) | КРАСНАЯ (5) | `a_raised_minimum_does_not_empty_the_next_committee`, `equal_stake_top_k_preserves_solidity_roster_order`, `keys_activating_after_the_selection_epoch_are_filtered_before_the_cut`, `production_exclusion_refuses_at_the_committee_floor_and_leaves_no_trace`, `the_committee_seats_the_next_eligible_validator_instead_of_a_keyless_one` | — |
| S21 | selected_validators: drop the STATUS_ACTIVE filter | ЗЕЛЁНАЯ |  | ЗЕЛЁНАЯ |
| S22 | selected_validators: ignore the committee cap | КРАСНАЯ (1) | `equal_stake_top_k_preserves_solidity_roster_order` | — |
| V1 | ensure_non_payable: accept value | КРАСНАЯ (1) | `handlers_refuse_value_and_refuse_to_mutate_inside_a_static_frame` | — |
| V2 | ensure_mutable: mutate inside a static frame | КРАСНАЯ (1) | `handlers_refuse_value_and_refuse_to_mutate_inside_a_static_frame` | — |
| V3 | decode: swallow a decode error into a default value | ЗЕЛЁНАЯ |  | ЗЕЛЁНАЯ |
| V4 | write_abi: emit an empty return instead of the encoded value | КРАСНАЯ (33) | `a_committee_stays_readable_far_past_the_retired_pruning_horizon`, `a_delegator_who_left_after_the_selection_still_earns_it_at_the_older_rate`, `a_raised_minimum_does_not_empty_the_next_committee`, … (всего 33) | — |
| V5 | reserve_available: drop the zero-token guard | ЗЕЛЁНАЯ |  | ЗЕЛЁНАЯ |
| V6 | reserve_available: drop the zero-balance short circuit | ЗЕЛЁНАЯ |  | ЗЕЛЁНАЯ |
| V7 | reserve_available: ask the token about the caller, not the reserve | КРАСНАЯ (10) | `a_permissionless_owner_claim_pays_the_owner_off_the_reserve`, `an_approval_over_an_empty_reserve_covers_nothing`, `an_epoch_with_a_committee_but_no_recorded_block_draws_no_pot`, … (всего 10) | — |
| V8 | erc20_scalar_read: score a failed read as MAX instead of zero | КРАСНАЯ (1) | `an_unreadable_reserve_zeroes_the_epoch_without_failing_the_close` | — |
| V9 | write_validators_with_keys: leak a key that activates later than the asked epoch | ЗЕЛЁНАЯ |  | ЗЕЛЁНАЯ |
| V10 | get_epoch_committee_with_stakes: answer the record's own length, not the index length | ЗЕЛЁНАЯ |  | — |
| V11 | get_epoch_committee_with_stakes: answer a missing weight ring as a full one | КРАСНАЯ (1) | `a_short_successor_cannot_let_its_predecessor_read_the_wrong_weights` | — |
| V12 | committee_length_at: read the record pointer as the length | КРАСНАЯ (3) | `record_production_belt_holds_and_the_epoch_cursor_precedes_the_overwrite`, `the_recorder_keeps_the_block_count_equal_to_the_sum_of_its_credits`, `the_recorder_takes_its_height_from_the_block_context` | — |
| V13 | committee_at: swap the record and length halves | КРАСНАЯ (41) | `a_clean_run_retires_the_kick_ladder_and_one_epoch_short_does_not`, `a_close_past_the_ring_forfeits_the_epoch_and_says_so`, `a_committee_stays_readable_far_past_the_retired_pruning_horizon`, … (всего 41) | — |
| V14 | main_entry: drop the four-byte selector length gate | ЗЕЛЁНАЯ |  | ЗЕЛЁНАЯ |
| V15 | main_entry: answer an unknown selector with success instead of UnknownMethod | КРАСНАЯ (1) | `the_production_shape_answers_no_view_selector` | — |

Комбинированные, прогнаны вручную:

- **K1** = L4 + L5 вместе (снять И парковку при незакоммиченном комитете, И пояс
  по `leader_index`): **КРАСНАЯ**, 2 теста —
  `an_uncommitted_committee_parks_the_block_instead_of_reverting`,
  `the_recorder_keeps_the_block_count_equal_to_the_sum_of_its_credits`. Это и
  показывает, что L4 избыточна, а не непокрыта.
- **K2** = C1 + C2 вместе (снять И `ensure_non_payable`, И `ensure_mutable` из
  `ensure_governance_mutation`): до правки **ЗЕЛЁНАЯ**, после —
  КРАСНАЯ, `handlers_refuse_value_and_refuse_to_mutate_inside_a_static_frame`.
- **A8** = округление середины вверх во всех пяти бинарных поисках `staking.rs`:
  прогон НЕ ЗАВЕРШАЕТСЯ (поиск перестаёт сходиться), убит вручную. Обнаружение
  есть, assert'а нет.

## §5 Оставлено как есть

Каждая строка — с причиной; ни одной «пока не дошли руки» без механизма.

**Из П2 (K-18). Точки входа `config.rs`, которым гейт не нужен.** Все тринадцать
вьюх — одно чтение поля через `write_abi(… get_checked …)`, ни деления, ни вызова
`current_epoch`; измерено, не вычитано (вторая половина теста
`the_two_views_that_reach_the_epoch_formula_refuse_an_uninitialized_contract`
утверждает для каждой по имени, что она отвечает `Ok` на неинициализированном
контракте). Все двенадцать сеттеров — `ensure_governance_mutation` →
`ensure_governance` → `ensure_initialized` (`util.rs:60`); отдельный гейт был бы
второй такой же проверкой. Четыре вьюхи `liveness.rs` под `devnet-views` — тоже
чистые чтения поля, в том же списке.

**Из П2. `ERR_CONSENSUS_KEYS_ALREADY_SET` в `store_consensus_keys`.** П5 удалил ДВЕ
перепроверки, названные заданием; третья — эта — оставлена: задание её не называет, а
F-4 (`E1-8-TESTS.md`) уже разобрал её как недостижимую и честно так названную в коде.

**Из П4в. 21 зелёная мутация классов.** Полный список с вердиктами — §2 П4(в).
Каждая записана как дыра; закрывать все в пункте «только тесты» — не тот объём (§3
Д-07).

**Из П4г. A21** (обход наград владельца на эпоху дальше): открыта. На всех фикстурах
лишняя эпоха пуста, потому что кредиты пишутся только в эпохи, которые тест назвал;
чтобы её поймать, нужен кредит ЗА окном, а это новая фикстура, не новое утверждение.

**Из П5. `STATUS_ACTIVE`** — оставлен и задокументирован; разбор в §3 Д-08.

**Из П5. `effective_epoch`** — правки не потребовалось: док поля уже соответствует
коду, и его утверждение «поле никто не читает» перепроверено grep'ом.

**Из П5. README, разделы «Solidity parity» и таблица аудита событий** — не сверял с
деревом: первый про соседний репозиторий (вне объёма сессии), вторая — про
байт-формат событий, которого я не измерял. Не трогал, чтобы не подписаться под
непроверенным.

**Из П6. `setBlendStipendPerEpoch`** — в таймлок не входит, решено 09-04 с
основанием; в код добавлена строка, которая это называет, ровно там, где следующий
захочет схему расширить.

**Из П8. `decode_args`** — динамический путь не ужесточён: у динамического кортежа
законные кодировки различаются раскладкой смещений. Дыры хвоста и усечения остаются
открытыми на `initialize` и трёх маршрутах улик.

**Из П8. SDK-декодер** — не правился; список того, что сломала бы строгость, — §2 П8
и §6.

**12 тестов маршрута улик и `evidence.rs`** — не трогал, как и велено: ждут Д-4.

## §6 Всплыло

- `devnet/local-dpos-smoke/genesis-bootstrap/src/bootstrap.rs:135` держит
  `seated >= 1`, а `:324` подаёт `seated` как `activeValidatorsLength`. После П1
  стенд с `committee_size` 1..3 будет падать на `initialize`, а не подниматься
  и вставать на первом `commitEpochCommittee`. Поведение по существу то же (цепь
  всё равно не жила), но сообщение другое; `devnet/` вне объёма этой сессии,
  не правил.
- **K-13, кого сломала бы строгость в SDK** (перечисление из П8, вынесено сюда как
  того требует задание): `crates/sdk/src/universal_token/command.rs:26` (калдата
  чужого контракта), `crates/sdk/src/universal_token/storage.rs:81` (чтение
  хранилища), `crates/sdk/src/universal_token/storage.rs:167,179` (**две версии
  `InitialSettings` перебираются по одному payload'у — строгость ломает сам
  механизм версионирования**), `crates/sdk/src/storage_legacy.rs:41` (чтение
  хранилища с откатом на `T::default()`), `contracts/webauthn/src/lib.rs:44,65`
  (калдата другого контракта). Предложение, если строгий декодер когда-нибудь
  понадобится как общий: не менять `SolidityABI::decode`, а добавить рядом
  `decode_exact`, и переводить на него по одному вызывающему.
- **`delegator_reward_claimable` и `consume_delegator_reward` несут побайтно
  одинаковый блок обхода наград** (`staking.rs`, ~20 строк: `changed_at` через
  `first_reward_epoch_for`, `end = min(...)`, деление доли по винтажу отбора).
  Замечено в П4г: каждая арифметическая мутация этого блока пришлось ставить с
  `count: 2`. Дубль расходится молча — правка в одном месте меняет ЧТО начислено, а
  в другом ЧТО выплачено.
- **`06_staking_layer.md:496` — третий сайт дрейфа `carry_committee_forward`**,
  не входящий в список из `PLAN.md` §1 (там названы `15_smoke_cases…:41` и
  `00a_errata…:11`). Предложение в том же абзаце опровергается блоком `[REVISED
  2026-09-08]` двумя предложениями ниже. Не правил: это не поведение, которое
  меняла эта сессия.
- **`ERR_CONSENSUS_KEYS_ALREADY_SET`** после удаления двух соседних перепроверок
  (П5) остался единственным «поясом» в `store_consensus_keys`. F-4 уже признала его
  недостижимым; если следующая чистка захочет удалить и его — сначала стоит
  проверить, не стал ли он достижим от `initialize`, который зовёт
  `store_consensus_keys` в цикле.
- **Мутация A8 (округление середины бинарного поиска вверх) ловится зависанием**,
  а не падением. Никакой assert её не называет; тест с бюджетом итераций поймал бы
  её как тест, а не как повисший прогон.

## §7 Где проверка слабее всего

1. **Тесты таймлока (П6) не мутировались.** Механизм новый, тестов два, и ни одной
   мутации по ним я не ставил: снять сравнение `current < effective_at`, снять
   сброс заявки после применения, снять сентинел — ни одно из этих снятий не
   прогонялось. По собственному правилу сессии (Д-12) это ровно тот класс, который
   «зелёный при сломанном коде» не исключает.
2. **Блоб П6+П7 не гонялся на живом стенде.** Ни `make case-growth`, ни smoke. Всё,
   что о нём известно, — селекторный скан, `agreement_check.py` и e2e; двухшаговая
   ротация резерва живьём на девнете не выполнялась ни разу.
3. **`cargo test -p fluentbase-genesis-bootstrap` прогнан только после П1** (8/0).
   После П6/П7/П8 не перезапускался, а П8 меняет то, что контракт принимает как
   калдату, и `bootstrap.rs` — один из его вызывающих. Косвенное свидетельство есть
   (e2e и `agreement_check` гоняют `initialize` против того же блоба), прямого нет.
4. **21 зелёная мутация классов и одна арифметическая остались открытыми** (§2
   П4в, §5). Это известные дыры, а не неизвестные, но они дыры.
5. **`decode_args` не ужесточён** (П8): `initialize` с шестнадцатью аргументами и
   три маршрута улик по-прежнему принимают хвост и усечённые целые.
6. **Замер «тесты, не упавшие ни от одной мутации» опирается на harvest имён из
   `E1-8-TESTS.md` §2 регуляркой**, а не на перепрогон мутаций 09-08. Если та
   таблица где-то называет тест не в обратных кавычках, он попал в остаток
   неправильно.
7. **Округление и границы в новых тестах П4г я выбирал сам.** Например,
   `a_redelegation_under_the_staking_minimum_is_paid_out_instead` подбирает кредит
   так, чтобы ПОЛОВИНА была ровно на одну компактную единицу ниже минимума; это
   развязывает минимум от округления, но обе границы я поставил, а не измерил.
8. **Строчки `.dpos-study` правились скриптами с `assert` на единственность
   якоря**, и я ни разу не перечитал итоговый файл целиком. Проверки §7 ниже —
   формальные (кавычки, «…», пустые скобки), смысловой вычитки не было.
9. **Полные списки упавших тестов для восьми мутаций с 20+ падениями не сохранены
   в репозитории** (§3 Д-05) — только в scratchpad этой сессии. Для будущей сессии
   они восстановимы лишь перепрогоном.
