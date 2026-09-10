# E5-BEACON-DESIGN — повторный разбор модуля beacon и проектирование этапа Э5

Дата: 2026-09-10. Ветка: `djadjka/dpos-reth-2.2-squashed` @ 42280d93. Сессия: одна голова, без субагентов.
Commonware чекаут: `~/.cargo/git/checkouts/monorepo-*/3c4e02ce` (tag v2026.4.0, sha 3c4e02ceede03126f524216605a1195e1cee7d0e по `Cargo.lock:3204`).

Документ собран после двух фаз чтения; заметки по файлам сохранены в §2.9 как свидетельства к §2–§4.

## §0. Прямые ответы на закрывающие вопросы

1. Вне `beacon/` seed/σ/PK_E/share хранят или кэшируют (продакшн) 8 мест (§2.5 без строки 4, которая хранит только номер раунда, плюс `engine.rs:225` из §2.6 п.5): 3 транзиентных копии σ в executor (`executor.rs:134`; `:199,936`; `:564`), 1 второй долговременный носитель σ вне модуля — архив финализаций marshal, который crash-replay читает напрямую (`dpos.rs:657-706`), 3 держателя оракула эпохи внутри схем (`outer.rs:258`, `cert_inlet.rs:403`, `engine.rs:225`), 1 производная σ в конфиге движка (`engine.rs:80`). Внутренних типов beacon, видимых в продакшн-коде снаружи: 2 типа по внутренним путям (`VerifiedSeed`, `InvalidSeed` — `spec_exec.rs:18`, `cert_inlet.rs:18`) плюс 4 `pub(crate)`-шва (`CommitteeSource`, `frozen_dkg_qual`, `agreement_partition`, `absent_unregistered`) и дескриптор `Arc<dyn SeedOracle>`, который выходит через `oracle_for` и живёт в схемах. После Э5 из этого списка ОСТАЮТСЯ три транзиентные копии σ в executor (до 6.1) и `Arc<dyn SeedOracle>` в схемах (по замыслу, §8); свойство 2 после Э5 — «один владелец, три транзиентных читателя по значению», не «ноль копий».
2. Находок 46 (§4; E5-46 добавлена по контр-рецензии), каждая в одной основной категории: «дефект при ≤ f» — 12; «граница протокола при > f» — 2; «нарушение границы модуля» — 7; «оверинженеринг/костыль» — 12; «непредсказуемое состояние» — 13. Правило «не покрыта»: в тексте находки ни один из П-2/П-3/П-9 не назван закрывающим (в том числе «частично»); таких 21 — E5-01, E5-02, E5-03, E5-04, E5-05, E5-06, E5-08, E5-17, E5-20, E5-21, E5-22, E5-23, E5-24, E5-30, E5-32, E5-33, E5-34, E5-40, E5-44, E5-45, E5-46.
3. Три самых опасных места: (а) идентичность dealer-лога по ключу дилера, не по хэшу — честный член теряет share на эпоху и все carry-forward при одном византийском дилере, `ceremony.rs:406-422` (R-002); (б) для не-члена живой эпохи нет ни одного вызывающего кода, который запросил бы артефакт `PK_E` — нулевое пересечение комитетов останавливает цепь без единой ошибки, `actor.rs:2084-2163` + `epoch_manager.rs:1677` (R-121/R-122); (в) пять хранилищ пишут warn-and-continue при ошибке диска, память авторитетна — рестарт садится в состояние, которое никто не определял, `actor.rs:1006-1016`, `key_journal.rs:280`, `share_state.rs:307-319`.
4. Изменено: П-2 — не «удалить SeedStore, σ из marshal», а «один владелец σ внутри beacon, marshal-сертификат — вход через `observe_certificate`, не второй стор» (Д-9 решается так: ни П-2, ни B-1 дословно); П-3 — сохранено и расширено путём получения артефакта для не-членов живой эпохи (R-121/R-122), правилом persist «share ⇒ отказ, артефакт ⇒ RAM + повтор», обязательным `validate_share_on_poly` и НАЗВАННЫМ разменом: удаление W1 и копии артефакта в share-файле означает, что после сбоя sync артефакта ключ приходит только от пиров и исполнение до этого паркуется (§5.4); П-9 — сохранено, дополнено явными терминальными состояниями (включая `Unfrozen`), четырьмя входами автомата и выносом подметания партиций из epoch_manager (band-sweep по номеру эпохи переносится как есть, SafetyHalt — защёлка). Снято: `hint_finalized` как штатный выход из удержания (П-2), `Randomness` в форме 15 методов (трейт остаётся — шов стенда — но из 9 методов + `subscribe` + `faults`). Оценка: было 3–4 + 3 + 2–3 = 8–10 нед; стало 5.0 (1–1,5: добавлен шов стенда) + 5.1 (2–3) + 5.2 (1,5–2, плюс 1–2 дн чужой работы 4.0(в)) + 5.3 (3) + 5.4 (1: band-sweep, join, защёлка) ≈ 8,5–10,5 нед [ГИПОТЕЗА]; против первой версии проекта (8–9,5) рост на ~1 нед целиком из 5.0 и 5.4.
5. По частям: 5.0 → 5.1 → 5.2 → 5.3 → 5.4, каждая оставляет компилируемое дерево; «зелёный стенд без правки тестов» — НЕ на каждой границе: после 5.0 стенд зелёный только после переписывания трёх тестовых реализаций трейта и обёртки роли (`stand.rs:1782-1788`, `byzantine_roles.rs:351-370`); после 5.1 C7 обязан УПАСТЬ и быть переписан (его фальсификатор — «любой узел перешёл высоту парковки», `testbed/tests.rs:1948-1950`); после 5.2 вторая половина `a_forged_seed_slot_…` меняет исход, и её потребитель (`DataFault` → ротация) в стенде появляется только с 4.0(в). Остановка после 5.1 — остаются два пути σ (как сегодня); после 5.2 — остаётся R-002 (как сегодня); опасная половина одна: 5.1, сделанная с сохранённым W1 рядом с артефактным ключом, возвращает класс «локальный ключ против сетевого» — W1 удаляется тем же коммитом, что переключает владельца ключа.
6. Прошлый аудит `history/AUDIT-BEACON.md` в механизмах не ошибся, но был поверхностным в трёх местах и неточен в одном: (а) он не смотрел границу модуля вовсе — ни проверку σ в `spec_exec`/`cert_inlet`, ни владение agreement-хэндлами и партициями у `epoch_manager`, ни crash-replay в обход `VerifiedSeed`, ни четыре производителя схем с оракулом; (б) не рассматривал рестарт как свойство — warn-and-continue в пяти сторах прошёл только как A-8, и неоднородность replay (фатал у одних журналов, skip у других) не отмечена; (в) не нашёл отсутствия вызывающего кода для ключа не-члена (R-121/R-122 нашёл стенд позже); (г) тяжесть A-1 «остановка цепи от одного участника» смешана с границей протокола: при пороге `quorum(n)` одно удержание partial-а останавливает seed и без эквивокации логов — это принятая граница; реальная потеря A-1 при ≤ f — честный член навсегда без share (§3 I3), что тяжесть меняет с «BLOCKER-по-живучести» на «дефект инварианта модуля».
7. На [ГИПОТЕЗА] в §5 стоят: (а) одного фидера часов (ordering tip marshal) достаточно вместо трёх — по МЕХАНИЗМУ теперь [KNOWN] (inlet отдаёт финализацию marshal-у, marshal шлёт Tip выше tip, Tip приходит в тот же клок: `cert_inlet.rs:921-931`, CW `marshal/core/actor.rs:567-595,1453-1457`, `application.rs:1027-1034`; начальное значение — стартовый Tip из архива, `:397-401`); [ГИПОТЕЗА] остаётся по ВРЕМЕНИ для догоняющего валидатора (tee фирит до `store_finalization`, Tip — после) — эксперимент: стендовый `CertInlet` (PLAN 4.0(в)) + валидатор с отстающим EL, сравнить момент `seal` с tee и без; стенд без inlet-а этот вопрос НЕ проверяет (там фидер и так один); (б) карантин как состояние автомата вместо задачи-промоутера не теряет окно `NoKey` — эксперимент: первая половина `a_forged_seed_slot_is_admitted_with_no_key_and_refused_when_the_key_lands` остаётся зелёной, вторая переписывается под `DataFault` и требует 4.0(в); (в) подметание `dkg_epoch_*` внутри beacon (band-sweep перенесён как есть, с join) не оставляет партиций — эксперимент: реплей-тест стенда с оставленной партицией «прошлого процесса» + перечисление партиций storage после прогона; (г) `Sharing` из `artifact.group_key` достаточно для `verify_partial` без копии `Output` в `CeremonyStore` — проверить типом (`DkgOutcome = Output` с полем `public: Sharing`, `outcome.rs:29-30`, `CW dkg.rs:438-454`) и стендом B1–B5; (д) все оценки трудоёмкости; (е) `Torn` до seal ⇒ Dealing с reconstruct безопасен (ни один лог не ушёл, дилер детерминирован — тот же довод, что у `Present` до дедлайна, `actor.rs:1655-1660`), но порванный журнал мог содержать уже отправленные Ack-и как игрок — эксперимент: стенд с порванным первым рекордом до дедлайна, проверить отсутствие второго лога того же дилера у пиров и совпадение share; (ж) входное ограничение «seed-журнал follower-а тоже пишется» противоречит сегодняшнему RAM-only follower-у (`follower.rs:127-129,168-171`) — не эксперимент, а вопрос владельцу; до ответа `Follower.partition_prefix = None`.
8. Строки длиннее 500 символов читались целиком инструментом `Read` (он выдаёт строку без обрезки) и `sed -n 'N,Mp'`; поиск таких строк — `awk 'length > 500 {print FILENAME": "FNR": "length}' <file>`. В прочитанных документах их 80 (REGISTER 46, DECISIONS 19, PLAN 14 до правки блока Э5 и 17 после, 08 — 1); ни `cut -c`, ни `head -c` не применялись.

## §1. Покрытие

| файл | строк | прочитан | инструмент |
|---|---|---|---|
| beacon/mod.rs | 123 | целиком | `cat -n` |
| beacon/wire.rs | 102 | целиком | `cat -n` |
| beacon/seed.rs | 159 | целиком | `cat -n` |
| beacon/verified_seed.rs | 143 | целиком | `cat -n` |
| beacon/confirmations.rs | 224 | целиком | `cat -n` |
| beacon/dkg_transport.rs | 260 | целиком | `cat -n` |
| beacon/log_store.rs | 262 | целиком | `cat -n` |
| beacon/dkg_msg.rs | 297 | целиком | `cat -n` |
| beacon/outcome.rs | 300 | целиком | `cat -n` |
| beacon/metrics.rs | 306 | целиком | `cat -n` |
| beacon/resolve.rs | 394 | целиком | `cat -n` |
| beacon/key_journal.rs | 417 | целиком | `cat -n` |
| beacon/carry.rs | 474 | целиком | `cat -n` |
| beacon/dkg_oracle.rs | 194 | целиком (cfg(test)) | `cat -n` |
| beacon/oracle.rs | 841 | целиком | Read |
| beacon/plane.rs | 878 | целиком | Read |
| beacon/certify.rs | 901 | целиком | Read |
| beacon/seed_journal.rs | 998 | целиком | Read |
| beacon/keys.rs | 1031 | целиком | Read |
| beacon/follower.rs | 1092 | целиком | Read |
| beacon/share_state.rs | 1304 | целиком | Read |
| beacon/artifact.rs | 1644 | целиком (2 страницы) | Read |
| beacon/dkg_engine.rs | 1612 | целиком (2 страницы) | Read |
| beacon/ceremony.rs | 2088 | целиком (2 страницы) | Read |
| beacon/surface.rs | 2202 | целиком (2 страницы) | Read |
| beacon/dkg_agree.rs | 3180 | целиком (3 страницы) | Read |
| beacon/log_resolver.rs | 578 | целиком | `cat -n` |
| beacon/actor.rs | 8076 | целиком (продакшн 1-2448; тесты 2450-8076, 8 страниц) | Read |
| bls/src/beacon.rs | 264 | целиком | `cat -n` |
| bls/src/oracle.rs | 64 | целиком | `cat -n` |
| bls/src/share_seal.rs | 49 | целиком | `cat -n` |
| bls/src/scheme.rs | 153 | целиком | `cat -n` |
| bls/src/lib.rs | 137 | целиком | `cat -n` |
| bls/src/combined_scheme.rs | 1093 | целиком | Read |
| bls/src/keys.rs | 316 | целиком | `cat -n` |
| bls/src/keystore.rs | 327 | целиком | `cat -n` |
| bls/src/secret_store.rs | 295 | целиком | `cat -n` |
| CW cryptography/src/bls12381/dkg.rs | 3661 | 1-2985 целиком (продакшн 1-1926, test_plan 1928-2984); тесты 2985+ не читал | Read |
| CW consensus/src/simplex/scheme/mod.rs | 153 | целиком | `cat -n` |
| CW consensus/src/simplex/scheme/bls12381_threshold/vrf.rs | 2041 | 1-872 (продакшн); тесты 873+ не читал | Read |
| CW broadcast/src/buffered/engine.rs | 441 | целиком | `cat -n` |
| CW broadcast/src/buffered/mod.rs | 1423 | 1-37 (заголовок; остальное тесты) | `sed -n` |
| consensus/spec_exec.rs | 129 | целиком | `cat -n` |
| consensus/scheme.rs | 78 | целиком | `cat -n` |
| consensus/engine.rs | 357 | целиком | `cat -n` |
| consensus/cert_follow.rs | 212 | целиком | `cat -n` |
| consensus/byzantine.rs | 339 | целиком | `cat -n` |
| consensus/fault.rs | 326 | 1-259 (продакшн) | `sed -n` |
| consensus/plane_upstream.rs | 420 | 1-359 (продакшн) | `sed -n` |
| consensus/order_block.rs | 902 | 1-457 (продакшн) | `sed -n` |
| consensus/sync_metrics.rs | 859 | 1-606 (продакшн) | `sed -n` |
| consensus/slasher/evidence.rs | 1116 | 1-641 (продакшн) | `sed -n` |
| consensus/application.rs | 2640 | 1-1201 (продакшн; 1202+ тесты) | `Read` |
| consensus/epoch_manager.rs | 3204 | 1-1844 (продакшн; 1845+ тесты) | `Read` (2 стр.) |
| consensus/cert_inlet.rs | 3303 | 1-963, 2891-3303 (продакшн; 964-2890 тесты) | `Read` |
| consensus/outer.rs | 2022 | 1-1750 (продакшн; 1751+ тесты) | `Read` (2 стр.) |
| consensus/cold_start_jump.rs | 2008 | 1-969 (продакшн; 970+ тесты) | `Read` |
| consensus/dpos.rs | 5121 | 1-3961 (продакшн; 3962+ тесты) | `Read` (4 стр.) |
| consensus/lib.rs | 104 | целиком | `Read` |
| consensus/executor.rs | 12210 | 1-3855 (продакшн; 3856+ тесты) | `Read` (4 стр.) |
| node/src/consensus.rs | 127 | целиком | `Read` |
| node/src/importer.rs | 134 | целиком | `Read` |
| node/src/cert_inlet.rs | 137 | целиком | `Read` |
| node/src/cert_follow/mod.rs | 271 | целиком | `Read` |
| node/src/consensus_rpc/state.rs | 254 | 1-212 (продакшн) | `Read` |
| node/src/dpos.rs | 2866 | 1-2509 (продакшн; 2510+ тесты) | `Read` (3 стр.) |
| node/src/derive.rs | 1475 | 1-644 (продакшн; 645+ тесты) | `Read` |
| node/src/evm.rs | 1975 | только grep (beacon/randao/seed/dkg): совпадения в комментарии `:943-945` (dkgQual) и тесте `:1818`; продакшн-тело не читал | `grep -n` |
| bins/fluent/src/main.rs | — | только grep: `:121-122` комментарий «beacon MANDATORY always-on, no flag»; тест `beacon_arggroup_tests :426-460` | `grep -rn` |
| consensus/testbed/mod.rs | 87 | целиком | `Read` |
| consensus/testbed/fakes.rs | 1603 | целиком | `Read` (2 стр.) |
| consensus/testbed/stand.rs | 2130 | целиком | `Read` (2 стр.) |
| consensus/testbed/byzantine_roles.rs | 936 | целиком | `Read` |

## §2. Модель модуля as-is (только из кода)

### 2.1 Акторы и задачи (что запускает `beacon::build`, `plane.rs:429-850`)
- [KNOWN] `DkgActor` (`actor.rs:791-854`, `select!` над heights / p2p receiver / resolver_rx / pinned_rx / artifacts_rx) — церемонии, агрирование-анонсы, recompute-heal, serve логов.
- [KNOWN] Resolver-движок `commonware_resolver::p2p` на `BEACON_RESOLVER_CHANNEL` (`plane.rs:631-643`, `log_resolver.rs:290-341`) — fetch/serve dealer-логов и артефактов.
- [KNOWN] Agreement launcher (`dkg_engine.rs:557-640`) и по одному supervisor+`simplex::Engine` на target-эпоху (`dkg_engine.rs:267-442`); хэндлы уезжают в `epoch_manager.dkg_agreements` через `agreement_intake` (`epoch_manager.rs:433,828-865`).
- [KNOWN] Write-back артефакта (`plane.rs:256-287`) — `set_pk(Agreed)` + `adopt_tx` в актор; паркуется навсегда.
- [KNOWN] Три writer-задачи журналов: key (`key_journal.rs:262-300`), seed (`seed_journal.rs:410-456`), artifact (`artifact.rs:565-617`) — drain-хэндлы.
- [KNOWN] Seed promoter (`plane.rs:815-835`) — на `key_edge` прогоняет карантин через `promote_epoch`.
- [KNOWN] Итого узел получает 8 хэндлов и один приёмник `agreement_intake` (`plane.rs:384-420`) и супервизирует их по-разному: 5 supervised (`node/dpos.rs:830-840`), 3 drain (`:856-873`), 1 intake (`:785`).

### 2.2 Состояние на диске
| Носитель | Что | Кто пишет | Кто читает | Когда чистится | При ошибке записи |
|---|---|---|---|---|---|
| `beacon-share-e<E>.bin` (`share_state.rs:85-103`, v2 = output ‖ share ‖ artifact?) | share + Output + копия артефакта | `adopt_share` (`actor.rs:999-1025`), recompute (`actor.rs:2178-2266`) | `load_all` при старте (`plane.rs:479`) | `reconcile_journals`: старше активного floor `max{e ≤ now}` (`share_state.rs:668-684`) | warn, share принят в RAM (`actor.rs:1006-1016`) |
| `beacon-dkgjournal-e<E>.bin` (`share_state.rs:368,524-526`, свой append-лог) | ReceivedDealing / OwnSeal / PeerLog / OwnDealerAck | `on_message`, `seal_dealings`, `ingest_log` | `maybe_start` (resume), `log_store` (serve), recompute | `epoch + 1 < now` (`share_state.rs:668-684`, `JOURNAL_RETENTION_EPOCHS = 1`) | best-effort; ack удерживается при не-durable (`actor.rs:1832-1934`), лог остаётся в памяти |
| `beacon-key-ordinal` (`key_journal.rs:146-148`, `Ordinal`) | только `KeySource::Agreed` (`:170-181`) | `set_pk` при любом исходе `insert` (`keys.rs:280-298`) | replay при старте (`:230-258`) | никогда (`:38-54`) | warn, ключ остаётся RAM-only (`:280`); sync-ошибка — счётчик + warn (`:291-297`) |
| `beacon-seed-ordinal` (`seed_journal.rs:153-158`, `Ordinal`, индекс `epoch<<32\|view`) | σ проверенных раундов + terminal пины | writer с батчем ≤ 256 (`:410-456`) | replay при старте без перепроверки (`:351-383`, `VerifiedSeed::from_journal`) | prune по эпохе при `rolled` (`:446-451`) | init/replay — фатально (`?`), запись/sync/prune — warn (`:430,438-451`) |
| `beacon-artifact-metadata` (`artifact.rs:565-617`, `Metadata<U64, Vec<u8>>`) | `AgreedArtifact` по эпохе минта | `ArtifactStore::insert` first-wins (`:452-474`) | `restart_replay` (`:537-549`), `resolve_artifact` (`dkg_engine.rs:469-487`) | никогда (`:406-416`) | warn (`:668-677`) |
| `dkg_epoch_{E}` (commonware journal, `dkg_engine.rs:256-258`) | voter-журнал второй simplex-плоскости | commonware voter | commonware voter | `epoch_manager::prune_agreements` (`epoch_manager.rs:318-366`) — ВНЕ модуля | commonware |

Три формата хранения (Ordinal, Metadata, рукописный append-лог) и два владельца очистки (beacon и epoch_manager).

### 2.3 Состояние в памяти
- [KNOWN] `CeremonyStore = Arc<RwLock<BTreeMap<u64,(CeremonyOutput, Share)>>>` (`actor.rs:169`) — mint → (Output, share); prune по `ceremony_retain_floor` (`actor.rs:224-227`).
- [KNOWN] `BeaconKeys` (`keys.rs:153-175`) — epoch → (pk, `LocalDkg < Carried < Agreed`); `Agreed` не чистится; W1/W3 пишут `LocalDkg` (`surface.rs:1743,1790-1808`), лестница `get_pk` пишет `Agreed`/`Carried` (`keys.rs:603-623`).
- [KNOWN] `SeedStore` (`certify.rs:71-112`) — четыре карты: seeds (≤ 4096), quarantined, waiters (мёртвые), terminal.
- [KNOWN] `ArtifactStore` RAM (`artifact.rs:418-506`) — first-wins, без ретенции.
- [KNOWN] `DkgActor` — 30+ полей (`actor.rs:335-521`): `ceremonies`, `pending`, `nondurable_logs`, `recompute_pending`, `terminal_recompute`, `agreed_pinned`, `agreement_announced`, три warn-latch, `log_store`, `confirmations`, `recorded_dkg_logs` и семь `Option`-швов.
- [KNOWN] `DealerLogStore` кэш serve (`log_store.rs:74-89`), `ConfirmPool` (`dkg_agree.rs:321-436`), `frozen_dkg_qual` кэш битов без границы (`carry.rs:223-242`).

### 2.4 Входы и выходы
Входы (`BeaconConfig`, `plane.rs:294-364`): ключи (ed25519, BLS, seal key), `share_dir`, p2p-половины BEACON и BEACON_RESOLVER, четыре mux-брокера, четыре staking-замыкания (`committee_for`, `committee_pair_for`, `committee_source`, `dkg_qual_at`/`dkg_qual_probe`), канал высот `heights` (три фидера снаружи: поллер `node/dpos.rs:1645,1675`, inlet tee `cert_inlet.rs:899`, `FluentApp::report` `application.rs:1027-1034`), `plane_clock`, `geometry` future, `partition_prefix`.
Выходы: трейт `Randomness` — 15 методов (`surface.rs:113-232`): `record_seed`, `quarantine_seed`, `on_invalid_seed`, `seed_for`, `terminal_seed_at`, `seed_edge`, `mandatory_at`, `share_probe`, `signer_scheme`, `participation_edge`, `oracle_for`, `ensure_key`, `key_edge`, `observe_epoch`, `observe_cert`; плюс `ArtifactSource` (RPC `consensus_getEpochArtifact`, `node/consensus_rpc/state.rs:204-210`), `agreement_intake`, 9 хэндлов.
Кто что зовёт (продакшн): spec_exec — `oracle_for`, `record_seed`, `quarantine_seed`; cert_inlet — `ensure_key(Local)`, `oracle_for`, `record_seed`, `quarantine_seed`, `on_invalid_seed`, `observe_cert`, `mandatory_at`; epoch_manager — `participation_edge`, `key_edge`, `observe_epoch`, `share_probe`, `signer_scheme`, `mandatory_at`, `terminal_seed_at`, `oracle_for`, `ensure_key(Local|Thorough)`; executor — `mandatory_at`, `seed_for`, `seed_edge`; outer (span) — `oracle_for`; dpos crash-replay — `mandatory_at`, `seed_for`.

