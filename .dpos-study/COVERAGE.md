# Покрытие аудита crates/dpos

Сводка к `AUDIT.md`. Все числа посчитаны скриптом по текущим файлам (2026-09-03); строки — `wc -l`.

## 1. Карта покрытия

Ссылки собраны регулярным выражением по токенам `*.rs` в AUDIT.md (все части) и UNDERSTANDING.md. Неполные имена (`actor.rs`, `keys.rs`, `mod.rs`, `oracle.rs`, `error.rs`, `ingress.rs`, `lib.rs`) разрешены по контексту строки (упоминание slasher/charge/VoteStore ⇒ slasher; bls/SeedOracle ⇒ bls; иначе beacon/p2p/staking-reader). «Прод. строк» — строки до первого `mod tests`/`#[cfg(test)] mod` в файле. «Находок» — пункты части A, у которых первая ссылка `file:line` указывает на этот файл (одна находка — один файл).

| Файл | Строк | Прод. строк | Упомин. AUDIT | Упомин. UNDERSTANDING | Находок (A) |
|---|---:|---:|---:|---:|---:|
| `bls/src/beacon.rs` | 264 | 168 | 0 | 3 | 0 |
| `bls/src/combined_scheme.rs` | 1093 | 463 | 3 | 8 | 2 |
| `bls/src/encoding.rs` | 124 | 68 | 0 | 1 | 0 |
| `bls/src/error.rs` | 57 | 57 | 0 | 3 | 0 |
| `bls/src/keys.rs` | 316 | 159 | 1 | 6 | 1 |
| `bls/src/keystore.rs` | 327 | 246 | 1 | 2 | 0 |
| `bls/src/lib.rs` | 135 | 117 | 0 | 0 | 0 |
| `bls/src/oracle.rs` | 64 | 64 | 0 | 3 | 0 |
| `bls/src/pop.rs` | 117 | 49 | 1 | 1 | 0 |
| `bls/src/scheme.rs` | 153 | 93 | 1 | 0 | 1 |
| `bls/src/secret_store.rs` | 295 | 205 | 0 | 3 | 0 |
| `bls/src/share_seal.rs` | 49 | 49 | 0 | 1 | 0 |
| `bls/tests/common/mod.rs` | 56 | 56 | 0 | 0 | 0 |
| `bls/tests/conformance.rs` | 59 | 59 | 0 | 1 | 0 |
| `bls/tests/ed25519_ordering_conformance.rs` | 155 | 155 | 0 | 1 | 0 |
| `bls/tests/eip2335_conformance_vectors.rs` | 36 | 36 | 0 | 1 | 0 |
| `bls/tests/eip2537_conformance_vectors.rs` | 245 | 245 | 0 | 1 | 0 |
| `bls/tests/eip2537_roundtrip.rs` | 53 | 53 | 0 | 1 | 0 |
| `bls/tests/hash_to_g1_conformance.rs` | 344 | 344 | 0 | 1 | 0 |
| `consensus/src/application.rs` | 2648 | 1206 | 8 | 19 | 3 |
| `consensus/src/beacon/actor.rs` | 8076 | 2449 | 11 | 38 | 6 |
| `consensus/src/beacon/artifact.rs` | 1644 | 1012 | 2 | 12 | 0 |
| `consensus/src/beacon/carry.rs` | 474 | 243 | 2 | 6 | 1 |
| `consensus/src/beacon/ceremony.rs` | 2088 | 1058 | 3 | 17 | 1 |
| `consensus/src/beacon/certify.rs` | 901 | 451 | 4 | 5 | 0 |
| `consensus/src/beacon/confirmations.rs` | 224 | 224 | 0 | 4 | 0 |
| `consensus/src/beacon/dkg_agree.rs` | 3180 | 1649 | 0 | 13 | 0 |
| `consensus/src/beacon/dkg_engine.rs` | 1587 | 734 | 2 | 8 | 1 |
| `consensus/src/beacon/dkg_msg.rs` | 297 | 156 | 0 | 5 | 0 |
| `consensus/src/beacon/dkg_oracle.rs` | 194 | 105 | 0 | 1 | 0 |
| `consensus/src/beacon/dkg_transport.rs` | 260 | 130 | 0 | 6 | 0 |
| `consensus/src/beacon/follower.rs` | 1092 | 520 | 4 | 6 | 0 |
| `consensus/src/beacon/key_journal.rs` | 417 | 301 | 2 | 4 | 1 |
| `consensus/src/beacon/keys.rs` | 1031 | 640 | 3 | 8 | 1 |
| `consensus/src/beacon/log_resolver.rs` | 578 | 429 | 0 | 4 | 0 |
| `consensus/src/beacon/log_store.rs` | 262 | 202 | 1 | 3 | 0 |
| `consensus/src/beacon/metrics.rs` | 306 | 306 | 0 | 3 | 0 |
| `consensus/src/beacon/mod.rs` | 123 | 123 | 3 | 3 | 1 |
| `consensus/src/beacon/oracle.rs` | 841 | 348 | 1 | 2 | 0 |
| `consensus/src/beacon/outcome.rs` | 300 | 113 | 0 | 3 | 0 |
| `consensus/src/beacon/plane.rs` | 831 | 831 | 3 | 11 | 1 |
| `consensus/src/beacon/resolve.rs` | 394 | 135 | 2 | 4 | 0 |
| `consensus/src/beacon/seed.rs` | 159 | 120 | 0 | 5 | 0 |
| `consensus/src/beacon/seed_journal.rs` | 998 | 457 | 2 | 5 | 0 |
| `consensus/src/beacon/share_state.rs` | 1304 | 723 | 4 | 9 | 2 |
| `consensus/src/beacon/surface.rs` | 2195 | 833 | 3 | 8 | 0 |
| `consensus/src/beacon/verified_seed.rs` | 143 | 143 | 0 | 3 | 0 |
| `consensus/src/beacon/wire.rs` | 102 | 77 | 0 | 3 | 0 |
| `consensus/src/byzantine.rs` | 339 | 237 | 0 | 3 | 0 |
| `consensus/src/cert_follow.rs` | 212 | 212 | 0 | 3 | 0 |
| `consensus/src/cert_inlet.rs` | 3301 | 960 | 10 | 16 | 2 |
| `consensus/src/cold_start_jump.rs` | 1997 | 957 | 4 | 8 | 1 |
| `consensus/src/digest.rs` | 95 | 81 | 0 | 3 | 0 |
| `consensus/src/dpos.rs` | 5117 | 3954 | 18 | 20 | 5 |
| `consensus/src/engine.rs` | 327 | 327 | 3 | 6 | 2 |
| `consensus/src/epoch_manager.rs` | 3185 | 1825 | 7 | 14 | 2 |
| `consensus/src/epocher.rs` | 170 | 80 | 0 | 4 | 0 |
| `consensus/src/executed.rs` | 169 | 64 | 0 | 3 | 0 |
| `consensus/src/executor.rs` | 12187 | 3833 | 12 | 17 | 5 |
| `consensus/src/extra_data.rs` | 233 | 137 | 0 | 5 | 0 |
| `consensus/src/fault.rs` | 326 | 259 | 0 | 4 | 0 |
| `consensus/src/feed_sink.rs` | 50 | 50 | 1 | 4 | 0 |
| `consensus/src/lib.rs` | 102 | 102 | 0 | 6 | 0 |
| `consensus/src/order_block.rs` | 912 | 464 | 2 | 9 | 0 |
| `consensus/src/outer.rs` | 2017 | 1745 | 6 | 18 | 1 |
| `consensus/src/plane_upstream.rs` | 404 | 343 | 2 | 6 | 0 |
| `consensus/src/scheme.rs` | 78 | 78 | 0 | 1 | 0 |
| `consensus/src/slasher/actor.rs` | 1525 | 1295 | 8 | 23 | 4 |
| `consensus/src/slasher/evidence.rs` | 1116 | 641 | 1 | 6 | 0 |
| `consensus/src/slasher/gossip.rs` | 365 | 214 | 1 | 8 | 1 |
| `consensus/src/slasher/ingress.rs` | 137 | 137 | 3 | 4 | 1 |
| `consensus/src/slasher/mod.rs` | 17 | 17 | 0 | 2 | 0 |
| `consensus/src/slasher/tombstone.rs` | 140 | 59 | 1 | 7 | 0 |
| `consensus/src/spec_exec.rs` | 129 | 129 | 5 | 5 | 1 |
| `consensus/src/sync_metrics.rs` | 859 | 606 | 2 | 5 | 1 |
| `consensus/src/timeouts.rs` | 148 | 107 | 0 | 4 | 0 |
| `consensus/src/weighted_vrf.rs` | 739 | 282 | 2 | 6 | 1 |
| `consensus/tests/cold_restart_init_arithmetic.rs` | 98 | 98 | 0 | 3 | 0 |
| `consensus/tests/equivocation_evidence_conformance.rs` | 856 | 856 | 0 | 3 | 0 |
| `consensus/tests/slasher_integration.rs` | 1072 | 1072 | 0 | 2 | 0 |
| `p2p/src/bootstrappers.rs` | 496 | 255 | 0 | 3 | 0 |
| `p2p/src/config.rs` | 109 | 109 | 2 | 3 | 1 |
| `p2p/src/constants.rs` | 214 | 214 | 1 | 3 | 1 |
| `p2p/src/ingress.rs` | 138 | 57 | 0 | 4 | 0 |
| `p2p/src/lib.rs` | 502 | 382 | 1 | 4 | 1 |
| `p2p/tests/convergence.rs` | 227 | 227 | 0 | 4 | 0 |
| `staking-reader/src/epoch_transition.rs` | 2447 | 743 | 6 | 20 | 5 |
| `staking-reader/src/error.rs` | 129 | 129 | 2 | 2 | 0 |
| `staking-reader/src/lib.rs` | 45 | 45 | 0 | 0 | 0 |
| `staking-reader/src/reader.rs` | 1496 | 850 | 7 | 28 | 3 |

