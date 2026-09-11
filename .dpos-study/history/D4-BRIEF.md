# Д-4. Бриф под решение: один маршрут слэшинга или два

Читано в этой сессии: контракт `contracts/staking/src/{lib,consensus,evidence,bls,consts,storage,tests}.rs`;
узел `crates/dpos/consensus/src/{application.rs,slasher/{actor,evidence,ingress}.rs,committee/{mod,store}.rs,lib.rs}`,
`crates/dpos/bls/src/{lib,encoding,scheme}.rs`, `crates/node/src/{evm,slasher_sink}.rs`, `crates/types/src/staking_protocol.rs`;
тесты `crates/dpos/consensus/tests/{equivocation_evidence_conformance,slasher_integration}.rs`, `e2e/src/staking_bls.rs`;
commonware — checkout `~/.cargo/git/checkouts/monorepo-27b478c9bb41d208/3c4e02c` (это пин: `Cargo.lock:3253-3255`,
`tag=v2026.4.0#3c4e02c`; в `.claude/COMMONWARE_INTERNALS.md` записан ДРУГОЙ каталог того же коммита,
`monorepo-9732103c47eb4665` — на диске есть оба, содержимое я сверял только по первому).

Всё ниже — `[KNOWN]`, если не помечено иначе.

---

## §1 Что есть по коду

### 1.1 Системный маршрут

| | |
|---|---|
| Селектор | `SIG_SLASH_EQUIVOCATION` = `sig::<abi::slashEquivocationCall>()`, `consts.rs:193`; литеральный пин `0xdc6fb3f2` — `tests.rs:1344` |
| Диспетчер | `lib.rs:129` |
| Обработчик | `consensus::slash_equivocation`, `consensus.rs:883-911` |
| Подпись | `slashEquivocation(uint64 epoch, uint32 signerIdx)` — `crates/staking-abi/src/lib.rs:126` (ОДНА декларация на обе стороны) |

Кто вызывает во всём дереве (`git grep -n "slashEquivocation" -- crates/ bins/ e2e/ devnet/ contracts/`):

- `crates/node/src/evm.rs:606` — `sol!`-импорт `slashEquivocationCall` из `fluentbase-staking-abi` (`evm.rs:588`);
  единственный продакшн-вызов — `evm.rs:1163-1177`, `encode_slash_equivocation_call(current_epoch, accused)`;
- `devnet/local-dpos-smoke/scripts/xp/agreement_check.py:918` — строка подписи в списке селекторов для
  сверки блоба; это не вызов;
- `contracts/staking/src/tests.rs` — хелпер `system_slash` (`tests.rs:7447`) и пять тестов
  (`:7095`, `:7494`, `:7549`, `:7617`, `:7723`);
- вызывающих в `bins/` и `e2e/` нет.

Что проверяется, по шагам:

| проверка | якорь |
|---|---|
| не payable / не в static-кадре / инициализирован | `consensus.rs:884-886` |
| `contract_caller() == SYSTEM_CALLER`, иначе `ERR_ONLY_SYSTEM_CALL` | `consensus.rs:887-889` |
| декод `EpochSignerCommand` | `consensus.rs:890` |
| личность: `committee_member_at(epoch, signer_idx)` | `consensus.rs:891` → `:636-657` |
| комитет эпохи закоммичен: `len != 0`, иначе `ERR_EPOCH_COMMITTEE_NOT_COMMITTED` | `consensus.rs:641-644` |
| индекс в пределах длины, иначе `ERR_SIGNER_INDEX_OUT_OF_RANGE` | `consensus.rs:645-651` |
| окна по эпохам | НЕТ |
| улики и подписи | НЕТ, их нет в calldata |
| повтор: уже tombstoned ⇒ тихий `Ok(())`, без ревёрта | `consensus.rs:903-909` |

Успех: `apply_equivocation_penalty(validator, command.epoch)` — `consensus.rs:910`.

### 1.2 Маршрут улик

| | |
|---|---|
| Селекторы | `consts.rs:195`, `:197`, `:199` — из `abi::slashEquivocation{Notarize,Finalize,NullifyFinalize}Call` |
| Диспетчер | `lib.rs:130-132` |
| Обработчики | `consensus.rs:978-987`, `:992-1001`, `:1006-1015` — все три уходят в `slash_from_evidence` (`:913-973`) |
| Подписи | `crates/staking-abi/src/lib.rs:130-135` |

Кто вызывает во всём дереве:

- `crates/dpos/consensus/src/slasher/actor.rs:69-72` — реэкспорт трёх `*Call` из `fluentbase-staking-abi`;
  кодировщик `encode_calldata` (`actor.rs:1122-1150`); единственный продакшн-путь к нему —
  `enqueue_fallback` (`actor.rs:844-917`) → WAL → `run_consumer` (`actor.rs:1007-1067`) →
  `SlasherTxSink::submit` → `crates/node/src/slasher_sink.rs:266`;
- `crates/dpos/consensus/tests/equivocation_evidence_conformance.rs` — пины селекторов и calldata;
- `crates/dpos/consensus/tests/slasher_integration.rs` — стенд со стабом стока;
- `e2e/src/staking_bls.rs:169`, `:181` — единственный вызов маршрута на настоящем rWasm;
- `contracts/staking/src/tests.rs` — хелпер `slash_with_evidence` (`:6183`) и 13 тестов;
- вызывающих в `bins/` и `devnet/` нет.

Что проверяется:

| проверка | якорь |
|---|---|
| не payable / не в static-кадре / инициализирован | `consensus.rs:979-981` (и то же в `:993-995`, `:1007-1009`) |
| гейт на вызывающего | **НЕТ НИ ОДНОГО** — ни `SYSTEM_CALLER`, ни `ensure_governance`; тесты шлют от обычного EOA `EQUIVOCATION_RELAYER` (`tests.rs:6179`, `:6189`) |
| структура blob | `evidence::decode` — `consensus.rs:919` → `evidence.rs:62-76` |
| один подписант в обеих половинах | `evidence.rs:86-92` (и `:125-131` для nullify/finalize) |
| один раунд в обеих половинах (эпоха И вид) | `evidence.rs:93-99`, `:132-138` |
| пропозалы РАЗНЫЕ (только для conflicting) | `evidence.rs:100-106`; для nullify/finalize проверки нет и она невозможна — `evidence.rs:139-141` |
| точная длина blob, без хвоста | `evidence.rs:298-303` |
| личность: `bls_pubkey_owner[keccak256(compress_g2(pk))]`, ноль ⇒ `ERR_EQUIVOCATION_KEY_NOT_REGISTERED` | `consensus.rs:920-932` |
| комитет эпохи улики | **НЕ ЧИТАЕТСЯ ВООБЩЕ** |
| эпоха активации ключа против эпохи улики | **НЕ ПРОВЕРЯЕТСЯ** |
| окно по эпохам | НЕТ |
| повтор: уже tombstoned ⇒ **РЕВЁРТ** `ERR_ALREADY_SLASHED_FOR_EQUIVOCATION` | `consensus.rs:935-941` |
| консенсусные ключи установлены | `consensus.rs:942-945` |
| поданные сжатые подписи совпадают с теми, что внутри blob | `consensus.rs:946-952` |
| две BLS-проверки, каждая под доменом СВОЕГО вида сообщения | `consensus.rs:953-971`, домен — `namespace(sdk, kind)` `consensus.rs:741-750` |

Успех: `apply_equivocation_penalty(validator, evidence.epoch)` — `consensus.rs:972`.

### 1.3 Что происходит при успехе — одинаково для обоих

