# Ревью строки 4.3, заход А — граница p2p (`TrackedPeers{primary, secondary}`, `Ingress`, лимиты `deque_size`)

Ревьюер: Opus 5, свежий контекст, 2026-09-12. Репозиторий `/home/djadjka/Work/fluentbase`,
ветка `djadjka/dpos-reth-2.2-squashed`, база HEAD `86fe709812819d27e15a4c08debda7cc8e69bb59`
(`git rev-parse HEAD`, прогнано). Объект — незакоммиченное рабочее дерево, 12 файлов под
`crates/`, `+1170/−166` (`git diff HEAD --stat -- crates/`, прогнано).

Снимок дерева сверен ДО и ПОСЛЕ ревью: `md5sum -c …/scratchpad/gates/a3f-tree.md5` — все 12
строк `ЦЕЛ` оба раза. Мутаций — 2, обе откачены, md5 совпал (§2).

Пути без префикса — от `crates/dpos/consensus/src/`; `node/` = `crates/node/src/`;
`reader/` = `crates/dpos/staking-reader/src/`; `p2p/` = `crates/dpos/p2p/src/`;
`CW:` = `~/.cargo/git/checkouts/monorepo-27b478c9bb41d208/3c4e02c`.

---

## §0. Прямые ответы

### 1. Ворота — verbatim [KNOWN]

`…/scratchpad/gates/a3f-status.txt` (прочитан 14:0x, содержит `DONE`):

~~~
lib exit 0
stand exit 0
stand-nofeat exit 0
node exit 0
reader exit 0
p2p exit 0
slasher exit 0
fmt exit 0
clippy exit 0 warnings 12
clippy-feat exit 0 warnings 6
doc exit 0 unresolved 6
DONE
~~~

Полные строки результатов — §4. Набор я не перегонял; точечно прогнал четыре теста и две
мутации (§2). **Оговорка к «clippy exit 0 warnings 12»:** четыре из двенадцати предупреждений
принесены этим диффом (P-13), ворота их не валят, потому что `-D warnings` в этом прогоне нет.
«doc unresolved 6» — все шесть в файлах вне диффа (`a3f-doclinks.txt`: `beacon/mod.rs:2`,
`cold_start_jump.rs:775`, `engine.rs:144`, `executor.rs:368`, `:389`, `slasher/evidence.rs:510`),
к заходу отношения не имеют.

### 2. `TrackedPeers` [KNOWN]

**Подпись сменена во всех четырёх местах** — прочитано целиком:
трейт `reader/epoch_transition.rs:182`; два вызова `:845` и `:863`; адаптер
`OracleHandle` `p2p/lib.rs:309-318`; стендовый `TrackSink` `stand.rs:1044-1094`. Плюс два
тест-синка того же крейта (`KeySink` `:1120-1125`, `RecordingSink` `:1128-1136`) и два
вызова в интеграционных тестах (`p2p/tests/convergence.rs:64-72`,
`consensus/tests/slasher_integration.rs:880-890`). Компилируется и зелено (§4).

**Состав primary.** `assemble_tracked_peers` (`:739-762`): `C[E−1]` и `C[E+1]` — через
`push_neighbour_committee` (`:767-795`), читающий `self.reader.epoch_committee_snapshot`;
`C[E]` — из `snap` вызывающего. **Записи берутся у reader'а, не у модуля `committee/`** —
крейт `staking-reader` лежит НИЖЕ consensus в графе (`crates/dpos/consensus/Cargo.toml`
объявляет reader зависимостью), и это честно записано в доке `:735-739` и в журнале как
Д-122. Оба читают один write-once закоммиченный слот, так что по содержанию совпадают.

- **`C[E+1]` не закоммичен:** `Ok(_)` с пустым списком ⇒ `debug!` и тир ПРОПУСКАЕТСЯ
  (`:781-785`); запись просто отсутствует, а не пустая — различие сделано намеренно и
  задокументировано (`:141-147`). Тест `uncommitted_incoming_committee_skips_the_union_and_still_triggers`
  ждёт `(5, 6)` = `C[4] ∪ C[5]`.
- **`C[E−1]` ниже активации/окна:** `epoch.checked_sub(1)` (`:746`) снимает `E = 0`; для
  `E ≥ 1` при нечитаемой записи — тот же пропуск (`Ok(_)` ⇒ `debug`, `Err` ⇒ `warn`).
  Падения нет, граница не встаёт.
- **Последствие пропуска — P-01, SERIOUS.** Ни один из двух соседних чтений не `?`, и
  `last_tracked_epoch` продвигается на успешном `try_send` независимо от деградации
  (`:888`); CW игнорирует повторный `track` на уже зарегистрированный индекс
  (`.claude/COMMONWARE_INTERNALS.md:363`, подтверждено `CW:…/tracker/directory.rs:220-233`);
  `track_peers` защёлкнут одноразовой `peers_tracked` (`node/dpos.rs:1494`, `:1624`, `:1638`).
  Значит деградированный набор живёт ВСЮ эпоху. См. P-01.

**`check_peer_set_size` только по primary — верно по CW [KNOWN].** Прочитано в чекауте:
`CW:p2p/src/authenticated/discovery/actors/tracker/actor.rs:151-164` —
`assert!(peers.primary.len() as u64 <= max, …)` с комментарием «Secondary peers are not
checked here because max_peer_set_size exists to cap the bitvec size, which only covers
primary peers». Вызов на дереве: `:760`.

**Реестровый не-член ушёл из primary — последствия [KNOWN, CW]:**
- `buffered`: `CW:broadcast/src/buffered/engine.rs:312-322` — `insert_message` СНАЧАЛА будит
  ожидающих `subscribe` (`:313-318`), и только потом `latest_primary_peers.position(&peer)
  .is_none() ⇒ Ineligible` (`:319-322`). То есть тела реестрового узла больше не кэшируются,
  но подписчик по названному им самим дайджесту их всё ещё получит (M11 проекта цел; якорь
  проекта `:321-323` сместился на `:319-322`).
- resolver: `CW:resolver/src/p2p/engine.rs:204` — `self.fetcher.reconcile(update.latest.primary
  .as_ref())`; secondary в `participants` не попадает ⇒ у него не фетчат. Обслуживают его при
  этом без тира: `handle_network_request` (`:397-412`) отвечает любому пиру.
- транспорт: `CW:…/tracker/record.rs:170-173` `is_outbound_target() = primary_sets > 0 ||
  persistent`, `:258-266` `dialable` первым делом возвращает `Unavailable` при
  `!is_outbound_target()`, `:341-348` `eligible()` истинно и для secondary. Secondary
  подключается входящим и обслуживается, но его никто не набирает.

**Регрессия для входящего члена `C[E+1]`, чьё тело DKG должно удерживаться `buffered`
(HEAD `reader/epoch_transition.rs:663-669`) — СОХРАНЕНА В УСПЕШНОМ ПУТИ, НО ПОТЕРЯЛА
СТРАХОВКУ.** На HEAD коммент прямо говорил: «The registry branch above covers most incoming
members incidentally; this makes it a guarantee» — то есть покрытие было двойное (реестр +
явное объединение с `C[E+1]`). В дереве реестр ушёл в secondary, и покрытие `C[E+1]` держится
ТОЛЬКО на `push_neighbour_committee(epoch + 1)`. Это и есть P-01.

### 3. `Ingress` [KNOWN]

**Источник окна — тот же набор, что ушёл в `track`, без `Provider::peer_set` на кадр.**
`TrackedWindow` (`p2p/lib.rs:333-341`) — `Arc<RwLock<Option<(u64, TrackedPeers)>>>`, пишется
одним писателем: адаптер `PeerSetSink for OracleHandle` (`:309-318`) делает
`self.window.record(epoch, &peers)` ДО `Manager::track`. Все клоны `OracleHandle` делят одно
хранилище; проводка в ноде: `node/dpos.rs:1316-1324` берёт `handles.oracle.window()`, а ET
строится на `handles.oracle.clone()` (`:1344-1351`) — один Arc, проверено по коду.

