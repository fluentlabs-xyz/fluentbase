# Э2.1 + Э2.2 — общий источник для величин на границе узел↔контракт

Дата: 2026-09-09. Дерево `/home/djadjka/Work/fluentbase`, ветка
`djadjka/dpos-reth-2.2-squashed`, база HEAD `f16fdd90`.

Что сделано: две новые точки объявления вместо четырнадцати пар копий.

- `crates/types/src/staking_protocol.rs` — числовые пределы, ширины и две
  функции (`fault_tolerance`, `epoch_at_block`). `no_std`, зависимость одна —
  `alloy-primitives`, которая у `fluentbase-types` уже была.
- `crates/staking-abi` (`fluentbase-staking-abi`) — один `sol!` на 13 вызовов,
  6 событий, 1 ошибку и структуру `ConsensusKeys`. `no_std`, единственная
  зависимость `alloy-sol-types`.

Контракт берёт селекторы из второго крейта (`u32::from_be_bytes(<C as
SolCall>::SELECTOR)` вместо `derive_keccak256_id!("…")`) и пределы из первого;
узел объявляет типы из второго и удалил три своих `sol!`-блока плюс четвёртый в
`slasher/actor.rs`.

---

## 1. Таблица трассировки: 14 групп `DUPLICATES.md`

Номера строк — актуальные на сегодня (в `DUPLICATES.md` они от 2026-09-04 и
протухли; каждое место найдено заново).

| # | Величина | Где было (контракт / узел) | Где стало | Чем удерживается теперь | Статус |
|---|---|---|---|---|---|
| 1 | Суффиксы namespace подписи | `consensus.rs::namespace` / `bls/src/lib.rs::fluent_namespace` + commonware | — | — | **отложено, 2.3** (принадлежит commonware, ждёт Д-4) |
| 2 | Индексное пространство комитета | `consensus.rs:596` `members.sort_unstable_by_key(\|m\| m.peer_pubkey)` / commonware `BiMap` + `reader.rs::check_committee_ordering` | — | — | **отложено, 2.3** |
| 3 | ABI-сигнатуры системных вызовов и вьюх | `consts.rs` (13 × `derive_keccak256_id!`) / `node/src/evm.rs` `sol!`, `reader.rs` `sol!`, `slasher_sink.rs` `sol!`, `slasher/actor.rs` `sol!` | `crates/staking-abi/src/lib.rs` | **компилируемый импорт с обеих сторон**: `consts.rs::sig::<abi::…Call>()`; узел `use fluentbase_staking_abi::…`. Внешний свидетель — `selectors_match_the_deployed_artefact_scan` (крейт ABI) против hex из `STAKING_ARTEFACT.md`, и `derived_selectors_match_independent_hex_pins` (`tests.rs`) с контрактной стороны | **закрыта** |
| 4 | Арность возврата `getEpochCommitteeWithStakes` | `consensus.rs:764` `write_returns(&(validators, keys, stakes, tombstoned))` / `reader.rs` `sol!` | форма — в общем `sol!`; но контракт кодирует её СВОИМ кодеком по Rust-кортежу, то есть второе написание живо | **тест `the_view_returns_decode_under_the_node_s_declaration`** (`contracts/staking/src/tests.rs`): зовёт настоящий хендлер и отдаёт его СЫРОЙ вывод в `abi_decode_returns` узла. Проверен мутацией — краснеет и на удалении ноги, и на перестановке двух. Плюс `epoch_committee_return_matches_the_contract_abi_encoding` (`reader.rs`) против вектора `cast abi-encode` | **закрыта тестом, не импортом** — см. §10 |
| 5 | `MIN_COMMITTEE_LENGTH = 4` | `consts.rs:384` / `reader.rs:183` (комм. «MUST mirror») | `staking_protocol::MIN_COMMITTEE_LENGTH` | `pub use` с обеих сторон | **закрыта** |
| 6 | `BALANCE_COMPACT_PRECISION = 1e10` | `consts.rs:302` (`U256`) / `reader.rs:189` (`u128`) | `staking_protocol::BALANCE_COMPACT_PRECISION` (`u128`) + `…_U256`, выведённый из него `U256::from_limbs` | `pub use` с обеих сторон; тест `the_u256_precision_is_the_same_number_as_the_u128_one` | **закрыта** |
| 7 | Граница компактного стейка `2^112` | `math.rs:8` `U112 = Uint<112,2>` + `StorageUint112` / `reader.rs` `MAX_COMPACT_STAKE = 1 << 112` | `staking_protocol::COMPACT_STAKE_BITS = 112`, `MAX_COMPACT_STAKE = 1 << COMPACT_STAKE_BITS` | контракт: `type U112 = Uint<{ staking_protocol::COMPACT_STAKE_BITS }, 2>`; узел: `use staking_protocol::MAX_COMPACT_STAKE` | **закрыта** (остаток: `StorageUint112` — имя типа в SDK, туда 112 не заводится) |
| 8 | Потолок комитета `51` | `consts.rs:329` `MAX_ACTIVE_VALIDATORS_LENGTH` / `p2p/src/constants.rs` `MAX_COMMITTEE_SIZE` | `staking_protocol::MAX_COMMITTEE_SIZE` | `pub use` с обеих сторон, **одно имя вместо двух**; тест `the_committee_cap_fits_the_one_byte_leader_index` | **закрыта** |
| 9 | Адрес системного вызывающего `0xff…fe` | `consts.rs:487` `SYSTEM_CALLER` (свой литерал) / `fluentbase_types::SYSTEM_ADDRESS` | `fluentbase_types::SYSTEM_ADDRESS` | контракт: `pub use fluentbase_sdk::SYSTEM_ADDRESS as SYSTEM_CALLER` | **закрыта** |
| 10 | Формула эпохи от блока | `math.rs::epoch_at_block` / `reader.rs::epoch_of_block` | `staking_protocol::epoch_at_block(block, activation, interval) -> Option<u64>` | контракт вызывает её из `math::epoch_at_block`, узел `pub use`-ит как `reader::epoch_at_block`; `epoch_of_block` удалён | **закрыта частично** — формула и клэмп «до активации» одни; арм `activation == 0` остался только у контракта, намеренно. Разбор — §2 |
| 11 | Горизонт коммита `2` | `consts.rs:420` `MAX_COMMITTEE_LOOKAHEAD_EPOCHS` / литерал `current_epoch + 2` в `evm.rs` | `staking_protocol::MAX_COMMITTEE_LOOKAHEAD_EPOCHS` | контракт `pub use`; узел `drive_ahead_commit` и три теста мок-курсора считают через константу | **закрыта** |
| 12 | Формула бюджета отказов `(n−1)/3` | `math.rs::fault_tolerance` / commonware `N3f1::max_faults` | `staking_protocol::fault_tolerance` | контракт `pub use`; узел — тест `the_shared_fault_budget_is_commonwares_budget` (`reader.rs`) сверяет общую функцию с `N3f1::max_faults` **по исходнику пиннутого чекаута**, на всём диапазоне `1..=MAX_COMMITTEE_SIZE` | **закрыта** |
| 13 | Размеры BLS 96 / 48 / 256 / 128 | `consts.rs:422-430` `BLS_*` / `bls/src/lib.rs` `PUBKEY_BYTES` и др. | `staking_protocol::{BLS_PUBKEY_LENGTH, BLS_SIGNATURE_LENGTH, BLS_PUBKEY_UNCOMPRESSED_LENGTH, BLS_SIGNATURE_UNCOMPRESSED_LENGTH}` | `pub use` с обеих сторон (узел — под своими старыми именами через `as`) | **закрыта** |
| 14 | Длина payload предложения `32` | `consts.rs:431` `PROPOSAL_PAYLOAD_LENGTH` / `consensus/src/digest.rs` `FixedSize::SIZE` | `staking_protocol::PROPOSAL_PAYLOAD_LENGTH` | контракт `pub use`; `impl FixedSize for Digest { const SIZE = staking_protocol::PROPOSAL_PAYLOAD_LENGTH; }` | **закрыта** |

