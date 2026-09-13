# REGISTER — реестр находок: R-001..R-123, K-1..K-40, 14 групп дублей, упрощения

Состояние на 2026-09-11. Это единственное место, где записан статус находки; `PLAN.md` ссылается сюда, `history/` — не свидетельство статуса. Форма записи: file + имя символа, без номеров строк (номера от 09-03 протухли после Э0–Э2; они остались в `history/REGISTER.md`). Механизмы сжаты — полный разбор каждой записи в `history/REGISTER.md`, откуда взяты суть, механизм, якоря и уверенность (все — [KNOWN] на 09-03, если запись не говорит иначе).

## Оглавление

- §0 Шкала, статусы, откуда что взято
- §1 R-001..R-123 (узел; R-101..R-110 — граница узел↔контракт; R-111..R-113 — инцидент пола; R-114..R-119 — дубли; R-120 — принятый риск; R-121..R-123 — находки стенда Э3.2, 09-09)
- §2 K-1..K-40 (контракт) и 14 групп дублей DUPLICATES
- §3 Упрощения B-1..B-11, BB-1..BB-12, CB-1..CB-13
- §4 Трассировка A/BA/C/COVERAGE/UNDERSTANDING/CONTRACT-UNDERSTANDING → R и K

## §0 Шкала, статусы, откуда что взято

Шкала тяжести (REGISTER 09-03): **BLOCKER** — узел компрометируется, финализируются расходящиеся блоки, или сеть останавливается действиями одного участника; **SERIOUS** — узел падает или самопроизвольно останавливается, защита не работает, ресурс исчерпывается удалённо; **MODERATE** — некорректное поведение в конкретном сценарии без потери безопасности; **MINOR** — дефект без наблюдаемого последствия в проде; **NIT**. Пересмотры тяжести из `history/PLAN.md` §4 применены на месте с пометкой «пересмотрено: было X» (R-003, R-005, R-085, R-119).

Статус — ровно один на запись, одна из форм: **открыта** / **закрыта — sha** / **смягчена — чем** / **отложена — этап** / **снята — почему** / **не пересматривалась после 09-04** (ни один документ после 09-04 о записи не говорит; рядом — куда её относил план 09-04). Каждый sha проверен `git cat-file -e <sha>^{commit}` в `/home/djadjka/Work/fluentbase` 2026-09-09 [KNOWN]; ненайденных нет (отчёт §3).

Строка «Ход по плану 09-04 (INDEX)» — столбцы «Этап / Как — Что делать» из `history/INDEX.md`: `С` закрывается структурной работой, `С+` смягчается с остатком, `П` поштучная правка, `—` не трогать. Это исходная точка, а не статус.

Соответствие «работа плана → коммит» — `PLAN.md` §2 (здесь sha только в статусах записей).

## §1 R-001..R-123

### R-001 · BLOCKER · Прыжок cold-start/re-jump синхронизирует reth к неаутентифицированному хэшу, а комитет для проверки читается из уже синхронизированного состояния
- Механизм: любой отслеживаемый пир отвечает на `Latest` парой `(finalization, block)`, где `payload == block.digest`, `block.height > anchor + threshold`, `block.result` — хэш его собственной EVM-ветки с подменённым стейкинг-состоянием. Узел даёт reth FCU на этот хэш, reth скачивает ветку по devp2p и финализирует её (эксперимент AUDIT A-1, п. 3), после чего `verify_jump_authenticated` читает `committee[E]` из подменённого состояния и проверка проходит. Эпоха раунда сертификата не сверяется с `epoch_of(block.height)`.
- Последствие: (а) узел живёт на чужой цепи, RPC отдаёт чужое состояние, валидатор перестаёт участвовать; (б) если атакующий сохранил настоящий комитет — через K блоков `ResultDivergence` → `SafetyHalt` с маркером на диске (`executor.rs`), узел выключен до ручного вмешательства; (в) — снят Ex-2, см. статус — даже при `AuthFailed` reth уже получил `finalized` на чужой хэш и начал pipeline-backfill к нему, обратный FCU reth отклоняет; узел ротирует upstream и повторяет прыжок (`dpos.rs`), но EL остаётся с чужим finalized-указателем (это C-08).
- Якоря: `cold_start_jump.rs` (порядок `get_latest` → `verify_jump_structural` → `el.sync_to` → `verify_jump_authenticated`), (`sync_to`: FCU `head=safe=finalized=latest.block.result`), (`scheme_at(epoch, landing_hash, None)` — комитет из состояния на `landing_hash`). Входы: `dpos.rs` (cold start валидатора, L1 = `None`), (re-jump, L1 = `None`), `executor.rs` (`maybe_re_jump`), `plane_upstream.rs ` (`deliver` принимает любую декодируемую пару), (`fetch_one`: `FrontierKey::Latest` — резолвер сам выбирает пира из отслеживаемого набора). Отслеживаемый набор = реестр ∪ `committee[e]` ∪ `committee[e+1]` (`staking-reader/src/epoch_transition.rs`), до 4096 (`p2p/src/constants.rs`).
- Уверенность (REGISTER, 09-03): `[KNOWN]` порядок вызовов, чтение комитета при `landing_hash`, отсутствие L1 на валидаторе, `deliver`/`fetch_one`. Эксперимент AUDIT A-1 (reth финализирует названный хэш после devp2p-доставки) — `[LIKELY]`, в сессии не повторялся.
- Ход по плану 09-04 (INDEX): Э4 / С — П-4 (фронтир только по проверенной финализации; `sync_to` после аутентификации) + П-1 (комитет из локально финализированного состояния)
- Связано: R-004 (ложный frontier заставляет здоровый узел прыгать), R-016 (порог прыжка мал), R-009 (`deliver` не связывает ключ с содержимым), R-040 (эпоха ↔ высота не связаны на by-height путях), R-007 (re-jump — единственный выход из σ-hold, то есть R-007 ведёт сюда).
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э4 4.2 (П-4, вместе с П-1). Ex-1/Ex-23 подтвердили звенья, Ex-2 снял пункт (в) — BLOCKER держится на (а)/(б). _Источник:_ history/EXPERIMENTS.md Ex-1, Ex-2, Ex-23; history/PLAN.md §2 Э4
- **Статус 2026-09-10 (стенд, Э3 закрытие, 3.3 B1):** воспроизведено частично — `testbed::tests::a_lying_upstream_lands_a_divergent_branch_and_authentication_refuses_it` (вариант Б: настоящая финализация, подменён только `result`, payload переставлен). Показано: с отдаваемой дивергентной веткой (`ElNetwork` стенда по хэшу) прыжок садится, и `verify_jump_authenticated` отвергает — `AuthFailed` на BLS-плече (`outcome_detail` «FAILED BLS verification against committee[E]»), потому что переставленный `proposal.payload` ломает мультиподпись при ЛЮБОМ комитете. Не достигнуто: сам механизм записи — проверка, ПРОХОДЯЩАЯ на подменённом состоянии (состав комитета `FakeStaking` не зависит от хэша); (б) SafetyHalt — гейт отвергает до исполнения; (в) — снята Ex-2, посадка на стенде — модель `land_canonical` (список хэшей без тел), не reth. Вариант А требует per-hash комитета в `FakeStaking` И переподписи 2f+1 ключами. _Источник:_ history/E3-CLOSEOUT.md §4
- **Статус 2026-09-12 (Э4 4.2 закрыта):** закрыта — `7cac7f3b` (4.2-А: `FrontierHandler::deliver` — единственная точка доверия: cap-декод, `block.height == h`, `epoch_of(height) == round.epoch`, `committee(epoch)` из модуля, BLS ⇒ `verify_block`), `20f47287` (4.2-Б1: цель прыжка — СОБСТВЕННАЯ архивная пара `pair_at(tip)`, посадка сверяется `holds(result)` ⇒ `InvalidTarget` ⇒ `Fault::corruption`; `upstream_frontier`/`live_height`/`servable` удалены), `dacd1bfa` (4.2-Б2: холодный старт `ColdStartKind::ElFinalized` на собственном EL-теге reth без `get_latest`; свежий follower входит только по операторскому checkpoint `sync_to_checkpoint(hash)`, на deployed-сети без checkpoint — отказ; стадии `verify_jump_*` и `JumpOutcome::{BadTarget, AuthFailed}` удалены — `sync_to` больше не получает неаутентифицированного хэша). Остаток (приёмка Б2, B2-01): лестница догона требует живого члена `committee(T±1)` собственной старой эпохи; escape — `--dpos.follower-upstream`. Живого прогона на девнете нет. _Источник:_ history/E4-2-A.md, E4-2-B1.md, E4-2-B2.md, E4-ORCHESTRATOR.md

### R-002 · BLOCKER · Эквивокация dealer-лога навсегда лишает share честные узлы, записавшие «не тот» лог; один византийский dealer, удерживающий partial, останавливает выпуск seed
- Механизм: dealer B шлёт `Reveal(L1)` узлу A и `Reveal(L2)` узлам C, D. Confirm'ы hash-sensitive; лидер пинует набор с H(L2), confirm'ов C+D+B = quorum ⇒ артефакт пинует L2. У A `all_held = false` навсегда: refetch не идёт (dealer уже в `recorded`), recompute зациклен. A без share на эпоху и все carry-forward эпохи. Подписантов seed остаётся n−1−f = t−1 честных ⇒ каждый seed требует partial от B; B удерживает ⇒ ни одного сертификата (`beacon/oracle.rs`: «NO certificate of this epoch can be assembled»). Обнаружение двух валидных подписей одного dealer'а под разными логами нигде не выполняется.
- Последствие: остановка цепи до смены комитета (которая сама требует блоков) при одном византийском участнике. Порог живучести beacon ниже BFT-границы. Охват шире, чем описан механизм: одна жертва — случай f = 1; общий случай — f жертв одним dealer'ом; порог живучести маяка — не f, а 0 (`history/VERIFY-BLOCKERS.md` R-002 «Тяжесть»). Ссылка `oracle.rs` в записи использована не по назначению (там же).
- Якоря: `beacon/ceremony.rs` (`record_checked_log`:`recorded.insert(pk)` — первый лог dealer'а побеждает, хэш не участвует), (`ingest_signed_log`: валидный лог того же dealer'а с другим хэшем ⇒ `(true, empty)` — fetch считается выполненным, лог отброшен), (`scoped_pinned_logs`: несовпадение хэша ⇒ `missing`), `beacon/actor.rs ` (`fetch_missing_logs` пропускает dealer'ов из `recorded`), (`want = dealers − held` по ключу dealer'а), (`validate_share_on_poly` ложен ⇒ запись остаётся в `recompute_pending`), `dkg_agree.rs` (`verify` паркуется на `Missing`, `fetch_bodies` доставляет лог, который отбрасывается). Комментарий кода признаёт остаток: `actor.rs`.
- Уверенность (REGISTER, 09-03): `[KNOWN]` все звенья (`record_checked_log`,`ingest_signed_log`,`scoped_pinned_logs`,`fetch_missing_logs`,`want`, gate recompute, `oracle.rs`). Предположение: порог seed t = quorum(n) — `[LIKELY]` по UNDERSTANDING §7 (`combined_scheme.rs`), в сессии не перечитано.
- Ход по плану 09-04 (INDEX): Э5 / С — П-9: идентичность лога `(dealer, hash)`; refetch по несовпадению хэша; две подписи одного dealer — улика
- Связано: R-036 (честная эквивокация лога через `NoFile` даёт тот же раскол без злого умысла), R-039 (share не проверяется против полинома на live-пути), R-025 (окно heal — 1 эпоха, поэтому жертва не восстановится и позже), R-026.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э5 5.2 (П-9); стенд Ex-22 не ставился. _Источник:_ history/PLAN.md §2 Э5; history/EXPERIMENTS.md ч.3
- **Статус 2026-09-09 (стенд, Э3.3 сессия A):** воспроизведено — `testbed::tests::a_dealer_with_two_logs_leaves_the_addressed_victim_without_a_share` (жертва без share, цепь идёт: трое = quorum(4)) и `…a_two_log_dealer_that_also_withholds_its_partial_stops_the_chain_silently` (молчаливая остановка на 63 от одного участника), sha `780f7e8e`/`3e3e9ff9`; порог seed перечитан: `M::quorum(participants)` (`bls/src/combined_scheme.rs::assemble`) — `[LIKELY]` снят; ссылка на `beacon/oracle.rs` в механизме — лог, не место расчёта порога. Не достигнуто: f жертв одним dealer'ом, carry-forward эпохи, чей хэш попал в пин (в прогоне пин — L1, confirm жертвы в артефакт не вошёл). По плану — Э5 5.2 (П-9). _Источник:_ history/E3-3-ROLES-1.md §5

### R-003 · SERIOUS (пересмотрено 09-04: было BLOCKER — нужны f+1 записей Active-реестра, туда пускает только governance) · `corroborate_frontier` считает любых отслеживаемых пиров; f+1 записей реестра переводят валидатор в verify-only до рестарта
- Механизм: при n=4 порог 2. Два пира из реестра (не из комитета) шлют по одному кадру на VOTE-сабканал с эпохой `10^9`.`highest_observed_epoch = 10^9`; каждая настоящая граница «не live», `reconcile_roles` делает `soft_enter` и выходит; `PINS_PER_SENDER` не мешает (одна эпоха на отправителя).
- Последствие: узел не спавнит движки до рестарта. Обработав так > f валидаторов, цепь останавливают. Нужно f+1 отслеживаемых идентичностей, то есть f+1 записей реестра; попасть в реестр (`getRegistryWithKeys` = список активных) можно только через `activateValidator`, а он governance-only (`consensus.rs`, `staking.rs`), так что f+1 идентичностей — не действие одного участника.
- Якоря: `epoch_manager.rs` (`threshold = (n−1)/3 + 1`, отправитель — любой аутентифицированный пир; `highest_observed_epoch` только растёт), (`is_live_epoch = epoch >= highest_observed_epoch`), (не live ⇒ только `soft_enter`, `return`); источник `their_epoch` — id сабканала кадра без подписи и без проверки членства (`node/dpos.rs`). Отслеживаемый набор — `staking-reader/src/epoch_transition.rs`.
- Уверенность (REGISTER, 09-03): `[KNOWN]` порог, отсутствие фильтра членства, `is_live_epoch`,`soft_enter`-ветка.
- Ход по плану 09-04 (INDEX): Э4 / С — П-4: `corroborate_frontier`,`observed_reporters`,`sender_pins` удаляются; тяжесть → SERIOUS (см. PLAN §8)
- Связано: R-004 (тот же класс — неподписанный сигнал двигает состояние узла), R-096 (hint целится в отправителя backup-голоса).
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э4 4.2 (П-4: `corroborate_frontier`, `observed_reporters`, `sender_pins` удаляются). _Источник:_ history/PLAN.md §4, §2 Э4
- **Статус 2026-09-12 (Э4 4.2 закрыта):** закрыта — `48bf62ed` + `bcebd3b7` (4.2-В: `is_live_epoch(E) = E == live_epoch(tip)`, `live_epoch(tip) = epoch_of(tip)` и `E+1` на `tip == last(E)`; tip — проверенная финализация marshal'а через `FluentApp::ordering_tip()` (`watch<u64>`, писатель — арм `Update::Tip` в `report`); `corroborate_frontier`, `observed_reporters`, `sender_pins`, `highest_observed_epoch`, `PINS_PER_SENDER`, `latest_live` удалены; у `Actor` не осталось поля, которое пишет обработчик входящего кадра — тегу эпохи с провода нечего двигать). Стенд-пин: `a_catching_up_member_takes_verify_only_at_every_boundary_below_its_own_tip` (партиция одного узла при неизменном комитете и статическом beacon'е — гейт живости единственный отказ; `is_live_epoch_at → true` красит, прогон оркестратора). Расхождение с текстом проекта §5.1 (конъюнкция `tip < last(E)` не тотальна на границе) — Д-112. _Источник:_ history/E4-2-V.md, E4-2-V-REVIEW.md

### R-004 · BLOCKER · `upstream_frontier` поднимается одним ответом на пробу и никогда не убывает; re-jump спавнится на каждый tip и блокирует исполнение живого валидатора на время watchdog'а
- Механизм: на здоровом валидаторе один тик без смены tip (обычный джиттер при 1 blk/s) вызывает пробу; вредоносный пир отвечает высотой `ordering_finalized + 10^6`. После этого `maybe_re_jump` спавнит прыжок на каждый `Update::Tip` и heartbeat; когда `get_latest` попадает на того же пира, reth получает FCU на несуществующий хэш, `sync_to` крутится до `StalledWithPeers` (300 с), всё это время derive заблокирован. Затем цикл повторяется. Один пир отслеживается всеми валидаторами.
- Последствие: валидатор не исполняет блоки ⇒ через K блоков result-gate в `verify_block` отказывает ⇒ не голосует. Один пир, обработав > f валидаторов, останавливает сеть. Даже при промахах пробы по честным пирам (`Lagging`)`upstream_frontier` остаётся вздутым и прыжки спавнятся на каждый tip.
- Якоря: `executor.rs` (`probe_frontier`:`get_latest` у любого пира → `fetch_max`), (`maybe_re_jump`:`max(tip, upstream_frontier) − ordering_finalized > threshold`, значение не убывает, подписи нет), (derive и спекуляция подавлены, пока `jump_done.is_some`), (`StalledWithPeers` не ротирует, перевооружается на следующий tip), `cold_start_jump.rs` (`sync_to` ждёт `Valid` до 300 с при `peers > 0` / 6 ч), (проба в `select!`).
- Уверенность (REGISTER, 09-03): `[KNOWN]fetch_max`, гейт `maybe_re_jump`, гейт derive на `jump_done`, поведение `StalledWithPeers`.`[GUESS]` доля попаданий пробы в атакующего (резолвер выбирает по задержке, CORE D-4). Эксперимент Ex-4 (отложен, часть 3 `EXPERIMENTS.md`).
- Ход по плану 09-04 (INDEX): Э4 / С — П-4: `upstream_frontier` поднимается только проверенным сертификатом; проба вне цикла executor
- Связано: R-001 (успешное попадание — захват), R-016, R-031 (проба выполняется inline в `select!`), R-003.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э4 4.2 (П-4). Ex-2 снял часть «честные FCU после SYNCING не спасают» на стороне EL; гейт узла (`executor.rs` `jump_done`) в силе. _Источник:_ history/EXPERIMENTS.md Ex-2; history/PLAN.md §2 Э4
- **Статус 2026-09-10 (стенд, Э3 закрытие, 3.3 B1):** воспроизведено частично — `testbed::tests::an_inflated_latest_probe_wedges_a_rotated_out_validator_in_re_jumps` с честным контролем в том же тесте. Показано: один раздутый ответ на `Latest` поднимает `upstream_frontier` до `real + 10^6` (`peak == inflate_to`), каждый прыжок кончается `Stalled`, контроль садится и уходит за 95, роль заклинена на 95. Не показано: механизм записи «FCU на несуществующий хэш → `StalledWithPeers` 300 с → derive заблокирован» — роль сохраняет настоящий `result`, прод отвечает `Valid` и падает на высоте посадки (`Stalled`, ротация); стоящий гейт клина — `awaiting_seed` (`executor.rs:1418`), не `jump_done`; клин только по ордер-курсору (EL узла унесён вперёд). Здоровая жертва недостижима: в детерминированном рантайме `BLOCK_INTERVAL == FRONTIER_PROBE_INTERVAL == 1 с`, проба здорового узла не срабатывает — вопрос латентности девнета. _Источник:_ history/E3-CLOSEOUT.md §4
- **Статус 2026-09-12 (Э4 4.2 закрыта):** закрыта — `20f47287` (4.2-Б1: `upstream_frontier`, `live_height`, `servable`, `upstream_frontier_series` удалены; триггер re-jump — `tip − ordering_finalized > threshold` по СОБСТВЕННОМУ marshal tip'у, цель — своя архивная пара; ответ на пробу узел больше не запоминает), `8a205eae` (4.2-Б2: ответ вне окна `[epoch(anchor)−8, epoch(anchor)+2]` или нечитаемой эпохи отбрасывается на обоих ключах с `dpos_frontier_dropped_total{reason}`, пир не наказывается). Стенд: `the_rejump_runs_the_production_jump_and_lands_on_its_own_archive_pair`, лестница `the_ladder_names_successive_rungs_and_the_lagging_node_reaches_every_one`. Остаток: живой прогон лестницы на девнете (приёмка 4.2); ступень как ПРИЧИНА подъёма на стенде не показана (Д-95). _Источник:_ history/E4-2-B1.md, E4-2-B2.md

### R-111 · BLOCKER · Популяция селекционно-видимых валидаторов ничем не удерживается выше `MIN_COMMITTEE_LENGTH`: как только видимых остаётся 3, системный вызов `commitEpochCommittee` ревёртит в предысполнении и вся сеть одновременно умирает фатальным `Corruption`, необратимо. Роковой шаг — обычная успешная транзакция честного оператора
- Механизм: (1) владелец валидатора снимает весь собственный стейк одной транзакцией `undelegate` — либо governance вызывает `disable_validator`; (2) `set_selection_visible(v, false, E)` (`staking.rs`) делает это действующим с эпохи `E+1`; (3) на первом блоке эпохи `E+1` узел коммитит `target = E+3`, контракт выбирает при `selection_epoch = E+1`; (4) если видимых осталось 3 — ревёрт; (5) узел превращает его в `BlockExecutionError` → `Corruption` → смерть executor'а → abort-all. Одновременность — общий вход: чистая функция от согласованного блока и его пред-состояния (`consensus.rs` утверждает это же и добавляет, что чинящая транзакция после этого невозможна).
- Последствие: необратимая остановка ВСЕЙ сети. Узлы умирают в пределах 3, 2–14 мс друг от друга; перезапуск воспроизводит ту же смерть; состояние живёт в контракте, а исправить его нечем — транзакция требует блока, блоков больше нет. Инициатор гибнет вместе со всеми.
- Якоря: контракт, `/home/djadjka/Work/audit-482/pr482-study/contracts/staking/src`):`consensus.rs ` (`commit_epoch_committee`;`selection_epoch = target − 2`, ревёрт `ERR_COMMITTEE_TOO_SMALL` с `(len, MIN)`), `consts.rs ` (`MIN_COMMITTEE_LENGTH = 4` и его обоснование), `consensus.rs` → `staking.rs` → `staking.rs` (кандидаты = видимые члены роестра) → `staking.rs` (`k = min(cap, len)`), `staking.rs ` (роестр только растёт). Небайзантийские писатели штампа невидимости, оба БЕЗ floor-проверки: `staking.rs` (`deactivate_validator_at`) ← `staking.rs` (`disable_validator`, governance) и ← `staking.rs, :1295-1300` (`undelegate` при `full_owner_exit`, **permissionless**; `staking.rs`: «A full exit is therefore never blocked»). Защищённый аналог существует и не переиспользован: `staking.rs` (`apply_production_exclusion` отказывает при `count_selection_visible_at(bite) <= active_validators_length_at(bite)`), причём против ПОТОЛКА, не против минимума.
- Уверенность (REGISTER, 09-03): **подтверждено экспериментом** (`EXPERIMENTS.md` §5.3, §5.5.5). E2 (`n = 4`, один выход `undelegate`: мертвы все четыре узла, ноль строк о тумбстоуне). E5 (`n = 6`, старт с `V = 5`: выход `V 5→4` — цепь живёт четыре эпохи; выход `V 4→3` — четыре узла умирают за 9 мс на `commitEpochCommittee(epoch 27)`; всего от `V = 6` понадобилось три честных выхода = `V − 3`; строк о джейле/эквивокации во всех шести логах — 0). Первый прогон E5 был негоден (гонка с фазой 1 стенда) и переигран с гейтом готовности. Достижимость при `V = 51` — по коду (`EXPERIMENTS.md` §5.5.3): 48 выходов, ни один ничем не ограничен; стенд на 51 узел не строился.
- Ход по плану 09-04 (INDEX): Э0 / С — контракт B-1: коммит не ревёртит, при недоборе переиспользует предыдущую запись; `dkg_qual=false`
- Связано: R-112 (тот же ревёрт, тот же фатал, но византийский писатель того же штампа — разделены, потому что разная достижимость и разный ответственный), R-113 (пропущенный инвариант, общая причина обеих), R-032/R-033 (класс «`Corruption` ⇒ shutdown»; здесь наблюдён end-to-end на пути ИСПОЛНЕНИЯ у валидатора).
- **Статус 2026-09-09:** снята — принято решением 1.0 (`c31c258f`, 09-07): перенос комитета удалён, ниже пола — отказ `CommitteeTooSmall`, остановка цепи — проектный исход (BFT-граница). Живьём 09-08: N=4 exit/byz — все узлы вышли с кодом 0, отказ ловится в плане ИСПОЛНЕНИЯ (`derive.rs`, `stage="finalize"`) на всех узлах (F5). Остаток принят: на сети ровно на полу один выход/тумбстоун = остановка транзакцией (R0.1, X2). Carry-over (Э0.2, `b22a8ed1`) — отменён. _Источник:_ history/PLAN.md §2 Э1 1.0; history/E1-CLOSEOUT.md §2, F5; history/E1-REFLECTION.md R0.1, X2