### 2.5 Где seed/σ/PK_E/share/ключ эпохи хранятся ВНЕ `beacon/` (св. 2)
| # | Место | Что | prod/test |
|---|---|---|---|
| 1 | `executor.rs:564` `Deferred.seed: Option<Seed>` | σ припаркованного блока | prod |
| 2 | `executor.rs:199,936` `ParkedSpec.seed`, `parked_spec` | σ припаркованных нотаризаций | prod |
| 3 | `executor.rs:134` `Notarized.seed` (mailbox) | σ в сообщении spec_exec → executor (`spec_exec.rs:117-125`) | prod |
| 4 | `executor.rs:160` `SpecExecuted.seed_round` | только раунд | prod |
| 5 | marshal `finalizations_by_height` (архив, `outer.rs:474-513`) + чтение σ из него в `dpos.rs:657-668` и upstream `:669-704` | σ внутри каждого `Finalization` | prod |
| 6 | `outer.rs:258` `EpochSchemeProvider` | схемы с `Arc<dyn SeedOracle>` внутри (ключ по ссылке) | prod |
| 7 | `cert_inlet.rs:403` `CertInlet.schemes` | то же, приватный кэш {prev, cur} | prod |
| 8 | `engine.rs:80` `fallback_seed: [u8;32]` ← `witness_fallback_seed(σ)` (`epoch_manager.rs:158-169`) | производная σ | prod |
| — | `testbed/fakes.rs:341` `FakeChain.seeds` | наблюдаемое стенда | test |

Ни одного ЖУРНАЛА seed/PK/share вне модуля нет (входное ограничение подтверждено); п. 5 — не журнал beacon-а, а сертификат, но он читается как источник σ в обход beacon.

### 2.6 Внутренние типы, видимые снаружи (св. 3)
| # | Что | Где снаружи | prod/test |
|---|---|---|---|
| 1 | `beacon::verified_seed::VerifiedSeed` (нет в `mod.rs:108-116`) | `spec_exec.rs:18`, `cert_inlet.rs:18` | prod |
| 2 | `beacon::keys::InvalidSeed` (нет в парадной двери) | `cert_inlet.rs:18` | prod |
| 3 | `beacon::seed::Seed` по внутреннему пути (тип экспортирован как `beacon::Seed`) | `application.rs:18`, `spec_exec.rs:18`, `executor.rs:134,199,564,605,2565,2963,3570` | prod |
| 4 | `pub(crate)` швы `CommitteeSource`, `frozen_dkg_qual`, `agreement_partition`, `absent_unregistered` (`mod.rs:120-123`) | `dpos.rs:3421,3444`, `epoch_manager.rs:17,352`, `application.rs:377`, `cert_inlet.rs:506` | prod |
| 5 | `Arc<dyn SeedOracle>` из `oracle_for` — хранится в схемах вне модуля | `outer.rs:258`, `cert_inlet.rs:403`, `engine.rs:225` | prod |
| 6 | `beacon::actor::DETERMINISTIC_BOOTSTRAP_EPOCH` | `epoch_manager.rs:1849`, `outer.rs:1821`, тесты executor | test |
| 7 | `beacon::surface::{PlaneRandomness, PlaneRandomnessConfig, StaticRandomness, testing::DealtOracle}` | тесты, `testbed/stand.rs:20` | test |
| 8 | `beacon::actor::{CommitteeFor, CommitteePairFor}`, `beacon::carry::DkgQualProbe` | `testbed/stand.rs:17-18` (`CommitteePairFor` также prod `node/dpos.rs:1410` через парадную дверь) | test / prod |
| 9 | `beacon::{ceremony::info_for, dkg_msg::*, wire::BeaconMessage, keys::InvalidSeed, verified_seed::VerifiedSeed}` | `testbed/byzantine_roles.rs:53-62` | test |
| 10 | `beacon::outcome::DkgOutcome` | `byzantine.rs:32-33` под `cfg(test)` | test |
| 11 | `beacon::{for_keys, for_seeds, absent}` — `pub`, «TEST ENTRY POINT» | `executor.rs:4537,4564`, `node/dpos.rs:2609`, `testbed/stand.rs:1788` — все в тестах | test |
| 12 | 8 хэндлов внутренних задач `Beacon` + приёмник `agreement_intake` | `node/dpos.rs:830-873,1986-2016`, `epoch_manager.rs:433` | prod |

### 2.7 Диаграмма DKG одной эпохи (по коду)
См. диаграмму в §2.9 «actor.rs» ниже (перенесена без изменений).

### 2.8 Куда идёт σ — три пути
1. Голос: `CombinedScheme::sign` ставит partial (`bls/combined_scheme.rs:268-293`), `assemble` восстанавливает σ при `quorum(n)` (`:359-393`), `verify_certificate` проверяет под `PK_E` или допускает при `NoKey` (`:395-445`).
2. Запись: `spec_exec.rs:52-128` (нотаризация) и `cert_inlet.rs:3012-3043` (два ингресса) зовут `VerifiedSeed::check` с `oracle_for` и `record_seed`/`quarantine_seed` — решение о вердикте снаружи.
3. Чтение: executor `seed_at_own_round` (`executor.rs:2915-2938`), `boundary_base` (`epoch_manager.rs:158-169`), crash-replay (`dpos.rs:564-706`) — последний при промахе стора читает σ из архива marshal и upstream без `VerifiedSeed`.

### 2.9 Заметки по файлам (свидетельства; фаза 1, только факты из кода)

### mod.rs — заявленная граница
- [KNOWN] `mod.rs:48-78`: все подмодули `pub(crate)`; `dkg_oracle` под `#[cfg(test)]` (`mod.rs:60`).
- [KNOWN] `mod.rs:108-116` `pub use`: `CommitteePairFor` (actor), `for_follower/ArtifactFetch/FollowerBeacon/FollowerRandomnessConfig` (follower), `AgreedKeys/BeaconKeys` (keys), `build/ArtifactSource/Beacon/BeaconConfig` (plane), `constant_fallback_seed/prev_randao_from_seed/witness_fallback_seed/Seed` (seed), `absent/for_keys/for_seeds/BeaconResolve/BeaconResolver/PinEffort/Randomness/ShareProbe/SignerVerdict/WithheldReason` (surface).
- [KNOWN] `mod.rs:120-123` `pub(crate) use`: `CommitteeSource`, `frozen_dkg_qual`, `agreement_partition`, `absent_unregistered`.
- [KNOWN] `mod.rs:106` `JOURNAL_RETENTION_EPOCHS = 1` — общая константа ретенции для журналов DKG и dealer-log кэша.
- Заявление в `//!` (`mod.rs:16-46`) о том, что продакшн ходит только через два яруса — это комментарий, проверяется в (д).

### wire.rs
- [KNOWN] `wire.rs:20-25`: единственный вариант `BeaconMessage::Dkg(Bytes)`, opaque; cap 64 KiB (`wire.rs:14`). Seed-партиалы по этому каналу не идут.

### seed.rs
- [KNOWN] `seed.rs:25-28` `Seed { target_round: Round, signature: BlsSignature }` — `pub`, публичный wire-тип.
- [KNOWN] `seed.rs:73-75` `prev_randao_from_seed = keccak256(signature.encode)`.
- [KNOWN] `seed.rs:93-106` `constant_fallback_seed(snap) = sha256(epoch_be ‖ sorted peer_pubkeys)`; `seed.rs:115-119` `witness_fallback_seed = sha256(signature)` — обе для «seedless arm» leader-elector. Фактический потребитель — проверить в (д).
- [KNOWN] `seed.rs:59-69` `parse_share` — декодер `Share` из hex-файла `beacon-share.hex` (по комментарию; потребитель проверить).

### verified_seed.rs
- [KNOWN] `verified_seed.rs:27-30` `VerifiedSeed { round, seed }` приватные поля, `pub` тип; конструктор `check(oracle: &dyn SeedOracle, …)` (`:43-52`) и `pub(crate) from_journal` (`:75-77`) без проверки.
- [KNOWN] `verified_seed.rs:94-143` `PkOracle` — `#[cfg(test)]` only. Значит `beacon::verified_seed::PkOracle` снаружи — только тест.
- Замечание: `VerifiedSeed` — `pub` (не `pub(crate)`), значит виден за пределы крейта, если родительский модуль реэкспортирует; в `mod.rs` реэкспорта нет, значит наружу крейта не виден, но внутри крейта — виден любому.

### confirmations.rs
- [KNOWN] `confirmations.rs:78-105` `Confirmations { me_key, committee_for, pool: Option<ConfirmPool>, recorded: Option<DkgLogIndex>, confirmed_len }` — «claimed-width» память: target epoch → ширина последнего заявленного набора dealer-логов. Инертна пока оба seam-а не подключены (`:154-160`).
- [KNOWN] `confirmations.rs:153-204` `mint(trigger)`: читает общий индекс `recorded_dkg_logs` (`DkgLogIndex` = actor), для каждой target-эпохи с ≥ quorum логов, ширина > предыдущей, при `Decisive` только первая ≥ quorum и полная `n`; подписывает `ShareConfirm` и кладёт в `pool`; возвращает `Outgoing` broadcast.
- [KNOWN] `confirmations.rs:212-217` `retain(floor)` — чистка по окну актора.

### dkg_transport.rs
- [KNOWN] `dkg_transport.rs:35-36` тела agreement едут через `commonware_broadcast::buffered::Engine<E, PeerPubkey, DkgProposal, P>`; `deque_size = MAX_COMMITTEE_SIZE` (`:123`), `mailbox 256` (`:32`), `priority: true`.
- [KNOWN] `dkg_transport.rs:66-71` sub-channel = `DKG_SUBCHANNEL_BASE | target_epoch`, отказ если `target_epoch >= BASE`.
- [KNOWN] `dkg_transport.rs:98-108` предусловие про `peers` (provider должен покрывать committee[target]) заявлено в комментарии и «checked nowhere» (сам комментарий это признаёт). Это точка тихого отказа: buffered хранит тело только от sender-а из tracked set (проверить в чекауте, `broadcast/src/buffered/engine.rs:298`).

### log_store.rs
- [KNOWN] `log_store.rs:74-89` `DealerLogStore { namespace, committee_for, share_dir: Option<PathBuf>, share_state: Arc<ShareState>, cache: BTreeMap<u64, ServeMap> }` — кэш dealer-логов для serve; positive-only (`:181-186`).
- [KNOWN] `log_store.rs:113-121` `get`: cache → cold parse журнала эпохи (`share_state::load_journal`) → `checked_serve_map`; `:136-158` `parse_journal` при `committee_for(epoch) == None` → пустая карта (не кэшируется).
- [KNOWN] `log_store.rs:171-177` `retain(floor)` возвращает выпавшие эпохи; удаление файлов — на акторе.

### dkg_msg.rs
- [KNOWN] `dkg_msg.rs:62-73` `DkgBody = Commitment | Share | Ack | Reveal | Confirm(ShareConfirm)`; `:78-81` `DkgMsg { ceremony_epoch: u64, body }` — `ceremony_epoch` не подписан.
- [KNOWN] `dkg_msg.rs:56` `pub type Ack = PlayerAck<PeerPubkey>` — `pub` (не `pub(crate)`), мелочь.

### outcome.rs
- [KNOWN] `outcome.rs:29-30` `DkgOutcome = Output<MinSig, PeerPubkey>` — `pub(crate)`; `:62` `pub fn group_public_key` — `pub`.
- [KNOWN] `outcome.rs:99-112` `validate_share_on_poly(outcome, committee, my_share)`: players == committee, `total == committee.len`, `partial_public(my_index) == share.public`. Комментарий `:79-95` сам утверждает, что gate C недостаточен без `verify_seed(σ₀, PK_E)`; какой вызывающий код запускает оба — проверить в actor/surface.

### metrics.rs
- [KNOWN] `metrics.rs:28-167` `pub struct BeaconMetrics` — 23 счётчика, все `pub` поля; регистрируется на commonware-реестре (`:172`). Комментарий `:1-3` заявляет, что `metrics::` макрос невидим на `:19100`.
- [KNOWN] Однако `resolve.rs:104-109,123-128`, `key_journal.rs:215-216,290-292` используют именно `metrics::counter!` (reth-реестр). Два реестра в одном модуле; это не дефект логики, но «второй путь рядом с первым» (св. 5) — заметка в §4.

### resolve.rs
- [KNOWN] `resolve.rs:69-134` `beacon_share_resolver(store: CeremonyStore, dkg_qual: DkgQualFor, namespace, group_keys: BeaconKeys) -> BeaconResolver`: читает `CeremonyStore` (RwLock<BTreeMap<u64,(CeremonyOutput, Share)>>, тип из actor), арбитраж `select_carry_scheme` по on-chain `dkgQual`, tripwire `mint_diverges_from_attested` (`:41-49`) против `BeaconKeys::attested(minted_at)`. Результат `BeaconResolve::Key((Sharing, Some(share), namespace))` или `Absent`.
- [KNOWN] `resolve.rs:76-78` `store.read` poisoned → `Absent` тихо.
- Замечание: `CeremonyStore` (память) + `BeaconKeys` (память + key_journal на диске) + `ArtifactStore` (диск, см. artifact.rs) — уже три носителя PK_E внутри модуля.

### key_journal.rs
- [KNOWN] `key_journal.rs:146-148` `KeyJournal` = `commonware_storage::ordinal::Ordinal<E, KeyRecord>`; запись `KeyRecord { pk: GroupPublic, source: KeySource }` 97 байт (`:119-121`).
- [KNOWN] `key_journal.rs:170-181` `append` персистит ТОЛЬКО `KeySource::Agreed`; `LocalDkg`/`Carried` — только RAM.
- [KNOWN] `key_journal.rs:230-258` `open(journal_ctx, writer_ctx, partition)`: пустая partition ⇒ RAM-only `BeaconKeys::new`; иначе replay → `BeaconKeys::with_persistence(rehydrated, tx)` + writer task.
- [KNOWN] `key_journal.rs:262-300` writer: unbounded mpsc; ошибка `append` → `warn!` и ключ остаётся RAM-only (`:280`); ошибка `sync` → счётчик + warn (`:291-297`). Оба — «warn-и-продолжить» (св. 4).
- [KNOWN] `key_journal.rs:195-218` replay: битые записи пропускаются с warn — miss, не wrong key.
- [KNOWN] Ретенции нет (`:38-54`): store растёт по одной записи на смену комитета.

### carry.rs
- [KNOWN] `carry.rs:40` `pub type DkgQualFor = Arc<dyn Fn(u64) -> Option<bool>>` — `pub`; `:44-55` `pub enum CarryVerdict`; `:158` `pub fn select_carry_scheme`; `:223` `pub fn frozen_dkg_qual` (реэкспортирован pub(crate) в mod.rs).
- [KNOWN] `carry.rs:74-76` `chain_key_epoch(epoch, dkg_qual)` создаёт СВЕЖИЙ memo на каждый вызов (`&Mutex::new(BTreeMap::new)`) ⇒ `select_carry_scheme` (`:158-174`) и через него `beacon_share_resolver` идут полным проходом `(BOOTSTRAP+1..=epoch).rev` при каждом вызове; memoised-вариант `chain_key_epoch_memoised` (`:110-153`) — для второго потребителя (keys.rs `AgreedKeys::key_for`, проверить). Два пути одной функции.
- [KNOWN] `carry.rs:115-126` `epoch < DETERMINISTIC_BOOTSTRAP_EPOCH` ⇒ `Some(None)`; комментарий признаёт скрытую зависимость `PlaneRandomness::signer_scheme` от этой строки.
- [KNOWN] `carry.rs:223-242` `frozen_dkg_qual`: кэш `epoch → bit` без ретенции; `None` если bit clear И committee not committed.
- [KNOWN] `carry.rs:131-147` при `dkg_qual(e) == None` в любой точке прохода — весь ответ `None` (undecided).

### dkg_oracle.rs (cfg(test))
- [KNOWN] `dkg_oracle.rs:51-104` `run_local_dkg` — однопроцессный референс JF-DKG (Dealer::start → Player::dealer_message → ack → Dealer::finalize.check → Logs::record → Player::finalize). Только тесты.

### oracle.rs
- [KNOWN] `oracle.rs:63-100` `BeaconOracle { epoch, ceremony: CeremonyStore, keys: BeaconKeys, dkg_qual, namespace, me: Option<Participant>, minted_at memo, два warn-latch, metrics }` — реализует `fluentbase_bls::oracle::SeedOracle` для ОДНОЙ эпохи; `pub(crate)`, все поля `pub(crate)`.
- [KNOWN] `oracle.rs:153-168` `with_material`: читает `ceremony.read` (poisoned ⇒ `None` тихо), `minted_at_for` (memo только `Serve`), затем divergence gate `mint_diverges_from_attested(keys, minted_at, sharing.public)` при КАЖДОМ вызове.
- [KNOWN] `oracle.rs:172-189` `sign_partial`: `self.me != Some(share.index)` ⇒ `None` + warn один раз. `:191-204` `verify_partial` через `beacon::verify_seed_partial(sharing, …)`. `:206-247` `recover` через `recover_seed_with_threshold`; порог передаёт вызывающий (scheme).
- [KNOWN] `oracle.rs:249-273` `verify_seed` читает `keys.cached_only(self.epoch)` — ключ ПОД ЖИВОЙ эпохой (не под mint), любого provenance (`LocalDkg` включительно) ⇒ `Some(_) if !verify ⇒ Invalid`. Divergence gate `with_material` сюда НЕ доходит: если под живой эпохой лежит расходящийся `LocalDkg` ключ, честный σ получит `Invalid`. Кто пишет `LocalDkg` под живой эпохой (W1) — найти в surface/epoch_manager.
- [KNOWN] `oracle.rs:296-339` `KeyOnlyOracle` — follower: sign/verify_partial/recover всегда негативны, `verify_seed` как выше.

### plane.rs — сборка
- [KNOWN] `plane.rs:429-850` `build(context, BeaconConfig) -> Beacon`. Создаёт: `ceremony_store` (RAM, `share_state::load_all` из `share_dir` `:479`), `recorded_dkg_logs: DkgLogIndex` (`:499`), `BeaconMetrics` (`:503-504`), `ConfirmPool` (`:514`), `BeaconKeys` + key_journal (`:529-534`), `SeedStore` + seed_journal (`:567-573`), `ArtifactStore` + writer (`:586-591`) с дозаливкой артефактов из share-файлов (`:598-614`), `restart_replay` (`:623`), resolver seam (`:631-643`), `DkgActor` (`:674-720`), agreement launcher (`:726-753`), write-back (`:759`), `PlaneRandomness` (`:786-802`), seed promoter (`:815-835`).
- [KNOWN] `plane.rs:294-364` `pub struct BeaconConfig` — 20 полей, в т.ч. четыре mux-а, `heights: mpsc::Receiver<u64>`, `plane_clock: crate::sync_metrics::PlaneClock`, `geometry: BoxFuture<Option<(u64,u64)>>`, `partition_prefix`.
- [KNOWN] `plane.rs:384-420` `pub struct Beacon` — 8 handle-ов (3 из них `Option`) + `agreement_intake: mpsc::Receiver<(Epoch, Handle<()>)>` + `randomness: Arc<dyn Randomness>` + `artifact_bytes: ArtifactSource`. То есть наружу выходит не «один объект», а набор задач, которые узел обязан супервизировать по-разному (supervised / drain / abort-only).
- [KNOWN] `plane.rs:256-287` write-back: `beacon_keys.set_pk(target_epoch, pk, Agreed)` + `adopt_tx.send(artifact)` → актор. `:281-285` паркуется навсегда (`pending`), чтобы супервизор не счёл выход смертью.
- [KNOWN] `plane.rs:684-693` geometry `None` ⇒ `error!` и `DkgActor` не стартует, процесс живёт (fail-soft заявлен для не-валидатора; для валидатора «уже упал раньше» — по комментарию, проверить в node).
- [KNOWN] `plane.rs:598-614` артефакт из share-файла, который не декодируется ⇒ `warn` и «re-agreeing».
- [KNOWN] `plane.rs:786-802` `PlaneRandomnessConfig { seeds, keys, resolver: beacon_share_resolver(...), ceremony, dkg_qual, held, pull, participation: share_notify, metrics, chain_id }`.
- [KNOWN] `plane.rs:815-835` seed promoter: ждёт `beacon_keys.subscribe`, для каждой quarantined-эпохи `randomness.oracle_for(epoch)` → `promote_epoch`.
- [KNOWN] Метрики регистрируются в `build` (`:504`) — при двух `build` на одном контексте (testbed) — проверить префикс.

### certify.rs — SeedStore
- [KNOWN] `certify.rs:71-112` `pub struct SeedStore { seeds: round→σ (RAM, ≤ SEED_RETENTION=4096), notify, persist: Option<UnboundedSender>, quarantined: round→σ, waiters: round→Vec<oneshot>, terminal: epoch→(round,σ) }` — четыре карты в одном типе.
- [KNOWN] `certify.rs:179-181` `record(VerifiedSeed)` — единственный вход в served map; `:191-242` `insert`: poisoned lock ⇒ warn+drop (`:194-199`); две разные σ за один раунд ⇒ `error!` + первая остаётся (`:201-209`) — это «safety witness», обработанный как лог.
- [KNOWN] `certify.rs:246-248` `lookup(round)` — синхронный `pub`.
- [KNOWN] `certify.rs:255-279` `quarantine(round, σ)` без проверки; `:288-327` `promote_epoch(epoch, &dyn SeedOracle)`; `:366-378` `pin_terminal` — «highest seen», не «terminal»; `:390-396` `terminal_at(round)` только точное совпадение.
- [KNOWN] Комментарий `:3-5,42-45` называет писателя — notarization Reporter `crate::spec_exec::Mailbox`, т. е. ВНЕ beacon (проверить в spec_exec.rs). Кто ещё пишет: ingress (cert_inlet), by-round transport (surface?).

### seed_journal.rs
- [KNOWN] `seed_journal.rs:153-158` `SeedJournal = Ordinal<E, BlsSignature>` с индексом `epoch<<32|view` (`:136-143`, отказ при переполнении). `ITEMS_PER_BLOB = 2^32` ⇒ blob == epoch.
- [KNOWN] `seed_journal.rs:351-383` `open` → `SeedStore::with_persistence(rehydrated, terminals)`; ошибки init/replay — фатальны (`?`), ошибки записи/sync/prune в writer — `warn` и продолжить (`:430,438-444,446-451`).
- [KNOWN] `seed_journal.rs:410-456` writer: одна fsync на batch ≤ 256; handle — drain, не supervised.

### keys.rs — BeaconKeys
- [KNOWN] `keys.rs:78-102` `KeySource = LocalDkg < Carried < Agreed` (Ord = политика конфликта).
- [KNOWN] `keys.rs:153-175` `pub struct BeaconKeys { map: epoch→(pk, source), notify, extra_notifiers, reported_invalid_seed, persist }`.
- [KNOWN] `keys.rs:280-298` `set_pk` — единственный вход; `:300-343` `insert`: конфликт значений ⇒ `warn` + `metrics::counter!("dpos_group_key_conflict_total")` + сильнейший provenance побеждает; два РАЗНЫХ `Agreed` — только `debug_assert!` (`:339-342`), в release «first_write_kept» и процесс живёт. Комментарий сам называет это «fork-grade, never a handled state» — а код его обрабатывает молча (св. 4).
- [KNOWN] `keys.rs:374-378` `retain_from(oldest)`: `Agreed` не чистится никогда; `LocalDkg`/`Carried` — окно.
- [KNOWN] `keys.rs:416-431` `on_invalid_seed(epoch) -> Quarantine | RefuseLoud | RefuseQuiet` по наличию `Agreed`-записи под ЭТОЙ эпохой (не под mint!): на стабильном комитете под живой эпохой `Agreed` нет никогда (только `Carried`), значит любой неверный σ уходит в карантин, а не в «RefuseLoud» — подозрение, проверить потребителей (cert_inlet).
- [KNOWN] `keys.rs:466-506` `AgreedKeys { at: AgreedKeyAt, dkg_qual, carry_memo }` — `key_for(epoch)` = `chain_key_epoch_memoised` → `at(minted_at)`.
- [KNOWN] `keys.rs:603-623` `get_pk(epoch, KeySources{held,pull,store_floor})` — лестница: store → held artifact → pull; результат пишется `Agreed` под mint и `Carried` под epoch.
- [KNOWN] Писатели `LocalDkg` (W1/W3) в этом файле не видны — искать в surface.rs / epoch_manager.rs.

