# E5-3-MAP-RESEARCH — инвентарь для карты строки 5.3 (DKG-автомат, П-9)

Дата: 2026-09-14. HEAD `18573e76`, ветка `djadjka/dpos-reth-2.2-squashed`. Только чтение, агентов не запускал. Пути ниже — от `crates/dpos/consensus/src/beacon/`, если не сказано иначе. Каждый факт — с `file:line`, прочитанным в этой сессии; выводы помечены [ГИПОТЕЗА]. Свидетельство — код; комментарии цитируются только как «код говорит о себе», не как факт.

Объём продакшн-кода актора: `actor.rs:1-2815` (тесты с `:2816`); `ceremony.rs:1-1058` (тесты с `:1059`); `share_state.rs` — тесты с `:708`.

---

## §1. Неявный автомат сегодня (одна эпоха E)

### 1.1 Поля, кодирующие фазу эпохи E

| Поле | Тип / ключ | Где объявлено | Что кодирует для E |
|---|---|---|---|
| `ceremonies[E]` | `BTreeMap<u64, DkgCeremony>` | `actor.rs:383` | «церемония жива». Внутри: `dealer: Option` (`ceremony.rs:137`) = дилинг открыт/закрыт (`dealing_closed()` `:580-582`); `player: Option` (`:139`) = ещё можно финализировать (`can_finalize()` `:837-839`); `recorded: BTreeSet<PeerPubkey>` (`:148`), `signed_logs: BTreeMap<PeerPubkey, DealerReveal>` (`:154`); `own_pub_msg: Option` (`:159`) = живой дилер; `unsent` (`:163`); `emitted_acks` (`:169`); `pending_pub/pending_priv` (`:143-144`) |
| `pending[E]` | `BTreeMap<u64, BTreeMap<PeerPubkey, PendingDealings>>` | `actor.rs:421` | дилинги, пришедшие ДО старта (start-race); `PendingDealings{commitment: Option, share: Option}` `:321-324` |
| `store[E]` (`CeremonyStore`, shared `Arc<RwLock<BTreeMap<u64, Share>>>`) | `actor.rs:184`, поле `:357` | «share принят» = Finalized/Keyed |
| `agreed_pinned[E]` | `BTreeMap<u64, AgreedSet{pinned: BTreeMap<u8,B256>}>` | `actor.rs:516`, `:542-544` | «артефакт принят, набор запинен» |
| `agreement_announced ∋ E` | `BTreeSet<u64>` | `actor.rs:504` | инстанс соглашения запрошен (только логовая одноразовость, `:965-975`) |
| `deferred_reported ∋ (E, reason)` | `BTreeSet<(u64,&str)>` | `actor.rs:387` | finalize отложен по `below_quorum`/`missing_body` (`:1628-1632`) — одноразовый warn |
| `recompute_pending[E]` | `BTreeMap<u64, RecomputeState{outcome, want: BTreeSet<PeerPubkey>}>` | `actor.rs:162-165`, поле `:468` | демот-heal идёт; `want` пуст/не пуст = «логи собраны / ещё тянем» |
| `terminal_recompute ∋ E` | `BTreeSet<u64>` | `actor.rs:490` | share доказуемо невосстановим (`MissingPlayerDealing`) |
| `torn_warned ∋ E` | `BTreeSet<u64>` | `actor.rs:457` | sit-out по `Torn` — ПОСТОЯННЫЙ для процесса (читается как память решения `:1750-1751`) |
| `eval_logged ∋ E` | `BTreeSet<u64>` | `actor.rs:452` | диагностическая одноразовость |
| `nondurable_logs[E]` | `BTreeMap<u64, BTreeSet<PeerPubkey>>` | `actor.rs:443` | лог дилера в памяти, но не в журнале |
| `recorded_dkg_logs[E]` (shared `DkgLogIndex`) | `epoch → idx(u8) → B256` | `actor.rs:206`, поле `:495` | опубликованные хэши логов (что предлагает соглашение) |
| `confirmations.confirmed_len[E]` + `pool` | `confirmations.rs:103`, `:93` | какая ширина уже заявлена на проводе |
| `log_store.cache[E]` | `log_store.rs:88` | эпоха обслуживается из кэша (после finalize `actor.rs:1692` или `warm_from_journal` `:2624`) |
| `last_height: Option<u64>` | `actor.rs:437` | часы; `None` до первого тика — отдельное состояние (`on_confirm` `:1557`, `height_now()` `:1979-1981` = 0) |
| `reconciled_journals: bool` | `actor.rs:412` | одноразовый стартовый reconcile |
| диск: журнал `beacon-dkgjournal-e<E>.bin` | `JournalLoad::{NoFile, Present, Torn}` | `share_state.rs:536-553`, `load_journal` `:559-601` | «участвовал ли уже» |
| диск: share-файл `beacon-share-e<E>.bin` | `share_state.rs:288-290`, `load_all` `:314-349` | загружается один раз в `store` при `build` (`plane.rs:645-653`) |
| снаружи: `ArtifactStore[E]` через `outcome_at` | `actor.rs:461`, замыкание `plane.rs:790-797` | «артефакт держим» |
| снаружи: `changed(E)` (`mints_at`) | `actor.rs:2515-2523` | «эпоха минтит» |
| снаружи: `committee_for(E)` | `actor.rs:344` | членство/ростер; `None` = транзиент |

Итого: 14 полей актора + 6 внутри `DkgCeremony` + 3 внешних читателя + 2 файла на диске.

### 1.2 Состояния по факту (для одной target-эпохи E; время = эпоха E−1 и далее)