`apply_equivocation_penalty` (`consensus.rs:838-871`) общая:
статус читается до тумбстоуна (`:843-847`, `ERR_VALIDATOR_NOT_FOUND` если `STATUS_NOT_FOUND`);
`tombstoned[validator] = true` (`:849-852`); если `STATUS_ACTIVE` — `remove_active` (`:853-855`);
`status = STATUS_JAIL` (`:856`); `penalty_epoch = current_epoch(sdk)` (`:857`);
`set_selection_visible(validator, false, penalty_epoch)` (`:858`);
`seize_self_stake` (`:860`) — конфискат владельцу в `slashFundAddress` или в `EQUIVOCATION_BURN_SINK`,
отказ получателя РЕВЁРТИТ всё наказание (`consensus.rs:818-820`, K-22);
события `ValidatorJailed{epoch: penalty_epoch}` и `EquivocationSlashed{epoch: conflict_epoch}` (`:861-870`).

**Разницы в последствиях между маршрутами нет ни одной.** Разница ровно в двух местах:
откуда берётся `validator` (место в комитете против владельца ключа) и что происходит при повторе
(тихий `Ok(())` против ревёрта).

### 1.4 Таблица «свойство × маршрут»

| свойство | системный | по уликам |
|---|---|---|
| кто может вызвать | только `SYSTEM_CALLER` (`consensus.rs:887`) | любой адрес; гейта нет (`consensus.rs:978-987`) |
| откуда личность | `committee_records[record_of(epoch)][signer_idx]` (`consensus.rs:652-656`) | `bls_pubkey_owner[keccak256(pk48-сжатый)]` (`consensus.rs:926-929`) |
| нужен ли закоммиченный комитет эпохи | ДА, иначе ревёрт (`consensus.rs:641-644`) | НЕТ; тест `an_uncommitted_evidence_epoch_does_not_block_a_slash` (`tests.rs:7047-7088`) слэшит по эпохе `CORPUS_EPOCH = 7`, у которой `epoch_index[7].length == 0` (`tests.rs:7058-7067`) |
| криптопроверка в контракте | НЕТ | ДВЕ пары (`consensus.rs:953-968`) |
| проверка «тот, кого назвали, сидел в комитете» | не нужна (личность И ЕСТЬ место) | ОТСУТСТВУЕТ — валидатор мог не быть в комитете никогда (`tests.rs:7137` `a_registered_but_never_activated_validator_can_be_slashed`) |
| повтор той же улики | `Ok(())`, состояние не меняется | ревёрт `AlreadySlashedForEquivocation`; пин на настоящем rWasm — `e2e/src/staking_bls.rs:181-198` |
| ревёрт виден узлу | мягко: `warn` + счётчик, состояние не коммитится (`evm.rs:1185-1207`) | обычная транзакция, ревёрт — её собственный |
| доля газа | системный вызов в предысполнении, бюджет 30 M | замерено `372 655` по записи 1.10 (в этой сессии не перепроверялось; `e2e/src/staking_bls.rs:201` печатает цифру при `--nocapture`) |

### 1.5 Досягаемость за границей своей эпохи

**Контракт.** Ни один из маршрутов не ограничен глубиной.

- Системный: `committee_member_at` читает `epoch_index[epoch]` и `committee_records[record]`
  (`consensus.rs:641`, `:652-656`); оба — `StorageMap` (`storage.rs:233-234`), пишутся только
  `commit_epoch_committee` (`consensus.rs:589-593`), не прунятся нигде; грепом `prune|Prune` по
  `contracts/staking/src/` — три попадания, все комментарии, ни одного кода
  (`consensus.rs:923`, `storage.rs:227`, `:298`). Тест
  `the_index_slash_route_still_resolves_far_past_the_retired_pruning_horizon`
  (`tests.rs:7095-7134`) разрешает `signer_idx` эпохи 0 после прыжка на
  `DEFAULT_EPOCH_BLOCK_INTERVAL * (DEFAULT_UNDELEGATE_PERIOD + 100)` блоков (`tests.rs:7118-7120`).
- По уликам: окна нет (см. §1.2), и комитет вообще не читается.

**Узел — вот где живут все ограничения, и их ТРИ, а не одно.**

1. **Блочный заряд предлагается только для эпохи ТЕКУЩЕГО раунда.**
   `next_charge(context.round.epoch().get())` — `application.rs:633-640`; гейт голосования требует
   `charged == epoch` блока и иначе `ChargeError::EpochMismatch` — `slasher/evidence.rs:143-149`,
   вызов `application.rs:900-907`; сам системный вызов подставляет эпоху ИСПОЛНЯЕМОГО блока:
   `encode_slash_equivocation_call(current_epoch, accused)` — `evm.rs:1164`.
   ⇒ **системным маршрутом наказуема ровно 0 эпох назад**: эпоха конфликта всегда равна текущей.
2. **Окно `[cursor−1, cursor]`** — это гейт на входящий госсип улик и floor хранилища голосов,
   а не на слэш: `EpochCursor::retains(epoch) = epoch <= current && epoch >= current.saturating_sub(1)`
   — `slasher/actor.rs:309-312`, вызывается на входе батча `slasher/gossip.rs:124`;
   `self.votes.retain_floor(self.epoch_cursor.get())` — `actor.rs:967`. Курсор двигает только
   собственный движок узла (`actor.rs:945-948`, провенанс — `slasher/ingress.rs:34-42`).
   ⇒ пара половинок собирается максимум в пределах одной эпохи грации.
3. **Окно чтения комитета `[anchor_epoch − 8, anchor_epoch + 2]`** — вот настоящий потолок
   транзакционного маршрута, и он в материалах Д-4 не назван.
   `CommitteeStore::window` даёт `(anchor_epoch − SCHEME_RETENTION_EPOCHS, anchor_epoch + MAX_COMMITTEE_LOOKAHEAD_EPOCHS)`;
   якорь — `committee/store.rs:178-184`;
   `SCHEME_RETENTION_EPOCHS = 8` (`crates/dpos/consensus/src/lib.rs:26`),
   `MAX_COMMITTEE_LOOKAHEAD_EPOCHS = 2` (`crates/types/src/staking_protocol.rs:73`);
   отказ до всякого чтения EVM — `committee/store.rs:437-442`.
   Ниже окна `is_transient()` ложно (`committee/mod.rs:262-268`, арм `OutOfWindow { epoch, hi, .. } => epoch > hi`),
   что в слэшере становится `HandleError::Permanent` (`actor.rs:716-724`) и заряд ВЫБРАСЫВАЕТСЯ:
   `drain_stale_charges` на Permanent зовёт `self.charges.release(key)` (`actor.rs:819-825`).
   А комитет нужен обоим шагам `enqueue_fallback`: `resolve_committee` в `drain_stale_charges`
   (`actor.rs:811`) и разрешение `signer_idx` в адрес жертвы через `committee.bimap.get` +
   `record.members` (`actor.rs:879-892`).
   ⇒ **маршрутом улик УЗЕЛ наказуем максимум 8 эпох назад**, после чего заряд удаляется навсегда.

**Ответ на вопрос §1.** Системным маршрутом — 0 эпох назад (контракт готов на любую глубину, узел
никогда не предъявляет ничего, кроме текущей). Маршрутом улик — 8 эпох назад узловым продюсером;
контрактом — любая глубина, если calldata приносит кто-то другой (улика и открытый ключ —
самодостаточные байты, `bls_pubkey_owner` не освобождается: `storage.rs:241-244`).

---

## §2 Сцепление с commonware

### 2.1 Что именно читает `evidence.rs` и чему это соответствует в checkout'е