### follower.rs
- [KNOWN] `follower.rs:120-191` `for_follower(ctx, FollowerRandomnessConfig{chain_id, committees, dkg_qual, fetch})` → `FollowerBeacon { randomness, artifact_bytes, fetch_handle }`. Регистрирует `BeaconMetrics` второй раз на своём контексте (`:124-125`) — follower и plane никогда в одном процессе, иначе двойная регистрация.
- [KNOWN] `follower.rs:342-360` `FollowerRandomness { keys: BeaconKeys (RAM), held: AgreedKeys, want_tx, seeds: SeedStore (RAM), idle, metrics }`; `share_probe`/`signer_scheme` всегда `Withheld(NoUsableShare)` (`:436-447`).
- [KNOWN] `follower.rs:284-337` `run_fetcher`: последовательный fetch, throttle `PULL_MIN_INTERVAL`; на `key_edge` — `promote_quarantined`; паркуется навсегда.
- [KNOWN] `follower.rs:200-264` `fetch_and_verify`: `verify_artifact_for_epoch` против `committee[minted_at]` из своего chain-state; `CommitteeUnreadable` ⇒ drop без вины.

### share_state.rs — диск
- [KNOWN] `share_state.rs:85-86,296-298` share-файл `beacon-share-e<E>.bin` (mode 0600), v2 = `(output, share, artifact?)` (`:94-103`), plaintext или XChaCha20-Poly1305 с AAD `(tag, version, epoch)` (`:212-219`).
- [KNOWN] `share_state.rs:307-319` `persist` — best-effort, ошибку логирует вызывающий; `:330-366` `load_all` — недекодируемый/insecure файл ⇒ `warn` + skip (никогда не abort).
- [KNOWN] `share_state.rs:368,524-526` DKG-журнал `beacon-dkgjournal-e<E>.bin` — рукописный append-лог `u32_be(len) ‖ framed`, записи `ReceivedDealing | OwnSeal | PeerLog | OwnDealerAck` (`:377-393`); `append_journal` best-effort (`:533-543`).
- [KNOWN] `share_state.rs:575-621` `load_journal` → `NoFile | Present(prefix) | Torn`: нулевой файл = `NoFile`; первый битый record = `Torn` (sit out); битый record посередине = обрезка с `warn`.
- [KNOWN] `share_state.rs:668-684` `reconcile_journals(dir, now)`: журналы `epoch + 1 < now` удаляются; share-файлы старше активного floor `max{e ≤ now}` удаляются.
- Это ТРЕТИЙ формат хранения (после Ordinal для seed/key и Metadata для artifact): собственный фрейминг, собственная AEAD-обёртка, собственная ретенция.

### artifact.rs
- [KNOWN] `artifact.rs:189` `pub type CommitteeSource = Arc<dyn Fn(u64) -> Option<EpochCommittee>>`; `:246-273` `verify_artifact(rng, chain_id, committee, (proposal, cert))`: epoch-совпадение (committee/proposal/cert), `cert.proposal.payload == proposal.digest`, `cert.verify` под `dkg_namespace` с `oracle: None`. Проверяет ТОЛЬКО multisig-кворум; содержимое `group_key` (полином) НЕ проверяется здесь — это ответственность голосования в agreement (`dkg_agree` verify), см. далее.
- [KNOWN] `artifact.rs:418-506` `pub struct ArtifactStore { ram: epoch→Arc<AgreedArtifact>, durable: Option<UnboundedSender> }` — first-wins (`:452-474`), без ретенции (`:406-416`); `:565-617` durable = `commonware_storage::metadata::Metadata<E, U64, Vec<u8>>` (третий storage-примитив в модуле); replay пропускает битые записи с warn.
- [KNOWN] `artifact.rs:537-549` `restart_replay(store, share_dir, held_shares)` — артефакты для эпох без share, но с журналом на диске → в write-back.
- [KNOWN] `artifact.rs:704-897` `ArtifactBridge` (`produce`/`deliver`); `deliver` возвращает `false` (перманентное исключение peer в resolver) только за недекодируемое/чужая эпоха/плохой сертификат; `:819-835` два РАЗНЫХ quorum-артефакта за одну эпоху ⇒ `warn`, держим свой — «fork-grade», обработано как лог (св. 4).
- [KNOWN] `artifact.rs:904-1011` `ArtifactPull::pull` — throttle 5 s/epoch, timeout 8 s, cancel при отсутствии ответа.

### dkg_engine.rs
- [KNOWN] `dkg_engine.rs:267-442` `spawn_agreement`: второй `simplex::Engine` на `dkg_namespace` (`:295`), `RoundRobin` elector, `NoopBlocker`, партиция `dkg_epoch_{E}` (`:256-258`), без `register_scheme`; supervisor ждёт `verdict_rx` от `DkgReporter`, abort'ит engine, `resolve_artifact` (`:469-487`: store → `bodies.subscribe(digest)` с таймаутом = certification 45 s), удаляет партицию, `artifacts.insert`, `out.send`.
- [KNOWN] `dkg_engine.rs:120-125` `LEADER_TIMEOUT = 30 s`, `CERTIFICATION_TIMEOUT = 45 s` — грубые по двум причинам (рост blob-ов журнала voter-а и `interesting` skew); при `n` view до соглашения — `n × 30 s`.
- [KNOWN] `dkg_engine.rs:390-397` тело не пришло за 45 s ⇒ `dkg_agree_body_lost` + warn, артефакта нет; «target epoch has to re-agree on a fresh instance» — но launcher держит target в `started` (`:584-587,608-609`), т. е. ПОВТОРНЫЙ запуск для этого target в этом процессе НЕ происходит (только при рестарте процесса). Комментарий и код расходятся; последствие — узел без артефакта до pull/heal.
- [KNOWN] `dkg_engine.rs:557-640` launcher: dedup `started`, `committee(target)==None` ⇒ warn+retry на следующем запросе; `adopted.send` fail ⇒ launcher выходит (`:610-624`), инстанс остаётся жить без владельца.
- [KNOWN] `dkg_engine.rs:653-740` `start_one`: регистрирует 4 sub-channel-а; `NotAMember` ⇒ settled.
- [KNOWN] `dkg_engine.rs:513-519` (и `dkg_transport.rs:98-108`) предусловие на `peers` — заявлено, не проверяется.

### ceremony.rs
- [KNOWN] `ceremony.rs:133-170` `DkgCeremony { epoch, info, dealer: Option<Dealer>, player: Option<Player>, logs: Logs, pending_pub/priv, recorded: BTreeSet, signed_logs, own_pub_msg, unsent, emitted_acks }` — состояние одной церемонии; seal-состояние = `dealer.is_none` (`:580-582`), финализируемость = `player.is_some` (`:837-839`).
- [KNOWN] `ceremony.rs:235-240` `dealer_seed_rng(me_key, epoch)` — полином дилера детерминирован из ed25519-подписи `(DEALER_SEED_NS, epoch)`: одинаков при любом рестарте И при любом составе комитета для того же номера эпохи.
- [KNOWN] `ceremony.rs:362-397` `handle(from, body)`: Commitment/Share → `try_ack`; Ack → `dealer.receive_player_ack` (ошибка игнорируется `let _ =`), при этом `unsent.remove(from)` и `OwnDealerAck` журналируется ДАЖЕ при невалидном ack (`:382-385`); Reveal → `check` → `record_checked_log`.
- [KNOWN] `ceremony.rs:521-557` `seal_dealings`: `dealer.take`, `finalize`, self-`check`; при провале self-check — `warn` и узел не вносит dealing (тихий выход из dealer-quorum, но с warn).
- [KNOWN] `ceremony.rs:680-828` `resume(records, reconstruct_dealer)`: pre-seal — re-derive dealer (тот же seed), replay `OwnDealerAck`; post-seal — player-only, re-broadcast `OwnSeal` если есть. `Player::resume` ошибка (`MissingPlayerDealing`) → `Err`.
- [KNOWN] `ceremony.rs:870-895` `scoped_pinned_logs`: idx вне комитета — молча skip; `:943-960` `derive_pinned`: idx вне комитета → `Unavailable`; missing → `Missing(idxs)`; `observe` Err → `Unusable`.
- [KNOWN] `ceremony.rs:988-997` `finalize_over_pinned`: `player.take.expect("can_finalize gates this")` — паника при нарушении контракта вызывающего; `Player::finalize` Err ⇒ player потерян навсегда (ceremony «sits out», восстановление только через `recompute_scoped` из журнала).
- [KNOWN] `ceremony.rs:1016-1057` `recompute_scoped(rng, ns, epoch, committee, me_key, dealers, records)` — heal-примитив: `Player::resume` + `finalize` над журналом, scope = `Output::dealers`.

### surface.rs — трейт `Randomness` (граница «ядро ↔ beacon» как она есть)
- [KNOWN] `surface.rs:113-232` `pub trait Randomness` — 15 методов: `record_seed(VerifiedSeed)`, `quarantine_seed(round, σ)`, `on_invalid_seed(epoch) -> InvalidSeed`, `seed_for(round) -> Option<Seed>`, `terminal_seed_at(round)`, `seed_edge`, `mandatory_at(epoch)` (default `epoch >= DETERMINISTIC_BOOTSTRAP_EPOCH`), `share_probe(epoch) -> ShareProbe`, `signer_scheme(epoch, snap, keypair) -> SignerVerdict`, `participation_edge`, `oracle_for(epoch) -> Option<Arc<dyn SeedOracle>>`, `ensure_key(epoch, PinEffort) -> bool`, `key_edge`, `observe_epoch(reconciled, frontier)`, `observe_cert(epoch)`.
- Наблюдение по св. 1–3: `record_seed`/`quarantine_seed`/`on_invalid_seed` означают, что ПРОВЕРКА σ и решение «карантин/отказ» выполняются потребителем (spec_exec/cert_inlet) с оракулом, полученным через `oracle_for`, и лишь потом «сдаются» в beacon. Beacon не «принимает partial и возвращает seed» — он выдаёт оракул, а сборкой σ занимается `CombinedScheme` в bls-крейте, записью — reporter в spec_exec. Три `Notify`-edge наружу (seed/participation/key) с правилом «один потребитель».
- [KNOWN] `surface.rs:43` `pub(crate) type BeaconKey = (Sharing<MinSig>, Option<Share>, Vec<u8>)` — share и полином гуляют кортежем внутри крейта.
- [KNOWN] `surface.rs:1633-1778` `promote_gates`: value-gate (`attested(epoch)` vs local pk), share-gate (`sign_seed_partial`/`verify_seed_partial` на пробном раунде `(epoch, view 1)`), W1: `group_keys.set_pk(epoch, pk, LocalDkg)` под ЖИВОЙ эпохой (`:1743`) если `attested(epoch)` пуст. `:1790-1808` W3: backfill `epoch-1` через resolver (`LocalDkg`).
- [KNOWN] `surface.rs:1954-1984` `share_probe`: `material.is_none` при `mandatory_at` ⇒ `NoUsableShare` + метрика `engine_demoted_no_polynomial` НА КАЖДЫЙ вызов (probe идёт на каждом reconcile-edge ⇒ счётчик растёт на каждом тике для verify-only ноды — семантика «событие» нарушена; мелочь).
- [KNOWN] `surface.rs:1986-2108` `signer_scheme`: повторный sample `material` (комментарий сам описывает «две выборки» как источник дефекта), `promote_gates`, seat через probe-`build_signer`, `RotatedKey` с `oracle_for`, иначе `Signs(build_signer(oracle_at(epoch, Some(seat))))`.
- [KNOWN] `surface.rs:2126-2174` `ensure_key`: `mandatory_at` → `get_pk(held, pull при Thorough, store_floor=Carried)`.
- [KNOWN] `surface.rs:2180-2201` `observe_epoch`/`observe_cert`: W3 + `retain_from` на трёх картах — два разных «frontier» (entered vs cert) для одной ретенции.
- [KNOWN] `surface.rs:252-268` `absent`/`absent_unregistered`; `:581-624` `for_seeds`/`for_keys` — `pub`, «TEST ENTRY POINT» по комментарию, проверить потребителей в (д)/(е).
- [KNOWN] `surface.rs:292-559` `StaticRandomness` (cfg(test)) — доказательство подстановки: seed = детерминированный deal от `H(chain_id‖epoch‖committee)`.

### dkg_agree.rs — вторая simplex-плоскость (agreement)
- [KNOWN] `dkg_agree.rs:447-452` `pub struct DkgProposal { target_epoch, logs: Vec<(u8,B256)>, group_key: DkgOutcome, confirms: Vec<ShareConfirm> }` — `pub` с `pub` полями; `:656` `pub type AgreedArtifact = (DkgProposal, Finalization<BlsScheme, Digest>)`.
- [KNOWN] `dkg_agree.rs:121-132` `ShareConfirm { idx, target_epoch, recorded, sig }`; подпись над `target_epoch ‖ keccak(canonical(recorded))` (`:156-163`).
- [KNOWN] `dkg_agree.rs:293-303` `entry_bar(n, view) = quorum + margin(f)` до `MARGIN_RELEASE_VIEW=3`, затем `quorum`; `margin = min(f/2, 2)`.
- [KNOWN] `dkg_agree.rs:321-436` `ConfirmPool` — карта epoch→idx→ShareConfirm + `watch` для пробуждения лидера.
- [KNOWN] `dkg_agree.rs:1104-1183` `build_proposal`/`attempt_proposal`: цикл до появления входов; первый допустимый набор, не самый широкий.
- [KNOWN] `dkg_agree.rs:1194-1280` `rejects_structurally` — чистые проверки; `:1295-1363` `decide`: certified-value bar (`certified_value(parent)`), тело через `bodies.subscribe`, `pinned.derive(set)`: `Derived==group_key ⇒ Accept`, иначе Reject; `Unusable ⇒ Reject`; `Missing ⇒ fetch + Park`; `Unavailable ⇒ Park`.
- [KNOWN] `dkg_agree.rs:959-969` `drive` — Park держит `tx` открытым до дропа получателя (иначе `IgnoredProposal` ⇒ nullify).
- [KNOWN] `dkg_agree.rs:1600-1648` `DkgReporter` — только `Finalization`, один раз; agreement-раунды не идут в slasher (комментарий признаёт: эквивокация внутри плоскости unslashable — «bounded cost»).
- Замечание (св. 6): проверка `group_key == derive(pinned)` требует ВСЕ тела pinned-логов у голосующего; голосующий без тел паркуется. Соглашение требует quorum голосов ⇒ ≥ quorum узлов с полным набором тел; при `≤ f` отсутствующих — живучесть за счёт resolver-fetch; корректно по протоколу.

### log_resolver.rs
- [KNOWN] `log_resolver.rs:50-53` `pub struct DkgLogKey { epoch, dealer }`; `:111-117` `pub enum BeaconFetchKey { Log, Artifact{epoch} }` (tag 2 retired).
- [KNOWN] `log_resolver.rs:290-341` `BeaconFetchHandler` (Consumer+Producer) → `LogHandler` (mpsc в актор, oneshot ответ) / `ArtifactBridge`.
- [KNOWN] `log_resolver.rs:385-401` `deliver`: если актор ушёл ⇒ `error!` + `false` (peer будет исключён навсегда за ОТСУТСТВИЕ актора — не его вина); `:403-410` `failed` — no-op.

### actor.rs — DkgActor (продакшн `:1-2448`)
- [KNOWN] `actor.rs:115` `DKG_MARGIN_BLOCKS = 20`; `:123` `pub const DETERMINISTIC_BOOTSTRAP_EPOCH: u64 = 2` (`pub`, читается снаружи: surface default `mandatory_at`, application и др. — проверить в (д)).
- [KNOWN] `actor.rs:169` `pub type CeremonyStore = Arc<RwLock<BTreeMap<u64, (CeremonyOutput, Share)>>>` — `pub`; `:178` `pub type DkgLogIndex`; `:231` `pub type CommitteeFor`; `:251-252` `pub type CommitteePairFor`; `:137-138` `pub type AgreedOutcomeAt`; `:150` `pub type PullArtifact`.
- [KNOWN] `actor.rs:335-521` `DkgActor` — 30+ полей: `ceremonies`, `pending`, `nondurable_logs`, `recompute_pending`, `terminal_recompute`, `agreed_pinned`, `agreement_announced`, `deferred_reported`, `eval_logged`, `torn_warned`, `log_store`, `confirmations`, `store` (CeremonyStore), `recorded_dkg_logs`, и семь опциональных seam-ов (`resolver`, `resolver_rx`, `pinned_rx`, `agreement_tx`, `artifacts_rx`, `outcome_at`, `pull_artifact`, `committee_pair_for`, `plane_clock`) — каждый «`None` ⇒ ветка инертна» (св. 5: конфигурация поведения через Option-поля, тестовый и продакшн путь различаются составом seam-ов).
- [KNOWN] `actor.rs:791-854` `run`: `select!` над heights / receiver / resolver_rx / pinned_rx / artifacts_rx. Закрытие resolver_rx ⇒ resolver отключается навсегда, процесс живёт (`:828-831`).
- [KNOWN] `actor.rs:1145-1272` `on_height`: monotone clamp; первый тик — `reconcile_journals`; порядок: seal due → evict pending → `drive_finalization` → `mint(AnyGrowth)` → announce → `sweep_epoch_state` → retransmit → `maybe_start(now+1)` → broadcast → `drive_recompute` → `fetch_missing_logs`.
- [KNOWN] `actor.rs:1436-1577` `drive_finalization`: `publish_recorded_logs`; для каждой закрытой церемонии с `agreed_pinned` — `pinned_ready`; при `all_held && ready` — `finalize_over_pinned`; Ok ⇒ удалить церемонию, `log_store.seed`, `adopt_share` (persist → store.insert → notify); Err ⇒ `dkg_ceremony_fail` + warn, церемония остаётся с `can_finalize==false` (навсегда для процесса).
- [KNOWN] `actor.rs:999-1025` `adopt_share`: persist best-effort (warn), затем RAM insert, `notify_one`, метрика.
- [KNOWN] `actor.rs:1582-1722` `maybe_start(target)`: skip если ceremony/store содержат target или `torn_warned`; читает пару комитетов; `next == cur && target != 2` ⇒ carry-forward; не член ⇒ выход; tri-state журнала: NoFile→`start_fresh`, Present→`resume(reconstruct = height < deadline)`, Torn→sit out (warn once). Затем drain `pending`.
- [KNOWN] `actor.rs:1832-1934` `on_message`: decode; `Confirm` перехватывается; активная церемония — `handle` → journal (ack удерживается при не-durable записи) → broadcast → при новом логе `drive_finalization` + `mint(Decisive)`; иначе buffer.
- [KNOWN] `actor.rs:1951-2065` `fetch_missing_logs`: для каждой живой церемонии и `recompute_pending` — `fetch_targeted` по roster; `retain` в resolver = wanted ∪ unreadable.
- [KNOWN] `actor.rs:2084-2163` `drive_recompute`: окно `[max(2, now-1), now]`; член без share и с артефактом (`outcome_at`) ⇒ `recompute_pending{outcome, want}`; без артефакта ⇒ `pull_artifact(e)`.
- [KNOWN] `actor.rs:2178-2266` `try_recompute_pending`: `recompute_scoped` + `validate_share_on_poly` gate (только C-gate! `verify_seed(σ₀, PK_E)` из `outcome.rs:79-95` здесь НЕ выполняется — комментарий outcome.rs требует «callers MUST run both»; вызывающий полагается на то, что `outcome` пришёл из quorum-артефакта, что делает высокостепенную подделку невозможной без quorum — см. §4); `MissingPlayerDealing` ⇒ terminal.
- [KNOWN] `actor.rs:2346-2390` `ingest_log`: undecodable ⇒ `false` (peer блокируется навсегда — принято осознанно, комментарий `:2340-2345`).
- [KNOWN] `actor.rs:2434-2447` `broadcast_all`: send-ошибки игнорируются (`let _ =`).
- [KNOWN] `actor.rs:224-227` `ceremony_retain_floor` + комментарий `:190-223`: корректность prune `CeremonyStore` зависит от инварианта «mint ⇒ dkgQual bit set», который код не проверяет.
- Диаграмма DKG одной эпохи (по коду, target E, время — эпоха E−1):
  ~~~
  [Idle] --maybe_start(E): committee changed (or E==2) && me∈committee[E]-->
     NoFile  --> [Dealing]   (DkgCeremony::start; commitment broadcast, shares p2p; journal ReceivedDealing(me))
     Present --> [Dealing]   if height < seal_deadline (reconstruct dealer)  |  [PlayerOnly] otherwise
     Torn    --> [SatOut]    (permanent for the process; torn_warned)
  [Dealing] --on_message Commitment+Share--> ack (journal-gated) ; --Ack--> prune unsent ; --Reveal--> record log
  [Dealing] --height >= epoch_start(E)-20--> seal_dealings --> [Sealed]  (OwnSeal journal, Reveal broadcast)
  [Sealed]/[PlayerOnly] --announce_agreement_targets--> agreement instance (dkg_engine) for E
  [Sealed] --artifact (own instance | pull | restart_replay)--> agreed_pinned[E]
  [Sealed]+agreed_pinned --pinned_ready(all_held && ready)--> finalize_over_pinned
         Ok  --> [Finalized]: persist share file, CeremonyStore[E], share_notify, log_store.seed; ceremony removed
         Err --> [Stalled-NoPlayer]: can_finalize=false forever; serves logs; heal only via drive_recompute after sweep
  [Sealed] without artifact by sweep (E+1 < now) --> evicted; journal deleted --> [Gone]
  [Gone]+me∈committee[E]+no share+artifact --> recompute_pending --> recompute_scoped --> validate_share_on_poly
         ok --> [Finalized] (no artifact bytes in share file) | MissingPlayerDealing --> [Terminal]
  ~~~
### bls crate (в) — где seed едет в голосе
- [KNOWN] `bls/combined_scheme.rs:102-105` `CombinedSignature { vote, seed: Option<BlsSignature> }` фиксированные 97 B; `:140-143` `CombinedCertificate { vote, seed: Option<BlsSignature> }`.
- [KNOWN] `bls/combined_scheme.rs:268-293` `sign`: в beacon-active эпохе КАЖДЫЙ subject (Notarize/Finalize/Nullify) несёт partial `oracle.sign_partial(round)`; если оракул вернул `None` — голос НЕ отдаётся вовсе (`?` на `:286`).
- [KNOWN] `bls/combined_scheme.rs:295-357` `verify_attestation`: сначала multisig, затем при оракуле — `verify_partial`; отсутствие/невалидность partial ⇒ голос целиком невалиден (подтверждает входное ограничение «CombinedScheme отвергает голос целиком»).
- [KNOWN] `bls/combined_scheme.rs:359-393` `assemble`: threshold = `M::quorum(participants.len)` (`:387`), `oracle.recover(partials, threshold)`; если хоть один attestation без seed — `seed = None` (`:369-391`) при живом оракуле, т. е. сертификат без σ (потом отвергнется в `verify_certificate:443`).
- [KNOWN] `bls/combined_scheme.rs:395-445` `verify_certificate`: multisig → binds → при оракуле `verify_seed`: `Invalid` ⇒ false; `NoKey` ⇒ ДОПУСК (vote-only); `seed == None` ⇒ false.
- [KNOWN] `bls/beacon.rs:147-156` `recover_seed_with_threshold` требует `threshold == sharing.required::<N3f1>()` — жёсткая связь «n комитета == total шаринга».
- [KNOWN] `bls/beacon.rs:81-87` partial = `threshold::sign_message(share, ns, round.encode)`; `:161-167` `verify_seed(pk, ns, round, σ)`.
- [KNOWN] `bls/oracle.rs:38-63` `trait SeedOracle { sign_partial, verify_partial, recover, verify_seed }` — sync by contract.
- [KNOWN] `bls/scheme.rs:68-92` `build_signer(ns, bimap, keypair, epoch, oracle)`/`build_verifier` — единственные конструкторы схемы; `EpochCommittee { epoch, bimap }`.
- [KNOWN] `bls/keys.rs:55-72` `derive_share_seal_key` — HKDF от BLS-скаляра, salt = chain namespace; `bls/secret_store.rs:110-134` `write_mode_0600` атомарно (staging+rename+fsync), `:174-192` `append_mode_0600` с fsync на каждую запись (журнал DKG = fsync на каждый record).
- Вывод по входным ограничениям: (1) «seed едет в голосе и восстанавливается из сертификата» — ПОДТВЕРЖДЕНО; (2) «порог = quorum(n)» — ПОДТВЕРЖДЕНО (`combined_scheme.rs:387`); (3) «CombinedScheme отвергает голос целиком при неверном partial» — ПОДТВЕРЖДЕНО (`:348-351`).