| # | Состояние по факту | Какими полями выражено | Где читается | Где пишется |
|---|---|---|---|---|
| S0 | Недостижима (E > now+2) / буферизуема (E ∈ [now+1, now+2]) | ничего; `pending[E]` возможен | `is_bufferable` `actor.rs:1951-1969`; `epoch_is_actionable` `:1995-2001` | `on_message` `:2134-2148` |
| S1 | Не решено (комитет/бит нечитаем) | `ceremonies` ∌ E, `store` ∌ E, `torn_warned` ∌ E, `eval_logged` ∌ E | `maybe_start` `:1766-1782` | `eval_logged.insert` `:1770` |
| S2 | Carry-forward (¬mints_at) | как S1 + `changed(E)=Some(false)` | `maybe_start` `:1785-1786`; `acquire_mint_artifacts` `:2494-2496` | — |
| S3 | Не член | `me ∉ committee_for(E)` | `maybe_start` `:1790-1791`; `drive_recompute` `:2359-2361` | — |
| S4 | Dealing (дилер жив) | `ceremonies[E].dealer = Some` | `dealing_closed()` false: `on_height` `:1270-1278`, `retransmit` `ceremony.rs:482-486`, `is_bufferable` `:1960-1963`, `announce` `:961` | `start_fresh` `:1881-1897`; `resume(reconstruct=true)` `ceremony.rs:764-796` |
| S5 | Dealing closed / Sealed (дилер снят, artifact нет) | `dealer=None`, `player=Some`, `agreed_pinned` ∌ E | `drive_finalization` `:1600,1611` (`?` на `agreed_pinned` ⇒ пропуск); `announce` `:961` | `seal_dealings` `ceremony.rs:521-561`; `resume(reconstruct=false)` `:797-812`; `resume` при `init_dealer` Err ⇒ player-only `:786-796` |
| S6 | Pinned (artifact принят, ждём тела/кворум) | `agreed_pinned[E]` + `player=Some`; подсостояния `deferred_reported (E,"missing_body")`/`(E,"below_quorum")` | `drive_finalization` `:1611-1633` | `on_artifact` `:939-943` |
| S7 | Finalized/Keyed | `store[E]` есть; `ceremonies` ∌ E; `log_store.cache[E]`; `agreed_pinned` ∌ E | `maybe_start` `:1735-1741`; `on_artifact` `:908-916`; `drive_recompute` `:2350` | `drive_finalization` `:1678-1703` → `adopt_share` `:1048-1112` |
| S8 | Finalize-Err (игрок потрачен) | `ceremonies[E].player = None`, `agreed_pinned[E]` ОСТАЁТСЯ | `can_finalize()` false ⇒ `drive_finalization` больше не трогает `:1600`; серв логов жив `:2681-2690` | `finalize_over_pinned` `ceremony.rs:994-996` (`player.take()`), `drive_finalization` Err-арм `:1712-1719` |
| S9 | Refused (share отвергнут: off-poly / persist Err) | `store` ∌ E, `ceremonies` ∌ E (удалена до adopt `:1684-1689`), `agreed_pinned` ∌ E (`:1703`), журнал на диске | `drive_recompute` при `now == E` `:2324-2400` | `adopt_share` `:1057-1067`, `:1069-1091` |
| S10 | SatOut-Torn | `torn_warned ∋ E`, `ceremonies` ∌ E | `maybe_start` `:1750-1751` | `maybe_start` `:1816` |
| S11 | SatOut-ResumeErr (resume вернул Err) | НИЧЕГО не помечено: `ceremonies` ∌ E, `torn_warned` ∌ E ⇒ `maybe_start` перечитывает журнал и повторяет resume КАЖДЫЙ тик с warn | `maybe_start` `:1801-1811`, `resume_from_journal` `:1934-1941` | — (нет памяти) |
| S12 | Recompute (демот-heal) | `recompute_pending[E]`; `want` пуст/не пуст | `try_recompute_pending` `:2540-2544`; `fetch_missing_logs` `:2247-2270`; `ingest_log` `:2753-2755` | `drive_recompute` `:2399-2400`; `ingest_recompute_log` `:2785-2789` |
| S13 | Unrecoverable | `terminal_recompute ∋ E`, `recompute_pending` ∌ E | `drive_recompute` `:2330`; `fetch_missing_logs` `:2256` | `try_recompute_pending` `:2574-2591` |
| S14 | Partial success (share есть, артефакта нет) | `store[E]` есть, `outcome_at(E)=None` | `drive_recompute` `:2350-2356` (⇒ `pull(E)`) | — (достижимо только через рестарт, комментарий `:2343-2348`) |
| S15 | Swept (E + 8 < now) | все карты без E; журнал удалён | `sweep_epoch_state` `:1147-1230`; `reconcile_journals` `share_state.rs:652-666` | там же |
| S16 | Не-член, ждёт артефакт (Acquiring{artifact}) | нет поля; итерация `[lo, now+1]` каждый тик | `acquire_mint_artifacts` `:2478-2505` | — |
| S17 | Unfrozen (геометрия не заморожена) | актор НЕ СОЗДАН: `plane.rs:833-845` ждёт первый `Some` в `geometry` | `surface.rs:2290-2292` (`WithheldReason::GeometryUnfrozen`) | `plane.rs:521-531` (watch) |

Итого 18 состояний по факту (S0–S17), из них три без своего поля (S11, S14, S16 — выводятся из комбинаций/итерации), одно вне актора (S17).

### 1.3 Где фаза читается по НЕСКОЛЬКИМ полям сразу (места, которые явный enum меняет)

| Место | Поля, читаемые совместно |
|---|---|
| `maybe_start` `actor.rs:1732-1832` | `ceremonies`, `store`, `torn_warned`, `changed` (через `mints_at`), `committee_for`, `eval_logged`, журнал на диске (`load_journal`), `height_now()` против дедлайна `:1808-1809` |
| `drive_finalization` `:1600-1618` | `ceremonies[E].dealer`, `.player`, `agreed_pinned[E]`, `committee_for`, `pinned_ready` (внутри — `signed_logs` + хэши) |
| `drive_recompute` `:2324-2400` | `recompute_pending`, `terminal_recompute`, `store`, `outcome_at`, `committee_for`, `log_store.parse_journal` (диск) |
| `on_artifact` `:908-937` | `store`, `agreed_pinned` |
| `is_bufferable` `:1951-1969` | `ceremonies[E].dealing_closed`, `last_height`, `store` |
| `epoch_is_actionable` `:1995-2001` | `ceremonies`, `last_height` |
| `fetch_missing_logs` `:2183-2270` | `ceremonies` + `recorded`, `committee_for`, `recompute_pending.want`, `terminal_recompute` |
| `sweep_epoch_state` `:1147-1230` | все 11 карт (перечислены `:1163-1207`) + `store` (`:1223-1229`) |
| `announce_agreement_targets` `:957-966` | `ceremonies[E].dealing_closed`, `agreement_announced` |
| `on_height` шаг 1 `:1270-1278` | `dealing_closed`, `height ≥ epoch_start(E) − 20` |
| `on_height` шаг 1b `:1293-1294` | `pending`, `now`, `ceremonies[E].dealing_closed` |
| `serve_log` `:2681-2690` | `ceremonies[E].signed_logs`, потом `log_store` |
| `ingest_log` `:2736-2755` | `ceremonies[E]`, иначе `recompute_pending` |
| `plane.rs restart_replay` (`artifact.rs:1287-1299`) | `ArtifactStore.epochs()`, `held_shares` (store), `journal_epochs` (диск) |

---

## §2. Входы

Цикл: `run` `actor.rs:807-870`, `select!` из ПЯТИ рукавов (`:819-867`): `heights`, `receiver`, `resolver_rx`, `pinned_rx`, `artifacts_rx`. Дизайн называет четыре; пятый — `pinned_rx` (вопросы инстанса соглашения, `:851-857`).

### 2.1 `clock` — `on_height` `actor.rs:1232-1367`
Три фидера в один канал `dkg_height_tx` (ёмкость 256, `node/dpos.rs:1326`): поллер `fin + K` (`node/dpos.rs:1496`, `:1526`), тии живого фронтира из cert-inlet (`consensus/src/cert_inlet.rs:274-277`), marshal tip через `FluentApp` (`consensus/src/application.rs:1081`). Монотонный clamp `actor.rs:1239-1240`.
Порядок шагов одного тика (переходы и гейты):
1. `reconcile_journals` один раз (`:1258-1263`).
2. Seal: для каждой церемонии с `!dealing_closed() && height ≥ epoch_start(E)−20` ⇒ `seal_dealings` (`:1270-1286`) — S4→S5.
3. `pending.retain(E > now && !dealing_closed)` (`:1293-1294`).
4. `drive_finalization` (`:1300`) — S6→S7/S8/S9. Гейт: `dealing_closed && can_finalize && agreed_pinned[E] && committee_for && pinned_ready`.
5. `mint(AnyGrowth)` (`:1305`), `announce_agreement_targets` (`:1310`) — гейт `dealing_closed`, `try_send` на `agreement_tx`.
6. `sweep_epoch_state(now)` (`:1316`) — →S15.
7. `retransmit` всех церемоний (`:1336-1338`), `maybe_start(now+1)` (`:1340`) — S1/S2/S3/S4/S5/S10/S11.
8. `broadcast_all` (`:1342`).
9. `drive_recompute` (`:1350`) — S9/S14→S12→S7/S13. Гейт: `outcome_at` и `share_dir` не `None` (`:2306-2311`), окно `[max(2, now−8), now]` (`:2320-2324`).
10. `acquire_mint_artifacts` (`:1358`) — S16; гейт `pull_artifact && outcome_at`, окно `[max(2, now−8), now+1]`, только не-член и только `mints_at` (`:2487-2503`).
11. `fetch_missing_logs` (`:1366`) — гейт `resolver.is_some()` (`:2168-2170`).
Замечание: `drive_finalization` (шаг 4) идёт ДО `maybe_start` (шаг 7) — церемония, возобновлённая на этом тике, финализируется не раньше следующего события (см. §3, T9).