Итого: 90 файлов, 80816 строк (42636 прод.). Файлов без единого упоминания в AUDIT.md: 43 (12500 строк, 9112 прод.).

### По каталогам

| Каталог | Файлов | Строк | Прод. строк | Находок (A) | Находок / 1000 строк | Находок / 1000 прод. строк | CRITICAL/HIGH |
|---|---:|---:|---:|---:|---:|---:|---:|
| bls | 19 | 3942 | 2686 | 4 | 1.01 | 1.49 | 2 |
| p2p | 6 | 1686 | 1244 | 3 | 1.78 | 2.41 | 0 |
| staking-reader | 4 | 4117 | 1767 | 8 | 1.94 | 4.53 | 1 |
| consensus/src (верхний уровень) | 24 | 35744 | 18038 | 24 | 0.67 | 1.33 | 5 |
| consensus/src/beacon | 28 | 30001 | 14512 | 15 | 0.50 | 1.03 | 1 |
| consensus/src/slasher | 6 | 3300 | 2363 | 6 | 1.82 | 2.54 | 0 |
| consensus/tests | 3 | 2026 | 2026 | 0 | 0.00 | 0.00 | 0 |

A-61 (контрагент в `crates/node/src/evm.rs`) в каталоги `crates/dpos` не входит и в подсчёт не включён.