### R-006 · SERIOUS · Guard #2 читает канонический хэш reth до FCU целевого блока: на догоне проверка пуста, при спекулятивном сиблинге — ложный SafetyHalt
- Механизм: узел отстал ≥ K и догоняет без спекуляции; `block_hash(h) = None` ⇒ `result_matches = None` ⇒ guard молча пропускает; расхождение ловится только обратной проверкой на h+K, когда h..h+2 уже FCU'нуты как `safe`. Сценарий 2: узел спекулятивно исполнил сиблинг A на высоте h (нотаризация, затем nullify), сеть финализировала B, узел отстал ≥ K; `correctly_speculated = false`, re-derive B → import без FCU; `block_hash(h) = A ≠ hash(B)` ⇒ `Some(false)` ⇒ `Fault::fork_safety(ResultDivergence)` ⇒ `park_halted` + маркер.
- Последствие: сценарий 1 — заявленный «единственный детектор на пути догона» не работает (комментарий); сценарий 2 — необратимая остановка честного узла при штатной комбинации «таймаут лидера + отставание на K». Тест `guard2_convergence_mismatch_engages_safety_halt` (`executor.rs`) проходит, потому что `FakeDeriver` канонизирует при derive, чего reth не делает.
- Якоря: `executor.rs` (guard #2: `result_matches(block_{h+K}.result, …, spec_executed_hash)`; срабатывает только на `Some(false)`), → (для целевого блока только `submit_finalized_payload`, без FCU), (FCU целевого блока — позже), (спекулятивный блок канонизируется FCU `head=derived`), (`correctly_speculated`).`spec_executed_hash = provider.block_hash(h)` — каноническая цепь (`node/ordering.rs`); `order_block.rs` (`result_matches` ⇒ `None`, когда хэша нет). reth: `InsertExecutedBlock` не меняет канонической цепи (`RETH:crates/engine/tree/src/tree/mod.rs`, по AUDIT/CORE).
- Уверенность (REGISTER, 09-03): `[KNOWN]` порядок guard → import → FCU, `result_matches`, FCU в `spec_execute`,`correctly_speculated`.`[LIKELY]` поведение `InsertExecutedBlock` в reth (по анкерам CORE/AUDIT, checkout не открывался).
- Ход по плану 09-04 (INDEX): Э6 / С — П-6 п.3: guard #2 сравнивает с локальным `derived_hash`; тест переписать на conformance-фейке (Э3)
- Связано: R-015 (тот же постусловный блок `try_derive`), R-050 (маркер halt), R-074.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э6 6.1 (П-6 п.3: guard #2 сравнивает с локальным `derived_hash`). Ex-5: сценарий 2 не воспроизвёлся; сценарий 1 открыт (нужен расходящийся блок, Э3.3). _Источник:_ history/EXPERIMENTS.md Ex-5; history/PLAN.md §2 Э6
- **Статус 2026-09-10 (стенд, Э3 закрытие, 3.1):** сценарий 1 воспроизведён — `testbed::tests::guard_two_on_the_catch_up_path_reads_a_pre_fcu_height`: после того как `FakeChain` получил семантику reth (канон только по FCU), узел, догоняющий без спекуляции на ≥ K, при расходящемся блоке h=6 НЕ останавливается guard'ом #2 (`spec_executed_hash(6)` = `None` до FCU ⇒ `result_matches` = `None` ⇒ молчаливый пропуск; лог `el_events`: `Derived(6)` раньше `Canonicalized(6)`); 6, 7, 8 FCU'нуты и курсор продвинут; halt приходит обратной проверкой `h − K` на 9 (`executor.rs:3247`). Под старой семантикой фейка guard #2 срабатывал на 6 — ровно ложь, о которой говорит запись. `executor.rs:7448` по-прежнему зелёный на своей фикстуре с ранней канонизацией. Сценарий 2 — не ставился. _Источник:_ history/E3-CLOSEOUT.md §3

### R-007 · SERIOUS · σ захватывается только из `Activity::Notarization` и из by-height backfill; финализация без локально виденной нотаризации держит блок в `awaiting_seed` до порога re-jump
- Механизм: узел получает `Certificate::Finalization` вида, нотаризацию которого не собрал и не получил (CW сообщает `Finalization` независимо: `voter/actor.rs`; батчер не строит сертификаты для view ≤ finalized, `batcher/actor.rs` — по AUDIT). Marshal доставляет блок, `seed_for(Round(E, v)) = None`; сертификат финализации содержит тот же σ, но его никто не извлёк.
- Последствие: исполнение стоит до `min(1024, interval)` блоков (≈17 мин при 1 blk/s, одна эпоха при малом интервале); всё это время узел не проходит result-gate и не голосует; выход — re-jump, то есть поверхность R-001. AUDIT A-4 писал «навсегда» — уточнено по коду: re-jump снимает hold (`reseed_forward` ack'ает `awaiting_seed`, `executor.rs`). Одновременный hold у > f валидаторов (кратковременный разрыв сети) останавливает цепь на это время.
- Якоря: `spec_exec.rs` (`let Activity::Notarization(n) = activity else { return }`); полный список точек захвата: `spec_exec.rs`, `cert_inlet.rs` (follower ingest; `UpstreamResolver::spawn_finalized`, только Hybrid); `outer.rs` — при резолвере `Plane` захвата из backfill нет (в проде недостижимо: node всегда даёт `Some(upstream)`, `node/dpos.rs` [ссылка исправлена, history/REFS.md], `dpos.rs`). Hold: `executor.rs` (детектор только логирует через 60 с; гейт останавливает все финализации; `maybe_re_jump` намеренно не гейтится на hold — комментарий там же), порог `min(1024, interval)` (`dpos.rs`).
- Уверенность (REGISTER, 09-03): `[KNOWN]spec_exec.rs`, hold, детектор, гейты, порог. `[LIKELY]` независимая доставка `Finalization` в commonware (по AUDIT-анкерам).
- Ход по плану 09-04 (INDEX): Э5 / С — П-2: σ читается из `get_finalization(h)` marshal; выход из удержания — `hint_finalized`
- Связано: R-008 (второй путь к тому же hold), R-001, R-016, R-018, R-020; закрывается AUDIT B-1.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э5 5.3 (П-2 или Д-9). Ex-6: механизм подтверждён (~0,5 события на 3-с разрыв), удержание < 60 с; >f одновременно на n=4 не проверяемо. _Источник:_ history/EXPERIMENTS.md Ex-6; history/PLAN.md §2 Э5

### R-008 · SERIOUS · `verify_certificate` при `SeedCheck::NoKey` принимает любой σ; follower без `PK_E` кладёт подделанный сертификат в архив, а после прихода ключа σ отбрасывается и блок держится
- Механизм: upstream (или пир по FRONTIER) отдаёт валидный multisig-сертификат с изменённым слотом σ, пока у follower нет `PK_E`. Сертификат проходит, блок доставлен, σ в карантине. При приходе ключа σ отброшен как `Invalid`;`seed_for(round) = None`; executor держит блок. `record_data_fault` не вызывается (verify прошёл) — ротации upstream нет. Архив финализаций содержит сертификат с плохим σ и отдаёт его другим узлам, которые видят data fault и ротируют от честного follower.
- Последствие: остановка follower до re-jump (следует из того же гейта, что R-007; у follower re-jump есть, `dpos.rs` симметричен) и отравление архива. AUDIT писал «до ручного вмешательства» — уточнено: hold снимается re-jump'ом, отравление архива остаётся.
- Якоря: `bls/src/combined_scheme.rs` (`NoKey` ⇒ принять); `cert_inlet.rs` (`ensure_key` только `Local`), (verify → `capture_certificate_seed` → карантин), (сертификат уходит в marshal через `verify_block` + `report_finalization`);`beacon/certify.rs ` (`promote_epoch`:`Invalid` ⇒ удалить — «чтобы раунд можно было запросить снова», но по-раундового запроса больше нет: `beacon/log_resolver.rs`, `TAG_SEED_RETIRED`);`executor.rs ` hold.
- Уверенность (REGISTER, 09-03): `[KNOWN]NoKey`-ветка,`Local`-effort, путь в marshal, `promote_epoch`,`TAG_SEED_RETIRED`. Не проверено: как marshal отдаёт отравленный сертификат при backfill (COVERAGE §5 п. 3, п. 8).
- Ход по плану 09-04 (INDEX): Э5 / С+ — П-2 даёт выход из удержания; отравление архива остаётся, пока не решено Д-3 (`NoKey`-приём)
- Связано: R-007, R-069 (`on_invalid_seed` на carry-forward эпохах всегда `Quarantine`), R-063; закрывается AUDIT B-1.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э5 5.3 смягчает (П-2 даёт выход из удержания); отравление архива — открытое решение Д-3; Ex-21 в очереди первым. _Источник:_ history/PLAN.md §2 Э5, §5 Д-3; history/EXPERIMENTS.md ч.3
- **Статус 2026-09-09 (стенд, Э3.3 сессия A; формулировка уточнена 09-10 по `history/E3-REVIEW.md` §2 — «целиком» было переоценкой, два звена из семи не достигнуты):** воспроизведено, включая отравление архива — `testbed::tests::a_forged_seed_slot_is_admitted_with_no_key_and_refused_when_the_key_lands`, sha `780f7e8e`/`3e3e9ff9`: подменённый слот σ принят в окне `NoKey` (`verify_certificate` → `verify_seed`), шесть подделанных раундов отвергнуты `promote_epoch` при приходе `PK_2`, follower отдал подделку дальше (`served_seed_replays`). Не достигнуто: ротация от честного follower'а (второй follower тоже без ключа), R-069 (эпоха mint), плечо `CertInlet::ingest`/`ensure_key(Local)` — стенд без inlet'а; `record_data_fault` в стенде не существует, «ротации нет» там тривиально. Ex-21 на девнете — приёмка после Э5.3 (3.4 отложено 09-10). _Источник:_ history/E3-3-ROLES-1.md §5; history/E3-CLOSEOUT.md §6
- **Статус 2026-09-13 (стенд, Э5 5.0а, `7790d1bc`):** закрыто плечо, названное недостижимым выше — `CertInlet::ingest`/`ensure_key(Local)` в стенде ЕСТЬ, и `record_data_fault` наблюдаем. Два арма в `testbed/cert_inlet_tests.rs`: KEYLESS — узел вне `committee[2]` допускает сертификаты эпохи 2 с НЕПРОВЕРЕННОЙ σ (`dpos_cert_vote_only_admissions_total = 1683`) и это НЕ стоит ротации, то есть «отсутствие ключа — не data fault» теперь пиновано, а не тривиально; KEYED — тот же класс подделки при УЖЕ полученном `PK_2` валит BLS в `ingest` и стоит ровно `6 / MAX_UPSTREAM_FAULTS = 2` ротаций, причём `carry_forward_fails = 6` (`cert_inlet.rs:666-668`) свидетельствует, что ключ держался в момент каждого вердикта. Механизм, ради которого это вообще работает (проверен оркестратором по коду): плоскость строит свой верификатор БЕЗ оракула эпохи (`plane_upstream.rs:382-387` — `build_verifier(.., None)`), поэтому подделанная σ под целым multisig проходит `deliver` и доходит до inlet-а, где схема уже с оракулом (`cert_inlet.rs:620`). Остаётся открытым: ротация от ЧЕСТНОГО follower'а и R-069 (эпоха mint) — как было; девнетная приёмка Ex-21 — после 5.2. _Источник:_ history/E5-0a-A.md §0; history/E5-ORCHESTRATOR.md

### R-009 · SERIOUS · `FrontierHandler::deliver` не связывает ключ с содержимым: быстрый пир «удовлетворяет» `Finalized{h}` чужой высотой и морит gap-repair
- Механизм: валидатор догоняет по plane-пути (единственный by-height путь в Hybrid — upstream, `outer.rs`). Самый быстрый пир на каждый `Finalized{h}` отвечает валидной парой высоты h−1. Дыра на h не закрывается, пока пир остаётся самым быстрым.
- Последствие: догон одного узла стоит; выход — re-jump после порога (R-001). Один пир против любого догоняющего валидатора.
- Якоря: `plane_upstream.rs` (`true` для любой декодируемой пары; высота с `key` не сверяется); `cert_inlet.rs` (`spawn_finalized` при `false` ничего не делает, при `true` захватывает σ раунда из ответа); marshal отвергает несовпадение высоты (`CW:marshal/core/actor.rs`, по CORE), но резолвер уже считает fetch выполненным и пира не штрафует; следующий sweep выбирает пира снова по задержке (`CW:resolver/src/p2p/fetcher.rs`, по CORE).
- Уверенность (REGISTER, 09-03): `[KNOWN]deliver` и `spawn_finalized`.`[LIKELY]` поведение резолвера при `deliver == true` с неверным содержимым (CORE D-4, Ex-7).
- Ход по плану 09-04 (INDEX): Э4 / С — П-4: `deliver` сверяет высоту и эпоху с ключом, `false` на несовпадение
- Связано: R-001, R-003, R-063, R-095.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э4 4.2 (П-4: `deliver` сверяет высоту и эпоху с ключом); Ex-7 в очереди. _Источник:_ history/PLAN.md §2 Э4
- **Статус 2026-09-10 (стенд, Э3 закрытие, 3.3 B2):** воспроизведено — `testbed::tests::a_wrong_height_answer_satisfies_the_fetch_and_starves_the_by_height_gap` (`Role::WrongHeightFinalized`, честный контроль в том же тесте). Единственный upstream отвечает на каждый `Finalized{h}` своей настоящей парой высоты h−1 (самопроверка `height == h−1`, `payload == digest`); жертва приняла все 42 пары (`deliver` не сверяет высоту с ключом — `plane_upstream.rs:201`; `deliveries_rejected` считает только ошибки декода), 43 by-height запроса «доставлены», marshal молча отверг несовпадение высоты (`CW:marshal/core/actor.rs:987-993`, `send_lossy(false)`, без лога — выведено дифференциально, стенд захватывает только WARN), дыра не закрылась: узел стоит на 5, контроль дошёл до 15; re-jump выключен (`rejump_calls == 0` — гейт фикстуры). Не поставлено: мультипировая гонка «пока пир самый быстрый» (один источник; `[LIKELY]` про resolver не снят), плечо `CertInlet::ingest`, штраф/ротация пира. По плану — Э4 4.2 (П-4: `deliver` сверяет высоту и эпоху с ключом). _Источник:_ history/E3-CLOSEOUT.md §5
- **Статус 2026-09-12 (Э4 4.2 закрыта):** закрыта — `7cac7f3b` (4.2-А: `deliver` сверяет `block.height == h` и `epoch_of(height) == round.epoch` с ключом до `verify_block`; несовпадение — `false`, CW исключает пира). Стенд Э3 `a_wrong_height_answer_satisfies_the_fetch_and_starves_the_by_height_gap` переписан как «после»: лжец отвергнут, догон от честного. Ex-7 из очереди снят. _Источник:_ history/E4-2-A.md

### R-010 · SERIOUS · `verify_block` не проверяет `fee_recipient`
- Механизм: лидер ставит любой адрес; верификаторы голосуют «да»; deriver в `crates/node` исполняет блок с этим получателем.
- Последствие: любой лидер присваивает комиссии любого блока.
- Якоря: `application.rs` (`structural_checks`:`proposal_view`, timestamp, gas, production record, Σ gas — поля `fee_recipient` нет), (`verify_block` — grep по файлу: `fee_recipient` только в конфигурации и `build_proposal`); поле `order_block.rs`.
- Уверенность (REGISTER, 09-03): `[KNOWN]`.
- Ход по плану 09-04 (INDEX): Э0 / П — удалить поле `fee_recipient` из `OrderBlock`; deriver ставит `PRECOMPILE_FEE_MANAGER` (EL иначе отвергает блок, `node/consensus.rs`)
- Связано: —. Правка: сверять с адресом, привязанным к `ctx.leader` в снапшоте (маппинг peer→address есть в `ValidatorSetSnapshot`).
- **Статус 2026-09-09:** закрыта — `30e2dd68` (Э0.3: поле `fee_recipient` удалено из `OrderBlock`, deriver ставит `PRECOMPILE_FEE_MANAGER`); стенд: пять узлов сошлись, `miner` = `0x…520fee`. Последствие пересмотрено (§4 PLAN): не кража комиссий, а неимпортируемый блок. _Источник:_ history/E0-LOG.md 0.3; history/PLAN.md §4

### R-013 · SERIOUS · BROADCAST: буфер тел держит до 64 сообщений по 4 MiB на каждого «primary»-пира; primary-набор — реестр, не комитет
- Механизм: пир из реестра за 8 с заливает 64 валидно закодированных `OrderBlock` по 4 MiB ⇒ 256 MiB на пира; сотни записей реестра ⇒ десятки GiB. CPU: до 32 MiB/с декодирования на пира.
- Последствие: удалённое исчерпание памяти и CPU.
- Якоря: `outer.rs` (`buffered::Engine` с `deque_size = self.deque_size`), `dpos.rs ` (`deque_size: 64`), `order_block.rs ` (`MAX_ORDER_BLOCK_SIZE = 4 MiB`), (`read` включает RLP-декод транзакций; подписи не проверяются), `p2p/src/constants.rs` (8/с), (4 MiB). `CW:broadcast/src/buffered/engine.rs` — кэш для любого пира из `latest.primary` (по CORE/UNDERSTANDING §12.6).
- Уверенность (REGISTER, 09-03): `[KNOWN]deque_size`, размеры, квота. `[LIKELY]` семантика `primary` в commonware (CORE D-2, Ex-8). Стоимость декодирования не измерена (Ex-9).
- Ход по плану 09-04 (INDEX): Э4 / С+ — П-5 сужает primary до комитета; отдельно `deque_size ≤ 4` + байтовый лимит в `outer.rs`
- Связано: R-037 (тот же механизм в DKG body engine), R-029 (нет блокировщика), R-054.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э4 4.3 смягчает (П-5), 4.4 закрывает (`deque_size ≤ 4` + байтовый лимит). _Источник:_ history/PLAN.md §2 Э4
- **Статус 2026-09-12 (Э4 4.3 закрыта):** закрыта — `48412eb3` (4.3-А: primary = записи `C[E−1] ∪ C[E] ∪ C[E+1]`, реестр — secondary; CW `buffered` удерживает тела только primary-отправителей; BROADCAST `deque_size` 64 → 4 в трёх местах). Байтового лимита у CW `buffered::Config` нет — бюджет `4 × MAX_ORDER_BLOCK_SIZE` на пира есть следствие, не настройка (граница библиотеки, записано в коде). _Источник:_ history/E4-3-A.md §0(1), §0(4)

### R-014 · SERIOUS · `VoteStore` хранит непроверенные голоса; далёкие view никогда не вытесняются
- Механизм: член комитета шлёт голоса со своим индексом и view `2^40..`, 128/с; каждый становится записью (≈100–200 Б). Ложные пары `Conflicting*` ⇒ `resolve_committee` (EVM) на каждую.
- Последствие: ≈1, 6 GiB в сутки на каждый узел от одного члена комитета; голоса за далёкие view не эквивокация, слэша нет. Одного византийского члена достаточно, чтобы вывести из памяти весь комитет за дни — но медленно и наблюдаемо.
- Якоря: `slasher/actor.rs` (`remember_*` без проверки подписи), (`retain_floor`:`view >= floor − 64`, верхней границы нет). Reporter получает голоса до batch-verify; батчер привязывает отправителя к индексу подписанта и отбрасывает не-участников (`CW:batcher/round.rs`, по AUDIT).
- Уверенность (REGISTER, 09-03): `[KNOWN]VoteStore`,`retain_floor`.`[LIKELY]` порядок Reporter/batch-verify в commonware. Рост — Ex-10.
- Ход по плану 09-04 (INDEX): Э4 / П — голоса в `VoteStore` только после batch-verify; view сверху ≤ `floor + activity_timeout`
- Связано: R-027, R-059.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э4 4.4 (голоса в `VoteStore` только после batch-verify; view ≤ `floor + activity_timeout`). _Источник:_ history/PLAN.md §2 Э4

### R-112 · SERIOUS · Тумбстоун за эквивокацию — третий писатель того же штампа невидимости, тоже без floor-проверки; при `V = 4` один византийский валидатор необратимо останавливает сеть
- Последствие: то же, что у R-111.
- Якоря: `consensus.rs` (`apply_equivocation_penalty`, штамп безусловный), входы — системный вызов `slashEquivocation` (`consensus.rs`) и permissionless-обработчики с уликой `slash_notarize` / `slash_from_evidence` (`consensus.rs`, — без `ensure_governance` и без `SYSTEM_CALLER`). Далее — ровно цепочка R-111, шаги (2)–(5). Единственное ограничение — одноразовость на валидатора.
- Уверенность (REGISTER, 09-03): **подтверждено экспериментом** при `n = 4` и `n = 5` (`EXPERIMENTS.md` §5.3): E1 (`n=4`, один эквивокатор — сеть мертва за 35,3 с), E3 (`n=5`, один эквивокатор — сеть ЖИВА, `V 5→4`), E4 (`n=5`, второй эквивокатор — мертва, `V 4→3`). Ревёрт во всех случаях `0x0a87ec8d(3, 4)`. Недостижимость при `V = 51` — по коду.
- Ход по плану 09-04 (INDEX): Э0 / С — то же, что R-111 (B-1); tombstoned член в перенесённом комитете маскируется узлом
- Связано: R-111 (тот же штамп, тот же ревёрт, небайзантийские писатели), R-113, R-034 (`tombstoned` как единственный источник).
- **Статус 2026-09-09:** снята — как R-111 (1.0): тумбстоун на сети ровно на полу останавливает цепь на ближайшем коммите (X2); `smoke-byzantine` держится только пятым узлом. Маскирование tombstoned-члена узлом (отказ лидеру, разрыв транспорта) не вычёркивает место. _Источник:_ history/PLAN.md §2 Э1 1.0; history/E1-REFLECTION.md X2

### R-114 · SERIOUS · ABI-сигнатуры системных вызовов объявлены независимо на обеих сторонах; промах селектора у двух fail-loud вызовов даёт фатальную ветку R-111
- Механизм: сигнатура-строка существует в двух экземплярах; селектор каждая сторона считает от своего. Правка на одной стороне (переименование, смена арности) меняет её селектор, вызовы узла перестают попадать в обработчик. Для `recordProduction` и `commitEpochCommittee` диспозиция в узле fail-loud (`evm.rs`) ⇒ `BlockExecutionError` ⇒ `Corruption` ⇒ abort-all у всех узлов одновременно, без самолечения (та же цепочка, что в R-111). Для `slashEquivocation` диспозиция мягкая (`evm.rs`) ⇒ тихая потеря слэша.
- Якоря (узел): `crates/node/src/evm.rs` (`recordProduction(uint8)`, `commitEpochCommittee`, `slashEquivocation(uint64, uint32)`) — блок `sol!`; `crates/dpos/staking-reader/src/reader.rs` — семь вьюх (`getEpochCommitteeWithStakes`, `getRegistryWithKeys`, `getDkgQual`, `getEpochBlockInterval`, `getDposActivationBlock`, `getUndelegatePeriod`, `getActiveValidatorsLength`). Контракт: те же строки, объявленные заново через `derive_keccak256_id!` — `consts.rs`. Диспетчер rWasm сопоставляет сырые 4 байта, так что расхождение строки — не мис-декод, а `ERR_UNKNOWN_METHOD`-ревёрт против живой цепи.
- Уверенность (REGISTER, 09-03): `[KNOWN]` — обе стороны прочитаны, тесты открыты.
- Ход по плану 09-04 (INDEX): Э2 / С — общий ABI-крейт: `sol!` узла и селекторы контракта из одного источника
- Связано: R-111 (та же фатальная ветка), R-113 (тот же класс «инвариант держится согласием чисел»), R-119. Полный разбор класса — `history/DUPLICATES.md`.
- **Статус 2026-09-09:** закрыта — `9b6213be` + `065003ad` (2.1: крейт `crates/staking-abi`, один `sol!`, узел удалил четыре своих; внешние свидетели `selectors_match_the_deployed_artefact_scan`, `derived_selectors_match_independent_hex_pins`). _Источник:_ history/E2-ABI.md §1 гр.3

### R-115 · SERIOUS · Пространство имён подписи для улик собрано вручную в контракте, а его суффиксы принадлежат commonware: расхождение молча выключает слэшинг по уликам
- Механизм: три величины дублированы разом — 15-байтовый префикс, порядок байт chain_id и три суффикса. Расхождение любой из них меняет подписываемое сообщение, `SIG_BLS_VERIFY` возвращает `false`, и `slash_from_evidence` ревёртит `ERR_EQUIVOCATION_SIGNATURE_INVALID` (`consensus.rs`). Это обычная транзакция, а не системный вызов: цепь продолжает работать, а путь доказательства эквивокации по уликам просто перестаёт существовать.
- Якоря: узел): `crates/dpos/bls/src/lib.rs` — `fluent_namespace(chain_id) = b"FLUENT_DPOS_V1_" ‖ chain_id.to_be_bytes`; doc явно говорит, что суффиксы `_NOTARIZE`/`_NULLIFY`/`_FINALIZE`/`_SEED` добавляет commonware, а не этот код. Источник суффиксов — `CW:consensus/src/simplex/scheme/mod.rs`. контракт): `consensus.rs`, `fn namespace(sdk, kind)` — собирает всё сам: `b"FLUENT_DPOS_V1_"` ‖ `block_chain_id.to_be_bytes` ‖ `b"_NOTARIZE" | b"_NULLIFY" | b"_FINALIZE"`, выбирая суффикс по `EVIDENCE_MESSAGE_KIND_*` (`consts.rs`).
- Уверенность (REGISTER, 09-03): `[KNOWN]` — прочитаны все три стороны (узел, контракт, checkout commonware).
- Ход по плану 09-04 (INDEX): Э2 / С — namespace+суффиксы: тест узла читает литералы commonware, контракт получает вектор из узла
- Связано: R-022 (слэшер и доставка улик), R-114, R-113. Разбор — `history/DUPLICATES.md`, группа 1.
- **Статус 2026-09-09:** отложена — Э2.3 (группа 1 DUPLICATES; исчезает вместе с `evidence.rs` при Д-4 = КB-4). Инлайн верификатора (1.10) перенёс дубль `namespace()` в `bls.rs`, не устранил. _Источник:_ history/PLAN.md §2 Э2 2.3; history/E1-REFLECTION.md R10.2

### R-116 · SERIOUS · Индексное пространство комитета задано дважды: сортировкой в контракте и порядком `Participant` в commonware; расхождение разрешает `signerIdx` в другого валидатора
- Механизм: обе стороны обязаны сортировать по одному ключу в одну сторону, но правило записано дважды и разными словами. Расхождение (смена ключа сортировки в контракте либо `Ord` в commonware) сдвигает индексы: `slashEquivocation(epoch, signerIdx)` разрешится в ДРУГОГО члена комитета, и честный валидатор получит тумбстоун и джейл за чужую эквивокацию — а это, помимо самого по себе, уменьшает популяцию по механизму R-111. Молча: ни одна сторона такого расхождения не замечает.
- Якоря: контракт): `consensus.rs members.sort_unstable_by_key(|member| member.peer_pubkey)`, обоснование — «Peer-key ascending IS the consensus index space: `record_production` credits `produced[epoch][leader_index]` and `judge` resolves that same index against this array». Разрешение индекса в личность — `committee_member_at` (`consensus.rs:~995`), единственный потребитель — системный слэш. узел/commonware): `Participant` (u32) = позиция бинарного поиска в отсортированном векторе ключей, `Ord` для ed25519 — байт-лексикографический над 32 байтами (`CW:utils/src/ordered.rs, :290-292`; `CW:ed25519/scheme.rs`; сводка — `.claude/COMMONWARE_INTERNALS.md`).
- Уверенность (REGISTER, 09-03): `[KNOWN]` — обе стороны и текст теста прочитаны.
- Ход по плану 09-04 (INDEX): Э2 / С — conformance-вектор порядка: узел генерирует отсортированный комитет, тест контракта сверяет
- Связано: R-111 (убыль популяции как следствие), R-112, R-114. Разбор — `history/DUPLICATES.md`, группа 2.
- **Статус 2026-09-09:** отложена — Э2.3 (группа 2; единственный оставшийся «MUST mirror» — `bls/src/scheme.rs`). _Источник:_ history/PLAN.md §2 Э2 2.3; history/E2-ABI.md §5

### R-119 · MODERATE (пересмотрено 09-04: было SERIOUS — дрейф между ветками не подтверждён; 09-09 — опровергнут) · Форма возврата `getEpochCommitteeWithStakes` продублирована, дрейф между ветками контракта задокументирован в самом узле, а пиняющий её тест кодирует своим же типом
- Механизм: форма возврата в селектор не входит, поэтому селекторный пин её не покрывает — это прямо сказано в комментарии узла . При расхождении арности каждое чтение комитета падает `AbiDecode`; у follower любая `Corruption`-ошибка чтения комитета = shutdown (R-033), у валидатора — отказ на пути границы эпохи.
- Якоря: узел): `staking-reader/src/reader.rs` — четыре массива `(addrs, keys, stakes, tombstoned)`; комментарий озаглавлен «KNOWN CONTRACT DRIFT» и перечисляет ветки, где обработчик заканчивается `write_returns(sdk, &(validators, keys, stakes))` — тремя. контракт): в этой выкладке — четыре (`consensus.rs`).
- Уверенность (REGISTER, 09-03): `[KNOWN]` — комментарий, тест и текущая сторона контракта прочитаны. Состояние названных веток в этой сессии не проверялось — цитируется комментарий узла.
- Ход по плану 09-04 (INDEX): Э2 / С — форма возврата из общего ABI; тяжесть → MODERATE (дрейф между ветками не подтверждён, см. PLAN §8)
- Связано: R-033, R-114. Разбор — `history/DUPLICATES.md`, группа 4.
- **Статус 2026-09-09:** закрыта — `065003ad` + `0972059d` (2.1 и контр-ревью: тест внесён `0972059d`). Общий `sol!` форму возврата НЕ закрывает (контракт кодирует ответ своим кодеком по Rust-кортежу); держит тест `the_view_returns_decode_under_the_node_s_declaration` (проверен мутацией). Дрейф между ветками не существовал — ветка слита `f16fdd90`. _Источник:_ history/E2-ABI.md §1 гр.4, §10 п.1

### R-005 · MINOR (пересмотрено 09-04: было MODERATE в REGISTER, BLOCKER в AUDIT; VERIFY-BLOCKERS предлагал NIT) · PoP ключей комитета узлом не проверяется; защита от rogue-key целиком на контракте
- Механизм: если контракт не проверяет PoP, участник регистрирует `pk' = pk_x − Σ pk_i`; агрегат кворума схлопывается в его ключ, и он один собирает сертификат.
- Последствие: подделка сертификата одним участником ⇒ финализация расходящихся блоков.
- Якоря: `bls/src/scheme.rs` — комментарий-обещание «PoP verified on-chain at `Staking.setConsensusKeys`»;`staking-reader/src/reader.rs ` (`decode_consensus_keys`: только `BlsPubkey::decode`, то есть subgroup-check). `verify_pop` вызывается только из тестов `bls/tests/*` (grep по `crates/`). Ссылка AUDIT `bls/src/scheme.rs` неверна: файл 153 строки, продакшн до.
- Уверенность (REGISTER, 09-03): `[KNOWN]` узел не проверяет; `[KNOWN]` контракт проверяет.
- Ход по плану 09-04 (INDEX): — / — — узел PoP не проверяет осознанно; остаток — K-1/K-23 контракта (Э1)
- Связано: R-034, R-035, R-011, R-012 (тот же класс — ВЕРА контракту).
- **Статус 2026-09-09:** закрыта — `f70ceffe` (1.10: верификатор инлайн, `setBlsVerifier` удалён; K-1 закрыт). Узел PoP не проверяет осознанно (INDEX: «—»). Остаток: предеплои `0x02/0x05/0x0b/0x0f/0x10` обновляемы через `runtime-upgrade` (R10.1). _Источник:_ history/PLAN.md §2 Э1 1.10; history/E1-REFLECTION.md R10.1

### R-011 · MODERATE · Геометрия эпох заморожена на первом чтении; изменение `epochBlockInterval`/`dposActivationBlock` в контракте расколет сеть между перезапущенными и работающими узлами
- Механизм: governance меняет interval; стартовавшие после узлы считают эпохи по-новому, работающие — по-старому; `OriginEpocher`, `is_epoch_boundary`, партиции `consensus_epoch_{E}` и сабканалы расходятся.
- Последствие: две группы валидаторов с разными эпохами ⇒ нет кворума либо два кворума при n ≥ 8.
- Якоря: `staking-reader/src/epoch_transition.rs` (`freeze_or_warn`: расхождение только warn); `dpos.rs` (валидатор читает при `cs_finalized_hash` на старте).
- Уверенность (REGISTER, 09-03): `[KNOWN]` узел; `[KNOWN]` контракт.
- Ход по плану 09-04 (INDEX): Э2 / С — B-9: геометрия в chainspec; сеттеры контракта удаляются, genesis-bootstrap берёт из chainspec
- Связано: R-024 (константы не согласованы с интервалом), R-019.
- **Статус 2026-09-09:** отложена — Э2.4 (B-9/П-10: геометрия в chainspec); механизм после 09-04 не пересматривался. _Источник:_ history/PLAN.md §2 Э2 2.4

### R-015 · MODERATE · Цикл повторного применения в `try_derive` не ограничен, если EL не канонизирует производный блок
- Механизм: reth принимает payload, FCU отвечает `Valid`, но канонической на h остаётся другой блок. Цикл крутится внутри `try_derive`: mailbox не читается, Tip/SpecNotarized не обрабатываются, heartbeat не шлётся, латч halt не проверяется.
- Последствие: тихая остановка исполнения (виден только gauge `FinalizeApply`).
- Якоря: `executor.rs` (`while spec_executed_hash(h) != Some(derived)`: derive + import + FCU каждые 200 мс; счётчики только на `ParentHeaderMissing`/`PrefixSeedMissing`; при `fcu.is_valid` и неизменном `block_hash` — без выхода).
- Уверенность (REGISTER, 09-03): `[KNOWN]` структура цикла.
- Ход по плану 09-04 (INDEX): Э6 / С — П-6: все `while` с `await` ограничены, выход в `Deferred` с re-poke
- Связано: R-006 (тот же блок кода — править одной правкой), R-031, AUDIT B-8.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э6 6.1 (П-6: все `while` с `await` ограничены). Ex-5: цикл не наблюдался. _Источник:_ history/PLAN.md §2 Э6; history/EXPERIMENTS.md Ex-5