### 2.2 Артефакт — `on_artifact` `actor.rs:906-945`
Источники в `artifacts_rx`: свой инстанс и pull через `agreed_tx` → `spawn_write_back` (`plane.rs:465-494`) → `adopt_tx` (`plane.rs:729-730`); стартовый `restart_replay` (`plane.rs:761`, `artifact.rs:1287-1299`, push `plane.rs:922-935`). Гейты: `store ∌ E` (`:908-916`), непустой набор (`:918-925`), первый-побеждает `agreed_pinned` (`:930-932`). Переходы: → S6, сразу `drive_finalization` + `fetch_missing_logs` (`:943-944`). Прекондиция «артефакт проверен против комитета» — только контрактная (`:510-517`).

### 2.3 p2p — `on_message` `actor.rs:2019-2150`
Гейты по порядку: декод конверта `BeaconMessage::Dkg` (`:2024-2027`, `wire.rs:20-23` — единственный вариант); пик эпохи из первых 8 байт (`:2036-2038`); `epoch_is_actionable` (`:2039-2042`, счётчик `"epoch"`); `beacon_member` (`:2043-2052`, `"not_member"`); полный декод `DkgMsg` (`:2054-2057`).
Типы `DkgBody` (`dkg_msg.rs:62-76`) и что каждый двигает:

| Тело | Путь | Переход |
|---|---|---|
| `Confirm` | перехват до церемонии `:2069-2072` → `on_confirm` `:1524-1582` | пул подтверждений (не церемония); окно `[now, now+2]` только при `last_height=Some` `:1557-1569` |
| `Commitment` / `Share` | живая церемония: `handle` → `try_ack` (`ceremony.rs:362-371`, `:434-480`) ⇒ Ack + `ReceivedDealing`; ack гейтится durable-журналом (`actor.rs:2093-2108`) | S4 (ack-путь); без церемонии и `is_bufferable` ⇒ `pending` `:2134-2148` (S0) |
| `Ack` | `handle` → `dealer.receive_player_ack`, `unsent.remove`, `OwnDealerAck` (`ceremony.rs:372-385`) | S4 (сужение `unsent`) |
| `Reveal` | `handle` → `check` → `record_checked_log` (`ceremony.rs:386-389`, `:406-421`) ⇒ `PeerLog`; затем при `recorded_a_log` ⇒ `drive_finalization` + `mint(Decisive)` (`actor.rs:2110-2131`) | S5/S6 (рост набора) → S7 |
| любое тело без церемонии и не буферизуемое | молча отброшено (`:2134` — ветка `else if` не срабатывает) | — |

### 2.4 resolver — `on_resolver_message` `actor.rs:2637-2671`
`LogMessage::{Produce, Deliver}` (`log_resolver.rs:367-386`). `Produce` → `serve_log` (`:2681-2690`). `Deliver` → `ingest_log` (`:2712-2756`): живая церемония ⇒ `ingest_signed_log` (`ceremony.rs:628-655`) → журнал → `drive_finalization` (`:2744`); без церемонии и `recompute_pending ∋ E` ⇒ `ingest_recompute_log` (`:2766-2797`) → `try_recompute_pending` (`:2790`); иначе `true` (`:2755`). После валидной доставки — `mint(AnyGrowth)` (`:2666`). Исходящая половина — `fetch_missing_logs` (`:2167-2283`) и `fetch_bodies` инстанса (`dkg_agree.rs:1366-1390`).

### 2.5 Входы, которых дизайн не называет
- `pinned_rx` — `PinnedRequest` от инстанса (`actor.rs:266-270`, обработка `:851-857`, `derive_pinned` `:885-893`). Читает `committee_for` + `ceremonies[E]`; после finalize отвечает `Unavailable` навсегда (`:889-892`).
- Закрытие `resolver_rx` — самостоятельное событие: `resolver_rx = None; self.resolver = None` (`:844-847`).
- Закрытие `pinned_rx` / `artifacts_rx` / `heights` / `receiver`: первые два паркуются (`:858`, `:866`), последние два — `break` (`:826`, `:830`).
- Стартовый `restart_replay` (`plane.rs:922-935`) — артефакт, приходящий до первого тика.
- Watch `geometry` (`plane.rs:833-845`) — вход «до актора».
- Таймеров внутри актора нет: `grep` по `sleep|interval(|timeout|tokio::time` в `actor.rs:1-2815` даёт только комментарии и `epoch_interval` (`:596`). Таймеры живут в резолвере (`plane.rs:70-72`) и в `resolve_artifact` (`dkg_engine.rs:469-487`).

---

## §3. Терминалы и отказы