## 2. Файлы без находок

43 файла без единого упоминания в AUDIT.md. Ниже они разложены по трём заданным причинам плюс одна группа, которую в три причины уложить нельзя без искажения: файл прочитан целиком в первом проходе, но находок не дал и тривиальным не является. Она выделена отдельно.

### 2.1 Не рассматривались предметно (не читались ни целиком, ни частями)

| Файл | Строк | Что там |
|---|---:|---|
| `consensus/src/byzantine.rs` | 339 | `VoteEquivocator` под `cfg(any(test, feature="dpos-devnet-byzantine"))` (`lib.rs:34-35`). В production-сборку не входит — это единственное основание не читать; сам код не открывался. |
| `consensus/src/beacon/metrics.rs` | 306 | `BeaconMetrics` — 21 counter + `register`. Не открывался; риск дублирующей регистрации имён при двух `register` в одном процессе (см. §5, п. 9) не проверен. |
| `consensus/src/beacon/dkg_oracle.rs` | 194 | `run_local_dkg`, только `cfg(test)` (`beacon/mod.rs:60-61`). Не открывался. |
| `bls/tests/*.rs` (7 файлов) | 948 | Conformance-тесты (ed25519 ordering, EIP-2335/2537, hash-to-G1). Не открывались; их наличие известно только из `UNDERSTANDING.md` §10 и `ls`. |
| `consensus/tests/*.rs` (3 файла) | 2026 | `cold_restart_init_arithmetic`, `equivocation_evidence_conformance`, `slasher_integration`. Не открывались; список тестов взят из UNDERSTANDING (реле). |
| `p2p/tests/convergence.rs` | 227 | 1 живой тест + 3 `#[ignore]` заглушки. Не открывался. |

Сюда же по смыслу относятся все `mod tests` внутри production-файлов (≈38 000 строк, разница между «Строк» и «Прод. строк» в таблице §1): при чтении они вырезались фильтром и ни один тест не был прочитан.

### 2.2 Тривиальны (прочитаны; ломаться нечему)