### R-016 · MODERATE · Порог re-jump для валидатора равен `min(1024, interval)`: при малом интервале короткая задержка исполнения запускает прыжок с поверхностью R-001
- Механизм: `interval = 32`, узел отстал на 33 блока (например, hold R-007 30 с) ⇒ спавнится re-jump.
- Последствие: лишние прыжки, каждый — вход в R-001/R-004.
- Якоря: `dpos.rs` (`JUMP_THRESHOLD.min(interval)`), `executor.rs `.
- Уверенность (REGISTER, 09-03): `[KNOWN]`.
- Ход по плану 09-04 (INDEX): Э5 / С+ — после П-2 (удержание больше не зависит от re-jump) порог сделать абсолютным
- Связано: R-001, R-004, R-007 (поднимать порог можно только после того, как σ-hold перестанет зависеть от re-jump — конфликт, см. часть 2).
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э5 5.3 после П-2 (порог re-jump абсолютный). _Источник:_ history/PLAN.md §2 Э5

### R-018 · MODERATE · Отложенный спавн движка (`deferred_spawns`) ждёт σ терминального раунда, но пробуждение зависит только от executor
- Механизм: терминальный σ приходит в `SeedStore` (например, `capture_certificate_seed` при backfill), но executor стоит (R-007/R-015) ⇒ `spawn_unblocked` не срабатывает ⇒ движок эпохи не поднимается, хотя все входы есть.
- Последствие: валидатор пропускает эпоху.
- Якоря: `epoch_manager.rs` (`Missing(TerminalSeed)` ⇒ `deferred_spawns`), (`select!` без ветки на `seed_edge` — grep `seed_edge` по файлу пуст), пробуждение через `spawn_unblocked` (`executor.rs`), share/key edge, границу.
- Уверенность (REGISTER, 09-03): `[KNOWN]`.
- Ход по плану 09-04 (INDEX): Э5 / С+ — П-2 + ребро «финализация легла в marshal → epoch_manager» (добавить при П-2)
- Связано: R-007, R-045; правка — `seed_edge` в `select!` при непустом `deferred_spawns`.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э5 5.3 смягчает (ребро «финализация легла в marshal → epoch_manager» добавить при П-2; VERIFY: ни одно П его не даёт). _Источник:_ history/PLAN.md §2 Э5; history/VERIFY-REDESIGN.md ч.1

### R-020 · MODERATE · Журналы ключей и сидов пишутся write-behind; валидатор без upstream после kill -9 может не подняться
- Механизм: блок финализирован, σ в памяти и в канале журнала, процесс убит до `sync`; если архив финализаций marshal тоже не успел, σ негде взять ⇒ без upstream — фатальный старт с требованием ресинка EL.
- Последствие: узел не стартует после сбоя без внешнего источника.
- Якоря: `beacon/key_journal.rs` (батч ≤ 64, `sync` после батча), `beacon/seed_journal.rs` (батч ≤ 256), `beacon/certify.rs` (память авторитетна); восстановление `dpos.rs` (`recover_replay_seed`: store → локальный сертификат → upstream → `Unavailable`), (без upstream ⇒ `Err`).
- Уверенность (REGISTER, 09-03): `[KNOWN]` порядок записи.
- Ход по плану 09-04 (INDEX): Э5 / С — П-2 (σ — в архиве marshal, `sync_finalized`) + П-3 с синхронной записью `ArtifactStore`
- Связано: R-021, R-017, R-050; закрывается AUDIT B-1/B-7.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э5 5.1/5.3 (синхронный `ArtifactStore::insert`, σ из архива marshal). Ex-11: 0 из 20 SIGKILL, архив marshal всегда впереди указателя исполнения. _Источник:_ history/EXPERIMENTS.md Ex-11; history/PLAN.md §2 Э5
- **Статус 2026-09-13 (Э5 5.1, `129f2754` + `fca59634`):** **закрыта НАПОЛОВИНУ, и это названо.** Ключевая половина ушла: `key_journal.rs` удалён, write-behind по ключам больше нет. СИД-половина остаётся — `seed_journal.rs` и его write-behind живут до строки 5.2. Артефактный стор пишется RAM-first с повтором, и повтор триггерится следующей записью, а не часами (`5.1д-Д-1`, цена названа). _Источник:_ history/E5-1-A.md

### R-021 · MODERATE · Ошибка записи share-файла не останавливает принятие share; после рестарта heal возможен только в окне одной эпохи
- Последствие: валидатор с ошибкой диска подписывает до рестарта, после — verify-only без сообщения о причине, кроме старого warn.
- Якоря: `beacon/actor.rs` (warn и продолжение), (`lo = now − 1`), `beacon/mod.rs ` (`JOURNAL_RETENTION_EPOCHS = 1`).
- Уверенность (REGISTER, 09-03): `[KNOWN]`.
- Ход по плану 09-04 (INDEX): Э5 / П — ошибка persist ⇒ не принимать share (громкая демоция) — правка в `beacon/actor.rs` при П-3
- Связано: R-025, R-020, R-072.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э5 5.1 (ошибка persist ⇒ не принимать share). _Источник:_ history/PLAN.md §2 Э5
- **Статус 2026-09-13 (Э5 5.1, `129f2754` + `fca59634`):** **закрыта.** Ошибка записи share-файла теперь ОТКАЗ: доля не принимается, узел остаётся verify-only, повтор идёт существующим ребром `drive_recompute`, а не таймером; тест `beacon::actor::…::a_share_whose_persist_fails_is_refused_and_the_node_stays_verify_only` (`actor.rs:6715`). Вторая половина находки (heal только в окне одной эпохи) закрыта R-025. _Источник:_ history/E5-1-A.md

### R-022 · MODERATE · Слэшер: неудачная отправка транзакции не повторяется до рестарта; consumer запущен отвязанным; недекодируемая запись WAL ack'ается и теряется
- Последствие: одна ошибка RPC откладывает все слэши до перезапуска; паника consumer незаметна.
- Якоря: `slasher/actor.rs` (`Failed` ⇒ без ack, без повтора), (`_consumer_handle` отброшен), (decode Err ⇒ ack).
- Уверенность (REGISTER, 09-03): `[KNOWN]`.
- Ход по плану 09-04 (INDEX): Э6 / С+ — П-7 супервизирует consumer и повторяет tx; ack недекодируемой записи WAL — отдельно (`slasher/actor.rs`); при Д-4=B-4 WAL исчезает
- Связано: R-027, R-028, R-032.
- **Статус 2026-09-09:** отложена — Э6 6.2 (П-7) / зависит от Д-4: при КB-4 WAL и consumer исчезают целиком (1.2, открыта); механизм после 09-04 не пересматривался. _Источник:_ history/PLAN.md §2 Э1 1.2, Э6 6.2; history/DECISIONS.md Д-4

### R-023 · MODERATE · `Confirm` с произвольной эпохой от любого пира вызывает EVM-чтение комитета до проверки подписи
- Механизм: поток `Confirm` с произвольными эпохами: 1 сообщение → 1 staticcall; при 4096 пирах — тысячи чтений в секунду.
- Последствие: CPU; реалистично только от многих пиров.
- Якоря: `beacon/actor.rs` (`on_confirm` → `committee_for(confirm.target_epoch)` до `pool.record`), (`on_message` не проверяет членство `from`); квота BEACON 128/с (`p2p/src/constants.rs`).
- Уверенность (REGISTER, 09-03): `[KNOWN]`. Стоимость `committee_for` — предположение.
- Ход по плану 09-04 (INDEX): Э4 / С+ — П-5 отсекает не-членов; фильтр `target_epoch ∈ [now, now+2]` до `committee_for` — отдельно
- Связано: R-054, R-029, R-038; правка — фильтр `target_epoch ∈ [now, now+2]` и кэш по эпохе до чтения.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э4 4.3 смягчает (П-5), 4.4 (фильтр `target_epoch ∈ [now, now+2]` до `committee_for`). _Источник:_ history/PLAN.md §2 Э4
- **Статус 2026-09-12 (Э4 4.3 закрыта):** закрыта — `48412eb3` (4.3-А: `on_confirm` отбрасывает `target_epoch ∉ [now, now+2]` ДО `committee_for`, `now = epoch_of(last_height)`; окно применяется только после первого тика высоты — `last_height: Option<u64>`, Д-129; выше по потоку `GatedReceiver` роняет неотслеживаемого/tombstoned/реестрового отправителя до декода). _Источник:_ history/E4-3-A.md §0(2), часть В P-05

### R-024 · MODERATE · `DKG_MARGIN_BLOCKS = 20` — константа, не согласованная с интервалом эпохи
- Последствие: при малом интервале окно деалинга нулевое, при чуть большем — окно в секунды (формулировка «ни одна церемония не завершится» опровергнута Ex-14 — см. статус).
- Якоря: `beacon/actor.rs`, (`saturating_sub`: при `interval ≤ 20` окно деалинга нулевое).
- Уверенность (REGISTER, 09-03): `[KNOWN]`.
- Ход по плану 09-04 (INDEX): Э2 / С — B-9: `DKG_MARGIN` как доля интервала в chainspec; проверка при старте
- Связано: R-011, AUDIT B-9, R-091 (комментарий говорит «10»).
- **Статус 2026-09-09:** отложена — Э2.4 (`DKG_MARGIN` как доля интервала в chainspec). Ex-14: нулевое окно даёт мгновенное закрытие печати, не отказ — формулировку править («сколько дилингов успеет — вопрос n и RTT», Ex-12). _Источник:_ history/EXPERIMENTS.md Ex-14; history/PLAN.md §2 Э2 2.4

### R-025 · MODERATE · Окно recompute-heal = 1 эпоха; журнал удаляется на первом тике следующей
- Последствие: узел, простоявший больше одной эпохи, теряет право восстановить share даже при наличии всех логов у пиров.
- Якоря: `beacon/mod.rs`, `beacon/share_state.rs`, `beacon/actor.rs`.
- Уверенность (REGISTER, 09-03): `[KNOWN]`.
- Ход по плану 09-04 (INDEX): Э5 / С — П-3: журналы DKG хранятся `SCHEME_RETENTION_EPOCHS`
- Связано: R-021, R-017, R-002, R-026; правка — хранить журналы `SCHEME_RETENTION_EPOCHS` эпох.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э5 5.1 (журналы DKG хранятся `SCHEME_RETENTION_EPOCHS`). _Источник:_ history/PLAN.md §2 Э5
- **Статус 2026-09-13 (Э5 5.1, `129f2754` + `fca59634`):** **закрыта.** `JOURNAL_RETENTION_EPOCHS` стал алиасом `SCHEME_RETENTION_EPOCHS = 8` (`beacon/mod.rs:126`, `lib.rs:37` — открыл сам): окно recompute-heal 1 → 8 эпох. Цена — ≈ +3 МБ диска на узел (наборы QUAL при n=51 плюс восемь эпох секретных вью вместо одной); направление безопасно по доку самой константы («widening only trades disk for heal reach»). _Источник:_ history/E5-1-A.md

### R-026 · MODERATE · Потеря тела согласованного предложения: инстанс агрирования не перезапускается, узел входит в эпоху без share; heal только через pull после начала эпохи
- Механизм: узел парковал `verify` (тела нет), сертификат пришёл по сети, `bodies.subscribe` не дождался за 45 с (решённое тело никто не ретранслирует).
- Последствие: узел пропускает начало эпохи как verify-only, затем heal через pull + `finalize_over_pinned`; если он единственный держатель тела — ключ эпохи потерян для всех (A-33).
- Якоря: `beacon/dkg_engine.rs` (`resolve_artifact == None` ⇒ `dkg_agree_body_lost`, инстанс завершается без артефакта; комментарий «has to re-agree on a fresh instance» ложен), (`started` содержит target, повтор игнорируется); `beacon/actor.rs` (актор повторяет только announce); pull артефакта стартует из `drive_recompute` для `e ∈ [now−1, now]`.
- Уверенность (REGISTER, 09-03): `[KNOWN]dkg_engine.rs`; путь heal — `[LIKELY]` по BEACON.
- Ход по плану 09-04 (INDEX): Э5 / С — П-9: потеря тела ⇒ немедленный `pull_artifact`
- Связано: R-025, R-002, R-037; правка — `pull_artifact` сразу при потере тела.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э5 5.2 (П-9: потеря тела ⇒ немедленный `pull_artifact`). _Источник:_ history/PLAN.md §2 Э5

### R-027 · MODERATE · Слэшер: пустой комитет при чтении = `Permanent` drop улики, хотя комитет может быть закоммичен позже
- Механизм: EL отстаёт на эпоху (обычно при догоне), улика текущей эпохи приходит от движка ⇒ снапшот пуст ⇒ улика уничтожена.
- Якоря: `slasher/actor.rs` (пустой снапшот ⇒ `Permanent`), чтение при EL-finalized хэше.
- Уверенность (REGISTER, 09-03): `[KNOWN]`.
- Ход по плану 09-04 (INDEX): Э4 / С — П-1: пустой ответ = `NotYetCommitted`, всегда retry
- Связано: R-022, R-014, R-066; правка — `Transient`.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э4 4.1 (П-1: пустой ответ = `NotYetCommitted`, всегда retry). _Источник:_ history/PLAN.md §2 Э4
- **Статус 2026-09-11:** закрыта — `0f9be693` + `65a23db7` (Э4 4.1: модуль `committee/` отвечает `NotReadable`/транзиентным `Read` на ещё не закоммиченную эпоху; слэшер `resolve_committee` маппит их в `Transient`, `OutOfWindow`-ниже/постоянный `Read` — в `Permanent`; юниты `tests/slasher_integration.rs` `an_epoch_above_the_window_is_retried_until_the_anchor_reaches_it`, `a_torn_anchor_probe_costs_the_evidence_a_retry_not_its_life`; стенд `a_node_below_the_chain_refuses_what_it_cannot_see_without_an_evm_call` пинует транзиентность отказа без слэшера в цепочке). Остаток: голова очереди слэшера блокируется на повторе (B1-03) — Э7. _Источник:_ history/E4-1-B1.md, E4-1-B3.md

### R-029 · MODERATE · Все блокировщики — `NoopBlocker`; единственная защита — квоты на пира
- Последствие: злонамеренный пир из реестра шлёт невалидные голоса/тела/запросы бесконечно в пределах квоты; при 4096 пирах суммарная нагрузка ограничена только числом соединений.
- Якоря: `p2p/src/lib.rs`; использование `dpos.rs`, `beacon/plane.rs`, `beacon/dkg_engine.rs`.
- Уверенность (REGISTER, 09-03): `[KNOWN]`.
- Ход по плану 09-04 (INDEX): Э4 / С — П-5: собственный бан по каналу на типе `Ingress`;`NoopBlocker` остаётся
- Связано: R-013, R-014, R-023, R-037, R-054, R-059, R-060.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э4 4.3 (П-5: собственный бан по каналу; `NoopBlocker` остаётся — PLAN §8 п.1). _Источник:_ history/PLAN.md §2 Э4, §8
- **Статус 2026-09-12 (Э4 4.3 закрыта):** смягчена — `48412eb3` (4.3-А: на BEACON/EVIDENCE — `Ingress` по зарегистрированному peer-set без штрафа и без счётчика на пира (П-5), метрика `dpos_ingress_dropped_total{channel, reason}`; на резолверных каналах `deliver == false` уже исключает пира — 4.2-А `7cac7f3b`). `NoopBlocker` остаётся сознательно — PLAN §8 п.1; глобальный бан — только тумбстоун. _Источник:_ history/E4-3-A.md §0(2)

### R-030 · MODERATE · Классификация транзиентных ошибок reth по подстрокам
- Последствие: смена текста в reth ⇒ транзиент классифицирован как `Backend` ⇒ `Corruption` ⇒ follower падает (R-033).
- Якоря: `staking-reader/src/reader.rs` (`display.contains(TORN_RANGE_DISPLAY | SHORT_READ_DISPLAY | SNAPSHOT_DISPLAY)`), `staking-reader/src/error.rs `.
- Уверенность (REGISTER, 09-03): `[KNOWN]`.
- Ход по плану 09-04 (INDEX): Э8 / П — B-11: типизированные транзиентные ошибки в форке reth
- Связано: R-033; закрывается AUDIT B-11.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э8 (B-11: типизированные ошибки в форке reth). _Источник:_ history/PLAN.md §2 Э8

### R-031 · MODERATE · Executor — один актор; сетевая проба и бесконечные циклы транспорта выполняются inline в его цикле
- Последствие: пока пиры не отвечают или reth недоступен — Tip не обрабатывается, heartbeat не шлётся, ack'и marshal не отдаются (окно 16 заполняется), латч halt не проверяется; ordering-плоскость уходит вперёд до потолка 2 эпох, затем узел выпадает из комитета без явного сигнала.
- Якоря: `executor.rs` (`probe_frontier` awaited в теле `select!`; на plane-пути это `fetch_one` с таймаутом 8 с, `plane_upstream.rs, `; при быстрой каденции 200 мс — до 8 с из каждых 8, 2 с), `executor.rs` [ссылка уточнена, history/REFS.md] (`fcu_retrying_transport`: бесконечный retry по 200 мс внутри обработчика), `application.rs` (`derive_with_visibility_retry`, 10 с), R-015.
- Уверенность (REGISTER, 09-03): `[KNOWN]`.
- Ход по плану 09-04 (INDEX): Э6 / С — П-6: проба и `sync_to` в spawn; циклы ограничены
- Связано: R-004 (проба — её вход), R-015, R-061; закрывается AUDIT B-8, CORE CB-12/CB-13.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э6 6.1 (П-6: проба и `sync_to` в spawn). _Источник:_ history/PLAN.md §2 Э6