Контракт разбирает НЕ сырую `Activity`, а `Conflicting*::<VoteScheme>::encode()` —
голую конкатенацию без тега и без длины (`evidence.rs:15-24`). Тег варианта у `Activity` ЕСТЬ
(`CW:consensus/src/simplex/types.rs:2011-2054`: `ConflictingNotarize` = `7u8`, `ConflictingFinalize` = `8u8`,
`NullifyFinalize` = `9u8`), и контракт его не видит: дискриминатор — только точка входа
(`evidence.rs:26-29`). Узел снимает тег сам, вызывая `.encode()` на самом `Conflicting*`
(`slasher/evidence.rs:328-339`, `:388-399`, `:452-463`).

Побайтовое соответствие, слева контракт — справа checkout:

| поле | контракт | commonware |
|---|---|---|
| `ConflictingNotarize` = `Notarize` ‖ `Notarize` | `evidence.rs:185-193` | `types.rs:2293-2298` |
| `ConflictingFinalize` = `Finalize` ‖ `Finalize` | `evidence.rs:185-193` | `types.rs:2421-2426` |
| `NullifyFinalize` = `Nullify` ‖ `Finalize` | `evidence.rs:195-203` | `types.rs:2537-2542` |
| `Notarize` = `Proposal` ‖ `Attestation` | `evidence.rs:187-188` | `types.rs:872-877` |
| `Finalize` = `Proposal` ‖ `Attestation` | `evidence.rs:199-200` | `types.rs:1373-1378` |
| `Nullify` = `Round` ‖ `Attestation` | `evidence.rs:197-198` | `types.rs:1139-1144` |
| `Proposal` = `Round` ‖ `parent` ‖ `payload[32]` | `evidence.rs:269-284` | `types.rs:750-756` |
| `Round` = `epoch` ‖ `view` | `evidence.rs:258-267` | `CW:consensus/src/types.rs:612-617` |
| `Epoch`, `View`, `parent` — LEB128 uvarint | `evidence.rs:224-246` | `CW:consensus/src/types.rs:126-130`, `:332-336` (обе через `UInt`) |
| `Attestation` = `uvarint(signer)` ‖ `sig` | `evidence.rs:286-296` | `CW:cryptography/src/certificate.rs:104-109` |
| `signer` — `Participant(u32)` uvarint, отказ выше `u32::MAX` | `evidence.rs:287-290` | `CW:utils/src/lib.rs:97-101` |
| подпись пишется СЫРЫМИ байтами, без префикса длины | `evidence.rs:291` (`take(48)`) | `CW:codec/src/types/lazy.rs:174-184` |
| ширины `48` и `32` | `consts.rs:488` реэкспортирует из `crates/types/src/staking_protocol.rs:129`, `:141` — ОДИН источник на обе стороны |

Одно сознательное расхождение: контракт принимает padded-varint, commonware их не эмитит
(`CW:codec/src/varint.rs:125-127` — `if byte == 0 && self.bits_read > 0 { Err(InvalidVarint) }`),
контракт это знает и ссылается ровно на эти строки (`evidence.rs:218-223`). Якорь в комментарии
живой — я его открыл.

### 2.2 Что сломается при смене версии commonware

Прямо ломающие изменения, каждое — молча, с тем же исходом «`ERR_INVALID_EVIDENCE_ENCODING` или
`ERR_EQUIVOCATION_SIGNATURE_INVALID` на каждой улике»:

1. **Порядок полей** в любом из семи `Write` выше. Например перестановка `parent` и `payload` в
   `Proposal::write` — контракт прочитает 32 байта payload как начало varint.
2. **Смена кодировки `Epoch`/`View`/`Participant`** с varint на фиксированную ширину
   (в `CW:consensus/src/types.rs` это одна строка `UInt(self.0).write(buf)`).
3. **Добавление любого поля** в `Proposal`, `Round` или `Attestation` — `Cursor::finish`
   (`evidence.rs:298-303`) отвергнет blob как «есть хвост».
4. **Смена ширины подписи** (переход MinSig→MinPk меняет 48 на 96) — `take(BLS_SIGNATURE_LENGTH)`
   молча съест чужие байты; общий `staking_protocol.rs:129` спасает от рассинхрона сторон,
   но не от того, что ПАРСЕР надо переписать.
5. **Смена суффиксов домена** `_NOTARIZE`/`_NULLIFY`/`_FINALIZE` — `CW:consensus/src/simplex/scheme/mod.rs:122-125`.
6. **Смена `union` на `union_unique`** в `notarize_namespace` и соседях (`scheme/mod.rs:137-152`)
   — сегодня это голая конкатенация (`CW:utils/src/lib.rs:166-171`), а `union_unique` добавляет
   varint длины (`CW:utils/src/lib.rs:176-185`).
7. **Смена тела подписываемого сообщения**: `Subject::message()` возвращает `proposal.encode()`
   для Notarize/Finalize и `round.encode()` для Nullify (`CW:consensus/src/simplex/scheme/mod.rs:99-105`).
   Контракт не пересобирает эти байты, он вырезает их из blob как срез (`evidence.rs:110`, `:113`,
   `:145`, `:148`) — это защищает от рассинхрона кодировщика, но НЕ от смены того, ЧТО подписывается.

Переупаковка при откате: узел уже сегодня перекодирует улику из боевой `Scheme` (комбинированная,
97 байт на подпись: vote 48 ‖ flag 1 ‖ seed 48) в `VoteScheme` (48 байт) — `slasher/evidence.rs:240-246`,
`:326-339`. То есть на узле уже стоит ровно тот слой, который надо будет переписать под новую версию,
и он вдобавок проходит через `encode()` → `read_cfg()` round-trip, потому что поля `Conflicting*`
приватны (`slasher/evidence.rs:30-41`, `:295-300`).

**Два РАЗНЫХ проводных формата одной и той же улики в одном дереве.** Блочный заряд везёт
`activity.encode()` целиком — то есть С тегом и над комбинированной `Scheme` (97-байтные подписи):
`application.rs:641-646`, декод `Activity::read_cfg` в `slasher/evidence.rs:132-136`.
Контрактный формат — без тега и над `VoteScheme`. Один и тот же `ConflictingNotarize` едет по двум
маршрутам в двух несовместимых кодировках. Это цена, которую платит вариант (Б), и она в
материалах Д-4 не названа.

### 2.3 R-115: namespace

Три стороны:

- **узел, база:** `fluent_namespace(chain_id) = b"FLUENT_DPOS_V1_" ‖ chain_id.to_be_bytes()`
  — `crates/dpos/bls/src/lib.rs:113-118`. (В задании этот адрес указан как `bls/src/encoding.rs`
  — там его нет: `encoding.rs` целиком про EIP-2537, 124 строки, `fluent_namespace` не упоминает.)
- **commonware, суффикс:** `notarize_namespace/nullify_namespace/finalize_namespace = union(ns, SUFFIX)`
  — `CW:consensus/src/simplex/scheme/mod.rs:137-152`, константы `:123-125`, `union` — голая
  конкатенация (`CW:utils/src/lib.rs:166-171`). Само подписываемое — `union_unique(ns, msg)`
  (`CW:cryptography/src/bls12381/primitives/ops/mod.rs:55`, `:85`, `:103`).
- **контракт, всё вручную:** `b"FLUENT_DPOS_V1_"` ‖ `block_chain_id().to_be_bytes()` ‖
  `b"_NOTARIZE" | b"_NULLIFY" | b"_FINALIZE"` — `consensus.rs:741-750`; свой `union_unique`
  однобайтовым префиксом с ревёртом при `len >= 0x80` — `bls.rs:107-120`.

Сегодня сходятся байт в байт: `15 + 8 + 9 = 32 < 0x80`.

**Теста, сверяющего байты контракта и узла, НЕТ.** Что есть:

- `crates/dpos/bls/src/lib.rs:124-125` `fluent_namespace_layout_is_stable` — сверяет узловую базу с
  СВОИМИ ЖЕ литералами, суффиксов не касается, о контракте не знает;
- `contracts/staking/src/tests.rs:6247-6249` — константы `NS_NOTARIZE`/`NS_NULLIFY`/`NS_FINALIZE`,
  написанные руками ВНУТРИ контрактного крейта, с `chain_id = 0`; тест
  `each_slash_route_verifies_under_the_domain_its_kinds_name` (`tests.rs:6618-6654`) сверяет
  контрактный `namespace()` с ними. Это контракт против самого себя;
- `crates/dpos/consensus/tests/equivocation_evidence_conformance.rs` — шесть тестов, все про
  корпус, экстрактор, селекторы и раскладку calldata; `_NOTARIZE` там встречается только как часть
  имени константы `SEL_NOTARIZE` (`:578`). Namespace-байты не пинятся;
- `e2e/src/staking_bls.rs:120-202` — единственная честная сверка ПО СУЩЕСТВУ: две подписи,
  объявленные сделанными узловым blst под `_NOTARIZE` (`e2e/src/staking_bls.rs:93-95`), проходят
  контрактный `verify` на настоящих предеплоях. Но подписи лежат ХЕКС-КОНСТАНТАМИ (`:96-107`), и
  генератора для них в дереве нет (для PoP-корпуса генератор описан — `e2e/src/bls_vectors.rs:7-14`;
  для этих двух — нет). То есть тест доказывает, что контракт согласен с ТЕМ, КТО ИХ ПОДПИСАЛ, а
  кто это был — в дереве не проверяемо. `[ГИПОТЕЗА]`, что подписант шёл через
  `notarize_namespace(fluent_namespace(1))`.

Итог R-115: **расхождения сегодня нет, теста через шов тоже нет.** Дублируются ровно три величины
(префикс, порядок байт `chain_id`, три суффикса); ширины `48`/`32` НЕ дублируются — они общие через
`crates/types/src/staking_protocol.rs:129`, `:141`.

### 2.4 R-116: индексное пространство `signerIdx`

- **контракт:** `members.sort_unstable_by_key(|member| member.peer_pubkey)` — `consensus.rs:564`,
  обоснование `:556-563`; массив пишется в `committee_records[record]` в этом порядке (`:578-585`),
  `committee_member_at` берёт `at(signer_idx)` (`consensus.rs:652-656`).
- **узел:** `EpochCommittee::from_pairs` → `try_collect` в `BiMap<PeerPubkey, BlsPubkey>`
  (`crates/dpos/bls/src/scheme.rs:43-49`); `BiMap::try_from_iter` → `Map::try_from_iter`, который
  делает `items.sort_by(|(lk,_),(rk,_)| lk.cmp(rk))` (`CW:utils/src/ordered.rs:473-493`,
  обёртка `:710-720`). Индекс — позиция в этом векторе (`BiMap::value(index)`, `:670`).
- **ключ сортировки на узле:** `PeerPubkey = ed25519::PublicKey` (`crates/dpos/bls/src/lib.rs:54`),
  `#[derive(Ord, PartialOrd)]` над единственным полем `key: VerificationKey`
  (`CW:cryptography/src/ed25519/scheme.rs:123-126`); `Ord for VerificationKey` делегирует в
  `A_bytes` (`~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ed25519-consensus-2.1.0/src/verification_key.rs:122-125`),
  а `VerificationKeyBytes([u8;32])` — `#[derive(Ord)]` над массивом (`:32-34`), то есть
  байт-лексикографика над теми же 32 байтами, что контракт держит в `B256`.

**Расхождение по коду сегодня невозможно**: обе стороны сортируют по одним и тем же 32 байтам
лексикографически по возрастанию. Что его удерживает — ничто, кроме совпадения: правило записано
в двух местах разными словами, теста через шов нет (в `equivocation_evidence_conformance.rs`
порядок упомянут только в док-комментарии `:69-76`, который ссылается на
`crates/dpos/bls/tests/ed25519_ordering_conformance.rs` — тест порядка BiMap ВНУТРИ узла,
контракта он не касается).

Кто читает это пространство:

- системный маршрут — единственный потребитель `committee_member_at` (`consensus.rs:891`;
  док там же на `:634-635` говорит, что второй, `resolveSigner`, удалён);
- `recordProduction` по `leader_index` — тот же массив;
- узловой `enqueue_fallback` выходит на жертву через `bimap.get(signer_idx)` и потом ищет этот
  peer-ключ в `record.members` (`actor.rs:879-892`), то есть маршрут улик на УЗЛЕ это пространство
  тоже использует — и там расхождение поймается как `Permanent`-ошибка «BiMap-resolved peer pubkey
  not in the committee record» (`actor.rs:886-891`), а не как слэш невиновного.

⇒ R-116 при варианте (А) остаётся и становится ЕДИНСТВЕННОЙ защитой личности: расхождение
тумбстоунит честного молча. При варианте (Б) остаётся тоже, но у него появляется второй,
независимый способ выйти на личность (`bls_pubkey_owner`) — который сегодня с первым НЕ сверяется
(`slash_from_evidence` индекс подписанта даже не получает: `DecodedEvidence` его не содержит,
`evidence.rs:43-52`).

---

## §3 Что даёт каждый маршрут сверх другого

### 3.1 Досягаемость за границей эпохи — ПОДТВЕРЖДЕНО, но не в заявленном объёме

Достижимо для маршрута улик: да, и в контракте безусловно (§1.5), и на узле — но только на 8 эпох
назад (`committee/store.rs:178-184`). Утверждение Д-4 09-07 «маршрут улик не смотрит на комитет,
поэтому полная смена комитета на него не влияет» верно про КОНТРАКТ и неверно про УЗЕЛ: чтобы
собрать calldata, узлу нужен `record.members` эпохи улики, чтобы превратить `signer_idx` в адрес
(`actor.rs:879-892`). Полная смена состава этому не мешает (комитет читается с цепи, а не из
памяти), но выход эпохи за нижнюю границу окна — мешает насмерть.

Недостижимо для системного маршрута: подтверждено тремя независимыми гейтами (`application.rs:634`,
`slasher/evidence.rs:143-149`, `evm.rs:1164`), любой из которых достаточно снять, чтобы получить
неверный слэш, — поэтому снимать надо все три согласованно.

Чем пытался опровергнуть: искал в контракте хоть одну проверку эпохи на пути улик
(`grep -n "epoch" contracts/staking/src/consensus.rs` в диапазоне 913-1026 — единственное
употребление это `evidence.epoch`, переданная в `apply_equivocation_penalty`); искал в узле
второй источник адреса жертвы кроме `record.members` — нет; искал прунинг комитетов в контракте — нет.

### 3.2 «Поздний вердикт заложен» — ПОДТВЕРЖДЕНО, и это свойство ОБЩЕЕ, а не маршрута улик

`apply_equivocation_penalty(sdk, validator, conflict_epoch)` штампует наказание ТЕКУЩЕЙ эпохой
(`penalty_epoch = current_epoch(sdk)`, `consensus.rs:857`) и несёт эпоху конфликта отдельным полем
в событии (`:866-869`); док на `:831-833` называет это прямо. Но ту же функцию зовёт и системный
маршрут (`consensus.rs:910`) с `command.epoch` — то есть «поздний вердикт заложен» в ОБЩЕЙ части,
а не в маршруте улик. Тезис 09-07 верен как утверждение о контракте и не является доводом за
маршрут улик: системный маршрут не ограничен контрактом ни на эпоху, ограничивает его узел.