| # | Где | Что | Восстановимо? |
|---|---|---|---|
| T1 | `maybe_start` Torn `actor.rs:1812-1825` | `torn_warned.insert`, ceremony не создаётся | ТУПИК для процесса: гейт `:1750-1751`; снимается только sweep (`:1206`) когда E недостижима как target. По-дизайну = `SatOut` (есть по сути; но БЕЗ разбиения «до/после seal» — R-072/R-051) |
| T2 | `maybe_start` NoFile `:1800` | `start_fresh` без проверки дедлайна | не отказ, а R-036: после дедлайна стартует дилер и печатает на СЛЕДУЮЩЕМ тике (`:1270-1286`) — «честная эквивокация». По-дизайну должно стать `SatOut` |
| T3 | `resume_from_journal` Err `:1934-1941` | warn, `false`, `pending.remove` | НЕ тупик и НЕ память: повтор каждый тик с warn (S11). Ближе всего к `Unrecoverable`, но не помечено |
| T4 | `start_fresh` Err `:1892-1895` | warn, `false` | как T3 — повтор каждый тик |
| T5 | `drive_finalization` Err `:1712-1719` | `dkg_ceremony_fail`, warn; ceremony остаётся с `player=None` | ТУПИК внутри окна: `can_finalize` false (`ceremony.rs:837-839`), `agreed_pinned[E]` остаётся до sweep (`:1200`). Выход только через `drive_recompute` при `now ≥ E` (`:2324`) при `share_dir` и артефакте — [ГИПОТЕЗА] по чтению `:2306-2311, 2374-2400`; либо рестарт (resume пересобирает `player`). По-дизайну: `Acquiring{logs}` |
| T6 | `try_recompute_pending` `MissingPlayerDealing` `:2574-2591` | `terminal_recompute.insert`, `recompute_pending.remove` | ТУПИК до sweep (`:1179`). = `Unrecoverable` по сути |
| T7 | `try_recompute_pending` `Err(_)`/off-poly `:2592`, `:2601-2609` | entry остаётся, повтор каждый тик (`load_journal` + `resume` + `finalize` — R-038) | бесконечный ретрай; при R-002 — навсегда до sweep |
| T8 | `adopt_share` off-poly `:1057-1067` / persist Err `:1069-1091` | `false`; ceremony уже удалена `:1684-1689`, `agreed_pinned.remove` `:1703` | ретрай только через `drive_recompute` при `now == E` (комментарий `:1084-1089` — код `:2324`); в окне E−1 — verify-only. Событие `Stalled{PersistFailed}` не поднято (тест `:6734-6737` говорит «ждёт 5.3») |
| T9 | Рестарт с артефактом на диске: `restart_replay` push до тика (`plane.rs:922-935`) | `on_artifact` при `ceremonies ∌ E` ⇒ `agreed_pinned` записан, `drive_finalization` ничего не находит, `fetch_missing_logs` пуст (`:2183` цикл по `ceremonies`) | [ГИПОТЕЗА] на остановленной цепи с ПОЛНЫМ журналом finalize ждёт события, которого нет: следующий тик (`:1300` до `:1340` в одном тике) или Reveal/Deliver. Юнит `a_restart_replays_the_stored_artifact_back_into_the_actor` (`:8816`) вызывает `on_height` ДО `on_artifact` (`:8880`, `:8929`) — обратный порядок к продакшну |
| T10 | `derive_pinned` после finalize `:889-892` | `Unavailable` навсегда | по контракту; инстанс паркует `verify` (`dkg_agree.rs:1355-1361`) — жив до `prune_agreements` |
| T11 | Инстанс: тело не пришло `dkg_engine.rs:381-397` | `dkg_agree_body_lost`, warn, `None`, `return` `:418` | ТУПИК: launcher помнит `started` (`:586-588`, `:611`) и не перезапускает; актор повторяет только announce (`:954-984`). Heal — pull через `drive_recompute` при `now == E` (`:2374-2377`). По-дизайну: `Acquiring{artifact}` немедленно (R-026) |
| T12 | Инстанс: комитет нечитаем `dkg_engine.rs:592-598` | `continue`, повтор по следующему announce | восстановимо (announce каждый тик) |
| T13 | `resolver_rx` закрылся `actor.rs:844-847` | `resolver = None` ⇒ `fetch_missing_logs` no-op (`:2168-2170`) | мёртвая ветка: `beacon_resolver` — supervised-ребёнок (`plane.rs:1000`), узел падает. Снимать «gossip-only» безопасно |
| T14 | `on_artifact` второй артефакт `:930-932` | молчаливый `return` | не `Conflict`; `ArtifactStore::insert` первый-побеждает `artifact.rs:485-489` — тоже молча (только `false`) |
| T15 | `MintIndex::record` расходящийся mint `artifact.rs:702-713` | warn + `dpos_mint_index_conflict_total`, первый стоит | ближайшее к `Conflict` — но это индекс минта, не артефакт |
| T16 | `drive_finalization` deferrals `:1619-1633` | `missing_body`/`below_quorum` — warn/error один раз | `missing_body` при хэш-несовпадении — ТУПИК (см. §4); `below_quorum` — ждёт Reveal/Deliver. По-дизайну = `Stalled{QuorumMissing}`; отдельного состояния нет |
| T17 | `seal_dealings` self-check fail `ceremony.rs:544-557` | warn, `OwnSeal`/`Reveal` не эмитятся, `dealer` уже `take`н | узел без своего дилинга; финализируется как игрок — не тупик |
| T18 | Unfrozen `plane.rs:833-845` | актор не построен; `geometry.changed()` Err ⇒ `return` `:839-843` | восстановимо первым `Some`; `Unfrozen` есть как `WithheldReason` (`surface.rs:61`), не как состояние актора |
| T19 | `heights`/`receiver` закрылись `actor.rs:826`, `:830` | `break` — актор завершается | dkg — supervised-ребёнок (`plane.rs:999`) ⇒ узел падает |

Сопоставление с терминалами строки плана:
- `Unfrozen` — есть (`surface.rs:61`, `plane.rs:833-845`), вне актора.
- `SatOut` — есть по сути (T1), без правила по дедлайну; NoFile после дедлайна = T2 (R-036).
- `Unrecoverable` — есть по сути (T6, `terminal_recompute`), только для heal-пути; T3 не помечен.
- `Conflict` — НЕТ: T14 молчит, T15 — mint-индекс с warn.
- `Stalled{reason}` — НЕТ: есть `deferred_reported` (T16) + счётчики; `BeaconEvent` без `Stalled` (комментарий теста `:6734-6737`).
- `Acquiring{artifact}` — по сути `drive_recompute` `:2350-2356`, `:2374-2377` и `acquire_mint_artifacts`, без состояния и без немедленности при T11.
- `Acquiring{logs}` — по сути `fetch_missing_logs` для живой церемонии; T5 в него не переходит.

---

## §4. Идентичность лога

| Структура | Ключ сегодня | Второй лог того же дилера | Затронет смена на `(dealer, hash)` |
|---|---|---|---|
| `DkgCeremony.recorded: BTreeSet<PeerPubkey>` `ceremony.rs:148` | дилер | `record_checked_log` `:415-417`: `!insert ⇒ Step::default()` — ПЕРВЫЙ побеждает, второй отброшен без следа | да: `record_checked_log`, `recorded_log_count` `:563-565`, `recorded_dealers` `:570-572`, `own_log_recorded` `:590-592`, `seal_dealings` `:539` |
| `DkgCeremony.signed_logs: BTreeMap<PeerPubkey, DealerReveal>` `:154` | дилер | не перезаписывается (гейт `recorded`) | да: `signed_log` `:598-600`, `signed_log_hash` `:844-846`, `take_signed_logs` `:609-611`, `scoped_pinned_logs` `:878-898` (хэш-сравнение `:884`), `retry_nondurable_journals` `actor.rs:1432-1438`, `serve_log` `:2681-2686` |
| `DkgCeremony.logs: Logs<...>` (commonware) `:140` | дилер (`logs.record(pk, log)` `:418`) | commonware `Logs::record` — не читал [LOOKUP] | да, если commonware допускает второй record под тем же ключом |
| `ingest_signed_log` `:628-655` | `expected` дилер | валидный лог того же дилера с другим хэшем ⇒ `(true, empty step)` `:644-649` — fetch считается выполненным, лог отброшен (R-002) | да — центральное место |
| `DkgLogKey {epoch, dealer}` `log_resolver.rs:52-55` | дилер | нет хэша в ключе ⇒ refetch «по хэшу» невозможен по протоколу | да — wire-ключ резолвера (кодек `:73-96`) и `BeaconFetchKey::Log` `:113-116` |
| `fetch_missing_logs` `actor.rs:2224-2237` | `roster − recorded` | дилер в `recorded` ⇒ не тянется даже при хэш-несовпадении | да |
| `fetch_bodies` `dkg_agree.rs:1366-1390` | idx→dealer | тянет `(epoch, dealer)` для `Missing(idx)` — доставка отбрасывается `ingest_signed_log` | да |
| `RecomputeState.want: BTreeSet<PeerPubkey>` `actor.rs:162-165`; заполнение `:2388-2393` (`dealers − held`, `held` = `parse_journal` по дилеру) | дилер | журнальный лог того же дилера с другим хэшем считается «held» | да |
| `ingest_recompute_log` `:2766-2797` | `key.dealer` | `check` + `pk == key.dealer` ⇒ `PeerLog` дописывается в журнал БЕЗ дедупа (`:2781-2782`); `want.remove` | да |
| `nondurable_logs[E]: BTreeSet<PeerPubkey>` `:443` | дилер | — | да (`:2101-2105`, `:2740-2743`, `publish_recorded_logs` `:1477-1480`) |
| `recorded_dkg_logs: epoch→idx→hash` `:206` | idx (позиция) | `insert(idx, hash).is_none()` `:1482` — первый хэш побеждает, второй молча игнорируется | частично: хэш уже есть, но ключ — seat |
| `Confirmations` `confirmations.rs:171-199` | набор `(idx, hash)` | читает индекс выше | косвенно |
| Журнал `JournalRecord::PeerLog` `share_state.rs:361-377` | БЕЗ ключа (append-only) | при replay — `log_map.insert(pk, …)` ПОСЛЕДНИЙ побеждает: `resume` `ceremony.rs:718-724`, `checked_serve_map` `:216-219`, `recompute_scoped` `:1035-1040` | да: live-путь first-wins, replay-путь last-wins — две разные семантики одного ключа |
| `DealerLogStore` `ServeMap = BTreeMap<PeerPubkey, DealerReveal>` `log_store.rs:70`, `get(epoch, dealer)` `:113-121` | дилер | из `checked_serve_map` — last-wins | да |
| `AgreedSet.pinned: BTreeMap<u8, B256>` `actor.rs:542-544` | idx→hash | единственное место, где хэш — часть ключа | нет (уже хэш) |
| `PinnedDerive::Missing(Vec<u8>)` `dkg_agree.rs:673` | idx | — | нет |