### commonware (г) — чекаут `~/.cargo/git/checkouts/monorepo-27b478c9bb41d208/3c4e02c`, sha 3c4e02ceede03126f524216605a1195e1cee7d0e
- [KNOWN] `dkg.rs:1425-1443` `Logs::select`: `pre_verify` → фильтр валидных → `.take(required_commitments)` в порядке ключей `BTreeMap<P, DealerLog>` (порядок pubkey), т. е. канонический «первые q валидных»; `required_commitments = dealers.quorum::<M>()` (`:549-557`); недобор ⇒ `DkgFailed` (`:1439-1441`).
- [KNOWN] `dkg.rs:1804-1869` `Player::finalize(self, …)`: `MismatchedLogs` если `logs.info != self.info`; после select — `MissingPlayerDealing`, если выбранный лог содержит наш ack, а `view` не содержит dealing (`:1818-1823`); share = Σ dealings (свежий DKG, `:1853-1860`); output = `reckon` (Σ commitments, `:1635-1639`). Метод потребляет `self`.
- [KNOWN] `dkg.rs:1726-1758` `Player::resume`: replay `dealer_message` по msgs, затем `MissingPlayerDealing` если в logs есть валидный наш ack без соответствующего msg.
- [KNOWN] `dkg.rs:1477-1527` `Dealer::start(rng, info, me, share)` — полином из `rng` (`Poly::new_with_constant(&mut rng, degree, share)`), `degree = quorum(players)-1` (`:545-547`); `:1533-1546` `receive_player_ack` проверяет подпись, невалидный ack молча игнорируется (Ok); `:1551-1569` `finalize` → `TooManyReveals` при reveals > `max_faults(players)`.
- [KNOWN] `dkg.rs:581-594` `check_dealer_pub_msg`: степень commitment проверяется ТОЧНО (`degree_exact`) — для dealing-ов; `Output::read_cfg` (`:438-454`) степень полинома не пиняет (только `max_participants`), что и описывает caveat в `outcome.rs:79-95`.
- [KNOWN] `dkg.rs:106-112` (doc): при рестарте дилер обязан использовать seeded randomness — Fluent это реализует `dealer_seed_rng`.
- [KNOWN] `dkg.rs:125-158` (doc): под синхронией ≤ f reveals; под асинхронией до 2f — известный caveat библиотеки; Fluent ничего не добавляет поверх.
- [KNOWN] `buffered/engine.rs:312-323` `insert_message`: waiters оповещаются ДО проверки `latest.primary` (`:313-318`), кэшируется тело только от peer ∈ `latest.primary` (`:320-323`). Значит `subscribe(digest)`, выданный ДО прихода тела, получит тело даже от «ineligible» отправителя; `subscribe` ПОСЛЕ прихода — промахнётся (тело не в `items`). Комментарий `dkg_transport.rs:98-108` («drops every proposal body on the floor… verify parks on a body that is never cached») точен только для второго порядка событий — дрейф комментария.
- [KNOWN] `simplex/scheme/mod.rs:122-132` commonware сам добавляет суффиксы `_SEED/_NOTARIZE/_NULLIFY/_FINALIZE` к base namespace; Fluent-овый `seed_namespace = base ‖ "_BEACON_SEED"` (`bls/beacon.rs:29,39-44`) — отдельный домен, с commonware-овским `_SEED` не пересекается.
- [KNOWN] `vrf.rs:569-591` commonware VRF-схема: seed partial над `round.encode` под `namespace.seed` в каждом голосе; `:728-776` `assemble` → `recover_pair` при `≥ required::<M>()`; `:778-806` `verify_certificate` — batch verify vote+seed против `identity`. Fluent `CombinedScheme` повторяет форму, заменяя threshold-vote на multisig-vote и делая seed опциональным.

### Потребители (д), малые файлы
- [KNOWN] `spec_exec.rs:18` импортирует `beacon::{seed::Seed, verified_seed::VerifiedSeed, Randomness}` — `verified_seed::VerifiedSeed` НЕ в парадной двери `mod.rs` (продакшн-утечка внутреннего пути; сам тип `pub`). `:52-128` `report(Notarization)`: `n.certificate.seed` → `Seed`; `randomness.oracle_for(epoch)` → `VerifiedSeed::check` → `record_seed` / `NoKey ⇒ quarantine_seed` / иначе `error!` (σ, восстановленный ИЗ СОБСТВЕННЫХ проверенных partial-ов, не верифицируется под своим ключом — только лог, `:108-115`); затем `Command::SpecNotarized{digest, seed: Option<Seed>}` в executor — т. е. σ едет в executor ВТОРЫМ путём, минуя beacon.
- [KNOWN] `scheme.rs:57-78` `soft_enter_verifier(snap, chain_id, oracle: Option<Arc<dyn SeedOracle>>)` — `build_verifier` с оракулом из `Randomness::oracle_for`.
- [KNOWN] `engine.rs:97` `EpochEngineConfig.scheme: BlsScheme` приходит готовым из `signer_scheme`; `:80` `fallback_seed: [u8;32]` для `WeightedVrf` (`:288`); `:225` `register_scheme(epoch, scheme.clone)` — схема (с оракулом внутри) регистрируется в `EpochSchemeProvider` (outer.rs) — ещё одно место, где оракул/ключ эпохи «живёт» вне beacon (через `Arc<dyn SeedOracle>` внутри схемы).
- [KNOWN] `cert_follow.rs:108-110` `CertUpstream::get_epoch_artifact(epoch) -> Option<Vec<u8>>` default `None`; WS-handle переопределяет (node).
- [KNOWN] `byzantine.rs:32-33` `DkgOutcome` — `#[cfg(test)]` only; `forge_outcome_same_committee` test-only. Продакшн-роль `Equivocate` beacon не касается.
- [KNOWN] `fault.rs`, `plane_upstream.rs`, `order_block.rs`, `sync_metrics.rs`, `slasher/evidence.rs` — seed/PK/share не хранят. `order_block.rs:246-254,371-376`: биты 0/1/3 `beacon_flags` зарезервированы (retired `beacon_outcome`/`parent_seed`/`dkg_logs`), decode отвергает — подтверждает «в теле блока seed'а нет». `sync_metrics.rs:132-139` `crash_recover_stray_seed` — счётчик σ в локальном seed-store на beacon-неактивном раунде при crash-replay (executor читает seed store при replay, см. executor). `slasher/evidence.rs:227-246` evidence отбрасывает seed-половину `CombinedSignature`; `:503-519` pre-submit verify vote-only, т. к. полином эпохи «not recoverable at slash time».