Итого: **11 закрыто, 1 закрыта частично (10), 2 отложены на 2.3 (1, 2).**

Оговорка к «закрыто»: группы 5-9, 11-14 держатся КОМПИЛИРУЕМЫМ ИМПОРТОМ —
односторонняя правка не собирается. Группы 3 и 4 держатся импортом СЕЛЕКТОРА плюс
тестом на форму: тип возврата в селектор не входит, а контракт кодирует ответ своим
кодеком, поэтому там, где у остальных стоит компилятор, у этих двух стоит тест. Разница
существенная и в первой редакции этого отчёта была стёрта — §10.

---

## 2. Расхождения, найденные ДО правок

### 2.1. Ширина возврата трёх вьюх: контракт `uint64`, узел `uint32` (R-108, подтверждено)

Дословно, контракт:

```rust
// storage.rs:27-30
active_validators_length: StorageU64,
epoch_block_interval:     StorageU64,
undelegate_period:        StorageU64,
```
и три хендлера в `config.rs:192,205,231` отдают это через `write_abi`, то есть
одним 32-байтным словом из `u64`.

Дословно, узел (`reader.rs`, до правки):

```rust
function getEpochBlockInterval()    external view returns (uint32);
function getUndelegatePeriod()      external view returns (uint32);
function getActiveValidatorsLength() external view returns (uint32);
```
(плюс четвёртое объявление `getEpochBlockInterval() returns (uint32)` в
`node/src/evm.rs`).

**Выбрана сторона контракта — `uint64`.** Селектор ему принадлежит, а ширина
возврата в селектор не входит, поэтому селекторный скан не двигается (проверено:
`getEpochBlockInterval` `0x346c90a8`, `getActiveValidatorsLength` `0x32cc6f08`,
`getUndelegatePeriod` `0x5e7b72ad` — все три по одному вхождению в
пересобранном блобе).

Наблюдаемость расхождения сегодня: нулевая — значения по построению влезают в
`u32` (сеттеры принимают `U32Command`). Направление отказа было безопасным
(декодер alloy у узла отверг бы слишком широкое слово). Цена правки: `u32 → u64`
разошлось на `epoch_transition.rs`, `dpos.rs`, `cert_inlet.rs`, `outer.rs` и на
7 моков в тестах — механически, поведение не менялось. Побочно исчезли шесть
`as u64` и один `debug_assert!(… <= u32::MAX)` в `outer.rs`, который сторожил
сужение `NonZeroU64 → u32`, которого больше нет.

### 2.2. Функция эпохи: разные аргументы и разная семантика при `activation == 0` (R-102)

Дословно, контракт (`math.rs`, до правки):

```rust
pub fn epoch_at_block(block_number: u64, activation_block: u64, interval: u64) -> Option<u64> {
    if interval == 0 { return None; }
    if activation_block == 0 || block_number < activation_block { return Some(0); }
    Some((block_number - activation_block) / interval)
}
```

Дословно, узел (`reader.rs`, до правки):

```rust
pub fn epoch_of_block(block_number: u64, epoch_block_interval: u32, dpos_activation_block: u64) -> u64 {
    block_number.saturating_sub(dpos_activation_block) / epoch_block_interval as u64
}
```

Расходятся дважды: **порядок аргументов** (`interval`/`activation` переставлены —
вызов с перепутанными аргументами компилировался бы у обоих) и **`activation ==
0`**: контракт отвечает `0` на любой высоте, узел — `block/interval`.

**Выбрано: общая функция несёт формулу и клэмп «до активации»; арм
`activation == 0` остаётся у контракта.** Почему так, а не «взять контрактную
семантику целиком»:

- Арм контракта — не формула, а политика про состояние, в котором бывает только
  контракт. Его док прямо называет цену снятия: без клэмпа невзведённая цепь
  считает эпохи от генезиса и роняет их обратно в 0 в момент, когда назначается
  настоящий блок активации — обратный скачок, переписывающий, к какому чекпойнту
  относится делегация.
- Это состояние **живое на девнете**: `production_path` инициализирует контракт
  с `dposActivationBlock = 0` намеренно (§15 архитектуры). То есть расхождение
  наблюдаемо ровно там, где спрашивал вопрос — да, девнет с активацией в нулевом
  блоке.