Где refetch сегодня и по какому ключу: `fetch_missing_logs` (`(epoch, dealer)` для `roster − recorded`, `actor.rs:2224-2244`; для `want`, `:2247-2270`), `fetch_bodies` (`(epoch, dealer)` по `Missing(idx)`, `dkg_agree.rs:1366-1390`). Ни один не знает хэш.
Обнаружение двух подписей одного дилера: нигде (второй лог падает в `Step::default()` `ceremony.rs:415-417`; в `ingest_signed_log` — `(true, …)`).

---

## §5. Вход p2p и классификация отправителя

Два источника:
1. `GatedReceiver` до декода — `consensus/src/dpos.rs:89-149`. `admits`: `window.classify(from)` (`:116-128`): `None` (до первого `track`) ⇒ пропуск; `Member` ⇒ пропуск; `Tracked` ⇒ отказ при `members_only=true`; `Dropped` (не в наборе или tombstone) ⇒ отказ. Для BEACON `members_only=true` (`node/src/dpos.rs:1892-1897`). Что исключает: tier-2 (registry), нетрекаемых, tombstoned. Что НЕ проверяет: соответствие эпохи кадра — `EpochMask` (`p2p/src/lib.rs:433-467`) вычисляется, но `GatedReceiver` его не читает (`Ingress::Member { .. } => true`, `:121`).
2. `beacon_member` после пика эпохи — `actor.rs:2015-2017`: `committee_for(epoch).position(from)`. Исключает не-членов ИМЕННО эпохи кадра; читает запись комитета (модуль `committee/`, через `CommitteeReads::committee` `plane.rs:610-613`).

Множества: `TrackedWindow.latest` = записи `{E−1, E, E+1}` эпохи перехода E (`staking-reader/src/epoch_transition.rs:775-782`; нечитаемый сосед — пропущен `:805-813`) ∪ secondary; `beacon_member(E')` = `committee[E']` для E' ∈ `[now, now+2]` ∪ живые церемонии. Различие: окно ET считается от `fin` (переход), актор — от `max(fin+K, upstream, tip)` ⇒ E' = `now+2` может не иметь записи в `TrackedWindow`: отправитель ТОЛЬКО из `committee[now+2]` режется гейтом как `Dropped` раньше, чем актор его примет [ГИПОТЕЗА — зависит от того, равна ли `E` перехода `now` актора; не измерял].

Как маска доходит до бикона: НЕ доходит. `ValidatorInputs` (`plane.rs:501-563`) содержит `peers: P` (oracle, `:521`) и `beacon_channel: (Se, Re)` (`:523`), где `Re` в продакшне — уже `GatedReceiver` (`node/src/dpos.rs:1892-1897`); `TrackedWindow` в `ValidatorInputs` не передаётся (`git grep TrackedWindow -- consensus/src/beacon` — пусто). `EpochMask::member_of` (`p2p/src/lib.rs:412-414`) никем в beacon не читается.

Кадр `now+3` (R-126): окно задаётся ТРИЖДЫ:
- `epoch_is_actionable` `actor.rs:1999-2000`: `now..=now+2` ∪ `ceremonies` (после первого тика; до него `height_now()=0` ⇒ `now=0` ⇒ окно `[0,2]` — кадры для эпох >2 режутся с `"epoch"` до первого тика; для дилингов лечится ретрансмитом `:1336-1338`, для `Reveal` — refetch резолвером [ГИПОТЕЗА о последствиях]);
- `on_confirm` `:1557-1569`: `now..=now+2`, только при `last_height=Some`;
- `is_bufferable` `:1968-1971`: `(now, now+2]`.
Во что упирается `+3`: `TrackedWindow` даёт максимум `E+1` относительно перехода; `committee_for(now+3)` — запись, которой обычно нет (комитет `now+3` не закоммичен). Расширение окна до `+3` без записи комитета даёт `beacon_member=false` ⇒ `"not_member"` вместо `"epoch"` — то есть решение «кадр `+3`» упирается в оба источника, не в один [ГИПОТЕЗА].

---

## §6. `recover(E)`, `Option`-швы, метрики, `deque_size`

### 6.1 Функции восстановления (по имени и вызовам)
| Функция | Что восстанавливает | Вызывается из |
|---|---|---|
| `share_state::load_all` `share_state.rs:314-349` | share-файлы → `store` | `plane.rs:645-653` (один раз) |
| `share_state::reconcile_journals` `:652-666` | удаляет журналы `E+8<now`, лишние share-файлы | `on_height` первый тик `actor.rs:1258-1263` |
| `restart_replay` `artifact.rs:1287-1299` | артефакты для `epochs ∩ journaled − held` → `on_artifact` | `plane.rs:761`, push `:922-935` |
| `maybe_start` → `load_journal` `actor.rs:1867-1874` → `resume_from_journal` `:1905-1943` → `DkgCeremony::resume` `ceremony.rs:680-826` | церемония из журнала (reconstruct/player-only по `height_now() < deadline` `actor.rs:1808-1809`) | каждый тик `:1340` |
| `drive_recompute` `:2305-2405` + `try_recompute_pending` `:2538-2632` + `ingest_recompute_log` `:2766-2797` + `recompute_scoped` `ceremony.rs:1016-1057` | share после границы из журнала + артефакта | `on_height` `:1350`; `ingest_log` `:2753-2755` |
| `acquire_mint_artifacts` `:2478-2505` | артефакт для не-члена | `on_height` `:1358` |
| `fetch_missing_logs` `:2167-2283` | недостающие логи живых церемоний и `want` | `on_height` `:1366`, `on_artifact` `:944` |
| `retry_nondurable_journals` `:1421-1449` | повторная запись журнала | `publish_recorded_logs` `:1460` |
| `adopt_share` ветка частичного успеха | share есть, артефакта нет ⇒ `pull` | `drive_recompute` `:2350-2356` |

Ветки `recover(E)` по дизайну (`E5-BEACON-DESIGN.md:616`) размазаны по пяти функциям: (share, артефакт) — `load_all` + `ArtifactStore` replay; (share, ¬артефакт) — `drive_recompute:2350-2356`; (артефакт, ¬share, журнал) — `restart_replay` + `maybe_start` resume (T9 — порядок); (журнал Present/Torn/NoFile × до/после seal) — `maybe_start:1800-1825` (три клетки из четырёх; NoFile-после-seal и Torn-до-seal неверны относительно дизайна).