### Потребители (д), большие файлы
- [KNOWN] `application.rs:18` `beacon::seed::Seed` (продакшн, путь в подмодуль). `:257` `randomness: Arc<dyn Randomness>`; `:329` `dkg_height_tx: Option<mpsc::Sender<u64>>` — третий фидер часов DkgActor (тип plain u64, но поле ядра знает про «beacon plane height channel»); `:1027-1034` `report(Update::Tip)` шлёт высоту в него. `:1101` `DerivedBlock::beacon_active`; `:1135` `PrefixSeedMissing`; `:1154-1199` `derive_with_visibility_retry(seed: Option<Seed>)` и `DerivedBlockBuilder::derive_and_execute(.., seed: Option<Seed>)` — `Seed` уходит в node-крейт как аргумент derive. Хранения seed/PK нет.
- [KNOWN] `epoch_manager.rs:17-19` импорт `agreement_partition, constant_fallback_seed, witness_fallback_seed, PinEffort, Randomness, ShareProbe, SignerVerdict`. `:433 dkg_agreements: BTreeMap<Epoch, Handle<()>>` — ядро ВЛАДЕЕТ хэндлами agreement-инстансов beacon-а (св. 1/3: жизненный цикл DKG-плоскости управляется epoch_manager); `:318-366 prune_agreements` — ядро удаляет журнальную партицию beacon-а `agreement_partition(prefix, epoch)` (св. 2: чужой модуль чистит диск beacon-а); `:828-865` intake/abort/replace; `:711-714` SafetyHalt abort. `:158-169 boundary_base`: `mandatory_at(prev)` → `terminal_seed_at(round)` → `witness_fallback_seed` → `[u8;32]` → `EpochEngineConfig.fallback_seed` (производное от σ едет в конфиг engine, не хранится). `:1071,1127 share_probe`; `:1198 signer_scheme`; `:1545-1561 register_soft_entered` с `oracle_for`; `:1667-1702 repair_keyless_schemes` `ensure_key(Local)`→`ensure_key(Thorough)`; `:666-673 participation_edge/key_edge`; `:997-999 observe_epoch(epoch, highest_entered)`. Дрейф комментариев: `:1049` «W1 `insert_group_key`» — такого символа нет в дереве (grep пуст); `:672,:1287` ссылаются на `beacon::keys` (внутренний модуль).
- [KNOWN] `cert_inlet.rs:18` `beacon::{keys::InvalidSeed, verified_seed::VerifiedSeed, PinEffort, Randomness}` — два внутренних пути в продакшне. `:375-377 CachedScheme{scheme}` — per-epoch BlsScheme с оракулом внутри (Arc<dyn SeedOracle>, ключ читается live — не копия PK). `:693 ensure_key(epoch, Local)` НА КАЖДЫЙ серт; `:731 oracle_for(epoch)`; `:824-826` `CERT_VOTE_ONLY_ADMISSIONS` при `!key_known && mandatory_at`; `:866 capture_certificate_seed`; `:888 observe_cert(epoch)`. `:3012-3043 capture_certificate_seed`: `oracle_for` None ⇒ return; `VerifiedSeed::check` Ok⇒`record_seed`, NoKey⇒`quarantine_seed`, Invalid⇒`on_invalid_seed` {Quarantine⇒quarantine, RefuseLoud⇒`error!`, RefuseQuiet⇒drop}. `:3153-3155` UpstreamResolver: capture только после `handler.deliver(..)==true`. Дрейф док-комментариев: `:329-335` ссылаются на `beacon::actor::CommitteeFor`/`DkgActor`; `:404-413` на `BeaconKeys::get_pk` ladder, `beacon::keys::AgreedKeys` (внутренние имена в док-комментах ядра). `:3007-3011` признание: σ через gap-door не прунится до следующего live-серта.
- [KNOWN] `outer.rs:257-259 EpochSchemeProvider{map: BTreeMap<Epoch, Arc<BlsScheme>>}` — реестр схем; каждая схема несёт `Option<Arc<dyn SeedOracle>>` (`:360 is_beacon_active`), т.е. ОРАКУЛ (а через него ключ эпохи по ссылке) живёт вне beacon как часть схемы. Retention `SCHEME_RETENTION_EPOCHS=8` (`:373-375`, lib.rs:26). `:536 randomness`, `:589 dkg_height_tx`, `:690 agreement_intake` — три beacon-поля в OuterBuilder. `:750` OuterEngine держит `randomness` только ради `UpstreamResolver` (`:1445,:1676`). `:1120-1121 spec_exec::Mailbox::new(executor_mailbox, randomness)`. `:1192-1193` soft_enter_span берёт `oracle_for(epoch)`. Комментарий `:1119` «Also writes the recovered seed into `seed_store`» — имя внутреннего стора.
- [KNOWN] `cold_start_jump.rs:685 scheme_at(epoch, landing_hash, None)` — oracle=None на landing: vote-only верификация прыжка (принятый остаток). Seed/PK не хранит.
- [KNOWN] `dpos.rs:10` `beacon::{Randomness, Seed}`; `:116 SEED_JOURNAL_PARTITION="beacon-seed-ordinal"`, `:119 KEY_JOURNAL_PARTITION="beacon-key-ordinal"`, `:125 pub ARTIFACT_JOURNAL_PARTITION="beacon-artifact-metadata"` — имена партиций beacon-а объявлены в dpos.rs (док-коммент `:101-102` ссылается на `beacon::certify::SeedStore`, `:122` на `beacon::artifact::ArtifactStore`). `:525-592 ReplaySeedSource/replay_seed_source`: `mandatory_at` → `seed_for(round)`; stray-seed на inactive-раунде считается и игнорируется (`crash_recover_stray_seed`). `:600-608 seed_from_cert` — извлечение σ из finalization с round-pin; `:610-706 recover_replay_seed`: store → локальный cert-архив (БЕЗ проверки σ) → upstream (BLS-проверенный cert, σ не проверяется под PK_E) → `Unavailable` ⇒ defer/fatal (`:846-858`). Т.е. crash-replay берёт σ из сертификата ВНЕ beacon и передаёт в derive, минуя `VerifiedSeed` (св. 1/2: второй путь получения seed). `:1066-1102 SharedBeaconPlane{oracle, randomness, 5 mux, vote_backup, tombstones, plane_clock, dkg_height_tx}`; `:967 beacon_plane`, `:979 agreement_intake` в DposLayerConfig. `:1424 artifact_bytes: Option<ArtifactSource>` в DposLayerHandle. `:2414-2417` комментарий про `beacon_share_resolver`/`beacon::carry` (имена внутренних модулей). `:2722-2724 build_verifier(.., None)` cold-start register без оракула; `:3421-3438 frozen_dkg_qual` follower-копия; `:3444-3459 CommitteeSource`; `:3463-3474 ArtifactFetch` через `CertUpstream::get_epoch_artifact`; `:3495-3507 for_follower(FollowerRandomnessConfig{chain_id, committees, dkg_qual, fetch})` → `FollowerBeacon{randomness, artifact_bytes, fetch_handle}`; `:3951 follower_artifact_fetch` supervised. `:2788-2791` комментарий: «Both journal writers moved into `beacon::build`» — drain пуст.
- [KNOWN] `lib.rs:26 SCHEME_RETENTION_EPOCHS=8` — «одна политика с двумя исполнителями по разные стороны randomness-поверхности» (реестр схем и derived-tiers BeaconKeys). `:29 pub mod beacon`; `:81-85` re-export `SharedBeaconPlane` и др.
- [KNOWN] `executor.rs:134,199,564,605,2565,2963,3570` `crate::beacon::seed::Seed` (продакшн, путь в подмодуль, 7 мест). Executor ХРАНИТ σ в RAM вне beacon: `:564 Deferred.seed: Option<Seed>` (парк guard #2 несёт σ), `:199 ParkedSpec.seed` (`parked_spec: BTreeMap<u64, ParkedSpec>` :936), `:160 SpecExecuted.seed_round` (только раунд), `:134 Notarized.seed` (сообщение mailbox). Это копии на время жизни блока в pipeline, не журнал; но св. 2 формально нарушено (σ живёт в executor-структурах, а `spec_execute :2624-2650` при mismatch раунда читает `seed_for(canonical)` — второй consumer seed по значению). `:2915-2938 seed_at_own_round`: `mandatory_at` → `seed_for(round)`; stray-seed на inactive-раунде ⇒ счётчик+warn. `:1167 seed_edge` — notify-арм `:1483-1495`. `:570-600 HeldForSeed`/`SEED_HOLD_STALL_THRESHOLD=60s` — детектор, не дедлайн (`:2864-2890`). `:3612-3631` gap-walk: `PrefixSeedMissing` ⇒ парк. `:3791-3795 DerivedBlock::beacon_active` → `seed_active/digest_fallback` счётчики. Executor читает у Randomness ровно 3 метода: `mandatory_at`, `seed_for`, `seed_edge` (`:700-709`). Seed в executor НЕ верифицируется — доверие store.

### Node-крейт (е)
- [KNOWN] `node/src/dpos.rs:1943-1975 beacon::build(ctx, BeaconConfig{chain_id, peer_keypair, bls_keypair, share_dir=<datadir>/beacon, share_seal_key, peers: oracle, beacon_channel, resolver_channel, vote_mux, cert_mux, resolver_mux, bodies_mux, committee_for, committee_pair_for, committee_source, dkg_qual_at, dkg_qual_probe, heights: dkg_height_rx, plane_clock, geometry: future, partition_prefix: ""})` — узел собирает для beacon 4 staking-замыкания (`:1410-1543`) + сеть/муксы + канал высот; `:1361 beacon_dir`. Обратно узел получает `Beacon{dkg_handle, resolver_handle, seed_promoter_handle, agreement_launcher_handle, artifact_writer_handle, write_back_handle, key_writer_handle, seed_writer_handle, agreement_intake, randomness, artifact_bytes}` (`:1986-2016`) — 8 хэндлов внутренних задач beacon-а поднимаются в супервизор узла под именами `dkg/beacon_resolver/seed_promoter/agreement_launcher/agreement_write_back` (`:830-840`) и в drain (`:856-873`). Узел ЗНАЕТ структуру задач beacon-а (св. 3, слабое: хэндлы, не типы).
- [KNOWN] `node/src/dpos.rs:1557 dkg_height_tx` (mpsc 256) с тремя фидерами: poller `:1645,:1675` (`fin + K`), inlet tee `:817`, FluentApp (`outer.rs`). `:1058 live_height` atomic — `committee_for` читает при `max(fin, live)`. `:1072 marshal_slot: OnceLock<MarshalMailbox>` — комментарий `:791-793` говорит «DkgActor's deferred marshal READ handle … demote-heal recompute reads pinned boundary outcomes» — но в `BeaconConfig` (`:1945-1973`) marshal_slot НЕ передаётся; он идёт только в `plane_upstream::new_bridge` (`:1801-1802`). Дрейф комментария `:791-794` и `:1065-1072`.
- [KNOWN] `node/src/dpos.rs:1977-1979` «dkgQual set DETERMINISTICALLY by the contract at commitEpochCommittee (= committee[e] != committee[e−1])» — комментарий; сам контракт не открывал.
- [KNOWN] `node/src/dpos.rs:2178-2179` и `cert_follow/mod.rs:118-120`: «deriver computes prev_randao = H(seed) … no on-chain PK_E read» — подтверждено `derive.rs:78-87 resolve_prev_randao` → `prev_randao_from_seed(s)`; `derive.rs:10` импорт `beacon::{prev_randao_from_seed, Seed}` (парадная дверь). Seed в node не хранится; `DerivedExecution.beacon_active: Option<bool>` (`:37`).
- [KNOWN] `node/src/consensus_rpc/state.rs:62 pub use beacon::ArtifactSource`; `:68 artifacts: Arc<RwLock<Option<ArtifactSource>>>`; `:204-210 get_epoch_artifact(epoch)` — RPC `consensus_getEpochArtifact` отдаёт байты артефакта (PK_E + логи + confirms + Finalization) вовне. Это единственный «RPC key» — отдаётся wire-encoded `AgreedArtifact`, не PK_E отдельно.
- [KNOWN] `node/src/cert_inlet.rs:81-88 spawn_cert_inlet(.., tee: LiveFrontierTee, randomness)` — `with_randomness(randomness)` (`:113`); комментарий `:64-66` «ladder's boundary-walk rung deleted 2026-08-19». `cert_follow/mod.rs:251-253 set_artifact_source(handle.artifact_bytes)`; `:179` «NO DkgActor, NO beacon oracle, NO signer» на follower. `consensus.rs`, `importer.rs` — beacon не касаются.
- [KNOWN] `bins/fluent/src/main.rs:121-122` — beacon всегда включён, флагов нет (комментарий + тест `:441`).

### Стенд (ж)
- [KNOWN] `testbed/stand.rs:14-22` импортирует `beacon::{actor::{CommitteeFor, CommitteePairFor}, carry::DkgQualProbe, seed::Seed, surface::StaticRandomness, ArtifactSource, BeaconConfig, CommitteeSource}` — тест использует 4 внутренних пути (actor, carry, seed, surface). `:1782-1911` три режима: `Beacon::Static` → `StaticRandomness::build(CHAIN_ID, all_validators_snapshot)` (`surface.rs` тестовый вход), `Role::AbsentBeacon` → `beacon::absent`, `Beacon::Live` → `beacon::build(BeaconConfig{.., share_dir: share_root/node{i}, share_seal_key: None, partition_prefix: "node{i}-", geometry: et.frozen_geometry})` (`:1859-1889`). `:1912` `dkg_height_tx` только при Live. `:1183-1191` артефакты собираются через `ArtifactSource` (что отдаёт `consensus_getEpochArtifact`). `:341 FakeChain.seeds: BTreeMap<u64, Option<Seed>>` — стенд записывает σ, которую executor передал в derive (наблюдаемое, не хранилище узла).
- [KNOWN] `testbed/byzantine_roles.rs:53-62` импортирует `beacon::{ceremony::info_for, dkg_msg::{DealerReveal, DkgBody, DkgMsg}, keys::InvalidSeed, seed::Seed, surface::{PinEffort, Randomness, ShareProbe, SignerVerdict}, verified_seed::VerifiedSeed, wire::BeaconMessage}` — 7 внутренних модулей (тест, feature-gated). Роли: `TwoRevealSender` (`:139-342`) — перехват `DkgBody::Reveal` на BEACON_CHANNEL, второй `Dealer::start::<N3f1>` над тем же `Info` (`:177-189`), жертва получает L2; `WithholdingRandomness` (`:351-470`) — 17-метровая обёртка `Randomness`, `signer_scheme` пересобирает схему через `build_signer(.., Some(oracle_for))` с verify-only оракулом ⇒ `sign_partial` None ⇒ нет голоса; `ForgedSeedProducer` (`:573-937`) — `ForgeMode::{Watch, SeedSlot (R-008), InflatedLatest (R-004), LyingLatest (R-001 var Б), WrongHeightFinalized (R-009)}`, `FORGE_WINDOW=64..=70` (`:479`), подмена σ-слота при неизменном multisig (`:653-713`), самопроверка на wire-байтах.
- [KNOWN] `testbed/fakes.rs:7` `beacon::seed::Seed`; `:726` `beacon::seed::prev_randao_from_seed` в `FakeDeriver` (derive = `sealed_at(parent, h, keccak(digest ‖ prev_randao(seed)))`). `FakeStaking::dkg_qual` (`:962-980`) реализует правило контракта `dkgQual[e] = committee[e] != committee[e-1]`, epoch 0 ⇒ `(false, true)`. `:1211-1322 JumpElSync` — `sync_to` без σ (EL sync несёт `prev_randao` в заголовке). `:1282-1287` [ГИПОТЕЗА] в самом стенде о reth `Invalid` vs `SYNCING`.
- [KNOWN] Что стенд НЕ покрывает по beacon (из его же док-комментариев): нет CertInlet на стенде (`stand.rs:1601-1610` `live_height` тиится ДО verify); `ReJump::rotate = None` (`:1747`); journals под общим in-memory Storage с префиксом `node{i}-`; шары в реальном temp-каталоге; `Role::AbsentBeacon` моделирует «валидатор без beacon-модуля», не «член, не сдавший deal».

## §3. Инварианты, которые модуль обязан держать

| # | Инвариант | Держит ли код сейчас | Где ломается |
|---|---|---|---|
| I1 | Один раунд — одна σ у всех честных узлов (уникальность порогового BLS под одним `PK_E`) | Да при ≤ f: криптографически (`bls/beacon.rs:161-167`, `CW vrf.rs:728-776` та же форма). Две разные σ за раунд возможны только под двумя разными ключами | `certify.rs:201-209` — конфликт σ логируется `error!`, первая остаётся; при > f (два артефакта за эпоху) — молчаливая расходимость, см. E5-08 |
| I2 | Один `PK_E` у всех честных узлов эпохи | Да при ≤ f: `select` — чистая функция pinned-набора (`CW dkg.rs:1425-1443`), набор согласован кворумом (`dkg_agree.rs:1295-1363`), `group_key` пересчитывается каждым голосующим | Два разных `Agreed` — `debug_assert!` (`keys.rs:339-342`); два quorum-артефакта — `warn` (`artifact.rs:819-835`); локальный `LocalDkg` под живой эпохой без арбитража в `verify_seed` (`oracle.rs:249-273`, E5-09) |
| I3 | Каждый честный член `committee[E]` держит share к `epoch_start(E)` при ≤ f | Нет. Один византийский дилер делит комитет по хэшу лога (R-002, `ceremony.rs:406-422`, стенд `a_dealer_with_two_logs_…`); потеря тела предложения не переагрируется (R-026, `dkg_engine.rs:390-397,584-609`); `NoFile` после дедлайна переизлагает (R-036, `actor.rs:1660-1661`) | там же |
| I4 | Любой узел, верифицирующий сертификаты эпохи E, может получить `PK_E` (в силе) в ограниченное время | Нет. Ниже фронтира — sweep (`epoch_manager.rs:1667-1702`); на фронтире для НЕ-члена — вызывающего кода нет (`drive_recompute` только для членов, `actor.rs:2084-2163`; `soft_enter` зовёт только `oracle_for`, `epoch_manager.rs:1545-1561`) — R-121/R-122 | `epoch_manager.rs:1018-1023,1677`; follower: только через upstream RPC (`dpos.rs:3463-3474`) |
| I5 | Живучесть seed при ≤ f | Граница протокола, не свойство кода: порог `quorum(n) = n − f` (`bls/combined_scheme.rs:387`) означает, что каждая σ требует ≥ 1 partial от византийского при f византийских. Принято входным ограничением | Код хуже границы ровно там, где ломает I3 (честный член без share) |
| I6 | Рестарт в любой точке ⇒ определённое состояние | Нет. Пять сторов warn-and-continue (E5-07); replay неоднороден: seed/key journal init — `?` (фатал), share/artifact/DKG-журнал — warn+skip, `Torn` — sit-out; key journal может содержать проигравший `Agreed` (R-068) | `plane.rs:479,529-534,567-573,586-591,623`; `share_state.rs:330-366,575-621`; `key_journal.rs:170-181,280` |
| I7 | В `seed_for` попадает только σ, проверенная под `PK_E` (правило 34b) | Да для пути записи (тип `VerifiedSeed`, `verified_seed.rs:43-52`); нет для crash-replay (σ из сертификата без проверки, `dpos.rs:600-608,657-704`) и журнала (по замыслу, `:75-77`) | `dpos.rs:610-706` |
| I8 | Один владелец каждого факта | Нет: PK_E — 5 носителей (`CeremonyStore`, `BeaconKeys` + key journal, `ArtifactStore` + Metadata, share-файл v2, `log_store` для логов); σ — `SeedStore` + seed journal + архив marshal; очистка партиций — два модуля | §2.2, §2.3 |

## §4. Находки (не отфильтровано)

Формат: id · тяжесть · категория (св. 6) · механизм с якорями · последствие · уверенность · чем пытался опровергнуть · дубль/связь с R-nnn · покрытие П-2/П-3/П-9.

**E5-01 · SERIOUS · нарушение границы модуля.** Проверка σ и вердикт «запись/карантин/отказ» выполняются ПОТРЕБИТЕЛЯМИ: `spec_exec.rs:97-125` и `cert_inlet.rs:3012-3043` берут `oracle_for`, зовут `VerifiedSeed::check`, затем `record_seed`/`quarantine_seed`/`on_invalid_seed`. Beacon не «принимает сертификат и отдаёт seed» — он отдаёт оракул и принимает уже решённое. Последствие: два ингресса с разной обработкой `Invalid` (`error!` в spec_exec `:108-115`, `on_invalid_seed` в inlet) и третий путь в crash-replay (E5-03). [KNOWN]. Опровержение: искал третьего писателя `record_seed` — только эти два + журнал (`verified_seed.rs:75-77`); подтверждает, что вердикт всегда снаружи. Новая (класс не в реестре). П-2 удаляет `record_seed` вместе с `SeedStore`, но заменяет чтением архива marshal — граница остаётся у потребителя.

**E5-02 · MINOR · нарушение границы модуля.** Внутренние пути в продакшне: `VerifiedSeed` (`spec_exec.rs:18`, `cert_inlet.rs:18`), `InvalidSeed` (`cert_inlet.rs:18`), `seed::Seed` (9 мест, §2.6 п.3); `pub(crate)`-швы `frozen_dkg_qual`/`CommitteeSource`/`agreement_partition`/`absent_unregistered`. [KNOWN] по grep. Опровержение: `mod.rs:16-46` заявляет «two doors only» — заявление не соответствует. Новая. Не покрыто П-*.

**E5-03 · MODERATE · нарушение границы модуля + дефект при ≤ f.** Crash-replay берёт σ из локального архива финализаций (`dpos.rs:657-668`) и из BLS-проверенного upstream-сертификата (`:669-704`) через `seed_from_cert` (`:600-608`) — без `VerifiedSeed`, минуя beacon. Локальный архив принят как доверенный «по симметрии с телами блоков» (комментарий `:613-622`); upstream-σ проверяется только multisig-ом, не под `PK_E`. Последствие: при ≤ f честный upstream даст верную σ (уникальность), но ложный σ-слот в чужом сертификате (R-008-класс) уйдёт в derive без проверки под ключом — restart-fork на одном узле. [KNOWN] код; достижимость подмены — [LIKELY] (стенд R-008 показал отравлённый архив у follower, `E3-3-ROLES-1.md`). Опровержение: искал вызов `oracle_for` в replay — нет. Связано R-020, R-008, R-032 (правило 32 прямо описывает это как принятое). П-2 делает этот путь основным (Д-9) — усиливает, не закрывает.

**E5-04 · MODERATE · нарушение границы модуля.** `epoch_manager` владеет хэндлами agreement-инстансов (`epoch_manager.rs:433,828-865`), абортит их на SafetyHalt (`:711-714`) и УДАЛЯЕТ партиции журнала beacon-а `dkg_epoch_{E}` (`prune_agreements`, `:318-366`, join перед sweep). Последствие: жизненный цикл второй simplex-плоскости решается ядром; beacon не знает, когда его инстанс убит; sweep по номеру эпохи «в окне 8» — костыль под отмену. [KNOWN]. Опровержение: искал sweep внутри beacon — нет (`dkg_engine.rs:267-442` удаляет партицию только на happy-path). Новая. П-9 не касается.

**E5-05 · MINOR · нарушение границы модуля.** Узел супервизирует 8 внутренних задач beacon-а по именам (`node/dpos.rs:830-840,856-873`), `Beacon` отдаёт 8 хэндлов трёх классов плюс приёмник `agreement_intake` (`plane.rs:384-420`). Последствие: ошибка классификации (drain vs supervised) — тихая потеря или ложный фатал; узел знает структуру модуля. [KNOWN]. Новая.

**E5-06 · MINOR · оверинженеринг/костыль.** Часы DKG собираются из трёх фидеров снаружи (`node/dpos.rs:1645,1675`, `cert_inlet.rs:899`, `application.rs:1027-1034`) в один `mpsc(256)` с clamp внутри (`actor.rs:1145-1272`) и счётчиком дропов. Последствие: «max трёх источников» реализован каналом с потерями; поллер `fin + K` — вывод ordering tip из EL-finalized, тогда как ordering tip доступен напрямую. [KNOWN]. Опровержение: комментарий `08:1035-1058` обосновывает третий фидер halted-execution — при одном фидере (marshal tip) первые два избыточны [ГИПОТЕЗА]. Новая. Не покрыто.

**E5-07 · SERIOUS · непредсказуемое состояние.** Ошибка записи — warn-and-continue в пяти сторах: share-файл (`actor.rs:1006-1016`, `share_state.rs:307-319`), DKG-журнал (`share_state.rs:533-543`), key journal (`key_journal.rs:280,291-297`), seed journal (`seed_journal.rs:430-451`), artifact (`artifact.rs:466-471,668-677`). Память авторитетна. Последствие: узел подписывает до рестарта, после — verify-only без причины (R-021); `Agreed` в RAM, не на диске ⇒ после рестарта `NoUsableMint`; σ в RAM ⇒ hold после рестарта. [KNOWN]. Опровержение: искал `SafetyHalt`/fatal по ошибке диска в beacon — нет; только `secret_store::write_mode_0600` атомарен (`bls/secret_store.rs:110-134`). Дубль R-021, связано R-020. П-3 (частично: «`validate_share_on_poly` перед adopt» не про диск).

**E5-08 · MODERATE · непредсказуемое состояние (при > f — граница, но исход не объявлен).** Три свидетеля форк-класса обработаны как лог: два разных `Agreed` за эпоху — `debug_assert!` (`keys.rs:339-342`, release: первый остаётся), два quorum-артефакта — `warn` (`artifact.rs:819-835`), две σ за раунд — `error!` (`certify.rs:201-209`). Последствие: узел продолжает работать на одном из двух «фактов» без явного состояния; key journal пишет проигравшее значение (R-068). [KNOWN]. Опровержение: искал `SafetyHalt::engage` из beacon — нет. Дубль R-053, R-068. П-3 «first-wins» сохраняет молчание.

**E5-09 · MODERATE · дефект при ≤ f [ГИПОТЕЗА по достижимости].** W1 пишет `LocalDkg` под ЖИВОЙ эпохой (`surface.rs:1743`, если `attested(epoch)` пуст — на стабильном комитете `Agreed` лежит под минтом, не под живой), а `BeaconOracle::verify_seed` читает `cached_only(live)` любого provenance без арбитража (`oracle.rs:249-273`; арбитраж только в `with_material` `:153-168`). Последствие: если локальный минт расходится с сетевым, а артефакта минта у узла нет, каждый честный σ получает `Invalid` ⇒ `verify_certificate` = false ⇒ узел не принимает ни одного сертификата эпохи. Достижимость: расходящийся минт при согласованном pinned-наборе невозможен (I2); остаётся `MissingPlayerDealing`/recompute с чужим scope — [ГИПОТЕЗА]. Опровержение: `resolve.rs:41-49` tripwire против `attested(minted_at)` — не срабатывает, если `Agreed` отсутствует. Связано R-069, soak 2026-07-14. П-3 удаляет W1 — закрывает.

**E5-10 · MINOR · непредсказуемое состояние.** `on_invalid_seed` судит по `Agreed` под ЭТОЙ эпохой (`keys.rs:416-431`), `Agreed` хранится под минтом ⇒ на стабильном комитете всегда `Quarantine`, `RefuseLoud` недостижим; в `cert_inlet.rs:3034-3040` `RefuseLoud` = только `error!`, data fault не считается, ротации upstream нет. Дубль R-069 (+ половина R-008). П-2 удаляет `on_invalid_seed`.

**E5-11 · MODERATE · дефект при ≤ f.** Потеря тела согласованного предложения ⇒ `dkg_agree_body_lost`, артефакта нет, и повторного инстанса в процессе не бывает (`dkg_engine.rs:390-397`; `started` `:584-587,608-609`); комментарий «re-agree on a fresh instance» ложен. Heal — pull из `drive_recompute` после начала эпохи. Дубль R-026. П-9 («потеря тела ⇒ немедленный pull_artifact»).

**E5-12 · MINOR · непредсказуемое состояние.** `finalize_over_pinned` Err ⇒ церемония остаётся с `can_finalize == false` до sweep (`actor.rs:1567-1574`, `ceremony.rs:988-997` — `expect("can_finalize gates this")` паника при нарушении контракта). Состояние без выхода внутри процесса, не названо. Связано R-025. П-9 автомат.

**E5-13 · NIT · дефект при ≤ f (смягчённый).** Recompute принимает share по `validate_share_on_poly` без `verify_seed(σ₀, PK_E)` (`actor.rs:2178-2266` против требования `outcome.rs:79-95` «callers MUST run both»). Смягчение: `outcome` берётся из quorum-артефакта, степень полинома пересчитана голосующими. Новая (дрейф контракта комментария). П-3 требует `validate_share_on_poly` — оставляет полуправило.

**E5-14 · BLOCKER (по I3) · дефект при ≤ f.** Идентичность лога по дилеру: `record_checked_log` first-wins по `PeerPubkey` (`ceremony.rs:406-422`), `ingest_signed_log` отбрасывает второй лог того же дилера (`:628-646`), `scoped_pinned_logs` ⇒ `missing` при несовпадении хэша (`:870-895`), `fetch_missing_logs` не перезапрашивает записанного дилера (`actor.rs:1993-2012`). Один византийский дилер оставляет честного члена без share на эпоху и carry-forward; при удержании partial-ов — остановка (та часть — граница I5). [KNOWN], воспроизведено стендом. Дубль R-002. П-9.

**E5-15 · MODERATE · дефект при ≤ f.** `NoFile` после дедлайна печати ⇒ `start_fresh` (`actor.rs:1660-1661`) — тот же полином (`ceremony.rs:235-240`), другой набор ack/reveal ⇒ второй валидный лог = честная эквивокация. Дубль R-036. П-9.

**E5-16 · NIT · оверинженеринг/костыль.** Детерминированный полином дилера от `(ключ, номер эпохи)` (`ceremony.rs:235-240`) переиспользуется при любом составе комитета того же номера. [ГИПОТЕЗА] о достижимости повторной церемонии с тем же номером (только пересоздание сети с тем же chain_id). Снята как дизайн (PLAN §8 п.10). Связано R-055.

**E5-17 · MINOR · непредсказуемое состояние.** Закрытие `resolver_rx` ⇒ resolver отключён навсегда, процесс живёт (`actor.rs:828-831`); `LogHandler::deliver` при ушедшем акторе возвращает `false` ⇒ пир исключается из resolver навсегда за чужую смерть (`log_resolver.rs:385-401`). Связано R-067. Новая (вторая половина). П-9 не касается.

**E5-18 · MODERATE · непредсказуемое состояние.** Два разных floor-а очистки для одного факта: диск удаляет share-файлы ниже `max{e ≤ now}` (`share_state.rs:677-683`), RAM держит по `ceremony_retain_floor` (`actor.rs:224-227`), корректность которого зависит от непроверенного инварианта «минт ⇒ бит `dkgQual` выставлен» (`actor.rs:190-223`). Дубль R-017. П-3 «одно правило вытеснения».

**E5-19 · MINOR · оверинженеринг/костыль.** `chain_key_epoch` со свежим memo на каждый вызов (`carry.rs:74-76`) в `select_carry_scheme` (`:158-174`) — полный проход `(BOOTSTRAP, E]` на каждый `share_probe`/`signer_scheme`; вторая, мемоизированная копия — `chain_key_epoch_memoised` (`:110-153`). Дубль R-056/BB-3. П-3 удаляет carry.

**E5-20 · MODERATE · непредсказуемое состояние.** `geometry` = `None` ⇒ `error!`, `DkgActor` не стартует, процесс живёт (`plane.rs:684-693`). На валидаторе это узел без DKG, который никогда не сдаст share и не увидит этого иначе как по `-1` в lag-gauge. Комментарий полагается на «валидатор упал раньше» — не проверяется. [KNOWN]. Новая. Не покрыто.

**E5-21 · NIT · оверинженеринг/костыль.** `share_probe` и `signer_scheme` инкрементируют `engine_demoted_no_polynomial` на каждый вызов (`surface.rs:1961`, `:2015`), а probe идёт на каждом reconcile-ребре ⇒ счётчик «событий» растёт на каждом тике verify-only узла. Новая. Не покрыто.

**E5-22 · MINOR · оверинженеринг/костыль.** Два реестра метрик в одном модуле: `BeaconMetrics` на commonware (`metrics.rs:172`) и `metrics::counter!` в `resolve.rs`, `key_journal.rs`, `seed_journal.rs`, `artifact.rs`, `keys.rs`, `surface.rs`. Дубль R-070 (смягчена compose-ом, не в коде). Э8.

**E5-23 · MINOR · оверинженеринг/костыль.** Семь `Option`-швов в `DkgActor` (`actor.rs:335-521`: `resolver`, `resolver_rx`, `pinned_rx`, `agreement_tx`, `artifacts_rx`, `outcome_at`, `pull_artifact`; ещё `committee_pair_for`, `plane_clock`) — «`None` ⇒ ветка инертна»; продакшн-сборка (`plane.rs:674-720`) всегда ставит все, тесты — подмножества. Поведение зависит от состава швов; `DKG_SETTLE_BLOCKS` уже удалили по той же причине (`08:1136-1140`). Дубль BB-10. П-9 не называет.

**E5-24 · MINOR · непредсказуемое состояние.** Предусловие «`peers` покрывает `committee[target]`» заявлено и «checked nowhere» (`dkg_transport.rs:98-108`, `dkg_engine.rs:513-519`); `buffered` кэширует тело только от пира из `latest.primary` (`CW buffered/engine.rs:320-323`), но ожидающих оповещает до проверки (`:313-318`) — комментарий Fluent точен лишь для порядка «тело раньше subscribe». Последствие: при tracked-set без `committee[E+1]` — `verify` паркуется навсегда, инстанс молчит. [KNOWN]. Связано R-037, R-026. Новая (дрейф + непроверенное предусловие).

**E5-25 · — · граница протокола при > f (принято).** Эквивокация внутри agreement-плоскости unslashable (`dkg_agree.rs:1600-1648`, reporter не соединён со slasher-ом). Решение Д-6. Документировать, не чинить в Э5.

**E5-26 · MODERATE · оверинженеринг/костыль.** PK_E живёт в пяти носителях: `CeremonyStore` (Output), `BeaconKeys` + key journal, `ArtifactStore` + Metadata, share-файл v2 (копия артефакта, `share_state.rs:94-103`), `log_store` (логи) — три формата хранения (§2.2). Дубль B-7, BB-6, связано R-020. П-3 сводит к артефакту (оставляет share-файл копию и `CeremonyStore` Output — недостаточно).

**E5-27 · MINOR · оверинженеринг/костыль.** Две продакшн-реализации `Randomness` (`PlaneRandomness`, `FollowerRandomness`) + `Absent` + тестовые `StaticRandomness`/`Canned`; `for_keys`/`for_seeds`/`absent` — `pub` без продакшн-вызывающих (`surface.rs:252-268,581-624`; вызовы только в тестах, §2.6 п.11); follower регистрирует `BeaconMetrics` второй раз (`follower.rs:124-125`). Дубль B-6, BB-9. П-2 (B-6) частично.

**E5-28 · NIT · оверинженеринг/костыль.** `SeedStore` — четыре карты, из них `waiters`/`wait_for`/`prune_waiters` мёртвые после отмены by-round запроса (`certify.rs:89-96,405-436`); `terminal` pin хранит «highest seen» (`:366-378`) и сам код запрещает читать его иначе как по точному раунду (`terminal_at`, `:390-396`) — отдельная карта ради одного правила вытеснения. Дубль BB-1, BB-11. П-2.

**E5-29 · MINOR · непредсказуемое состояние.** Три разных фронтира прунят одни и те же карты: `observe_epoch(entered)` (`surface.rs:2180-2201`), `observe_cert(cert epoch)` (`cert_inlet.rs:888`; follower `follower.rs:513-518` на каждый серт), актор — по `now`. Последствие: inlet впереди менеджера удаляет `Carried`, который executor ещё деривит (R-090). Дубль R-090. П-3 «один ретеншн».

**E5-30 · MODERATE · нарушение границы модуля.** Beacon-активность эпохи (наличие оракула в схеме) решают четыре производителя ВНЕ модуля: `register_soft_entered` (`epoch_manager.rs:1545-1561`), span (`outer.rs:1192-1193`), `engine.rs:225` и `cold_start_register` с `None` (`dpos.rs:2722-2724,3629-3636`); реестр держит схему с оракулом (`outer.rs:258`) и защищается `ORACLE_DROP_REFUSED` (`:360-368`) — «defence in depth against a producer that does not exist». Последствие: свойство «эпоха проверяет σ» не принадлежит beacon-у; cold-start окно vote-only. [KNOWN]. Связано R-075, R-045. Новая. Не покрыто П-*; территория П-1 (4.1).

**E5-31 · MINOR · оверинженеринг/костыль.** Два маршрута доставки артефакта с раздельным кодом: plane — resolver + `ArtifactPull` (`artifact.rs:704-1011`), follower — RPC `get_epoch_artifact` + `run_fetcher` (`follower.rs:200-337`, `dpos.rs:3463-3474`); валидатор с upstream артефакт по RPC не берёт. Дубль B-6. П-2 (B-6).

**E5-32 · MINOR · дефект при ≤ f.** R-121/R-122: для не-члена живой эпохи никто не запрашивает артефакт (`drive_recompute` — только члены, `actor.rs:2084-2163`; sweep — только ниже фронтира, `epoch_manager.rs:1677`; `soft_enter` — только `oracle_for`); span, поднявший фронтир, sweep не будит (`epoch_manager.rs:752-757` зовёт `reconcile_live`, который выходит при не-live). Последствие: нулевое пересечение — молчаливая остановка (стенд C7). Дубль R-121, R-122. П-3 не добавляет вызывающего; в Э5 добавить (5.1).

**E5-33 · MINOR · дефект при ≤ f.** R-123: `committee_read_hash` пробует заголовок, не состояние (`node/dpos.rs:1382-1404`), при бэкфилле `committee_for` ⇒ `None` на всё окно ⇒ DKG-актор слеп. Дубль R-123. Вход beacon-а, чинится в Э4 (`executed_state_hash`).

**E5-34 · NIT · непредсказуемое состояние.** `spec_exec.rs:108-115`: σ, восстановленный из СОБСТВЕННЫХ per-partial-проверенных голосов, при `Invalid` под своим ключом — только `error!`. Такой исход означает локальную порчу (ключ/share) и должен быть громким отказом. Новая. Не покрыто.

**E5-35 · MODERATE · граница протокола при > f.** Коррелированный crash > f членов с `Torn` журналами ⇒ каждый sit-out ⇒ церемония ниже dealer-quorum ⇒ epoch без ключа (принято, `08:1776-1781`); порог σ = `quorum(n)` ⇒ одно удержание partial-а останавливает seed (входное ограничение). Оба — граница; код обязан выставить явное состояние `Stalled{reason}` — сегодня это `dkg_finalize_deferred` warn/error один раз на пару (эпоха, причина) (`actor.rs:1495-1515`) и `engine_demoted_no_polynomial`. Связано R-071. П-9 автомат должен назвать состояние.

**E5-36 · MINOR · дефект при ≤ f.** `Torn` первого рекорда до дедлайна ⇒ sit-out, хотя ничего не отправлено (`share_state.rs:593-621`, `actor.rs:1673-1686`; детерминированный дилер сделал бы байт-идентичный старт). Дубль R-072, R-051. П-9 («пересмотреть»).

**E5-37 · MINOR · дефект при ≤ f.** Окно heal = 1 эпоха (`mod.rs:106`, `share_state.rs:668-684`) против ретенции схем 8 (`lib.rs:26`) — узел, простоявший > 1 эпохи, не восстановит share при наличии логов у пиров. Дубль R-025. П-3 (журналы `SCHEME_RETENTION_EPOCHS`).

**E5-38 · MINOR · дефект при ≤ f.** Key journal пишет любое `Agreed` независимо от исхода `insert` (`keys.rs:280-298`, `key_journal.rs:170-181`), `Ordinal::put` перезаписывает индекс (`CW ordinal/storage.rs:262-289` по AUDIT-BEACON, в сессии не перечитано — [LIKELY]) ⇒ после рестарта RAM и диск расходятся. Дубль R-068. П-3 удаляет key journal.

**E5-39 · NIT · оверинженеринг/костыль.** `frozen_dkg_qual` собирается дважды по обе стороны (`node/dpos.rs:1519-1543`, `dpos.rs:3414-3438`), правило «бит = комитет сменился» узел выводит сам сравнением ростеров (`actor.rs:1582-1722`, `committee_pair_for`) — Д-7. Дубль R-012. П-3 (Д-7).

**E5-40 · NIT · нарушение границы модуля.** Имена партиций beacon объявлены в `dpos.rs:116-125` (`SEED_JOURNAL_PARTITION`, `KEY_JOURNAL_PARTITION`, `pub ARTIFACT_JOURNAL_PARTITION`), а не в модуле. Новая. Не покрыто.

**E5-41 · MINOR · дефект при ≤ f.** `ensure_key(Local)` на каждый серт inlet-а (`cert_inlet.rs:693`) и допуск при `NoKey` (`bls/combined_scheme.rs:436-441`) ⇒ сертификат с подменённым σ ложится в архив marshal и раздаётся дальше (R-008; стенд `a_forged_seed_slot_…`). Решение Д-3. Дубль R-008. П-2 смягчает (выход из hold), отравление остаётся.

**E5-42 · NIT · оверинженеринг/костыль.** Seed promoter — отдельная задача (`plane.rs:815-835`) поверх карантина; на follower та же логика второй копией внутри `run_fetcher` (`follower.rs:284-337`). Карантин — состояние, не задача. Связано BB-7. П-2.

**E5-43 · MINOR · непредсказуемое состояние.** Replay при старте неоднороден: seed/key journal `Ordinal::init`/replay — `?` (`seed_journal.rs:351-383`, `key_journal.rs:230-258`), share/artifact/DKG-журнал — warn+skip (`share_state.rs:330-366`, `artifact.rs:565-617`), `Torn` — sit-out. Нет одной функции «состояние после рестарта». Новая. П-3 частично.

**E5-44 · MINOR · нарушение границы модуля.** `boundary_base` (`epoch_manager.rs:158-169`) сам решает «предикат прежде стора» и превращает σ в базу элекции (`witness_fallback_seed`), executor сам решает «предикат прежде стора» (`executor.rs:2915-2938`), crash-replay — третий раз (`dpos.rs:564-592`); правило `mandatory_at` живёт в трёх потребителях, а не в `seed_for`. Новая. Не покрыто.

**E5-45 · MINOR · непредсказуемое состояние.** Отравленный мьютекс обрабатывается «тихо продолжить»: `adopt_share` пишет share на диск, а в RAM кладёт только `if let Ok(mut store) = self.store.write()` (`actor.rs:1019-1021`) — при отравлении share на диске есть, в памяти нет, повтора нет, узел verify-only до рестарта; тот же паттерн `certify.rs:193-199`, `keys.rs:301-304`, `oracle.rs:153`, `resolve.rs:76-78` (`Absent` тихо). [KNOWN]. Найдено контр-рецензией, проверено мной по этим строкам. Новая. Не покрыто.

**E5-46 · MINOR · непредсказуемое состояние (в ПРОЕКТЕ §5.1 первой версии, не в коде).** Замена трёх `Notify` на `tokio::sync::broadcast` теряет свойство ХРАНИМОГО ПЕРМИТА: `notify_one` держит пермит, и «запись, попавшая между промахом и await, не теряется» (`executor.rs:1478-1482`, `actor.rs:995-998`); `broadcast` при отставании получателя ОТБРАСЫВАЕТ сообщения и отдаёт `RecvError::Lagged` (tokio-1.49 `sync/broadcast.rs:39-41`). Для `KeyAvailable` (потребитель — ремонт схем, `epoch_manager.rs:666-673`) потеря = ожидание навсегда, если потребитель не перечитывает состояние; для `DataFault` потеря = пропущенная ротация upstream-а при R-008. [KNOWN] по семантике tokio и коду пермита. Найдено контр-рецензией (A10/A19/B29), проверено мной. Закрыто в §5.1: события — пробуждения с обязательным перечитыванием, `Lagged` = пробуждение; `DataFault` — отдельный unbounded-канал `faults()`. 5.0.

Сводка по категориям (одна основная категория на находку): дефект при ≤ f — E5-03, 09, 11, 13, 14, 15, 32, 33, 36, 37, 38, 41 (12); граница протокола при > f — E5-25, 35 (2); нарушение границы модуля — E5-01, 02, 04, 05, 30, 40, 44 (7); оверинженеринг/костыль — E5-06, 16, 19, 21, 22, 23, 26, 27, 28, 31, 39, 42 (12); непредсказуемое состояние — E5-07, 08, 10, 12, 17, 18, 20, 24, 29, 34, 43, 45, 46 (13). Итого 46. Вторичная категория у E5-03 (нарушение границы), E5-07 (дефект при ≤ f по последствию), E5-14 (половина — граница I5), E5-32/E5-41 (вызывающий снаружи).

## §5. Целевая архитектура

### 5.1 Граница (всё, что выходит из `beacon/`)
~~~
pub trait Beacon: Send + Sync { … }       // ТРЕЙТ из 9 запросов/приёмов + subscribe + faults (ниже); потребители держат Arc<dyn Beacon>.
                                          // Трейт, не структура: шов подстановки стенда — Arc<dyn Randomness> в трёх режимах
                                          // (stand.rs:1782-1788) и обёртка WithholdingRandomness (byzantine_roles.rs:351-370) —
                                          // остаётся тем же швом; продакшн-реализация одна (LiveBeacon), тестовые (Static, Absent,
                                          // Withholding-обёртка) переписываются под 9 методов в 5.0 (это и есть цена 5.0, см. §7)
pub fn build(inputs: BeaconInputs) -> (Arc<dyn Beacon>, Tasks)   // синхронно, геометрии не ждёт
pub enum BeaconInputs {                   // всё, что модуль берёт извне, одним типом; две сборки
    Validator {
        keys: (ed25519, ValidatorBlsKeypair, Option<ShareSealKey>), share_dir, partition_prefix,
        p2p: beacon_channel, resolver_channel, muxes: {vote, cert, resolver, bodies},
        committees: Arc<dyn CommitteeReads>,  // committee_for / pair / source / dkg_qual — одним трейтом; dkg_qual читается на ТОМ ЖЕ курсоре max(fin, live), что и пара ростеров (node/dpos.rs:1382-1404), не на finalized-хэше (Д-7, §6)
        clock: watch::Receiver<u64>,          // ordering tip marshal — ЕДИНСТВЕННЫЙ фидер; начальное значение = Tip, который marshal шлёт на старте из своего архива (CW marshal/core/actor.rs:397-401)
        geometry: watch::Receiver<Option<(activation, interval)>>,   // морозит поллер узла ПОСЛЕ старта (node/dpos.rs:1690-1703); до Some автомат стоит в явном состоянии Unfrozen (§5.2), build не ждёт и не падает
    },
    Follower {                                // --cert-follow: без ключей, муксов, DKG (cert_follow/mod.rs:179)
        committees: Arc<dyn CommitteeReads>,
        artifact_upstream: Arc<dyn ArtifactUpstream>,   // RPC get_epoch_artifact
        partition_prefix: Option<..>,         // None = RAM-only, как сегодня (follower.rs:127-129,168-171: «рестарт стоит один fetch на эпоху»); Some — если владелец подтвердит входное ограничение «seed-журнал follower-а пишется» (открыто, §0.7(ж))
    },
}
pub struct Tasks { supervised: Handle<()>, drain: Handle<()> }   // ровно два хэндла; внутри — ЯВНЫЙ супервизор с правилом «смерть любого supervised-ребёнка (актор, resolver-движок, launcher) завершает supervised»; drain-писатели журналов спавнятся из контекста сборки, ВНЕ линии спавна движка — тест node/dpos.rs:2648 остаётся и охраняет это
// запросы
fn seed(&self, round) -> Option<Seed>                    // СИНХРОННО (executor.rs:2934, epoch_manager.rs:158-169); проверенная σ ТОЛЬКО по точному раунду; terminal pin остаётся правилом вытеснения внутри индекса, никогда не отвечает соседним раундом (certify.rs:379-396, epoch_manager.rs:152-157)
fn epoch_key(&self, epoch) -> Option<GroupPublic>        // PK в силе (carry внутри)
fn mandatory_at(&self, epoch) -> bool
fn can_participate(&self, epoch) -> ShareProbe           // дешёвая проба (два чтения), зовётся на каждом ребре reconcile ПЕРЕД граничным чтением (epoch_manager.rs:1071, :1124-1127 «POSITION IS LOAD-BEARING»); свёртка в signer заставила бы платить полный resolve на каждом ребре
fn signer(&self, epoch, snap, keypair) -> SignerVerdict  // «я подписант E?» + схема; W1/W3 нет; ЯВНО гейтится mandatory_at — сегодня арм Signs обходит дверь oracle_for и безопасен только через Some(None) в carry.rs:118-129, которая удаляется вместе с carry.rs
fn oracle_for(&self, epoch) -> Option<Arc<dyn SeedOracle>>   // вот-путь; sync
// приём
fn observe_certificate(&self, cert: &Finalization|&Notarization) -> Observed { Recorded, Pending, Refused }   // синхронный вердикт возвращается ЗДЕСЬ; приобретение ключа внутри — только сетевой рунг вне vote/cert-пути (как PinEffort::Local/Thorough сегодня, surface.rs:216-224)
fn artifact_bytes(&self, epoch) -> Option<Vec<u8>>       // RPC-раздача
// события — ПРОБУЖДЕНИЯ, не факты: каждый потребитель на пробуждении ПЕРЕЧИТЫВАЕТ состояние запросом; RecvError::Lagged = такое же пробуждение (tokio broadcast ТЕРЯЕТ сообщения отставшего получателя, tokio-1.49 sync/broadcast.rs:39-41); это сохраняет свойство хранимого пермита notify_one (executor.rs:1478-1482, actor.rs:995-998)
fn subscribe(&self) -> broadcast::Receiver<BeaconEvent>  // SeedRecorded(round) | KeyAvailable(epoch) | ParticipationChanged(epoch) — «моя способность участвовать могла измениться» в обе стороны, не «share появилась» | Stalled{epoch, reason}
fn faults(&self) -> mpsc::UnboundedReceiver<DataFault>   // ФАКТ, а не пробуждение: поздний вердикт Refused (σ принята при NoKey, ключ пришёл, σ не сошлась) — один потребитель (inlet/UpstreamResolver), без потерь; на broadcast ему нельзя (E5-46)
~~~
Уходит внутрь: `record_seed`/`quarantine_seed`/`on_invalid_seed` (вердикт — в `observe_certificate`), `ensure_key`/`PinEffort` (приобретение — реакция на `observe_certificate(NoKey)` и на `clock`; разделение «сетевой рунг только вне vote-пути» сохраняется внутри), `terminal_seed_at` (в `seed`), три `Notify` (в `subscribe`), `agreement_intake` (инстансы живут и подметаются внутри), 8 хэндлов и `agreement_intake` (в `Tasks`), `for_keys`/`for_seeds`/`FollowerRandomness` (одна продакшн-реализация; follower = сборка `BeaconInputs::Follower`; `absent`/`StaticRandomness` — тестовые реализации трейта, уезжают под `cfg(test)`). Удаляются БЕЗ замены: `observe_epoch`+`observe_cert` — после 5.1 (нет `keys.rs`, нет `Carried`) и 5.2 (карантин = состояние, terminal = правило) фронтиру нечего прунить: журналы и serve-кэш чистит актор от часов (`actor.rs:1095-1108`), seed-журнал — писатель по `rolled` (`seed_journal.rs:446-451`), `ArtifactStore` не вытесняется (§8); слияние в один `observe_frontier` не решало бы, чей из двух фронтиров побеждает (`surface.rs:226-232`), а R-090 закрывается исчезновением `Carried`, не слиянием. `share_probe` НЕ сворачивается в `signer` — остаётся как `can_participate` (цена — resolve на каждом ребре, `epoch_manager.rs:1124-1127`). Метода `prev_randao(round)` в границе нет: дериватор получает `Option<&Seed>` аргументом (`derive.rs:78-87`, `application.rs:1154-1199`) и хэндла beacon-а не держит; второй путь к чистой функции `prev_randao_from_seed` не нужен. Плейсхолдер `absent_unregistered` сегодня стоит в двух продакшн-конструкторах до `with_randomness` (`cert_inlet.rs:506`, `application.rs:377`): в целевой форме `CertInlet` и `FluentApp` получают `Beacon` в конструкторе, то есть `beacon::build` идёт раньше них — порядок сборки в `node/dpos.rs` меняется; что это возможно без цикла зависимостей (beacon-у нужны только каналы и трейты) — [ГИПОТЕЗА], проверяется в 5.0 компиляцией. Остаются снаружи как чистые производные σ, не хранилища: `prev_randao_from_seed` (`node/derive.rs:78-87`), `witness_fallback_seed`/`constant_fallback_seed` (`epoch_manager.rs:158-169`).
Потребители после переноса: spec_exec — `observe_certificate(Notarization)`; cert_inlet и `UpstreamResolver` — `observe_certificate(Finalization)` (синхронный `Refused` ⇒ data fault inlet-а) + `faults()` (поздний вердикт); executor — `seed`, `mandatory_at`, `subscribe`; epoch_manager — `can_participate`, `signer`, `seed` (terminal), `mandatory_at`, `subscribe`; outer/engine — `oracle_for`; crash-replay — `observe_certificate` для локального и upstream-сертификата, затем `seed`; `Pending` (ключа ещё нет) ⇒ `ReplaySeed::Defer` по существующей петле (`dpos.rs:846-858`), возобновление по `KeyAvailable`; ключ после рестарта нормально лежит на диске (артефакт — durable в момент появления, `artifact.rs:452-474,664-677`), так что ожидание возникает только после провала sync артефакта и ограничено одним fetch у пиров; сегодняшнее «архив читается без проверки, потому что ключа может не быть» (`dpos.rs:613-622`) заменяется проверкой под `PK_E`, чем закрывается E5-03. `node/derive.rs` beacon НЕ зовёт (получает `Option<Seed>` аргументом, как сегодня); `cert_follow` — сборка `Follower`.

### 5.2 Внутренний автомат (на target-эпоху E; входы — `clock`, артефакт, p2p-сообщения, resolver)
Четыре входа, не один: арм артефакта сегодня прямо обоснован как «ЕДИНСТВЕННЫЙ арм, не зависящий от потока высот: при остановленной цепи на `epoch_start(E+1)` часы стоят, и именно артефакт снова двигает ключ эпохи» (`actor.rs:844-847`) — это сценарий halted-chain recovery из §5.6, и часами он не покрывается.
~~~
Unfrozen ──(geometry = Some)──▶ Idle          [явное состояние вместо «error! и актор не стартует» (plane.rs:684-693); can_participate ⇒ Withheld(GeometryUnfrozen), signer не строится; события/запросы по эпохе работают]
Idle ──(h ≥ start(E−1), change(E) ∨ E==BOOTSTRAP, me ∈ C[E])──▶ Dealing
Dealing ──(h ≥ seal(E))──▶ Sealed ──(instance certified)──▶ Agreed{artifact}
Sealed ──(instance: body lost | committee unreadable)──▶ Acquiring{artifact} (pull, retry без предела, gauge)
Agreed ──(all pinned bodies held)──▶ Finalizing ──Ok──▶ Keyed{share}
Agreed ──(bodies missing)──▶ Acquiring{logs} ──▶ Finalizing
Finalizing ──Err(MissingPlayerDealing)──▶ Unrecoverable(E)   [терминал, событие Stalled]
Finalizing ──Err(other)──▶ Acquiring{logs}                  [не «сидеть с can_finalize=false»]
любое состояние ──(второй quorum-артефакт за эпоху | вторая σ за раунд, ПРОВЕРЕННАЯ под PK_E)──▶ Conflict(E) [терминал; событие Stalled{Conflict}; подписание эпохи остановлено]
        // триггеры сужены до проверенных значений: два разных кворума ⇒ ≥ 2q−n Byzantine (свидетель > f); две σ под одним PK_E невозможны при ≤ f (bls/beacon.rs:161-167);
        // «второй Agreed» как отдельный случай исчезает — Agreed не хранится; подделка одним пиром не достигает Conflict: артефакт отвергается кворум-проверкой до стора (artifact.rs:246-273), σ — VerifiedSeed::check
me ∉ C[E] ∨ ¬change(E): KeyOnly ──(нужен PK для cert E)──▶ Acquiring{artifact(chain_key_epoch(E))} ──▶ Keyed{no share}
Журнал Torn, h < seal(E) ──▶ Dealing с reconstruct   [ничего не отправлено: «h < deadline ⇒ мы никогда не печатали ⇒ ни один лог не ушёл» (actor.rs:1655-1660), дилер детерминирован (ceremony.rs:235-240) — тот же гейт, что у Present; закрывает E5-36/R-072]
Журнал Torn, h ≥ seal(E) ──▶ SatOut(E) [терминал]; NoFile после seal ──▶ SatOut(E) (сегодня start_fresh = R-036)
Persist Err share при Keyed ──▶ не Keyed: остаёмся Finalizing/Acquiring, событие Stalled{PersistFailed}, повтор по часам
Persist Err артефакта ──▶ артефакт ПРИНЯТ в RAM (Agreed/Keyed идут дальше), запись повторяется, событие Stalled{PersistFailed} + gauge   [отказ принять артефакт превращал бы локальную ошибку диска в «нет PK_E ⇒ все σ Pending ⇒ парк исполнения»]
~~~
Рестарт: `recover(E) = f(share-файл, DKG-журнал, artifact store, clock)` — одна функция с перечислимыми ветками: (share есть, артефакт есть) ⇒ Keyed; (share есть, артефакта нет — частичный успех: share записан, sync артефакта не прошёл, `artifact.rs:668-677`) ⇒ Acquiring{artifact} с сохранённым share, Keyed по приходу артефакта от пиров (при ≤ f они его держат); (артефакт есть, share нет, журнал есть) ⇒ Finalizing; (журнал Present, h < seal) ⇒ Dealing с reconstruct; (Present, h ≥ seal) ⇒ Sealed player-only; (Torn, h < seal) ⇒ Dealing с reconstruct; (Torn, h ≥ seal) ⇒ SatOut; (NoFile, h ≥ seal) ⇒ SatOut; (NoFile, h < seal) ⇒ Dealing. σ: `seed` = RAM-индекс, журнал только для рестарта (чтение `Ordinal` асинхронно, `seed` обязан оставаться синхронным); на промахе — `Pending` до `observe_certificate`.

### 5.3 Единственный владелец каждого факта
| Факт | Владелец | Диск | Что удаляется |
|---|---|---|---|
| σ(round) | `SeedIndex` внутри beacon — RAM-индекс с ретенцией `SEED_RETENTION` по числу раундов + исключение терминального раунда эпохи (счёт глобальный, а спрашивает следующая эпоха: `certify.rs:97-99,141-146`); журнал — только для рестарта | `beacon-seed-ordinal` (как есть) | `SeedStore.waiters/terminal/quarantined` как отдельные карты (карантин = `Pending` в индексе, не на диске; terminal — правило вытеснения, не карта), промоутер-задача, `certify.rs` имя. Дедуп «одна ERROR-строка на эпоху» из `reported_invalid_seed` (`keys.rs:419-430`) переезжает защёлкой на `faults()` |
| σ ВНЕ beacon (остаток до Э6) | executor: `Notarized.seed` (`executor.rs:134`), `ParkedSpec.seed` (`:199`), `Deferred.seed` (`:564`) — три транзиентные копии на время жизни блока в pipeline | — | НЕ удаляются в Э5: это executor seed intake, строка 6.1 (правило «5.2 и 6.1 нельзя параллельно», общие строки `executor.rs:2635`, `:2934`); свойство 2 после Э5 — «один владелец, три транзиентных читателя по значению» |
| PK_E и полином | `ArtifactStore` (по эпохе минта) + memo `chain_key_epoch` (правила memo сохраняются: `None` никогда не мемоизируется, иначе догоняющий узел навечно пинит бутстрап-ключ, `carry.rs:83-95`; два слоя `Option` — транзиент против «эпоха до beacon-а», `carry.rs:70-73`) | `beacon-artifact-metadata` (как есть) | `keys.rs`, `key_journal.rs`, `carry.rs` (кроме memo), `resolve.rs`, W1/W3, `Carried`/`LocalDkg`, копия артефакта в share-файле, `Output` в `CeremonyStore` (Sharing строится из `artifact.group_key`, `outcome.rs:29-30`). Размен W1: сегодня share без артефакта недостижим (finalize берёт pinned-набор ТОЛЬКО из `agreed_pinned`, который пишет только `on_artifact`, `actor.rs:926,1463`; recompute читает `outcome_at` = ArtifactStore, `actor.rs:2085`), поэтому «член доделал DKG, а артефакт потерял» не существует; единственный реальный случай — рестарт после провала sync артефакта при живом share-файле, где сегодня ключ даёт копия в share-файле (`plane.rs:591-597`) или W1 из `CeremonyStore`, а в проекте — `Acquiring{artifact}` у пиров; на это время σ эпохи `Pending` и исполнение паркуется (`executor.rs:2934`, `:3612-3631`) — см. §5.4 |
| share | `ShareFile` по эпохе минта; RAM cache `mint → Share` | `beacon-share-e<E>.bin` v2 без артефакта; v1 удаляется | `CeremonyStore` как тип |
| dealer-логи | DKG-журнал + serve-кэш | `beacon-dkgjournal-e<E>.bin`, ретенция `SCHEME_RETENTION_EPOCHS` | `JOURNAL_RETENTION_EPOCHS = 1` |
| инстанс agreement | beacon (supervisor + band-sweep партиций ПО НОМЕРУ ЭПОХИ в окне `SCHEME_RETENTION_EPOCHS`, перенесённый как есть: только так собираются остатки прошлого процесса и партиция, пересозданная останавливающимся voter-ом, `epoch_manager.rs:300-308`; `abort` + JOIN перед удалением партиции, `:333-341`; пропуск эпох с живым инстансом, `:346-351`) | `dkg_epoch_{E}` | `epoch_manager.dkg_agreements`, `prune_agreements`, `agreement_intake`. SafetyHalt читается как ЗАЩЁЛКА (`is_engaged()`) при каждом старте инстанса, не как одноразовое событие: `SafetyHalt::engage` — один `notify_one`, а защёлка восстанавливается из маркера datadir и «узел, остановившийся вчера, переадоптирует на каждом рестарте» (`epoch_manager.rs:829-852`) |
| часы | `clock` из marshal tip: `watch`, начальное значение — стартовый `Update::Tip` marshal-а из его архива (CW `marshal/core/actor.rs:397-401`, без сети); дальше — Tip на каждой финализации выше tip (`:1453-1457`) | — | поллер `fin + K` как фидер (задача остаётся — она морозит геометрию, `node/dpos.rs:1690-1703`), inlet tee в `dkg_height_tx`, канал 256. Tee избыточен ПО МЕХАНИЗМУ: inlet после tee отдаёт ту же финализацию marshal-у (`cert_inlet.rs:921-931` `report_finalization`), marshal кладёт её и шлёт Tip (`core/actor.rs:567-595` → `:1453-1457`), Tip приходит в тот же клок через `FluentApp::report` (`application.rs:1027-1034`); разница — порядок на миллисекунды, не покрытие. Что это не даёт регрессии догоняющего валидатора ПО ВРЕМЕНИ (Tip после `store_finalization`, tee — до) — проверяется только тестом с `CertInlet` (PLAN 4.0(в)), стенд без inlet-а этого не видит |

### 5.4 Модель отказов (исход у каждого — явный)
| Отказ | Исход |
|---|---|
| Ошибка persist share | share НЕ принимается (иначе «подписываю сейчас, нем после рестарта» — R-021); состояние Finalizing/Acquiring; событие `Stalled{PersistFailed}`, gauge; повтор на следующем тике часов; узел verify-only |
| Ошибка persist артефакта | артефакт ПРИНЯТ в RAM (ключ отовсюду перезапрашиваем, отказ его принять остановил бы верификацию и исполнение от локальной ошибки диска); запись повторяется; `Stalled{PersistFailed}` + gauge. Порядок: артефакт в RAM и в очередь записи РАНЬШЕ share (как сегодня «persist до evict журнала», `actor.rs:2246-2253`) |
| Частичный успех (share на диске, артефакт нет; рестарт) | `recover(E)` ⇒ `Acquiring{artifact}` с сохранённым share; ключа эпохи нет локально ⇒ σ эпохи `Pending` ⇒ `seed` промахивается ⇒ исполнение паркуется (`executor.rs:2934`, `:3612-3631`) до прихода артефакта от пиров (один fetch при ≤ f). ЭТО ИЗМЕНЕНИЕ ЖИВУЧЕСТИ против сегодняшнего кода, где копия в share-файле (`plane.rs:591-597`) или W1 (`surface.rs:1712-1745`, читает `verify_seed` `oracle.rs:265-273`) давали ключ без сети; размен принят: I2 (один `PK_E`, только кворум-заверенный) против локального ключа при сбое диска. Случай узок: сегодня share без артефакта недостижим иначе (`actor.rs:926,1463,2085`) |
| Локального артефакта нет (любая причина) | все σ эпохи `Pending`, исполнение паркуется на первой высоте эпохи; `Acquiring{artifact}` без предела, `Stalled{NoArtifact}`; плоскость соглашения/пиры — жёсткая зависимость живучести исполнения (для не-члена и follower это и сегодня так: I4) |
| Ошибка persist σ | σ принята в индекс (RAM), запись повторяется на каждом следующем `observe_certificate` до успеха, `error!` + gauge `seed_persist_failed`; восстановление из сертификата НЕ гарантировано: у ancestry-finalized высоты сертификата может не быть нигде (`dpos.rs:624-627`), поэтому рестарт до успешной записи ⇒ `Pending` ⇒ crash-replay через `observe_certificate` из архива/upstream, а при отсутствии сертификата — `Stalled{SeedLost{round}}` и defer вызывающего (как сегодня), без выдумывания σ |
| Потеря/порча DKG-журнала | `Torn` и `NoFile` — одно правило по таймингу (§5.2): до seal ⇒ Dealing с reconstruct (ничего не отправлено, дилер детерминирован), при/после seal ⇒ SatOut(E) явное |
| Артефакт отсутствует у не-члена / follower | Acquiring без предела с gauge и событием `Stalled{NoArtifact}` (I4); throttle `PULL_MIN_INTERVAL` и последовательный fetch follower-а (`follower.rs:284-337`) переезжают в Acquiring |
| Рестарт до дедлайна | reconstruct dealer (как сейчас) |
| Рестарт после дедлайна | player-only (как сейчас) |
| Рестарт без ключа эпохи (crash-replay) | `observe_certificate(локальный/upstream cert)` ⇒ `Pending` ⇒ `ReplaySeed::Defer` (`dpos.rs:846-858`), возобновление по `KeyAvailable` после `Acquiring{artifact}`; сертификата нет нигде (ancestry-финализированная высота, `dpos.rs:624-627`) ⇒ `Stalled{SeedLost{round}}` и defer вызывающего, без выдумывания σ |
| > f молчат (partial-ы/дилинги/тела) | seed не восстанавливается — цепь стоит; beacon в `Stalled{QuorumMissing}`; без fallback-seed, без re-deal |
| Два quorum-артефакта за эпоху / две проверенные σ за раунд | `Refused` + `Stalled{Conflict}` и остановка подписания эпохи (не warn); поздний `Refused` для σ, принятой при `NoKey`, — `DataFault{round}` в `faults()` |
| `geometry` не заморожена | явное состояние `Unfrozen` (§5.2): `can_participate ⇒ Withheld(GeometryUnfrozen)`, gauge; `build` НЕ падает — заморозка асинхронна и идёт после старта поллером узла (`node/dpos.rs:1690-1703`), а `Beacon` нужен `CertInlet`/`FluentApp` в конструкторе раньше; громкий отказ валидатора остаётся на launch узла, где ET не морозится |
| resolver умер | это один случай, не два: движок спавнится один раз и не перезапускается (`actor.rs:826-830`), закрытие `resolver_rx` = смерть движка; движок под `Tasks.supervised` ⇒ узел падает (сегодня уже так, `node/dpos.rs:834`); заявленная деградация «gossip-only» (`actor.rs:817-831`) убирается СОЗНАТЕЛЬНО — она описывала жизнь после смерти supervised-задачи, которой узел всё равно не переживает |

### 5.5 Что удаляется / что остаётся (файлы)
Удаляются: `keys.rs`, `key_journal.rs`, `resolve.rs`, `follower.rs`, `carry.rs` (→ функция `chain_key_epoch` внутри artifact index), `certify.rs` (→ `seed_index.rs`), `surface.rs::{for_keys, for_seeds, FollowerRandomness, observe_epoch, observe_cert, ensure_key/PinEffort как публичные}` (`StaticRandomness`/`Absent`/`Canned` — под `cfg(test)` как реализации нового трейта, `WithholdingRandomness` в стенде переписывается под него же), `verified_seed.rs` как публичный тип (остаётся внутренним), `outcome.rs::validate_share_on_poly` остаётся И становится обязательным перед ЛЮБЫМ `adopt_share` (требование П-3, `DECISIONS.md:86`; сегодня gate стоит только на recompute-пути, `actor.rs:2178-2266`), `dkg_oracle.rs` (тест) остаётся. Внутренние модули становятся приватными (`mod x;`, не `pub(crate)`): «внутренний путь снаружи» перестаёт компилироваться — это и есть проверка 5.0 вместо grep-а, который упирается в тестовые модули тех же файлов (`epoch_manager.rs:1849-1851`, `cert_inlet.rs:967-968`, `application.rs:1956-1965`). Остаются: `actor.rs` (переписан как автомат, ~половина), `ceremony.rs` (идентичность `(dealer, hash)`), `dkg_agree.rs`, `dkg_engine.rs` (+ sweep партиций), `dkg_transport.rs`, `artifact.rs`, `share_state.rs` (v2 без артефакта), `seed_journal.rs`, `oracle.rs` (одна реализация, `verify_seed` через `epoch_key`), `log_resolver.rs`, `log_store.rs`, `confirmations.rs`, `dkg_msg.rs`, `wire.rs`, `seed.rs`, `metrics.rs` (один реестр), `plane.rs` (build), `mod.rs`.

### 5.6 Сценарий, оправдывающий каждый компонент
- DKG-церемония + agreement-плоскость: смена комитета без блоков (halted-chain recovery, `08:2320-2327`).
- `ArtifactStore` + pull/RPC: не-член живой эпохи и follower получают `PK_E` (R-121, FLU-1167).
- Seed index + журнал: рестарт валидатора без upstream (Ex-11, R-020); follower деривит без нотаризаций.
- Share-файл: рестарт после финализации (08 «item A»).
- DKG-журнал + resume: рестарт в окне (`08:1417-1493`).
- Dealer-log resolver: recompute-heal и agreement `verify` с недостающими телами.
- Quarantine как состояние: окно `NoKey` до прихода артефакта (стенд `a_forged_seed_slot_…`).
- `oracle_for`: синхронный vote-path (PLAN §8 п.2).
- Событийная шина: три `Notify` с «одним ожидающим» — ловушка, описанная кодом (`13:571-575`); шина — пробуждения с перечитыванием, `Lagged` = пробуждение (E5-46).
- `faults()` отдельным каналом: поздний вердикт σ — факт, который нельзя потерять (ротация upstream-а при R-008).
- Детерминированный дилер: resume (PLAN §8 п.10).
- `can_participate` отдельно от `signer`: дешёвая проба на каждом ребре reconcile (`epoch_manager.rs:1124-1127`).
- Band-sweep по номеру эпохи: остатки прошлого процесса после рестарта (`epoch_manager.rs:300-308`).
- Состояние `Unfrozen`: геометрия морозится после старта (`node/dpos.rs:1690-1703`).
Не включено (нет сценария): W1/W3 (сценарий «share без артефакта» недостижим, `actor.rs:926,1463`; остаток при сбое диска — размен §5.4), `Carried`-memo как провенанс, terminal pin как ОТДЕЛЬНАЯ КАРТА (само правило «терминальный раунд не вытесняется» остаётся в индексе, §5.3 — сценарий есть: `epoch_manager.rs:158-169`), промоутер-задача, второй маршрут артефакта, три фидера часов (tee избыточен по механизму, §5.3), `for_keys`/`for_seeds`, `prev_randao` в границе, `observe_frontier`.

## §6. Сверка с решениями и планом

| Пункт | keep/change/drop | Причина | Следствие для порядка |
|---|---|---|---|
| П-2 (σ из marshal, `SeedStore` удалить) | change | Входное ограничение: seed-журнал — целевой носитель для follower-путей; marshal-сертификат — вход (`observe_certificate`), не стор; `hint_finalized` как выход из hold — не нужен (hold ждёт события `SeedRecorded`) | 5.2 не зависит от 4.x |
| П-3 (артефакт — единственный факт) | keep + расширить | Правильно; добавить: `Output` из артефакта (нет копий), ошибка persist share ⇒ не принимать / артефакта ⇒ принять в RAM и повторять (§5.4), вызывающий для не-члена живой эпохи (R-121/122), одна ретенция `SCHEME_RETENTION_EPOCHS`, `validate_share_on_poly` перед любым `adopt_share` (`DECISIONS.md:86`). Названный размен: удаление W1 + копии в share-файле = после сбоя sync артефакта ключ приходит только от пиров (§5.4) | 5.1; 4.1 НЕ обязателен — beacon получает `CommitteeReads` трейтом, 4.1 меняет реализацию трейта |
| П-9 (событийный автомат, хэш-идентичность) | keep + расширить | Добавить терминальные состояния, состояние `Unfrozen` (не `Err` из `build` — заморозка асинхронна), resolver-exit ⇒ fatal (один случай с движком), band-sweep партиций внутрь КАК ЕСТЬ (по номеру эпохи, abort+join), SafetyHalt — защёлка, `Option`-швы → конфиг; входы автомата — четыре (`clock`, артефакт, p2p, resolver) | 5.3 после 5.1 (владение фактами раньше автомата) |
| Д-3 (приём при `NoKey`) | decide: вариант (в) внутри beacon | Сертификат принимается multisig-ом (безопасность цепи не зависит от σ); σ — `Pending`; при `Refused` после прихода ключа beacon кладёт `DataFault{round}` в `faults()` (не в broadcast — E5-46) ⇒ inlet считает data fault и ротирует, marshal перезапрашивает высоту. (а) `Thorough` до verify — pull на vote-пути, отвергнуто PLAN §8 п.2. Проверка ротации требует стендового `CertInlet` — PLAN 4.0(в) в «Нужно до» 5.2 | 5.2 |
| Д-6 (слэшить эквивокацию dealer-лога он-чейн) | defer | В Э5 — локальный бан + сохранённая пара логов как evidence; контракт не трогаем (Д-4 открыт) | 5.3 записывает evidence, не шлёт |
| Д-7 (`dkgQual`: выводить или читать) | decide: читать — на курсоре пары | Одно EVM-чтение против класса R-012. ТОЧКА ЧТЕНИЯ: тот же state-хэш `max(fin, live)`, на котором узел читает пару ростеров (`node/dpos.rs:1382-1404`, `:1410-1420`), а не finalized-хэш `dkg_qual_at` (`:1520-1524`): решение «дилю ли я» на бите с finalized-хэша дало бы `None` (undecided) догоняющему валидатору и пропуск церемонии — сегодняшний тест `next != cur` идёт на `max(fin, live)` ради него (`node/dpos.rs:798-802`, `:1058`, `actor.rs:1616-1651`). `committee_pair_for` удаляется: решение — только по биту; `committee_for(E)` остаётся для членства | 5.1 |
| Д-9 (П-2 или B-1) | decide: ни то, ни другое дословно — beacon-owned index + intake | см. П-2 | 5.2 |
| Д-2 (checkpoint > 2 эпох) | без изменений | Э5 не влияет; артефактный путь для не-члена делает догон ключа независимым от checkpoint-а | — |
| PLAN §8 п.2 (sync oracle) | keep | `verify_attestation` inline в батчере | — |
| §8 п.3 (per-partial O(n)) | keep | порог = кворум, атрибуция нужна | — |
| §8 п.7 (вторая плоскость, свой namespace) | keep | подтверждено `bls/beacon.rs:64-69`, `CW simplex/scheme/mod.rs:122-132` | — |
| §8 п.8 («`Randomness` 15 → 9») | keep по форме, change по составу | Трейт остаётся трейтом (шов подстановки стенда `Arc<dyn Randomness>`: `stand.rs:1782-1788`, `byzantine_roles.rs:351-370` — три режима и роль `WithholdingRandomness`); состав — 9 запросов/приёмов + `subscribe` + `faults` вместо 15 (`surface.rs:113-232`). Цена: тестовые реализации (`StaticRandomness`, `Absent`, `Canned`, `Withholding`-обёртка) переписываются под новый трейт — входит в 5.0 | 5.0 |
| §8 п.10 (детерминированный dealer RNG) | keep | resume (`08:1432-1439`) | — |
| Ex-7, Ex-21, Ex-22 из старых «нужно до» | Ex-22 — выполнен (стенд `testbed/tests.rs:2295`, E3.3); Ex-7 (надёжность резолвера при `deliver == true` с неверным телом, `DECISIONS.md:55` — условие обоих П-2/B-1) — статус по `REGISTER.md:106-109`: «в очереди», не выполнен; остаётся условием 5.2, потому что crash-replay и `observe_certificate` для upstream опираются на тот же резолвер; Ex-21 — девнет-приёмка Д-3 после 5.2, как и раньше «после» | 5.2 |
| Порядок Э4→Э5→Э6 | change | 4.1 не блокирует 5.1 (см. П-3); 5.0 можно сейчас по зависимостям, но с переписыванием шва стенда (§8 п.8); 5.2 нуждается в PLAN 4.0(в) (стендовый `CertInlet`) для проверки `DataFault`-ротации; 5.2 и 6.1 нельзя параллельно (executor seed intake, общие строки `executor.rs:2635`, `:2934`); 5.1 и 4.1 параллельно можно при зафиксированном `CommitteeReads` | 5.0 → (5.1 ‖ 4.1) → (4.0(в)) → 5.2 → 5.3 → 5.4 → 6.1 |

Дрейф документов (документ:строка | утверждает | в коде):
| Документ | Утверждает | В коде |
|---|---|---|
| `13_invariants_gotchas_rules.md:484-487` | halt-арм one-shot, «adopted AFTER … never aborted … still OPEN» | `epoch_manager.rs:852-858` отказывает в адопции при взведённой защёлке — закрыто |
| `03_epoch_machinery.md:128` | W3 в `epoch_manager.rs:849` | W3 в `surface.rs:1790-1808` |
| `03_epoch_machinery.md:178-182` | `SCHEME_RETENTION_EPOCHS` определена `outer.rs:246` и «now `pub(crate)`» | `lib.rs:26`, `pub const` |
| `03_epoch_machinery.md:251-276` | follower `held_keys: None, pull_keys: None`, «PERMANENT» | `dpos.rs:3495-3507` `for_follower` с маршрутом RPC (`:3463-3474`); блок `:107-112` уже правлен, этот — нет |
| `08_…:2282-2285` | «target epoch re-agrees on a fresh instance» | `dkg_engine.rs:584-609` `started` не даёт повторного инстанса |
| `08_…:1806-1814` | `epoch_engine_demoted_no_polynomial` в `engine.rs` demote sites | `surface.rs:1961` (`share_probe`) и `:2015` (`signer_scheme`), на каждый вызов |
| `08_…:983-989` | узел «no longer names … the agreement launcher» | `node/dpos.rs:837` супервизирует `agreement_launcher_handle`; типов не называет, хэндлы — да |
| `PLAN.md:195` (§8 п.8) | `Randomness` 15 → 9 после П-2 | сегодня 15; после §5 — хэндл, не трейт |
| `epoch_manager.rs:1049` (комментарий) | «W1 `insert_group_key` is idempotent» | символа нет в дереве |
| `cert_inlet.rs:329-335,404-413` (док-комменты) | `beacon::actor::CommitteeFor`, `DkgActor`, `BeaconKeys::get_pk` ladder | внутренние имена в ядре |
| `node/dpos.rs:791-794` | `marshal_slot` — «DkgActor's deferred marshal READ handle … recompute reads pinned boundary outcomes» (док-коммент поля `:1065-1072` уже верен: «for the plane-native frontier resolver») | `BeaconConfig` (`:1945-1973`) слот не получает; он идёт только в `plane_upstream::new_bridge` (`:1801`) |
| `dkg_transport.rs:98-108` | buffered «drops every proposal body … verify parks on a body never cached» | `CW buffered/engine.rs:313-318` оповещает waiters до фильтра — верно лишь при subscribe после доставки |
| `dkg_oracle.rs:149-152` | `DKG_MARGIN_BLOCKS = 10` | `actor.rs:115` = 20 |
| `share_state.rs:545-551` | «re-dealing fresh would draw new OsRng randomness» | дилер детерминирован (`ceremony.rs:235-240`) |
| `outcome.rs:79-95` | «callers MUST run both» C-gate и `verify_seed` | recompute запускает только C-gate (`actor.rs:2178-2266`) |
| `mod.rs:16-46` | продакшн ходит только через две двери | `spec_exec.rs:18`, `cert_inlet.rs:18` — внутренние пути |
| `epoch_manager.rs:805` (комментарий) | sweep «fires on a `BeaconKeys::record`» | метода `record` нет, писатель — `set_pk` (`keys.rs:280`) |
## §7. Э5 как план

Было (PLAN.md §2, строки 106-111):
~~~
### Э5. Beacon: ключ из артефакта, DKG по хэшу, σ из marshal — открыт
| [ ] 5.1 | П-3 (`DECISIONS.md` §2) | R-017, R-025, R-039, R-053, R-056, R-068, R-090, R-052, R-021, BB-3, BB-6, BB-12 / R-012 (Д-7), R-020 | 4.1, Д-7 | 3–4 нед. |
| [ ] 5.2 | П-9 | R-002, R-036, R-038, R-026, BB-4, BB-10 / R-051, R-072 | 5.1, стенд 3.3 (Ex-22) | 3 нед. |
| [ ] 5.3 | П-2 или альтернатива B-1 (Д-9); затем R-016 порог абсолютный | R-007, R-069, R-088, R-090, BB-1, BB-7, BB-11 / R-008 (Д-3), R-018, R-020, R-086, R-016 | 5.1, Ex-7; «до» для R-008 даёт стенд (`history/E3-3-ROLES-1.md`), Ex-21 — девнет-приёмка после (3.4 отложено) | 2–3 нед. |
~~~
Стало:
| # | Работа | Закрывает / смягчает | Нужно до | Оценка |
|---|---|---|---|---|
| [ ] 5.0 | Граница: трейт `Beacon` из 9 методов + `subscribe` + `faults` (вместо 15), `BeaconInputs` (`CommitteeReads`, `clock`, `geometry: watch`, `ArtifactUpstream`), `observe_certificate` с синхронным вердиктом, `can_participate` рядом с `signer`, события как пробуждения (`Lagged` = пробуждение); потребители переведены (spec_exec, cert_inlet, executor, epoch_manager, crash-replay); ТЕСТОВЫЕ реализации трейта переписаны (`StaticRandomness`, `Absent`, `Canned`, `WithholdingRandomness` стенда — `stand.rs:1782-1788`, `byzantine_roles.rs:351-370`); внутренние модули приватные (`mod x;`), `for_keys`/`for_seeds` удалены; `prev_randao`/`observe_frontier` в границу не вводятся; состояние `Unfrozen` вместо `geometry None ⇒ Err` | E5-01, E5-02, E5-20, E5-34, E5-44, E5-46 / R-069 (половина) | ничего (до 4.1) | 1–1,5 нед [ГИПОТЕЗА] (было 3–4 дн; добавлен шов стенда) |
| [ ] 5.1 | Факт PK_E: артефакт — единственный владелец; удалить `keys.rs`, `key_journal.rs`, `carry.rs` (memo и его правила остаются), `resolve.rs`, W1/W3, копию артефакта в share-файле и `Output` в `CeremonyStore`; `Sharing` из `artifact.group_key`; `validate_share_on_poly` перед любым `adopt_share`; приобретение артефакта для не-члена живой эпохи и follower одним кодом (артефактная половина `follower.rs` уходит здесь; throttle fetch-а сохраняется); persist: share ⇒ отказ, артефакт ⇒ RAM + повтор, оба с `Stalled{PersistFailed}`; ЧАСТИЧНЫЙ УСПЕХ (share на диске, артефакт нет) ⇒ `Acquiring{artifact}` — названное изменение живучести (§5.4); одна ретенция `SCHEME_RETENTION_EPOCHS`; Д-7 = читать бит НА КУРСОРЕ `max(fin, live)` (`node/dpos.rs:1382-1404`), не на finalized-хэше; `signer` гейтится `mandatory_at` явно (замена `Some(None)` из `carry.rs:118-129`) | R-017, R-025, R-039, R-053, R-056, R-068, R-090, R-052, R-021, R-121, R-122, BB-3, BB-6, BB-12, B-7 / R-012, R-020, E5-07, E5-08, E5-09, E5-13, E5-18, E5-26, E5-29, E5-31 | 5.0 | 2–3 нед [ГИПОТЕЗА] |
| [ ] 5.2 | Факт σ: `SeedIndex` — RAM-индекс, синхронный `seed`, журнал только для рестарта — единственный владелец; ретенция `SEED_RETENTION` + исключение терминального раунда как правило; карантин как состояние `Pending`; вердикт внутри `observe_certificate` (синхронный `Refused`) + поздний `DataFault` через `faults()` (Д-3 = вариант (в)), дедуп ERROR-строки защёлкой на эпоху; удалить промоутер-задачу, `waiters`, terminal-карту как отдельную карту, `on_invalid_seed`, seed-половину и сам файл `follower.rs`; crash-replay через `observe_certificate` (`Pending ⇒ Defer`, возобновление по `KeyAvailable`); `hint_finalized` как выход из hold не вводить; затем R-016 порог абсолютный. ОСТАТОК: три транзиентные копии σ в executor (`executor.rs:134,199,564`) до 6.1 | R-007 (частично), R-008 (Д-3), R-069, R-086, R-088, BB-1, BB-7, BB-11, B-6 / R-018 (событие `SeedRecorded` для epoch_manager), R-016, E5-03, E5-10, E5-28, E5-42, E5-45 (мьютексы σ) | 5.0, Ex-7 (в очереди, `REGISTER.md:109`), PLAN 4.0(в) — стендовый `CertInlet` для проверки `DataFault`-ротации; Ex-21 — девнет-приёмка после | 1,5–2 нед [ГИПОТЕЗА] + 1–2 дн чужой работы 4.0(в) |
| [ ] 5.3 | DKG-автомат (П-9): идентичность лога `(dealer, hash)`, refetch по хэшу, две подписи одного дилера — локальный бан + сохранённая пара; явные состояния и терминалы (`Unfrozen`, `SatOut`, `Unrecoverable`, `Conflict` с суженными триггерами, `Stalled{reason}`); входы автомата — `clock` + артефакт + p2p + resolver; body-lost ⇒ `Acquiring{artifact}`; `Torn`/`NoFile` одно правило: до seal ⇒ Dealing с reconstruct, при/после seal ⇒ `SatOut`; finalize Err ⇒ `Acquiring{logs}`; resolver-exit = смерть движка ⇒ fatal (деградация «gossip-only» убирается сознательно); `Option`-швы → конфиг; один реестр метрик; `recover(E)` одной функцией (с веткой частичного успеха) | R-002, R-036, R-038, R-026, R-051, R-072, R-067, R-070, BB-4, BB-10 / R-071, E5-11, E5-12, E5-15, E5-17, E5-23, E5-24, E5-35, E5-36, E5-43 | 5.1 (5.2 желательно), стенд 3.3 (Ex-22 выполнен) | 3 нед [ГИПОТЕЗА] |
| [ ] 5.4 | Владение диском и жизненным циклом: beacon сам держит и подметает `dkg_epoch_*` — band-sweep ПО НОМЕРУ ЭПОХИ переносится как есть (`abort` + join перед удалением, пропуск живых, `epoch_manager.rs:300-366`); `epoch_manager.dkg_agreements`/`prune_agreements`/`agreement_intake` удаляются; SafetyHalt читается как ЗАЩЁЛКА при старте инстанса (`epoch_manager.rs:829-852`); один `clock` (`watch` от marshal Tip, начальное значение — стартовый Tip marshal-а), поллер `fin + K` и inlet tee как фидеры удаляются (tee избыточен по механизму: `cert_inlet.rs:921-931` → marshal `core/actor.rs:1453-1457` → `application.rs:1027-1034`); узлу — `Tasks{supervised, drain}` с явным супервизором, drain-писатели вне линии спавна движка (тест `node/dpos.rs:2648` остаётся); имена партиций — в модуль | E5-04, E5-05, E5-06, E5-40 / R-041 (частично) | 5.3; тест «догоняющий валидатор дилит на живом фронтире без tee» — требует `CertInlet` в стенде (PLAN 4.0(в)) | 1 нед [ГИПОТЕЗА] (было 3–5 дн; band-sweep, join-семантика и защёлка — не «перенос кода») |

Для каждой строки: «наполовину», проверка, риск переписывания.
- 5.0. Наполовину: трейт введён, но потребители ещё зовут старые методы через адаптер — допустимо один коммит; нельзя оставлять `record_seed` снаружи рядом с `observe_certificate`. Проверка: стенд 35/0 ПОСЛЕ переписывания трёх тестовых реализаций и обёртки роли (не «только импорты»: `stand.rs:1782-1788`, `byzantine_roles.rs:351-370`); внутренние модули приватны — «внутренний путь снаружи» не компилируется (grep по `consensus/src` не годится: тестовые модули лежат в тех же файлах, `cert_inlet.rs:967-968`, `epoch_manager.rs:1849-1851`); тест «spec_exec не может сконструировать `VerifiedSeed`» — компиляция. Риск переписывания: низкий по продакшну (перенос кода), средний по стенду.
- 5.1. Наполовину: артефактный владелец рядом с W1 — класс «локальный против сетевого» (П-3 «наполовину») — W1 удаляется тем же коммитом; `Carried` рядом с `chain_key_epoch` — допустимо. Проверка: стенд B1–B5; C7 (`a_zero_overlap_boundary_halts_the_chain_verify_only`, `testbed/tests.rs:1932-1950`) ПЕРЕПИСЫВАЕТСЯ, а не «зеленеет»: сегодня он пинует остановку и его собственный фальсификатор — «любой узел перешёл свою высоту парковки»; после 5.1 утверждение обратное — обе половины переходят границу (выходящая добирает артефакт эпохи 3 «вперёд», входящая — эпохи 2 «назад», `:1938-1943`), `prev_randao` всех узлов побайтно равны (фейк не врёт: артефакт проверяется `verify_artifact` против `committee[minted_at]` из `FakeStaking`, `artifact.rs:246-273`); тест persist-ошибки share: `share_dir` read-only ⇒ узел verify-only + событие, не подписант; тест persist-ошибки артефакта: узел продолжает верифицировать, `Stalled{PersistFailed}` виден; тест частичного успеха: share-файл есть, партиция артефактов пуста ⇒ после рестарта `Acquiring` → `Keyed` от пиров; `a_stable_committees_attested_mint_outlives_the_retention_window` пишется заново над новым владельцем (сегодня живёт в удаляемом `resolve.rs:304`). Риск: средний (`surface.rs` ≈ 1,1 тыс. строк продакшна переписывается).
- 5.2. Наполовину: индекс введён, промоутер оставлен — два пути к `Recorded`, нельзя; crash-replay не переведён — допустимо (текущее). Проверка: `a_forged_seed_slot_is_admitted_with_no_key_and_refused_when_the_key_lands` (`testbed/tests.rs:2635`) — первая половина (допуск при `NoKey`) остаётся зелёной, вторая МЕНЯЕТ исход: было `error!`, стало `DataFault` в `faults()` + ротация — наблюдаемо только со стендовым `CertInlet` (PLAN 4.0(в), в «Нужно до»); реплей-тест SIGKILL стенда (журналы per-node); crash-replay без ключа: партиция артефактов стёрта ⇒ `Defer` до `KeyAvailable`, затем деривация совпадает с пирами. Риск: средний.
- 5.3. Наполовину: хэш-идентичность без автомата — нормальное промежуточное состояние (П-9); автомат без хэш-идентичности — нет. Проверка: `a_dealer_with_two_logs_leaves_the_addressed_victim_without_a_share` (`:2295`) меняет вердикт на «жертва получает share по refetch» (роль самопроверяет подмену); `…_stops_the_chain_silently` (`:2471`) остаётся остановкой (граница I5) но с событием `Stalled{QuorumMissing}` и ERROR-строкой; тесты Torn/NoFile по одному правилу §5.2 (четыре клетки: {Torn, NoFile} × {до seal, после seal}); `Conflict` только от проверенных значений (подделка одним пиром не достигает). Риск: высокий (actor.rs 2,4 тыс. строк продакшна).
- 5.4. Наполовину: sweep в beacon при живом `prune_agreements` — двойное удаление; делать одним коммитом. Проверка: реплей-тест + перечисление партиций in-memory storage после прогона (нет `dkg_epoch_*` ниже фронтира, включая партиции «прошлого процесса» — рестарт стенда с оставленной партицией); SafetyHalt-защёлка: узел с маркером halt после рестарта не адоптирует инстанс; тест «один фидер» В СТЕНДЕ ВАКУУМЕН (там фидер и так один: `stand.rs:1757,1912,1932`, `CertInlet` отсутствует) — доказательство покрытия догоняющего валидатора требует стендового `CertInlet` (4.0(в)): валидатор с отстающим EL и живым inlet-ом печатает share на живом дедлайне; до этого теста удаление tee — [ГИПОТЕЗА] по времени при [KNOWN] по механизму. Риск: средний.

Одним шагом или последовательно: последовательно. Одним шагом невозможно проверить стендом промежуточные инварианты (владение ключом отдельно от владения σ), а объём (≈ 6,6 тыс. строк продакшна beacon удаляется/переписывается целиком: `actor` 2431, `surface` 1060, `keys` 641, `certify` 452, `key_journal` 302, `carry` 244, `resolve` 136, `follower` 521, `plane` 852; ещё ≈ 3,6 тыс. трогаются точечно: `artifact`, `dkg_engine`, `share_state`, `oracle`, `seed_journal`, `metrics`; плюс 4 потребителя и стенд) не помещается в один ревьюируемый коммит. Нельзя параллельно: 5.1 и 5.3 (оба `actor.rs`/`surface.rs`); 5.2 и 6.1 (executor seed intake); 5.4 и любые правки `epoch_manager.rs` из Э4.3. Можно параллельно: 5.0/5.1 с 4.1 при зафиксированном `CommitteeReads`.

## §8. Оставить как есть

- Порог σ = `quorum(n)` и «каждая σ требует partial от византийского при f» — граница протокола (входное ограничение); не вводить fallback-seed, k-lag, reveal-less re-deal.
- Синхронный `SeedOracle` на vote-пути (PLAN §8 п.2) и per-partial проверка O(n) (п.3) — оставить.
- Вторая simplex-плоскость с собственным namespace, `RoundRobin`, без `register_scheme`, без slasher (п.7; Д-6 отложен).
- Детерминированный дилер от подписи (п.10).
- `LEADER_TIMEOUT 30 s`/`CERTIFICATION_TIMEOUT 45 s`/`DKG_MARGIN_BLOCKS 20` — не трогать в Э5; геометрия из chainspec — Э2.4 (R-024).
- `commonware` `Logs::select` первые-q-по-ключу и `Player::finalize` view-first — библиотека; Fluent ничего не добавляет и не должен.
- Формат share-файла v2 и AEAD с AAD `(tag, version, epoch)` — оставить (минус копия артефакта).
- Терминальный вывод при `MissingPlayerDealing` — оставить как терминал (подтверждено `CW dkg.rs:1804-1869`).
- `ArtifactStore` без вытеснения — обоснован (сотни байт на смену комитета, `08:2300-2303`).
- Правило «серт принимается multisig-ом, σ проверяется отдельно» (`combined_scheme.rs:395-445`) — оставить; менять только судьбу σ (Д-3).
- Executor-сторона: `awaiting_seed` hold без дедлайна, детектор 60 с, парк guard #2 — вне Э5 (Э6).
- `NoopBlocker` на BEACON_RESOLVER — оставить (PLAN §8 п.1); бан по каналу — Э4.3.
- `prev_randao = keccak256(σ)` в `derive.rs` без перепроверки — оставить (`resolve_prev_randao`, `derive.rs:78-87`).

## §9. Где проверка была слабее всего

- `crates/node/src/evm.rs` продакшн-тело не читал (только grep); `bins/fluent` — только grep. Пред-исполнение `commitEpochCommittee`/`dkgQual` — по комментарию `evm.rs:941-946`, не по коду контракта.
- `CW storage` (`Ordinal::put` перезапись индекса для E5-38) — не перечитывал, взят из AUDIT-BEACON [LIKELY].
- `CW consensus/src/simplex/actors/voter` (что делает engine при `Park` в `verify`, `interesting` skew) — не открывал; опираюсь на комментарии `dkg_agree.rs:959-969` и предыдущий аудит.
- Тесты beacon (5,6 тыс. строк в `actor.rs` и др.) читались в прошлой части сессии до сжатия контекста; в этой части их не перечитывал — утверждения «что пинует тест» опираются на прочитанные ранее заголовки, не на повторное чтение.
- Достижимость E5-09 и E5-16 — [ГИПОТЕЗА]; E5-24 порядок «subscribe после доставки» — не проверен экспериментом.
- Стоимость: чтение 28 + 9 + 5 + 18 + 8 + 4 файлов, ~75 тыс. строк; повторные чтения после сжатия контекста — `cert_inlet.rs` хвост, `epoch_manager.rs`, `executor.rs` целиком.
- Команда чтения длинных строк: `sed -n 'N,Mp' file` и инструмент `Read` (обе выдают строки целиком); поиск: `awk 'length > 500 {print FILENAME": "FNR": "length}' file`.
- Контр-рецензия (Opus 5, свежий контекст, 61 обращение к коду): 21 расхождение и 3 пропуска; все проверены мной по названным строкам и внесены: арифметика §0 (категории, список непокрытых, «8 мест», «8 хэндлов»), ложная строка дрейфа A-12 удалена, якоря `PLAN.md:195`, `node/dpos.rs`, `actor.rs:1495-1515`, `surface.rs:2015`; в §5 — точный раунд в `seed`, `Conflict` в автомате, модель потери σ без сертификата, сборка `Follower`, конструкторы с `absent_unregistered`, `derive.rs`; в §6/§7 — судьба Ex-7/Ex-21/Ex-22, разделение `follower.rs` между 5.1/5.2; новая находка E5-45 (отравленные мьютексы). Не принято: ничего.
- Вторая контр-рецензия (Opus 5, свежий контекст, 80 обращений к коду, `history/E5-DESIGN-CRITIQUE.md`): 69 разобранных элементов, из них принято 57, частично 11, отклонено целиком 0, не решить по коду 1 — прослеживание в §10. Мои собственные [KNOWN] этого прохода: все якоря §10 открыты мной в этой сессии; подсчёт продакшн-строк beacon — свой скрипт (блоки `#[cfg(test)]` вырезаны по балансу скобок): `actor` 2431, `surface` 1060, `keys` 641, `certify` 452, `key_journal` 302, `carry` 244, `resolve` 136, `follower` 521, `plane` 852 (Σ 6639), `artifact` 989, `dkg_engine` 742, `share_state` 724, `oracle` 349, `seed_journal` 452, `metrics` 307 (Σ 3563). Где §5 первой версии вводил в заблуждение: (1) «ошибка persist ⇒ не принимать» для артефакта — превращала сбой диска в остановку исполнения; (2) `geometry ⇒ Err` при сборке — невозможен, заморозка асинхронна и после старта; (3) «стенд 35/0 без изменения тестов кроме импортов» — неверно, шов `Arc<dyn Randomness>` переписывается; (4) «C7 должен стать зелёным» — тест пинует остановку, он обязан упасть и быть переписан; (5) «sweep по своей карте» — отменял сбор остатков прошлого процесса; (6) «SafetyHalt — одно событие» — возвращал закрытую дыру защёлки; (7) удаление W1 подано без названного размена живучести; (8) шина `broadcast` без правила `Lagged` теряла пермит; (9) `Follower` с обязательной партицией на посылке, которую код опровергает.
- Текстовые проверки после всех правок (этот файл и `PLAN.md`): «…» в конце строки — 0/0; нечётные обратные кавычки — 0/0 (до сборки было 8 ограждений из трёх кавычек, заменены на `~~~`); две запятые подряд — 0/0; пустые двойные кавычки — 0/0; пустые круглые скобки — 24/4: в этом файле все в цитатах кода или в ограждении `~~~` §5.1 (Rust-тип единицы `Handle<()>`, turbofish `::<M>()`, вызов `write` в E5-45, `faults()`, `subscribe()`, `is_engaged()`), проверено скриптом по чётности обратных кавычек до совпадения; в `PLAN.md` три вне блока Э5 (строки 26, 89, 92 — §1 и Э3, не трогал по условию задачи) и одна в строке 5.2 (`faults()`, код).
- Команды проверок: `grep -c` по шаблону «многоточие в конце строки», по двум запятым и по пустым скобкам; нечётные кавычки — `awk '{n=gsub(/`/,"`"); if(n%2==1) print FILENAME": "NR}' f`.