| Файл | Что лежит | Почему нечему ломаться |
|---|---|---|
| `bls/src/error.rs` | `enum Error`, 13 вариантов | Данные без логики. |
| `bls/src/lib.rs` | псевдонимы типов, константы длин, `fluent_namespace` | Единственная функция — конкатенация `b"FLUENT_DPOS_V1_" ‖ chain_id` (`:111-116`); проверена тестом на длину и различимость chain_id. Смена литерала = хардфорк, это задокументировано в самом файле. |
| `bls/src/share_seal.rs` | newtype `ShareSealKey(Zeroizing<[u8;32]>)` + константа HKDF-info | Нет ветвлений; вывод ключа проверен в `keys.rs:55-72` (прочитан). |
| `bls/src/oracle.rs` | trait `SeedOracle`, enum `SeedCheck` | Только сигнатуры; семантика `NoKey` проверена у потребителя (`combined_scheme.rs:428-444`, A-3). |
| `consensus/src/digest.rs` | newtype `Digest(B256)` с codec | 32 байта без длины/тегов. |
| `consensus/src/lib.rs` | реэкспорты, `SCHEME_RETENTION_EPOCHS`, буферы | Значение 8 обсуждается в A-10/B-9 через потребителей. |
| `consensus/src/slasher/mod.rs`, `staking-reader/src/lib.rs` | реэкспорты | — |
| `consensus/src/beacon/verified_seed.rs` | newtype с одним `check` (`:43-52`) и `from_journal` без проверки (`:75-77`) | `from_journal` — `pub(crate)`, единственный вызов в `certify.rs:163` для rehydrate; доверие к диску там же обсуждено (A-20). |
| `consensus/src/beacon/wire.rs` | один тег `Dkg`, cap 64 КиБ (`:51-66`) | Длина проверяется до копирования; неизвестный тег — ошибка. |
| `consensus/src/beacon/seed.rs` | codec `Seed`, `prev_randao_from_seed = keccak(σ)`, два fallback-хэша | Прямолинейные хэши; `constant_fallback_seed` предсказуем по построению — учтено в A-34. |
| `consensus/src/epocher.rs` | `OriginEpocher` с `checked_*` арифметикой (`:53-70`) | Переполнение ⇒ `None`; сверено с `FixedEpocher` тестом; согласованность с `epoch_of_block` проверена в UNDERSTANDING 12.2. |
| `consensus/src/executed.rs` | трёхзначное чтение хэша по высоте (`:45-63`) | Единственная развилка `height > best ⇒ None`; потребители проверены (`epoch_transition.rs:390`). |
| `consensus/src/fault.rs` | enums `FaultClass/DeferReason`, `From<eyre::Report> ⇒ Corruption` | Сам `From` — источник поведения «любая немаппленная ошибка = shutdown», это учтено в A-11/A-53 у потребителей. |
| `consensus/src/timeouts.rs` | константы + `validated()` | Инварианты проверяются при старте (`outer.rs:806-808`). |
| `consensus/src/scheme.rs` | два конструктора над `EpochCommittee::from_pairs` | Дубликаты ключей ⇒ `Err`; дальше проверено в `reader.rs:375-413`. |
| `consensus/src/beacon/dkg_transport.rs` | `dkg_subchannel = BASE | epoch` с проверкой диапазона (`:66-71`), сборка buffered engine с `deque_size = 51` | Единственная логика — сдвиг id; коллизия с эпохами исключена диапазоном `< 2^32`. |

### 2.3 Покрыты через другой файл