### 6.2 `Option`-швы актора и что значит `None`
| Поле | Где | `None` = | Продакшн |
|---|---|---|---|
| `resolver: Option<R>` | `actor.rs:339`, `new` `:557` | нет резолвера ⇒ `fetch_missing_logs` no-op `:2168` | `Some(logs)` `plane.rs:852` |
| `resolver_rx` | `:343`, `:558` | ветка парк `:252-257` | `Some` `plane.rs:853` |
| `changed: Option<ChangedAt>` | `:353`, builder `:708-711` | `mints_at=false` кроме E=2 `:2519-2522` ⇒ ничего не стартует | `with_changed_bit` `plane.rs:866` |
| `share_dir: Option<PathBuf>` | `:368`, `:565` | нет журнала/share-файла; `append_journal ⇒ true` `:779-781`; `drive_recompute` no-op `:2309-2311`; `load_journal ⇒ NoFile` `:1868-1870` | `Some(share_dir)` `plane.rs:862` |
| `outcome_at: Option<AgreedOutcomeAt>` | `:461`, `:567` | `drive_recompute` и `acquire_mint_artifacts` no-op `:2306-2308`, `:2479-2482` | `Some` `plane.rs:864` |
| `pull_artifact: Option` | `:478`, builder `:642-645` | pull не выдаётся | `plane.rs:871` |
| `plane_clock: Option` | `:448`, `:651-654` | gauge молчит `:1246-1248` | `plane.rs:872` |
| `agreement_tx` / `artifacts_rx` | `:509`, `:522`, `:664-672` | announce no-op `:955-957`; артефакт-арм парк | `plane.rs:870` |
| `recorded_dkg_logs: Option<DkgLogIndex>` | `:495`, `:721-727` | `publish_recorded_logs` no-op `:1461-1463`; `mint` no-op `confirmations.rs:154-156` | `plane.rs:867` |
| `confirmations.pool: Option` | `confirmations.rs:93`, `:121-123` | `on_confirm` `return` `actor.rs:1525-1527`; `mint` пуст | `plane.rs:868` |
| `pinned_rx: Option` | `:500`, `:632-638` | ветка парк | `plane.rs:869` |
| `last_height: Option<u64>` | `:437` | «до первого тика» — НЕ шов, состояние часов (`on_confirm` `:1557`) | — |
Итого 11 конфигурационных `Option` (не считая `last_height`); продакшн заполняет все 11 (`plane.rs:846-873`). Тесты строят с `None` и подключают по одному (например `actor.rs:3095`, `:8862-8865`).

### 6.3 Реестры метрик, которых касается модуль
| Реестр | Семьи | Точки регистрации/инкремента |
|---|---|---|
| commonware (`Metrics::register`, `:19100`) — `BeaconMetrics` | `metrics.rs:28-188` (24 counters) | `register` `metrics.rs:193-338`; вызовы `plane.rs:658` (валидатор) и `plane.rs:1168` (follower `build_resolved`) — два места, по одному на класс узла |
| commonware — `PlaneClock` (`sync_metrics.rs`) | `dpos_dkg_clock_height`, drop-счётчик | регистрируется в узле `node/src/dpos.rs:1451`; актор пишет `actor.rs:1246-1248` |
| reth `metrics::` макро | `dpos_ingress_dropped_total` (`consensus/src/dpos.rs:63-68`) — актор `actor.rs:1566`, `:2040`, `:2050`; `dpos_artifact_store_*` / `dpos_mint_*` (`artifact.rs:506,712,1206,1207,1235,1244,1363,1364,1431,1441`); `dpos_seed_journal_*` (`seed_journal.rs:265,266,453,455`) | 3 + 10 + 4 = 17 вызовов |
Итого: два реестра (commonware / reth), три «владельца» (`BeaconMetrics`, `PlaneClock`, `metrics::`-макро). Актор сам пишет в оба реестра (`self.metrics.*` и `record_ingress_drop`). R-070 в Э0.4 признан «не сливать в один эндпойнт» (REGISTER.md:691) — строка 5.3 говорит «один реестр метрик»: противоречие, которое карте надо разрешить явно [ГИПОТЕЗА: имеется в виду «одна семья-владелец внутри бикона», не слияние экспозиций].

### 6.4 `deque_size` и P-16
`dkg_transport.rs:135` — `deque_size: 2`, обоснование `:123-134` (одно тело на (инстанс, отправитель) по замеру `testbed/preconditions.rs:562`; второй слот — «запас на перепредложение после nullify, которого замер не дал»). Юнит «два тела на отправителя после nullify» отсутствует: `dkg_engine.rs:1514` `a_nullified_first_view_still_finalizes_and_tears_down` идёт через продакшн `build_body_engine` (`dkg_engine.rs:312`) и проверяет только финализацию; юниты `dkg_agree.rs` строят движок с `deque_size: MAX_SET_LEN` (`dkg_agree.rs:2227`) — свойство `=2` ими не проверяется. Перепредлагает ли лидер ДРУГОЕ тело после nullify — по коду не устанавливал [LOOKUP: `dkg_agree.rs::propose` при `certified_value`, `:3135` тест `propose_re_proposes_the_certified_value` говорит о том же значении].

---

## §7. Связанность для разбиения

Общие функции/поля по парам заходов (А — автомат; Б — идентичность лога; В — вход p2p; Г — гигиена):

| Пара | Общее |
|---|---|
| А–Б | `drive_finalization` (`:1600-1633` читает `pinned_ready` ⇒ `scoped_pinned_logs` хэш-сравнение); `fetch_missing_logs` (`:2224-2237` `recorded`); `ingest_log`/`ingest_signed_log`; `RecomputeState.want`; `nondurable_logs`; sweep. Терминал `Conflict` (А) — это и есть «вторая подпись дилера» (Б) |
| А–В | `epoch_is_actionable` `:1995-2001`, `is_bufferable` `:1951-1969`, `on_confirm` окно `:1557-1569` — все читают `last_height`/`ceremonies`; `on_message` dispatch по `ceremonies[E]` `:2082` |
| А–Г | resolver-exit арм `:844-847` (Г) — внутри `run` (А); `Option`-швы (Г) — гейты каждого шага `on_height` (А: `:2168`, `:2306-2311`, `:2479`); метрики `self.metrics.*` во всех переходах (`:1051`, `:1078`, `:1111`, `:1640`, `:1645`, `:1713`, `:2586`) |
| Б–В | почти нет: `on_message` только доставляет `Reveal` в `handle` (`:2082-2083`); классификация отправителя не знает хэша. Единственное касание — `record_checked_log` не различает `from` и `pk` (`ceremony.rs:73-81`) |
| Б–Г | `DkgLogKey` кодек (`log_resolver.rs:73-96`) при смене ключа — wire; юнит `deque_size` — нет |
| В–Г | `record_ingress_drop` (`:1566`, `:2040`, `:2050`) — «один реестр» (Г) меняет три вызова во входе (В); `TrackedWindow` в `ValidatorInputs` — новый параметр `plane.rs:501-563` (шов, а не `Option` — Г) |

