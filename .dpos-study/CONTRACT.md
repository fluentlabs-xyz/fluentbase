# Граница «узел ↔ контракт staking»

Дата: 2026-09-03. Источник контракта: `/home/djadjka/Work/audit-482/pr482-study/contracts/staking`
(worktree `feat/flu-989-port-solidity-delta`, HEAD `29ae97ef`, рабочее дерево грязное —
см. часть 3). Все ссылки вида `<файл>:<строки>` без префикса — от
`contracts/staking/src/`; `node:` — от `/home/djadjka/Work/fluentbase/`.

Шкала тяжести и уверенности — из `REGISTER.md`. `[KNOWN]` здесь означает: файл открыт
и прочитан в этой сессии по обе стороны границы. Комментарии обеих сторон
доказательством не считались: там, где комментарий утверждает свойство, оно
перепроверено по реализации, и расхождение комментария с кодом вынесено в находку.

---

## Часть 1. Одиннадцать вопросов

### К-1. Может ли governance изменить `epochBlockInterval` / `dposActivationBlock` после активации?

**Нет — после активации оба поля неизменяемы; до активации меняются оба.** `[KNOWN]`

`set_epoch_block_interval` (`config.rs:532-557`) и `set_dpos_activation_block`
(`config.rs:562-586`) оба начинаются с `ensure_governance_mutation` и оба вызывают
`ensure_dpos_not_active`. Сам страж: `config.rs:25-30` —

```
if activation != 0 && sdk.context().block_number() >= activation { revert(ERR_DPOS_ALREADY_ACTIVE) }
```

То есть запрет включается ровно на блоке активации и уже не снимается. Пока
`activation == 0` («не запланировано») или `block_number < activation`, оба сеттера
открыты. Дополнительно они держат взаимную согласованность: интервал обязан делить
активацию (`config.rs:544-546` и `:572-574`, `ERR_UNALIGNED_ACTIVATION_BLOCK`), интервал
не может быть нулём (`:538-540`), активация не может быть в прошлом (`:575-577`).

**Что это меняет.** R-011 сформулирована как «governance меняет interval [на работающей
сети]; стартовавшие после узлы считают эпохи по-новому, работающие — по-старому». На
работающей DPoS-сети этот механизм **невозможен**. Но окно не пустое, и оно ровно то, в
котором узел уже морозит геометрию — см. **R-101** в части 2. R-011 остаётся, но с
другим триггером и другой тяжестью: **SERIOUS → MODERATE**.

### К-2. Точное правило `getDkgQual(e)`

**`dkgQual[e] = (множество адресов committee[e] ≠ множество адресов committee[e−1])`,
позиционно, по адресу валидатора. Ни BLS-ключ, ни стейк в правило не входят.** `[KNOWN]`

`commit_epoch_committee` (`consensus.rs:697-776`) собирает `members`, сортирует их
`sort_unstable_by_key(|m| m.peer_pubkey)` (`:626`), считает `changed =
committee_changed(target, &members)` (`:628`) и записывает `dkg_qual[target] = changed`
(`:661-664`). Само сравнение — `committee_changed` (`consensus.rs:394-421`): сперва длина
из индекса `epoch_index[target−1].length`, затем поэлементно
`incumbent.at(index) != member.validator`.

Эквивалентность узловому правилу доказуема, а не предполагаема. Узел сравнивает
множества peer-ключей (`node:crates/dpos/consensus/src/beacon/actor.rs:1645-1648`
`next == cur` над `Set<PeerPubkey>`, наполняемым в
`node:crates/node/src/dpos.rs:1417-1428`). Соответствие «адрес ↔ peer-ключ» биективно и
вечно: `peer_pubkey_owner[peer]` пишется один раз (`consensus.rs:257-260`) и нигде не
очищается, `consensus_keys[v].peer_pubkey` пишется один раз (`:253-254`) под охраной
`ERR_CONSENSUS_KEYS_ALREADY_SET` (`:222-224`), а `ERR_PEER_PUBKEY_ALREADY_IN_USE`
(`:142-149`) запрещает второго владельца. Обе стороны отсортированы по одному ключу,
дублей нет — значит позиционное равенство адресов ⇔ равенство множеств peer-ключей.

**Сценарий А из R-012 (ротация BLS-ключа при том же peer-ключе) недостижим:** в
контракте нет функции смены consensus-ключей вообще. Единственный писатель —
`store_consensus_keys`, и он вызывается только из `register_validator`
(`staking.rs:1063`) и `initialize` (`initializer.rs:559`); повторный вызов для того же
валидатора отвергается `ERR_CONSENSUS_KEYS_ALREADY_SET`. `setConsensusKeys` в контракте
нет — ни как обработчика, ни как селектора (`lib.rs:43-147`, `consts.rs` целиком).

**Что это меняет.** R-012: механизм недостижим на этом контракте. **SERIOUS → MINOR**
(остаётся как «узел не читает бит для решения о церемонии, а выводит его сам» —
структурная хрупкость без достижимого следствия). Э-20 (`REGISTER.md` часть 5) —
**снять**: сценарий не воспроизводим, ротации ключа не существует.

Побочно закрывается ещё одна ветка R-012: «`epoch_index[target−1].length == 0` ⇒
`changed = true`, а узел церемонию не запустит, потому что `roster(target−1)` вернёт
`None`». Курсор `last_committed_epoch_p1` (`consensus.rs:706-708`, `:665-667`) читается и
инкрементируется на единицу — эпохи коммитятся строго подряд, дыр не бывает, а каждая
закоммиченная эпоха имеет `length ≥ MIN_COMMITTEE_LENGTH = 4` (`:611-617`). Пустой
`target−1` возможен только при `target == 0`, где `committee_changed` коротит в `false`
(`:399-401`).

### К-3. Атомарность `dkgQual` и коммита; может ли бит быть сброшен задним числом?

**Оба значения пишутся в одном системном вызове; переложить комитет или сбросить бит
нельзя ни одной функцией контракта.** `[KNOWN]`

`commit_epoch_committee` в одном вызове пишет `committee_records[record]` (`:640-650`),
`epoch_index[target].{record,length}` (`:651-655`), кольцо весов (`:660`),
`dkg_qual[target]` (`:661-664`) и курсор (`:665-667`). Вызов системный
(`SYSTEM_CALLER`-only, `:594-596`), исполняется в pre-execution, любой revert — ошибка
исполнения блока, то есть откат целиком.

Перезапись исключена курсором: `target` читается из `last_committed_epoch_p1` и в конце
того же вызова становится `target+1`. Полный перечень писателей в consensus-хранилище
(`grep` по всем `*_accessor()` вне `tests.rs`) даёт ровно эти места; ни `dkg_qual`, ни
`epoch_index`, ни `committee_records`, ни `last_committed_epoch_p1` не пишутся больше
нигде — governance-ручки для них нет.