## §10. Сверка с контр-рецензией (`history/E5-DESIGN-CRITIQUE.md`)

Каждый разобранный элемент — один раз. Вердикт: принято / отклонено / частично / не решить по коду. Якоря в столбце причины открыты мной в этой сессии.

| Идентификатор у критика | Вердикт | Причина с file:line | Куда внесено |
|---|---|---|---|
| §0.2(а) `prev_randao` в границе | принято | дериватор берёт `Option<&Seed>` аргументом и хэндла beacon-а не держит (`derive.rs:78-87`, `application.rs:1154-1199`) | §5.1 (метод удалён), §5.6, §7 5.0 |
| §0.2(б) `Follower` с партицией | не решить по коду | код: follower RAM-only осознанно (`follower.rs:127-129,168-171`); «входное ограничение» — посылка владельца, кодом не проверяется | §5.1 `partition_prefix: Option`, §0.7(ж) — вопрос владельцу |
| §0.2(в) `observe_frontier` | принято | после 5.1/5.2 нечего прунить фронтиром: актор чистит по часам (`actor.rs:1095-1108`), seed-журнал по `rolled` (`seed_journal.rs:446-451`); слияние не решает «чей фронтир» (`surface.rs:226-232`) | §5.1 (удалён без замены), §5.6 |
| §0.3(а) правило persist | принято | артефакт durable write-behind с warn (`artifact.rs:452-474,664-677`); отказ принять его = нет `PK_E` ⇒ `Pending` ⇒ парк (`executor.rs:2934,3612-3631`); порядок persist-до-evict (`actor.rs:2246-2253`) | §5.2, §5.4 (три строки), §6 П-3, §7 5.1 |
| §0.3(б) `geometry` при сборке | принято | заморозка асинхронна и после старта (`plane.rs:665-693`, `node/dpos.rs:1690-1703`); `build` до `CertInlet`/`FluentApp` ⇒ `Err` невозможен | §5.1 `geometry: watch`, §5.2 `Unfrozen`, §5.4, §6 П-9 |
| §0.3(в) crash-replay без ключа | принято | `dpos.rs:613-627` (архив без проверки, ключа может не быть), `:846-858` (`Defer`) | §5.1 потребители, §5.4 строка «Рестарт без ключа», §7 5.2 |
| §0.4(а) W1 | частично | сценарий «член доделал DKG без артефакта» недостижим: finalize только над `agreed_pinned`, который пишет только `on_artifact` (`actor.rs:926,1463`), recompute читает `outcome_at` (`:2085`); остаток — рестарт после провала sync артефакта (`artifact.rs:668-677`) при живом share-файле — принят и назван | §5.3 (строка PK_E), §5.4 «Частичный успех», «Локального артефакта нет», §0.4 |
| §0.4(б) inlet tee | частично | «не показано» опровергнуто по механизму: inlet отдаёт финализацию marshal-у после tee (`cert_inlet.rs:921-931`), marshal шлёт Tip выше tip (CW `marshal/core/actor.rs:567-595,1453-1457`), Tip идёт в тот же клок (`application.rs:1027-1034`); принято: время (tee до `store_finalization`) проверяется только тестом с `CertInlet`; начальное значение — стартовый Tip из архива (`:397-401`), не персистнутый finalized-маркер | §5.3 строка «часы», §0.7(а), §7 5.4 |
| §0.4(в) `observe_epoch`+`observe_cert` → один метод | принято | см. §0.2(в); R-090 закрывается исчезновением `Carried`, не слиянием | §5.1 |
| §0.4(г) band-sweep | принято | `epoch_manager.rs:300-308` (по номеру эпохи ради остатков прошлого процесса), `:333-341` (join), `:346-351` | §5.3 строка «инстанс agreement», §6 П-9, §7 5.4 |
| §0.4(д) копия артефакта в share-файле | частично | копия закрывает разрыв «share записан, запись артефакта не синкнулась» (`plane.rs:591-597`); удаление сохранено, исход частичного успеха назван: `Acquiring{artifact}` от пиров | §5.2 `recover`, §5.4 «Частичный успех» |
| §0.4(е) `Tasks` одним join-set | принято | тест `node/dpos.rs:2648-2696` двусторонний; write-back «паркуется навсегда» (`plane.rs:281-285`) | §5.1 `Tasks` (явный супервизор, drain вне линии спавна), §7 5.4 |
| §0.4(ж) трейт → структура | принято | шов `Arc<dyn Randomness>` в трёх режимах (`stand.rs:1782-1788`) и обёртке (`byzantine_roles.rs:351-370`); реализаций сегодня 6 (`surface.rs:436,636,753,1919`, `follower.rs:395`, `byzantine_roles.rs:371`) | §5.1 (трейт из 9 методов), §6 §8 п.8, §7 5.0 |
| §0.4(з) `share_probe` внутрь `signer` | принято | `epoch_manager.rs:1071`, `:1124-1127` «POSITION IS LOAD-BEARING» | §5.1 `can_participate`, §5.6 |
| §0.4(и) `Some(None)` в `chain_key_epoch` | принято | `carry.rs:118-129`; арм `Signs` через `oracle_at` в обход `mandatory_at` (`surface.rs:2078-2100`) | §5.1 `signer` явно гейтится `mandatory_at`, §7 5.1 |
| §0.5 порядок (Д-7 точка чтения; объём 5.0; 5.2‖6.1) | принято | `dkg_qual_at` — finalized-хэш (`node/dpos.rs:1520-1524`), пара — `max(fin, live)` (`:1382-1404`, `:1410-1420`); объём — см. §0.4(ж); 5.2‖6.1 — совпадает с проектом | §6 Д-7, §6 «Порядок», §7 5.0/5.1 |
| §0.6 две границы (стенд после 5.0; 5.2/C7) | принято | `stand.rs:1782-1788`; C7 фальсификатор `testbed/tests.rs:1948-1950`, тело `:1962-1985` пинует остановку | §0.5, §7 5.0/5.1/5.2 |
| §0.7 свойства 2 и 4; восемь правок | принято | три копии σ `executor.rs:134,199,564`; правки 1–8 внесены построчно (см. строки выше и ниже) | §0.1, §5.3 строка «σ ВНЕ beacon», §7 5.2 |
| A2 | принято | = §0.2(а) | §5.1 |
| A5 | принято | = §0.4(з) | §5.1 |
| A8 | принято | = §0.2(в) | §5.1 |
| A10 `subscribe`/`Lagged` | принято | пермит `executor.rs:1478-1482`, `actor.rs:995-998`; tokio-1.49 `sync/broadcast.rs:39-41` | §5.1 (события — пробуждения), §4 E5-46 |
| A11 `BeaconInputs` | принято | `ArtifactUpstream` только у `Follower` — уже так в §5.1; сборка без цикла (`node/dpos.rs:1943-1975`) | §5.1 (без изменений формы) |
| A12 `geometry` готовое значение | принято | = §0.3(б); выбран вариант «`watch` + явное состояние», не двухфазный `start` | §5.1, §5.2 |
| A13 `Follower` партиция | частично | = §0.2(б) | §5.1 |
| A14 `Tasks` | принято | = §0.4(е) | §5.1 |
| A17 `ShareReady` имя | принято | ребро взводится и снятием способности (`epoch_manager.rs:1066-1075` abort живого движка по `Withheld`) | §5.1 `ParticipationChanged` |
| A19 `DataFault` по терящей шине | принято | = A10; синхронный вердикт — возврат `Refused`, поздний — `faults()` | §5.1, §6 Д-3, §7 5.2 |
| A20 один фидер часов | частично | = §0.4(б) | §5.3, §0.7(а) |
| A31 `Conflict` триггеры | принято | `bls/beacon.rs:161-167` (уникальность σ), `artifact.rs:246-273` (подделка отвергается до стора), `keys.rs:339-342` исчезает с `Agreed` | §5.2 (сужены до проверенных значений) |
| A32 / A44 три формулировки Torn/NoFile | принято | `actor.rs:1655-1660` (гейт `h < deadline ⇒ не печатали`), `ceremony.rs:235-240`, `share_state.rs:575-621` | §5.2 (одно правило, четыре клетки), §5.4, §7 5.3; эксперимент §0.7(е) |
| A33 / A42 persist «иначе» | принято | = §0.3(а) | §5.2, §5.4 |
| A35 «вход — один `clock`» | принято | `actor.rs:844-847` (арм артефакта не зависит от высот) | §5.2 шапка, §6 П-9, §7 5.3 |
| A36 «индекс НАД журналом» | принято | `seed` синхронный (`executor.rs:2934`, `epoch_manager.rs:158-169`), `Ordinal` асинхронен; исключение терминального раунда (`certify.rs:97-99,141-146`) | §5.2 `recover`, §5.3 строка σ, §7 5.2 |
| A40 sweep «по своей карте» | принято | = §0.4(г) | §5.3 |
| A50 `geometry ⇒ Err` | частично | = §0.3(б): направление (громко, явно) сохранено, `Err` заменён состоянием | §5.4 |
| A51 resolver умер | принято | движок один раз, без рестарта (`actor.rs:826-830`); supervised узлом (`node/dpos.rs:834`); деградация «gossip-only» (`actor.rs:817-831`) убирается сознательно | §5.4 |
| B1 (в) дедуп, (г) ретенция с минтом | принято | `keys.rs:419-430`; ретенция — `ArtifactStore` не вытесняется (§8), минт-исключение беспредметно | §5.3 строка σ (защёлка на `faults()`) |
| B3 (в) `Some(None)`; (а)/(б) правила memo | принято | `carry.rs:70-73,83-95,118-129` | §5.1 `signer`, §5.3 строка PK_E |
| B5 follower (а)(б)(в) | частично | (а) типовая гарантия → рантайм-ветка — принято как цена; (б) = §0.2(б); (в) throttle `follower.rs:284-337` — сохраняется | §5.4 строка «Артефакт отсутствует у не-члена / follower» |
| B6 / B17 terminal pin | принято | правило есть в §5.1, §5.6 противоречил — исправлен; сценарий `epoch_manager.rs:158-169` | §5.3, §5.6 |
| B7 W1 | частично | = §0.4(а) | §5.3, §5.4 |
| B11 поллер (а) первый тик | частично | (б) `watch` коалесцирует — принято; (а) «слеп до первого сетевого Tip» опровергнуто: стартовый Tip из локального архива (`marshal/core/actor.rs:397-401`) — но начальное значение теперь названо | §5.3 строка «часы» |
| B12 tee | частично | = §0.4(б) | §5.3, §7 5.4 |
| B14 `prune_agreements` | принято | = §0.4(г) | §5.3 |
| B15 SafetyHalt защёлка | принято | `epoch_manager.rs:829-852` | §5.3, §6 П-9, §7 5.4 |
| B20 копия в share-файле | частично | = §0.4(д) | §5.4 |
| B24 `absent`/`StaticRandomness`/шов | принято | = §0.4(ж) | §5.1, §5.5, §7 5.0 |
| B26 `ensure_key`/`PinEffort` | принято | разделение `surface.rs:216-224` сохраняется внутри; crash-replay исход назван (`dpos.rs:613-627,846-858`) | §5.1, §5.4 |
| B28 | принято | = A5 | §5.1 |
| B29 | принято | = A10 | §5.1 |
| B30 | принято | = §0.4(в) | §5.1 |
| B31 | принято | = A14 | §5.1 |
| C.1 Д-7 «держится частично» | принято | = §0.5 | §6 Д-7 |
| C.1 §8 п.8 «не по цене» | принято | = §0.4(ж) | §6 §8 п.8 |
| C.1 Ex-7 «непроверяемо» | принято (проверено реестром) | `REGISTER.md:106-109`: Ex-7 «в очереди», не выполнен — условие 5.2 стоит | §6 Ex-7, §7 5.2 |
| C.2 `surface.rs` 1,2 тыс. | принято | свой подсчёт: 1060 (тестовый модуль `surface.rs:834-1625`) | §7 5.1 |
| C.2 «≈ 6 тыс. строк» | принято | свой подсчёт: 6639 удаляется/переписывается + 3563 трогается | §7 (абзац «одним шагом») |
| C.3 5.0 «не держится» | принято | = §0.4(ж) | §7 5.0 (1–1,5 нед) |
| C.3 5.2 «4.0(в) не в „Нужно до“» | принято | стенд без `CertInlet` (`stand.rs:1601-1610`, проза §7 первой версии) | §7 5.2 «Нужно до», §6 «Порядок» |
| C.3 5.4 «не перенос кода» | принято | = §0.4(г), B15 | §7 5.4 (1 нед) |
| C.5 5.0 «35/0 без изменения тестов» | принято | = §0.4(ж) | §7 5.0 |
| C.5 5.0 grep-проверка | принято | тестовые модули в тех же файлах (`cert_inlet.rs:967-968`, `epoch_manager.rs:1849-1851`) | §5.5 (приватные модули = компиляция), §7 5.0 |
| C.5 5.1 «C7 станет зелёным» | принято | `testbed/tests.rs:1948-1950,1962-1985` | §7 5.1, §0.5 |
| C.5 5.2 `a_forged_seed_slot_…` | принято | `testbed/tests.rs:2635`; ротацию стенд без inlet-а не видит | §7 5.2, §0.7(б) |
| C.5 5.4 тест «один фидер» вакуумен | принято | `stand.rs:1757,1912,1932` | §7 5.4, §0.7(а) |
| §D — изменённые мной пункты | D.18 («`geometry ⇒ Err` как направление»): направление сохранено (громко и явно), но `Err` из `build` заменён состоянием `Unfrozen` по A12 — `Err` в точке сборки невозможен (`node/dpos.rs:1690-1703`). Остальные 19 пунктов §D не тронуты |