| Файл | Через что | Почему достаточно |
|---|---|---|
| `bls/src/beacon.rs` | `combined_scheme.rs` (A-3, A-41), `beacon/oracle.rs`, CW `threshold.rs:28-39` | Файл — тонкие обёртки над `threshold::sign/verify/recover`; единственная собственная проверка `threshold == sharing.required::<N3f1>()` (`:147-156`) прочитана вместе с потребителем `oracle.rs:206-247`; поведение при избытке partial'ов проверено в CW (`prepare_evaluations` усекает до `t`). |
| `bls/src/encoding.rs` | `slasher/evidence.rs:285-472` (потребитель) | Перекладка байтов blst → EIP-2537 без ветвлений, кроме отказа на infinity; корректность раскладки утверждают conformance-тесты, которые я не читал — поэтому «покрыт» здесь означает «потребитель проверен, сама раскладка принята по тестам, не по чтению». |
| `bls/src/secret_store.rs` | A-20, A-23, A-24 (обсуждение fsync/атомарности), `share_state.rs:307-319` | Прочитан; `write_mode_0600` (staging + rename + fsync каталога, `:110-134`) и `append_mode_0600` (fsync на запись, `:174-192`) — эталонные; находок нет, слабое место — `reject_insecure_mode` не на Unix — no-op (`:79-82`). |
| `consensus/src/extra_data.rs` | `application.rs:227-235` (`production_record_ok`) | Единственный дефект файла (диапазон `leader_index` не проверяется при декоде, `:123`) обезврежен сравнением с `expected` у потребителя; вне карты комитета запись не проверяется вовсе — это A-2/B-10, не отдельная находка. |
| `consensus/src/cert_follow.rs` | `cold_start_jump.rs` (A-1), `dpos.rs:2562-2606, 3336-3378` | `fetch_verified_boundary` (`:166-212`) проверяет высоту, структуру и BLS при локальном `at_hash` — здесь порядок правильный, поэтому файл не разделяет дефект A-1. Trait `CertUpstream` — сигнатуры. |
| `consensus/src/beacon/outcome.rs` | `beacon/actor.rs:2210-2213` (A-27), `dkg_agree.rs:601-606` | `parse_outcome` с cap 51 и `validate_share_on_poly` прочитаны; единственный вопрос к ним — где они НЕ вызываются (live-путь `adopt_share`), и это A-27. |
| `consensus/src/beacon/dkg_msg.rs` | `beacon/actor.rs:1832-1934` (A-28), UNDERSTANDING 11.16/12.8 | Codec с `Cfg = committee size`; неподписанный `ceremony_epoch` разобран у потребителя. |
| `consensus/src/beacon/log_resolver.rs` | `beacon/actor.rs:2271-2431`, `artifact.rs:764-844`, `plane.rs:124-243` | Адаптер ключей/сообщений между resolver p2p и актором; собственной логики — `retain` не трогает Artifact (`:270-278`), `deliver` возвращает `false` при обрыве канала (`:396-400`). Оба поведения учтены при разборе `fetch_missing_logs`. |
| `consensus/src/beacon/confirmations.rs` | `dkg_agree.rs` (`entry_bar`, `covers`, `ConfirmPool::record`), `beacon/actor.rs:1218, 1913, 2300` | Прочитан целиком (`mint`, `:153-203`). Логика «шириной больше — заменить» и Decisive-триггер проверены против `ConfirmPool::record` (`dkg_agree.rs:379-407`). Находок нет; не тривиален — мог бы стоять в §2.4. |
| `p2p/src/ingress.rs` | тесты парсера (не читал) + `bootstrappers.rs:81-83, 249-250` | Один парсер `host:port` с отказом на all-numeric host (`:44-52`); вход — только конфиг оператора, не сеть. |
| `p2p/src/bootstrappers.rs` | `p2p/src/config.rs` (A-50), сама DNS-политика — в части A не отмечена | Прочитан до `:270`: retry-окно 120 с с пустым результатом без ошибки (`:157-163`) — осознанное решение, LOW; вход — DNS/файл оператора. |

### 2.4 Прочитаны целиком, не тривиальны, находок не дали

| Файл | Что проверял | Что могло остаться |
|---|---|---|
| `consensus/src/beacon/dkg_agree.rs` (1649 прод. строк) | codec `ShareConfirm`/`DkgProposal` (границы `MAX_SET_LEN`, монотонность индексов, `:481-512, 583-636`), `entry_bar`/`margin` (`:270-303`), `rejects_structurally` (`:1194-1280`), `decide` (`:1295-1363`), `Relay` (`:1502-1543`), `DkgReporter` once-only (`:1634-1647`), `BuiltProposal::arm` (`:731-753`). | Не прослежено против контракта commonware `Automaton`: что делает simplex, когда `verify` паркует навсегда (`Decision::Park` ⇒ `tx.closed().await`) — таймаут вида или зависание инстанса; поведение при `certify` всегда `true`. См. §5 п. 2. |

## 3. Провалы по каталогам

### p2p (1244 прод. строк, 3 находки: A-44, A-45, A-50)

Что смотрел: `lib.rs:42-96` (загрузка ключа: проверка mode 0o077, zeroize, `from_hex`), `:199-288` (9 × `register` с квотами), `:301-381` (`OracleHandle` как `PeerSetSink`/`Blocker`/`Provider`, `NoopBlocker`); `config.rs:56-108` (`into_commonware_config`: cooldown/gossip по chain_id, `allow_private_ips`, `max_peer_set_size`); `constants.rs` целиком (ids, квоты, backlogs, `MAX_MESSAGE_SIZE`, `DKG_SUBCHANNEL_BASE`, `epoch_from_subchannel`); `ingress.rs:25-56`; `bootstrappers.rs:35-254`.

По каким признакам искал: неограниченные буферы, доверие к сети без аутентификации, отсутствие бана, хардкод сетевых идентификаторов, парсинг внешнего ввода, монотонность `track`.

Что крейт делает и чего в нём быть не может: это конфигурационный адаптер над `commonware_p2p::authenticated::discovery` — он не держит консенсусного состояния, не хранит ничего на диске, не принимает сетевых сообщений сам (все каналы отдаются потребителям как `(Sender, Receiver)`), не делает криптографии, кроме декодирования ed25519-ключа. Классы проблем, которых здесь нет по построению: гонки состояния, порча журнала, ошибки протокола консенсуса. Что может быть и что проверено: неверные квоты/лимиты (A-45), политика блокировки (A-44), расхождение сетевых констант с chainspec (A-50), парсинг операторского ввода (ingress/bootstrappers — прочитаны, входы доверенные).