Со стороны узла `(bit, committed)` тоже читаются согласованно: `dkg_qual_probe`
(`node:crates/node/src/dpos.rs:1526-1540`) делает два EVM-вызова, но оба против одного и
того же неизменяемого состояния блока `at`. Это и есть требуемая атомарность.

**Что это меняет.** «Soak v47» — сценарий, где контракт сбросил бит после проведённой
узлом церемонии, — **на этом контракте невозможен**. R-017 стоит на этом допущении и
теряет свой контрактный триггер; остаётся достижимый: узел сам провёл церемонию на
эпоху, чей бит `false` (то есть комитет не менялся, а узел решил иначе) — что после К-2
означает баг узла, а не контракта. R-017: **MODERATE → MINOR**, допущение о контракте из
формулировки снять.

### К-4. Проверяется ли PoP BLS-ключа?

**Да, на единственном пути записи ключей, до сохранения.** `[KNOWN]` (перепроверено по
исходникам; совпадает с выводом `VERIFY-BLOCKERS.md` по артефактам сборки)

`verify_consensus_keys` (`consensus.rs:121-211`) вызывается из `register_validator`
(`staking.rs:1048-1054`) и `initialize` (`initializer.rs:549`) — иных путей нет.
Последовательность: tombstone-проверка (`:129-135`), длины
(`BLS_PUBKEY_UNCOMPRESSED_LENGTH`, `BLS_POP_UNCOMPRESSED_LENGTH`, ненулевой peer-ключ,
`:136-141`), уникальность peer-ключа (`:142-149`), сжатие G2 через внешний верификатор
(`:157-167`), уникальность BLS-ключа по `keccak256(compressed)` (`:168-176`), и затем
собственно PoP:

```
external_call(verifier, SIG_BLS_VERIFY,
    (fluent_namespace(sdk), compressed, BLS_POP_DST, bls_pop_uncompressed, bls_pubkey_uncompressed))
→ !valid ⇒ revert ERR_INVALID_PROOF_OF_POSSESSION      (consensus.rs:177-193)
```

Форматы сходятся с узлом побайтно:
- namespace: `b"FLUENT_DPOS_V1_" ‖ chain_id.to_be_bytes()` (`consensus.rs:106-110`) против
  `node:crates/dpos/bls/src/lib.rs:111-116` — идентично, 23 байта, без суффикса;
- DST: `b"BLS_POP_BLS12381G1_XMD:SHA-256_SSWU_RO_POP_"` (`consensus.rs:27`) против
  `node:crates/dpos/bls/src/pop.rs:5` — идентично;
- подписываемое тело: узел использует commonware
  `ops::sign_proof_of_possession::<MinSig>` над `union_unique(namespace, pubkey.encode())`
  (`node:crates/dpos/bls/src/pop.rs:24-28, 33-34`); верификатор строит
  `_hashToG1(unionUnique(namespace, message), dst)`
  (`/home/djadjka/Work/audit-482/sol/contracts/libraries/BLS12381Verifier.sol:65`).
  Совпадает.