Чем пытался опровергнуть: искал у системного маршрута собственный, более строгий штамп — нет,
`slash_equivocation` не трогает эпоху вовсе, кроме как для резолва индекса.

### 3.3 Третье свойство — НАЙДЕНО: наказание не требует, чтобы держатель улики попал в лидеры

Системный маршрут требует, чтобы заряд СЕЛ В БЛОК: `next_charge` отдаёт заряд только тому, кто
сейчас строит блок (`application.rs:633-640`), и не более одного на блок (`:612-613`). Кто заряд
держит, но лидером не станет до конца эпохи, вердикт не проведёт — это и есть R-120.

Маршрут улик требует только, чтобы обычная транзакция попала в любой блок любого лидера:
`slasher_sink.rs:266` подаёт её в `TransactionPool` от собственного EOA слэшера. Это НЕ то же самое,
что §3.1: там речь о времени (за границей эпохи), здесь — о РОЛИ (не нужно быть избранным лидером
даже ВНУТРИ своей эпохи).

Насколько это ценно, по коду: при 86 400 блоках в эпохе почти нисколько — свободных слотов десятки
тысяч. Практически различие проявляется ровно в R-028: `next_charge` отдаёт НАИМЕНЬШИЙ ключ и
выбрасывает запись только когда жертва уже `tombstoned` (`actor.rs:208-229`), так что при
НЕСКОЛЬКИХ эквивокаторах в одной эпохе заряд с меньшим индексом занимает единственный слот на блок,
пока не сядет. Транзакционный маршрут этого ограничения не имеет: WAL отдаёт записи подряд
(`actor.rs:1016-1019`).

Три кандидата, которые я проверил и ОТВЕРГ:

1. **Наказание при остановленном консенсусе.** Недостижимо ни одним маршрутом: улика — обычная
   транзакция, ей тоже нужен блок (`slasher_sink.rs:266`). Разницы нет.
2. **Наказание валидатора, уже вышедшего из комитета.** Достижимо ОБОИМИ: системный маршрут
   резолвит по ИСТОРИЧЕСКОМУ массиву эпохи конфликта (`consensus.rs:641`), а не по живому составу.
   Не свойство маршрута улик.
3. **Устойчивость к цензуре со стороны текущего комитета.** Формально да: блок с зарядом должен
   собрать кворум, и голосующие могут отдать `false`. Но честный голосующий отдаёт `true` на
   валидной улике (`application.rs:737-750`), а коалиция размером больше `f` ломает всё остальное
   тоже. По принципу проекта («принять BFT-границу») это не свойство, а переформулировка границы.

### 3.4 Что даёт системный маршрут сверх маршрута улик

Одно, и оно не в контракте: **вердикт проверен КОМИТЕТОМ до попадания в состояние**
(`application.rs:900-907`), тогда как маршрут улик проверяет улику в контракте, платя за это двумя
парингами на цепи. И одно в контракте: **повтор не ревёртит** (`consensus.rs:903-909`), что нужно
именно потому, что два честных предлагающих могут нести один и тот же заряд.

---

## §4 Цена каждого варианта

### (А) Один системный маршрут

**Что удаляется — посчитано, не оценено.** Команда:
`.dpos-study` не трогалась; скрипт лежит в scratchpad, содержимое приведено в §7.

| файл / диапазон | строк |
|---|---|
| `contracts/staking/src/evidence.rs` целиком | 744 |
| `contracts/staking/src/consensus.rs:736-750` (`BLS_SIG_DST` + `namespace`) | 15 |
| `contracts/staking/src/consensus.rs:913-1026` (`slash_from_evidence`, три обработчика, `decode_equivocation`) | 114 |
| `contracts/staking/src/lib.rs:130-132` | 3 |
| `crates/dpos/consensus/src/slasher/evidence.rs:202-472` (три `extract_from_*`, `SlashCallArgs`, `sig_compressed`, `pk_compressed`, `check_epoch_match`) | 271 |
| `crates/dpos/consensus/src/slasher/actor.rs:102-140` (`SlasherTxSink`, `SubmitOutcome`) | 39 |
| `crates/dpos/consensus/src/slasher/actor.rs:800-836` (`drain_stale_charges`) | 37 |
| `crates/dpos/consensus/src/slasher/actor.rs:844-917` (`enqueue_fallback`) | 74 |
| `crates/dpos/consensus/src/slasher/actor.rs:1005-1170` (`run_consumer`, `init_wal_queue`, `encode_calldata`, WAL-payload) | 166 |
| `crates/node/src/slasher_sink.rs` целиком | 426 |
| `crates/dpos/consensus/tests/equivocation_evidence_conformance.rs` целиком | 852 |
| **итого** | **2741** |

Плюс не посчитанное построчно: константы `consts.rs` (11 попаданий грепом по
`ERR_EVIDENCE_*|ERR_EQUIVOCATION_*|ERR_ALREADY_SLASHED|EVIDENCE_MESSAGE_KIND|PROPOSAL_PAYLOAD_LENGTH|BLS_SIGNATURE_LENGTH`),
`EquivocationCommand` в `types.rs`, четыре объявления в `crates/staking-abi/src/lib.rs:130-135`
и их пины `:362-374`, поля `wal_writer`/`wal_reader`/`sink` в `Config` и `Actor`,
слэш-половина `e2e/src/staking_bls.rs` (замер `372 655` уходит вместе с ней).

**Тесты, которые удаляются или переписываются:**

| где | штук | что |
|---|---|---|
| `contracts/staking/src/evidence.rs` | 21 | все собственные тесты парсера |
| `contracts/staking/src/tests.rs` | 13 | гоняющие `slash_with_evidence` (12 класса «личность (улики)» плюс `the_slash_fund_rotates_immediately_because_a_refused_seizure_needs_it`, `tests.rs:12520`, добавленный работой 1.3) |
| `crates/dpos/consensus/tests/equivocation_evidence_conformance.rs` | 6 | весь файл |
| `crates/dpos/consensus/tests/slasher_integration.rs` | 14 из 16 | все, кроме `reporter_multiplex_routes_conflicting_notarize_to_slasher` (`:433`) и `slash_abi_selectors_are_pinned` (`:1205`, он тоже уйдёт — селекторов не останется) |
| `crates/dpos/consensus/src/slasher/evidence.rs` | 4 из 10 | `extract_returns_signer_index_out_of_range_for_empty_committee` (`:755`), `extract_rejects_epoch_mismatch` (`:775`), `extract_returns_signer_index_out_of_range_for_short_committee` (`:933`), `verify_pre_submit_rejects_tampered_signature` (`:796`) |
| `e2e/src/staking_bls.rs` | половина 1 | слэш-плечо единственного теста |
| **итого** | **~58** | |

**Что теряется (по §3):** досягаемость 8 эпох назад на узле и любой глубины сторонним подателем
(3.1); независимость вердикта от попадания держателя в слот лидера (3.3), с практическим
проявлением в R-028 при нескольких эквивокаторах в одной эпохе.

**Что упрощается.**
R-115 **исчезает** — контрактный `namespace()` (`consensus.rs:741-750`) удаляется, дублирующихся
величин не остаётся ни одной. Подтверждено: `namespace` не имеет других вызывающих,
`grep -n "namespace(sdk" contracts/staking/src/consensus.rs` даёт только `:955` и `:963`, оба внутри
`slash_from_evidence`. **Внимание:** свой `union_unique` в `bls.rs:107-120` НЕ удаляется — он нужен
PoP при регистрации (`consensus.rs:75-151`), так что «однобайтовый префикс против полного varint»
остаётся живым расхождением в PoP-домене.
R-116 **остаётся и становится критичнее**: после удаления `bls_pubkey_owner`-пути `committee_member_at`
— единственный способ выйти на личность, и его расхождение с `Ord` ed25519 тумбстоунит честного.
Подтверждено: `bls_pubkey_owner` при (А) НЕ удаляется — он write-once-индекс занятости ключа,
читается регистрацией (`consensus.rs:163-167` док), так что хранилище не трогается.
R-022 (WAL) исчезает целиком вместе с `actor.rs:1005-1067`.
R-028 остаётся в первой половине (`next_charge` держит наименьший ключ) и становится единственной
очередью на вердикт.
DUPLICATES группа 1 (`PLAN.md` 2.3) исчезает; группа 2 (индексное пространство) остаётся.