### R-032 · MODERATE · Выход любой подсистемы (включая slasher) валит весь `OuterEngine`
- Последствие: ошибка в некритичной подсистеме останавливает консенсус узла.
- Якоря: `outer.rs` (`select!` по пяти handle'ам; `AbortAll`, если латч halt не взведён).
- Уверенность (REGISTER, 09-03): `[KNOWN]`.
- Ход по плану 09-04 (INDEX): Э6 / С — П-7: таблица супервизии по подсистемам
- Связано: R-022, R-078.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э6 6.2 (П-7). _Источник:_ history/PLAN.md §2 Э6

### R-033 · MODERATE · Follower: любая `Corruption`-ошибка чтения комитета (revert контракта, AbiDecode) = shutdown
- Якоря: `cert_inlet.rs` (только три варианта ⇒ `Defer`, остальное `Corruption`), (`Err(e) ⇒ return Err`), `dpos.rs ` (break ⇒ `shutdown.cancel`).
- Уверенность (REGISTER, 09-03): `[KNOWN]`.
- Ход по плану 09-04 (INDEX): Э4 / С+ — П-1 единая классификация ошибок; П-7 перезапуск inlet вместо shutdown
- Связано: R-030, R-042, R-063.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э4 4.1 смягчает (П-1 единая классификация), Э6 6.2 (перезапуск inlet вместо shutdown). _Источник:_ history/PLAN.md §2 Э4, Э6
- **Статус 2026-09-11:** смягчена — `65a23db7` (Э4 4.1 Б2: `CertInlet::ingest` без фатального исхода — комитетных чтений inlet не делает, `Committee::scheme` → `Option`; постоянный класс (ревёрт/`AbiDecode`) различает, считает `dpos_committee_read_permanent_total{reason}` и логирует один раз модуль (`store.rs::failed`), узел не падает — Д-34). Остаток: `enter_finalized_epoch` follower'а и другие `Corruption`-пути вне inlet'а — Э6 6.2. _Источник:_ history/E4-1-B2.md Д-34, E4-1-B2-REVIEW.md B2-04

### R-036 · MODERATE · `maybe_start` при `NoFile` после дедлайна печати переизлагает и переподписывает второй лог — честная эквивокация логов, включающая R-002
- Механизм: потеря/восстановление `share_dir` из бэкапа после печати; узлы, уже записавшие L1, второй Reveal игнорируют (first-wins), опоздавшие записывают L2 ⇒ раскол как в R-002 без злого умысла.
- Якоря: `beacon/actor.rs` (`NoFile ⇒ start_fresh` без проверки `last_height` против дедлайна; проверка есть только для `Present`), (`seal_dealings` на следующем тике с тем же детерминированным полиномом, `ceremony.rs`, но другим набором ack/reveal ⇒ другой `SignedDealerLog`). Комментарий `share_state.rs` («re-dealing fresh would draw new OsRng randomness») устарел.
- Уверенность (REGISTER, 09-03): `[KNOWN]` ветка `NoFile`;`seal_dealings` — `[KNOWN]`.
- Ход по плану 09-04 (INDEX): Э5 / С — П-9: `NoFile` после дедлайна ⇒ не стартовать
- Связано: R-002, R-072.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э5 5.2 (П-9: `NoFile` после дедлайна ⇒ не стартовать). _Источник:_ history/PLAN.md §2 Э5

### R-037 · MODERATE · Body-engine агрирования буферизует до 51 тела на каждого отправителя из `latest.primary`, а не только от членов комитета
- Последствие: 4096 пиров × 51 × 154 KiB ≈ 30 GiB в худшем случае; окно — время жизни агрирования.
- Якоря: `beacon/dkg_transport.rs` (`deque_size = MAX_COMMITTEE_SIZE`, кодек `()`); тело `DkgProposal` ≤ ~154 KiB (`beacon/artifact.rs`, по BEACON); `CW:broadcast/src/buffered/engine.rs` (по BEACON).
- Уверенность (REGISTER, 09-03): `[KNOWN]deque_size`; размер тела и семантика `primary` — `[LIKELY]` (Ex-8).
- Ход по плану 09-04 (INDEX): Э4 / С+ — П-5 + `deque_size = 1..2` в `dkg_transport.rs`
- Связано: R-013, R-029; правка — `deque_size = 1..2`.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э4 4.3 смягчает, 4.4 (`deque_size = 1..2` в `dkg_transport.rs`). _Источник:_ history/PLAN.md §2 Э4
- **Статус 2026-09-12 (Э4 4.3 закрыта):** закрыта — `48412eb3` (4.3-А: `dkg_transport.rs` `deque_size: MAX_COMMITTEE_SIZE` → 2 по замеру Д5 (одно тело на (инстанс, отправитель) + запас на перепредложение); `latest.primary` — теперь три записи комитетов, не реестр). Остаток (P-16, реляция R10): перепредложение после nullify в замере не наблюдалось; юниты согласования гоняются на тест-хелпере с `MAX_SET_LEN`. _Источник:_ history/E4-3-A.md §0(4)

### R-038 · MODERATE · Крипто-тяжёлая работа в цикле DKG-актора повторяется каждый тик без backoff
- Последствие: при n=51 порядка 35×51 ed25519-проверок + MSM за вызов; актор однопоточен.
- Якоря: `beacon/actor.rs` (`pinned_ready` для каждой отложенной церемонии на каждый `on_height`/`Reveal`/`Deliver`;`Logs` создаётся заново, `ceremony.rs`), `beacon/actor.rs` [ссылка исправлена, history/REFS.md] (`try_recompute_pending` грузит журнал и делает `Player::resume` + `finalize` каждый тик, пока `validate_share_on_poly` ложен — при R-002 навсегда до sweep), `beacon/actor.rs` [ссылка исправлена, history/REFS.md] (`derive_pinned` на каждый `verify`/`propose`).
- Уверенность (REGISTER, 09-03): `[LIKELY]` по BEACON (`beacon/actor.rs` открыт частично).
- Ход по плану 09-04 (INDEX): Э5 / С — П-9: событийный драйвер, `pinned_ready` по событию с кэшем
- Связано: R-002, R-023, R-067.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э5 5.2 (П-9: событийный драйвер). _Источник:_ history/PLAN.md §2 Э5

### R-039 · MODERATE · На live-пути `adopt_share` share не проверяется против полинома; проверка есть только при recompute
- Механизм: dealer прислал dealing (A, share_A), получил ack, запечатал лог с другим `pub_msg` B, где слот узла — `Reveal`; лог валиден, share берётся из `view` ⇒ вне полинома.
- Последствие: узел производит невалидные partial'ы, его голоса отбрасываются (`combined_scheme.rs`); при t == quorum один такой член делает nullify-кворум недостижимым.
- Якоря: `beacon/actor.rs` (`finalize_over_pinned` → `adopt_share` без `validate_share_on_poly`) vs (гейт на recompute). Предпосылка view-first в `Player::finalize` подтверждена AUDIT по `CW:cryptography/src/bls12381/dkg.rs`.
- Уверенность (REGISTER, 09-03): `[KNOWN]` отсутствие проверки на live-пути; view-first — `[LIKELY]` по AUDIT.
- Ход по плану 09-04 (INDEX): Э5 / С — П-3/П-9: `validate_share_on_poly` перед любым `adopt_share`
- Связано: R-002 (одна правка: проверять share при любом adopt).
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э5 5.1/5.2 (`validate_share_on_poly` перед любым `adopt_share`). _Источник:_ history/PLAN.md §2 Э5
- **Статус 2026-09-13 (Э5 5.1, `129f2754` + `fca59634`):** **закрыта.** `validate_share_on_poly` — ПЕРВЫЙ оператор `adopt_share` (`beacon/actor.rs:1055`, открыл сам), а `committee` — обязательный параметр (`:1051`), поэтому третий путь принятия доли нельзя написать, не передав комитет. Гейт стал свойством функции, а не её вызывающих. _Источник:_ history/E5-1-A.md

### R-040 · MODERATE · By-height fetch'и не связывают эпоху раунда с высотой; сертификат старого комитета за любой высотой попадает в архив
- Механизм: ключи `committee[E′]` старой эпохи скомпрометированы; upstream отвечает на `Finalized{last(E−1)}` блоком нужной высоты с сертификатом `round.epoch = E′`;`boundary_lookup` берёт из него `proposal_view` (`epoch_manager.rs`) ⇒ иная база лидера или отложенный спавн.
- Якоря: `cert_follow.rs` (только `block.height == height`), `cold_start_jump.rs ` (эпоха из `round` сертификата), `dpos.rs` (`refetch_verified_archive_hole` — то же); результат уходит в marshal через `verified` + `report(Finalization)` без проверки высота↔эпоха (`CW:marshal/core/actor.rs`, по CORE). Follower-инлет имеет `epoch_bind` (`dpos.rs`), эти три пути — нет.
- Уверенность (REGISTER, 09-03): `[KNOWN]` три места; marshal — `[LIKELY]`.
- Ход по плану 09-04 (INDEX): Э4 / С — П-1/П-4: одна проверка `epoch_of(height) == round.epoch`
- Связано: R-001 (та же проверка нужна и там), R-009.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э4 4.1/4.2 (одна проверка `epoch_of(height) == round.epoch`). _Источник:_ history/PLAN.md §2 Э4
- **Статус 2026-09-11:** смягчена — `65a23db7` (Э4 4.1: схема эпохи — из записи ТОЙ ЖЕ эпохи в одной карте, `scoped(E)` не может отдать схему чужого комитета за высоту); вторая половина — проверка `epoch_of(height) == round.epoch` в `deliver` — Э4 4.2. _Источник:_ history/E4-1-B2.md
- **Статус 2026-09-12 (Э4 4.2 закрыта):** закрыта — вторая половина `7cac7f3b` (4.2-А: `epoch_of(height) == round.epoch` в `deliver`). _Источник:_ history/E4-2-A.md

### R-041 · MODERATE · Партиции `consensus_epoch_{E}` никогда не удаляются
- Последствие: одна партиция на эпоху за весь срок процесса; при devnet-интервале 32 — тысячи.
- Якоря: `engine.rs` (партиция на эпоху), `epoch_manager.rs` (`prune_agreements` подметает только `dkg_epoch_*`); других `remove` для ordering-партиций нет (grep CORE).
- Уверенность (REGISTER, 09-03): `[KNOWN]prune_agreements`,`engine.rs `.
- Ход по плану 09-04 (INDEX): Э7 / П — sweep `consensus_epoch_{e}` в окне `AGREEMENT_SWEEP_SPAN` (`epoch_manager.rs`)
- Связано: R-025 (единая политика retention).
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э7 (sweep `consensus_epoch_{e}` в окне `AGREEMENT_SWEEP_SPAN`); после Э0.4 (атрибут `epoch`) больше не задаёт долю потерь метрик. _Источник:_ history/PLAN.md §2 Э7; history/E0-LOG.md 0.4

### R-042 · MODERATE · Follower: `sync_to` при известной геометрии фатален на любой `SyncFailure`
- Последствие: перезапуск-шторм follower'ов при проблемах devp2p.
- Якоря: `dpos.rs`, (`el.sync_to(&latest).await?`) — 90 с без devp2p-пиров или 300 с застоя валят `launch_follower`; ниже тот же прыжок обёрнут в `cold_start_jump_self_heal` с вечным повтором.
- Уверенность (REGISTER, 09-03): `[KNOWN]`.
- Ход по плану 09-04 (INDEX): Э6 / С — B-2: один лончер, `sync_to` под self-heal и у follower
- Связано: R-033, R-064; закрывается AUDIT B-2.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э6 6.3 (B-2: один лончер; зависит от Д-5). _Источник:_ history/PLAN.md §2 Э6
- **Статус 2026-09-12:** якорь устарел — `cold_start_jump_self_heal` удалён `dacd1bfa` (4.2-Б2); follower: datadir-арм входит без `sync_to`, свежий datadir — `sync_to_checkpoint` только по операторскому checkpoint, devnet-`get_latest ⇒ sync_to` за `is_deployed_network`. Класс «`SyncFailure` фатален» остаётся — Э6 6.3. _Источник:_ history/E4-2-B2.md

### R-101 · MODERATE · Узел морозит геометрию эпох в том самом окне, где контракт ещё разрешает её менять
- Механизм: governance планирует активацию на `H`; узел на первом же finalized-блоке видит `Some(H)`, проходит гейт и морозит пару `(I, H)` — это происходит до блока `H`. Governance переносит запуск (`I'`, `H'`) — сеттеры ещё открыты, потому что `block_number < H`. Работавшие узлы держат `(I, H)` и печатают один `warn!`; стартовавшие после морозят `(I', H')`; контракт и pre-execution обоих считают по `(I', H')`.
- Последствие: consensus-плоскость расходится и между узлами, и со своим же EL. `leader_index` в `extra_data` считается против комитета `E_frozen`, а `recordProduction` кредитует `produced[E_live][leader_index]` и валидирует индекс против `committee_length_at(E_live)` (`liveness.rs, 76-99`); `OriginEpocher`, `is_epoch_boundary`, партиции `consensus_epoch_{E}` и сабканалы расходятся.
- Якоря: узла: `staking-reader/src/epoch_transition.rs` (гейт `scheduled_dpos_activation`), и (`freeze_or_warn` для интервала и активации), (позднейшее расхождение — только `warn!`); живое, незамороженное чтение на стороне исполнителя — `node/evm.rs` (интервал на каждом блоке) и `node/evm.rs` (`current_epoch`). контракта: `config.rs` (`ensure_dpos_not_active` запрещает правку только при `activation != 0 && block_number >= activation`), (`setEpochBlockInterval`), (`setDposActivationBlock`); `util.rs` и `math.rs` — эпоха контракта считается по живым значениям.
- Уверенность (REGISTER, 09-03): `[KNOWN]` обе стороны.
- Ход по плану 09-04 (INDEX): Э2 / С — B-9: геометрия из chainspec, менять нечего
- Связано: R-011 (несёт её достижимый остаток), R-102, AUDIT B-9.
- **Статус 2026-09-09:** отложена — Э2.4 (B-9: геометрия из chainspec, менять нечего); механизм после 09-04 не пересматривался. _Источник:_ history/PLAN.md §2 Э2 2.4

### R-113 · MODERATE · Минимум размера комитета утверждается на ВЫХОДЕ останавливающего цепь системного вызова, а на ВХОДЕ проверяется только по пути потолка; путь популяции не проверяется нигде
- Механизм: константа документирует собственное допущение и признаёт, что закрывает только одну дыру — `consts.rs`: «asserts that assumption rather than testing a condition the contract expects to meet … The **one way to break the assumption by configuration rather than by circumstance** is a committee cap below this floor». Путь «по обстоятельствам» назван и оставлен открытым, хотя в том же файле есть три перехода состояния, которые им и являются.
- Последствие: не самостоятельное — это общая причина R-111 и R-112. Заведена отдельно, потому что это пропущенный инвариант, а не дефект одного вызова: любая будущая причина уменьшить популяцию унаследует тот же исход.
- Якоря: единственные два места, где `MIN_COMMITTEE_LENGTH` вообще участвует в проверке (исчерпывающий `grep` по не-тестовым файлам): `consensus.rs` — ревёрт `commitEpochCommittee`, то есть УТВЕРЖДЕНИЕ допущения на выходе, останавливающее цепь; `config.rs` — гвард `setActiveValidatorsLength`, то есть проверка на входе, но только для потолка. Пять писателей штампа видимости (`grep "set_selection_visible"`:`consensus.rs `, `staking.rs ` плюс прямая сид-запись `staking.rs`) — floor-проверка есть ровно у одного, `staking.rs`, и та против потолка, а не против минимума.
- Уверенность (REGISTER, 09-03): `[KNOWN]` целиком. Перечни писателей штампа и мест проверки минимума получены исчерпывающим `grep` по не-тестовым файлам, а не по памяти; недостижимость `ERR_EPOCH_NOT_YET_COMMITTABLE` обоснована по коду обеих сторон (`EXPERIMENTS.md` §5.5.8). `[GUESS]` в записи не осталось.
- Ход по плану 09-04 (INDEX): Э0 / С — B-1 закрывает исход; общий крейт (Э2) связывает `MIN_COMMITTEE_LENGTH`, lookahead `2` и формулу `f`
- Связано: R-111, R-112.
- **Статус 2026-09-09:** смягчена — `9b6213be` (2.2: `MIN_COMMITTEE_LENGTH`, `fault_tolerance`, `MAX_COMMITTEE_LOOKAHEAD_EPOCHS` из `staking_protocol`; бюджет отказов сверен с commonware `N3f1::max_faults`; литерал `+2` в узле заменён константой). Пропущенный инвариант (floor-проверка у писателей штампа) остаётся осознанно — вместе с решением 1.0 «ниже пола — отказ». `ERR_EPOCH_NOT_YET_COMMITTABLE` — недостижим (EXPERIMENTS §5.5.8). _Источник:_ history/PLAN.md §2 Э2 2.2; history/EXPERIMENTS.md §5.5.8

### R-117 · MODERATE · Три числовых предела контракта скопированы в `staking-reader` с пометкой «MUST mirror» и ссылками, которые уже протухли
- Механизм: три величины продублированы, удерживаются только комментариями, и комментарии просят об этом прямым текстом — «MUST mirror the contract — drift mis-weights leaders», «Keep the two in step» (, где признано, что инвариант «был документирован на стороне Rust и обеспечивался только на стороне Solidity»). Уже протухшие номера строк в двух из трёх ссылок — прямое свидетельство, что механизма здесь нет.
- Последствие: `MIN_COMMITTEE_LENGTH` — узел считает легальным состояние, которое цепь отвергает (расхождение с реальным порогом R-111); `BALANCE_COMPACT_PRECISION` — веса лидеров считаются в других единицах, лотерея лидера смещена молча (форка нет, все узлы неправы одинаково); `MAX_COMPACT_STAKE` — ломается аргумент о невозможности переполнения префиксной суммы в `WeightedVrf::build`.
- Якоря: `staking-reader/src/reader.rs MIN_COMMITTEE_LENGTH = 4` (ссылается на `consts.rs`, фактически `consts.rs`; и на `consensus.rs`, фактически); `BALANCE_COMPACT_PRECISION = 10_000_000_000` (ссылается на `consts.rs`, фактически); `MAX_COMPACT_STAKE = 1 << 112` против типа хранения `StorageUint112` (`storage.rs`) и `math::U112` (`math.rs`).
- Уверенность (REGISTER, 09-03): `[KNOWN]`.
- Ход по плану 09-04 (INDEX): Э2 / С — `MIN_COMMITTEE_LENGTH`,`BALANCE_COMPACT_PRECISION`,`2^112` — из общего крейта констант
- Связано: R-111, R-113, R-057. Разбор — `history/DUPLICATES.md`, группы 5-7.
- **Статус 2026-09-09:** закрыта — `9b6213be` + `065003ad` (2.2: `MIN_COMMITTEE_LENGTH`, `BALANCE_COMPACT_PRECISION`, `COMPACT_STAKE_BITS`/`MAX_COMPACT_STAKE` в `staking_protocol`; `grep 'MUST mirror'` по `crates/` — одна строка про порядок комитета). _Источник:_ history/E2-ABI.md §1 гр.5–7

### R-118 · MODERATE · Потолок размера комитета `51` объявлен дважды под разными именами: кодек сертификатов узла и предел контракта
- Механизм: одна и та же величина под разными именами и без общего источника. Это ровно то число, которое захотят поднять при росте сети, и поднять его можно на одной стороне: если контрактный предел вырастет, комитет станет больше кодека и сертификаты перестанут декодироваться у всех.
- Якоря: `crates/dpos/p2p/src/constants.rs MAX_COMMITTEE_SIZE: u64 = 51` против `consts.rs MAX_ACTIVE_VALIDATORS_LENGTH: u64 = 51` (`setActiveValidatorsLength` отвергает значения выше, `config.rs`).
- Уверенность (REGISTER, 09-03): `[KNOWN]`.
- Ход по плану 09-04 (INDEX): Э2 / С — `MAX_COMMITTEE_SIZE`/`MAX_ACTIVE_VALIDATORS_LENGTH` — одна константа
- Связано: R-019 (следствие того же числа), R-013. Разбор — `history/DUPLICATES.md`, группа 8.
- **Статус 2026-09-09:** закрыта — `9b6213be` + `065003ad` (2.2: одно имя `MAX_COMMITTEE_SIZE`, контрактное `MAX_ACTIVE_VALIDATORS_LENGTH` удалено). _Источник:_ history/E2-ABI.md §1 гр.8

### R-012 · MINOR · Узел решает «комитет сменился» по равенству множеств peer-ключей; контракт ставит `dkgQual` по своему правилу; расхождение = эпоха без пригодного ключа
- Механизм: валидатор ротирует BLS-ключ при том же peer-ключе. Контракт ставит `dkgQual[E] = true`. Узлы видят `next == cur` и не запускают церемонию. `chain_key_epoch(E) = E`, `has_mint(E) = false` ⇒ `NoUsableMint` у всех ⇒ `Withheld` ⇒ ни один движок не спавнится.
- Последствие: остановка цепи.
- Якоря: `beacon/actor.rs` (`next == cur` на `Set<PeerPubkey>`), `node/dpos.rs ` (`committee_pair_for` строит ростеры только из `peer_pubkey`); потребители бита: `beacon/carry.rs` (`select_carry_scheme`: ключ в силе = `chain_key_epoch` по `dkgQual`), `resolve.rs `, `oracle.rs `.
- Уверенность (REGISTER, 09-03): `[KNOWN]` узловая логика; `[KNOWN]` правило контракта.
- Ход по плану 09-04 (INDEX): Э5 / С+ — П-3 сравнивает записи П-1; решение Д-7 — читать `getDkgQual` вместо вывода
- Связано: R-017, R-005, R-035.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э5 5.1 смягчает; решение Д-7 (читать `getDkgQual` вместо вывода) открыто. _Источник:_ history/PLAN.md §2 Э5, §5 Д-7
- **Статус 2026-09-13 (Э5 5.1, `129f2754` + `fca59634`):** **закрыта (Д-7).** Решение «комитет сменился» больше не сравнивает множества peer-ключей: `maybe_start` читает бит `changed` с якоря модуля комитета, а `committee_pair_for`/`CommitteePairFor` и страж `!(bit \|\| committed)` удалены вместе с `carry.rs`. Расхождение с правилом контракта как класс снято — бит один и он контрактный. _Источник:_ history/E5-1-A.md

### R-017 · MINOR · Дисковое правило `reconcile_journals` удаляет действующий mint при более новом «отклонённом» mint'е; RAM-правило его сохраняет; после двух рестартов — verify-only
- Механизм: store держит mint 3 (бит установлен) и mint 5 (бит сброшен). До рестарта резолвер отдаёт mint 3. Первый `on_height` после рестарта при `now ≥ 5` удаляет `beacon-share-e3.bin`; следующий рестарт грузит только mint 5 ⇒ `NoUsableMint`.
- Последствие: тихая демоция после двух рестартов; на всём комитете — ноль подписантов.
- Якоря: `beacon/share_state.rs` (удаляются share-файлы строго ниже `max{e ≤ now}`), `beacon/actor.rs ` (`ceremony_retain_floor`: RAM хранит всё `≥ max{k ≤ now − 8}`), `beacon/carry.rs ` (ключ в силе = последний `dkgQual`-бит, не максимальный mint); код сам держит `declined`-ветви, потому что доказательство опирается на контракт (`actor.rs`).
- Уверенность (REGISTER, 09-03): `[KNOWN]` оба floor'а.
- Ход по плану 09-04 (INDEX): Э5 / С — П-3: одно правило вытеснения
- Связано: R-012, R-025, R-020; правка — дисковый floor = `ceremony_retain_floor`, либо не удалять share-файлы вовсе.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э5 5.1 (П-3: одно правило вытеснения). _Источник:_ history/PLAN.md §2 Э5
- **Статус 2026-09-13 (Э5 5.1, `129f2754` + `fca59634`):** **смягчена.** RAM-правило исчезло вместе с `keys.rs` (стора ключей больше нет, сравнивать нечему); дисковое `reconcile_journals` живёт в `share_state.rs`, но теперь его окно — `SCHEME_RETENTION_EPOCHS = 8`, а не 1. Расхождение «диск против RAM» как класс снято, остаток — само дисковое правило. _Источник:_ history/E5-1-A.md

### R-019 · MINOR · `MAX_COMMITTEE_SIZE` проверяется один раз при старте; рост комитета после старта делает сертификаты недекодируемыми у всех
- Механизм: контракт коммитит комитет из 52 членов ⇒ битмапы > 51 отвергаются, узел молча перестаёт принимать сертификаты эпохи.
- Последствие: остановка сети без предупреждения от узла.
- Якоря: `dpos.rs` (проверка `activeValidatorsLength ≤ 51` только в `launch`); декодеры с cap: `plane_upstream.rs`, `cert_inlet.rs`, `beacon/artifact.rs `, `slasher/evidence.rs `; индексы > 255 — `IndexExceedsWireFormat` (`application.rs`).
- Уверенность (REGISTER, 09-03): `[KNOWN]` узел; `[KNOWN]` контракт.
- Ход по плану 09-04 (INDEX): Э4 / С — П-1 проверяет размер в конструкторе записи; число — из общего крейта (Э2)
- Связано: R-011; правка — проверять размер каждого снапшота в `epoch_committee_snapshot`.
- **Статус 2026-09-09:** смягчена — `9b6213be` (2.2: число 51 общее, `staking_protocol::MAX_COMMITTEE_SIZE`; R-118 закрыта); разовая проверка при старте осталась — по плану Э4 4.1 (П-1: размер в конструкторе записи). _Источник:_ history/PLAN.md §2 Э4; history/E2-ABI.md §1 гр.8
- **Статус 2026-09-11:** смягчена — `9b6213be` (как 09-09); в Э4 4.1 разовая проверка при старте (`consensus/dpos.rs:1983`) ОСТАЛАСЬ, размер в конструкторе записи модуля не проверяется (`grep MAX_COMMITTEE_SIZE committee/` — пусто) — П-1 в этой части не реализован; кандидат Э7 (проверка в `CommitteeRecord::new`). _Источник:_ history/E4-ORCHESTRATOR.md (закрытие 4.1)

### R-028 · MINOR · In-block equivocation charge не имеет контрактного обработчика; `next_charge` повторяет тот же charge в каждом блоке и блокирует остальных обвиняемых
- Последствие: верификация charge в `verify_block` и `extra_data` — мёртвая машинерия (все валидаторы платят BLS-проверку двух подписей за блок); наказание приходит только через WAL-транзакцию на смене эпохи; один обвиняемый с низким индексом занимает слот charge на все блоки.
- Якоря: `slasher/actor.rs` (удаление только при `tombstoned`), `application.rs `, (гейт в `verify_block`); контрагент `node/evm.rs` («The contract has no counterpart at all» для `slashEquivocation(uint64, uint32)`), (revert складывается в skip).
- Уверенность (REGISTER, 09-03): `[KNOWN]next_charge`;`[KNOWN]` контрактная сторона.
- Ход по плану 09-04 (INDEX): Э1 / С+ — Д-4: при B-4 in-block charge — единственный путь; `next_charge` снимать после включения в блок
- Связано: R-022, R-027, R-104. AUDIT B-3 (добавить контрактный обработчик) — отменён.
- **Статус 2026-09-09:** открыта в остатке — `next_charge` держит один заряд во всех блоках; первая половина («нет контрактного обработчика») снята 09-03 (К-6: обработчик есть, R-104). Остаток — работа 1.2 (Д-4 под вопросом). _Источник:_ history/REGISTER.md R-028 пересмотр; history/PLAN.md §2 Э1 1.2

### R-034 · MINOR · `tombstoned` — единственный источник; узел рвёт транспорт и отказывает лидеру по флагу без доказательства
- Последствие: честный валидатор изолирован сетью.
- Якоря: `application.rs` (отказ лидеру), `slasher/tombstone.rs`, `node/dpos.rs` (бан транспорта по EL-finalized).
- Уверенность (REGISTER, 09-03): `[LIKELY]` по AUDIT (`application.rs` открыт, остальное нет); `[KNOWN]` контрактная сторона.
- Ход по плану 09-04 (INDEX): Э4 / С+ — Д-1: форма записи комитета решает, читать ли `tombstoned` живьём
- Связано: R-005, R-044.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э4 4.1 смягчает; форма — открытое решение Д-1. _Источник:_ history/PLAN.md §2 Э4, §5 Д-1

### R-035 · MINOR · `getEpochCommitteeWithStakes` с пустыми `stakes` ⇒ движок не спавнится ни у кого
- Последствие: если контракт отдаёт пустые `stakes` раньше, чем узлы входят в эпоху, — остановка сети.
- Якоря: `staking-reader/src/reader.rs`, `weighted_vrf.rs` (`weights: None` ⇒ `WeightsUnavailable`), `epoch_manager.rs ` (`Err` ⇒ `false`, без повтора).
- Уверенность (REGISTER, 09-03): `[KNOWN]weighted_vrf.rs`,`epoch_manager.rs `; reader — `[KNOWN]`; контракт — `[KNOWN]`.
- Ход по плану 09-04 (INDEX): Э4 / С+ — П-1 якорь гарантирует веса; повтор спавна при `WeightsUnavailable` — отдельно (`epoch_manager.rs`)
- Связано: R-045 (нет повтора спавна), R-012.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э4 4.1 смягчает (якорь П-1 гарантирует веса), Э7 (повтор спавна при `WeightsUnavailable`). _Источник:_ history/PLAN.md §2 Э4, Э7
- **Статус 2026-09-11:** смягчена — `ef6e6265` + `65a23db7` (Э4 4.1: веса в записи обязательны, `weights: None` внутри окна ⇒ `Read(permanent)` с `error!` и метрикой `reason="weights_none"`, записи и схемы нет — стенд `a_weightless_committee_inside_the_window_is_refused_permanently_and_loudly` (`03c3151f`); `WeightedVrf` строится из записи ДО `upgrade_scheme`/`spawn_engine` — Д-30). Остаток: повтор спавна при `WeightsUnavailable` — Э7; постоянный отказ не мемоизируется (B3-12) — Э7. _Источник:_ history/E4-1-B3.md, E4-1-B2.md Д-30
- **Статус 2026-09-12 (перепроверка по коду, R-128):** остаток сжался до одного пункта и тот почти мёртв. (1) B3-12 закрыт для «невозможного» класса — `weights: None` в окне теперь отравляет слот (`26609cff`), а сама эпоха ОСТАНАВЛИВАЕТ узел через `SafetyHalt{contract_fork}`, то есть исход «движок не спавнится ни у кого молча» больше не существует: он стал громким и наблюдаемым. (2) Повтор спавна при `WeightsUnavailable` по-прежнему отсутствует — `epoch_manager.rs:1414`: `error!` + `soft_enter` + `return`, очереди повтора нет; но ветка достижима только через подменённую реализацию `EpochReads`: `WeightedVrf::try_new` строится из `CommitteeRecord::snapshot_view()`, где `weights` всегда `Some`, а длина сверена в `CommitteeStore::build`. _Источник:_ history/R-128-FIX.md §4

### R-043 · MINOR · `reseed_forward` рассинхронизирует in-memory finalized и forkchoice reth
- Последствие: reth получает `finalized = landing` до локального derive; сам по себе не форк
- Якоря: `executor.rs` (`update_finalized(landing)` в памяти, FCU шлёт `finalized = floor_hash`), heartbeat затем шлёт `finalized = landing`
- Уверенность (REGISTER, 09-03): `[KNOWN]`
- Ход по плану 09-04 (INDEX): Э6 / С — П-6: один `ElWriter`
- Связано: R-074; закрывается CORE CB-6.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э6 6.1 (П-6: один `ElWriter`). _Источник:_ history/PLAN.md §2 Э6

### R-044 · MINOR · Отказ голосовать за блок tombstoned-лидера зависит от момента чтения снапшота
- Последствие: только живучесть (пустые view лидера)
- Якоря: `application.rs`; `TombstoneSet::observe` из `node/dpos.rs` по EL-finalized
- Уверенность (REGISTER, 09-03): `[KNOWN]` `application.rs`
- Ход по плану 09-04 (INDEX): Э4 / С+ — как R-034 (Д-1)
- Связано: R-034. К-8 закрыт: флаг необратим и монотонен (`consensus.rs`), так что расхождение между узлами разрешается только в одну сторону; тяжесть не менялась.
- **Статус 2026-09-09:** не пересматривалась после 09-04; как R-034 (Д-1). _Источник:_ history/PLAN.md §2 Э4

### R-045 · MINOR · `EpochEngine::new` регистрирует схему до `WeightedVrf::try_new`; при `WeightsUnavailable` эпоха без движка и без повтора, а signer-схема исключает эпоху из repair-sweep
- Якоря: `engine.rs` vs ; `epoch_manager.rs` (`false` без записи в `deferred_spawns`); `outer.rs` (`verifier_epochs` исключает схемы с `me`); понижение signer→verifier запрещено (`outer.rs`)
- Уверенность (REGISTER, 09-03): `[KNOWN]`
- Ход по плану 09-04 (INDEX): Э4 / С+ — П-1 смягчает; elector до регистрации схемы — правка `engine.rs /258` отдельно
- Связано: R-035, R-018, R-075.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э4 4.1 смягчает; elector до регистрации схемы — Э7. _Источник:_ history/PLAN.md §2 Э4, Э7
- **Статус 2026-09-11:** смягчена — `65a23db7` (Э4 4.1 Б2: elector (`WeightedVrf`) строится ДО `Committee::upgrade_scheme` и ДО `spawn_engine`, отказ `try_new` не оставляет signer-схемы в карте — Д-30, ревью B2 §0.12 «поведение стало лучше»). Остаток: повтор при `WeightsUnavailable` — Э7. _Источник:_ history/E4-1-B2.md Д-30

### R-046 · MINOR · Single-slot `pending_boundary` в release молча теряет границу
- Якоря: `staking-reader/src/epoch_transition.rs` (`debug_assert!` + перезапись)
- Уверенность (REGISTER, 09-03): `[KNOWN]` по коду
- Ход по плану 09-04 (INDEX): Э7 / П — `BTreeSet<u64>` вместо `Option<u64>` или fail-closed вместо `debug_assert` (`epoch_transition.rs`)
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э7 (`BTreeSet<u64>` или fail-closed вместо `debug_assert`). Ex-14: не воспроизвелось при interval=12, 51 граница. _Источник:_ history/EXPERIMENTS.md Ex-14; history/PLAN.md §2 Э7

### R-047 · MINOR · `cold_start` понижает `anchor_height` безусловно
- Якоря: `staking-reader/src/epoch_transition.rs` vs (`raise_anchor_height` монотонен)
- Уверенность (REGISTER, 09-03): `[KNOWN]`
- Ход по плану 09-04 (INDEX): Э7 / П — `cold_start` через `raise_anchor_height` (`epoch_transition.rs`)
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э7 (`cold_start` через `raise_anchor_height`). _Источник:_ history/PLAN.md §2 Э7

### R-048 · MINOR · `check_peer_set_size` считает размер до дедупликации
- Якоря: `staking-reader/src/epoch_transition.rs`
- Уверенность (REGISTER, 09-03): `[KNOWN]`
- Ход по плану 09-04 (INDEX): Э7 / П — считать после `from_iter_dedup` (`epoch_transition.rs`)
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э7 (считать после `from_iter_dedup`). _Источник:_ history/PLAN.md §2 Э7

### R-049 · MINOR · Follower читает комитеты при нулевом хэше, если finalized ещё нет; результат начального FCU игнорируется; ошибка `cold_start_register` пропускается
- Якоря: `dpos.rs` (`unwrap_or_default`), (`let _ =`)
- Уверенность (REGISTER, 09-03): `[KNOWN]` первые два
- Ход по плану 09-04 (INDEX): Э6 / С+ — B-2 убирает половину; `let _ =` и ZERO-хэш — отдельно (`dpos.rs`)
- Связано: R-074; закрывается AUDIT B-2.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э6 6.3 (B-2 убирает половину); `let _ =` и ZERO-хэш — отдельно. _Источник:_ history/PLAN.md §2 Э6

### R-050 · MINOR · Маркер SafetyHalt пишется `std::fs::write` без fsync, без rename и после защёлки
- Последствие: сбой между защёлкой и записью ⇒ узел стартует не halted; при повторе той же дивергенции обратная проверка снова взводит латч
- Якоря: `sync_metrics.rs` (`persist_marker`), вызов после `latch` (, по CORE)
- Уверенность (REGISTER, 09-03): `[KNOWN]`
- Ход по плану 09-04 (INDEX): Э7 / П — маркер: tmp + fsync + rename, до `latch` (`sync_metrics.rs`)
- Связано: R-006, R-077.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э7 (маркер: tmp + fsync + rename, до `latch`). _Источник:_ history/PLAN.md §2 Э7

### R-051 · MINOR · Torn-журнал DKG (первая запись нечитаема) = пропуск эпохи; хвост усекается молча
- Якоря: `beacon/share_state.rs`, `beacon/actor.rs`
- Уверенность (REGISTER, 09-03): `[KNOWN]` `actor.rs`
- Ход по плану 09-04 (INDEX): Э5 / С+ — П-9 пересмотреть: `Torn` до дедлайна ⇒ `start_fresh`
- Связано: R-072.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э5 5.2 смягчает (пересмотреть вместе с R-072: `Torn` до дедлайна ⇒ `start_fresh`). _Источник:_ history/PLAN.md §2 Э5

### R-052 · MINOR · Share-файлы v1 в открытом виде принимаются при включённом шифровании
- Якоря: `beacon/share_state.rs` (`TAG_PLAINTEXT` без проверки `state`)
- Уверенность (REGISTER, 09-03): `[KNOWN]`
- Ход по плану 09-04 (INDEX): Э5 / С — BB-5: удалить v1/v2-ветви чтения share-файла (свежий генезис)
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э5 5.1 (BB-5: удалить v1/v2-ветви чтения share-файла; бесплатно при свежем генезисе). _Источник:_ history/PLAN.md §2 Э5, §7
- **Статус 2026-09-13 (Э5 5.1, `129f2754` + `fca59634`):** **закрыта.** Теги v1 share-файла ВЫВЕДЕНЫ: `share_state.rs:270` (открыл сам) отказывает с явным текстом «v1 tags … are RETIRED for the share file and re-run the ceremony». Заодно из формата ушло третье поле — копия артефакта. _Источник:_ history/E5-1-A.md

### R-053 · MINOR · Два разных `Agreed`-ключа для одной эпохи — только `debug_assert`; в release побеждает первый
- Последствие: достижимо только при ≥ quorum эквивокации комитета агрегации.
- Якоря: `beacon/keys.rs`; `beacon/artifact.rs` (first-wins)
- Уверенность (REGISTER, 09-03): `[KNOWN]` `keys.rs`
- Ход по плану 09-04 (INDEX): Э5 / С — П-3: ключ только из артефакта, first-wins
- Связано: R-068.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э5 5.1 (П-3: ключ только из артефакта, first-wins). _Источник:_ history/PLAN.md §2 Э5
- **Статус 2026-09-13 (Э5 5.1, `129f2754` + `fca59634`):** **закрыта классом.** Тир `KeySource::Agreed` и стор, в котором два `Agreed`-значения могли столкнуться, удалены вместе с `keys.rs`; сравнивать нечего. Значение теперь одно — у артефакта, стор insert-only. _Источник:_ history/E5-1-A.md

### R-054 · MINOR · Ингресс DKG буферизует Commitment/Share от любого пира без проверки членства
- Якоря: `beacon/ceremony.rs` (`pending_pub`/`pending_priv` по `from`), `beacon/actor.rs ` (`pending` для будущих эпох по отправителю, per-sender слот). Объём: `DealerPubMsg` ограничен кодеком размером комитета (≈3, 4 KiB при n=51, по BEACON) × число подключённых пиров ≈ 14 MiB при 4096
- Уверенность (REGISTER, 09-03): `[KNOWN]` буферизация; размер — `[LIKELY]` по BEACON
- Ход по плану 09-04 (INDEX): Э4 / С — П-5: `Member` на входе
- Связано: R-023, R-029.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э4 4.3 (П-5: `Member` на входе). _Источник:_ history/PLAN.md §2 Э4
- **Статус 2026-09-12 (Э4 4.3 закрыта):** закрыта — `48412eb3` (4.3-А: `on_message` читает `ceremony_epoch` из первых 8 байт и проверяет `epoch_is_actionable` + `beacon_member` ДО `DkgMsg::read_cfg`; `GatedReceiver` на BEACON до любого декода). Оговорка Д-123: маска на BEACON — из `committee_for` (записи модуля), не из `TrackedWindow`; стоимость ограничена тремя actionable-эпохами, запись write-once. _Источник:_ history/E4-3-A.md §0(2)

### R-055 · MINOR · Deterministic dealer RNG из подписи ed25519-ключа по эпохе
- Якоря: `beacon/ceremony.rs`
- Уверенность (REGISTER, 09-03): `[LIKELY]`
- Ход по плану 09-04 (INDEX): — / — — дизайн: детерминированный dealer RNG нужен для resume
- **Статус 2026-09-09:** снята — дизайн: детерминированный dealer RNG нужен для `resume` (PLAN §8 п.10); не трогать. _Источник:_ history/PLAN.md §8; history/REDESIGN.md ч.3 п.11

### R-056 · MINOR · Скан `chain_key_epoch` — по одному EVM-чтению на эпоху без смены комитета; `select_carry_scheme` вызывается со свежим memo на каждый probe
- Последствие: на devnet-интервале 32 через год ~10⁶ шагов на probe; на проде пренебрежимо
- Якоря: `beacon/carry.rs`; `resolve.rs`; `surface.rs`
- Уверенность (REGISTER, 09-03): `[KNOWN]` `carry.rs`
- Ход по плану 09-04 (INDEX): Э5 / С — П-3: скан `chain_key_epoch` удаляется
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э5 5.1 (скан `chain_key_epoch` удаляется). Закрывается BB-3 (§3); эксперимент Ex-15 в очереди (`EXPERIMENTS.md` §3). _Источник:_ history/PLAN.md §2 Э5
- **Статус 2026-09-13 (Э5 5.1, `129f2754` + `fca59634`):** **закрыта.** `chain_key_epoch` и `select_carry_scheme` удалены вместе с `carry.rs`; адресацию минта несёт `MintIndex::minted_at` над кэшем бит и ДОЛГОВЕЧНЫМ мемо `epoch → minted_at`, поэтому перезапущенный узел не делает ни одного chain-read за уже разрешённые эпохи. Форма ходока (без кэпа, на пути сертификата) по `DPOS_AUDIT.md` B10 остаётся, стоимость — нет. _Источник:_ history/E5-1-A.md; history/E5-1-DOCS.md

### R-057 · MINOR · Предсказуемость лидера: лидер v+1 = f(σ_v); в эпохах без witness — константный seed
- Якоря: `weighted_vrf.rs`, `epoch_manager.rs`
- Уверенность (REGISTER, 09-03): `[LIKELY]`
- Ход по плану 09-04 (INDEX): — / — — дизайн (лидер v+1 = f(σ_v))
- **Статус 2026-09-09:** снята — дизайн (лидер v+1 = f(σ_v)); не трогать. _Источник:_ history/INDEX.md §1

### R-058 · MINOR · Ошибочная эквивокация честного узла при потере simplex-журнала — ни защиты, ни документа «не удалять `consensus_epoch_*`»
- Якоря: `engine.rs`
- Ход по плану 09-04 (INDEX): Э7 / П — документ оператора «не удалять `consensus_epoch_*`»
- Связано: R-041.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э7 (документ оператора «не удалять `consensus_epoch_*`»). _Источник:_ history/PLAN.md §2 Э7

### R-059 · MINOR · EVIDENCE: батч до 102 голосов = 102 BLS-проверки + чтение комитета на батч
- Якоря: `slasher/gossip.rs` (`0..=2·MAX_COMMITTEE_SIZE`); квота 16/с
- Уверенность (REGISTER, 09-03): `[KNOWN]` cap
- Ход по плану 09-04 (INDEX): Э7 / П — кэш комитета на эпоху в `gossip.rs`; батч ≤ 102 оставить
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э7 (кэш комитета на эпоху в `gossip.rs`). _Источник:_ history/PLAN.md §2 Э7

### R-060 · MINOR · BEACON_RESOLVER: запрос лога некэшированной эпохи читает и расшифровывает журнал с диска
- Якоря: `beacon/actor.rs`, `beacon/log_store.rs`; квота 16/с
- Уверенность (REGISTER, 09-03): `[LIKELY]`
- Ход по плану 09-04 (INDEX): Э7 / П — кэш логов для serve или квота ниже (`beacon/actor.rs`)
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э7 (кэш логов для serve или квота ниже). _Источник:_ history/PLAN.md §2 Э7

### R-061 · MINOR · Неограниченные очереди: `executor::Mailbox`, `FeedSink`, slasher `Mailbox`, каналы key/seed-журналов
- Якоря: `executor.rs`, `feed_sink.rs`, `slasher/ingress.rs`, `beacon/keys.rs`, `beacon/certify.rs`
- Уверенность (REGISTER, 09-03): `[LIKELY]`
- Ход по плану 09-04 (INDEX): Э6 / С+ — П-6 ограничивает mailbox executor; `FeedSink`/slasher — bounded отдельно
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э6 6.1 смягчает (mailbox executor ограничен стадией); `FeedSink`/slasher — отдельно. _Источник:_ history/PLAN.md §2 Э6

### R-062 · MINOR · Chain ID развёрнутых сетей захардкожены в p2p отдельно от chainspec
- Якоря: `p2p/src/config.rs`
- Уверенность (REGISTER, 09-03): `[LIKELY]`
- Ход по плану 09-04 (INDEX): Э2 / С — chain_id только из chainspec
- **Статус 2026-09-09:** отложена — Э2.4 (chain_id только из chainspec); механизм после 09-04 не пересматривался. _Источник:_ history/PLAN.md §2 Э2 2.4

### R-063 · MINOR · `cert_inlet` молча отбрасывает сертификат, когда комитет ещё не читается и кэша нет
- Последствие: высота восстанавливается повторным запросом marshal при `FollowerResolver::Upstream`; при `Noop` — потеряна, но `Noop` «not a reachable production config» (`cert_inlet.rs`, по CORE CB-2)
- Якоря: `cert_inlet.rs`, (`Entry::Vacant(_) => return Ok(())`)
- Уверенность (REGISTER, 09-03): `[KNOWN]` ветки drop
- Ход по плану 09-04 (INDEX): Э4 / С+ — П-1 меняет код, не поведение; ветка `Noop` уходит с CB-2 (Э6)
- Связано: R-009, R-033.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э4 4.1 (П-1 меняет код, не поведение); ветка `Noop` уходит с CB-2 (Э6 6.3). _Источник:_ history/PLAN.md §2 Э4, Э6
- **Статус 2026-09-11:** смягчена — `65a23db7` (Э4 4.1 Б2: inlet спрашивает `Committee::scheme(E)`; `None` = ещё не читается/отказано ⇒ пропуск без фатала, постоянный класс громкий в модуле). Ветка `Noop` остаётся — Э6 6.3 (CB-2). _Источник:_ history/E4-1-B2.md Д-34

### R-064 · MINOR · Бесконечные циклы ожидания без give-up: `wait_for_activation_block`, `cold_start_jump_self_heal`, re-poke границы, `EL_SYNC_BACKSTOP_CEILING = 6 ч`
- Якоря: `dpos.rs`; `cold_start_jump.rs`
- Уверенность (REGISTER, 09-03): `[KNOWN]` `dpos.rs` [ссылка уточнена, REFS.md] частично
- Ход по плану 09-04 (INDEX): Э6 / С+ — П-7: алерт-метрики; циклы без give-up остаются осознанно
- Связано: R-042.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э6 6.2 смягчает (алерт-метрики; циклы без give-up остаются осознанно). _Источник:_ history/PLAN.md §2 Э6
- **Статус 2026-09-12 (Э4 4.2 закрыта):** смягчена — `cold_start_jump_self_heal` с вечным повтором удалён `dacd1bfa` (4.2-Б2; посадка проверяется один раз, отказ — `Fault`); остальные три цикла (`wait_for_activation_block`, re-poke границы, `EL_SYNC_BACKSTOP_CEILING`) без изменений — Э6 6.2. _Источник:_ history/E4-2-B2.md

### R-065 · MINOR · `enter_boundary` порождает по одному вечному re-poke циклу на каждый вызов под общим мьютексом `EpochTransition`
- Якоря: `dpos.rs` (цикл на каждый `Update::Block` и на каждое приземление re-jump; два цикла на одну границу признаны комментарием)
- Уверенность (REGISTER, 09-03): `[KNOWN]`
- Ход по плану 09-04 (INDEX): Э8 / П — CB-8: один драйвер границы с `watch<u64>`
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э8 (CB-8: один драйвер границы с `watch<u64>`). _Источник:_ history/PLAN.md §2 Э8

### R-066 · MINOR · Пустой комитет трактуется как «ещё не закоммичен» и паркует границу без предела
- Якоря: `staking-reader/src/epoch_transition.rs`; `dpos.rs`
- Уверенность (REGISTER, 09-03): `[LIKELY]`
- Ход по плану 09-04 (INDEX): Э7 / П — только комментарий (R-106); поведение верно
- Связано: R-027. К-10 закрыт (history/CONTRACT.md): «непусто ⇔ закоммичено» — точная эквивалентность (`consensus.rs`); пропустить эпоху курсор не может, только отстать, а комитеты `e+1` и `e+2` закоммичены к первому блоку эпохи `e`. Трактовка узла верна, тяжесть не менялась.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э7 (только комментарий; поведение верно — К-10). _Источник:_ history/PLAN.md §2 Э7

### R-067 · MINOR · Резолвер ждёт ответ DKG-актора inline, актор шлёт в mailbox резолвера с ожиданием — взаимная зависимость с ограниченной ёмкостью 256
- Якоря: `CW:resolver/src/p2p/engine.rs`, `beacon/log_resolver.rs`, `beacon/actor.rs`, `beacon/plane.rs`
- Уверенность (REGISTER, 09-03): `[LIKELY]`
- Ход по плану 09-04 (INDEX): Э7 / П — `try_send` из актора в mailbox резолвера (`beacon/actor.rs`)
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э7 (`try_send` из актора в mailbox резолвера). _Источник:_ history/PLAN.md §2 Э7

### R-068 · MINOR · Key-journal записывает проигравшее значение при конфликте `Agreed`-vs-`Agreed`; после рестарта RAM и диск расходятся
- Якоря: `beacon/keys.rs` (`set_pk` шлёт в persist всегда, независимо от исхода `insert`), `beacon/key_journal.rs `;`Ordinal::put` перезаписывает индекс
- Уверенность (REGISTER, 09-03): `[KNOWN]set_pk`
- Ход по плану 09-04 (INDEX): Э5 / С — П-3: `key_journal.rs` удаляется
- Связано: R-053; исчезает при AUDIT B-7.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э5 5.1 (`key_journal.rs` удаляется). _Источник:_ history/PLAN.md §2 Э5
- **Статус 2026-09-13 (Э5 5.1, `129f2754` + `fca59634`):** **закрыта.** `key_journal.rs` удалён целиком (417 строк): диска у стора ключей больше нет, расходиться с RAM нечему. Щель, которую журнал закрывал (адресация минта без chain-read), передана долговечному мемо — см. R-056. _Источник:_ history/E5-1-A.md

### R-069 · MINOR · `on_invalid_seed` судит по ключу живой эпохи, а `Agreed` хранится под эпохой mint'а: на carry-forward эпохах вердикт всегда `Quarantine`; отброшенный при promote σ запросить нельзя
- Якоря: `beacon/keys.rs` (`cached_at_least(epoch, Agreed)` при живой `epoch`), `beacon/certify.rs `, `beacon/log_resolver.rs `
- Уверенность (REGISTER, 09-03): `[KNOWN]`
- Ход по плану 09-04 (INDEX): Э5 / С — П-2: `on_invalid_seed`/`promote_epoch` удаляются
- Связано: R-008 (часть его цепочки).
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э5 5.3 (`on_invalid_seed`/`promote_epoch` удаляются). _Источник:_ history/PLAN.md §2 Э5

### R-070 · MINOR · Два реестра метрик: `BeaconMetrics` на commonware (`:19100`), свидетели расхождения ключей — на `metrics::`
- Якоря: `beacon/metrics.rs`; `keys.rs`, `surface.rs`, `resolve.rs`, `key_journal.rs`, `seed_journal.rs`, `artifact.rs`
- Уверенность (REGISTER, 09-03): `[KNOWN]` `keys.rs`
- Ход по плану 09-04 (INDEX): Э0 / П — все счётчики в один реестр; reth-реестр открыть в compose
- **Статус 2026-09-09:** смягчена наполовину — `2cc13bcd` (Э0.4: реестр reth открыт `--metrics=0.0.0.0:9001` в compose, семьи `metrics::` видны — 515 семейств). Единого реестра нет и не будет в этой форме: часть вызовов (`node/src/derive.rs`, `node/src/evm.rs`) без контекста `impl Metrics`; склейка двух экспозиций в один эндпойнт опасна (дубль имени семьи роняет весь скрейп). _Источник:_ history/E0-LOG.md 0.4

### R-071 · MINOR · Ненаблюдаемые состояния beacon: размер quarantine/terminal-пинов, живые церемонии и фазы, `agreed_pinned`, возраст отложенного finalize, `recompute_pending`/`want`, `nondurable_logs`, sit-out по `Torn`, view агрирования, in-flight резолвера
- Якоря: `beacon/certify.rs`, `beacon/actor.rs `
- Уверенность (REGISTER, 09-03): `[LIKELY]`
- Ход по плану 09-04 (INDEX): Э7 / П — gauges beacon: церемонии, фазы, `recompute_pending`,`nondurable_logs`
- Связано: R-077.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э7 (gauges beacon). _Источник:_ history/PLAN.md §2 Э7

### R-072 · MINOR · Torn первого же журнального рекорда ⇒ сидеть вне эпохи, хотя ничего не отправлено
- Якоря: `beacon/actor.rs` (`start_fresh` пишет журнал до рассылки), `share_state.rs`, `actor.rs`
- Уверенность (REGISTER, 09-03): `[KNOWN]`
- Ход по плану 09-04 (INDEX): Э5 / С+ — П-9 пересмотреть вместе с R-051
- Связано: R-036 (обратная сторона: `Torn` до дедлайна ⇒ `start_fresh` безопасен по детерминизму), R-051.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э5 5.2 (пересмотреть вместе с R-051). _Источник:_ history/PLAN.md §2 Э5

### R-073 · MINOR · `RotatedKey`-движок спавнится и никогда не снимается, вопреки комментарию
- Якоря: `epoch_manager.rs`; на следующем reconcile живой движок снимается только при `share_probe == Withheld`
- Уверенность (REGISTER, 09-03): `[KNOWN]`
- Ход по плану 09-04 (INDEX): Э7 / П — abort `RotatedKey`-движка на следующем reconcile (`epoch_manager.rs`)
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э7 (abort `RotatedKey`-движка на следующем reconcile). _Источник:_ history/PLAN.md §2 Э7

### R-074 · MINOR · Вердикт `Ok(Invalid)` FCU игнорируется в `reseed_forward` и в heartbeat
- Якоря: `executor.rs` (только `Err`), (`Ok(_)` ⇒ recover)
- Уверенность (REGISTER, 09-03): `[KNOWN]`
- Ход по плану 09-04 (INDEX): Э6 / С — П-6: один читатель вердикта FCU
- Связано: R-043, R-049.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э6 6.1 (П-6: один читатель вердикта FCU). _Источник:_ history/PLAN.md §2 Э6

### R-075 · MINOR · `soft_enter_span` считает эпоху зарегистрированной, даже если `register` отказал
- Якоря: `outer.rs` (`registered = epoch` после `register`), (три ветки молчаливого отказа с `error!`);`epoch_manager.rs `
- Уверенность (REGISTER, 09-03): `[KNOWN]`
- Ход по плану 09-04 (INDEX): Э4 / С+ — П-1 убирает ветку «другой комитет»; `registered` только после `Ok` — отдельно (`outer.rs`)
- Связано: R-045. К-11 закрыт (history/CONTRACT.md): состав, ключи и веса закоммиченной эпохи неизменяемы, governance-пути к ним нет; тяжесть не менялась.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э4 4.1 смягчает; `registered` только после `Ok` — отдельно. _Источник:_ history/PLAN.md §2 Э4
- **Статус 2026-09-11:** закрыта — `65a23db7` (Э4 4.1 Б2: `soft_enter_span`, `register_soft_entered`, `EpochSchemeProvider::register` и второй кэш `CertInlet.schemes` удалены; чтение записи через модуль И ЕСТЬ регистрация схемы — `EpochEntry{record, scheme}`, одна карта; `register_span` останавливается на первой нечитаемой ЗАПИСИ). Стенд `four_nodes_at_four_heights_hold_one_committee_record_per_epoch` (`03c3151f`) пинует одну запись на эпоху у четырёх узлов на четырёх якорях. _Источник:_ history/E4-1-B2.md §0.1–0.2, E4-1-B3.md В§0.5

### R-076 · MINOR · Follower: эпоха, пропущенная `enter_finalized_epoch`, не имеет схемы; сертификаты этой эпохи marshal «принимает» без сохранения
- Якоря: `dpos.rs` (принято как допустимое), `CW:marshal/core/actor.rs` (по CORE)
- Уверенность (REGISTER, 09-03): `[KNOWN]` комментарий/код `dpos.rs`
- Ход по плану 09-04 (INDEX): Э4 / С — П-1: схема есть для любой `E ≤ epoch(fin)+2`
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э4 4.1 (П-1: схема есть для любой `E ≤ epoch(fin)+2`). _Источник:_ history/PLAN.md §2 Э4
- **Статус 2026-09-11:** закрыта — `65a23db7` (Э4 4.1 Б2: схема follower'а берётся из карты модуля лениво — `CertProvider::scoped(E)` = `Committee::scheme(E)` читает любую эпоху внутри окна `[epoch(anchor)−8, +2]`, пропуск `enter_finalized_epoch` схему не теряет). Стендом не пинуется (follower'а на стенде нет). Цена ленивого чтения — два staticcall'а в акторе marshal'а на промахе (B2-03) — замер Э7. _Источник:_ history/E4-1-B2.md Д-38, E4-1-B2-REVIEW.md B2-03

### R-077 · MINOR · Наблюдаемость: у `awaiting_seed` нет gauge, у маркера SafetyHalt нет высоты/хэша
- Якоря: `executor.rs`, `sync_metrics.rs`
- Уверенность (REGISTER, 09-03): `[KNOWN]`
- Ход по плану 09-04 (INDEX): Э7 / П — gauge `awaiting_seed`; высота/хэш в маркере halt
- Связано: R-007, R-050, R-071.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э7 (gauge `awaiting_seed`; высота/хэш в маркере halt). _Источник:_ history/PLAN.md §2 Э7

### R-078 · MINOR · Паника marshal при ошибке архива (`panic!("failed to finalize")`) превращается супервизором в abort-all
- Якоря: `CW:marshal/core/actor.rs` (по CORE); `outer.rs`
- Уверенность (REGISTER, 09-03): `[KNOWN]` `outer.rs`
- Ход по плану 09-04 (INDEX): Э6 / С+ — П-7: причина выхода в маркере; abort-all остаётся
- Связано: R-032.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э6 6.2 смягчает (причина выхода в маркере; abort-all остаётся). _Источник:_ history/PLAN.md §2 Э6

### R-079 · MINOR · `load_from_dns` на исходе 120-секундного окна возвращает пустой список без ошибки
- Якоря: `p2p/src/bootstrappers.rs` (по COVERAGE §2.3)
- Уверенность (REGISTER, 09-03): `[LIKELY]`
- Ход по плану 09-04 (INDEX): Э7 / П — пустой список bootstrappers ⇒ ошибка (`p2p/bootstrappers.rs`)
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э7 (пустой список bootstrappers ⇒ ошибка). _Источник:_ history/PLAN.md §2 Э7

### R-102 · MINOR · `epoch_of_block` и `epoch_at_block` расходятся при `activation == 0`; оба комментария об этом неверны
- Якоря: Узел: `staking-reader/src/reader.rs` — `block_number.saturating_sub(activation)/interval`, то есть `block_number/interval`, и doc на называет это «absolute numbering». Контракт: `math.rs` — при нулевой активации всегда эпоха `0`, и doc на утверждает, что «the node reads it the same way». Оба комментария о том же месте противоречат друг другу; тесты обеих сторон закрепляют свой вариант (`math.rs` против `reader.rs`). Достижимость: на продакшн-путях `0` сворачивается в `None` до вызова (`reader.rs`, `node/evm.rs`), так что корректность держится на гейте, а не на самой функции. Тег: MINOR. Новая. Уверенность: `[KNOWN]` обе реализации; `[LIKELY]` недостижимость.
- Уверенность (REGISTER, 09-03): `[KNOWN]` обе реализации; `[LIKELY]` недостижимость
- Ход по плану 09-04 (INDEX): Э2 / С — одна функция эпохи в общем крейте
- **Статус 2026-09-09:** смягчена — `9b6213be` (2.2: общая `staking_protocol::epoch_at_block`, `epoch_of_block` удалён; порядок аргументов выровнен). Арм `activation == 0` («невзведённый сентинел» ⇒ эпоха 0) остаётся у контракта осознанно — живое состояние девнета; узел до него не доходит. _Источник:_ history/E2-ABI.md §2.2

### R-103 · MINOR · `getActiveValidatorsLength()` отдаёт запланированный кап, а не действующий
- Якоря: Контракт пишет скаляр немедленно, а чекпойнт — на `next_epoch` (`config.rs`), и отбор комитета читает чекпойнт (`config.rs` через `staking.rs`), тогда как view возвращает скаляр (`config.rs`). Узел читает эту view один раз при старте ради проверки «≤ 51» (`dpos.rs`), а doc в reader'е (`staking-reader/src/reader.rs`) называет её действующим капом и «размером будущих комитетов». Для самой проверки последствия нет (обе величины ≤ 51); последствие — для любого будущего потребителя. Тег: MINOR. Новая. Уверенность: `[KNOWN]`.
- Уверенность (REGISTER, 09-03): `[KNOWN]`
- Ход по плану 09-04 (INDEX): Э1 / С+ — B-6 удаляет view либо doc reader'а исправить
- **Статус 2026-09-09:** закрыта — `c31c258f` (1.1: чекпойнты cap удалены, кап читается скаляром; по столбцу «Закрывает» PLAN 1.1). _Источник:_ history/PLAN.md §2 Э1 1.1

### R-104 · MINOR · Узел утверждает, что контрактного обработчика `slashEquivocation(uint64,uint32)` не существует; он есть
- Якоря: `node/evm.rs`: «**The contract has no counterpart at all** … verified 2026-08-14: zero hits for the signature in `consts.rs` on every branch». Ветка в утверждении — та самая, из которой читались исходники. Обработчик: `consts.rs`, диспетчер `lib.rs`, реализация `consensus.rs` (только `SYSTEM_CALLER`, молчаливый `Ok()` на уже tombstone'нутой жертве). Селектор `0xdc6fb3f2` присутствует ровно один раз в развёрнутом devnet-блобе (скан `.rwasm`, `history/CONTRACT.md` часть 3). Тег: MINOR. Новая; отменяет механизм R-028 и задачу AUDIT B-3. Уверенность: `[KNOWN]` обе стороны.
- Уверенность (REGISTER, 09-03): `[KNOWN]` обе стороны
- Ход по плану 09-04 (INDEX): Э2 / С — комментарий уходит вместе с ручным `sol!`
- **Статус 2026-09-09:** закрыта — `065003ad` (2.1: комментарий «The contract has no counterpart at all» удалён вместе с тестом-носителем). _Источник:_ history/E2-ABI.md §2.3

### R-105 · MINOR · Узел утверждает, что контракт не проверяет межвалидаторную уникальность ключей; проверяет, и дважды каждый
- Якоря: Четыре места узла ссылаются на несуществующую `Staking.setConsensusKeys`:`scheme.rs ` и, `bls/src/scheme.rs`, `engine.rs`; два из них утверждают свойство — «does NOT enforce cross-validator uniqueness of peerPubkey/blsPubkey». Контракт отвергает занятый peer-ключ (`consensus.rs`) и занятый BLS-ключ, и перепроверяет оба после внешних вызовов верификатора против reentrancy; перезапись собственных ключей запрещена . Защитный код узла остаётся оправданным, но заявленная достижимость («reachable from on-chain data», `engine.rs`) отсутствует. Тег: MINOR. Новая. Уверенность: `[KNOWN]` обе стороны.
- Уверенность (REGISTER, 09-03): `[KNOWN]` обе стороны
- Ход по плану 09-04 (INDEX): Э7 / П — четыре комментария про `setConsensusKeys` удалить
- **Статус 2026-09-09:** смягчена — `065003ad` (2.1: после неё единственный оставшийся «MUST mirror» — `bls/src/scheme.rs`, ссылается на `Staking.sol`, правится с 2.3); четыре комментария про `setConsensusKeys` — по плану Э7. _Источник:_ history/PLAN.md §2 Э7; history/E2-ABI.md §5

### R-106 · MINOR · Узел ссылается на несуществующее расписание коммита и на несуществующее состояние «пропущенная эпоха»
- Якоря: `staking-reader/src/epoch_transition.rs` — «`Staking.sol` allows an epoch with no `commitEpochCommittee` … a skip is safe»; курсор `last_committed_epoch_p1` читается как `target` и становится `target+1` в том же вызове (`consensus.rs`), так что эпоха может отстать, но не может быть пропущена. Там же — «the v41 QUALIFY-BEFORE-COMMIT schedule … committed at `H_qual = B−8`»; ни того расписания, ни `H_qual` в контракте нет, `commitEpochCommittee` не принимает аргументов и её собственная документация фиксирует удаление той схемы (`consensus.rs`), а пацинг целиком узловой (`node/evm.rs`). Поведение узла (парковка + re-poke) для реального состояния верно, и фактическая гарантия сильнее заявленной. Тег: MINOR. Новая. Уверенность: `[KNOWN]` обе стороны. Связано: R-066.
- Уверенность (REGISTER, 09-03): `[KNOWN]` обе стороны
- Ход по плану 09-04 (INDEX): Э7 / П — комментарий `epoch_transition.rs` заменить на реальное расписание
- Связано: R-066.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э7 (комментарий `epoch_transition.rs` про `H_qual`). _Источник:_ history/PLAN.md §2 Э7

### R-110 · MINOR · Выравнивание `activation % interval == 0` названо в узле неподдерживаемым соглашением; контракт его требует
- Якоря: `staking-reader/src/epoch_transition.rs` — «a devnet bootstrap convention, **NOT enforced**». Контракт требует его во всех трёх местах, где любое из полей может быть установлено: `config.rs` (инициализация), (`setEpochBlockInterval`), (`setDposActivationBlock`), везде `ERR_UNALIGNED_ACTIVATION_BLOCK`. Дефекта нет — относительная арифметика узла верна при любом раскладе, — но это инвариант, а не соглашение, и рассуждения о расхождении абсолютной и относительной границы стоят на обратном. Тег: MINOR. Новая. Уверенность: `[KNOWN]` обе стороны.
- Уверенность (REGISTER, 09-03): `[KNOWN]` обе стороны
- Ход по плану 09-04 (INDEX): Э7 / П — комментарий `epoch_transition.rs`
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э7 (комментарий `epoch_transition.rs` про выравнивание). _Источник:_ history/PLAN.md §2 Э7

### R-120 · ПРИНЯТО (остаточный риск) · Эквивокация в последних видах эпохи при нулевом пересечении комитетов остаётся безнаказанной — ПРИНЯТО как остаточный риск (решение Д-4, 2026-09-04)
- Механизм: заряд эпохи E может попасть в блок только пока идёт E. Ротированный из комитета узел заряд сохраняет, но предложить его не может (нет слота лидера) и его курсор замерз на E. Узел, у которого слот в E+1 есть, заряда не держит: он не был в комитете E, а после конца E улики эпохи E никто не переиздаёт. При нулевом пересечении комитетов (принято как проектное допущение) эти два множества не пересекаются, поэтому эквивокация, замеченная в последних видах эпохи, до контракта не доходит ни одним путём, кроме маршрута по уликам.
- Последствие: эквивокатор не получает tombstone, а значит не срабатывает ничего из зависящего от флага — ни отказ привязывать его предложение, ни разрыв транспорта, ни конфискация самостейка. Он остаётся в комитете со своим весом.
- Якоря: `application.rs` (`next_charge(round.epoch())` — заряд предлагается только для эпохи текущего раунда), `slasher/evidence.rs` (гейт голосования требует `charged == epoch блока`), `slasher/actor.rs` (курсор двигают только активности собственного движка; `republish` вызывается только по `Nullification`/`Notarization`, то есть внутри эпохи; заряд и голоса переживают границу), `outer.rs` (слэшер строится один раз на процесс, движки — по эпохам), `slasher/tombstone.rs` + `application.rs` (весь хвост реакции висит на он-чейн флаге `tombstoned`).
- Уверенность (REGISTER, 09-03): `[KNOWN]` по механизму недоставки — все шесть якорей прочитаны; `[LIKELY]` по характеру последствия (см. выше).
- Связано: R-022, R-028, R-116; `DECISIONS.md` Д-4.
- **Статус 2026-09-09:** принята как остаточный риск (Д-4, 09-04) — не задача. Если Д-4 разворачивается (маршрут улик остаётся), запись теряет основание. _Источник:_ history/DECISIONS.md Д-4; history/REGISTER.md R-120

### R-080 · NIT · `test_only_mailbox` публичен без `cfg(test)`; ревью-комментарии в коде
- Якоря: `test_only_mailbox` публичен без `cfg(test)`; ревью-комментарии в коде. `slasher/ingress.rs`; `slasher/actor.rs`, `slasher/ingress.rs, ` [ссылки исправлены, history/REFS.md]. Прежний: AUDIT A-39 NIT. `[LIKELY]`.
- Ход по плану 09-04 (INDEX): Э8 / П — `cfg(test)` на `test_only_mailbox`; ревью-маркеры `****` удалить
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э8 (гигиена). _Источник:_ history/PLAN.md §2 Э8

### R-081 · NIT · Слот σ при флаге 0 не проверяет нулевые байты — кодирование голоса неканонично; commonware дедуплицирует по (view, signer)
- Якоря: Слот σ при флаге 0 не проверяет нулевые байты — кодирование голоса неканонично; commonware дедуплицирует по (view, signer). `bls/src/combined_scheme.rs`. Прежний: AUDIT A-41 NIT. `[LIKELY]`.
- Ход по плану 09-04 (INDEX): Э8 / П — отвергать ненулевые байты при флаге 0 (`combined_scheme.rs`)
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э8 (гигиена). _Источник:_ history/PLAN.md §2 Э8

### R-082 · NIT · Пространства имён DKG
- Якоря: Пространства имён DKG (`Info` на seed-namespace, confirm — seed‖`_DKG_CONFIRM`, agreement — chain‖`_DKG_AGREE`): коллизий нет, именование затрудняет ревью. `beacon/plane.rs`. Прежний: AUDIT A-42 NIT. `[LIKELY]`.
- Ход по плану 09-04 (INDEX): Э8 / П — переименовать namespace-константы
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э8 (гигиена). _Источник:_ history/PLAN.md §2 Э8

### R-083 · NIT · `decrypt` без проверки длины на production-пути; keystore v3 отвергается; PBKDF2 `c` не ограничен (оператор сам себе)
- Якоря: `decrypt` без проверки длины на production-пути; keystore v3 отвергается; PBKDF2 `c` не ограничен (оператор сам себе). `bls/src/keys.rs`, `bls/src/keystore.rs`. Прежний: AUDIT A-43 NIT. `[LIKELY]`.
- Ход по плану 09-04 (INDEX): Э8 / П — верхняя граница PBKDF2 `c`;`decrypt_fixed` на прод-пути
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э8 (гигиена). _Источник:_ history/PLAN.md §2 Э8

### R-084 · NIT · `activationEpoch` декодируется и не используется; `getUndelegatePeriod` объявлен и не вызывается. К-12 закрыт: `activationEpoch` — эпоха, с которой ключи валидатора действуют; контракт применяет её при отборе
- Якоря: `activationEpoch` декодируется и не используется; `getUndelegatePeriod` объявлен и не вызывается. К-12 закрыт: `activationEpoch` — эпоха, с которой ключи валидатора действуют; контракт применяет её при отборе (`consensus.rs`), поэтому у каждого члена закоммиченного комитета ключи действующие по построению, и узлу она не нужна. `staking-reader/src/reader.rs`. Прежний: AUDIT A-60 NIT. `[KNOWN]`.
- Ход по плану 09-04 (INDEX): Э2 / С — `activationEpoch` не декодировать после перехода на общий ABI
- **Статус 2026-09-09:** закрыта — `065003ad` (2.1: объявление `getUndelegatePeriod` удалено из узла — E2-ABI §2.7; `activationEpoch` после перехода на общий ABI не декодируется — по столбцу «Закрывает» PLAN 2.1, отдельно не подтверждено [LIKELY]). _Источник:_ history/E2-ABI.md §2.7; history/PLAN.md §2 Э2 2.1

### R-085 · MINOR (пересмотрено 09-04: было NIT — по Ex-17) · Регистрация метрик при двух плоскостях в одном процессе: `prometheus-client` не отклоняет дубликаты имён; в проде регистрация одна на процесс; 8 семей без префикса `dpos_`
- Якоря: Регистрация метрик при двух плоскостях в одном процессе: `prometheus-client` не отклоняет дубликаты имён; в проде регистрация одна на процесс; 8 семей без префикса `dpos_` (`beacon/metrics.rs`). Прежний: BEACON BA-12 NIT; COVERAGE §5 п. 9 (тот же вопрос). **Повышено до MINOR** по итогам эксперимента. Эксперимент 2026-09-04 (Ex-17, `EXPERIMENTS.md`): «8 семей без `dpos_`» — подтверждено точно. Дублирующихся СЕМЕЙ в живом экспорте нет (280 семей, ни одного повторного `# TYPE`) и при регистрации «одна на процесс» быть не может; зато **подтверждено экспериментом** дублирование СЕРИЙ — per-epoch simplex-движки регистрируются под фиксированным префиксом без метки эпохи, и настоящий Prometheus при `up = 1` и пустой `lastError` молча отбрасывает часть samples на каждом скрейпе (`Error on ingesting samples with different value but same timestamp`), ровно консенсусная наблюдаемость; цифры серий и доли потерь — `EXPERIMENTS.md` §1 Ex-17. Дубль ИМЕНИ (а не серии) парсер отвергает целиком (`second HELP line for metric name`), то есть терялся бы весь скрейп. Связано: R-073/R-041 — число живых движков прямо задаёт долю потерь.
- Ход по плану 09-04 (INDEX): Э0 / П — per-epoch simplex-метрики с меткой эпохи либо abort старых движков (Д-8); тяжесть MINOR по Ex-17
- Связано: R-073/R-041 — число живых движков прямо задаёт долю потерь.
- **Статус 2026-09-09:** закрыта — `2cc13bcd` (Э0.4: `EpochEngine::new` вешает атрибут `epoch` на контекст; дублей серий 0, Prometheus `num_dropped` 0). Остаток вне записи: `promtool check metrics` — линт (183 семьи без HELP, 40 имён `_total_total`) — отдельная гигиена. _Источник:_ history/E0-LOG.md 0.4; history/EXPERIMENTS.md Ex-17

### R-086 · NIT · `keys.rs`: неограниченные множества `reported_invalid_seed`
- Якоря: `keys.rs`: неограниченные множества `reported_invalid_seed` (, растёт на эпоху), `extra_notifiers` . Кэш `carry.rs` без границы обоснован — не дефект. Прежний: BEACON BA-16 NIT. `[LIKELY]`.
- Ход по плану 09-04 (INDEX): Э5 / С+ — П-2 убирает `reported_invalid_seed`;`extra_notifiers` — bounded
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э5 5.3 смягчает (`reported_invalid_seed` уходит с П-2). _Источник:_ history/PLAN.md §2 Э5

### R-087 · NIT · `handle(Ack)` снимает игрока с retransmit при невалидной подписи ack
- Якоря: `handle(Ack)` снимает игрока с retransmit при невалидной подписи ack (`beacon/ceremony.rs`) — только self-harm. Прежний: BEACON BA-17 NIT. `[LIKELY]`.
- Ход по плану 09-04 (INDEX): Э8 / П — не снимать игрока с retransmit при невалидном ack
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э8 (гигиена). _Источник:_ history/PLAN.md §2 Э8

### R-088 · NIT · `seed_journal::append` для раунда ниже pruned-floor пересоздаёт blob, следующий prune его удаляет
- Якоря: `seed_journal::append` для раунда ниже pruned-floor пересоздаёт blob, следующий prune его удаляет (`beacon/seed_journal.rs` — `append` [ссылка добавлена, history/REFS.md]; — prune в `spawn_writer`). Прежний: BEACON BA-19 NIT. `[LIKELY]`.
- Ход по плану 09-04 (INDEX): Э5 / С — `seed_journal.rs` удаляется с П-2
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э5 5.3 (`seed_journal.rs` удаляется с П-2). _Источник:_ history/PLAN.md §2 Э5

### R-089 · NIT · `plane.rs` replay блокирует `build` при > 32 артефактах до старта актора
- Якоря: `plane.rs` replay блокирует `build` при > 32 артефактах до старта актора (`beacon/plane.rs`, канал 16 + `adopt_tx` 16); недостижимо (журналы 1–2 эпох). Прежний: BEACON BA-21 NIT. `[LIKELY]`.
- Ход по плану 09-04 (INDEX): Э8 / П — канал replay ≥ числа артефактов
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э8 (гигиена). _Источник:_ history/PLAN.md §2 Э8

### R-090 · NIT · `follower.rs::observe_cert` делает `retain_from(epoch − 8)` на каждый сертификат: далеко ушедший upstream удаляет `Carried`-ключи эпох, которые executor ещё деривит ⇒ повторные `ensure_key`/pull
- Якоря: `follower.rs::observe_cert` делает `retain_from(epoch − 8)` на каждый сертификат: далеко ушедший upstream удаляет `Carried`-ключи эпох, которые executor ещё деривит ⇒ повторные `ensure_key`/pull (`beacon/follower.rs`). Прежний: BEACON BA-22 NIT. `[LIKELY]`.
- Ход по плану 09-04 (INDEX): Э5 / С — П-3/B-6: `Carried` и `follower.rs` удаляются
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э5 5.1/5.3 (`Carried` и `follower.rs` удаляются). _Источник:_ history/PLAN.md §2 Э5
- **Статус 2026-09-13 (Э5 5.1, `129f2754` + `fca59634`):** **закрыта классом.** Тир `Carried` и `retain_from` стора ключей удалены вместе с `keys.rs`; `observe_cert` больше не подметает ключи (осталась только σ-нога, до 5.2). _Источник:_ history/E5-1-A.md

### R-091 · NIT · Устаревшие комментарии:
- Якоря: Устаревшие комментарии: `dkg_oracle.rs` («DKG_MARGIN_BLOCKS=10» при 20), `share_state.rs` («OsRng»), `outcome.rs`, `seed.rs`, `keys.rs`, тесты `actor.rs`. Плюс CORE E-1..E-15 (см. часть 6). Прежний: BEACON BA-23 NIT. `[KNOWN]` только `share_state.rs` через R-036.
- Ход по плану 09-04 (INDEX): Э8 / П — устаревшие комментарии (список в REGISTER R-091, CORE E-1..E-15)
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э8 (гигиена). _Источник:_ history/PLAN.md §2 Э8

### R-092 · NIT · Тесты-тавтологии:
- Якоря: Тесты-тавтологии: `dkg_agree.rs`, `dkg_engine.rs`. Прежний: BEACON BA-24 NIT. `[LIKELY]`.
- Ход по плану 09-04 (INDEX): Э8 / П — два теста-тавтологии удалить
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э8 (гигиена). _Источник:_ history/PLAN.md §2 Э8

### R-093 · NIT · После halt каждый reconcile логирует `error!` об отказе понижения схемы
- Якоря: После halt каждый reconcile логирует `error!` об отказе понижения схемы (`epoch_manager.rs` → `outer.rs`). Прежний: CORE C-22 NIT. `[KNOWN]outer.rs`.
- Ход по плану 09-04 (INDEX): Э7 / П — после halt не логировать `error!` на каждый reconcile
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э7. _Источник:_ history/PLAN.md §2 Э7

### R-094 · NIT · Result-gate делает 41 итерацию
- Якоря: Result-gate делает 41 итерацию (`0..=polls`), BiMap клонируется на каждый verify с обвинением (`application.rs`). Прежний: CORE C-23 NIT. `[LIKELY]`.
- Ход по плану 09-04 (INDEX): Э8 / П — `0..polls`; не клонировать BiMap
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э8 (гигиена). _Источник:_ history/PLAN.md §2 Э8

### R-095 · NIT · `UpstreamResolver::cancel` снимает высоту из `inflight`, пока задача летит — возможен дубль pull'а
- Якоря: `UpstreamResolver::cancel` снимает высоту из `inflight`, пока задача летит — возможен дубль pull'а (`cert_inlet.rs` vs). Прежний: CORE C-24 NIT. `[LIKELY]`. Связано: R-009.
- Ход по плану 09-04 (INDEX): Э8 / П — `cancel` не трогать `inflight`, пока задача жива
- Связано: R-009.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э8 (гигиена). _Источник:_ history/PLAN.md §2 Э8

### R-096 · NIT · `hint_finalized` целится в отправителя backup-голоса, который может быть не членом комитета
- Якоря: `hint_finalized` целится в отправителя backup-голоса, который может быть не членом комитета (`epoch_manager.rs`); в Hybrid цель игнорируется (`cert_inlet.rs`). Прежний: CORE C-25 NIT. `[LIKELY]`. Связано: R-003.
- Ход по плану 09-04 (INDEX): Э4 / С — П-4/П-5: `hint_finalized` только в `Member`
- Связано: R-003.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э4 4.2/4.3 (`hint_finalized` только в `Member`). _Источник:_ history/PLAN.md §2 Э4
- **Статус 2026-09-12 (Э4 4.2 закрыта):** закрыта — `48bf62ed` (4.2-В: catch-up span и его `hint_finalized` по отправителю backup-голоса удалены; единственный адресный хинт — ступень лестницы Б1 `hint_finalization(last(T+1), committee(T+1))` из `executor::probe_frontier`, `20f47287`). _Источник:_ history/E4-2-V.md §0(2)

### R-097 · NIT · `OrderBlock::read_cfg` зависит от непрерывности буфера
- Якоря: `OrderBlock::read_cfg` зависит от непрерывности буфера (`order_block.rs`, `buf.chunk`); задокументировано. Прежний: CORE C-26 NIT. `[LIKELY]`.
- Ход по плану 09-04 (INDEX): Э8 / П — документировано; `copy_to_bytes` при сегментированном буфере
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э8 (гигиена). _Источник:_ history/PLAN.md §2 Э8

### R-098 · NIT · `lock().unwrap()` на мьютексах в spawn-задачах
- Якоря: `lock.unwrap` на мьютексах в spawn-задачах (`cert_inlet.rs`; `plane_upstream.rs`; `outer.rs`) — отравленный мьютекс роняет задачу. Прежний: CORE C-27 NIT; UNDERSTANDING §11.7. `[KNOWN]plane_upstream.rs`.
- Ход по плану 09-04 (INDEX): Э8 / П — `unwrap_or_else(into_inner)` в 8 местах
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э8 (гигиена). _Источник:_ history/PLAN.md §2 Э8

### R-099 · NIT · `reject_insecure_mode` не на Unix — no-op
- Якоря: `reject_insecure_mode` не на Unix — no-op (`bls/src/secret_store.rs`, по COVERAGE §2.3). Прежний: COVERAGE §2.3 («слабое место, находок нет»). `[LIKELY]`.
- Ход по плану 09-04 (INDEX): Э8 / П — `reject_insecure_mode` — ошибка не на Unix или явный unsupported
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э8 (гигиена). _Источник:_ history/PLAN.md §2 Э8

### R-100 · NIT · `decode_production_record` не проверяет диапазон `leader_index`
- Якоря: `decode_production_record` не проверяет диапазон `leader_index` (`extra_data.rs`); обезврежено сравнением с `expected` в `production_record_ok` (`application.rs`), вне карты комитета запись не проверяется. Прежний: COVERAGE §2.3, UNDERSTANDING §11.6. `[LIKELY]`. Связано: AUDIT B-10.
- Ход по плану 09-04 (INDEX): Э8 / П — B-10: `committee_index` обязателен; диапазон `leader_index` при декоде
- Связано: AUDIT B-10.
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э8 (гигиена). _Источник:_ history/PLAN.md §2 Э8

### R-107 · NIT · Комментарий
- Якоря: Комментарий `staking-reader/src/reader.rs` («KNOWN CONTRACT DRIFT — the fourth array has no contract-side counterpart … ends this handler with `write_returns(sdk, &(validators, keys, stakes))` — THREE arrays») устарел: контракт возвращает четыре массива (`consensus.rs`), нога `tombstoned` собирается в и документирована как намеренная . Предупреждение указывает на несуществующий риск. Новая. `[KNOWN]` обе стороны.
- Ход по плану 09-04 (INDEX): Э2 / С — комментарий «KNOWN CONTRACT DRIFT» уходит с ручным `sol!`
- **Статус 2026-09-09:** закрыта — `065003ad` (2.1: комментарий «KNOWN CONTRACT DRIFT» удалён; дрейфа нет — ветка слита `f16fdd90`, контракт возвращает четыре массива). _Источник:_ history/E2-ABI.md §2.4

### R-108 · NIT · `getEpochBlockInterval`, `getActiveValidatorsLength`, `getUndelegatePeriod` объявлены `returns (uint32)`
- Якоря: `getEpochBlockInterval`, `getActiveValidatorsLength`, `getUndelegatePeriod` объявлены `returns (uint32)` (`staking-reader/src/reader.rs`, интервал ещё раз в `node/evm.rs`), а контракт хранит и отдаёт эти поля как `u64` (`config.rs` — запись; — чтение). На проводе одно слово, значения по построению влезают в `u32` (сеттеры принимают `U32Command`, `config.rs `), так что ширину проверяет декодер alloy на узле, а не ABI контракта. Направление отказа безопасное, но одностороннее. Новая. `[KNOWN]` обе стороны.
- Ход по плану 09-04 (INDEX): Э2 / С — ширины из общего ABI
- **Статус 2026-09-09:** закрыта — `065003ad` (2.1: три вьюхи объявлены `uint64` по стороне контракта; `u32 → u64` разошлось на `epoch_transition.rs`, `dpos.rs`, `cert_inlet.rs`, `outer.rs`). Живьём проверены 1,5 вьюхи из 3 (E2-ABI §9). _Источник:_ history/E2-ABI.md §2.1, §9

### R-109 · NIT · Контракт считает, что узел не декодирует `EpochWeightsUnavailable`:
- Якоря: Контракт считает, что узел не декодирует `EpochWeightsUnavailable`:`events.rs ` — «the event is mute until an arm is added for it». Ветка есть: `node/evm.rs`. Все семь close-событий, объявленных узлом (`node/evm.rs`), имеют точные контрагенты по имени и типам полей в `events.rs`. Новая. `[KNOWN]` обе стороны.
- Ход по плану 09-04 (INDEX): Э8 / П — комментарий `events.rs` в контракте
- **Статус 2026-09-09:** не пересматривалась после 09-04; по плану — Э8 (гигиена). _Источник:_ history/PLAN.md §2 Э8

### R-121 · BLOCKER · Нулевое пересечение комитетов на границе останавливает цепь молча: уходящие паркуются на последнем блоке E без `PK_{E+1}`, входящие — на границе E−1 без `PK_E`; `SafetyHalt` не срабатывает, ERROR-строк нет
- Механизм: входящий комитет `committee[E+1]` без пересечения с `committee[E]` сковывает артефакт E+1 как его члены, но `PK_E` у него никогда не было — σ эпохи E он проверить не может и встаёт на первой обязательной высоте E; уходящий комитет держит только артефакт E, `PK_{E+1}` подтянуть некому (ремонтный sweep — R-122). Ни одна из половин не видит расхождения: `halted` пуст, `diverged` нет, обе стоят на разных высотах.
- Последствие: постоянная остановка сети на легитимной конфигурации — полная смена комитета проектно допустима (память проекта: «assume FULL committee turnover»); нигде не enforced ни в контракте, ни в узле. Девнет и смоуки этого никогда не ловили: два валидатора пиняются якорем и не ротируются, пересечение есть в каждом прогоне.
- Якоря: воспроизведение — `crates/dpos/consensus/src/testbed/tests.rs::a_zero_overlap_boundary_halts_the_chain_verify_only` (C7: N=8, `[0,1,2,3]` до эпохи 2, `[4,5,6,7]` с эпохи 3; уходящие стоят на 95, входящие на 63; 400 с виртуальных, таймаут); контрактная сторона — `contracts/staking/src/consensus.rs` (`commitEpochCommittee` выбирает состав из `target − 2`, пересечение не требует).
- Уверенность: `[KNOWN]` по воспроизведению на стенде (настоящая beacon-плоскость и `EpochTransition` над фейком стейкинга, прогон 2026-09-09 подтверждён независимо); `[LIKELY]` по тому, что второго пути доставки `PK_E`/σ в прод-узле нет (память проекта, 08-13/08-14; в этой работе не перечитывалось).
- Ход по плану 09-04 (INDEX): — (добавлена 09-09 по итогу Э3.2; кандидат в Э5 рядом с П-2 — σ-доставка по раунду; девнет-подтверждение Ex-19 — после Э4/П-4, 3.4 отложено 09-10; `[LIKELY]` «второго пути доставки `PK_E`/σ нет» стендом не снят).
- Связано: R-120 (принятый остаток той же конфигурации — там про улики, здесь про живость), R-122 (уходящей половине нечем подтянуть ключ), R-007, R-018; `EXPERIMENTS.md` §3 Ex-19.
- **Статус 2026-09-09:** открыта — воспроизведена тестом C7 (`ad9ccd02`), правки нет. _Источник:_ history/E3-2-STAND-4.md §3, §5.2
- **Статус 2026-09-13 (Э5 5.1, `129f2754` + `fca59634`):** **ЗАКРЫТА — и это центральная приёмка строки.** Не-член живой эпохи теперь приобретает артефакт (`acquire_mint_artifacts`), а член с долей и без артефакта просит его каждый тик (`beacon/actor.rs:2348-2355`, ханк открыл сам). Стенд-тест C7 ИНВЕРТИРОВАН: `a_zero_overlap_boundary_halts_the_chain_verify_only` → `a_zero_overlap_boundary_is_crossed_by_acquiring_the_other_halfs_key` — восемь узлов переходят границу, σ побайтно равны у всех восьми, `heights=[100 ×8] artifacts=[[2,3] ×8] halted=[]`. Старая форма теста упала прогоном (`[95,95,95,95,127,127,127,127]`) прежде, чем была переписана. _Источник:_ history/E5-1-A.md; history/E5-1-A-REVIEW.md

### R-122 · MODERATE · Догоняющий span поднимает фронтир эпох, не будя ремонтный sweep: выбывший из комитета узел без re-jump навсегда остаётся без `PK_E` живой эпохи
- Механизм: не-член на фронтире идёт в `soft_enter`, который спрашивает у `Randomness` только локальный `oracle_for`; сетевой pull артефакта живой эпохи не делает никто, а ремонтный sweep исключает фронтир по построению (`e < frontier`). У sweep ровно два будильника — приход границы и запись ключа в локальное хранилище. Ветка vote-backup, поднявшая `highest_entered_epoch`, зовёт только `reconcile_live`, который выходит сразу, если эпоха уже не живая. Итог: фронтир сдвинулся, ремонт не разбужен, ключа нет. Раньше стенд этого не показывал, потому что расписание отдавало `committee[E+2]` до контрактного коммита, span регистрировал будущую эпоху, её pull писал ключ и будил sweep — снятая подмена шага 5.
- Последствие: без re-jump узел паркуется на последнем блоке эпохи, где он ещё был членом (стенд: `[394,394,394,95]`), тихо. В проде маскируется steady-state re-jump'ом (`JUMP_THRESHOLD.min(interval)`): с ним узел прыгает и забирает артефакты (B2 зелёный). Re-jump подключён только при наличии upstream'а (`dpos.rs`, `upstream.as_ref().map(..)`) — узел без upstream'а выхода не имеет `[LIKELY]`. Tee живого фронтира не выход: в проде он вешается только при `--dpos.follower-upstream`, а для отставшего узла без исполненного блока на `read_at` откатывается на тот же финализированный хэш.
- Якоря: `crates/dpos/consensus/src/epoch_manager.rs` (будильники sweep — `sweep_wake.send_replace` после границы и по `key_n`; vote-backup → `reconcile_live` только при сдвиге пары `(observed, entered)`; `reconcile_live` — `if !self.is_live_epoch(epoch) { return }`; sweep — `candidates.iter().filter(|e| **e < frontier)`; `soft_enter` → `oracle_for`); `crates/dpos/consensus/src/dpos.rs` (`re_jump_threshold = JUMP_THRESHOLD.min(interval)`, re-jump только при `upstream`); `crates/node/src/dpos.rs` (tee только с `cert_inlet`, `committee_read_hash` fallback на `fin`).
- Уверенность: `[KNOWN]` по цепочке будильников (все якоря `epoch_manager.rs` прочитаны 09-09) и по обоим прогонам стенда (без re-jump — парковка, с ним — догон); `[LIKELY]` по «узел без upstream'а выхода не имеет».
- Ход по плану 09-04 (INDEX): — (добавлена 09-09). Минимальная правка — будить sweep после `handle_msg_for_unregistered_epoch`, если пара `(observed, entered)` сдвинулась; поведенческая, с «before»-тестом.
- Связано: R-018 (тот же класс — пробуждение зависит от одного источника), R-045 (signer-схема исключена из sweep), R-016 (порог re-jump; при абсолютном пороге маскировка ослабнет), R-121.
- **Статус 2026-09-09:** открыта — «before»-тест есть (`a_rotated_out_node_without_the_rejump_parks` — имя исправлено 09-10 по оценке `history/E3-REVIEW.md` §2, прежде было записано несуществующее `…_without_the_live_tee_parks`; `ad9ccd02`: парковка на 95 при `re_jump: None`), правки нет. _Источник:_ history/E3-2-STAND-4.md §5.1, §11
- **Статус 2026-09-12 (Э4 4.2 закрыта):** пересмотрена — спан удалён `48bf62ed` (4.2-В), фронтир sweep'а = `(live_epoch(tip), highest_entered)`; множество ПРОБУЖДЕНИЙ sweep'а не расширено ребром tip'а сознательно (Д-113: построено и измерено исполнителем — припаркованный узел чинит `PK_E` по сети и выходит из парковки БЕЗ прыжка, `heights [168,168,168,168]`, что снимает предпосылку трёх стенд-фикстур, одна — в `testbed/preconditions.rs`; откачено, реляция). Остаток открыт: узел без re-jump и без границ по-прежнему не будит sweep — отдельная работа с пересмотром трёх фикстур. _Источник:_ history/E4-2-V.md Д-113
- **Статус 2026-09-13 (Э5 5.1, `129f2754` + `fca59634`):** **закрыта.** Тот же механизм, что у R-121: ремонтный sweep по-прежнему исключает `epoch >= frontier` (это его замысел), но живую эпоху теперь обслуживают две ноги приобретения — членская в `drive_recompute` и не-членская в `acquire_mint_artifacts`. Тест C8 переписан: узел возвращается КЛЮЧОМ при закрытом гейте re-jump (`rejumps=[0,0,0,0]`). _Источник:_ history/E5-1-A.md

### R-123 · MODERATE · Комитетные чтения beacon-плоскости откатываются на финализированный хэш только при отсутствии заголовка; при бэкфилле reth «заголовок есть, состояния нет» чтение падает и `committee_for` молчит
- Механизм: `committee_read_hash` берёт `read_at = max(fin, live)` и пробует `provider.block_hash(read_at)` с `or_else(block_hash(fin))`. `block_hash` — заголовочная проба: при pipeline-бэкфилле заголовки уходят далеко вперёд исполненного состояния, проба отвечает `Some`, `or_else` не срабатывает, `epoch_committee_snapshot` читает состояние по этому хэшу → `StateNotMaterialized` → `.ok()?` → `None`. Для границы та же ловушка уже закрыта `executed_state_hash` с гейтом на `best_block_number()` (`executed.rs`, шапка модуля описывает ровно этот инцидент), но комитетные замыкания плоскости через него не ходят.
- Последствие: на всё окно бэкфилла `committee_for`/`committee_pair_for`/`committee_source` отвечают `None` вместо чтения на отстающем, но доступном финализированном состоянии — DKG-actor и vote-путь не видят комитет. Не фатально (не счётчик ошибок, а `None`), но это разница между «читаю старое» и «не читаю ничего».
- Якоря: `crates/node/src/dpos.rs` (`committee_read_hash`: `let read_at = fin.max(live)`; `provider.block_hash(read_at).ok().flatten().or_else(|| provider.block_hash(fin).ok().flatten())`); `crates/dpos/consensus/src/executed.rs` (шапка: «`provider.block_hash(n)` alone is a HEADER probe … runs FAR ahead of executed state»; `executed_state_hash`); `crates/dpos/staking-reader/src/reader.rs` (`StateNotMaterialized`).
- Уверенность: `[KNOWN]` — оба якоря прочитаны 09-09, тип ошибки подтверждён контр-ревью до `reader.rs`; на стенде не воспроизводится (у `FakeChain` нет заголовков без состояния) — только чтение кода.
- Ход по плану 09-04 (INDEX): — (добавлена 09-09). Правка — заменить голую `block_hash` на `executed_state_hash` с тем же `or_else` на `fin`; кандидат в Э4/Э5 рядом с R-006.
- Связано: R-006 (чтение канонического хэша до FCU — тот же класс «заголовок ≠ состояние»), R-001/R-004 (окно после прыжка — когда бэкфилл и происходит), R-122.
- **Статус 2026-09-09:** открыта — только по коду. _Источник:_ history/E3-2-STAND-4.md §1, §10б #6
- **Статус 2026-09-11:** закрыта — `5451f5ab` + `0f9be693` (Э4 4.1 Б1: чтения плоскости идут через модуль на ИСПОЛНЕННОМ якоре — `RethAnchor::executed_hash` через `executed_state_hash`, «заголовок есть, состояния нет» ⇒ `Ok(None)` ⇒ `NotReadable` (транзиент), не падение; транзиентные ошибки провайдера классифицируются `classify_transient_provider_error`, не `Backend`). Пинуется юнитами `committee/tests.rs`; стенд плечо `Ok(None)` не достигает (`StandAnchor` ≤ исполненного tip'а — `E4-1-B3.md` §6). _Источник:_ history/E4-1-B1.md часть В (B1-01, Д-21), E4-1-B3.md

### R-124 · MODERATE · `EpochTransition` при многоэпохном догоне паркует две границы подряд: `debug_assert!` «two boundaries pending at once» в отладке, в релизе вторая граница затирает первую
- Механизм: однослотовая парковка границы `last(E)` до материализации; при `interval ≤ MAX_PENDING_ACKS + K` (= 16 + 3 = 19) на догоне вторая `last(E+1)` приходит раньше, чем первая материализована. Комментарий у ассерта называет старое обоснование `interval > MAX_PENDING_ACKS + result_lag` снятым — по факту оно и было условием однослотовости.
- Последствие: для продакшена (`EPOCH_LENGTH_BLOCKS ≫ 19`) не живая дыра; для стенд-фикстур догона с короткой эпохой — молчаливая потеря epoch handoff в релизной сборке и паника в отладочной.
- Якоря: `crates/dpos/staking-reader/src/epoch_transition.rs:426` (ассерт), `outer.rs:236` (`MAX_PENDING_ACKS`), `order_block.rs:20` (`K`).
- Уверенность: `[LIKELY]` — реляция исполнителя F11 (4.2-В третий проход): воспроизведено им дважды на фикстуре `epoch_len = 5` (парк 9/14), с `epoch_len = 24` не воспроизводится; оркестратор ассерт открыл (`:426`), прогон не повторял.
- Ход по плану: — (добавлена 09-12). Правка — явный инвариант `interval > MAX_PENDING_ACKS + K` на старте ИЛИ очередь вместо однослотовой парковки; кандидат в отдельный тикет / Э5 (ET).
- Связано: Д-119 (`history/E4-2-V.md`), RULE 46 (`.claude/dpos_architecture/13`).
- **Статус 2026-09-12:** открыта — только по реляции. _Источник:_ history/E4-2-V.md Д-119

### R-125 · MINOR · `prune_agreements` подметал полосу из `SCHEME_RETENTION_EPOCHS` партиций на КАЖДОМ финализированном блоке
- Механизм: арм `committee_readable` epoch_manager'а реконсилирует живую эпоху на каждом `anchor_advanced` (публикация всегда, один на дерайв), `reconcile_roles` безусловно зовёт `abort_below` ⇒ `prune_agreements` ⇒ band-sweep `cutoff−8..cutoff` по `Storage::remove` (каждый — `remove_dir_all` под глобальным мьютексом рантайма). Существовало и ДО 4.2-В; ревью R9 приписало частоту новому ребру tip'а (V-02), третий проход нашёл настоящий корень (Д-117).
- Якоря: `epoch_manager.rs` `prune_agreements` (гейт `cutoff > swept_to || aborted_one`, мемо `agreements_swept_to`), арм `committee_readable`, `committee/store.rs` `anchor_advanced`.
- Уверенность: `[KNOWN]` — оркестратор открыл арм, публикацию и `prune_agreements` до и после правки.
- **Статус 2026-09-12:** закрыта — `48bf62ed` (4.2-В: band-sweep стал edge-driven — только при росте cutoff или прерванном инстансе в том же вызове; ребро tip'а реконсилирует только при СМЕНЕ live-эпохи, мемо `tip_reconciled_live`). Остаток Д-117б: вызов с cutoff'ом ниже наибольшего и пустым `stale` полосу не подметает (по построению безвредно). Величина сэкономленного не измерена. _Источник:_ history/E4-2-V.md Д-116, Д-117

### R-126 · MINOR · BEACON-ingress роняет кадр эпохи `now+3`: отставший по `last_height` узел может терять DKG-трафик соседей
- Механизм: `epoch_is_actionable` = `[now, now+2]` ∪ идущие церемонии, `now = epoch_of(height_now())`; узел, чей `last_height` отстал на эпоху, видит кадры соседей для `now+3` и отбрасывает их с `reason = "epoch"`. Дилинги лечит дилерский ретрансмит на каждом pre-seal тике; конфирмации — переиздание `mint` по росту ширины (≤ одна эпоха).
- Якоря: `crates/dpos/consensus/src/beacon/actor.rs` (`epoch_is_actionable`, `on_message`); ретрансмит — там же (`:1255-1262` по реляции).
- Уверенность: `[KNOWN]` механизм по коду (оркестратор открыл `epoch_is_actionable`); величина разброса `last_height` под нагрузкой — не измерена.
- Ход по плану: — (добавлена 09-12, Д-126). Замер разброса на девнете; при необходимости — расширение окна.
- **Статус 2026-09-12:** открыта — по коду; в прогонах B1/B2/B3/C9 и под разрезом сети не проявилась. _Источник:_ history/E4-3-A.md Д-126, часть В P-07

### R-127 · MINOR · Два источника классификации отправителя на BEACON: `GatedReceiver` по `TrackedWindow`, `beacon_member` по `committee_for`
- Механизм: окно `TrackedWindow` пишет `EpochTransition` по `fin`, актор живёт по `fin + K` — расхождение ≈ K блоков в начале эпохи, плюс два независимых читателя одних записей; расхождение никем не ловится, отброс неотличим от легитимного в метрике.
- Якоря: `crates/dpos/p2p/src/lib.rs` (`TrackedWindow`), `crates/dpos/consensus/src/beacon/actor.rs` (`beacon_member`), `beacon/plane.rs` (сборка `DkgActor` — окно туда не протащено, файл вне списка захода).
- Уверенность: `[KNOWN]` по коду (Д-123).
- Ход по плану: — (добавлена 09-12). Один параметр `ValidatorInputs` — Э5.
- **Статус 2026-09-12:** открыта. _Источник:_ history/E4-3-A.md Д-123, часть В P-07/P-08

### R-128 · SERIOUS · Постоянный отказ чтения комитета в окне не даёт `Fault::corruption` и не кэшируется — проект §5.4 (стр. 555) требовал громкой остановки
- Механизм: `CommitteeStore::failed` считает метрику, пишет `error!` ОДИН раз на эпоху и возвращает `CommitteeError::Read(permanent)` (`committee/store.rs:192-207`); отказ намеренно не кэшируется (`:76-88`) и пере-выводится каждым вызовом (два staticcall'а на горячем пути, в т.ч. из `CertProvider::scoped` на задаче marshal'а). Ни один потребитель не превращает его в отказ узла: фасад сворачивает в `None` (`committee/facade.rs:47-54`), `epoch_manager::reconcile_roles` — `debug!` + `return` без `deferred_reconciles` (`epoch_manager.rs:1141-1157`), `deliver` — отброс с `true`, inlet — отложение, слэшер — release charge. Все валидаторы читают один контракт ⇒ коррелированное тихое неучастие всей сети под зелёным liveness-чеком.
- Последствие: §5.4 стр. 554 (ревёрт ⇒ `error!`, повтора нет) выполнена буквально; стр. 555 (`AbiDecode`/дубликаты/`weights: None` ⇒ `Fault::corruption`, узел стоит) — НЕ выполнена, и подмена не записана как Д-nn в 4.1 (Ex-4.1e фиксирует «один `error!`, цепь ниже жива» как поведение, R-035 смягчена). Плюс B3-12: отсутствие отрицательного кэша.
- Якоря: выше; проект `history/E4-CORE-DESIGN.md:554-556`.
- Уверенность: `[KNOWN]` — оркестратор открыл `store.rs:186-210`, `:74-90`, `facade.rs:44-56`, `epoch_manager.rs:1138-1160` и строки проекта 2026-09-12 по оценке `E4-REVIEW.md` §9.1.
- Ход по плану: — (добавлена 09-12). Решение владельца: (а) реализовать стр. 555 — `!is_transient()` для эпохи в окне ⇒ `Fault::corruption` из `epoch_manager` + «отравленный» слот в сторе (кэш отказа); (б) ратифицировать текущее поведение как Д-nn и переписать стр. 555. Тест-потребитель «эпоха в окне отказала постоянно ⇒ узел встал громко» отсутствует в обоих случаях.
- Связано: R-035, R-042, B3-12, R-129.
- **Статус 2026-09-12:** **закрыта — `26609cff`** (журнал `history/R-128-FIX.md`).
  Постоянный класс разделён в слое, который владеет ошибкой: `ReadError::class()` — один
  исчерпывающий `match` на `Transient` / `Permanent` (`CallReverted`, `Backend`) /
  `Impossible` (остальные девять, включая `AbiDecode`, который модуль строит для
  `weights: None` в окне, длины весов, неуникальных ключей и повторного чтения с другим
  значением); `is_transient()` и новый `is_contract_impossible()` — виды на него.
  `CommitteeStore` ОТРАВЛЯЕТ слот эпохи на «невозможном» ответе (`Poison{error, reason}`,
  проверяется до записи, пруним тем же полом окна, пишется и форк-плечом `install`), так
  что следующий `committee(E)` повторяет вердикт без staticcall'а, счётчик тикает,
  `error!` один — **B3-12 закрыт для этого класса**; ревёрт намеренно не отравляется.
  `epoch_manager::reconcile_roles` — ЕДИНСТВЕННАЯ точка, где чтение комитета становится
  остановкой узла: «невозможный» отказ эпохи, которую reconcile был обязан войти, ⇒
  `error!` + `SafetyHalt::engage(SyncReason::ContractFork)` (одиннадцатый вариант) и
  выход; ревёрт — `debug!` + выход, как было. Фасад не тронут, формы `CommitteeError` для
  потребителей не менялись. Стенд: Ex-4.1e переписан под halt (красный на HEAD), плюс
  новая фикстура ревёрта `StandConfig::reverts_for`. Остаток: после отравления эпоха
  теряет и схему, то есть marshal перестаёт верифицировать её сертификаты (журнал §5), и
  R-129 на этой эпохе становится достижимее. _Источник:_ history/E4-REVIEW.md §9.1,
  history/R-128-FIX.md

### R-129 · MODERATE · На marshal-резолвере «не могу аутентифицировать эту эпоху» (`scoped ⇒ None`) отвечает `deliver == false` и исключает честного пира навсегда — правило «`false` только на лжи» доведено только до FRONTIER
- Механизм: CW `verify_delivered` при `provider.scoped(epoch) == None` оставляет элемент непроверенным и отвечает `send_lossy(false)`; резолвер на `false` — `block!` + `fetcher.block(peer)`, `excluded` не очищается (`.claude/COMMONWARE_INTERNALS.md:236` «Scheme `None` … inside `verify_delivered` → `false` → retry + honest peer penalized»; чекаут по реляции оценщика: `marshal/core/actor.rs:1085-1115`, `resolver/p2p/engine.rs:425-441`, `fetcher.rs:509-517`). `scoped` = `Committee::scheme` (`outer.rs:287-289`), `None` на любом отказе модуля (`store.rs:517-524`): вне окна, ниже `commit_height`, постоянный отказ (R-128), несобранная verify-схема. На FRONTIER тот же случай закрыт явно (`plane_upstream.rs:467-490`, 4.2-Б2 A2-04); §5.2/§5.4 про MARSHAL-канал молчат.
- Достижимость: в установившемся режиме marshal просит высоты в окне (tip ≤ `last(epoch(fin)+2)`, `commit_height(E) = start(E−2)` ≤ anchor) — основной путь сюда через R-128 (форк контракта) и «запись читается, схему собрать нельзя» (V1 §4 п.5, 4.2-В).
- Уверенность: `[LIKELY]` — класс подтверждён по `COMMONWARE_INTERNALS.md:236` и `store.rs:517-524` (оркестратор), цепочка резолвера — по реляции оценщика; стенд-фикстуры нет.
- Ход по плану: — (добавлена 09-12). Сначала стенд-тест «узел получает финализацию эпохи, схемы которой у него нет» с ассертом на `requests_created`/`excluded`; затем либо `CertProvider::scoped` не молчит, либо свой `Consumer`-адаптер над marshal-handler'ом, как на фронтире. Кандидат в Э5 5.2/5.4.
- **Статус 2026-09-12:** открыта — по коду и докам CW. _Источник:_ history/E4-REVIEW.md §9.2
- **Статус 2026-09-13 (Э5 5.0а, `7790d1bc`):** фикстура ЧАСТИЧНО построена, сам тест остаётся 5.2. Что появилось: источник сертификатов БЕЗ гейта `deliver` — `CertInletSource::PeerArchive` читает архив донора напрямую через `FrontierMarshal::pair_at` (`plane_upstream.rs:256`), и на нём узел получает финализации эпох, схем которых у него нет: `defers = 32` при `rotations = 0` (`testbed::cert_inlet_tests::a_donors_archive_hands_the_inlet_an_epoch_it_cannot_read_and_it_defers`, прогон оркестратора 09-13). Это доказывает, что путь «схему собрать нельзя» достижим в стенде. Чего не хватает тесту R-129: наблюдаемых MARSHAL-резолвера (`requests_created`/`excluded`), пути отказа, несущего ПРИЧИНУ (сегодня только процессный `dpos_frontier_dropped_total{reason}`), и второго честного пира — иначе «исключён навсегда» неотличимо от «пиров больше нет». Попутно: ветка defer недостижима через ПЛОСКОСТЬ по двум независимым причинам, обе проверены оркестратором — `deliver` шаг (5) дропает этот класс (`plane_upstream.rs:486-490`), а by-height-фидер по арифметике окна (`committee/store.rs:218`, `committee/mod.rs:616`) не даёт эпоху выше `epoch(anchor)+1`. _Источник:_ history/E5-0a-A.md §0; history/E5-ORCHESTRATOR.md

### R-130 · MODERATE · Геометрия процесса существует в двух независимо прочитанных копиях (`geometry_rx` модуля и `OriginEpocher` слоя), а peer-set — из второго читателя комитета; проверка размера реестра снята
- Механизм: слой читает `dpos_activation_block`/`epoch_block_interval` сам (`consensus/dpos.rs:1745-1748`) и строит `OriginEpocher` для executor/движков (`outer.rs:722`), тогда как модуль/beacon/фронтир берут замороженную пару ET через `geometry_rx`; дока модуля утверждает единственность (`committee/mod.rs:388-398`). При живых сеттерах `setEpochBlockInterval`/`setDposActivationBlock` расхождение заставит шаг (3) `deliver` считать честные ответы ложью. Peer-set (`TrackedPeers`) собирает `EpochTransition` из своих `epoch_committee_snapshot` (Д-122), а не из записей модуля, и именно он теперь решает, чьи кадры декодируются (4.3). `check_peer_set_size` считает только primary (`epoch_transition.rs:788`; на `1b11b61f:654` — объединение с реестром), CW secondary не проверяет вовсе — граница размера реестра потеряна.
- Уверенность: `[KNOWN]` — оркестратор открыл `dpos.rs:1740-1750`, `outer.rs:718-726`, `mod.rs:388-398`, `epoch_transition.rs:788`.
- Ход по плану: — (добавлена 09-12). (а) отдать слою ту же `geometry_rx`, снять `OriginEpocher` из `outer.rs` (П-10); (б) Д-122 — `Cargo.toml` инверсия или тест-пин «маска ET == записи модуля» на двух якорях; (в) вернуть проверку размера для secondary (`MAX_REGISTRY_PEER_SET`).
- Связано: Д-122, Д-123/R-127, R-126, П-10.
- **Статус 2026-09-12:** открыта — по коду. _Источник:_ history/E4-REVIEW.md §9.3

## §2 K-1..K-40 (контракт) и группы дублей

Тяжесть — по `history/AUDIT-CONTRACT.md` (09-04); пути — `contracts/staking/src/` (контракт с 09-09 в этом дереве, слияние `f16fdd90`). K-3 = R-111. Часть T AUDIT-CONTRACT (тесты как ложная уверенность, T-1..T-8): T-1 (заглушка `verify` всегда `true`) ушла с 1.10; T-5 (эмуляция самовызова) — с 1.7; T-6 закрыта тестом `handlers_refuse_value_and_refuse_to_mutate_inside_a_static_frame` (E1-8-TESTS F-5); T-7 частично (F-11); остаток — `history/E1-8-TESTS.md` §3. Вердикты «не дефект» V-15 и V-19 (`history/AUDIT-CONTRACT.md` часть V) опирались на то, что КB-2 уберёт историю видимости целиком; 1.1 (`c31c258f`) её не убрала (X4) — оба вердикта требуют перепроверки. F7 (`history/E1-CLOSEOUT.md`): `getValidatorOwner(address)` и `getGovernance()` в ABI отсутствуют — владелец читается только перебором `getValidatorByOwner`, адрес governance с цепи не читается вовсе; открыта, проверено 09-09 (`grep` по `consts.rs`).

| № | Тяжесть (AUDIT-CONTRACT) | Суть | Файл (`contracts/staking/src/`) | Ход по плану 09-04 | **Статус 2026-09-09** | Источник |
|---|---|---|---|---|---|---|
| K-1 | SERIOUS | governance через `setBlsVerifier` + `setSlashFundAddress` получает право tombstone'ить любого валидатора и забирать самостейк | `config.rs`, `consensus.rs` | Э1 / С+ — КB-4 убирает слэш по улике; сеттер верификатора после init удалить (1.3); остаток — поддельный PoP при регистрации (дыра §6) | смягчена — `f70ceffe` (1.10: верификатор инлайн, `setBlsVerifier` удалён), и это ЕДИНСТВЕННАЯ опора: подделать улику governance больше нечем, право «tombstone'ить любого» с этого маршрута снято. Таймлок 1.3 (`b237a81e` → `0f283a82`) к K-1 отношения не имеет — он стоит на `setBlendReserve`, которого в механизме K-1 нет, а на `setSlashFundAddress` таймлока нет по решению 09-11 (асимметрия, `DECISIONS.md` §3). Остаток, названный 2026-09-11 по `history/E1-CONTRACT-REVIEW.md` F-3: (1) **MINOR** — governance одним немедленным вызовом переводит `slashFundAddress` на себя (`config.rs` `set_slash_fund_address` пишет сразу) и забирает конфискат уже состоявшегося слэша (`consensus.rs` `seize_self_stake` читает поле на месте); наказание при этом не фабрикуется — `slashEquivocation` только системный, маршруты улик проверяют подписи инлайн, — governance перенаправляет чужие деньги, а не создаёт повод; (2) подменяемость сужена до механизма обновления предеплоев `runtime-upgrade` (R10.1) | history/PLAN.md 1.10; history/E1-REFLECTION.md R10.1 |
| K-2 | SERIOUS при общем chain_id | улика эквивокации не привязана ни к комитету, ни к эпохе, ни к развёртыванию — только к `chain_id`; сеть-двойник даёт улику против честного | `consensus.rs` | Э1 / С — КB-4; если улики остаются — namespace с genesis hash, `committee_member_at`, `activation_epoch` (Д-4) | смягчена — Д-4 (09-04): `chain_id` уникален для каждого развёртывания ⇒ MINOR; удаление маршрута улик (1.2) не сделано, Д-4 под вопросом с 09-07 | history/DECISIONS.md Д-4; history/PLAN.md 1.2 |
| K-3 | BLOCKER | = R-111 | `consensus.rs`, `staking.rs` | Э0 / С — КB-1 (0.2) | = R-111: снята решением 1.0 (`c31c258f`) — ниже пола отказ, остановка принята | history/PLAN.md 1.0 |
| K-4 | MODERATE | после полного выхода владелец возвращается с самостейком ниже `minValidatorStakeAmount` и активируется | `staking.rs` | Э1 / С — КB-5 (1.4) | отложена — 1.4 (КB-5), открыта | history/PLAN.md 1.4 |
| K-5 | MODERATE | владелец не может забрать последний `minValidatorStakeAmount`, пока есть хоть один делегатор; комментарий утверждает обратное | `staking.rs` | Э1 / С — КB-5 (1.4), Д-11 | отложена — 1.4 (КB-5), открыта; решение Д-11 перекрыто 1.0 | history/PLAN.md 1.4 |
| K-6 | MODERATE | изменение cap до активации не влияет на первые три комитета (sel = 0), live-view видит новое сразу | `config.rs`, `consensus.rs` | Э1 / С — КB-2 (1.1) | закрыта — `c31c258f` (1.1: чекпойнты cap удалены, кап читается живьём) | history/PLAN.md 1.1 |
| K-7 | MINOR | история видимости глубиной 3 + отстающий коммит = ложный «невидим» и `CommitteeTooSmall` | `staking.rs` | Э1 / С — КB-1 + КB-2 | открыта — 1.1 её НЕ закрыла (история видимости в три перехода на месте; E1-REFLECTION D1); недостижима, потому что узел коммитит на каждом блоке [LIKELY]; исход после 1.0 — отказ на полу | history/E1-REFLECTION.md D1, X4 |
| K-8 | MINOR | доля награды делегатора по стейку E, вес места по E−2 | `staking.rs` | Э1 / П — экономика (Д-10) | закрыта — `50e87d33` (1.9б: доли делегаторов по снимку эпохи отбора E−2, `selection_epoch_for`) | history/PLAN.md 1.9; history/E1-CLOSEOUT.md |
| K-9 | MINOR | остатки округления оседают на контракте без учёта | `staking.rs` | Э1 / П — экономика (Д-10); при КB-3 поле `unpaid_liabilities` | не пересматривалась после 09-04; Д-10: при принятой форме пыль не несущая (покрытие считается по резерву) | history/DECISIONS.md Д-10 п.2 |
| K-10 | MODERATE | после конфискации инвариант `total = Σ делегаций` нарушен для прошлых эпох; доля самостейка невостребуема; JAIL-владелец получает комиссию | `consensus.rs`, `staking.rs` | Э1 / П — экономика (Д-10) | закрыта — `100c02c4` (первая половина: невостребуемое начисление не списывается с резерва) + `50e87d33` (1.9а: комиссия tombstoned-владельца гасится). Остаток X1: гейт срабатывает только на уже поставленный тумбстоун, `claimValidatorFee` беспермиссионный — до вердикта комиссию выводит любой | history/PLAN.md 1.7, 1.9; history/E1-REFLECTION.md X1 |
| K-11 | MODERATE / SERIOUS для системы | системный слэш доверяет байту `accused`; в узле голосующий без `committee_index` голосует `true` без проверки | `consensus.rs`; `application.rs` | Э1/Э8 / С — КB-4 оставляет один маршрут, B-10 убирает `Option<committee_index>` | смягчена — бай-пас на узле (`committee_index == None` ⇒ `true`) недостижим по коду (DECISIONS п.10); контракт байт `accused` по-прежнему не проверяет; B-10 — Э8; при КB-4 гейт узла — единственная защита | history/DECISIONS.md Д-4 п.10; history/PLAN.md Э8 |
| K-12 | MINOR | успех ERC-20 по пустому ответу и одному байту `bool` | `util.rs` | — / — — дыра: поведение BLEND не известно (D-8) | не пересматривалась после 09-04; поведение BLEND неизвестно (D-8 не ставился) | history/PLAN.md §6 |
| K-13 | MINOR | ABI-декодер молча усекает целые и игнорирует хвост | `util.rs`, SDK `crates/codec` | Э1 / П — строгий декодер в SDK (1.6) | смягчена — обёрткой `util::decode` на статическом пути (`9cd156db`, 1.6, 09-11): она сверяет пере-кодированные байты со входом. Форма статуса поправлена 2026-09-11 по `history/E1-CONTRACT-REVIEW.md` F-19: «закрыта на статическом пути» формой `REGISTER.md` §0 не является, а по существу записи динамический путь открыт. SDK НЕ правился: строгость ломает перебор версий `InitialSettings` (`sdk/src/universal_token/storage.rs:167,179`) и ещё четыре места — список в журнале §6. Остаток открыт: `decode_args` (динамика — `initialize`, три маршрута улик). По замеру усекались целые ВНУТРИ слова и принимался хвост; короткий буфер отвергался и до правки | history/E1-CONTRACT-2.md §2 П8 |
| K-14 | MINOR/NIT | `StorageVec::grow_checked` не обнуляет элемент; старые слоты после `clear_checked` | `crates/sdk/src/storage/vec.rs` | — / — — на свежем состоянии не про что; дисциплина «писать все поля при grow» | снята — на свежем состоянии не про что (PLAN §7); дисциплина «писать все поля при grow» остаётся | history/PLAN.md §7 |
| K-15 | MINOR | `settleEpochStipend` ревёртит `EpochNotAccrued` для `current−1` до закрытия; узел его не вызывает | `staking.rs` | Э1 / С — КB-3 (Д-10) | закрыта — `100c02c4` (1.7: оба `settleEpochStipend*` удалены) | history/PLAN.md 1.7 |
| K-16 | MINOR | `initialize` принимает cap 1..3 — первый коммит всегда ревёртит | `config.rs` | Э1 / П — cap ≥ 4 при init (1.6) | закрыта — `5386a635` (1.6, 09-11): тот же гейт `cap < MIN_COMMITTEE_LENGTH` в `validate_initialization`, та же ошибка и та же нагрузка, что у сеттера; дизъюнкт `cap == 0` убран как недостижимый. Тест `initialize_refuses_a_committee_cap_below_the_floor` красный без гейта | history/E1-CONTRACT-2.md §2 П1 |
| K-17 | MINOR | два view «выборка на эпоху» дают разные списки; ни один не читается узлом | `consensus.rs` | Э1 / С — КB-2 / КB-6 | смягчена — `c31c258f` (1.1: один алгоритм отбора) + `2fa46f1b` (1.5); `getValidators`/`isValidatorActive` оставлены для губернатора и отдают нефильтрованный отбор (X5) | history/PLAN.md 1.1, 1.5; history/E1-REFLECTION.md D2, X5 |
| K-18 | MINOR | view без `ensure_initialized` падают `IntegerDivisionByZero` | `staking.rs` | Э1 / П — 1.6 | закрыта — `548f10bd` (1.6, 09-11). Падающих вьюх было ровно две, обе в `staking.rs`: `get_validators` и `get_validator_delegated_stake_at` (замер пробником, `history/E1-CONTRACT-2.md` §2 П2); `config.rs` не при чём — её 13 вьюх не считают ничего, её 12 сеттеров закрыты `ensure_governance`→`ensure_initialized` (`util.rs:60`), и на эту цепь теперь есть тест | history/E1-CONTRACT-2.md §2 П2 |
| K-19 | NIT | `as u32` без проверки: `stamp`, `record`, `len` | `consensus.rs` | Э8 / П — недостижимо; `try_into` | не пересматривалась после 09-04; Э8 | history/PLAN.md Э8 |
| K-20 | NIT | события при init сообщают выдуманные «предыдущие» значения | `config.rs` | Э8 / П — — | не пересматривалась после 09-04; Э8 | history/PLAN.md Э8 |
| K-21 | NIT | `min_undelegate_blocks` без сеттера ограничивает закрытые сеттеры | `config.rs` | Э1 / С — КB-6 | открыта — 1.5 оставила `min_undelegate_blocks` (поле `InitializeCommand`, смена селектора `initialize`); после Э2 общий источник снимает причину не удалять. Столбец «Закрывает» 1.5 расходится с текстом ячейки (отчёт §4) | history/PLAN.md 1.5 |
| K-22 | MINOR | отказ фонда оставляет конфискат на контракте без пути вывода; событие `seized = 0` | `consensus.rs` | Э1 / П — 1.6 | закрыта — `3c4560db` (1.6, 09-11), пересмотрено `52714f62` (09-11): отказ настроенного фонда не ревёртит, а переводит тот же конфискат на `EQUIVOCATION_BURN_SINK`; тумбстоун/тюрьма/удаление из активных/штамп невидимости стоят, событие несёт фактического получателя. Ревёрт `ERR_STAKING_TOKEN_CALL_FAILED` остался только на отказе ОБОИХ получателей. Принятая цена — `DECISIONS.md` §3 «K-22» | history/E1-CONTRACT-2.md §2 П3; history/E1-CONTRACT-REVIEW.md F-1 |
| K-23 | SERIOUS при подтверждении / NIT | личность в слэше через `compressG2Unchecked`; точка вне кривой с тем же x даёт тот же ключ | `consensus.rs` | Э1 / С+ — КB-4 снимает экспозицию слэша; для PoP остаётся дырой (верификатора нет в дереве, D-4) | снята — гипотеза опровергнута чтением верификатора 09-04 (PAIRING отвергает точку вне кривой; `_rejectInfinity` явно); после 1.10 верификатор инлайн | history/DECISIONS.md Д-4 п.9; history/AUDIT-CONTRACT.md K-23 |
| K-24 | MINOR | `kick_count` никогда не убывает | `liveness.rs` | Э1 / П — политика (Д-10) | закрыта — `50e87d33` (1.9в: сброс `kick_count` через 30 эпох с последнего провала). Остаток — решение Д-13 (потолок лестницы 128 недостижим) | history/PLAN.md 1.9, §5 Д-13 |
| K-25 | MODERATE | liveness-исключение — оружие большинства предложенцев против честного (кто пишет `leader_index`, тот судит) | `liveness.rs` | — / — — граница проекта; узел проверяет `leader_index == expected` (`application.rs:227-235`); B-10 | снята — граница проекта (узел проверяет `leader_index == expected`); B-10 — Э8 | history/INDEX.md §2 |
| K-26 | MINOR | `MAX_SETTLE_CATCHUP = 4` замораживает все награды, пока курсор отстаёт | `staking.rs` | Э1 / С — КB-3 (Д-10) | закрыта — `100c02c4` (1.7: курсор и `MAX_SETTLE_CATCHUP` удалены). Цена: отсрочка стала форфейтом, форфейт — гриферским (W2, F-16) | history/PLAN.md 1.7; history/E1-REFLECTION.md W2 |
| K-27 | NIT | реентрантность через верификатор — двойной `apply_equivocation_penalty` с двумя наборами событий | `consensus.rs` | Э1 / С — КB-4 | снята — реентрантность через верификатор исчезла вместе с внешним верификатором (`f70ceffe`) | history/PLAN.md 1.10 |
| K-28 | MINOR | `getValidatorStatus` показывает стейк на максимальную материализованную эпоху | `staking.rs` | Э1 / С — КB-6 | открыта — `getValidatorStatus` оставлен 1.5 (стенд `dpos_harness/core/nodes.py`). Столбец «Закрывает» 1.5 расходится (отчёт §4) | history/PLAN.md 1.5 |
| K-29 | MINOR | кольцо весов на 16 эпох: закрытие позже 13 эпох теряет вердикты и стипендию | `consensus.rs`, `liveness.rs` | Э1 / П — достижимо только без `recordProduction` >13 эпох; документировать | не пересматривалась после 09-04; документировать (кольцо 16 эпох) | history/INDEX.md §2 |
| K-30 | MINOR | roster растёт навсегда; `commitEpochCommittee` и `stamp` линейны по нему | `staking.rs` | Э1 / С — КB-2 | закрыта — `c31c258f` (1.1: `selection_roster` удалён) | history/PLAN.md 1.1 |
| K-31 | NIT | `setDposActivationBlock` допускает `value == block_number` | `config.rs` | Э2 / С — сеттер уходит с B-9 | отложена — Э2.4 (сеттер уходит с B-9) | history/PLAN.md 2.4 |
| K-32 | NIT | претензия владельца permissionless: любой двигает `claimed_at` | `staking.rs` | Э1 / С — КB-6 | открыта, и усилена — после 1.7 беспермиссионная претензия стала гриферским вектором (опустить резерв под пот ⇒ эпоха сгорает; W2); живьём 09-08 (прогон 7), запинена e2e `a_claim_between_two_closes_…` (F-16). Столбец «Закрывает» 1.5 расходится (отчёт §4) | history/E1-CLOSEOUT.md прогон 7; history/E1-8-TESTS.md F-16 |
| K-33 | MINOR | делегатор не может вывести ничего одну эпоху после любой делегации | `staking.rs` | Э1 / П — UX (Д-10) | не пересматривалась после 09-04; UX (Д-10 не касается) | history/DECISIONS.md Д-10 |
| K-34 | вопрос | семантика `fuel: None` на хосте | `util.rs`, `crates/sdk/src/system.rs` | Э1 / С — КB-3 снимает self-call | открыта, расширилась — самовызов удалён (1.7), но `fuel: None` теперь в пяти местах (`util.rs` ×3, `bls.rs` ×2; D11); замер F1 (`EXPERIMENTS.md` §1): горелка в слоте резерва съедает почти весь бюджет системного вызова, блок выживает на правиле 63/64 — запинено `the_fuel_burning_read_against_the_production_system_call_budget` (F-14) | history/E1-REFLECTION.md D11; history/E1-CLOSEOUT.md F1; history/E1-8-TESTS.md F-14 |
| K-35..K-39 | — | проверено, не находки (запас кольца, `u32`-счётчики, реентрантность через токен, `ensure_non_payable` везде) | — | — / — — — | не находки (проверено 09-04) | history/AUDIT-CONTRACT.md |
| K-40 | NIT | адрес валидатора — произвольный параметр `registerValidator`; чужой адрес можно занять навсегда | `staking.rs` | Э8 / П — требовать подпись адреса или `validator = caller` | не пересматривалась после 09-04; Э8 | history/PLAN.md Э8 |
### 14 групп величин, продублированных на границе узел↔контракт (DUPLICATES)

Источник статуса: `history/E2-ABI.md` §1 (09-09); постановка — `history/DUPLICATES.md` (09-04). «Закрыта компилятором» — обе стороны импортируют одно объявление, односторонняя правка не собирается; «закрыта тестом» — импорт селектора плюс тест на форму.

| # | Величина | Где стало | Чем удерживается | **Статус 2026-09-09** |
|---|---|---|---|---|
| 1 | Суффиксы namespace подписи (`_NOTARIZE/_NULLIFY/_FINALIZE`) + префикс + кодировка chain_id | — | — | отложена — Э2.3; исчезает с `evidence.rs` при Д-4 = КB-4 (R-115) |
| 2 | Индексное пространство комитета (сортировка по peer-ключу) | — | — | отложена — Э2.3; единственный оставшийся «MUST mirror» (`bls/src/scheme.rs`) (R-116) |
| 3 | ABI-сигнатуры системных вызовов и вьюх | `crates/staking-abi` (`fluentbase-staking-abi`, один `sol!`) | компилируемый импорт с обеих сторон; свидетели `selectors_match_the_deployed_artefact_scan`, `derived_selectors_match_independent_hex_pins` | закрыта — `9b6213be`+`065003ad` (R-114) |
| 4 | Арность/форма возврата `getEpochCommitteeWithStakes` | общий `sol!`; контракт кодирует ответ своим кодеком | **тест** `the_view_returns_decode_under_the_node_s_declaration` (мутация проверена) + `epoch_committee_return_matches_the_contract_abi_encoding` | закрыта тестом, не компилятором — `0972059d` поверх `065003ad` (R-119) |
| 5 | `MIN_COMMITTEE_LENGTH = 4` | `staking_protocol::MIN_COMMITTEE_LENGTH` | `pub use` обеих сторон | закрыта — `9b6213be` (R-117) |
| 6 | `BALANCE_COMPACT_PRECISION = 1e10` | `staking_protocol::BALANCE_COMPACT_PRECISION` (+ `_U256`) | `pub use`; тест `the_u256_precision_is_the_same_number_as_the_u128_one` | закрыта — `9b6213be` (R-117) |
| 7 | Граница компактного стейка `2^112` | `staking_protocol::COMPACT_STAKE_BITS`, `MAX_COMPACT_STAKE` | контракт `Uint<{COMPACT_STAKE_BITS},2>`; узел `use` | закрыта — `9b6213be` (R-117); остаток: имя типа `StorageUint112` в SDK — третье место, мутацией не проверено |
| 8 | Потолок комитета `51` | `staking_protocol::MAX_COMMITTEE_SIZE` (одно имя) | `pub use`; тест `the_committee_cap_fits_the_one_byte_leader_index` | закрыта — `9b6213be` (R-118); остаток: питоновские литералы `51` в стенде (Э3.10) |
| 9 | Адрес системного вызывающего | `fluentbase_types::SYSTEM_ADDRESS` | контракт `pub use … as SYSTEM_CALLER` | закрыта — `9b6213be` |
| 10 | Формула эпохи от блока | `staking_protocol::epoch_at_block` | контракт зовёт; узел `pub use`; третья копия `epocher.rs::OriginEpocher::containing` переведена на общую функцию (`0972059d`) | закрыта частично — арм `activation == 0` остался у контракта осознанно (R-102) |
| 11 | Горизонт коммита `2` | `staking_protocol::MAX_COMMITTEE_LOOKAHEAD_EPOCHS` | `pub use`; узел `drive_ahead_commit` через константу | закрыта — `9b6213be` (R-113 остаток) |
| 12 | Бюджет отказов `(n−1)/3` | `staking_protocol::fault_tolerance` | тест `the_shared_fault_budget_is_commonwares_budget` против `N3f1::max_faults` по исходнику пиннутого чекаута | закрыта — `9b6213be` |
| 13 | Размеры BLS 96/48/256/128 | `staking_protocol::BLS_*_LENGTH` | `pub use` (узел под старыми именами через `as`) | закрыта — `9b6213be` |
| 14 | Длина payload предложения `32` | `staking_protocol::PROPOSAL_PAYLOAD_LENGTH` | `impl FixedSize for Digest` | закрыта — `9b6213be` |

Копии ABI вне пары узел↔контракт, найденные 09-09 и не закрытые: `genesis-bootstrap/src/bootstrap.rs` (`initialize`, `commitEpochCommittee` — не тест, строит genesis стенда), четыре `e2e/src/staking*.rs` (`initialize`), `fluent-stf-sp1::dpos_exec.rs` (вне дерева) — работы Э3.9 (`PLAN.md` §2).

## §3 Упрощения B/BB/CB

Постановка — `history/REGISTER.md` часть 3 (09-03). B-n — из AUDIT, BB-n — из AUDIT-BEACON, CB-n — из AUDIT-CORE. Статус после 09-04 не менялся ни у одного, кроме B-3 (снят 09-03) и B-9 (Э2.4); ход по плану — по `history/PLAN.md` §2.

| # | Что | Закрывает из реестра | Зависит / конфликтует | **Статус 2026-09-09** (источник: `history/PLAN.md` §2, `history/REGISTER.md` часть 3) |
|---|---|---|---|---|
| B-1 | Один вход для σ: Reporter-адаптер в цепочке marshal на `Notarization \| Finalization`; executor при `Missing` читает сертификат из `finalizations_by_height`;`seed_journal.rs` и большая часть `certify.rs` удаляются | R-007, R-008, R-020; упрощает R-018; делает ненужными R-069 (promote drop), BB-1, BB-7, BB-11 | Раньше B-6, B-7, B-8. Конфликт с BB-11 | не пересматривалась после 09-04; Э5 5.3 — П-2 (полное удаление `SeedStore`) против B-1 (RAM-кэш) — решение Д-9 |
| B-2 | Слить `launch` и `launch_follower`; одна фабрика `CommitteeReads` | R-042, половина R-049; половина R-033/R-063 — расхождения двух копий | Поглощает CB-4; вместе с CB-2, CB-10 | не пересматривалась после 09-04; Э6 6.3 (зависит от Д-5) |
| B-3 | ОТМЕНЕНА: контрактный обработчик `slashEquivocation(uint64,uint32)` существует (`consensus.rs`, R-104), доводить нечего и убирать нечего | R-028 (переформулирована) | — | снята 09-03 (К-6: обработчик `slashEquivocation(uint64,uint32)` есть, R-104) |
| B-4 | Убрать корроборацию frontier по кадрам; догон только по проверенным сертификатам | R-003, R-096 | После R-009 (1a) | не пересматривалась после 09-04; Э4 4.2 (внутри П-4) |
| B-5 | Аутентификация до синхронизации; явный weak-subjectivity checkpoint; `probe_frontier` поднимает frontier только проверенным сертификатом | R-001, R-004, R-016, R-040 | — | не пересматривалась после 09-04; Э4 4.2 (аутентификация до `sync_to`); checkpoint — решение Д-2 |
| B-6 | Один `Randomness` вместо `PlaneRandomness` + `FollowerRandomness` + `Absent`; удалить `follower.rs` | R-090; источник расхождений класса R-008 (`ensure_key` без `Thorough` у follower) | После B-1; вместе с BB-9 | не пересматривалась после 09-04; Э5 5.3 (один `Randomness`) |
| B-7 | Пять durable-хранилищ beacon → два: ключи как кэш над `ArtifactStore`, сиды — B-1 | R-068, остаток R-020; `key_journal.rs` удаляется | После B-6; решение по BB-6 после | не пересматривалась после 09-04; Э5 5.1 (внутри П-3) |
| B-8 | Разделить executor: чистый derive, актор FCU, актор прыжков; все ретраи с верхней границей | R-015, R-031, R-061; тестируемость Ex-5 | После B-1 и стенда; поглощает CB-3, CB-6, CB-12, CB-13 | не пересматривалась после 09-04; Э6 6.1 (внутри П-6) |
| B-9 | `ProtocolParams` (chainspec) и `NodeTuning` (CLI) | R-024, R-011, R-101 | К-1 отвечен; конфликта со «сменой интервала» нет, задача сузилась до окна до активации; wire-релиз | отложена — Э2.4 (полный вариант; «дешёвый» П-10 снят PLAN §7) |
| B-10 | Убрать `Option` у обязательных зависимостей (`committee_index`,`charges`,`committee_pair_for` …) | R-100; ветки «без карты — принять любой `extra_data`» | — | не пересматривалась после 09-04; Э8 (закрывает R-100, K-11 частично) |
| B-11 | Типизировать транзиентные ошибки reth | R-030; снижает R-033 | В форке reth | не пересматривалась после 09-04; Э8 (в форке reth) |
| BB-1 | Удалить `SeedStore::wait_for`,`waiters`,`prune_waiters` | — (мёртвый код) | Поглощается B-1 | не пересматривалась после 09-04; поглощается П-2 (Э5 5.3) |
| BB-2 | Единый `verify_seed` для `BeaconOracle` и `KeyOnlyOracle` | — | — | не пересматривалась после 09-04; Э8 |
| BB-3 | Убрать `chain_key_epoch`;`select_carry_scheme` принимает memo | R-056 | — | не пересматривалась после 09-04; Э5 5.1 |
| BB-4 | Один warn-once ledger `(epoch, reason)` вместо четырёх; `torn_warned` оставить | — | — | не пересматривалась после 09-04; Э5 5.2 / Э8 |
| BB-5 | Удалить v1-ветви чтения share-файла | R-052 | Память `dpos deploy always fresh genesis` | не пересматривалась после 09-04; Э5 5.1 (бесплатно при свежем генезисе) |
| BB-6 | Артефакт хранится трижды: убрать копию из share-файла | — | Конфликт/порядок с B-7 | не пересматривалась после 09-04; Э5 5.1 (конфликт с B-7 по окну kill -9 назван в VERIFY-REDESIGN) |
| BB-7 | `InvalidSeed::{RefuseLoud, RefuseQuiet}` различаются только строкой | R-069 частично | Поглощается B-1 | не пересматривалась после 09-04; поглощается П-2 |
| BB-8 | `LogFetcher` — отдать актору `BeaconFetchKey` напрямую | — | — | не пересматривалась после 09-04; Э8 |
| BB-9 | `for_keys`,`for_seeds`,`absent` — под `cfg(feature="testing")` | — | Вместе с B-6 | не пересматривалась после 09-04; Э5 5.3 (вместе с B-6) |
| BB-10 | Семь `with_*` у `DkgActor` → конфиг-структура | — | — | не пересматривалась после 09-04; Э5 5.2 / Э8 |
| BB-11 | Terminal-пин в `SeedStore` + `terminal_per_epoch` заменить правилом «не вытеснять максимальный раунд эпохи» | — | Конфликт с B-1 | не пересматривалась после 09-04; не делать при П-2 (конфликт) |
| BB-12 | `BeaconKeys::notifier` + `subscribe` → один `subscribe` | — | Правка epoch_manager | не пересматривалась после 09-04; Э5 5.1 / Э8 |
| CB-1 | Удалить `DeferReason::{CommitteeNotCommitted, NeedAttestation}` — не конструируются | — | — | не пересматривалась после 09-04; Э8 |
| CB-2 | Удалить `MarshalResolver::Plane` и `Option<U>`;`FollowerResolver::Noop` туда же | Ветка R-007 без захвата; R-063 «Noop» | Вместе с B-2 | не пересматривалась после 09-04; Э6 6.3 |
| CB-3 | Три варианта `DeriveOutcome::Need*` → один + `ParkReason` | — | ⊂ B-8 | не пересматривалась после 09-04; Э6 6.1 (⊂ П-6) |
| CB-4 | Один хелпер `RethCommitteeSource` вместо 6–8 сборок | — | ⊂ B-2 | не пересматривалась после 09-04; Э4 4.1 (⊂ П-1) |
| CB-5 | Один сидер граничного блока (executor берёт `b`,`b+1`; outer — только `b`) | Расхождение E-11 | — | не пересматривалась после 09-04; Э8 |
| CB-6 | `LastCanonicalized.finalized_height` выводить, не хранить | R-043 | ⊂ B-8, но делается отдельно | не пересматривалась после 09-04; Э6 6.1 |
| CB-7 | `Inner` enum и `context` в `EpochEngine` — под `cfg(feature)` | — | — | не пересматривалась после 09-04; Э8 |
| CB-8 | Один драйвер границы с `watch<u64>` вместо цикла на каждый `enter_boundary` | R-065 | Сохранить «нет give-up» и панику как счётчик | не пересматривалась после 09-04; Э8 (R-065) |
| CB-9 | `boundary_hook: Fn(u64)` вместо клона `OrderBlock` до 4 MiB (`application.rs`; адаптер `dpos.rs` уже берёт только `height`) | — | — | не пересматривалась после 09-04; Э8 |
| CB-10 | `initial_head` вычислять один раз; убрать `canonical_state` из `OuterBuilder` | — | Вместе с B-2 | не пересматривалась после 09-04; Э6 6.3 |
| CB-11 | `CertInlet.schemes` → провайдер marshal | — | Конфликт с R-008 (эвикция после verify-fail); К-11 отвечен: комитет закоммиченной эпохи неизменяем, опираться на это безопасно | не пересматривалась после 09-04; П-1 (Э4 4.1) делает ненужным — не делать одновременно с П-1 |
| CB-12 | Убрать гистерезис `probe_fast_left`/`FRONTIER_PROBE_FAST_BURST` | — | После переноса пробы в spawn (R-031) | не пересматривалась после 09-04; Э6 6.1 |
| CB-13 | `ReJump.probe`/`rotate: Option` → свойство `CertUpstream` | — | Вместе с R-004/R-031 | не пересматривалась после 09-04; Э6 6.1 |
## §4 Трассировка A/BA/C/COVERAGE/UNDERSTANDING/CONTRACT-UNDERSTANDING → R и K

Дословно из `history/REGISTER.md` часть 6 (09-03), кроме подраздела `CONTRACT-UNDERSTANDING` §15 (добавлен 09-09 по `history/AUDIT-CONTRACT.md` части V); номера строк там — от 09-03.

Записи R-101..R-110 источника в этой таблице не имеют: они найдены сверкой обеих сторон
границы по исходникам контракта и описаны в `history/CONTRACT.md` часть 2, а не в одном из
входных документов.

R-114..R-119 источника во входных документах не имеют: это отдельный класс — величина, продублированная по обе стороны границы и согласованная только в текущих значениях; при текущих значениях он не срабатывает, поэтому ни аудитом узла, ни аудитом контракта, ни экспериментом не ловится. Полный перечень (14 групп) — `history/DUPLICATES.md`; в реестр вынесены шесть со значимым последствием, две уже известные группы (горизонт коммита и формула бюджета отказов) остались в R-113.

R-111, R-112 и R-113 источника во входных документах не имеют: найдены экспериментом 2026-09-04 (`EXPERIMENTS.md`, часть 5) и описаны там же. R-111 и R-112 разделены намеренно — один и тот же ревёрт и один и тот же фатал, но разные писатели штампа видимости, разная достижимость (популяционная убыль против бюджета отказов) и разный ответственный; R-113 — их общая причина, пропущенный инвариант.

Каждая находка каждого документа — ровно одна строка. «→ R-nnn» — вошла в запись; «объединена» — вошла в запись вместе с другой; иначе указана причина отсутствия отдельной записи.

### history/AUDIT.md, часть A

| ID | Куда |
|---|---|
| A-1 | → R-001 (объединена с C-01, C-08; вход `probe_frontier` вынесен в R-004) |
| A-2 | → R-010 |
| A-3 | → R-008 (не объединена с A-4: другой механизм) |
| A-4 | → R-007 (объединена с C-03) |
| A-5 | → R-003 (объединена с C-04) |
| A-6 | → R-006 (объединена с C-02) |
| A-7 | → R-015 |
| A-8 | → R-043 |
| A-9 | → R-044 |
| A-10 | → R-016 |
| A-11 | → R-011 |
| A-12 | → R-012 (сценарий А) и R-017 (сценарий Б, объединён с BA-3) |
| A-13 | → R-018 |
| A-14 | → R-045 (объединена с C-16) |
| A-15 | → R-046 |
| A-16 | → R-047 |
| A-17 | → R-048 |
| A-18 | → R-049 |
| A-19 | → R-019 (объединена с C-19) |
| A-20 | → R-020 |
| A-21 | → R-021 |
| A-22 | → R-022 |
| A-23 | → R-050 (объединена с C-14) |
| A-24 | → R-051 |
| A-25 | → R-052 (объединена с BA-20) |
| A-26 | → R-053 |
| A-27 | → R-039 |
| A-28 | → R-023 (часть «Confirm», объединена с BA-14) и R-054 (часть «буферизация», объединена с BA-18) |
| A-29 | → R-024 |
| A-30 | → R-025 |
| A-31 | → R-055 |
| A-32 | → R-056 (объединена с BA-15) |
| A-33 | → R-026 (объединена с BA-2) |
| A-34 | → R-057 |
| A-35 | → R-027 |
| A-36 | → R-014 |
| A-37 | → R-028 (с A-61) |
| A-38 | → R-058 |
| A-39 | → R-080 |
| A-40 | → R-005 (анкер исправлен) |
| A-41 | → R-081 |
| A-42 | → R-082 |
| A-43 | → R-083 |
| A-44 | → R-029 |
| A-45 | → R-013 (объединена с C-07) |
| A-46 | → R-059 |
| A-47 | → R-060 |
| A-48 | → R-061 |
| A-49 | → R-030 |
| A-50 | → R-062 |
| A-51 | → R-031 (объединена с C-09) |
| A-52 | → R-032 |
| A-53 | → R-033 |
| A-54 | → R-063 |
| A-55 | → R-064 |
| A-56 | → R-065 (объединена с C-15) |
| A-57 | → R-034 |
| A-58 | → R-066 |
| A-59 | → R-035 |
| A-60 | → R-084 |
| A-61 | → R-028 (сам документ помечает «учтено в A-37») |

### history/AUDIT.md, части B, D, E

| ID | Куда |
|---|---|
| B-1 … B-11 | → часть 3, те же номера |
| D список «контракт» | → К-1, К-2, К-5, К-4, К-8, К-9 |
| D список «эксперимент» | → Ex-5, Ex-5, Ex-11, Ex-1 |
| D-1 … D-14 | → Ex-6, Ex-1, Ex-3, Ex-18, Ex-19, Ex-12, Ex-10, Ex-11, Ex-20, Ex-21, Ex-5, Ex-14, Ex-9, Ex-15 |
| E-1 … E-16 | Расхождения с UNDERSTANDING, уже внесены в него; все указывают на находки, вошедшие в реестр (E-1 → R-008/R-007; E-2 → R-001; E-3 → R-014; E-4 → R-028; E-5 → R-011; E-6 → R-012; E-7 → R-049; E-8 → R-045; E-9 → R-046; E-10 → R-039; E-11 → R-023/R-054; E-12 → R-098; E-13 — счёт тестов, записи нет; E-14 → R-050; E-15 → R-027; E-16 → R-006) |

### history/AUDIT-BEACON.md

| ID | Куда |
|---|---|
| BA-1 | → R-002 |
| BA-2 | → R-026 (объединена с A-33) |
| BA-3 | → R-017 (объединена с A-12 Б) |
| BA-4 | → R-036 |
| BA-5 | → R-037 |
| BA-6 | → R-038 |
| BA-7 | → R-067 |
| BA-8 | → R-068 |
| BA-9 | → R-069 |
| BA-10 | → R-070 |
| BA-11 | → R-071 |
| BA-12 | → R-085 |
| BA-13 | → R-072 |
| BA-14 | → R-023 (объединена с A-28) |
| BA-15 | → R-056 (объединена с A-32) |
| BA-16 | → R-086 |
| BA-17 | → R-087 |
| BA-18 | → R-054 (объединена с A-28) |
| BA-19 | → R-088 |
| BA-20 | → R-052 (объединена с A-25) |
| BA-21 | → R-089 |
| BA-22 | → R-090 |
| BA-23 | → R-091 |
| BA-24 | → R-092 |
| BB-1 … BB-12 | → часть 3 |
| D-1 | → К-3 |
| D-2 | → К-3 |
| D-3 | → Ex-22 |
| D-4 | → Ex-8 |
| D-5 | → Ex-13 |
| D-6 | → Ex-16 |
| D-7 | → Ex-24 (анализ, записи нет) |
| D-8 | → Ex-17 |
| D-9 | закрыт в части 4 (квоты прочитаны в коде) |
| E-1 … E-11 | Расхождения с UNDERSTANDING: E-1 → R-002; E-2 → R-036; E-3 → R-069; E-4 → R-017; E-5 → R-026; E-6 — счёт счётчиков, записи нет; E-7 → R-037; E-8 → R-086 (не дефект); E-9 — опровержение UNDERSTANDING §11.25, записи нет; E-10 — счёт тестов, записи нет; E-11 → К-3 |

### history/AUDIT-CORE.md

| ID | Куда |
|---|---|
| C-01 | → R-001 (объединена с A-1) |
| C-02 | → R-006 (объединена с A-6) |
| C-03 | → R-007 (объединена с A-4) |
| C-04 | → R-003 (объединена с A-5) |
| C-05 | → R-009 |
| C-06 | → R-004 |
| C-07 | → R-013 (объединена с A-45) |
| C-08 | → R-001 (объединена: тот же код и пропуск; последствие (в)) |
| C-09 | → R-031 (объединена с A-51) |
| C-10 | → R-040 |
| C-11 | → R-041 |
| C-12 | → R-042 |
| C-13 | → R-073 |
| C-14 | → R-050 (объединена с A-23) |
| C-15 | → R-065 (объединена с A-56) |
| C-16 | → R-045 (объединена с A-14) |
| C-17 | → R-074 |
| C-18 | → R-075 |
| C-19 | → R-019 (объединена с A-19) |
| C-20 | → R-076 |
| C-21 | → R-077 |
| C-22 | → R-093 |
| C-23 | → R-094 |
| C-24 | → R-095 |
| C-25 | → R-096 |
| C-26 | → R-097 |
| C-27 | → R-098 |
| C-28 | → R-078 (повышена до MINOR) |
| CB-1 … CB-13 | → часть 3 |
| D-1 | → Ex-2 |
| D-2 | → Ex-8 |
| D-3 | → Ex-6 |
| D-4 | → Ex-7, Ex-4 |
| D-5 | → К-7 |
| D-6 | → К-11 |
| D-7 | → Ex-23 |
| D-8 | → Ex-5 |
| E-1 … E-15 | Расхождения doc-комментариев с кодом; записей не получают, кроме тех, что дублируют находки: E-1 → R-006; E-2 → R-073; E-3, E-4, E-5, E-6, E-7, E-10 — только комментарии (учтены в R-091 как класс); E-8 → R-076; E-9 — комментарий; E-11 → CB-5; E-12 → R-045; E-13, E-14 — совпадают с кодом, правки нет; E-15 → R-001 |
| F-1 … F-13 | Расхождения с UNDERSTANDING: F-1 → R-006; F-2 → R-007; F-3 → R-003; F-4 → R-009; F-5 → R-001; F-6 → R-045; F-7 → R-050; F-8 — комментарий `byzantine.rs` (E-5); F-9 → R-046; F-10 → R-094; F-11 → R-043; F-12 → R-041; F-13 → R-065 |

### history/COVERAGE.md

| Где | Куда |
|---|---|
| §2.3 `bootstrappers.rs` retry-окно с пустым результатом | → R-079 |
| §2.3 `secret_store.rs` `reject_insecure_mode` не на Unix | → R-099 |
| §2.3 `extra_data.rs` `leader_index` без диапазона | → R-100 |
| §2.3 `encoding.rs` «раскладка принята по тестам, не по чтению» | пробел проверки, записи нет |
| §2.4 `dkg_agree.rs` × `Automaton` | закрыто BEACON (часть 5, пробелы) |
| §5 п. 1–10 и «кроме десяти» | → часть 5, абзац «пробелы проверки»; п. 9 → Ex-17 |
| §1, §3, §4 (таблицы покрытия) | статистика, находок нет |

### history/CONTRACT-UNDERSTANDING.md, §15 (открытые вопросы)

Вердикты — `history/AUDIT-CONTRACT.md` часть V (V-1..V-22). Пятнадцать сведены в записи K (1 → K-40, 2 → K-16, 3 → K-6, 4 → K-30, 5 → K-10, 6 → K-28, 9 → K-26 + K-9, 10 → K-12, 11 → K-2, 13 → K-19, 14 → K-18, 17 → K-21, 18 → K-20, 22 → K-17). Без номера K остались 7, 8, 12, 15, 16, 19, 20, 21 — вердикт «не дефект» либо «часть K-2»; из них V-15 и V-19 требуют перепроверки (см. §2).

### history/UNDERSTANDING.md, §11 (подозрения) и прочее

| Пункт | Куда |
|---|---|
| 11.1 | → R-048 |
| 11.2 | → R-084 |
| 11.3 | → R-045 |
| 11.4 (`initial_epoch` vs tracked `e+1`) | ни один аудит не поднял; гипотеза безвредности не опровергнута; записи нет |
| 11.5 | → R-049 |
| 11.6 | → R-100 |
| 11.7 | → R-098 |
| 11.8 | → R-043 |
| 11.9 | → R-083 |
| 11.10 (`step_gas_limit` при parent < 5000) | недостижимо при anchor ≥ 5000 (по UNDERSTANDING §7); записи нет |
| 11.11 (`#[ignore]`-заглушки p2p) | пробел тестов, не дефект узла; записи нет |
| 11.12 (`epoch_of_block` деление на 0) | вызывающие проверяют (`apply_at`); записи нет |
| 11.13 | → R-011 |
| 11.14 | → R-047 |
| 11.15 | → R-062 |
| 11.16 | закрыто в §12.8 |
| 11.17 (mod-bias VRF) | пренебрежимо; записи нет |
| 11.18 | → R-022 |
| 11.19 | → R-053 |
| 11.20 | → R-056; кэш битов — не дефект (BA-16) |
| 11.21 (`ArtifactStore` без вытеснения) | ни один аудит не поднял; рост O(смен комитета), сотни байт на смену; записи нет |
| 11.22 | → R-008 |
| 11.23 | → R-025 |
| 11.24 (`fetch_targeted` для своего лога) | комментарий `actor.rs:1997-2001` называет безвредным; записи нет |
| 11.25 | опровергнуто BEACON E-9 |
| 11.26 (`enqueue_fallback` при tombstoned) | дедуп по `submitted_this_session`; записи нет |
| 11.27 (`next_charge` под write-lock) | короткая блокировка; записи нет |
| 11.28 | → R-014 |
| 11.29 | → R-080 |
| 11.30 | → R-080 |
| §4 «инварианты, которые код предполагает» | → R-012, R-024, R-001, R-003 |
| §12.X таблица ВЕРА | → часть 4 (К-1 … К-13), все закрыты по `history/CONTRACT.md` 2026-09-03; строки таблицы, оставшиеся «ВЕРОЙ» после сверки: ни одной |
| §9 «проглатываемые ошибки» | все вошли: R-049, R-087, R-022, R-027, R-021, R-011 |