**Маска эпох** — `EpochMask` (`p2p/lib.rs:440-470`), массив на три слота, без аллокации;
`Ingress::member_of` `:423`, `Ingress::refusal` `:428` (метка `reason`).

**tombstoned ⇒ `Dropped` до декода:** `classify` проверяет предикат ПЕРВЫМ, до взятия
блокировки (`:385-387`). Источник — `TombstoneSet` (`slasher/tombstone.rs:25-31`), инжектится
`with_tombstones` (`:357-364`) в `node/dpos.rs:1316-1324`. `OracleHandle::block` в
tombstone-поллере не тронут — П-5 проекта цел.

**BEACON — что декодируется ДО проверки членства.** `on_message` (`actor.rs:1882-1925`):
декодируется только `BeaconMessage` (внешний конверт), затем `u64::read_cfg` над первыми
восемью байтами полезной нагрузки — провод `DkgMsg` = `ceremony_epoch(u64) ‖ body_tag(u8) ‖
body`, проверено по `beacon/dkg_msg.rs:86-89` (`write`) и `:138-140` (`read_cfg`). Дорогие
декодеры (`DealerPubMsg` — полином, `SignedDealerLog`) для не-члена не запускаются. Далее
`epoch_is_actionable` (`:1901-1905`) и `beacon_member` (`:1910-1919`).

**`Confirm.target_epoch ∉ [now, now+2]` ⇒ drop до `committee_for`:** `actor.rs:1432-1442`,
чтение комитета — `:1443`. Источник `now` — `self.epoch_of(self.last_height)`, где
`last_height` инициализируется нулём (`:597`) и пишется ТОЛЬКО в `on_height` (`:1156`).
Отсюда P-05.

**EVIDENCE:** `from` прокинут (`node/dpos.rs:1830`, `slasher/gossip.rs:107`); тир-проверка
`:117-123` ДО `decode_batch`; `retains(epoch)` `:149`; `ingress.member_of(epoch)` `:160-169`;
`committee_for` только `:174`. Порядок верный.