- Узел до этого входа не доходит: `scheduled_dpos_activation` сворачивает `0` в
  «это ещё не DPoS-цепь» и пропускает весь раздел. У узла `activation == 0`
  встречается только в in-memory моках, где «абсолютная нумерация» — это и есть
  то, что они имеют в виду; `StakingStateRead::scheduled_dpos_activation` пишет
  об этом прямым текстом и просит не «примирять» две семантики сменой поведения.
- Взять контрактную семантику целиком означало бы переписать фикстуры почти всего
  тестового набора `epoch_transition` (моки по умолчанию отдают активацию `0` и
  ждут ненулевых эпох). Это переделка тестов, а не дедупликация.

Что осталось и почему это не дубль: одна ветка в одном месте
(`contracts/staking/src/math.rs`), с комментарием, объясняющим, кто в неё
попадает. Формула, которая при расхождении молча увела бы узел и контракт в
разные эпохи, теперь одна.

### 2.3. Ложный комментарий: «у контракта нет обработчика `slashEquivocation`» (R-104)

Дословно, `node/src/evm.rs` (до правки, док теста
`slash_equivocation_calldata_is_pinned`):

> **The contract has no counterpart at all** — `slashEquivocation(uint64,
> uint32)` is dispatched by neither `feat/flu-989-port-solidity-delta` nor
> `origin/feat/flu-989-rust-staking` (verified 2026-08-14: zero hits for the
> signature in `consts.rs` on every branch in this repo that carries the
> contract).

Обработчик есть: `consts.rs::SIG_SLASH_EQUIVOCATION`, диспетчер
`lib.rs:127`, реализация `consensus.rs:905`. Селектор `0xdc6fb3f2` — одно
вхождение в блобе. Комментарий удалён вместе с тестом, который его нёс.

### 2.4. Ложный комментарий: «контракт возвращает ТРИ массива» (R-107 / R-119)

Дословно, `reader.rs` (до правки, внутри `sol!`):

> KNOWN CONTRACT DRIFT — the fourth array has no contract-side counterpart.
> `feat/flu-989-port-solidity-delta` (and `origin/feat/flu-989-rust-staking`) end
> this handler with `write_returns(sdk, &(validators, keys, stakes))` — THREE
> arrays.

Контракт в этом дереве: `consensus.rs:764` —
`write_returns(sdk, &(validators, keys, stakes, tombstoned))`, четыре. Дрейф не
существует; ветка, названная в комментарии, слита как `f16fdd90`.

### 2.5. Устаревший «MERGE CHECKLIST» на 110 строк в `slasher/actor.rs`

Блок утверждал, что ветка-источник несёт шестиаргументные
`slashEquivocation*(bytes,bytes,bytes,bytes,address,bytes32)` и трёхмассивный
возврат комитета, и что слияние сломает работающий путь. В слитом дереве
`consts.rs` несёт четырёхаргументные формы (`0xe28d2f63` / `0xadd07a3e` /
`0xa10827e9` — они же в блобе, по одному вхождению) и четырёхмассивный возврат.
Блок удалён целиком.

### 2.6. Два имени у одной величины

- `MAX_ACTIVE_VALIDATORS_LENGTH` (контракт) и `MAX_COMMITTEE_SIZE` (p2p) — одно
  число `51`. Оставлено имя узла, контрактное удалено.
- `BLS_POP_UNCOMPRESSED_LENGTH` (контракт) и `SIGNATURE_EIP2537_BYTES` (узел) —
  одно число `128`. В общем крейте это
  `BLS_SIGNATURE_UNCOMPRESSED_LENGTH`; контракт импортирует его под своим
  старым именем через `as`, потому что «PoP» — это подпись G1, и оба имени
  описывают одно.

### 2.7. Мёртвое объявление `getUndelegatePeriod` у узла (R-084)

`reader.rs` объявлял вьюху в `sol!` и пинил её селектор, но геттера к ней нет —
узел её не вызывает. Объявление у узла удалено; в контракте `SIG_GET_UNDELEGATE_PERIOD`
остался через `derive_keccak256_id!` (стенд зовёт её напрямую, не через узел).

### 2.8. Что проверено и оказалось РАВНЫМ

Шесть событий, которые узел декодирует, объявлены на двух сторонах независимо
(`#[derive(Event)]` в `contracts/staking/src/events.rs` против `sol!` у узла) и
никогда не сверялись между собой. Сверены впервые — **совпадают все шесть, и
сигнатура, и topic0** (`close_event_topics_match_the_shared_abi`, зелёный с
первого запуска). `EpochCommitteeCommitted` дополнительно совпал с hex-пином
`015ffbf0…`, который в дереве уже лежал.

---

## 3. Что удалено

Тесты (file:line — до удаления):

| Тест | Где | Почему |
|---|---|---|
| `view_selectors_are_pinned` | `reader.rs:1374` | сверял селектор узла с hex из doc-комментария контракта; теперь одна декларация, hex-пин живёт в `crates/staking-abi` |
| `epoch_committee_return_arity_is_pinned` | `reader.rs:1324` | кодировал СВОИМ же `sol!`-типом и им же декодировал |
| `commit_epoch_committee_selector_is_pinned` | `evm.rs:1596` | сравнивал `SIGNATURE` со строковым литералом и `SELECTOR` с hex — обе копии узла |
| `next_epoch_to_commit_selector_is_pinned` | `evm.rs:1623` | то же |
| `block_zero_is_epoch_zero`, `exact_multiple_advances_epoch`, `off_by_one_below_boundary_stays`, `relative_to_activation` | `reader.rs:835-856` | четыре теста на функцию, которая теперь живёт в `crates/types` и там же протестирована; заменены одним — `a_zero_activation_is_absolute_numbering_on_this_side`, который фиксирует ровно то, что осталось узловым |
| блок `SIGNATURE`/`SIGNATURE_HASH` внутри `close_events_decode_from_fabricated_logs` | `evm.rs:1435-1460` | сверял декларацию узла с её же строковым написанием; сама проверка декода фабрикованных логов оставлена |
| половина `record_production_calldata_is_pinned` и `slash_equivocation_calldata_is_pinned` с `SIGNATURE`/`SELECTOR` | `evm.rs:1569,1657` | то же; пин ПАКОВКИ аргументов против вектора `cast calldata` оставлен — это настоящий внешний свидетель |