**Д-6** (эквивокация dealer-лога он-чейн). При (А) новый вид улики может приехать ТОЛЬКО через блок:
потребуется новый `SlashKind`, новое поле или расширение `extra_data` (сегодня три байта
`{version, leader_index, accused}`, `accused` — один байт, `actor.rs:525-528`), новый гейт голосования
и новый системный селектор. `REDESIGN.md:270` уже фиксирует, что два валидных лога одного dealer'а —
это отдельный `SlashKind` в EVIDENCE-канал; при (Б) достаточно четвёртого permissionless-селектора и
четвёртой формы в парсере. То есть **(А) делает Д-6 дороже**, и это цена, которую надо заплатить
заранее, а не потом. `[ГИПОТЕЗА]` в части «насколько дороже» — я не читал `ceremony.rs`.

**2.3 группа 1** при (А) снимается с плана целиком (см. выше). Группа 2 требует того же теста,
что и раньше: узел генерирует отсортированный комитет, контракт сверяет.

### (Б) Оба маршрута

**Что придётся содержать — список хрупких мест с якорями:**

| место | якорь контракта | якорь узла / commonware | чем ловится сегодня |
|---|---|---|---|
| кодек `Activity`-тел (7 `Write`-имплов) | `evidence.rs:185-303` | `CW:consensus/src/simplex/types.rs:872,1139,1373,2293,2421,2537`; `CW:consensus/src/types.rs:612`; `CW:cryptography/src/certificate.rs:104`; `CW:codec/src/types/lazy.rs:174` | корпус в `equivocation_evidence_conformance.rs`, скопированный в `evidence.rs:316-339` — ловит рассинхрон ПАРСЕРА, но корпус пересобирается вручную (`print_corpus`, `:822`), так что смена версии commonware даёт красноту только после ручной регенерации |
| namespace (3 величины) | `consensus.rs:741-750` | `crates/dpos/bls/src/lib.rs:113-118`; `CW:consensus/src/simplex/scheme/mod.rs:122-125,137-152` | **ничем** (§2.3) |
| `union_unique` (1 байт против varint) | `bls.rs:107-120` | `CW:utils/src/lib.rs:176-185` | контрактный ревёрт `ERR_BLS_NAMESPACE_TOO_LONG` при `>= 0x80`; расхождение ниже 128 байт невозможно |
| индексное пространство | `consensus.rs:564`, `:652-656` | `CW:utils/src/ordered.rs:473-493`; `ed25519-consensus-2.1.0/src/verification_key.rs:122-125` | **ничем через шов** (§2.4) |
| два проводных формата одной улики | `evidence.rs:15-24` (без тега, VoteScheme) | `application.rs:641-646` (с тегом, Scheme) | ничем; это два независимых кода |
| эпоха активации ключа против эпохи улики | не проверяется | — | ничем |
| `signer_idx` против владельца ключа | не проверяется (`DecodedEvidence` индекс не несёт, `evidence.rs:43-52`) | — | ничем |

**Что придётся переписать при апгрейде commonware** (по §2.2): парсер `evidence.rs` целиком, если
меняется любой из семи `Write`; перекодировщик `Scheme`→`VoteScheme` на узле
(`slasher/evidence.rs:240-246,326-339,388-399,452-463`); корпус в трёх местах
(`evidence.rs:316-339`, `equivocation_evidence_conformance.rs`, `e2e/src/staking_bls.rs:84-107`).

**Стоимость тестов — насколько 12 (сегодня 13) честны по Д-12.**
`E1-8-TESTS.md:158-170` даёт таблицу: **десять из двенадцати убиваются ЕДИНСТВЕННОЙ мутацией M91**
= «`store_consensus_keys`: убрать запись `bls_pubkey_owner`» (`E1-8-TESTS.md:315`). M91 отключает
маршрут целиком — `slash_from_evidence` получает нулевой адрес и ревёртит на `consensus.rs:930-932`,
то есть каждый из этих десяти краснеет по одной и той же причине, а не потому, что каждый сторожит
свою проверку. Честных по Д-12 из тринадцати:

- `each_slash_route_verifies_under_the_domain_its_kinds_name` (`tests.rs:6618`) — сверяет
  ПОСЛЕДОВАТЕЛЬНОСТЬ доменов, под которыми вызван verify, по маршрутам; подмена константы вида
  ловится. Честный.
- `a_blob_routed_through_the_wrong_entry_point_fails_verification` (`:6668`) — считает два
  записанных namespace, чтобы отличить ревёрт домена от более раннего гейта (`:6687-6693`). Честный.
- `equivocation_slash_tombstones_jails_and_seizes_the_self_stake_whole` (`:6722`) — убит ещё и M80,
  M80b (оставить нарушителя в активном наборе). Честный в части последствий.
- `a_seizure_stops_the_seized_bond_counting_as_stake` (`:6850`) — убит M27 (снятие WARMUP_DELAY).
  Честный, но сторожит общую, не маршрутную проверку.
- `a_slash_naming_an_unregistered_key_is_rejected` (`:7357`) — **мутация не ставилась вовсе**
  (`E1-8-TESTS.md:168`).
- Остальные восемь (`:6934`, `:7047`, `:7137`, `:7214`, `:7259`, `:7380`, `:7406`, `:12520`) — только M91.

Против какой заглушки: PAIRING — не крипто, а ПОЛИТИКА: `mock_precompile_reply` возвращает
слово-единицу, если записанный из SHA-256-преимиджа namespace входит в список принятых
(`tests.rs:657-670`), иначе ноль. Namespace извлекается из преимиджа по позиции
(`tests.rs:622-628`). Остальные четыре предеплоя отвечают детерминированной функцией от входа
(`tests.rs:603-616`). Значит: эти тринадцать проверяют МАРШРУТИЗАЦИЮ и СОСТОЯНИЕ, крипто-привязку
они не проверяют вообще. Единственный крипто-честный — `e2e/src/staking_bls.rs:120-202`, один тест.

**K-1 после 1.10.** Снят с маршрута улик полностью в той части, которая была про подменяемый адрес:
`bls.rs:37-41` — исходящие вызовы только на фиксированные `0x02/0x05/0x0b/0x0f/0x10`,
сеттера у них нет (`bls.rs:5-10`). Остаток, который НЕ закрыт инлайном и виден в коде: сами эти
предеплои — обычные rWasm-контракты, ставящиеся тем же `runtime-upgrade`, что и стейкинг
(`bls.rs:12-14` называет их адреса; `REGISTER.md:922` фиксирует остаток как R10.1). То есть
на пути конфискации по уликам остаётся ровно тот же механизм обновления, что и на пути всего
остального в цепи, — это не свойство маршрута улик.

### (В) Половина (А)