Можно ли Б, В, Г ДО А без переделки:
- Б до А — ДА и это «нормальное промежуточное» по П-9 (`DECISIONS.md:92`). Изменения Б локальны в `ceremony.rs:406-421, 563-611, 628-655, 718-724, 870-898`, `actor.rs:2224-2237, 2388-2393, 2766-2797`, `log_resolver.rs:52-96`, `log_store.rs`. Автомат потом только переименует переходы. Единственный риск: `Conflict` как терминал появится в А — Б должен оставить «бан + пара» как ДАННЫЕ (например поле в `DkgCeremony`), которые А превратит в состояние.
- В до А — ДА, если В ограничить «один источник + окно как чистая функция `(now, ceremonies) → bool`»: `epoch_is_actionable`/`is_bufferable`/`on_confirm` уже три копии одного правила (`:1968-1971`, `:1999-2000`, `:1562`); свести к одной функции можно сейчас, А её вызовет. `+3` — константа в этой функции.
- Г до А — ЧАСТИЧНО. resolver-exit ⇒ fatal (`:844-847` → `break` или `panic`) — независимо. `Option`→конфиг — переделает конструктор `new` `:553-625` и 11 тестовых сборок (`:3095, 3202, 3479, 3577, 3896, 4253, 4348, 4537, 4749, 7268` + `standalone_actor` `:4700-4786`), и А будет менять те же поля ⇒ ДВОЙНАЯ правка тестов, если Г раньше. Юнит `deque_size` — независим. «Один реестр» — независим, но затрагивает В (три вызова).
- Обязан идти первым: Б (по П-9 «автомат без хэш-идентичности — нет», `E5-BEACON-DESIGN.md:729`; по коду — `scoped_pinned_logs` и `ingest_signed_log` — единственные места, где хэш уже сравнивается, и автомат опирается именно на них).

Лучше ли другое разбиение: [ГИПОТЕЗА] Г разбить надвое: Г1 (resolver-exit ⇒ fatal, `deque_size` юнит, реестр) — до А, малые независимые правки; Г2 (`Option`→конфиг) — ВНУТРИ А, потому что переписывание `new`/builders и состояний трогает те же 11 тестовых сборок один раз. В — до А. Порядок: Б → В → Г1 → А(+Г2). Обоснование объёма — §9.

---

## §8. Тесты

Счётчики `#[test]`: `actor.rs` 54 (с `:2816`), `ceremony.rs` 16 (`:1059`), `share_state.rs` 20 (`:708`), `dkg_agree.rs` 34, `dkg_engine.rs` 8, `log_store.rs` 1, `dkg_transport.rs` 4, `log_resolver.rs` 4, `plane.rs` 10, `confirmations.rs` 0 (!), `testbed/tests.rs` 39, `testbed/preconditions.rs` 5, `testbed/cert_inlet_tests.rs` 8, `testbed/committee_tests.rs` 4.

| Раздел | Покрыто | Не покрыто |
|---|---|---|
| §1 состояния | S4/S5/S7: `dealing_open_ceremony_does_not_finalize_early` `actor.rs:6866`, `pre_deadline_resume_seals_via_on_height_then_finalizes` `:6629`; S8: `finalize_err_retains_ceremony_not_destroys` `:6937`; S9: `a_share_whose_persist_fails_is_refused…` `:6743`; S10: `torn_own_seal_post_deadline_refetches_not_reseals` `:6823`, `torn_own_seal_refetches_own_log_and_finalizes` `:6029`; S12/S13: `demoted_member_recomputes_share_and_heals` `:7302`, `an_unrecoverable_share_is_verdicted_once_and_never_retried` `:7911`; S14/S16: `a_restarted_member_with_a_share_and_no_artifact_asks_for_it…` `:7582`, `the_live_epoch_artifact_is_asked_for_every_tick…` `:7469`; S15: `stalled_ceremony_is_evicted_once_its_epoch_ages_out` `:3430`, `an_aged_out_epoch_is_neither_asked_for_nor_healed` `:7736` | S11 (resume Err каждый тик); T2 (NoFile после дедлайна ⇒ второй лог) — стенд R-036 не ставился (REGISTER.md:351); S17 Unfrozen — только через `plane.rs` follower-тесты `:1339-` и `surface.rs` [не читал]; T9 порядок replay/тик |
| §2 входы | clock: `ordering_clock_seeds_within_margin_lagged_clock_slips_k` `:3343`, `the_dkg_clock_gauge_is_the_actors_max_over_every_feeder` `:4900`; артефакт: `an_agreed_artifact_recovers_the_epoch_with_the_chain_halted` `:8367`, `an_artifact_that_arrives_only_by_pull_still_mints_the_share` `:8477`, `the_artifact_edge_fetches_the_pinned_bodies_it_lacks` `:8581`, `a_restart_replays_the_stored_artifact…` `:8816`; p2p: `start_race_buffers_early_dealings` `:3409`, `pending_buffer_is_per_sender_bounded` `:3529`, `a_beacon_frame_from_a_non_member_costs_no_committee_read_beyond_the_check` `:4787`, `a_confirmation_that_beats_the_first_height_tick_is_counted` `:5238`; resolver: `resolver_ingest_converges_rejects_forged_and_drops_wrong_epoch` `:3796`, `restart_midwindow_recovers_via_resolver` `:6004`, `fetch_missing_logs_cancels_dead_fetches` `:4205`; pinned_rx: `pinned_derive_answers_unavailable_for_every_node_local_gap` `:4596`, `pinned_mailbox_answers_unavailable_when_the_actor_cannot_reply` `:4655` | закрытие `resolver_rx` (T13) — нет теста; окно `[0,2]` до первого тика для `on_message` — нет |
| §3 терминалы | T5 `:6937`; T6 `:7911`; T8 `:6743`; T11 `dkg_engine.rs:1006` `a_certificate_is_paired_with_a_late_body_and_gives_up_when_there_is_none`; T16 — `dkg_finalize_deferred` в `:8581`? [не проверял тело]; Torn `share_state.rs:1076`, `:1241` | T1 с разбиением до/после seal; T2; T3/T4; T9; T14 (второй артефакт); `Stalled` событие |
| §4 идентичность | стенд: `a_dealer_with_two_logs_leaves_the_addressed_victim_without_a_share` `testbed/tests.rs:3117` (в PLAN.md:117 якорь `:2948` устарел), `a_two_log_dealer_that_also_withholds_its_partial_stops_the_chain_silently` `:3293`; `pinned_ready_all_held_false_when_a_pinned_body_is_missing` `ceremony.rs:1306`; `derive_pinned_separates_a_missing_body_from_an_unusable_set` `:1329`; `verify_parks_on_a_missing_dealer_log_body` `dkg_agree.rs:2313` | first-wins против last-wins в replay (§4 строка «Журнал»); дубль `PeerLog` через `ingest_recompute_log`; refetch по хэшу (нечего покрывать — нет кода) |
| §5 p2p вход | `GatedReceiver`: `consensus/src/dpos.rs:6479-6620` (тесты модуля); `a_beacon_frame_from_a_non_member…` `actor.rs:4787`; `a_registry_only_node_is_secondary_on_every_peer_set_and_still_follows` `testbed/tests.rs:4416`; `a_secondary_peer_on_the_authenticated_transport_is_accepted_and_heard` `preconditions.rs:617` | расхождение `TrackedWindow` vs `beacon_member` (R-127); кадр `+3` (R-126, REGISTER.md:977 «не проявилась») |
| §6 recover/швы/метрики/deque | `restart_midwindow_recovers_via_journal` `:3652`; `journal_survives_epoch_boundary_for_retained_window` `:7126`; `reconcile_*` `share_state.rs:1147-1231`; `dkg_bodies_per_peer_are_measured_under_a_partition…` `preconditions.rs:562`; `a_nullified_first_view_still_finalizes_and_tears_down` `dkg_engine.rs:1514` | `deque_size=2` при перепредложении (§6.4); `Option`-швы — только «включено/выключено» в конструкторах тестов, отдельных тестов на инертность нет; реестр — нет |

---

## §9. Числа (пересчитано скриптом по скобкам, `actor.rs:1-2815`)

Тело функции = от сигнатуры до закрывающей скобки; doc-строки не включены.