Комментарии и константы:

| Что | Где было |
|---|---|
| «MUST mirror the staking contract's `MIN_COMMITTEE_LENGTH` (`consts.rs`)» | `reader.rs:183` |
| «MUST mirror the contract — drift mis-weights leaders», ссылка `consts.rs:336` | `reader.rs:185-190` |
| «MUST mirror the staking module's `…::MAX_ACTIVE_VALIDATORS_LENGTH` … Update both in the SAME PR» | `p2p/src/constants.rs:166-175` |
| «KNOWN CONTRACT DRIFT — the fourth array has no contract-side counterpart» | `reader.rs:123-131` |
| «CONTRACT ABI DELTA — MERGE CHECKLIST» (110 строк) | `slasher/actor.rs:63-172` |
| «**The contract has no counterpart at all**» | `evm.rs:1644-1654` |
| ссылки вида `consensus.rs:538` (фактически `:596`) | `reader.rs:326`, `reader.rs:987` — заменены на имя функции без номера строки |
| ссылка «`consts.rs:160,204,208` the selectors, `events.rs:144-221` the close events» | `evm.rs:581-588`, удалена вместе с `sol!` |
| `const MAX_COMPACT_STAKE: u128 = 1 << 112` | `reader.rs:180` |
| `pub const MIN_COMMITTEE_LENGTH: usize = 4` | `reader.rs:184` |
| `pub const BALANCE_COMPACT_PRECISION: u128 = 10_000_000_000` | `reader.rs:190` |
| `pub const MAX_COMMITTEE_SIZE: u64 = 51` | `p2p/src/constants.rs:175` |
| `pub const PUBKEY_BYTES/SIGNATURE_BYTES/PUBKEY_EIP2537_BYTES/SIGNATURE_EIP2537_BYTES` | `bls/src/lib.rs:79-91` |
| `pub const MAX_ACTIVE_VALIDATORS_LENGTH: u64 = 51` | `consts.rs:329` |
| `pub const MIN_COMMITTEE_LENGTH: usize = 4` | `consts.rs:384` |
| `pub const BALANCE_COMPACT_PRECISION: U256 = uint!(…)` | `consts.rs:302` |
| `pub const MAX_COMMITTEE_LOOKAHEAD_EPOCHS: u64 = 2` | `consts.rs:420` |
| `pub const BLS_*_LENGTH` (четыре) + `PROPOSAL_PAYLOAD_LENGTH` | `consts.rs:422-431` |
| `pub const SYSTEM_CALLER: Address = address!("0xff…fe")` | `consts.rs:487` |
| `pub fn fault_tolerance` (тело) | `math.rs:32-38` |
| `pub fn epoch_of_block` | `reader.rs:303-321` |
| `sol!`-блоки: `evm.rs:581`, `reader.rs:109`, `slasher_sink.rs:78`, `slasher/actor.rs:169` | четыре штуки |
| литерал `current_epoch + 2` (×4: код + три теста) | `evm.rs:902,1727,1739,1755` |

---

## 4. Находки

**Ф-1. Три вьюхи объявлены у узла на 32 бита, а контракт хранит и отдаёт 64.**
Опора: `contracts/staking/src/storage.rs:27-30` (`StorageU64` ×3),
`config.rs:192,205,231`; `reader.rs` `sol!` и `node/src/evm.rs` `sol!` до правки.
Уверенность: **[KNOWN]** — обе стороны открыты в этой сессии.
Чем пытался опровергнуть: проверил, что сеттеры принимают `U32Command`
(`config.rs`), то есть значения по построению влезают в `u32`, и что декодер
alloy отверг бы более широкое слово (направление отказа безопасное).
Связано: R-108 (та же запись).
Последствие, если не трогать: сегодня — никакого; при первом же значении
`> u32::MAX` (например, интервал эпохи, поднятый до большого числа) узел получил
бы `AbiDecode` на пути границы эпохи вместо числа, то есть `Corruption` у
follower. Гейтом это не прикрыто — прикрыто только тем, что сеттер уже,
чем хранилище.

**Ф-2. `epoch_of_block` и `epoch_at_block` расходились и по семантике, и по
порядку аргументов.** Опора: два тела функций, приведены дословно в §2.2.
Уверенность: **[KNOWN]**.
Чем пытался опровергнуть: искал, достижим ли `activation == 0` на живом пути
узла — `scheduled_dpos_activation` (`reader.rs`) сворачивает `0` в `None`,
`node/src/evm.rs::classify_scheduled_activation` тоже, и трейт-док прямо это
описывает. Не опровергнуто: на девнете `production_path` инициализирует контракт
с `dposActivationBlock = 0`, так что контрактный арм — живой код, а не мёртвый.
Связано: R-102.
Последствие, если не трогать: перестановка аргументов на любом из ~10 вызовов
компилируется (оба `u64`), и узел молча считает чужие эпохи; при
`activation > 0` формулы совпадали, поэтому ни один тест этого не ловил.

**Ф-3. Комментарий узла отрицал существующий контрактный обработчик.**
Опора: цитата в §2.3 против `lib.rs:127` + `consensus.rs:905` + скан блоба.
Уверенность: **[KNOWN]**. Связано: R-104.
Последствие, если не трогать: следующий читатель принимает решение о пути улик
по несуществующей дыре — ровно то, что уже случилось с «MERGE CHECKLIST».

**Ф-4. `reader.rs` документировал несуществующий дрейф арности.**
Опора: цитата в §2.4 против `consensus.rs:764`. Уверенность: **[KNOWN]**.
Связано: R-107, R-119. Последствие: то же, что Ф-3.