Оговорка о границе доказательства: сам верификатор — отдельный контракт по адресу из
`chain_config.bls_verifier` (`consensus.rs:151-156`), задаваемому при `initialize`
(`types.rs:290`). Его rWasm-исходника в `pr482-study/contracts` нет
(`contracts/bls12381` — это роутер precompile'ов, не `verify(bytes,bytes,bytes,bytes,bytes)`);
я сверялся с солидити-оригиналом из соседнего дерева. `[LIKELY]` в части «развёрнутый
верификатор равен этому солидити-тексту»; закрыть это можно только сверкой байткода по
адресу верификатора в genesis.

**Что это меняет.** R-005 («защита от rogue-key целиком на контракте») — допущение
подтверждено, защита есть и покрывает оба ключа. Пометку «стоит на непроверенном
допущении» снять; тяжесть R-005 определяется тем, что узел не проверяет PoP сам, а это
теперь осознанное делегирование, а не дыра: **BLOCKER → MODERATE** (остаточный риск —
подмена адреса верификатора в genesis; узел его не читает и не проверяет).

### К-5. `MAX_ACTIVE_VALIDATORS_LENGTH == 51` и неизменяем? Может ли `activeValidatorsLength` вырасти после старта узлов?

**51 — константа компиляции, изменяется только пересборкой и передеплоем.
`activeValidatorsLength` — governance-изменяемый в диапазоне `[4, 51]`, в любой момент,
эффективен со следующей эпохи. Выйти за 51 нельзя.** `[KNOWN]`

`MAX_ACTIVE_VALIDATORS_LENGTH: u64 = 51` (`consts.rs:369`) — `const`, не хранилище.
`set_active_validators_length` (`config.rs:488-527`) отвергает `< MIN_COMMITTEE_LENGTH`
(`:499-505`) и `> MAX_ACTIVE_VALIDATORS_LENGTH` (`:506-512`). `validate_initialization`
(`config.rs:157-167`) отвергает `0` и `> 51` при инициализации. Сеттер **не** закрыт
`ensure_dpos_not_active` — то есть менять кап можно и на работающей сети.

Узловой кап — `MAX_COMMITTEE_SIZE: u64 = 51` (`node:crates/dpos/p2p/src/constants.rs:175`),
проверка при старте `node:crates/dpos/consensus/src/dpos.rs:1957-1972`. Значения равны.

**Что это меняет.** R-019 («рост комитета после старта делает сертификаты
недекодируемыми») — контрактный кап совпадает с узловым и жёстче не бывает, комитет из
52 членов невозможен без передеплоя. **MODERATE → MINOR** (остаётся как «проверка
одноразовая и не поймает передеплой контракта с другим капом»); подъём до SERIOUS,
предусмотренный формулировкой R-019, **не применять**.

Побочная деталь, вынесена в **R-103**: `getActiveValidatorsLength()` отдаёт *скаляр*, то
есть последнее запланированное значение (`config.rs:513-518` пишет скаляр немедленно, а
чекпойнт — на `next_epoch`), тогда как отбор комитета читает
`active_validators_length_at(epoch)` (`config.rs:263-278`, через
`staking.rs:385-390`). Для узловой проверки «≤ 51» разницы нет, для документации — есть.

### К-6. Обработчики слэшинга

**Все четыре есть, сигнатуры и селекторы совпадают; кодировка evidence сходится
побайтно; `AlreadySlashedForEquivocation` ведёт себя по-разному на двух путях.** `[KNOWN]`

| Вызов узла | Диспетчер | Обработчик | Кто может звать |
|---|---|---|---|
| `slashEquivocation(uint64,uint32)` `0xdc6fb3f2` | `consts.rs:221`, `lib.rs:143` | `consensus.rs:1053-1074` | только `SYSTEM_CALLER` (`:954-956`) |
| `slashEquivocationNotarize(bytes,bytes,bytes,bytes)` `0xe28d2f63` | `consts.rs:223-225`, `lib.rs:144` | `consensus.rs:1169-1178` | любой |
| `slashEquivocationFinalize(...)` `0xadd07a3e` | `consts.rs:226-228`, `lib.rs:145` | `consensus.rs:1183-1192` | любой |
| `slashEquivocationNullifyFinalize(...)` `0xa10827e9` | `consts.rs:229-231`, `lib.rs:146` | `consensus.rs:1197-1206` | любой |

Селекторы совпадают не по комментариям-хексам, а по строкам сигнатур: контракт считает
их `derive_keccak256_id!("slashEquivocation(uint64,uint32)")` и т. д., узел — из
собственного `sol!` (`node:crates/node/src/evm.rs:636`,
`node:crates/dpos/consensus/tests/slasher_integration.rs:839-862`). Строки идентичны.

Кодировка evidence. Контракт разбирает голое сцепление
`Attestation = uvarint(signerIdx) ‖ sig[48]` (`evidence.rs:283-294`,
`BLS_SIGNATURE_LENGTH = 48`, `consts.rs:508`). Узел в продакшн-пути **перекодирует**
evidence с `Scheme` (97-байтная `CombinedSignature`) на `VoteScheme` (48 байт) перед
отправкой: `vote_attestation` + повторный `ConflictingNotarize::<VoteScheme,_>::new(..).encode()`
(`node:crates/dpos/consensus/src/slasher/evidence.rs:236-246, 322-334`). Голден-корпус
контракта (`evidence.rs:317-330`, `CONFLICTING_NOTARIZE: [u8; 168]`) — 2 × (35 байт
proposal + 1 байт signer + 48 байт подписи) = 168, и он объявлен скопированным из узловой
фикстуры `node:crates/dpos/consensus/tests/equivocation_evidence_conformance.rs`, где та
же проекция делается независимой реализацией (`:187-196`). Сходится.

Namespace для проверки подписей: контракт строит
`b"FLUENT_DPOS_V1_" ‖ chain_id ‖ {"_NOTARIZE"|"_NULLIFY"|"_FINALIZE"}` (`consensus.rs:921-930`);
commonware строит суффиксы теми же литералами (`CW:consensus/src/simplex/scheme/mod.rs:123-125`)
и склеивает через `union`, то есть простой конкатенацией (`CW:utils/src/lib.rs:166-172`).
Сходится.

`AlreadySlashedForEquivocation` — **две разные семантики, и обе узлом обработаны**:
- системный путь: молча `Ok(())` (`consensus.rs:1066-1072`) — «дубликат это гонка, не
  ошибка»; узел мягко глотает revert на этом пути в любом случае
  (`node:crates/node/src/evm.rs:1224-1252`);
- путь с evidence: `revert_with(ERR_ALREADY_SLASHED_FOR_EQUIVOCATION, &validator)`
  (`consensus.rs:1108-1114`) — узел классифицирует его как `SubmitOutcome::AlreadySlashed`.

**Что это меняет.** R-028 стоит на утверждении узла «контрактного обработчика нет» —
см. **R-104**, утверждение ложно. R-028: **MODERATE → MINOR**, переформулировать на
оставшуюся половину (`next_charge` держит одного обвиняемого во всех блоках). R-022 в
части «приём контрактом» — допущение подтверждено, пометку снять. B-3 (`REGISTER.md`
часть 2) как задача «добавить контрактный обработчик» — **отменить**.

### К-7. Стоимость записи в реестр: нужен ли стейк, есть ли лимит?

**Регистрация permissionless и платная, но в `getRegistryWithKeys()` она не попадает:
туда попадает только `activateValidator`, а это governance-only. Явного лимита длины
нет.** `[KNOWN]`

`getRegistryWithKeys` (`consensus.rs:312-322`) обходит
`staking_storage().active_validators_accessor()` — список **активных**, не всех
зарегистрированных (док-комментарий обработчика на `:311` говорит «all registered
validators» и противоречит собственному коду; узел в этом месте прав —
`node:crates/dpos/staking-reader/src/reader.rs:747-750` пишет «the active-validator list,
NOT the stake-weighted top-k committee»).

Кто туда пишет:
- `register_validator` (`staking.rs:1024-1067`): permissionless, требует
  `initial_stake ≥ min_validator_stake_amount` (`:1042-1047`) плюс полную проверку PoP
  (два внешних вызова верификатора). Ставит статус `STATUS_PENDING`, и потому
  `set_validator` **не** пушит в `active_validators` (`staking.rs:155-159`, ветка только
  для `STATUS_ACTIVE`) и `seed_selection_membership(visible = false)` не пушит в
  `selection_roster` (`:174-181`);
- `activate_validator` (`staking.rs:865-914`): `ensure_governance` (`:869`), пушит в
  `active_validators` (`:898-901`) и в роcтер (`:902`);
- `initialize` — генезисные валидаторы, статус `STATUS_ACTIVE`.

Итого длину списка задаёт governance, не атакующий. Лимита на неё нет ни в контракте, ни
в узле на этом пути (`active_registry_peers`, `reader.rs:753-767`, размер не проверяет —
`check_peer_set_size` живёт выше, `reader.rs:333-345`).

**Что это меняет.** Ветка «permissionless-рост реестра» снимается со всех записей,
которые её несут. R-003 (тег «ВЕРА» про стоимость записи), R-013, R-037, R-054 — пометку
о зависимости от К-7 снять; тяжести не менять (их механизмы про буферизацию на пира, а не
про способ попасть в реестр). R-023, R-029 — то же. Остаётся операционное замечание:
`getRegistryWithKeys()` — линейный обход с 5 SLOAD на запись, а `selection_roster` вообще
никогда не сокращается (`ensure_rostered`, `staking.rs:206-216`, только добавляет), и его
обходят `selection_candidates_at` и `count_selection_visible_at` на **каждом** системном
вызове коммита и закрытия эпохи. Это внутреннее свойство контракта; на границе оно
проявляется как отказ fail-loud системного вызова. Одна строка в конце.

### К-8. Семантика `tombstoned`

**Необратим, ставится только по доказанному конфликту или по вердикту комитета,
governance-пути нет.** `[KNOWN]`

Единственная запись — `apply_equivocation_penalty` (`consensus.rs:1019-1022`),
`set_checked(sdk, true)`. Присваивания `false` в коде нет ни одного (полный grep по
`tombstoned_accessor` вне тестов: `consensus.rs:130, 783, 917, 964, 1006`,
`staking.rs:367, 1114, 2014` — все остальные чтения). Функции governance, которая
дотягивалась бы до этого поля, нет.

Два входа в `apply_equivocation_penalty`:
- `slash_equivocation` — системный вызов, доказательства не несёт и не проверяет
  (`consensus.rs:1047-1052` это заявляет прямо), опирается на то, что комитет уже проверил
  обвинение при голосовании за блок;
- `slash_from_evidence` — permissionless, но проверяет обе подписи через pairing и
  выводит личность из `bls_pubkey_owner`, а не из места в комитете
  (`consensus.rs:985-1002, 1033-1059`).

Следствия tombstone: `remove_active`, статус `STATUS_JAIL`, selection-невидимость,
изъятие self-stake (`consensus.rs:1023-1030`), запрет повторной регистрации ключей
(`:129-135`), запрет делегирования (`staking.rs:1110-1117`), запрет release из
production-exclusion (`staking.rs:363-370`).

**Что это меняет.** R-034 («узел рвёт транспорт по флагу без доказательства») —
допущение уточнено: ошибочный флаг может поставить только системный вызов, то есть блок,
за который проголосовал комитет, либо валидное криптографическое доказательство.
Governance флаг поставить не может. **MODERATE → MINOR**, «ВЕРА» снять; уверенность
поднять до `[KNOWN]` на контрактной стороне. R-044 не меняется (там про момент чтения
снапшота, а не про источник флага).

### К-9. Когда `getEpochCommitteeWithStakes(E)` отдаёт пустые `stakes` при непустых адресах?

**Ровно когда кольцо весов провернулось мимо `E`, то есть когда `E` старше примерно 14
эпох относительно последней закоммиченной.** `[KNOWN]`

`get_epoch_committee_with_stakes` (`consensus.rs:866-898`) берёт членство и tombstone из
хранилища, а веса — из `read_weights` (`:496-519`). `read_weights` возвращает `None`,
если штамп нулевой пары кадра не равен `epoch as u32` (`:506-508`); вызывающий делает
`weights.unwrap_or_default()` (`:789-790`), и `stakes` уезжает пустым вектором рядом с
непустыми остальными тремя.

Арифметика кадра: `ring_base(epoch) = (epoch % WEIGHT_RING_EPOCHS) * PAIRS_MAX`
(`:424-426`), `WEIGHT_RING_EPOCHS = 16` (`consts.rs:390`). Кадр эпохи `E` затирается
коммитом эпохи `E+16`, а коммит опережает текущую эпоху не более чем на
`MAX_COMMITTEE_LOOKAHEAD_EPOCHS = 2` (`consts.rs:498`, проверка `consensus.rs:709-715`).
Значит веса `E` живы от коммита до начала реальной эпохи `E+14`.

Узловая сторона декодирует это корректно: пустой `stakes` при непустых `addrs` ⇒
`weights: None` (`node:crates/dpos/staking-reader/src/reader.rs:697-712`), любая другая
длина ⇒ `AbiDecode`. Пустая эпоха (не закоммичена) даёт `addrs = []` и `stakes = []`, и
это попадает в ветку «равные длины» ⇒ `Some(vec![])` — тоже верно, потому что
`read_weights` при `length == 0` возвращает `Some(Vec::new())` (`:500-503`), а не `None`.

**Что это меняет.** У R-035 появляется конкретный и узкий триггер вместо «ВЕРЫ»: движок
не спавнится, если узел входит в эпоху, отстоящую больше чем на ~14 эпох от
EL-finalized, с которого он читает. Обычные пути этого не делают: старт читает эпоху
самого finalized-блока (`node:crates/dpos/consensus/src/dpos.rs:1950-1954, 1988-1989`), а
догоняющий `soft_enter_span` регистрирует verify-only схемы и весов не запрашивает
(`node:crates/dpos/consensus/src/dpos.rs:3222-3241`, там же в комментарии уже признано,
что «content-invariant» для ноги `stakes` неверен). Остаётся сценарий «исполнение
отстало более чем на 14 эпох». R-035: **MODERATE → MINOR**, «ВЕРА» снять, триггер
вписать. R-045 связана, тяжесть не меняется.

### К-10. «Комитет закоммичен» ⇔ ответ непуст? Гарантии по срокам коммита `e+1` и `e+2`?

**Да, эквивалентность точная. Сроки: контракт их не задаёт вообще — их задаёт узел, и
задаёт строже, чем предполагает `epoch_transition.rs`.** `[KNOWN]`

Эквивалентность. `committee_at(epoch)` читает `epoch_index[epoch].{record,length}`
(`consensus.rs:387-392`); незакоммиченная эпоха даёт нули, `length == 0` коротит все
циклы, и обработчик возвращает четыре пустых массива. Обратно: закоммиченная эпоха имеет
`length = members.len() ≥ MIN_COMMITTEE_LENGTH = 4` (`:611-617, 653-655`). Значит
«непусто» ⇔ «закоммичено», без промежуточных состояний.

Сроки. Контракт задаёт только потолок: `target > current + 2` ⇒
`ERR_EPOCH_NOT_YET_COMMITTABLE` (`consensus.rs:709-715`). Пола нет. Пол задаёт узел:
`drive_ahead_commit` (`node:crates/node/src/evm.rs:941-963`) на **каждом** блоке вычитывает
`nextEpochToCommit()` и коммитит, пока `next <= current_epoch + 2`, причём любой сбой
коммита — `BlockExecutionError`, то есть отказ блока. Отсюда: к первому блоку эпохи `e`
комитеты `e+1` и `e+2` уже закоммичены. Это сильно раньше, чем `last(e) − K`, и `e+2` —
не best-effort, а такое же fail-loud.

Читаемость `committee[E]` всю эпоху `E−1` — да: запись однократна (К-3), а значит от
момента коммита и далее ответ неизменен.

**Что это меняет.** R-066 («пустой комитет = ещё не закоммичен, парковка без предела») —
трактовка верна, но предположение о «пропущенном коммите» ложно: пропустить эпоху курсор
не может, только отстать. Оставить **MINOR**, зависимость от К-10 снять, формулировку не
трогать. R-024 (окно DKG) — читаемость комитета следующей эпохи гарантирована с запасом,
эта половина вопроса закрыта; согласованность `DKG_MARGIN_BLOCKS = 20` с интервалом
эпохи контракт не адресует, и это остаётся.

### К-11. Может ли `getEpochCommitteeWithStakes(E)` измениться после первого чтения?

**Три ноги из четырёх — нет, никогда, никаким вызовом. `tombstoned` — да, и это
намеренно. Плюс `stakes` исчезает через ~14 эпох (К-9), что формально тоже изменение.**
`[KNOWN]`

- `addrs`: `committee_records[record]` растится только в `commit_epoch_committee`
  (`consensus.rs:743-749`), `epoch_index[E]` пишется только там же (`:651-655`), и
  однократность гарантирована монотонным курсором. Идентификаторы записей — это номера
  эпох, в которых комитет менялся, так что наложения двух эпох на одну запись не бывает.
- `keys`: читаются живьём (`consensus.rs:883` → `read_consensus_keys`), но неизменяемы
  (К-2): единственный писатель однократен и охраняем.
- `stakes`: кольцо пишется один раз на эпоху (`:660`), затирается только через 16 кадров.
- `tombstoned`: читается живьём (`:781-786`), монотонно растёт, обнуления нет (К-8).

Governance-функции, которая меняла бы состав закоммиченного комитета, в контракте нет.
`setActiveValidatorsLength`, `disableValidator`, `delegate/undelegate` трогают отбор
будущих эпох и живой список активных, но не `committee_records`/`epoch_index`/кольцо.

**Что это меняет.** Заморозка комитета на эпоху — подтверждённое свойство контракта, а
не допущение узла. CB-11, R-075 — зависимость от К-11 снять, тяжесть не менять.
`EpochSchemeProvider::register` может опираться на неизменность безопасно.

### Побочно закрытые К-12 и К-13

**К-12.** Семь селекторов совпадают по строкам сигнатур, а не по хекс-комментариям:
`getEpochCommitteeWithStakes(uint64)` `consts.rs:214-215`, `getRegistryWithKeys()` `:199`,
`getDkgQual(uint64)` `:210`, `getEpochBlockInterval()` `:57`, `getDposActivationBlock()`
`:59`, `getUndelegatePeriod()` `:61`, `getActiveValidatorsLength()` `:51-52` — против
`node:crates/dpos/staking-reader/src/reader.rs:132-151`. Возвращаемые типы: расхождение
ширины на трёх из них, см. **R-108**.

`activationEpoch` — эпоха, с которой consensus-ключи валидатора считаются действующими.
Контракт использует её в отборе (`active_peer_key_at`, `consensus.rs:550-551`: кандидат
без `activation_epoch <= epoch` в комитет не попадает) и в `getValidatorsWithKeysAt`
(`:292-294`, обнуление ключей). Ставится в `next_epoch` при регистрации
(`staking.rs:1058, 1063`) и в `0` для генезисных (`initializer.rs:559`). Узел
её действительно может не использовать: в `getEpochCommitteeWithStakes` фильтр уже
применён на коммите, а ключи там отдаются сырыми — значит каждый член закоммиченного
комитета по построению имеет действующие ключи. R-084 остаётся **NIT**, но теперь с
обоснованием, а не «непонятно зачем».

**К-13.** Соответствие «адрес валидатора ↔ peerPubkey» — биекция, вечная и
единственная (доказательство в К-2). `enqueue_fallback` слэшера может не найти жертву
только потому, что её нет в снапшоте, которым он располагает, но не потому, что
соответствие изменилось.

---

## Часть 2. Расхождения на границе

Нумерация сквозная от R-101.

### R-101. Геометрия эпох: узел морозит её в окне, где контракт ещё разрешает менять

**Тяжесть: MODERATE. Уверенность: `[KNOWN]` обе стороны.**

Механизм по шагам.

1. Governance планирует активацию: `setDposActivationBlock(H)`, `H > block_number`
   (`config.rs:562-586`). Поле становится ненулевым.
2. Узел на первом же finalized-блоке видит `scheduled_dpos_activation → Some(H)`
   (`node:crates/dpos/staking-reader/src/reader.rs:604-620`: контракт с кодом и
   ненулевая активация), проходит гейт в `apply_at`
   (`node:crates/dpos/staking-reader/src/epoch_transition.rs:459-462`) и **морозит обе
   величины**: `freeze_or_warn(&mut self.frozen_interval, …)` (`:463-475`) и
   `freeze_or_warn(&mut self.frozen_activation, …)` (`:477-483`). Это происходит до
   блока `H`.
3. Governance переносит запуск: `setEpochBlockInterval(I')` и/или
   `setDposActivationBlock(H')`. `ensure_dpos_not_active` пропускает, потому что
   `block_number < H` (`config.rs:25-30`).
4. Узел, который уже заморозил `(I, H)`, при следующем чтении печатает `warn!` и
   **продолжает считать по старым значениям** (`epoch_transition.rs:47-56`). Узел,
   стартовавший после шага 3, морозит `(I', H')`. Контракт и pre-execution узла считают
   по живым значениям: `read_epoch_block_interval` вызывается **на каждом блоке** без
   заморозки (`node:crates/node/src/evm.rs:1102`), `current_epoch(sdk)` — тоже
   (`util.rs:67-85`).

Следствие. Ровно тот раскол, который описан в R-011: `OriginEpocher`,
`is_epoch_boundary`, партиции `consensus_epoch_{E}` и сабканалы у двух групп узлов
расходятся, а pre-execution обеих групп при этом согласовано с контрактом. То есть
consensus-плоскость расходится сама с собой и со своим же EL: `leader_index` в
`extra_data` считается против комитета эпохи `E_frozen`, а `recordProduction` кредитует
`produced[E_live][leader_index]` и валидирует индекс против `committee_length_at(E_live)`
(`liveness.rs:64-66, 76-99`).

Почему MODERATE, а не SERIOUS. Окно закрывается навсегда на блоке `H`, триггер — легальная
операция governance до запуска сети, и лечится перезапуском всех узлов после переноса.
Но обнаружить его узел не помогает: единственный сигнал — один `warn!` на узел.

Отношение к реестру: это и есть достижимый остаток R-011. Формулировку R-011 не трогать,
тяжесть **SERIOUS → MODERATE**, зависимость от К-1 снять, добавить ссылку сюда.

### R-102. `epoch_of_block` и `epoch_at_block` расходятся при `activation == 0`, и оба комментария об этом неверны

**Тяжесть: MINOR. Уверенность: `[KNOWN]` обе реализации; `[LIKELY]` недостижимость.**

Узел (`node:crates/dpos/staking-reader/src/reader.rs:293-300`):
`block_number.saturating_sub(activation) / interval`. При `activation == 0` это
`block_number / interval`. Док-комментарий на `:284` утверждает: «`0` ⇒ absolute
numbering».

Контракт (`math.rs:443-451`): при `activation_block == 0` возвращает `Some(0)` для любой
высоты — «unarmed sentinel, not armed at genesis». Док-комментарий на `:436-439`
утверждает: «`ensure_dpos_not_active` keeps the governance setters open on it **and the
node reads it the same way**».

Два комментария описывают одно место и противоречат друг другу; код обеих сторон
противоречит комментарию другой. Тест контракта закрепляет его вариант
(`math.rs:487-492`), узловой doc-тест — свой (`reader.rs:284-286`).

Достижимость. На продакшн-путях `activation == 0` до `epoch_of_block` не доходит: и
`scheduled_dpos_activation` в reader'е (`:618-620`), и её зеркало в исполнителе
(`node:crates/node/src/evm.rs:855-862`) сворачивают `0` в `None` и выключают всю
DPoS-секцию. Но `epoch_of_block` — `pub`, и корректность держится на этом гейте, а не на
самой функции.

Что делать в реестре: новая запись MINOR. К R-011 не относится.

### R-103. `getActiveValidatorsLength()` отдаёт запланированный кап, а не действующий

**Тяжесть: MINOR. Уверенность: `[KNOWN]`.**

`set_active_validators_length` пишет скаляр немедленно и чекпойнт — на `next_epoch`
(`config.rs:513-520`); собственный комментарий на `:515-517` это фиксирует.
`get_active_validators_length` (`:248-256`) читает скаляр. Отбор комитета читает
чекпойнт: `selected_validators_at` → `active_validators_length_at(epoch)`
(`staking.rs:385-390`, `config.rs:263-278`).

Узел читает эту view один раз при старте ради проверки «≤ 51»
(`node:crates/dpos/consensus/src/dpos.rs:1957-1972`), а doc в reader'е
(`node:crates/dpos/staking-reader/src/reader.rs:625-635`) называет её «the cap in force»
и «отражает размер будущих комитетов». В окне между `setActiveValidatorsLength` и началом
следующей эпохи это неверно: ни один комитет ещё не отобран под это значение.

Последствия для проверки нет (обе величины ≤ 51). Последствие есть для любого будущего
потребителя, который решит, что читает действующий кап.

### R-104. Узел утверждает, что контрактного обработчика `slashEquivocation(uint64,uint32)` не существует

**Тяжесть: MINOR (в узле; в реестре снимает MODERATE-запись). Уверенность: `[KNOWN]`.**

`node:crates/node/src/evm.rs:1594-1603`:

> **The contract has no counterpart at all** — `slashEquivocation(uint64, uint32)` is
> dispatched by neither `feat/flu-989-port-solidity-delta` nor
> `origin/feat/flu-989-rust-staking` (verified 2026-08-14: zero hits for the signature in
> `consts.rs` on every branch in this repo that carries the contract).

Ветка в утверждении — та самая, из которой я читаю. Обработчик есть: `consts.rs:221`
(`derive_keccak256_id!("slashEquivocation(uint64,uint32)")`), диспетчер `lib.rs:143`,
реализация `consensus.rs:1053-1074`. Более того, селектор `0xdc6fb3f2` присутствует ровно
один раз в развёрнутом байткоде devnet — скан
`node:devnet/local-dpos-smoke/contracts/fluentbase_contracts_staking.rwasm`, часть 3.

Следствие: обвинение, вложенное в блок, доходит до цепи — `committee_member_at(epoch,
signer_idx)` разрешает позицию в закоммиченном комитете (`consensus.rs:801-822`), дальше
`apply_equivocation_penalty`. Механизм R-028 («мёртвая машинерия, наказание приходит
только через WAL-транзакцию») неверен.

Мелочь на той же границе, проверенная попутно: узел шлёт `current_epoch` блока
(`node:crates/node/src/evm.rs:1211`), а обвинение берётся из эпохи **раунда**
(`node:crates/dpos/consensus/src/application.rs:610-635`), и `verify_block_charge`
отвергает обвинение любой другой эпохи (`:735`). Это согласовано с тем, против чего
контракт разрешает индекс, — расхождения нет.

### R-105. Узел утверждает, что контракт не проверяет межвалидаторную уникальность ключей

**Тяжесть: MINOR. Уверенность: `[KNOWN]`.**

Четыре места в узле ссылаются на `Staking.setConsensusKeys` — функцию, которой в
контракте нет: `node:crates/dpos/consensus/src/scheme.rs:23` и `:28`,
`node:crates/dpos/bls/src/scheme.rs:24`, `node:crates/dpos/consensus/src/engine.rs:162`
(плюс `node:crates/dpos/consensus/tests/equivocation_evidence_conformance.rs:92`). Две из
них утверждают свойство, а не только имя:

> `setConsensusKeys` does NOT enforce cross-validator uniqueness of peerPubkey/blsPubkey

Контракт проверяет оба, и дважды каждое — до внешних вызовов верификатора
(`consensus.rs:142-149` peer, `:169-176` BLS) и повторно после них, против reentrancy
(`:227-234`, `:237-248`). Плюс `ERR_CONSENSUS_KEYS_ALREADY_SET` (`:222-224`) запрещает
перезапись собственных ключей валидатора.

Защитный код в узле (`epoch_committee_from_snapshot` возвращает `Err` вместо паники)
остаётся оправданным — `BiMap` всё равно должен уметь отказать на битых данных, — но
обоснование в комментарии ложно, а комментарий в `engine.rs:160-163` описывает
достижимость («reachable from on-chain data»), которой нет.

### R-106. Узел ссылается на несуществующее расписание коммита и на несуществующий «пропуск эпохи»

**Тяжесть: MINOR. Уверенность: `[KNOWN]`.**

`node:crates/dpos/staking-reader/src/epoch_transition.rs:539-543`:

> `Staking.sol` allows an epoch with no `commitEpochCommittee` (unslashable by design;
> idempotent / monotonic — a skip is safe)

Курсор `last_committed_epoch_p1` читается как `target` (`consensus.rs:706-708`) и в конце
того же вызова становится `target + 1` (`:665-667`). Пропустить эпоху нельзя; можно
только отстать. «Missed-commit epoch» как состояние не существует.

Там же, `:546-559`:

> Under the v41 QUALIFY-BEFORE-COMMIT schedule … EVERY epoch now gets a committee —
> candidate-if-qualified else the incumbent carry — committed at `H_qual = B−8 ≤ B−1−K`

Ни qualify-before-commit, ни `H_qual` в контракте нет. `commit_epoch_committee` не
принимает аргументов, и её собственная документация фиксирует, что схему с передачей
комитета удалили (`consensus.rs:682-686`). Расписание целиком узловое —
`drive_ahead_commit` на каждом блоке (`node:crates/node/src/evm.rs:941-963`).

Поведение узла (парковка + re-poke) для реального состояния верно, и фактическая
гарантия сильнее заявленной (К-10). Менять нечего; но R-066 и рассуждения о границе
DKG-окна опираются на процитированное расписание, и его надо заменить настоящим.

### R-107. Комментарий «KNOWN CONTRACT DRIFT» про четвёртый массив устарел

**Тяжесть: NIT. Уверенность: `[KNOWN]`.**

`node:crates/dpos/staking-reader/src/reader.rs:126-131` утверждает, что
`feat/flu-989-port-solidity-delta` заканчивает обработчик
`write_returns(sdk, &(validators, keys, stakes))` — тремя массивами, и что нога
`tombstoned` контрагента не имеет. Контракт возвращает четыре:
`write_returns(sdk, &(validators, keys, stakes, tombstoned))` (`consensus.rs:897`), вектор
собирается в `:777-786`, и обработчик документирует ногу как намеренную (`:749-762`).
Drift ушёл; предупреждение осталось и теперь дезинформирует
(в частности, «не трогайте ногу в одностороннем порядке» указывает на несуществующий риск).

### R-108. Три view объявлены `uint32` на узле и записаны `u64` контрактом

**Тяжесть: NIT. Уверенность: `[KNOWN]`.**

`getEpochBlockInterval`, `getActiveValidatorsLength`, `getUndelegatePeriod` объявлены
`returns (uint32)` (`node:crates/dpos/staking-reader/src/reader.rs:145-151`, и интервал
ещё раз в `node:crates/node/src/evm.rs:655`). Контракт пишет содержимое полей, которые
хранятся как `u64`: `field.set_checked(sdk, value as u64)` в `config.rs:518`, `:551`,
`:607`, и `write_abi` отдаёт их как одно 32-байтное слово (`config.rs:250-255`, `:322-327`,
`:348-353`).

Ширина совпадает на проводе (одно слово), и значения по построению влезают в `u32` —
сеттеры принимают `U32Command` (`config.rs:493`, `:537`, `:593`), инициализация тоже
(`types.rs:284-286`). То есть проверку ширины делает декодер alloy на узле, а не ABI
контракта: если поле когда-нибудь получит значение > `u32::MAX`, узел упадёт в
`AbiDecode`, а не прочитает мусор. Направление отказа безопасное, но одностороннее.

### R-109. Контракт считает, что узел не декодирует `EpochWeightsUnavailable`

**Тяжесть: NIT. Уверенность: `[KNOWN]`.**

`events.rs:156-158`: «that is a closed signature match, so the event is mute until an arm
is added for it». Ветка есть: `node:crates/node/src/evm.rs:749-761`,
`EpochWeightsUnavailable::decode_log` → `error!` + счётчик
`dpos_epoch_weights_unavailable_total`. Все семь close-событий, объявленных узлом
(`evm.rs:592-655`), имеют точные контрагенты по имени и типам полей в `events.rs:159-248`.

### R-110. Узел считает выравнивание `activation % interval == 0` неподдерживаемым соглашением; контракт его требует

**Тяжесть: MINOR. Уверенность: `[KNOWN]`.**

`node:crates/dpos/staking-reader/src/epoch_transition.rs:493-499`:

> The absolute form `(number + 1) % interval == 0` only agrees when
> `activation % interval == 0` (a devnet bootstrap convention, **NOT enforced** — prod
> cold-start anchors on an arbitrary recent finalized height)

Контракт требует выравнивания во всех трёх местах, где любое из полей может быть
установлено: `validate_initialization` (`config.rs:168-173`), `set_epoch_block_interval`
(`:544-546`), `set_dpos_activation_block` (`:572-574`) — везде
`ERR_UNALIGNED_ACTIVATION_BLOCK`.

Поведение узла (относительная арифметика) корректно при любом раскладе, так что дефекта
нет. Но это инвариант, а не соглашение, и рассуждения реестра о расхождении абсолютной и
относительной границы стоят на обратном.

### Проверенное и НЕ давшее расхождения

Перечислено, потому что «проверил и сошлось» — тоже результат, и потому что каждый пункт
снимает «ВЕРУ» с таблицы `UNDERSTANDING.md` §12.X.

- **Селекторы семи view.** Совпадают по строкам сигнатур (К-12). Revert при опечатке —
  `ERR_UNKNOWN_METHOD` (`lib.rs:148`), как и предполагает узел.
- **Форма возврата `getEpochCommitteeWithStakes`.** Четыре массива, порядок
  `(addrs, keys, stakes, tombstoned)` (`consensus.rs:897`) = порядок узлового `sol!`
  (`reader.rs:133-135`). Длины трёх ног всегда равны `length`; четвёртая — пустая или
  равной длины, ровно как декодирует узел.
- **Порядок членов.** Контракт сортирует `sort_unstable_by_key(|m| m.peer_pubkey)` по
  `B256` (производный `Ord` = байт-лексикографический), `consensus.rs:729`. Узел
  проверяет строго возрастающий порядок по сырым байтам и отвергает дубли
  (`reader.rs:375-413`). Совпадает; дублей быть не может по К-2.
- **Нижняя граница комитета.** `MIN_COMMITTEE_LENGTH = 4` в обоих (`consts.rs:424` /
  `reader.rs:161`), и контракт её действительно навязывает (`consensus.rs:718-720`,
  `config.rs:499-505`).
- **`BALANCE_COMPACT_PRECISION` и диапазон стейка.** `1e10` в обоих (`consts.rs:342` /
  `reader.rs:170`); контракт хранит `uint112` (`math.rs:402`, `write_ring` через
  `compact_balance`), узел отвергает `≥ 2^112` (`reader.rs:180-196`). Аргумент об
  отсутствии переполнения в `WeightedVrf` держится.
- **Формат ключей.** `blsPubkey` — 96 байт сжатого G2, собирается из трёх слов
  (`consensus.rs:91-104`, `BLS_PUBKEY_WORDS = 3`, `consts.rs:502-507`); пустой ⇔ ключи не
  заданы (`consensus.rs:81-83` возвращает `ConsensusKeys::default()`), что и есть
  узловой сентинел `is_unset` (`reader.rs:417-420`).
- **Атомарность в обратную сторону.** `drive_ahead_commit` делает N отдельных системных
  вызовов и коммитит состояние каждого по отдельности
  (`node:crates/node/src/evm.rs:996-1000`), но отказ любого — `BlockExecutionError`, то
  есть отказ блока целиком. Наблюдаемо это одна операция.
- **Обработка неожиданного входа контрактом.** Все revert'ы — четырёхбайтный селектор
  custom error плюс, опционально, ABI-аргументы (`util.rs:16-33`). Узел ни один не
  декодирует: `exec_view` кладёт revert и halt в одну `ReadError::CallReverted(hex)`
  (`reader.rs:462-470`), исполнитель — в `BlockExecutionError` или в `None`
  (`node:crates/node/src/evm.rs:830-833`, `:875-891`). Ни одна ветка при этом не
  теряется, потому что обе стороны обрабатывают revert и halt одинаково. Отдельно:
  view комитета/реестра/`dkgQual` вызывают `ensure_initialized` (`consensus.rs:279, 313,
  680, 768`), а конфигурационные — нет (`config.rs:249, 321, 334, 347`). Асимметрия
  безвредна: на неинициализированном контракте узел получает `interval = 0` и
  `activation = 0` и корректно остаётся инертным.

---

## Часть 3. Версия

**Совпадает, и совпадение проверено тремя независимыми способами.** `[KNOWN]`

1. **Исходники.** `node:devnet/local-dpos-smoke/contracts/STAKING_ARTEFACT.md:100-118`
   записывает SHA-256 каждого файла `contracts/staking/src` на момент сборки. Пересчёт
   (`find src -name '*.rs' | sort | xargs sha256sum` в
   `/home/djadjka/Work/audit-482/pr482-study/contracts/staking`) даёт **все 14 хешей
   побайтно теми же**. HEAD worktree — `29ae97ef`, как и записано; список из десяти
   изменённых файлов (`git status --porcelain contracts/staking`) совпадает с записанным
   ровно.

2. **Блобы.** Записанные хеши (`STAKING_ARTEFACT.md:119-122`) —
   `8f5895a5…` для `.wasm` (414 513 байт) и `f30deb0d…` для `.rwasm` (2 854 198 байт).
   Пересчёт по файлам в `node:devnet/local-dpos-smoke/contracts/` — совпадает оба, и
   размеры совпадают.

3. **Селекторы в развёрнутом байткоде.** Скан `.rwasm` на little-endian слова: все 14
   селекторов, которые вызывает узел, присутствуют ровно по одному разу —
   `recordProduction`, `commitEpochCommittee`, `slashEquivocation`,
   `slashEquivocation{Notarize,Finalize,NullifyFinalize}`, `getEpochCommitteeWithStakes`,
   `getRegistryWithKeys`, `getDkgQual`, `getEpochBlockInterval`,
   `getDposActivationBlock`, `getUndelegatePeriod`, `getActiveValidatorsLength`,
   `nextEpochToCommit`. Оба маркера устаревшей сборки 2026-08-16
   (`commitEpochBeaconKey` `0x6ece9cb1`, `getEpochBeaconKey` `0xc9adaf5c`) отсутствуют.

Адрес расхождения дать не может: контракт своего адреса нигде не зашивает, а узел берёт
его из `staking-reader.json` (`node:crates/dpos/staking-reader/src/reader.rs:257-269`,
парс без default'а). В devnet это `0x0000000000000000000000000000000000520011`
(`node:devnet/local-dpos-smoke/dpos_harness/core/topology.py:270`).

Genesis-конфигурация: `InitializeCommand` (`types.rs:270-295`) — семнадцать полей,
включая `bls_verifier: Address`. Верификатор приходит именно отсюда, `setBlsVerifier`
на пути инициализации не используется. Это единственный параметр границы, который узел не
читает и не проверяет вовсе (см. оговорку в К-4).

Две оговорки, которые честнее назвать:

- Ни один из трёх способов не доказывает, что компилятор произвёл именно этот блоб из
  именно этих исходников — доказывается только то, что и исходники, и блоб те самые,
  которые записаны как пара, и что блоб несёт ожидаемый набор селекторов. Закрыть до
  конца можно только воспроизводимой пересборкой; `STAKING_ARTEFACT.md:123-127`
  утверждает, что детерминизм подтверждался на этой сборке, но это утверждение документа,
  не проверенный мной факт.
- Devnet-блоб собран с фичей `devnet-views` (`Cargo.toml:24-28`,
  `STAKING_ARTEFACT.md:11-16`), продакшн-артефакт будет без неё. На границу это не
  влияет: под фичей только `blocksInEpoch`, `producedAt`, `pendingExclusions`,
  `lastProcessedBlock` (`lib.rs:86-94`), и узел не вызывает ни одну из них — их читает
  только smoke-харнесс.

**Выводы частей 1-2 под сомнение это не ставит.** Исходники, по которым сделаны все
заключения, — те самые, из которых собран развёрнутый в devnet блоб.

---

## Сводная таблица

| Вопрос / расхождение | Вердикт | Затронутые записи | Новая тяжесть |
|---|---|---|---|
| К-1 | После активации оба поля неизменяемы; до активации оба меняются | R-011, B-9 | R-011 SERIOUS → **MODERATE**, триггер заменён на R-101 |
| К-2 | Правило по адресам ≡ по peer-ключам; ротации ключей в контракте нет | R-012, Э-20 | R-012 SERIOUS → **MINOR**; Э-20 **снять** |
| К-3 | Одна транзакция, курсор монотонный, сброс бита невозможен | R-017, R-012 | R-017 MODERATE → **MINOR** |
| К-4 | PoP проверяется, namespace и DST совпадают побайтно | R-005 | R-005 BLOCKER → **MODERATE** |
| К-5 | 51 — константа; `activeValidatorsLength` меняется в [4,51] | R-019 | R-019 MODERATE → **MINOR**, подъём не применять |
| К-6 | Все четыре обработчика есть; evidence сходится; `AlreadySlashed` двусемантичен | R-028, R-022, B-3 | R-028 MODERATE → **MINOR**; B-3 **отменить** |
| К-7 | Регистрация платная и permissionless, но в реестр пускает только governance | R-003, R-013, R-023, R-029, R-037, R-054 | тяжести без изменений, «ВЕРА» снять |
| К-8 | Необратим, governance-пути нет | R-034, R-044 | R-034 MODERATE → **MINOR**; R-044 без изменений |
| К-9 | Пусто ⇔ кольцо провернулось; горизонт ≈ 14 эпох | R-035, R-045 | R-035 MODERATE → **MINOR** с названным триггером |
| К-10 | Непусто ⇔ закоммичено; сроки задаёт узел, и строже | R-066, R-024 | без изменений, зависимость снять |
| К-11 | Три ноги неизменяемы; `tombstoned` живой намеренно | CB-11, R-075 | без изменений, зависимость снять |
| К-12 (побочно) | Селекторы совпадают; `activationEpoch` = эпоха ввода ключей | R-084 | **NIT**, обоснование добавлено |
| К-13 (побочно) | Биекция «адрес ↔ peerPubkey», вечная | slasher `enqueue_fallback` | без изменений |
| **R-101** | Заморозка геометрии в окне, где контракт ещё разрешает менять | новая; несёт достижимый остаток R-011 | **MODERATE** |
| **R-102** | `activation == 0`: узел считает `bn/interval`, контракт — `0` | новая | **MINOR** |
| **R-103** | `getActiveValidatorsLength()` — запланированный кап, не действующий | новая | **MINOR** |
| **R-104** | «Обработчика `slashEquivocation` нет» — неверно | новая; отменяет механизм R-028 | **MINOR** |
| **R-105** | «Контракт не проверяет уникальность ключей» — неверно | новая | **MINOR** |
| **R-106** | «Пропуск эпохи» и «H_qual = B−8» — несуществующая семантика | новая | **MINOR** |
| **R-107** | «KNOWN CONTRACT DRIFT» про четвёртый массив — устарел | новая | **NIT** |
| **R-108** | `uint32` на узле против `u64` в контракте, три view | новая | **NIT** |
| **R-109** | Контракт считает `EpochWeightsUnavailable` немым у узла | новая | **NIT** |
| **R-110** | Выравнивание активации — инвариант контракта, а не соглашение | новая | **MINOR** |
| Часть 3 | Исходники, блобы и селекторы совпадают с devnet | — | — |

---

## Одна строка о внутреннем устройстве контракта

`selection_roster` только растёт (`staking.rs:206-216`, удаления нет ни одного), а
`selection_candidates_at` и `count_selection_visible_at` обходят его целиком
(`staking.rs:284-298`, `:306-320`) на каждом системном вызове коммита и закрытия эпохи —
то есть стоимость двух per-block системных вызовов растёт линейно по числу валидаторов,
когда-либо активированных за всю историю цепи, и упирается это в fail-loud
`BlockExecutionError`, а не в revert транзакции.