| Группа | Функции (строки, тело) | Σ тело |
|---|---|---|
| Входы | `run` 807-870 (64); `on_height` 1232-1367 (136); `on_message` 2019-2150 (132); `on_artifact` 906-945 (40); `on_resolver_message` 2637-2671 (35); `on_confirm` 1524-1582 (59); `derive_pinned` 885-893 (9) | 475 |
| Переходы | `drive_finalization` 1584-1722 (139); `maybe_start` 1728-1861 (134); `start_fresh` 1881-1897 (17); `resume_from_journal` 1905-1943 (39); `adopt_share` 1048-1112 (65); `sweep_epoch_state` 1147-1230 (84); `announce_agreement_targets` 954-984 (31); `publish_recorded_logs` 1457-1496 (40) | 549 |
| Восстановление | `drive_recompute` 2305-2405 (101); `try_recompute_pending` 2538-2632 (95); `ingest_recompute_log` 2766-2797 (32); `acquire_mint_artifacts` 2478-2505 (28); `retry_nondurable_journals` 1421-1449 (29); `load_journal` 1867-1874 (8); `append_journal` 778-795 (18); `evict_journal` 798-802 (5); `mints_at` 2515-2523 (9) | 325 |
| p2p / resolver-исход | `fetch_missing_logs` 2167-2283 (117); `ingest_log` 2712-2756 (45); `serve_log` 2681-2690 (10); `broadcast_all` 2800-2813 (14); `is_bufferable` 1951-1969 (19); `epoch_is_actionable` 1995-2001 (7); `beacon_member` 2015-2017 (3); `height_now` 1979-1981 (3) | 218 |
| Служебные | `new` 553-625 (73); 8 builder-ов 632-741 (52); `epoch_of`/`epoch_start` 745-759 (10); `ceremony_retain_floor` 241-244 (4); `recv_or_never` 252-257 (6); `PinnedMailbox` 287-314 (25) | 170 |
| Итого тел | 47 функций | 1737 |
Остальные ~1078 строк региона 1-2815 — doc-комментарии, объявление `DkgActor` (`:330-540`, 211 строк) и модульный doc.

Оценка объёма заходов по коду [ГИПОТЕЗА, по касаемым функциям]:
- Б: `ceremony.rs` 406-421, 563-611, 628-655, 700-760, 870-898 (~150 строк) + `actor.rs` 2224-2237, 2388-2393, 2766-2797 (~50) + `log_resolver.rs` 52-96 + `log_store.rs` — ≈ 250 строк продакшна.
- В: `actor.rs` 1951-2052 + 1557-1569 (~120) + `plane.rs:501-563` (+1 поле) + `node/src/dpos.rs:1892-1897` — ≈ 150.
- Г1: `actor.rs:844-847` (4), `dkg_transport.rs` юнит, реестр — ≈ 50 + тест.
- А(+Г2): группы «Входы» + «Переходы» + «Восстановление» = 1349 строк тела + `new`/builders 125 + 11 тестовых сборок.

---

## §10. Где чтение слабее всего (по убыванию)

1. `dkg_agree.rs` целиком (3180 строк) — читал только `PinnedDerive` (`:655-682`), `decide` (`:1290-1364`), `fetch_bodies` (`:1366-1390`). Поведение `propose` при nullify/перепредложении (P-16) — не читал.
2. commonware `Logs::record`/`select` при повторном `record(pk, …)` — не открывал checkout (COMMONWARE_INTERNALS.md не читал в этой сессии). Влияет на §4 строку `DkgCeremony.logs`.
3. `surface.rs` (2507) — только `:52-66`, `:2284-2296`. `Unfrozen` как состояние вне актора описан по этим строкам и `plane.rs:833-845`.
4. `epoch_transition.rs` — только `assemble_tracked_peers` `:767-819`; какое `epoch` подаётся в `track` относительно актора `now` — не проверял; отсюда [ГИПОТЕЗА] в §5 о расхождении окон.
5. T9 (порядок `restart_replay` vs первый тик) — вывод из `select!` без приоритета (`actor.rs:819`), продакшн-порядок push (`plane.rs:922-935`) и порядка шагов `on_height`; живой прогон не делал.
6. Тесты `actor.rs:2816-9055` — читал только имена и два тела (`:6725-6745`, `:8860-8953`); утверждения о покрытии в §8 — по именам, кроме этих двух.
7. `artifact.rs` — только `insert` `:476-521`, `MintIndex::record` `:698-725`, `restart_replay` `:1287-1299`.
8. `dkg_engine.rs` — `:360-420`, `:469-487`, `:554-645`, `:1514-1530`.

---

## Прямые ответы

**(а)** Состояний по факту — 18 (S0–S17, §1.2), из них 3 без своего поля (S11 resume-Err без памяти, S14 partial success, S16 не-член ждёт артефакт) и 1 вне актора (S17 Unfrozen). Кодируют их 14 полей `DkgActor` + 6 полей `DkgCeremony` (`dealer`, `player`, `recorded`, `signed_logs`, `own_pub_msg`, `unsent`) + 3 внешних читателя (`store`, `outcome_at`, `changed`/`committee_for`) + 2 файла на диске (журнал tri-state, share-файл) — 25 носителей; 14 мест читают ≥ 2 носителя одновременно (§1.3).

**(б)** Три самых опасных места:
1. `ceremony.rs:415-417` + `:644-649` + `actor.rs:2233-2237` — first-wins по дилеру без хэша: второй лог отброшен как «дубль», fetch помечен выполненным, refetch не идёт ⇒ `missing_body` навсегда (R-002); при этом replay-пути (`ceremony.rs:718-724`, `:216-219`, `:1035-1040`) — last-wins, две семантики одного ключа.
2. `actor.rs:1800` — `NoFile ⇒ start_fresh` без гейта по дедлайну, печать на следующем тике `:1270-1286` ⇒ честная эквивокация (R-036); плюс `:1712-1719` — finalize Err оставляет `player=None` навсегда в окне, а `agreed_pinned` висит до sweep.
3. [ГИПОТЕЗА] `plane.rs:922-935` + `actor.rs:1300/1340` — стартовый replay артефакта гоняется с первым тиком в `select!` без приоритета; при проигрыше и остановленной цепи с полным журналом финализация ждёт события, которого нет; юнит `:8816` проверяет обратный порядок.

**(в)** Разбиение: Б (идентичность `(dealer, hash)` + «бан + пара» как данные) → В (одна функция окна, один источник маски через `ValidatorInputs`) → Г1 (resolver-exit ⇒ fatal `actor.rs:844-847`; юнит `deque_size`; реестр) → А + Г2 (автомат, терминалы, `recover(E)`, `Option`→конфиг — в одном заходе, потому что оба переписывают `new`/builders и 11 тестовых сборок). Первым — Б: П-9 (`DECISIONS.md:92`, дизайн `:729`) и по коду — `scoped_pinned_logs`/`ingest_signed_log` уже единственные хэш-сравнения, на которые автомат обопрётся.

**(г)** Не прочитано: `dkg_agree.rs` кроме `:655-682, 1290-1390` (в частности `propose`/перепредложение после nullify — P-16); commonware `Logs` (checkout не открывал); `surface.rs` кроме `:52-66, 2284-2296`; `epoch_transition.rs` кроме `:767-819`; тела тестов `actor.rs:2816-9055` кроме `:6725-6745` и `:8860-8953`; `artifact.rs` кроме трёх функций; `seed_*`, `oracle.rs`, `outcome.rs`, `dkg_oracle.rs`, `confirmations.rs` тестов нет; `.dpos-study/history/E4-ORCHESTRATOR.md` — по условию.

Файл: `/home/djadjka/Work/fluentbase/.dpos-study/history/E5-3-MAP-RESEARCH.md`