**Ф-5. 110-строчный «MERGE CHECKLIST» описывал слияние, которое уже
произошло.** Опора: блок `slasher/actor.rs:63-172` против `consts.rs`
(четырёхаргументные `slashEquivocation*`), `git log` (`f16fdd90`).
Уверенность: **[KNOWN]**.
Чем пытался опровергнуть: сверил три селектора с блобом — по одному вхождению,
шестиаргументных форм (`0x2bc5fb10` / `0xb034c58b` / `0x337e1437`) в дереве нет.
Последствие: документ, который сам себя объявлял «canonical copy», указывал в
противоположную сторону от кода.

**Ф-6. `getUndelegatePeriod` объявлялась узлом и никем не вызывалась.**
Опора: `grep` по `crates`/`bins`/`e2e` — единственное вхождение было в самом
`sol!` и в `view_selectors_are_pinned`. Уверенность: **[KNOWN]**.
Связано: R-084 (там же про неиспользуемый `activationEpoch`).
Последствие: мёртвое объявление, которое пин делал похожим на живое.

**Ф-7. Шесть событий совпадают побайтно, и это выяснилось впервые.**
Опора: `close_event_topics_match_the_shared_abi` (новый тест, зелёный),
`cast keccak` по шести сигнатурам.
Уверенность: **[KNOWN]** — тест прогнан здесь.
Чем пытался опровергнуть: `EpochCommitteeCommitted` совпал с hex-литералом
`015ffbf0…`, который в `evm.rs` был снят с контракта независимо, — это
третий свидетель для одной из шести.
Последствие, если бы не совпало: `decode_log` молча не срабатывает, и
`PartialEpoch`/`EpochWeightsUnavailable` — два события, вся ценность которых в
том, что они ломают тишину, — не печатаются никогда.

**Ф-8. Блоб уменьшился на 4 358 (wasm) / 23 822 (rwasm) байта.**
Опора: `stat` до и после, дайджесты в `STAKING_ARTEFACT.md`.
Уверенность: **[KNOWN]** — числа измерены; **[ГИПОТЕЗА]** — причина.
Правдоподобно: `sig::<C>()` сворачивается в ту же константу, что давал макрос,
а несколько дублированных констант и одна дублированная функция схлопнулись;
генерируемые `alloy-sol-types` пути кодирования — мёртвый код, линкер их
выкидывает. Не проверял, какой из двух крейтов даёт какую часть.
Последствие: никакого — но это ровно то место, где «ничего не изменилось» было
бы неправдой, поэтому записано.

**Ф-11. Единственный двусторонний сверщик был сломан этой же правкой, и один
его чек стал fail-open.** `devnet/local-dpos-smoke/scripts/xp/agreement_check.py`
читал `sol!`-блоки из `evm.rs` и `reader.rs`, которых больше нет ⇒
`node_signatures()` возвращал пустой словарь ⇒ G3 печатал
`[ok ] G3 ABI signatures: 0 node signatures all declared…`. Остальные группы падали
в `Unreadable` (литералы стали `pub use`), и весь офлайн-прогон выходил с кодом 3.
Опора: прогон скрипта. Уверенность: **[KNOWN]**.
Последствие, если не трогать: прибор, который ловил именно этот класс дефектов,
рапортует «ok», проверив ноль величин. Хуже, чем его отсутствие.
Исправлено: G3 читает общий крейт и **падает**, а не проходит, если деклараций ноль
(`NoSignatures`, отдельно от `Unreadable`); G4 сверяет общий `sol!` с Rust-кортежем
`write_returns`; G5-G14 проверяют не «два литерала равны», а «литерал не отрос
обратно и обе стороны импортируют общий». Прогон: **12 checks, 0 disagree, 0 unread**.
Fail-closed проверен мутацией (подмена пути к общему крейту → `1 disagree`).

**Ф-9. `cargo fmt --check` в корне был КРАСНЫМ до моей работы.**
Опора: `git show HEAD:crates/dpos/consensus/src/cert_inlet.rs | rustfmt --check`
— два расхождения на HEAD, в строках, которых я не касался. Всего шесть файлов:
`consensus/src/{application,beacon/plane,cert_inlet,executor,spec_exec}.rs`,
`genesis-bootstrap/tests/bootstrap_smoke.rs`. Уверенность: **[KNOWN]**.
Последствие: ворота `cargo fmt --check` в корне нельзя прочитать как
«зелёные/красные» без этого списка. Мои файлы чистые; чужие не трогал.

**Ф-10. Шесть `as u64` в `dpos.rs` стали избыточными и ловились clippy.**
Опора: `cargo clippy -p fluentbase-consensus --all-targets` после расширения
`interval` до `u64`. Уверенность: **[KNOWN]**. Убраны — иначе ворота clippy
покраснели бы по моей вине.

---

## 5. Оставлено как есть

- **`crates/dpos/consensus/src/epocher.rs::OriginEpocher::containing`** — была
  ТРЕТЬЯ запись формулы эпохи, и §2.2 в первой редакции этого не заметила. Правка:
  деление берётся из `staking_protocol::epoch_at_block`, а собственное правило типа
  (`None` для высоты ниже origin — доактивационный блок не принадлежит ни одной
  относительной эпохе, в отличие от клэмпа в 0 у общей функции) остаётся на месте с
  комментарием, почему оно не дубль.
- **`initialize` и `commitEpochCommittee` в `genesis-bootstrap/src/bootstrap.rs`** —
  собственный `sol!`, третья копия; `initialize` дублируется ещё в четырёх файлах
  `e2e/src/staking*.rs`. Это не тест: `bootstrap.rs` строит genesis-состояние живого
  стенда, и смена арности `initialize` там СОБЕРЁТСЯ и отревертит на genesis-init.
  Не тронуто (стенд), но заявление крейта «они нигде не дублируются» исправлено —
  теперь дырка названа в его же доке. В план вынесено.
- **`crates/dpos/bls/src/scheme.rs:7`** — единственный оставшийся «MUST mirror»
  в `crates/`. Он про ПОРЯДОК комитета (группа 2), а не про величину, и группа 2
  отложена на 2.3. Ссылается на `Staking.sol`, которого больше нет, — это
  устаревшее имя, но правка комментария без правки механизма ничего не удержит,
  а механизм — предмет 2.3.