**Удалить `evidence.rs`, оставить WAL-маршрут узла.** Невозможно как состояние: WAL кормит ровно
три селектора (`actor.rs:1122-1150` — `encode_calldata` строит только их), а без парсера в контракте
эти селекторы либо удалены (и тогда `main_entry` отвечает `ERR_UNKNOWN_METHOD`, `lib.rs:134`), либо
остались и ревёртят на `evidence::decode`. В обоих случаях каждая транзакция из WAL получает
`SubmitOutcome::Failed` при пред-симуляции (`slasher_sink.rs:207-264`), НЕ акается
(`actor.rs:1050-1057`) и переигрывается на каждом рестарте процесса — вечный цикл с громким логом.
`[KNOWN]` по обеим сторонам.

**Обратное — удалить WAL-маршрут узла, оставить `evidence.rs` в контракте.** Возможно и связно:
три селектора остаются доступными любому адресу, их просто никто в этом дереве не вызывает.
Практический смысл — внешний репортер (наблюдатель, эксплорер, второй оператор) может подать улику,
которую узел не подал. Цена содержания при этом остаётся ПОЛНОЙ (весь §4(Б) кроме WAL: парсер,
namespace, кодек), а выигрыш §3.1 теряется полностью: продюсера улик в дереве не остаётся, и весь
маршрут держится на предположении, что кто-то снаружи умеет собрать `Conflicting*::<VoteScheme>::encode()`.
Такого инструмента в дереве нет (`git grep` по `encode_calldata` — только `actor.rs` и тесты).
Это худший из трёх вариантов: платится всё, не получается ничего.

---

## §5 Ответы на вопросы 1-5 из `history/DECISIONS.md` Д-4

**1. Принимаешь ли ты потерю улик за окном.**
Позиция, не факт; по коду — потеря реальна и подтверждена (§1.5 п.1). Изменилось после 09-04:
масштаб потери больше, чем записан в R-120. R-120 говорит про «последние виды эпохи». По коду
системный маршрут не покрывает НИЧЕГО, кроме текущей эпохи, вообще: `evm.rs:1164` подставляет
`current_epoch` безусловно. То есть теряется не хвост эпохи, а любая улика, чей держатель не успел
в лидеры ДО конца эпохи, плюс любая пара, собравшаяся после границы (`actor.rs:790-795`).

**2. Стоит ли K-1 удаления маршрута.**
Отпал. После 1.10 (`bls.rs:37-41`, прочитано) подменяемого адреса на пути конфискации нет; после
1.3 (`0f283a82`) `setBlendReserve` под таймлоком, `setSlashFundAddress` из-под таймлока выведен
сознательно (`DECISIONS.md` §3, 09-11). K-1 в реестре помечен закрытым (`REGISTER.md:922`).
Довод за удаление маршрута, стоявший на K-1, больше не существует.

**3. Стоит ли K-2 правки базового namespace.**
Нет, и ответ не изменился. `chain_id` входит в базу на обеих сторонах
(`crates/dpos/bls/src/lib.rs:116`; `consensus.rs:743` — `sdk.context().block_chain_id()`, то есть
из цепи исполнения, подделать нельзя). Что изменилось: проектная память требует предполагать
перезапуск сетей с блока 0, но `chain_id` при этом уникален для развёртывания, а НЕ для запуска.
Сеть-двойник с тем же `chain_id` и теми же ключами — реальный, хоть и операторский, риск. Правка
базы меняет то, под чем подписывается ВЕСЬ консенсус (`fluent_namespace` — единственный аргумент
`VoteScheme::verifier`, `actor.rs:521`), поэтому цена несоразмерна. Остаётся MINOR.

**4. Нужен ли идентификатор развёртывания в базовом namespace.**
Нет, отпадает вместе с (3). Дополнительно к 09-04: сегодня 32 байта namespace против лимита 128
(`bls.rs:116`), места хватает — ограничение не техническое, а стоимостное.

**5. Нужен ли путь для улики старше двух эпох.**
Ответ ИЗМЕНИЛСЯ по сравнению с 09-04. Тогда было записано «сегодня её никто не доносит», и это
верно про госсип (`republish` вызывается только по `Nullification`/`Notarization` внутри эпохи,
`actor.rs:975-991`). Но улика старше двух эпох УЖЕ доносится по WAL: заряд, собранный в эпохе E,
переживает границу (`ChargeStore` не прунится, `actor.rs:179-187`) и выносится на транзакционный
маршрут при каждом повороте эпохи (`actor.rs:808-836`) — до тех пор, пока `E >= anchor_epoch − 8`.
То есть путь для улики возрастом до 8 эпох существует и работает СЕГОДНЯ, а не является
предложением. Именно это вариант (А) и удаляет.

---

## §6 Рекомендация

**Вариант (А): один системный маршрут.**

Причина одна: маршрут улик покупает 8 эпох досягаемости ценой 2741 строки, из которых 744 —
парсер чужого проводного формата, чей рассинхрон с commonware не ловится НИЧЕМ, кроме вручную
пересобираемого корпуса, а два оставшихся дубля через шов (namespace и индексное пространство)
не покрыты ни одним тестом.

Что владелец теряет, выбрав (А):

- улики, собранные в последних видах эпохи, и пары, собравшиеся после границы, не наказываются
  никогда — а с ними не срабатывает ВЕСЬ хвост, висящий на он-чейн `tombstoned`:
  `TombstoneSet` (`slasher/tombstone.rs`), отказ привязывать предложение эквивокатора
  (`application.rs:839` по записи REGISTER), разрыв транспорта, и конфискация;
- Д-6 (эквивокация dealer-лога) дорожает: новый вид улики придётся везти блоком, а значит трогать
  `extra_data`, гейт голосования и добавлять системный селектор, вместо четвёртого
  permissionless-селектора и четвёртой формы в парсере;
- откат необратим дёшево: парсер `Activity`-кодека придётся писать заново под ту версию commonware,
  которая будет актуальна тогда (асимметрия, зафиксированная в `history/DECISIONS.md` «Обратимость»,
  по коду верна — все семь `Write`-имплов внешние).

Что я НЕ считаю доводом за (А), хотя оно так выглядит: K-1 (закрыт инлайном, §5.2) и
«маршрут не проверен честным тестом» (снят 1.10, `e2e/src/staking_bls.rs`).

Что я НЕ считаю доводом за (Б), хотя оно так выглядит: «контракт не смотрит на комитет, значит
глубина любая» — узел смотрит, и глубина 8 (§3.1).

---

## §7 Оставлено без разбора

- `contracts/staking/src/tests.rs` целиком (12 855 строк) — открывал только двенадцать мест,
  перечисленных в §4; остальные 130+ тестов к Д-4 не относятся, но это значит, что я не проверял,
  сколько ЕЩЁ тестов сломает вариант (А) косвенно (через общие хелперы `record_transfers`,
  `equivocation_report`, `with_filler_validators`).
- `crates/dpos/consensus/tests/slasher_integration.rs` — прочитаны только имена 16 тестов и
  грепом — какие упоминают сток; тела не читал. Счёт «14 из 16» получен грепом, не чтением,
  и может быть завышен на 1-2.
- `crates/dpos/consensus/src/slasher/{gossip,tombstone}.rs` — не открывал; при (А) `gossip`
  остаётся (он кормит `VoteStore`, а не WAL), `tombstone` остаётся тоже. Если это неверно, счёт
  §4 занижен.
- `crates/dpos/consensus/src/committee/{facade,store}.rs` — читал только окно и его предикат
  (`store.rs:165-200`, `:430-450`); как `RethAnchor` выбирает якорь, не смотрел, поэтому
  «8 эпох» — это ширина окна, а не измеренная глубина на живой цепи.
- `contracts/staking/src/bls.rs:120-330` — читал только шапку и `union_unique`; сама арифметика
  hash-to-curve к выбору маршрута не относится.