Почему находок мало: код тонкий и доверенный ввод; но глубина проверки ограничена тем, что реальный риск живёт в commonware, который я читал точечно: `tracker/directory.rs:220-233` (монотонность `track`), `:444-446, 521-530` (eligibility), `spawner/actor.rs:138`. Не читал: обработку backlog/квот на приёме (что происходит с кадром при переполнении `VOTE_BACKLOG = 256` — дроп или backpressure на соединение), стоимость gossip bit-vec при `max_peer_set_size = 4096`, поведение `allow_dns` при ресолвинге. Итог: проверка p2p достаточна для его собственных ~1200 строк и поверхностна для свойств, которые он делегирует commonware.

### staking-reader (1767 прод. строк, 8 находок: A-11, A-15, A-16, A-17, A-49, A-58, A-59, A-60)

Что смотрел: `reader.rs` целиком в production-части (`:36-89` классификация ошибок по типам и подстрокам, `:106-152` ABI, `:187-196` `compact_stake`, `:293-329` арифметика эпох, `:337-413` проверки размера и порядка, `:420-442` декод ключей, `:455-484` `exec_view`/`decode_view`, `:523-561` `with_evm`, `:568-765` геттеры и снапшот, `:774-819` trait); `epoch_transition.rs` в production-части целиком (`:44-65` заморозка, `:294-326` read-height и anchor, `:344-438` `on_finalized` с парковкой, `:443-594` `apply_at`, `:608-675` `track_and_trigger`, `:698-727` `soft_enter_span`, `:734-741` `cold_start`); `error.rs` целиком.

По каким признакам искал: деление на ноль и переполнение, инварианты порядка/размера комитета, чтение по «latest» вместо хэша, кэширование governance-параметров, string-matching ошибок, потеря состояния при повторе (Full/Closed), одиночные слоты, монотонность anchor.

Что крейт делает и чего в нём быть не может: чистые чтения view-функций контракта при заданном хэше плюс детектор границ, живущий в памяти одного узла. Нет сети, нет диска, нет ключевого материала. Классы проблем, исключённые по построению: сетевой DoS, порча журналов, криптографические ошибки (кроме subgroup-check, делегированного `fluentbase-bls`). Что возможно: расхождение арифметики эпох между узлами (проверено — единая функция), расхождение снапшотов между узлами по высоте чтения (A-9 у потребителя), неверная классификация ошибок reth (A-49), потеря границы (A-15), заморозка параметров (A-11).

Почему находок «мало» относительно ядра — не потому, что проверка была поверхностной: по плотности (4.53 на 1000 прод. строк) это самый насыщенный каталог. Что не проверено: `transact_system_call` от `Address::ZERO` и его отличия от `eth_call` (семантика reth/revm — не читал `crates/node/src/evm.rs` `transact_system_call`, только обёртку здесь); ~30 тестов `epoch_transition` (не читал); поведение `soft_enter_span` при реорге executed-хэша между вызовами.

## 4. Покрытие по направлениям