- **`fluent-stf-sp1` / `dpos_exec.rs`** — третья независимая копия ABI
  (§10.4, §13 правило 15 архитектуры). Не в этом дереве (ветка
  `feat/dpos-committee-cert-verify`), трогать нечем. Отмечено в §13
  архитектуры: при слиянии он должен импортировать `fluentbase-staking-abi`.
- **`crates/node/src/cert_follow/l1.rs:16` и `crates/revm/src/bridge.rs:12`** —
  ещё два `sol!` в `crates/node`/`crates`. Первый — ABI контракта L1-rollup
  (Ethereum), второй — мост revm; ни один не про стейкинг-контракт, дубля у них
  нет.
- **Остальные ~120 селекторов контракта** через `derive_keccak256_id!` —
  `initialize`, все сеттеры, ERC-20, `devnet-views`, все `ERR_*`. Узел их не
  зовёт, второй декларации у них нет, значит и делить нечего. Правило переноса
  было: «то, что зовёт узел или стенд через узел».
- **`StorageUint112`** (`crates/sdk`) — ширина `112` встречается там ещё раз, в
  имени типа SDK. Заводить константу в SDK ради имени — не дедупликация, а
  переименование; `math::U112` теперь выведён из общей константы, и это та
  копия, которая ходила по контракту.
- **`epoch_committee_return_matches_the_contract_abi_encoding` и
  `registry_return_matches_the_contract_abi_encoding`** (`reader.rs`) — они
  выглядят как «кодирую своим типом», но пин у них — вектор `cast abi-encode`,
  внешний. Оставлены.
- **`peer_pubkey_ord_is_raw_byte_lex`** (`reader.rs`) — группа 2, оставлен как
  есть до 2.3.
- **Тестовые константы `COMMITTEE_N = 4`, `CHAIN_ID = 20_994` и т.п.** —
  `DUPLICATES.md` их сам исключил: копии внутри тестов, в работе не участвуют.
- **Шесть файлов с довиновным дрейфом `rustfmt`** (Ф-9) — не мои, не трогал.

---

## 6. Что случится, если применить наполовину

**Только общие крейты, без проводки.** `crates/types` получает модуль, который
никто не импортирует, `crates/staking-abi` — крейт, который никто не
использует. Всё собирается, все тесты зелёные, кроме двух новых в
`crates/staking-abi` (они самодостаточны и тоже зелёные). Ноль изменений в
поведении и ноль пользы: четырнадцать пар копий остаются на месте. Единственный
видимый след — `cargo check --workspace` компилирует два лишних крейта.

**Только контракт.** Контракт зависит от `fluentbase-staking-abi` и
`staking_protocol`; узел продолжает нести свои `sol!` и свои литералы. Сегодня
всё сходится, потому что значения равны, и селекторный скан это подтверждает.
Но `getEpochBlockInterval` у узла остаётся `uint32` против контрактных 64 бит,
а `MAX_ACTIVE_VALIDATORS_LENGTH` исчезает из контракта под именем
`MAX_COMMITTEE_SIZE` — узел этого не заметит, потому что читает своё. То есть
получается ровно исходная болезнь, только с одной стороны она теперь
механизирована, а с другой нет: правка общего крейта ломает сборку контракта, а
узел молча расходится. Хуже исходного состояния тем, что появляется ложное
ощущение «общий источник есть».

**Только узел.** Узел импортирует оба крейта, контракт остаётся на
`derive_keccak256_id!` и своих литералах. Собирается; `cargo test -p
fluentbase-node -p fluentbase-staking-reader` зелёные; **но контрактный тест
`close_event_topics_match_the_shared_abi` не существует**, а
`derived_selectors_match_independent_hex_pins` продолжает сверять контрактные
константы с hex — то есть контракт по-прежнему прибит к внешнему свидетелю, а
узел к общему крейту, и связь между ними — снова только через тот же hex.
Разница с исходным состоянием: у узла ушли четыре `sol!` и три «MUST mirror», у
контракта не изменилось ничего. Это единственная из трёх половин, которая не
делает хуже, — но и группы 5-14 она не закрывает: удерживать одностороннюю
константу нечем.

Общий вывод: коммиты не переставляются местами. `feat(types)!` (общие крейты +
проводка контракта) обязан идти первым, потому что `refactor(dpos)!` без него не
компилируется вовсе, а он без второго компилируется и создаёт видимость
готовности.

---

## 7. Вне объёма

- Группы 1 и 2 (`DUPLICATES.md`) — namespace подписи и порядок комитета: уходят
  в 2.3 после решения Д-4; в §5 записано, что именно осталось.
- `2.4` (`ProtocolParams` в chainspec) не трогалась: `epochBlockInterval` и
  `dposActivationBlock` по-прежнему читаются с цепи.
- `fluent-stf-sp1::dpos_exec.rs` — третья копия ABI, вне дерева.
- Довиновный дрейф `rustfmt` в шести чужих файлах — не правил.
- `.gitignore`-блобы и `.vendor-sha` — механика вендоринга блоба (кандидат на
  снятие ритуала, F3/F9 в `PLAN.md`) не менялась.
- `devnet/local-dpos-smoke/**` кроме `contracts/STAKING_ARTEFACT.md` и двух
  блобов — не трогал (блобы в `.gitignore`, в коммит не попадают).

---

## 8. Где проверка была самой слабой

0. **`agreement_check.py` (Ф-11).** Самое слабое место, и в первой редакции §8 его
   не было вовсе: работа сняла дубликаты и одновременно ослепила прибор, который
   дубликаты ловил, причём в режиме «зелено, проверено ноль». Найдено ревью, не мной.
1. **Живой прогон.** См. §9 — записан отдельно, потому что это единственное
   место, где «узел и контракт согласны на живой цепи» проверяется не тестом.
2. **Ф-8, причина уменьшения блоба.** Числа измерены, объяснение — гипотеза. Не
   разделял вклад двух крейтов, не смотрел на секции wasm.
3. **`no_std` контракта.** `alloy-sol-types` собирается под `wasm32` — это
   доказано тем, что блоб собрался и уменьшился. Но я не проверял, что в него не
   попал ни один байт кодека `alloy` (только то, что суммарно стало меньше);
   утверждение «линкер выкидывает» — вывод из размера, не из дизассемблера.