- `.dpos-study/history/AUDIT-CONTRACT.md`, `CONTRACT.md`, `CONTRACT-UNDERSTANDING.md` — читал
  выдачу грепа, не разделы. Их номера строк (`consensus.rs:1053-1074` и т.п.) устарели
  относительно дерева на сотню строк; я на них не опирался нигде.
- `e2e/src/staking_commit.rs` и `crates/dpos/consensus/src/testbed/committee_tests.rs` —
  незакоммиченная работа других сессий, не читал (правило «read only»).
- Скрипт подсчёта §4 — `scratchpad/count.sh`, `span() { awk -v A=$2 -v B=$3 'NR>=A&&NR<=B' file | wc -l; }`
  по одиннадцати диапазонам плюс `wc -l` по трём файлам целиком и `grep -c '#\[test\]'` по пяти.
- Живых прогонов не делал ни одного: `cargo test` не запускался, девнет не поднимался.

---

## §8 Всплыло

- `commit_epoch_committee` пишет указатель записи как `index.record_accessor().set_checked(sdk, record as u32)`
  (`consensus.rs:590`), где `record` — это `target` (u64-эпоха, `:585`). Молчаливое усечение при
  эпохе выше `2^32`. К Д-4 отношения не имеет.
- `verify_pre_submit` (`slasher/evidence.rs:486-500`) не имеет ни одного продакшн-вызывающего —
  только тесты; при этом док `Config::wal_writer` утверждает, что enqueue происходит «after
  `verify_pre_submit`» (`actor.rs:162`), тогда как реальный гейт — `verify_charge` (`actor.rs:516-523`),
  который зовёт `verify_pre_submit_vote_only`. Комментарий называет мёртвую функцию.
- `.claude/COMMONWARE_INTERNALS.md:6` указывает checkout `monorepo-9732103c47eb4665`, задание —
  `monorepo-27b478c9bb41d208`. На диске оба, коммит один (`3c4e02c`). Шапку доку стоит поправить
  или явно сказать, что каталог cargo не стабилен.
- В `slasher/ingress.rs` два русских маркера-заметки в коде: `:8` («наверное можно пренести в
  больший файл») и `:133` («а почему не `#[test]`?»). Они попадут в любой внешний ревью.
- `accused` едет в `extra_data` ОДНИМ байтом (`actor.rs:525-528`, `accused_index`), то есть комитет
  жёстко ограничен 256 местами на блочном маршруте, а `activeValidatorsLength` в e2e ставится 51
  (`e2e/src/staking_bls.rs:138`). При (А) это ограничение становится единственным на слэш.
- `slash_from_evidence` ревёртит на уже-тумбстоуненном (`consensus.rs:935-941`), а узловой сток
  трактует этот самый ревёрт как УСПЕХ и акает запись (`slasher_sink.rs:235-238`, `actor.rs:1040-1049`).
  Связка работает, но держится на селекторе ошибки, пиннутом литерально в двух местах
  (`slasher_sink.rs:405-421`).

---

## §9 Где проверка слабее всего

1. **Ширину окна 8 эпох я вывел из констант, а не измерил.** `SCHEME_RETENTION_EPOCHS = 8`
   (`lib.rs:26`) и `window()` (`store.rs:178-184`) прочитаны; что якорь `anchor_height` на живом
   узле идёт вровень с финализацией, я НЕ проверял — если он отстаёт, реальная глубина меньше 8.
2. **Счёт 2741 строки — это диапазоны, которые выбрал я.** Границы `actor.rs:1005-1170` и
   `slasher/evidence.rs:202-472` включают док-комментарии и могут захватывать пару строк, которые
   при (А) выживут (`SlashKind` используется и блочным гейтом). Погрешность вниз оцениваю в
   30-60 строк, вверх — в размер того, чего я не открывал (§7).
3. **«14 из 16 тестов `slasher_integration`» — греп по именам и упоминаниям, не чтение тел.**
4. **Провенанс двух подписей в `e2e/src/staking_bls.rs:96-107` не проверяем в дереве.** Я принял
   на веру комментарий `:93-95`, что они сделаны узловым blst под `_NOTARIZE`. Если это не так,
   единственный крипто-честный тест маршрута улик доказывает меньше, чем кажется, и §2.3 надо
   читать строже.
5. **Честность 13 тестов я оценивал по таблице `E1-8-TESTS.md`, а не переставляя мутации сам.**
   Открыты тела только четырёх (`:6618`, `:6668`, `:7047`, `:7095`).
6. **Про Д-6 сказано `[ГИПОТЕЗА]`** — `ceremony.rs` и `dkg_agree.rs` не открывались, опора только
   на `REDESIGN.md:270`.
7. **Предкоммитные счётчики по этому файлу:** строк с многоточием — 0; строк с нечётным числом
   обратных кавычек — проверено `awk` перед коммитом; сдвоенных запятых — 0; пустых пар обратных кавычек — 0;
   пустых круглых скобок вне кода — 0.

---

## Закрывающие ответы

1. Вариант (А), один системный маршрут: 8 эпох досягаемости не стоят 2741 строки, из которых 744 — парсер чужого формата, а два дубля через шов не покрыты ни одним тестом.
2. Системным — 0 эпох назад (узел всегда подставляет `current_epoch`, `evm.rs:1164`); маршрутом улик — 8 эпох назад узловым продюсером (`committee/store.rs:178-184` при `SCHEME_RETENTION_EPOCHS = 8`), любая глубина сторонним подателем, потому что контракт эпоху не проверяет.
3. Теста нет. Сегодня не расходится ничего: обе стороны дают `FLUENT_DPOS_V1_` ‖ `chain_id` big-endian ‖ суффикс через голую конкатенацию (`bls/src/lib.rs:113-118` + `CW:scheme/mod.rs:137-152` против `consensus.rs:741-750`), 32 байта при лимите 128.
4. 2741 строка и около 58 тестов; посчитано скриптом `scratchpad/count.sh` — `awk 'NR>=A&&NR<=B' | wc -l` по одиннадцати диапазонам, `wc -l` по трём файлам целиком, `grep -c '#\[test\]'` по пяти файлам и `awk` по вызовам `slash_with_evidence`.
5. Неверны два утверждения. 09-04 «системный маршрут не ограничен глубиной» — верно про контракт, но узел ограничивает его нулём, а не «хвостом эпохи» (`evm.rs:1164`, `application.rs:634`, `slasher/evidence.rs:143-149`). 09-07 «маршрут улик не смотрит на комитет вообще» — верно про контракт и неверно про узел: `enqueue_fallback` обязан резолвить комитет эпохи улики, чтобы назвать жертву (`actor.rs:811`, `:879-892`), и ниже окна `[anchor−8, anchor+2]` бросает заряд навсегда (`committee/store.rs:437-442`, `committee/mod.rs:265`, `actor.rs:819-825`).
6. Найдено: вердикт не требует, чтобы держатель улики получил слот лидера (транзакция едет в любом блоке, `slasher_sink.rs:266`), тогда как системный заряд едет только в блоке своего держателя (`application.rs:633-640`). Практическая ценность мала при 86 400 блоках в эпохе и проявляется только при нескольких эквивокаторах в одной эпохе (R-028).
7. Принял без чтения кода: провенанс двух хекс-подписей в `e2e/src/staking_bls.rs` (комментарий `:93-95`); честность девяти из тринадцати тестов улик (таблица `E1-8-TESTS.md:158-170`); стоимость контрактной части Д-6 (`REDESIGN.md:270`); газовые цифры `372 655` и `491 418` (запись 1.10); что `gossip.rs` и `tombstone.rs` при (А) выживают целиком.
8. Длинные строки `.dpos-study` читал `sed -n 'A,Bp'` и `awk 'NR>=A && NR<=B'` с печатью номера строки; `cut -c` и `head -c` не применял.