| Направление | Файлы, в которых искал (прочитанные строки) | Оценка ширины |
|---|---|---|
| 1. Безопасность консенсуса | `application.rs` (verify/propose), `executor.rs` (try_derive, guard #2, gap-walk, reseed), `cold_start_jump.rs`, `order_block.rs` (result_matches), `bls/combined_scheme.rs`, `spec_exec.rs`, `outer.rs` (EpochSchemeProvider), `plane_upstream.rs`, `cert_inlet.rs`, `dpos.rs` (прыжок/re-jump), `crates/node/src/ordering.rs:37-57`; вне крейта: reth `tree/mod.rs:1117-1348, 1545-1580, 2694-2780, 3088-3185`, CW `voter/{actor,state,round}.rs`, `batcher/{actor,round}.rs`. | Широкое. |
| 2. Границы эпох | `epoch_transition.rs`, `epoch_manager.rs`, `engine.rs`, `dpos.rs:2019-2304` (bridge, re-poke), `reader.rs:293-329`, `epocher.rs`, `beacon/actor.rs:1582-1722` (maybe_start), `carry.rs`, `crates/node/src/dpos.rs:1408-1430`. | Широкое. |
| 3. Восстановление после падения | `key_journal.rs`, `seed_journal.rs`, `certify.rs`, `share_state.rs`, `artifact.rs` (journal + restart_replay), `plane.rs:459-608`, `dpos.rs:272-913` (recover_*), `slasher/actor.rs:1131-1231` (WAL), `sync_metrics.rs:469-580`, `bls/secret_store.rs`. Не читал: commonware `Ordinal`/`Metadata`/`queue::shared` (семантика `sync`, поведение при частичной записи) и архивы marshal. | Среднее: узловая сторона прочитана целиком, слой хранения commonware — по именам методов. |
| 4. Beacon и DKG | `beacon/actor.rs`, `ceremony.rs`, `dkg_agree.rs`, `dkg_engine.rs`, `artifact.rs`, `keys.rs`, `oracle.rs`, `resolve.rs`, `carry.rs`, `surface.rs`, `confirmations.rs`, `outcome.rs`, `weighted_vrf.rs`, `bls/beacon.rs`; CW `dkg.rs:1810-1850`, `threshold.rs:28-39`. Остальная семантика `Player/Dealer/Logs::select` — по UNDERSTANDING 12.1 (реле). | Широкое по файлам крейта; commonware DKG — узко. |
| 5. Слэшер | `slasher/actor.rs`, `evidence.rs`, `gossip.rs`, `ingress.rs`, `tombstone.rs`, `application.rs:630-749`, `crates/node/src/evm.rs:1185-1262, 1590-1605`; CW `batcher/round.rs:115-135`. | Полное для подсистемы. |
| 6. Криптография BLS | `bls/*` целиком; CW `threshold.rs`. Не читал: `ops::sign_message`/`union_unique`/DST-константы commonware, blst-вызовы. | Закрыто одним крейтом — для этого направления нормально: весь собственный код здесь; корень доверия (PoP) лежит в контракте (A-40). |
| 7. Сеть и ресурсы | `p2p/*`, `wire.rs`, `dkg_msg.rs`, `order_block.rs:331-435`, `plane_upstream.rs`, `gossip.rs`, `beacon/actor.rs:1832-1934, 2315-2324`, `log_store.rs`, `dkg_transport.rs`; CW `tracker/directory.rs`, `spawner/actor.rs`. Не читал: `buffered::Engine` (память тел), `resolver::p2p::Engine` (квоты запросов, retain), `Muxer` (backlog). | Среднее: входы крейта проверены, поведение commonware под нагрузкой — нет. |
| 8. Живучесть | `executor.rs` (`select!`, ретраи), `outer.rs:1377-1743`, `epoch_manager.rs:638-874`, `dpos.rs` (циклы без give-up), `cert_inlet.rs:606-958`, `cold_start_jump.rs:230-605` (watchdog), `beacon/actor.rs:791-854, 1145-1272`, `dkg_engine.rs:263-438`, `follower.rs:284-337`, `slasher/actor.rs:749-808`. Не читал: marshal core (CW) — как он ведёт себя при удержанных ack и `set_floor`. | Широкое по крейту; marshal — по документу. |
| 9. Доверие к контракту | `reader.rs`, `epoch_transition.rs`, `carry.rs`, `beacon/actor.rs:1616-1653`, `slasher/actor.rs:169-176, 822-849`, `crates/node/src/dpos.rs:1408-1430`, `crates/node/src/evm.rs:629-672, 1191-1256`. | Закрыто двумя файлами крейта плюс два в `crates/node` — для этого направления нормально: все обращения к контракту идут через `reader.rs`; ненормально то, что сам контракт недоступен и все выводы условны (часть D, список ВЕРА). |

Направления, закрытые одним-двумя файлами: 6 (нормально — код криптографии сосредоточен в `bls`), 9 (нормально по структуре, ненормально по проверяемости — контракт вне репозитория), 5 (нормально — подсистема самодостаточна). Направления 3 и 7 закрыты широко по файлам крейта, но узко по зависимостям: ни одна из durable-структур commonware и ни один из сетевых движков commonware не читались.

## 5. Где проверка была слабее всего

1. **Все тестовые модули и `tests/` (≈41 000 строк).** Не читал ни одного теста; утверждения о покрытии (§10 UNDERSTANDING, часть D AUDIT) — реле. Пропущенный класс: тесты, закрепляющие неверное поведение (пример найден косвенно — `FakeChain.land_on_import` канонизирует при импорте, чего reth не делает, `executor.rs:4079, 4192`), и тесты с заглушками вместо утверждений (`p2p/tests/convergence.rs`: 3 `#[ignore]`).

2. **`beacon/dkg_agree.rs` × контракт commonware `Automaton`/`Relay`.** Тела `decide`/`propose` прочитаны, но не прослежено, что делает simplex, когда `verify` возвращает `Park` и oneshot закрывается по таймауту вида (`drive`, `:959-969`), и как `certify == true` без проверки (`:1481-1489`) взаимодействует с `Certification`-артефактом. Пропущенный класс: зависание инстанса агрегации на view с недоступным телом; повторная агрегация после `dkg_agree_body_lost` не запускается (`dkg_engine.rs:580-582`) — записано как A-33, но без прослеживания.

3. **Marshal core (commonware) и его вызовы из `executor.rs`/`cert_inlet.rs`.** `hint_finalized`, `set_floor`, `verified`, `report(Finalization)` без проверки, `MAX_PENDING_ACKS` — всё по `COMMONWARE_INTERNALS.md` и UNDERSTANDING 12.2/12.8, не по коду. Пропущенный класс: поведение при `set_floor` ниже уже доставленных высот; отравление архива непроверенным сертификатом через `report()` (следствие A-3) — глубина не проверена.

4. **`executor.rs` — комбинации гейтов `select!`.** Семь условий (`deferred`, `awaiting_seed`, `pending_backfill`, `jump_done`, `finalized_heights_to_backfill`, `safety_halt`, heartbeat) читались по одному; таблица состояний не строилась. Пропущенный класс: взаимная блокировка — например, `awaiting_seed` держит финализации, а `seed_notify` разбужен только при `deferred.is_none()` (`:1464-1466`); `reseed_forward` во время `deferred` сбрасывает `deferred` с ack (`:2422-2425`) — корректность этого ack не проверена.

5. **`dpos.rs` — матрица cold-start.** `resolve_cold_start_kind` × `has_upstream` × пустой архив × crash-recovery (`:1119-1169, 1664-1906`) читались линейно; ветки `DeferToElSync` с `read_with_visibility_belt` и последующее `empty_archive_requires_landed_jump` не проверены на каждую комбинацию. Пропущенный класс: старт с якорем ниже `dpos_activation_block` или с `archive_finalized` внутри эпохи 0.

6. **`beacon/actor.rs` — `fetch_missing_logs` и `resolver.retain`.** Семантика `retain`/`fetch_targeted` в `commonware_resolver::p2p` не читалась; `wanted ∪ unreadable` (`:2057-2064`) принято по имени. Пропущенный класс: голодание запросов при смене `wanted` каждый тик, повторные запросы своего же лога (UNDERSTANDING 11.24).

7. **`weighted_vrf.rs` — какой сертификат подаёт simplex в `elect(round, certificate)`.** Не прослежено в CW, чей `certificate` приходит для view v (нотаризация или нуллификация v−1, или `None`). σ одинаков для обеих (по построению), но путь с `None` (fallback по `view`) и путь с σ дают разные расписания; расхождение между узлами, один из которых держит сертификат, а другой — нет, не исключено чтением.

8. **`cert_inlet.rs` `UpstreamResolver` × `marshal::resolver::handler::Handler::deliver`.** Возврат `deliver == true` (`:3151`) трактован как «marshal принял»; семантика handler'а (что он делает с ключом, которого не запрашивал) не читалась. Пропущенный класс: захват σ для сертификата, который marshal отверг или проигнорировал как stale.

9. **Регистрация метрик при двух плоскостях в одном процессе.** `SyncMetrics::register`, `BeaconMetrics::register`, `EpochEngineMetrics::register`, `ExecutorMetrics::register` вызываются и в `launch`, и в `launch_follower`, и в `beacon::build`/`for_follower` (`dpos.rs:1598-1599, 2614-2617, 2958-2959, 3184-3187`; `plane.rs:488-489`; `follower.rs:124-125`). Что делает `commonware_runtime::Metrics::register` при повторном имени (паника / тихая замена) — не проверял; `metrics.rs` не читал. Пропущенный класс: паника при старте в unified-режиме или потеря серии.

10. **`share_state.rs` / `secret_store.rs` — обработка ошибок ввода-вывода у вызывающих.** Сами функции прочитаны; у вызывающих (`beacon/actor.rs:762-779, 1006-1016`) ошибка записи превращается в `warn` + «в памяти авторитетно». Не прослежено, какие именно `io::Error` возможны на `append_mode_0600` при `ENOSPC`/`EROFS` и как тогда ведёт себя `nondurable_logs`/ack (`:1878-1892`) на протяжении эпохи. Пропущенный класс: узел на переполненном диске продолжает участвовать в DKG, не имея ни одной записи журнала, и после рестарта садится на `Torn`/`NoFile`.

Кроме десяти: `p2p/bootstrappers.rs` DNS-путь (не запускался), `follower.rs` (прочитан, но `run_fetcher` при `Weak` upgrade и `WANT_MAILBOX = 16` дропах не прослежен до последствий), `bls/keystore.rs` (`derive_kdf_key` без верхней границы `c` для PBKDF2 — записано как NIT, глубже не смотрел).