4. **Ширина `uint64` у трёх вьюх.** Что селекторы не сдвинулись — проверено
   сканом. Что декод 64-битного слова на узле работает против ЖИВОГО контракта —
   проверяется только живым прогоном (§9), потому что юнит-тесты кодируют и
   декодируют одним и тем же типом, а e2e `staking*` эти три вьюхи не читает
   через `staking-reader`.
5. **Группа 7.** `COMPACT_STAKE_BITS` связал `math::U112` и
   `reader::MAX_COMPACT_STAKE`, но `StorageUint112` в SDK остался третьим местом,
   где живёт `112`. Одностороннее изменение общей константы даст ошибку
   компиляции в контракте (`Uint<113,2>` не сойдётся с `StorageUint112`), — я на
   это рассчитываю, но не проверял мутацией.
6. **Мутационная проверка новых тестов не делалась.** Ни один из трёх новых
   тестов (`selectors_match_the_deployed_artefact_scan`,
   `close_event_topics_match_the_shared_abi`,
   `the_shared_fault_budget_is_commonwares_budget`) не был поломан нарочно, чтобы
   увидеть, что он краснеет.

---

## 9. Ворота

| Ворота | Результат |
|---|---|
| `cargo test` из `contracts/staking` (без фичи) | **175 passed, 0 failed**. База без фичи не замерялась — 174 из брифа относится к прогону С фичей |
| `cargo test --features devnet-views` | **176 passed, 0 failed** (база 174/0; +2 — `close_event_topics_match_the_shared_abi` и `the_view_returns_decode_under_the_node_s_declaration`) |
| `cargo clippy --all-targets --features devnet-views` (контракт) | чисто |
| `cargo fmt --check` (контракт) | чисто |
| `cargo check --workspace` (корень) | чисто |
| `cargo test -p fluentbase-e2e --release` | **113 passed / 9 failed / 9 ignored** — те же девять `builtins::*`, что и до слияния; все `staking*` зелёные |
| `cargo test -p fluentbase-node` | **57 passed, 0 failed**. 59 — замер на этом же дереве до удаления двух селекторных тестов-копий, не на `HEAD` |
| `cargo test -p fluentbase-staking-reader` | **59 passed, 0 failed**. База не замерялась запуском; по счёту `#[test]` в `HEAD` было 63 (32 в `reader.rs` + 31 в `epoch_transition.rs`), стало 59 (28 + 31): −4 теста эпохи, −2 теста-копии, +1 замена (`a_zero_activation_is_absolute_numbering_on_this_side`), +1 сверка с commonware |
| `cargo test -p fluentbase-consensus` | **608 + 3 + 5 + 13 passed, 0 failed** |
| `cargo test -p fluentbase-p2p -p fluentbase-bls` | зелёные |
| `cargo clippy --all-targets` на затронутых крейтах | чисто (после снятия шести `as u64`) |
| `cargo fmt --check` (корень) | красный **на шести чужих файлах, красный и на HEAD** — см. Ф-9. Мои файлы чистые |
| Пересборка блоба (`cargo clean -p fluentbase-contracts` → `cargo build --release -p fluentbase-genesis --features devnet-views`) | собрано |
| Селекторный скан | **selector scan OK**, все три группы; плюс десять точек, добавленных в группу «ровно 1» |
| Дайджесты записаны в `STAKING_ARTEFACT.md` | да, старые демотированы в «Previous build» |
| Пересборка образа стенда (`docker compose build`) | `Image fluent-dpos-smoke:local Built`, exit 0 |
| `make case-growth` | **PASS** |

### Живой прогон

`make case-growth` на пересобранном образе и новом блобе — **PASS**, exit 0:

    CASE-GROWTH PASS: committee grew across 2 boundaries, finalized advanced
    fin0=132→finN=423 (~9 epoch-span, of which 35 came AFTER the last joiner
    seated); finalized advanced 35 block(s) (>= 32);
    dpos_dkg_pinned_idx_out_of_range_total=0 on 6/6 validator(s);
    no pinned-idx ERROR across 6 validator node(s)

Дошёл ли он до кода, а не упёрся в стенд — да. `commitEpochCommittee` —
fail-loud системный вызов: промах селектора был бы ревёртом
`ERR_UNKNOWN_METHOD` в блок-исполнении на каждом узле. Он прошёл, и его событие
декодировалось (`docker logs fluent-dpos-sim-validator-0-1`):

    2026-09-09T08:20:50.666001Z  INFO fluentbase::consensus: epoch_committee_committed epoch=3 members=4
    2026-09-09T08:21:23.142581Z  INFO fluentbase::consensus: epoch_committee_committed epoch=4 members=4
    2026-09-09T08:21:55.126706Z  INFO fluentbase::consensus: epoch_committee_committed epoch=5 members=5

Эти три строки — сразу два свидетельства на живой цепи, которых нет ни в одном
юнит-тесте: (1) calldata, собранная узлом из общего крейта, диспетчеризовалась
контрактом, собранным из того же крейта; (2) topic0 события
`EpochCommitteeCommitted`, объявленного в общем `sol!`, совпал с тем, что
контракт эмитит через свой `#[derive(Event)]` — иначе `decode_log` промолчал бы
и строки не было бы вовсе. Плюс `recordProduction` (тоже fail-loud) отработал на
каждом из 423 блоков, а `getEpochCommitteeWithStakes` / `getDkgQual` /
`getEpochBlockInterval` / `getDposActivationBlock` / `nextEpochToCommit`
прочитались на каждой границе. Из тройки §2.1, где ширина возврата поменялась с
`uint32` на `uint64`, живой прогон покрывает НЕ три вьюхи, а полторы:
`getEpochBlockInterval` читается на каждой границе, `getActiveValidatorsLength` —
один раз на старте (`dpos.rs`, проверка потолка), а `getUndelegatePeriod` узел не
читает вообще — его декларация удалена (§2.7). Комитет вырос 4 → 5 → 6 через две
границы, чего без корректного чтения интервала и курсора не случилось бы, но это
свидетельство про одну вьюху, а не про три.


---

## 10. Прогон контр-ревью (2026-09-09, после первой редакции)