**Есть ли путь, где кадр не-члена всё ещё вызывает чтение комитета — ДА, один, и он в
дизайне.** `beacon_member` (`actor.rs:1879`) сам зовёт `(self.committee_for)(epoch)`, а это
фасад модуля (`beacon/plane.rs:586-588` → `reads.committee(epoch, reads.read_at()?)`). Для
эпохи, чья запись ещё не мемоизирована, это ПЕРВОЕ чтение (два staticcall'а), купленное
кадром отслеживаемого не-члена. Границы: только эпохи, прошедшие `epoch_is_actionable`, то
есть ≤ 3 эпохи собственного окна, и каждая — один раз (модуль write-once). Серьёзность —
MODERATE, P-08: буквальная формулировка §5.3 «классификация без EVM» на BEACON не выполнена.
Других путей нет: `on_resolver_message` (`:2350-2384`) зовёт `serve_log`, который комитет не
читает (`:2394-2420`); остальные `committee_for` (`:673`, `:874`, `:1377`, `:1485`, `:1643`,
`:2047`, `:2110`, `:2195`, `:2265`, `:2485`) — на путях, инициированных этим узлом, или на
`pinned_rx`/`Deliver` собственного фетча.

**Гонка «peer-set обновился, окно `Ingress` ещё старое».** По коду её нет в ту сторону:
окно пишется ДО `Manager::track` (`:312-314`), то есть оно опережает commonware, а не
отстаёт. Реальная гонка — другая и она есть (P-07): часы ET (`fin`) и часы beacon-актора
(`fin + K`) разные, поэтому в начале каждой эпохи ≈K блоков актор уже в `T+1` и стартует
церемонию для `T+2`, а окно ещё для `T` (`C[T−1] ∪ C[T] ∪ C[T+1]`) — кадр нового члена
`C[T+2]`, которого нет в трёх записях, классифицируется `Tracked` (он в реестре) и
отбрасывается `GatedReceiver`'ом с `members_only = true`. Повторяет его дилерский
ретрансмит: `actor.rs:1244-1255` («re-send each un-acked dealing point-to-point while
pre-seal … it retransmits on every SUBSEQUENT pre-seal tick until acks drain»), поэтому
окно самозалечивается за ≈K блоков. Для `Confirm` ретрансмита НЕТ (P-05).

### 4. VOTE/CERT backup [KNOWN]

Перепроверено по коду, не по журналу. `observe_route_misses` (`node/dpos.rs:1083-1089`)
разбирает кадр как `(subchannel, _)` — тело выбрасывается, не декодируется; единственное
действие — `record_route_miss` (`:1060-1077`): инкремент `dpos_subchannel_route_miss_total` и
`warn!` по степеням двойки. Все пять backup-приёмников заведены на эту одну задачу
(`:1266-1284`). Контракт/комитет/состояние не трогаются. **Потребителя нет — фильтровать
нечего**, `Ingress` на backup не нужен; §5.3 в этой части устарела (заход В, `48bf62ed`).

`epoch_from_subchannel` (`p2p/constants.rs:101-107`) — **только метка**: `Some(id)` при
`id < DKG_SUBCHANNEL_BASE` (эпохное пространство) ⇒ `kind = "unknown"`, `None` (слайс
согласования) ⇒ `kind = "agreement"` (`node/dpos.rs:1061-1065`). Инверсия только кажущаяся —
имя функции и метка описывают разные вещи; соответствие доке `:1039-1041` сходится.

### 5. Лимиты [KNOWN]

BROADCAST `deque_size` 64 → 4 во всех трёх местах: `dpos.rs:2563`, `dpos.rs:3422`,
`stand.rs:2342`. Потребитель — `outer.rs:696` → `buffered::Config`. Других продовых
конструкций нет (`dkg_agree.rs:2227` `MAX_SET_LEN` — внутри `#[cfg(test)]`-хелпера
`body_mailbox`).

**Байтового лимита у CW нет — подтверждено чтением:** `CW:broadcast/src/buffered/config.rs:5-22`
несёт ровно `public_key`, `mailbox_size`, `deque_size`, `priority`, `codec_config`,
`peer_provider`. `insert_message` считает только длину очереди
(`CW:broadcast/src/buffered/engine.rs:353-359`). `MAX_ORDER_BLOCK_SIZE = 4 MiB`
(`order_block.rs:31`) ⇒ 16 MiB на primary-пира — ВЫВОД, не настройка. §5.3 называет его
второй настройкой рядом с `deque_size` — это расхождение с кодом, записано исполнителем
честно; я подтверждаю.

DKG bodies `MAX_COMMITTEE_SIZE` → 2: `beacon/dkg_transport.rs:135`.

**DKG-стенд-тесты зелёные в воротах оркестратора** (`a3f-stand.txt`, прочитано):
`four_nodes_agree_the_epoch_key_and_carry_the_seed_across_the_boundary`,
`one_absent_dealer_does_not_stop_the_key`, `a_live_dkg_run_reproduces_the_seed_trace_byte_for_byte`,
`three_boundaries_with_committee_rotation_keep_dkg_qual_honest`,
`a_zero_overlap_boundary_halts_the_chain_verify_only`,
`a_two_two_partition_stalls_finalization_and_heals_into_one_chain`,
`a_dealer_with_two_logs_leaves_the_addressed_victim_without_a_share`, плюс
`testbed::preconditions::dkg_bodies_per_peer_are_measured_under_a_partition_in_the_agreement_window`
— все `ok`.

**Перепредложение после nullify при `deque_size = 2` — по CW
`buffered/engine.rs:325-361` [KNOWN]:** политика — вставка в голову, вытеснение с хвоста;
повтор того же дайджеста НЕ растит очередь, а переносится в голову (`:331-337`). Значит при
третьем РАЗНОМ теле от одного отправителя самое старое вытесняется, и подписчик, который
подпишется на него ПОСЛЕ вытеснения, не получит его никогда (`subscribe` будит только на
вставке). Замер Д5 этого не встретил, и сам `E4-PRECONDITIONS.md:93` пишет: «перепредложение
после nullify не наблюдалось, +1 — запас, не замер». Остаточный риск, P-16.

### 6. Стенд [KNOWN]

**Тиры `commonware_p2p::simulated` моделируются наполовину, и ровно так, как написано в
доке теста** — прочитано в чекауте: `register_tracked_peer_set` держит `primary` и
`secondary` порознь и вычитает пересечение из secondary
(`CW:p2p/src/simulated/network.rs:263-313`), `latest_update` отдаёт оба тира подписчику
(`:623-630`). ДОСТАВКУ не тирует: `all_connected_peers` возвращает всех пиров любого тира
(`:638-640`), `is_connectable` спрашивает лишь «есть ли ключ хоть в одном наборе»
(`:643-645`). Транспортную половину закрывает
`testbed::preconditions::a_secondary_peer_on_the_authenticated_transport_is_accepted_and_heard`
на настоящем `authenticated::discovery` — зелёный в воротах.

**Что пинуют два теста.**
- `testbed::tests::a_registry_only_node_is_secondary_on_every_peer_set_and_still_follows`
  (`tests.rs:4131`) — тиринг: на каждом из 4 узлов, для каждого зарегистрированного набора,
  реестровый ключ отсутствует в `primary` и присутствует в `secondary`, все три члена — в
  `primary`, `tracked_mismatches == 0`, узел 3 продолжает исполнять цепь. Есть посылка
  «наборов ≥ 2 на узел» и `checked >= 8`, то есть невакуумность проверена изнутри теста.
- `slasher::gossip::tests::an_evidence_batch_from_outside_the_epochs_committee_resolves_nothing`
  (`gossip.rs:369`) и
  `beacon::actor::clock_tests::a_beacon_frame_from_a_non_member_costs_no_committee_read_beyond_the_check`
  (`actor.rs:4437`) — цена кадра не-члена.

**Мутация красит — проверено мной** (§2): оба юнита падают на точечной мутации своей
проверки. Стенд-тест тиринга я не мутировал (бюджет 2), но его посылка защищена изнутри
(`checked >= 8`, `peer_sets[i].len() >= 2`), а его «половину» независимо красит юнит reader'а
`the_registry_is_tier_two_and_the_outgoing_committee_is_tier_one`.

**«`FakeStaking`-чтения не вызваны» через `StakingReads` — НЕ написано.** `StakingReads`
(`testbed/fakes.rs:887`) используется только в `testbed/committee_tests.rs`; ни один новый тест
его не трогает. PLAN.md:104 просил стенд-тест «кадр не-члена на backup/BEACON/EVIDENCE не
вызывает `FakeStaking`-чтения и не меняет состояние» — вместо него два юнита со своими
счётчиками ближе к шву. Плюс: `GatedReceiver`/`TrackedWindow` в стенде вообще не
участвуют — grep по `testbed/` даёт ноль. P-06.

### 7. Юниты «красный до правки» — воспроизведено [KNOWN]

Мутации ≤ 2, обе точечные, обе откачены; md5 до/после в §2. Результат — оба новых юнита
красные ровно на своём свойстве:

~~~
a_beacon_frame_from_a_non_member_costs_no_committee_read_beyond_the_check
  assertion `left == right` failed: a non-member must cost exactly the one membership check
  left: [6, 6]   right: [6]
an_evidence_batch_from_outside_the_epochs_committee_resolves_nothing
  assertion `left == right` failed: a registry-tier sender must not buy a committee state read
  left: 1   right: 0
~~~

### 8. Гигиена [KNOWN]

Новый `pub`: `TrackedPeers` + `primary`/`epochs_of`/`is_secondary_only` (reader),
`TrackedWindow` + `with_tombstones`/`record`/`epoch`/`classify`, `Ingress` + `member_of`/
`refusal`, `EpochMask` + `contains`/`is_empty`/`iter`, `OracleHandle::window` (p2p),
`GatedReceiver` + `new`, `record_ingress_drop`, `INGRESS_DROPPED_TOTAL` (consensus). Три из
них без продового потребителя (P-18).

`#[allow]` — 0; `todo!`/`unimplemented!` — 0; таймеров/поллинга/счётчиков-штрафов — 0;
`unwrap`/`expect` на продовом пути — 0 (весь `unwrap` в диффе — тестовый). Метрика одна,
метки `&'static str`, кардинальность ограничена.

**Правки вне списка:** `beacon/**` — только `actor.rs` и `dkg_transport.rs` ✔;
`committee/**`, `preconditions.rs`, `epoch_manager.rs`, `outer.rs`, `Cargo.toml`,
`contracts/**` — не тронуты ✔. Два файла вне write-листа постановки исполнителя —
`p2p/tests/convergence.rs` и `consensus/tests/slasher_integration.rs`; обе правки —
механическая адаптация вызова, посылки не ослаблены (P-23).

### 9. Где журнал вводит в заблуждение; расхождения с §5.3; hard-stop'ы

**Журнал (`E4-3-A.md`) вводит в заблуждение в одном месте, остальное честно и с якорями.**

- **«Фальсификатор стал строже, а не слабее»** (§2, п. 2 о тесте 4b) — неверно. Удалённое
  `finalized_delivered == 0` не перепинено нигде (grep по `crates/dpos/consensus/src/`:
  отрицательного утверждения об этом счётчике не осталось), а замер показывает
  `finalized_delivered = 4` — то есть свойство было содержательным и оно потеряно. Порог
  `< 10` → `< 15` при цели членов 16 — послабление. `latest_calls > latest_delivered`
  держится счётом 18 > 17, то есть одним неотвеченным зондом, и выполнимо без всякого
  отсечения (первый зонд до появления соединений). Строже стала ровно одна посылка —
  новая `latest_delivered > 0`. P-02.
- Остальное — корректно. Отдельно отмечу как ЧЕСТНОЕ: §4 журнала сам называет Д-123,
  Д-126, отсутствие юнита на `GatedReceiver` и невыполненную девнет-приёмку; §0(7)
  перечисляет пять расхождений §5.3 с кодом; «конфирмация до первого тика высоты теперь
  отбрасывается» названо «реальным изменением поведения».

**Расхождения с §5.3 (`E4-CORE-DESIGN.md:534-548`)** — подтверждаю все пять из журнала и
добавляю два:
1. Якоря смены подписи сдвинуты (`p2p/lib.rs:306`, `stand.rs:804-809`, «вызов … `:647`» —
   вызовов два). Подтверждено.
2. «VOTE/CERT backup — `Member`, иначе drop» устарело: потребителя нет (§0(4)).
3. «`deque_size = 4` + байтовый лимит» — байтовый лимит невыразим в `buffered::Config`.
4. Якоря CW сдвинулись на единицы. Подтверждено (`:319-322`, `:313-318`, `:204`).
5. «из записей модуля» для ET невыполнимо без правки `Cargo.toml` (Д-122).
6. **Новое:** «классификация без EVM» на BEACON не выполнена буквально — `beacon_member`
   читает через модуль (P-08).
7. **Новое:** §5.3 нигде не оговаривает, что чтение соседней записи фаллибельно и что
   деградация не ретраится внутри эпохи; после ухода реестра в secondary это перестало быть
   безобидным (P-01).

**Hard-stop'ы оркестратора — все три НЕТ:**
- **(2) П-5/П-4 менять?** НЕТ. П-5 (`Ingress::Dropped` вместо `OracleHandle::block`)
  реализован именно так: `classify` отдаёт `Dropped` (`p2p/lib.rs:385-387`), а разрыв
  соединения остаётся за tombstone-поллером через `OracleHandle::block` — `Blocker for
  OracleHandle` (`p2p/lib.rs:484+`) не тронут. П-4 (лестница/ступень `Finalized{last(T+1)}`
  у `committee(T+1) ⊂ primary`) не ослаблен: `C[T+1]` остался в primary, `C[T−1]` добавлен —
  адресация ступени только расширилась.
- **(3) BLOCKER без ответа проекта?** НЕТ. Самая тяжёлая находка (P-01, SERIOUS) имеет
  дешёвый ответ внутри рамок 4.3 (§5 ниже) и не требует пересмотра проекта.
- **(4) Стенд опроверг проект?** НЕТ. Стенд подтвердил тиринг и не опроверг ни одного
  утверждения §5.3. Единственное, что стенд ПОКАЗАЛ против ожидания — что выбывший узел
  живёт на эпоху дольше (тест 4b), и это следствие самого проекта (`C[E−1]` в primary), а
  не опровержение.

---

## §1. Находки

| id | сер. | file:lines (дерево) | HEAD-якорь | Что не так | Чем пытался опровергнуть | увер. |
|---|---|---|---|---|---|---|
| P-01 | SERIOUS | `reader/epoch_transition.rs:767-795`, `:739-762`, `:888`; `dpos.rs:110-126`; `node/dpos.rs:1888-1898` | `reader/epoch_transition.rs:663-669` | Покрытие `C[E+1]` (и теперь `C[E−1]`) стало ОДНОИСТОЧНИКОВЫМ. На HEAD реестр входил в primary и, по собственному комменту, «covers most incoming members incidentally; this makes it a guarantee». Реестр ушёл в secondary, а `push_neighbour_committee` не `?`: `Ok(пусто)` ⇒ `debug`, `Err` ⇒ `warn`, тир пропущен. `last_tracked_epoch` при этом продвигается (`:888`), CW игнорирует повторный `track` на тот же индекс, `track_peers` защёлкнут (`node/dpos.rs:1494/1624/1638`) — деградация живёт ВСЮ эпоху. Два следствия сразу: (а) `buffered` не удерживает тела предложений инстанса согласования `E+1` (`CW:broadcast/src/buffered/engine.rs:319-322`) ⇒ `verify` паркуется на теле, которого не будет, «с нечем выше `debug`» — ровно тот отказ, который HEAD-коммент и описывал; (б) НОВОЕ — `GatedReceiver(members_only = true)` классифицирует этих членов как `Tracked` (они в реестре) и роняет их BEACON-кадры до декода. Сценарии не гипотетические: при `E ≤ 2` / на холодном старте `C[E+1]` штатно ещё не закоммичен (`uncommitted_incoming_committee_skips_the_union_and_still_triggers` это и фиксирует). | Искал ретрай: `track_and_trigger` (`:859-890`) ретраится только по `TriggerResult::Full/Closed`, деградация чтения к этому не приводит; `track_peers` — одноразовый; `Manager::track` на тот же индекс молча игнорируется (`COMMONWARE_INTERNALS.md:363`, `CW:…/tracker/directory.rs:220-233`). Искал вторую страховку в дереве — её нет: `GatedReceiver` — единственный шов, и он читает то же окно. Дилерский ретрансмит (`actor.rs:1244-1255`) не спасает: кадр роняют на приёме, а не теряют в сети | [KNOWN] по коду; масштаб ущерба — [ГИПОТЕЗА], живьём не воспроизводил |
| P-02 | MODERATE | `testbed/tests.rs:505-580` (посылки `:546`, `:566-575`) | `testbed/tests.rs:505-562`, посылки `:540`, `:545-552` | Фальсификатор 4b стал СЛАБЕЕ, вопреки журналу. (а) `finalized_delivered == 0` удалён без замены — grep по `crates/dpos/consensus/src/` не находит ни одного отрицательного утверждения об этом счётчике; замер даёт `finalized_delivered = 4`, то есть свойство было содержательным. (б) `heights[3] < 10` → `< 15` при цели членов `>= 16` — зазор в один блок. (в) `latest_calls > latest_delivered` держится счётом 18 > 17 и выполняется одним неотвеченным зондом; первый зонд до установления соединений даёт это и БЕЗ отсечения, так что посылка не различает «потерял пиров» и «ещё их не имел». (г) `heights[3] < heights[0]` — не свойство, а любая отставшесть. Механизм (`C[E−1]` держит выбывшего ещё эпоху) по §5.3 ВЕРЕН и проверен: `shrink_to_three` (`tests.rs:453-461`) даёт node 3 ∈ `C[0]` только, primary при `E = 1` = `C[0] ∪ C[1] ∪ C[2]` ∋ node 3, при `E = 2` — нет | Прогнал сам с `--nocapture`: `heights=[16,16,16,8]`, `u3 = {latest_calls: 18, latest_delivered: 17, finalized_calls: 5, finalized_delivered: 4, deliveries_decoded: 21}`. Искал перепин снятого свойства в (4c)/(4d) — `tests.rs:623` и `:2377` утверждают `finalized_delivered > 0` (положительные), `:3989` — `finalized_calls > finalized_delivered`; отрицательной формы нет нигде | [KNOWN] |
| P-03 | MODERATE | `beacon/actor.rs:60-119` | `beacon/actor.rs:60-117` | Новая константа `BEACON_CHANNEL_LABEL` (`:117`) вставлена ВНУТРЬ док-комментария `DKG_MARGIN_BLOCKS`: ~20 строк обоснования бюджета (`T_agree`, история v41/AMENDMENT 5, «do NOT raise this … without first re-measuring») теперь документируют строковую метку `"beacon"`, а `DKG_MARGIN_BLOCKS` (`:119`) остался БЕЗ доки. Это не косметика: коммент — единственное место, где записано, почему 20 нельзя поднимать | Прочитал `:100-125` целиком; `rustdoc` привязывает весь блок `///` к следующему элементу, им стал `BEACON_CHANNEL_LABEL` | [KNOWN] |
| P-04 | MODERATE | `slasher/gossip.rs:353-369` | `slasher/gossip.rs:353-357` | Тот же класс: трёхстрочная дока теста `a_batch_outside_the_retained_window_is_refused_before_any_committee_is_resolved` («The epoch a forwarded batch names is chosen by its sender …») теперь стоит ГОЛОВОЙ доки нового теста `an_evidence_batch_from_outside_the_epochs_committee_resolves_nothing` (`:369`), а сам старый тест (`:466`) остался без доки. Две разные посылки склеены в один блок | Прочитал `:339-380` целиком | [KNOWN] |
| P-05 | MODERATE | `beacon/actor.rs:597`, `:1156`, `:1432-1442`, `:1901-1905`; `beacon/confirmations.rs:184` | `beacon/actor.rs:1409-1434` (без окна) | `last_height` засеивается нулём при конструировании (`:597`) и пишется ТОЛЬКО в `on_height` (`:1156`); больше писателей нет (grep). Актор строится уже после заморозки геометрии (`beacon/plane.rs:821-840`), так что `epoch_of` осмыслен, но до первого дренированного тика `now = epoch_of(0) = 0`, и окно `[0, 2]` роняет ВСЁ: DKG-кадры с `reason = "epoch"`, конфирмации с `reason = "confirm_window"`. Окно узкое — поллер кладёт `cs_fin_num + K` в буферизованный канал ещё до создания актора (`node/dpos.rs:1494`), — но `tokio::select!` выбирает готовую ветку случайно, так что первый кадр может быть обработан раньше первого тика. Для дилингов это чинит ретрансмит (`:1244-1255`); **для `Confirm` — нет**: `Confirmations::mint` не переизлучает ту же ширину (`confirmations.rs:184` — `previous.is_some_and(|last| last >= confirmed.len()) ⇒ continue`), поэтому конфирмация ПОЛНОЙ ширины, оброненная в этом окне, у этого узла потеряна навсегда, и его планка входа недосчитывает одного члена на всю эпоху. Отличить «высота 0, потому что ничего не тикало» от «цепь на нуле» актор не может | Искал второго писателя `last_height` — нет. Искал переизлучение конфирмаций — `mint` вызывается из `ConfirmTrigger::{Decisive, AnyGrowth}`, оба на РОСТЕ; memo `confirmed_len` глушит повтор той же ширины. Пробовал списать на «окно в одну итерацию select» — не снимает: последствие постоянное, а не транзиентное | механизм [KNOWN]; вероятность и цена — [ГИПОТЕЗА], не воспроизводил |
| P-06 | MODERATE | `testbed/stand.rs` (весь), `testbed/tests.rs:4131` | — | Шов `Ingress` на стенде не участвует вообще: grep по `crates/dpos/consensus/src/testbed/` даёт ноль вхождений `GatedReceiver` и `TrackedWindow`. `GatedReceiver` не покрыт НИ ОДНИМ тестом (ни юнитом, ни стендом) — единственное доказательство его работы — компиляция. Запрошенный `PLAN.md:104` стенд-тест «кадр не-члена … не вызывает `FakeStaking`-чтения» через `StakingReads` (`testbed/fakes.rs:887`) не написан; `StakingReads` используется только в `testbed/committee_tests.rs`. Заменители (два юнита со своими счётчиками) проверяют ПОЭПОХНЫЙ шов, а не транспортный | Искал непрямое покрытие: EVIDENCE-юнит гоняет `ingest_batch` напрямую, минуя `GatedReceiver`; стенд-тест тиринга проверяет содержимое `TrackedPeers`, а не классификацию кадра. Признак «tombstoned не доходит до `recv`» не пинует никто | [KNOWN] |
| P-07 | MODERATE | `beacon/actor.rs:1878-1919`; `p2p/lib.rs:384-407`; `node/dpos.rs:1888-1898` | — | Д-123 в коде: на BEACON два источника классификации. Транспортный шов — `TrackedWindow` (набор ET, часы = `fin`); поэпохный — `committee_for` (модуль, часы актора = `fin + K`). Расхождения реальны в обе стороны. (а) Окно ПУСТОЕ, `committee_for` отвечает: `classify` возвращает `None`, `GatedReceiver::admits` пропускает ВСЁ (`dpos.rs:112-114`), и единственным барьером остаётся `epoch_is_actionable` + `beacon_member` — безопасно, но фактического тира нет весь холодный старт. (б) Окно ОТСТАЁТ: в первые ≈K блоков эпохи актор уже в `T+1` и стартует церемонию `T+2` (`:1257` `maybe_start(now + 1)`), а окно ещё для `T`; кадр нового члена `C[T+2]` ⇒ `Tracked` ⇒ роняется, хотя `committee_for(T+2)` его признаёт. (в) Окно ДЕГРАДИРОВАЛО (P-01) — то же, но на всю эпоху. Никто это расхождение не ловит и не считает: метрика `dpos_ingress_dropped_total{channel="beacon", reason="secondary"}` неотличима от легитимного отброса | Проверил, что (б) самозалечивается: дилерский ретрансмит (`:1244-1255`) повторяет неотквитованный дилинг на каждом pre-seal тике. Проверил, что `Confirm` в (б) не страдает: конфирмации для `T+2` минтятся членами `C[T+2]`, и до трека `T+1` их ещё нет в наборе — но их ширина растёт, значит `mint` их переиздаст | [KNOWN] по коду |
| P-08 | MODERATE | `beacon/actor.rs:1878-1880`, `:1901-1905`; `beacon/plane.rs:586-588` | — | §5.3 требует «классификация без EVM: по `TrackedPeers` текущего окна». На BEACON это не так: `beacon_member` зовёт `(self.committee_for)(epoch)` → фасад модуля → при промахе два staticcall'а. То есть отслеживаемый НЕ-член, назвав `now+1`/`now+2`, может купить ПЕРВОЕ чтение записи этой эпохи. Ограничено тремя эпохами окна и одним разом каждая (модуль write-once), поэтому не DoS — но буква §5.3 не выполнена, и в модуле есть ровно нужный метод `Committee::is_member(epoch, peer)` (`committee/mod.rs:294`), которого фасад `CommitteeReads` не отдаёт | Проверил по тесту `a_beacon_frame_from_a_non_member…`: для эпохи 10^9 — ноль чтений (ограничитель `epoch_is_actionable` работает), для эпохи в окне от не-члена — ровно одно. То есть «ноль» держится только вне окна | [KNOWN] |
| P-09 | MINOR | `reader/error.rs:92` | тот же | Операторское сообщение `PeerSetTooLarge` по-прежнему говорит `tracker peer-set size {size} (registry ∪ committee)`, тогда как счёт теперь ведётся по primary = три комитета (`epoch_transition.rs:760`). Оператор, поймавший эту ошибку, будет искать раздутый реестр | Проверил, что файл не в диффе и что текст был бы верен до правки | [KNOWN] |
| P-10 | MINOR | `p2p/constants.rs:46`, `:173-182`; `p2p/config.rs:95` | те же | Обоснование `MAX_REGISTRY_PEER_SET = 4096` целиком стоит на «the FULL Active validator registry ∪ current committee is tracked» — после 4.3 реестр в бит-век не входит, и гвардия считает ≤ 3 × `MAX_COMMITTEE_SIZE` = 153. Значение безвредно-щедрое, но его причина мертва; `config.rs:95` тянет тот же устаревший коммент `// tracker feed = registry ∪ committee` | Проверил по CW, что бит-век действительно только по primary (`tracker/actor.rs:155-158`) — то есть коммент описывает мир, которого больше нет | [KNOWN] |
| P-11 | MINOR | `node/dpos.rs:1186-1189` | те же строки | Внутри ФАЙЛА ИЗ ДИФФА остался коммент «a validator rotated out of the committee still sits in its peers' tracked set (registry ∪ committee ∪ committee+1)». Формула сменилась в этом же заходе | Прочитал `:1178-1200`; правка комментария не сделана | [KNOWN] |
| P-12 | MINOR | `beacon/log_resolver.rs:14-18`, `:102`; `beacon/plane.rs:488`; `beacon/dkg_engine.rs:517` | те же | Три доки строят аргумент достижимости на «`active_registry_peers ∪ committee[E]` … the log holders are in `latest.primary` via the registry union». Механизм умер; ВЫВОД уцелел по другой причине — при треке `E−1` primary = `C[E−2] ∪ C[E−1] ∪ C[E]` ∋ держатели логов. То есть доки теперь объясняют верный факт неверной причиной | Проверил вывод сам: во время `E−1` (дилинг `committee[E]`) `C[E]` входит в primary как `C[(E−1)+1]`. `log_resolver.rs` и `dkg_engine.rs` — вне write-листа исполнителя, `plane.rs` тоже | [KNOWN] |
| P-13 | MINOR | `p2p/lib.rs:340`; `beacon/actor.rs:4492`; `testbed/stand.rs:581`, `:1032` | — | Заход принёс новые clippy-предупреждения: `type_complexity` на `tombstoned: Option<Arc<dyn Fn(&PeerPubkey) -> bool + Send + Sync>>`, `useless_conversion` к `alloy_rlp::Bytes` в новом тесте, и два `type_complexity` на разросшихся кортежах стенда. Ворота не валятся (`clippy exit 0 warnings 12`, `-D warnings` не включён), но `epoch_transition.rs:184-186` прямо декларирует стиль «to stay clean under `-D warnings`» | Сверил с `a2v-clippy.txt`/`a2-clippy.txt`: там только `large size difference` в `node/dpos.rs` — но те прогоны шли по более узкому кэшу, поэтому «новыми» уверенно называю первые два (кода на этих строках на HEAD не было); для двух стендовых — вывод по смене типа с 2-кортежа на 3-кортеж | первые два [KNOWN], стендовые [ГИПОТЕЗА] |
| P-14 | MINOR | `p2p/lib.rs:366-371`, `:384-407` | — | `TrackedWindow` отказывает МОЛЧА и В ОТКРЫТУЮ при отравленной `RwLock`: `record` теряет обновление (`if let Ok(mut slot) = self.latest.write()`), `classify` возвращает `None` ⇒ `GatedReceiver::admits` пропускает всё (`dpos.rs:112-114`) и `ingest_batch` тоже (`gossip.rs:117-123`). Ни метрики, ни лога. Отравление требует паники под блокировкой — критические секции крошечные, — но исход «весь шов выключен навсегда, никто не знает» непропорционален | Проверил все три читателя; поведение `None` = «мнения нет» задокументировано намеренно для холодного старта, и именно поэтому отравление неотличимо от холодного старта | [KNOWN] |
| P-15 | MINOR | `p2p/lib.rs:446-455`; `reader/epoch_transition.rs:138-147` | — | `EpochMask::of` МОЛЧА обрезает на трёх слотах (`if (mask.len as usize) < mask.slots.len()`), а инвариант «ровно три записи» держится только построением в `assemble_tracked_peers`. `TrackedPeers.committees` — `pub Vec<(u64, Set)>` без проверки длины и без проверки уникальности эпох; его уже собирают вручную в двух местах (`gossip.rs:391-406` — две записи, `p2p/tests/convergence.rs:66-69` — одна). Ни `debug_assert!`, ни доккомментарий-инвариант на публичном поле | Проверил, что сегодня >3 не строит никто; то есть это не баг, а незащищённый инвариант публичного типа | [KNOWN] |
| P-16 | MINOR | `beacon/dkg_transport.rs:122-136` | `beacon/dkg_transport.rs:123` | Число `deque_size = 2` не покрыто ни одним юнитом: тестовый хелпер согласования строит свой `buffered::Engine` с `deque_size: MAX_SET_LEN` (`beacon/dkg_agree.rs:2227`), так что ~30 тестов `dkg_agree::tests::agreement::*` (включая `propose_rebuilds_when_a_late_dealer_log_lands`, `propose_re_proposes_the_certified_value`, `a_late_propose_task_cannot_replace_a_newer_rounds_body`) гоняются на 51-глубоком кэше. Остаются стенд и замер Д5, про который `E4-PRECONDITIONS.md:93` сам пишет: разрез не доказан покрывшим фазу согласования, перепредложение после nullify не наблюдалось. По CW третье РАЗНОЕ тело от одного отправителя вытесняет первое (`CW:broadcast/src/buffered/engine.rs:353-359`), и подписчик, подписавшийся после вытеснения, его уже не получит | Прочитал политику вытеснения в чекауте; проверил, что повтор дайджеста очередь не растит (`:331-337`); проверил по воротам, что все DKG-стенд-тесты зелёные — то есть на измеренных формах запаса хватает | механизм [KNOWN]; достаточность 2 — [ГИПОТЕЗА] |
| P-17 | NIT | `p2p/lib.rs:409-418`; `p2p/lib.rs:553-556` | — | Новый публичный `fluentbase_p2p::Ingress` конфликтует по имени с `commonware_p2p::Ingress` (адрес дозвона), который используется в ЭТОМ ЖЕ крейте (`p2p/ingress.rs`, `p2p/bootstrappers.rs`, `p2p/config.rs`) и в `node` (`node/dpos.rs:1160`, `node/cert_follow/mod.rs:200`). Внутри `mod tests` того же файла локальный `Ingress` уже затенён явным импортом CW-типа поверх `use super::*` — компилируется, но означает, что в этом модуле новый тип по имени недостижим | Проверил, что сборка проходит (glob уступает явному импорту); имя взято из §5.3, так что это цена проекта, а не отсебятина | [KNOWN] |
| P-18 | NIT | `p2p/lib.rs:466-468`, `:373-378`; `reader/epoch_transition.rs:169-171` | — | Публичный API без продового потребителя: `EpochMask::iter` — вызовов нет вообще; `TrackedWindow::epoch` — только юнит `p2p/lib.rs:643-648`; `TrackedPeers::is_secondary_only` — только тест `epoch_transition.rs:1208` | Прогнал grep по всем `*.rs` в `crates/` | [KNOWN] |
| P-19 | NIT | `beacon/actor.rs:1853-1864` vs `:1845-1848` | — | Дока `epoch_is_actionable` утверждает: «The SAME predicate the ceremony dispatch and [`Self::is_bufferable`] apply further down». Не тот же: `is_bufferable` использует `epoch <= now \|\| epoch > now + 2`, то есть `[now+1, now+2]`; `epoch_is_actionable` — `[now, now+2]` плюс идущие церемонии. Надмножество, так что безопасно, но утверждение ложно | Проверил обе строки; проверил, что при `epoch == now` без церемонии кадр всё равно упадёт ниже (нет ветки) — то есть поведение корректное, неверна только дока | [KNOWN] |
| P-20 | NIT | `beacon/actor.rs:1405-1411` vs `:1910-1919` | тот же коммент на HEAD | Дока `on_confirm` заявляет: «a relayed confirmation is as good as a directly-sent one and the sender is only ever a diagnostic». С 4.3 это неверно — `beacon_member(from, ceremony_epoch)` требует отправителя в `committee[target_epoch]` (равенство `target_epoch == envelope_epoch` проверено `:1417`), так что реле от не-члена роняется. Поведенчески безвредно: в дереве реле конфирмаций нет — единственный эмитент `confirmations.rs:190-200` подписывает своим `me_key` и только будучи в ростере | Прогрепал `DkgBody::Confirm` по всему `crates/dpos/consensus/src/` — конструируется в `confirmations.rs:198`, `dkg_msg.rs:275` (тест), `actor.rs:4494` (новый тест) и разбирается в `byzantine_roles.rs:159`. Продового пересыла нет | [KNOWN] |
| P-21 | NIT | `beacon/actor.rs:1432-1433` vs `:1853-1864` | — | Два окна в одном акторе разошлись: `epoch_is_actionable` допускает эпоху ИДУЩЕЙ церемонии даже ниже `now`, а `on_confirm` — только `[now, now+2]`. Церемония, ещё открытая для эпохи ниже `now`, пропускает свои DKG-кадры, но роняет свои конфирмации. Асимметрия нигде не оговорена | Искал, может ли церемония пережить `now`: сметание идёт в `on_height` после финализации (`:2024-2060`), так что окно между «часы ушли» и «церемония снесена» существует | [KNOWN] |
| P-22 | MINOR | `reader/epoch_transition.rs:783-789` | `:667-673` (тот же текст) | Дока говорит: «a degraded peer set that the next finalized block re-reads is strictly better than a stalled epoch». Для пути границы это неверно — перечитывания нет (см. P-01: `last_tracked_epoch` продвинулся, повторный `track` на тот же индекс CW игнорирует). Текст достался от HEAD, но после ухода реестра из primary он несёт вес, которого не выдерживает | Проверил ретрай-контур `on_finalized`/`PENDING_RETRY_BACKOFF` (`dpos.rs:2468-2485` по ссылке доки) — он про `TriggerResult::Full`, не про деградацию чтения | [KNOWN] |
| P-23 | NIT | `p2p/tests/convergence.rs:64-72`; `consensus/tests/slasher_integration.rs:880-890` | — | Два файла вне write-листа постановки исполнителя отредактированы. Раскрыто им самим; обе правки — чистая адаптация вызова под новую подпись, ни одной посылки не ослаблено (в `slasher_integration` передано пустое окно «мнения нет», что оставляет смысл теста прежним) | Прочитал обе правки целиком и сверил, что ассерты не тронуты | [KNOWN] |
| P-24 | MINOR | `slasher/gossip.rs:117-123` vs `node/dpos.rs:1814-1822` | — | EVIDENCE загорожен ДВАЖДЫ: `GatedReceiver(members_only = true)` уже роняет `Tracked`/`Dropped`, значит тир-проверка внутри `ingest_batch` в проде недостижима (до неё доходит только `Member` или `None`). Дублирование безвредно, но распределение покрытия перевёрнуто: тестами покрыт мёртвый в проде шов, а живой (`GatedReceiver`) — ничем (P-06) | Проверил, что `ingest_batch` вызывается из одного места в проде (`node/dpos.rs:1830`) и что этот приёмник — `GatedReceiver`. Живым остаётся `member_of(epoch)` (`:160-169`) — он в `GatedReceiver` невыразим | [KNOWN] |
| P-25 | MINOR | `testbed/stand.rs:1050-1080` | `testbed/stand.rs:1042-1068` | `tracked_mismatches` ослаб: `by_epoch` теперь хранит `connectable` = primary ∪ secondary (`:1056-1064`), и сравнение идёт по ОБЪЕДИНЕНИЮ. Два узла, согласные про объединение, но разошедшиеся в РАЗБИЕНИИ по тирам, счётчик не поймает — а разбиение и есть предмет 4.3. `the_epoch_transition_walks_the_boundaries_from_the_fake_state` (`tests.rs:2055-2058`) опирается на этот счётчик и только на него | Проверил, что новый тест тиринга компенсирует это в СВОЁМ сценарии (проверяет содержимое `primary`/`secondary` на каждом узле), но `the_epoch_transition_…` — нет. Причина смены на объединение честная и записана: модель разрыва связей читает `by_epoch`, а secondary тоже соединяется | [KNOWN] |

---

## §2. Поведение по ханкам (что делает каждый кусок и чем это проверено)

**`reader/epoch_transition.rs`.** Новый тип `TrackedPeers{committees, secondary}` (`:138-147`)
с `primary()` (`:151-157`), `epochs_of()` (`:161-167`), `is_secondary_only()` (`:169-171`).
Записи хранятся ПОРОЗНЬ ради маски — объединение на вопрос «в какой из трёх» не отвечает.
Подпись трейта `:182`. `assemble_tracked_peers` (`:739-762`) собирает три записи и реестр,
размерная проверка по `primary()` (`:760`). `push_neighbour_committee` (`:767-795`) — оба
соседа, оба некритичные. Вызовы `:845`, `:863`. Тесты крейта пересчитаны под новый союз
(MockReader даёт попарно непересекающиеся комитеты): 8 → 9, 3 → 6, 10 → 15, 20 → 30 —
проверил каждое число арифметикой по `MockReader`; ни одна посылка не снята, кроме той, что
и была смыслом правки (реестр больше не в primary).

**`p2p/lib.rs`.** `OracleHandle` получил поле `window` (`:111`), адаптер пишет окно ДО
`Manager::track` (`:312-314`) и передаёт CW `TrackedPeers::new(primary, secondary)` (`:315`).
`TrackedWindow` (`:333-407`), `Ingress` (`:409-435`), `EpochMask` (`:437-470`),
`OracleHandle::window()` (`:478-483`). Ручной `Debug` (`:343-352`) — не печатает набор.

**`consensus/dpos.rs`.** Метрика `INGRESS_DROPPED_TOTAL` + `record_ingress_drop` (`:63-68`).
`GatedReceiver<R>` (`:89-140`): `admits` (`:110-126`) — `None` ⇒ пропустить, `Member` ⇒
пропустить, `Tracked` ⇒ по `members_only`, `Dropped` ⇒ уронить + счётчик; `recv` (`:134-140`)
крутит цикл, пока кадр не пройдёт. `deque_size` 64 → 4 (`:2563`, `:3422`) с длинным
комментарием, включающим честное «There is NO byte cap to pair it with».

**`node/dpos.rs`.** `ingress_window` (`:1316-1324`) = окно оракула + предикат тумбстоунов.
EVIDENCE (`:1813-1822`) и BEACON (`:1885-1897`) обёрнуты `GatedReceiver`'ом с
`members_only = true`; `from` прокинут в `ingest_batch` (`:1830`, `:1838`).

**`beacon/actor.rs`.** `BEACON_CHANNEL_LABEL` (`:117` — см. P-03); окно `[now, now+2]` в
`on_confirm` (`:1432-1442`); `epoch_is_actionable` (`:1853-1864`); `beacon_member` (`:1866-1880`);
заголовочный пик + две проверки в `on_message` (`:1892-1919`) и `debug_assert_eq!` на
совпадение пика с декодом (`:1925-1928`). Правка старого теста
`share_confirmations_are_minted_on_growth_and_taken_only_from_their_signer` (`:4753-4760`) —
две строки, ставящие `last_height = 100`, плюс `assert_eq!(actor.epoch_of(...), TARGET - 1)`,
чтобы посылка была видимой. Посылки теста не тронуты; это правильная форма — предпосылка
сделана явной, а не обойдена.

**`beacon/dkg_transport.rs`.** `deque_size: 2` (`:135`) с измерением в комментарии; якорь CW
поправлен с `:298` на `:319-322` — проверил, верно.

**`slasher/gossip.rs`.** `EVIDENCE_CHANNEL_LABEL` (`:39`), `from` и `window` в сигнатуре
(`:106-113`), тир-проверка `:117-123`, `member_of` после `retains` (`:158-169`).

**`testbed/stand.rs`.** `Outcome::tracked` → `peer_sets` с тремя полями (`:581`); `TrackSink`
пишет оба тира и форвардит `commonware_p2p::TrackedPeers::new` (`:1044-1094`);
`deque_size: 4` (`:2342`).

**`testbed/tests.rs`.** Переписан 4b (P-02), пересчитан C2-блок (`:2046-2068`), добавлен
стенд-тест тиринга (`:4101-4195`).

### Мутации (2), откат сверен по md5

md5 ДО (и он же снимок оркестратора `a3f-tree.md5`):

~~~
16682ba27e9c4aef7843a4e805acbcd7  crates/dpos/consensus/src/beacon/actor.rs
cf08265ee9acf2e748746a00d2daff7b  crates/dpos/consensus/src/slasher/gossip.rs
~~~

* **М1** — `beacon/actor.rs:1879`:
  `roster.position(from).is_some()` → `roster.position(from).is_some() || true`
  (проверка членства снята, чтение комитета сохранено — чтобы красил именно шов, а не счётчик).
  Красит `a_beacon_frame_from_a_non_member_costs_no_committee_read_beyond_the_check`:
  `left: [6, 6], right: [6]` — «a non-member must cost exactly the one membership check».
* **М2** — `slasher/gossip.rs:117`:
  `window.classify(from)` → `window.classify(from).and(None)`
  (обе проверки — тир и `member_of` — обезврежены одним токеном).
  Красит `an_evidence_batch_from_outside_the_epochs_committee_resolves_nothing`:
  `left: 1, right: 0` — «a registry-tier sender must not buy a committee state read».

md5 ПОСЛЕ отката — те же две суммы; `md5sum -c a3f-tree.md5` — все 12 строк `ЦЕЛ`.

### Собственный прогон 4b (`--nocapture`)

~~~
(4b) heights=[16, 16, 16, 8] epoch-first-block views=[(1, 1), (2, 1), (3, 1)]
upstream=[…, UpstreamStats { latest_calls: 18, latest_delivered: 17, finalized_calls: 5,
finalized_delivered: 4, serve_requests: 0, deliveries_decoded: 21, deliveries_rejected: 0,
rejump_calls: 0 }] virtual=15.924s real=1.361850132s
test testbed::tests::a_node_outside_the_tracked_peer_set_stands_at_the_boundary ... ok
~~~

---

## §3. Граница

**Что я проверил сам, целыми строками:** весь дифф по 12 файлам; `reader/epoch_transition.rs`
`:119-200`, `:689-900`, `:1100-1260`, `:2940-2975`; `p2p/lib.rs:100-120`, `:255-270`,
`:299-490`, `:546-660`; `consensus/dpos.rs:57-140`, `:2549-2575`, `:3408-3430`;
`node/dpos.rs:1000-1020`, `:1039-1110`, `:1178-1300`, `:1310-1360`, `:1455-1660`, `:1795-1900`;
`beacon/actor.rs:100-125`, `:400-445`, `:590-600`, `:725-760`, `:795-830`, `:1149-1160`,
`:1240-1275`, `:1375-1500`, `:1670-1712`, `:1820-1990`, `:2340-2400`, `:2475-2500`, `:4420-4560`,
`:4740-4770`; `beacon/plane.rs:580-600`, `:800-870`; `beacon/dkg_msg.rs:75-152`;
`beacon/confirmations.rs:175-215`; `slasher/gossip.rs:1-180`, `:330-520`;
`testbed/stand.rs:574-600`, `:1025-1100`, `:2330-2350`; `testbed/tests.rs:450-600`, `:2040-2070`,
`:4098-4195`; `p2p/constants.rs:80-115`, `:155-195`; `reader/error.rs:85-100`.

**Чекаут commonware** (после `.claude/COMMONWARE_INTERNALS.md:351-365`, `:378`, `:395-397`,
`:445-446`): `p2p/src/lib.rs:320-356`; `p2p/src/authenticated/discovery/actors/tracker/actor.rs:150-175`;
`…/tracker/record.rs:160-180`, `:255-275`, `:335-355`; `broadcast/src/buffered/engine.rs:300-365`;
`broadcast/src/buffered/config.rs:1-30`; `resolver/src/p2p/engine.rs:196-212`, `:392-415`;
`p2p/src/simulated/network.rs:255-320`, `:600-646`.

**Прогоны, которые сделал я:** `a_node_outside_the_tracked_peer_set_stands_at_the_boundary`
(`--nocapture`); три юнита (`a_beacon_frame_from_a_non_member…`,
`an_evidence_batch_from_outside…`, `a_batch_outside_the_retained_window…`) — зелёные; те же два
под двумя мутациями — красные; `md5sum -c` дважды. Всё на `CARGO_BUILD_JOBS=4`.

**Чего я НЕ делал и что поэтому осталось неподтверждённым.** Полный набор ворот не
перегонял — беру из файлов оркестратора. Девнет не гонял (дозвон secondary по `dialable` —
приёмка 4.3, открыта). Не проверял `.claude/dpos_architecture/` на дрейф — это за
оркестратором, но по симптомам P-09…P-12 дрейф терминологии «registry ∪ committee» широкий и
разбор доков стоит сделать отдельным проходом. Не воспроизводил P-01 и P-05 живьём — оба
описаны механизмом по коду, не наблюдением. Не читал `contracts/**`, `devnet/**`.
`git` — только чтение (`rev-parse`, `status`, `diff`, `show`).

**Где моя собственная проверка слабее всего.** (1) P-13 в части двух стендовых clippy —
вывод, не сверка с прогоном на HEAD (прогон потребовал бы мутации дерева). (2) Вероятность
окна P-05 — не измерена; я показал только, что механизма переизлучения конфирмации нет.
(3) Достаточность `deque_size = 2` при перепредложении — не воспроизводил; опираюсь на
политику вытеснения CW и на самопризнание Д5. (4) Я не перечитывал §10 проекта целиком —
только строки по grep (§0.6, §0.8, A38, A63, A64, A73, C.3).

---

## §4. Ворота — verbatim

`a3f-status.txt`:

~~~
lib exit 0
stand exit 0
stand-nofeat exit 0
node exit 0
reader exit 0
p2p exit 0
slasher exit 0
fmt exit 0
clippy exit 0 warnings 12
clippy-feat exit 0 warnings 6
doc exit 0 unresolved 6
DONE
~~~

Строки результатов из соответствующих файлов:

~~~
a3f-lib.txt           test result: ok. 682 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 168.94s
a3f-stand.txt         test result: ok. 47 passed; 0 failed; 0 ignored; 0 measured; 643 filtered out; finished in 210.43s
a3f-stand-nofeat.txt  test result: ok. 39 passed; 0 failed; 0 ignored; 0 measured; 643 filtered out; finished in 166.19s
a3f-node.txt          test result: ok. 55 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 55.95s
a3f-reader.txt        test result: ok. 63 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
a3f-reader.txt        test result: ok. 0 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s
a3f-p2p.txt           test result: ok. 32 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
a3f-p2p.txt           test result: ok. 1 passed; 0 failed; 3 ignored; 0 measured; 0 filtered out; finished in 3.87s
a3f-p2p.txt           test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
a3f-slasher.txt       test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
~~~

`a3f-clippy.txt` — 12 предупреждений, места:

~~~
crates/dpos/staking-reader/src/epoch_transition.rs:2947:17  MutexGuard held across an await point
crates/dpos/p2p/src/lib.rs:340:17                            very complex type  (НОВОЕ)
crates/dpos/consensus/src/beacon/actor.rs:4492:21            useless conversion to alloy_rlp::Bytes  (НОВОЕ)
crates/dpos/consensus/src/testbed/stand.rs:581:20            very complex type
crates/dpos/consensus/src/testbed/stand.rs:1032:15           very complex type
crates/node/src/dpos.rs:1988:1                               large size difference between variants  (было и на 4.2)
~~~

`a3f-doclinks.txt` — 6 нерешённых ссылок, все вне диффа:

~~~
 --> crates/dpos/consensus/src/beacon/mod.rs:2:67
   --> crates/dpos/consensus/src/cold_start_jump.rs:775:68
   --> crates/dpos/consensus/src/engine.rs:144:7
   --> crates/dpos/consensus/src/executor.rs:368:11
   --> crates/dpos/consensus/src/executor.rs:389:32
   --> crates/dpos/consensus/src/slasher/evidence.rs:510:47
~~~

`a3f-tree.md5` — сверен `md5sum -c` до и после ревью, 12/12 `ЦЕЛ`.

---

## §5. Оставить как есть (и что чинить в первую очередь)

**Оставить без изменений:**

1. **Разделение `TrackedPeers` на два тира и уход реестра в secondary.** Механизм проверен по
   CW построчно; это закрывает R-013, R-037, E4-14 ровно так, как обещал проект.
2. **`C[E−1]` в primary.** Полный поворот комитета легитимен; выбрасывать уходящих в момент
   границы — тот же тихий раскол на эпоху раньше. Цена (тест 4b) видна и понята.
3. **Хранение записей ПОРОЗНЬ, а не объединением.** Маска без второго чтения — это и есть
   `Ingress::Member{epochs}`.
4. **Запись окна ДО `Manager::track`.** Правильный порядок: окно опережает CW, а не отстаёт.
5. **`None` = «мнения нет», а не «уронить».** Иначе холодный старт глушит плоскость.
6. **Отсутствие штрафа/счётчика по пиру.** §5.3, П-5, A64 — выполнено буквально.
7. **Заголовочный пик `ceremony_epoch` до `DkgMsg::read_cfg`.** Провод проверен, дорогие
   декодеры для не-члена не запускаются.
8. **`deque_size = 4` для BROADCAST** и запись «байтового лимита у CW нет» на месте, а не в
   доке. Это честная граница библиотеки.
9. **Правка `share_confirmations_are_minted_on_growth…`** — предпосылка сделана явной
   (`last_height = 100` + ассерт), а не обойдена. Образцовая форма.
10. **Отказ ставить `Ingress` на VOTE/CERT backup.** Потребителя нет; фильтр там был бы
    ритуалом.

**Порядок починки (моя рекомендация, не решение):** P-01 → P-03/P-04 (дешёвые, но теряют
знание) → P-05 → P-02 → P-06. P-01 закрывается в рамках 4.3 одним из трёх: (а) сделать
`push_neighbour_committee` для `E+1` фатальным (`?`) — тогда деградация ретраится штатным
контуром границы; (б) при пропущенном тире добавить реестр в primary как страховку именно для
этого случая; (в) не продвигать `last_tracked_epoch`, если хоть одна соседняя запись
пропущена по `Err`. Вариант (а) ближе всего к «treat the disease»: пустая запись (`Ok(_)`) —
законное «ещё не закоммичено», а `Err` — это отказ чтения, и молча жить с ним эпоху хуже,
чем повторить границу.

---

## §6. Вне рамок 4.3

- **Э5.** Д-123 (P-07/P-08) снимается одним параметром `ValidatorInputs` в
  `beacon/plane.rs:471-528`, чтобы beacon брал маску из `TrackedWindow`, а не из
  `committee_for` — файл вне write-листа этого захода. Туда же — использование
  `Committee::is_member` (`committee/mod.rs:294`) вместо выдачи ростера.
- **Э5/Э7.** Д-122: сделать модуль `committee/` единственным источником записей для ET
  (правка `Cargo.toml` или перенос ET) — сейчас два читателя одного слота.
- **Э7.** Байтовый бюджет BROADCAST: 4 × `MAX_ORDER_BLOCK_SIZE` = 16 MiB на primary-пира при
  primary до 3 × `MAX_COMMITTEE_SIZE`; если это когда-нибудь станет узким местом, лимит надо
  просить у commonware, а не обходить.
- **Э7.** Ревизия терминологии «registry ∪ committee» по всему дереву (P-09…P-12) — шесть
  файлов вне диффа, включая операторское сообщение об ошибке.
- **Приёмка 4.3 (девнет).** Дозвон secondary по `dialable` — не выполнен, как и записано в
  `PLAN.md:104`.