Два ревьюера с чистым контекстом: один на два коммита, один на этот отчёт и
архитектурные секции. У обоих был отключён Bash, поэтому всё, что ниже,
перепроверено мной командой или открытым файлом, прежде чем правиться.

**Подтвердилось и исправлено:**

| # | Находка | Что сделано |
|---|---|---|
| 1 | Группа 4 **не была закрыта**: контракт кодирует ответ своим кодеком по Rust-кортежу (`consensus.rs::write_returns` + `types.rs::ConsensusKeys`), общий `sol!` с этим ничем не связан. «Второй декларации не существует» в §1 — неверно | Добавлен `the_view_returns_decode_under_the_node_s_declaration`: зовёт настоящий хендлер, отдаёт сырой вывод в `abi_decode_returns` узла. Мутации: удаление ноги → ошибка компиляции, перестановка двух → красный ассерт. §1 переписана |
| 2 | `agreement_check.py` сломан и fail-open (Ф-11) | Переписан под новую форму, G3 fail-closed, 12/12 зелёных, fail-closed проверен мутацией |
| 3 | `epocher.rs::OriginEpocher::containing` — третья запись формулы эпохи, §2.2 её не заметила | Деление берётся из общей функции; собственное правило типа (`None` ниже origin) осталось с объяснением |
| 4 | `dpos.rs` в тексте ошибки велит оператору править `MAX_ACTIVE_VALIDATORS_LENGTH` в `consts.rs` — символа нет; `outer.rs` пишет «Mirrored constant: MAX_ACTIVE_VALIDATORS_LENGTH» | Оба текста переписаны на одну общую константу |
| 5 | `cert_inlet.rs` описывает режим отказа удалённой функции («`epoch_of_block` div-by-zeroes») | Переписано под `epoch_at_block → None` |
| 6 | `slasher_integration.rs` и `equivocation_evidence_conformance.rs` в сообщениях ассертов посылают за шестиаргументным вариантом «на ветке `feat/flu-989-port-solidity-delta`»; `slasher_sink.rs` — туда же | Все шесть сообщений переписаны; `grep feat/flu-989-port-solidity-delta crates/` пуст |
| 7 | `is_epoch_boundary(block, interval, activation)` осталась соседкой `epoch_at_block(block, activation, interval)` с переставленными аргументами, оба `u64` — перестановка компилируется | Порядок выровнен, в доке сказано почему |
| 8 | Док крейта ABI: «Every selector … pinned against the artefact scan» — четырнадцатая запись (`AlreadySlashedForEquivocation` `0x8300031d`) в скане отсутствует и не может там быть (скан ходит по хендлерам) | Док исправлен, назван настоящий свидетель — `e2e/src/staking_bls.rs`, живой ревёрт rWasm |
| 9 | Док крейта ABI: «Handlers the node never calls … are not duplicated anywhere» — ложь: `initialize` и `commitEpochCommittee` объявлены ещё раз в `genesis-bootstrap/src/bootstrap.rs`, `initialize` — ещё в четырёх `e2e/src/staking*.rs` | Дырка названа в доке крейта и в §5; правка стенда вынесена в план |
| 10 | Протухшие якоря в правленных секциях: `reader.rs:375/687-691/696-697/213-229/603/839`, `evm.rs:590/739/787/789/1073/297,331,1041`, `consts.rs:160/402,444`, `dpos.rs:2315-2331` | Заменены на имена символов — номер строки протухнет снова, имя нет |
| 11 | `08_node_integration:256-267` продолжал утверждать СЕМЬ close-арм, включая `StipendSkipped`/`StipendLegSkipped`, удалённые 2026-09-07 | Исправлено на пять, с пометкой, что стал устаревшим счёт, а не код |
| 12 | `07_slashing:353-359` — блок `[UNVERIFIED out-of-tree]`, прямо противоречащий абзацу над ним | Снят, заменён на `[RESOLVED]` с фактами из дерева |
| 13 | `13_invariants` rule 8 и `12:22`, `12:32` описывали пару зеркал и якорь `consts.rs:160` | Переписаны |
| 14 | `10_tee_sp1:97` «`sol!` COPIED from node `evm.rs`» — у `evm.rs` его больше нет | Переписано: копировать теперь у общего крейта |
| 15 | Арифметика ворот: новый тест не под фичей, значит обе цифры должны были сдвинуться | Перезамерено: **175 без фичи / 176 с фичей**; база без фичи не замерялась, так и записано |
| 16 | §9 подавал покрытие `uint64` живым прогоном как тройное | Исправлено: полторы вьюхи из трёх |

**Не подтвердилось (одна находка):** ревьюер кода заявил, что `unwrap_or(0)` на
`interval == 0` в `evm.rs` — изменение поведения, потому что старый код делил и
падал. Проверено: `git show f16fdd90:crates/node/src/evm.rs` — там стоит
`let current_epoch = if interval > 0 { … } else { 0 };`. Поведение байт-идентично,
находка снята.

**Оставлено осознанно** (записано в план, а не сделано):

- `genesis-bootstrap/src/bootstrap.rs` и `e2e/src/staking*.rs` — копии `initialize`
  и `commitEpochCommittee`. `bootstrap.rs` уже имеет незакоммиченные чужие правки.
- `fluent-stf-sp1::dpos_exec.rs` — четвёртая копия ABI, вне дерева.
- Протухшие якоря в файлах стенда (`bootstrap_smoke.rs:17` `consts.rs:388`,
  `compose_gen.py:101` `dpos.rs:931`) — стенд.
- Питоновские литералы `51` в стенде (`compose_gen.py`, `floor_halt_case.py`,
  `seed_continuity.py`) за общей константой не следуют. Это остаток группы 8,
  который не ловится компилятором ни при каком устройстве кода на Rust; его ловил
  бы `agreement_check.py`, если добавить туда чек — не добавлял.
- `15_smoke_cases_as_behavioral_spec_devnet_lo.md:41` и
  `00a_errata_2026_08_10_full_audit.md:11` описывают `carry_committee_forward` и
  `CommitteeCarriedOver`, удалённые 2026-09-07. Дрейф не мой, но живой.
