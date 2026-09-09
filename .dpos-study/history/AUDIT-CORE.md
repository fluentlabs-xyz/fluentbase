# Повторный аудит верхнего уровня `crates/dpos/consensus/src` (без beacon и slasher)

Дата: 2026-09-03. Ветка `djadjka/dpos-reth-2.2-squashed`. Зависимости проверялись по исходникам:
commonware `v2026.4.0` (`~/.cargo/git/checkouts/monorepo-27b478c9bb41d208/3c4e02c`, далее `CW:`),
reth `v2.2-patched-tree-escrow@8cc96a37` (`~/.cargo/git/checkouts/reth-9084db4313ec21c5/8cc96a3`, далее `RETH:`).
Существующие `AUDIT.md`/`AUDIT-BEACON.md` не читались. `UNDERSTANDING.md` прочитан целиком.

Все 24 файла прочитаны целиком, включая тела функций, ветки ошибок и `mod tests`
(для тестов executor/cert_inlet/epoch_manager/application/dpos прочитаны фикстуры, списки тестов и
тесты, на которые опираются находки; остальные тесты — целиком). Строки — по текущему дереву.

Обозначения уверенности: **[код]** — подтверждено прочитанным кодом; **[ГИПОТЕЗА: …]** — вывод,
чья названная часть не подтверждена чтением.

---

## Часть A. Находки

### C-01. CRITICAL — цель прыжка аутентифицируется состоянием, синхронизированным с той же цели; на plane-пути «upstream» — любой из ≤4096 отслеживаемых пиров

**Суть.** `cold_start_jump_with_threshold` берёт `latest` у upstream, проверяет только `payload == digest`,
затем `sync_to` даёт reth FCU `head = safe = finalized = latest.block.result` и ждёт, пока reth
скачает эту цепь по devp2p, и только потом читает `committee[round.epoch]` **из состояния на
`landing_hash`** и проверяет мультиподпись (`cold_start_jump.rs:805-847`, `:434-474`, `:662-682`).
Комитет читается из состояния, которое построила проверяемая сторона. На валидаторе L1-checkpoint
не передаётся вовсе (`dpos.rs:1839`, `:2524`). Эпоха раунда сертификата не связывается с
`epoch_of(latest.block.height)` нигде на этом пути.

На plane-пути upstream — `PlaneUpstreamHandle`: `get_latest` = один `fetch(FrontierKey::Latest)`,
resolver сам выбирает пира из всего отслеживаемого набора (реестр ∪ комитеты, до 4096:
`staking-reader/src/epoch_transition.rs:614-648`, `p2p/src/constants.rs:186`) в порядке
**лучшей задержки** (`CW:resolver/src/p2p/fetcher.rs:95`, `:233-250` — `PrioritySet`, без перемешивания
на первой попытке). `FrontierHandler::deliver` принимает любую декодируемую пару и отвечает `true`
(`plane_upstream.rs:198-212`). Валидатор всегда имеет upstream: `crates/node/src/dpos.rs:2026-2029`
(`ValidatorUpstream::{Ws, Plane}`), поэтому прыжок при каждом `Restart` доступен
(`dpos.rs:1795`, `:1136-1137`).

**Сценарий.** Валидатор перезапускается (`kind == Restart`). Пир с низкой задержкой из реестра
отвечает на `Latest` парой `(finalization, block)`, где `block.height > anchor + 1024`,
`block.result` — хэш блока его собственной EVM-цепи (форк с изменённым состоянием стейкинга и
комитетом из его ключей), а `finalization` подписана этим «комитетом». Узел даёт reth FCU на этот
хэш, reth скачивает цепь с devp2p-пиров атакующего, `verify_jump_authenticated` читает комитет из
этого состояния и проверка проходит. `latest_finalized_hash` становится хэшем атакующего
(`dpos.rs:1845-1847`), `initial_snapshot` читается оттуда же (`:1988`), marshal-floor = landing−K.

**Последствие.** Узел на чужой цепи. Дальше либо (а) честные сертификаты не проходят под чужим
комитетом и узел стоит; либо (б) если атакующий сохранил настоящий комитет, честные блоки
исполняются на чужом родителе и через K блоков `result_matches` даёт `ForkSafety` →
`SafetyHalt` с маркером на диске (`executor.rs:3215-3234`, `sync_metrics.rs:542-580`): узел
выключен из консенсуса до ручного вмешательства оператора и ресинка. Достаточно одного пира,
BFT-предположение не задействовано. Уверенность: механизм **[код]**; практическая стоимость
поставки форк-цепи по devp2p — **[ГИПОТЕЗА: не проверялось экспериментом]**.

**Исправление.** Аутентифицировать цель **до** `sync_to`: читать `committee[E_target]` из
собственного последнего финализированного состояния (комитеты закоммичены на эпоху вперёд —
это покрывает цель в пределах одной эпохи), а для более глубоких целей требовать L1-checkpoint
или f+1-корроборацию `Latest` от членов текущего комитета; связать `round.epoch` с
`epoch_of(height)`; в `sync_to` не ставить `finalized` на непроверенный хэш (только `head`).

### C-02. HIGH — guard #2 читает канонический хэш reth до FCU: на настоящем reth проверка либо пуста, либо даёт ложный SafetyHalt

**Суть.** В `try_derive` ветка re-derive выполняет для целевого блока только `import_derived`
(`executor.rs:3635-3645`), без FCU. Guard #2 сразу после этого сравнивает
`block_{h+K}.result` с `self.executed.spec_executed_hash(h)` (`:3131-3152`).
`spec_executed_hash` = `provider.block_hash(h)` — каноническая цепь reth
(`crates/node/src/ordering.rs:45-47`; `RETH:crates/storage/provider/src/providers/consistent.rs:576-582`),
а `InsertExecutedBlock` кладёт блок только в tree-state, невидимый провайдеру до FCU
(`RETH:crates/engine/tree/src/tree/mod.rs:1551-1573`; `.claude/RETH_INTERNALS.md` «Verdict (c)»).

**Сценарий 1 (проверка пуста).** Узел отстал на ≥K, догоняет без спекуляции: `block_hash(h)` = `None`
→ `result_matches` = `None` → guard молча пропускает (`order_block.rs:196`). Расхождение ловится
только обратной проверкой на высоте h+K (`:3215`), после того как h, h+1, h+2 уже подтверждены и
FCU'нуты как `safe`.

**Сценарий 2 (ложный halt).** Узел спекулятивно исполнил сиблинг A на высоте h (нотаризация вида v,
затем nullify), финализирован B в виде v+1; тем временем узел отстал на ≥K (например, блок h
держался в `awaiting_seed`, а сеть финализировала h+1..h+3). `correctly_speculated = false`
(`:2977-3007`), re-derive B → import; `block_hash(h)` = A (канонический со спекулятивного FCU),
`block_{h+K}.result` = hash(B) → `Some(false)` → `Fault::fork_safety` → `park_halted`, маркер на диске.

**Последствие.** Сценарий 1 — заявленный «единственный детектор на пути догона» не работает.
Сценарий 2 — необратимая остановка честного узла. Тест `guard2_convergence_mismatch_engages_safety_halt`
(`executor.rs:7430-7460`) проходит только потому, что `FakeDeriver` кладёт хэш в каноническую карту при
derive (`:4199-4209`), чего reth не делает. **[код]**

**Исправление.** Сравнивать `block_hk.result` с локальной переменной `derived_hash`, а не с
провайдером.

### C-03. HIGH — σ из финализаций не захватывается на валидаторе; финализация без нотаризации → блок держится в `awaiting_seed` до порога re-jump

**Суть.** `spec_exec::Mailbox::report` обрабатывает только `Activity::Notarization`
(`spec_exec.rs:52-54`); `Activity::Finalization` от собственного voter'а игнорируется, хотя
сертификат несёт σ. Второй источник — `UpstreamResolver::spawn_finalized` — срабатывает только на
by-height pull для дыры (`cert_inlet.rs:3134-3153`). Сертификат финализации, пришедший по
cert-каналу, хранится marshal'ом без захвата σ (`CW:consensus/src/marshal/core/actor.rs:567-605`).
Executor держит блок без выхода, кроме прихода σ (`executor.rs:1777-1794`), детектор только
логирует через 60 с (`:2845-2871`), единственный выход — re-jump при разрыве
`> min(1024, interval)` (`:2160-2185`, `dpos.rs:2458`).

**Сценарий.** Валидатор кратко отрезан (или потерял кадр нотаризации), voter получает
`Finalization(v)` от пиров (`CW:consensus/src/simplex/actors/voter/actor.rs:625-660` — reporter
получает только Finalization), marshal диспатчит блок, executor: `seed_for(Round(E, v))` = `None`
→ hold. Триггер (доставка Finalization без Notarization) — **[ГИПОТЕЗА: поведение commonware при
кратком разрыве не проверялось]**; цепочка кода — **[код]**.

**Последствие.** Исполнение узла стоит до `min(1024, interval)` блоков (≈17 мин при 1 blk/s,
одна эпоха при малом интервале), пока re-jump не «переступит» блок; всё это время узел не может
голосовать (result-gate). **Исправление.** Захватывать σ и из `Activity::Finalization`
(`capture_certificate_seed` уже есть).

### C-04. HIGH — `corroborate_frontier` считает любых отслеживаемых пиров, не членов комитета; f+1 записей реестра переводят узел в verify-only навсегда

**Суть.** Кадры с незарегистрированного сабканала VOTE идут в `vote_backup` с идентичностью
отправителя без проверки членства (`crates/node/src/dpos.rs:1137-1153`);
`corroborate_frontier` требует `(n−1)/3 + 1` **различных** отправителей, где n — размер комитета,
а отправители — любые пиры отслеживаемого набора (реестр ∪ комитеты)
(`epoch_manager.rs:1694-1726`, `staking-reader/src/epoch_transition.rs:614-648`).
`highest_observed_epoch` никогда не уменьшается; `is_live_epoch(E) = E >= highest_observed_epoch`
(`:947-949`); при `false` `reconcile_roles` делает только `soft_enter` (`:1006-1011`).

**Сценарий.** n=4 ⇒ порог 2. Два зарегистрированных (не в комитете) пира шлют по одному кадру на
сабканал `E+10^6`. `highest_observed_epoch = 10^6`. Реальные эпохи «не live» → движок не
спавнится; `PINS_PER_SENDER` не мешает (один epoch на отправителя). До перезапуска узел не
подписывает. Если так обработать >f валидаторов — цепь стоит. **[код]**

**Исправление.** Учитывать только отправителей из `committee[highest_entered]` (BiMap) и/или
ограничивать принимаемую эпоху `entered + CATCHUP_SPAN_CAP`; либо корроборировать подписанным
голосом.

### C-05. HIGH — `FrontierHandler::deliver` не связывает ключ с содержимым: быстрый пир «удовлетворяет» `Finalized{h}` чужой высотой и морит гап-репэйр

**Суть.** `deliver(key, value)` возвращает `true` для любой декодируемой пары, не сверяя высоту с
`key` (`plane_upstream.rs:198-212`). Для `Finalized{h}` в Hybrid-резолвере результат идёт в
marshal, который отвергает несовпадение высоты (`CW:marshal/core/actor.rs:988-996`), но resolver
уже считает fetch выполненным и не штрафует пира; `UpstreamResolver::spawn_finalized` тоже ничего
не делает при `false` (`cert_inlet.rs:3151-3153`). Следующий sweep выбирает пира снова по
задержке (`CW:resolver/src/p2p/fetcher.rs:233-250`, `PrioritySet`).

**Сценарий.** Валидатор догоняет по plane-пути (единственный by-height путь в Hybrid — upstream,
`outer.rs:94-105`). Самый быстрый пир на каждый `Finalized{h}` отвечает валидной парой высоты h−1.
Дыра на h не закрывается, пока пир остаётся самым быстрым. **[код]**

**Исправление.** В `deliver` проверять `block.height == height` для `Finalized` и возвращать
`false` на несовпадение (resolver тогда ротирует/блокирует).

### C-06. HIGH — `upstream_frontier` инфлируется одним ответом на пробу и никогда не убывает; re-jump спавнится после каждого tip/heartbeat и подвешивает живой валидатор на время watchdog'а

**Суть.** `probe_frontier` срабатывает, когда tip не сдвинулся за 1 с (`executor.rs:1904-1934`),
делает `get_latest` у любого пира и `fetch_max` в `upstream_frontier` (`:1917-1920`);
`maybe_re_jump` использует `max(tip, upstream_frontier) − ordering_finalized > threshold`
(`:2172-2185`) — значение не убывает, нет проверки подписи. Пока jump в полёте, все derive и
спекуляция подавлены (`:1388-1411`, `:1761`, `:2560`), а `sync_to` ждёт до 90 с / 300 с / 6 ч
(`cold_start_jump.rs:194`, `:211`, `:183`); `StalledWithPeers` не ротирует и перевооружается на
следующий tip (`executor.rs:1312-1334`).

**Сценарий.** На здоровом валидаторе один пропущенный тик (tip не изменился за секунду —
обычный джиттер при 1 blk/s) вызывает пробу; вредоносный пир отвечает высотой
`ordering_finalized + 10^6`. Дальше при каждом `Update::Tip` и heartbeat спавнится re-jump; когда
`get_latest` попадает на того же пира, reth получает FCU на несуществующий хэш и валидатор не
исполняет блоки до срабатывания watchdog'а (300 с при `peers > 0`), затем цикл повторяется. **[код]**

**Исправление.** Не поднимать `upstream_frontier` с неаутентифицированной пробы (корроборация
f+1 членов комитета или подпись); возвращать значение к `tip` после `Lagging`; гейтить re-jump
на живом сигнере отдельно.

### C-07. MEDIUM — буфер BROADCAST держит до 64 сообщений по 4 MiB на каждого «primary»-пира; primary-набор — реестр, а не комитет

**Суть.** `buffered::Engine` кладёт сообщение в кэш для любого пира из `latest.primary`
(`CW:broadcast/src/buffered/engine.rs:321-360`), до `deque_size` сообщений на пира
(`outer.rs:823-833`, `deque_size = 64` из `dpos.rs:2650`), каждое до `MAX_ORDER_BLOCK_SIZE` = 4 MiB
(`order_block.rs:31`, `:360-365`); квота 8/с (`p2p/src/constants.rs:128`). Отслеживаемый набор —
реестр ∪ комитеты (`epoch_transition.rs:614-648`).

**Сценарий.** Пир из реестра, не в комитете, за 8 с заливает 64 валидно закодированных OrderBlock
по 4 MiB (txs — любой валидный RLP; подписи при декоде не проверяются) → 256 MiB на пира; при
сотнях записей в реестре — десятки GiB. Уверенность: буфер и лимиты **[код]**;
что `latest.primary` — весь отслеживаемый набор, а не только комитет — **[ГИПОТЕЗА: структура
`Set{primary,…}` в commonware p2p не читалась]**. Если primary = комитет, потолок 51×256 MiB
(BFT-ограничен, но всё равно 13 GiB).

**Исправление.** Отдельный tracker-индекс/набор для broadcast, ограниченный комитетом; либо
`deque_size` ≤ 4 и байтовый лимит.

### C-08. MEDIUM — `sync_to` ставит reth `finalized` на непроверенный хэш до аутентификации; после `AuthFailed`/`Stalled` backfill reth не отменяется

**Суть.** FCU `head = safe = finalized = tip_hash` (`cold_start_jump.rs:470-474`) выполняется до
всех проверок; `AuthFailed` ведёт к `rotate` + повтор (`dpos.rs:1296-1311`), но reth уже запустил
pipeline-backfill к цели, во время которого отвечает `SYNCING` на всё (`RETH_INTERNALS.md`,
Gotcha 9; `RETH:…/tree/mod.rs` через `backfill_sync_state`), а недостающий заголовок запрашивается
у пиров без таймаута (Verdict (d)). Ничто на нашей стороне не сбрасывает цель. Последствие —
EL стоит после единственного плохого ответа. Требует эксперимента (D-1). Уверенность:
последовательность вызовов **[код]**; необратимость backfill в reth — **[ГИПОТЕЗА]**.

**Исправление.** См. C-01 (аутентификация до FCU); в `sync_to` не трогать `finalized`.

### C-09. MEDIUM — исполнитель монополизирует свой цикл: сетевая проба в теле select и бесконечные циклы re-apply/транспорта

**Суть.** `probe_frontier` ожидает `probe()` inline (`executor.rs:1519-1531`, `:1913`); на
plane-пути это `fetch_one` с таймаутом 8 с (`plane_upstream.rs:70`, `:280`) — при быстрой каденции
200 мс (`:1525-1531`) исполнитель может стоять 8 с из каждых 8,2 с, пока пиры не отвечают.
`fcu_retrying_transport` (`:1962-1997`) и постусловие re-apply (`:3370-3462`, при
`height > finalized_height` без предела) крутятся внутри `try_derive`: мейлбокс не читается,
heartbeat не шлётся, латч `SafetyHalt` не проверяется, ack'и marshal не отдаются (окно 16 заполняется).
**[код]**

**Исправление.** Пробу — в spawn как re-jump (`:2198-2205`); циклы re-apply — с уступкой select
(возвращать `Deferred`-парковку вместо внутреннего цикла).

### C-10. MEDIUM — by-height fetch'и не связывают эпоху раунда с высотой; сертификат старого комитета за любой высотой хранится через `store_verified_finalization`

**Суть.** `fetch_verified_boundary` пинует только высоту (`cert_follow.rs:200-202`);
`verify_jump_authenticated` берёт эпоху из `round` сертификата (`cold_start_jump.rs:669-673`);
`refetch_verified_archive_hole` так же (`dpos.rs:444-466`). Результат уходит в marshal через
`verified` + `report(Finalization)` (`executor.rs:2267-2272`, `outer.rs:957-958`), а этот путь
хранит без проверки высота↔эпоха (`CW:marshal/core/actor.rs:567-605`, `:1404-1430`).

**Сценарий.** Ключи комитета старой эпохи E′ скомпрометированы (long-range). Upstream отвечает на
`Finalized{last(E−1)}` блоком нужной высоты с сертификатом `round.epoch = E′`. Он верифицируется
`committee[E′]` и попадает в архив как граничный блок; `boundary_lookup` берёт из него
`proposal_view` для базы лидера (`epoch_manager.rs:1343-1354`) → `terminal_seed_at` промахивается →
спавн отложен, либо (если σ есть на подделанном раунде) — иная база лидера, чем у сети.
**[код]**; достижимость (компрометация старых ключей) — предположение.

**Исправление.** Проверять `epocher.containing(height).epoch() == round.epoch()` во всех трёх местах.

### C-11. MEDIUM — партиции `consensus_epoch_{E}` никогда не удаляются

**Суть.** Каждый движок эпохи открывает журнал `consensus_epoch_{E}` (`engine.rs:267`);
`prune_agreements` подметает только `dkg_epoch_*` (`epoch_manager.rs:318-362`); других вызовов
`remove` для ordering-партиций нет (grep по `crates/dpos`, `crates/node`). Ретеншн view внутри
партиции не удаляет саму партицию и её последние секции. Рост диска — одна партиция на эпоху за
весь срок процесса (при devnet-интервале 32 — тысячи). **[код]**

**Исправление.** Подметать `consensus_epoch_{e}` в том же окне, что `AGREEMENT_SWEEP_SPAN`.

### C-12. MEDIUM — follower: `sync_to` при известной геометрии фатален на любой `SyncFailure`

**Суть.** `el.sync_to(&latest).await?` (`dpos.rs:2996`, `:3040`) — 90 с без devp2p-пиров или
300 с застоя валят `launch_follower`, тогда как 30 строк ниже тот же прыжок обёрнут в
`cold_start_jump_self_heal` с вечным повтором (`:3094-3105`). Перезапуск-шторм follower'ов при
проблемах devp2p. **[код]**

### C-13. LOW — `RotatedKey`-движок спавнится и никогда не снимается, вопреки комментарию

`SignerVerdict::RotatedKey` спавнит движок (`epoch_manager.rs:1192-1200`), `roles` = Signer.
На следующем reconcile живой движок снимается только при `share_probe == Withheld`
(`:1041-1074`); ветки для «ключ не в BiMap» нет. Комментарии (`:1197`, `engine.rs:185-187`,
`:236-237`) утверждают обратное. Стоимость — verify-only simplex на всю эпоху (resolver-фетчи,
партиция). **[код]**

### C-14. LOW — маркер `SafetyHalt` пишется после защёлки, без fsync и без rename

`engage` → `latch` → `std::fs::write` (`sync_metrics.rs:542-580`). Падение между защёлкой и
записью или частичная запись → узел возвращается сигнером. Код это логирует как известный риск;
исправление — write-tmp + fsync + rename, и писать маркер до `latch`. **[код]**

### C-15. LOW — `enter_boundary` порождает по одному вечному re-poke циклу на каждый вызов

`dpos.rs:2149-2283`: цикл на каждый `Update::Block` и на каждое приземление re-jump; два цикла
могут работать на одну границу (комментарий `:2234-2243` это признаёт); каждый тик берёт
`et.lock()`. Стоимость — конкуренция за мьютекс `EpochTransition` и непредсказуемое число задач.
**[код]**

### C-16. LOW — схема регистрируется до `WeightedVrf::try_new`; при `WeightsUnavailable` эпоха остаётся без движка до следующего edge без повтора

`engine.rs:195` vs `:258`; `spawn_engine` возвращает `false` без записи в `deferred_spawns`
(`epoch_manager.rs:1402-1410`, `:1461-1464`). Схема-сигнер в `EpochSchemeProvider` остаётся, при
следующем soft_enter отказ на понижение логируется как `error!`. **[код]**

### C-17. LOW — вердикт `Ok(Invalid)` FCU игнорируется в `reseed_forward` и в heartbeat

`executor.rs:2329-2351` (проверяется только `Err`), `:2028-2039` (`Ok(_)` → recover). Reth,
отвергнувший названный head после прыжка, остаётся незамеченным до следующего derive. **[код]**

### C-18. LOW — `soft_enter_span` считает эпоху зарегистрированной, даже если `register` отказал

`outer.rs:1188-1192` ставит `registered = epoch` после `register(...)`, который может отказать
молча (`:339-369`); `highest_entered_epoch` уходит вперёд (`epoch_manager.rs:1792`), хинт целится в
границу без схемы. **[код]**

### C-19. LOW — рост комитета за 51 после старта не проверяется

Проверка только при запуске (`dpos.rs:1959-1973`). Позже: декодирование сертификатов с cap
`MAX_COMMITTEE_SIZE` (`plane_upstream.rs:173`, marshal cfg) отвергает битмапы >51, индексы >255 —
`IndexExceedsWireFormat` (`application.rs:471-476`). Узел молча перестаёт принимать сертификаты
эпохи. **[код]**; поведение контракта — вне репозитория.

### C-20. LOW — follower: эпоха, пропущенная `enter_finalized_epoch`, не имеет схемы; сертификаты этой эпохи marshal «принимает» без сохранения

`dpos.rs:1464-1482` (принято как допустимое), `CW:marshal/core/actor.rs:963-972` (нет схемы →
`true`, не сохранено). Дыра на высотах этой эпохи не закрывается до re-jump. **[код]**

### C-21. LOW — наблюдаемость: у `awaiting_seed` нет gauge, у маркера SafetyHalt нет высоты/хэша

Держание блока видно только по одноразовому счётчику после 60 с (`executor.rs:2857-2871`), в
отличие от `deferred_height` для парков (`:883-890`). Маркер содержит одну метку причины
(`sync_metrics.rs:565`); высота, ожидаемый и локальный хэши — только в логе. Оператор из маркера
не узнаёт, что именно разошлось и что ресинкать. **[код]**

### C-22. NIT — после halt каждый reconcile логирует `error!` об отказе понижения схемы

`epoch_manager.rs:1069-1070` (`roles = Verifier`, `soft_enter`) → `outer.rs:351-359` `error!` на
каждом edge при живом сигнер-scheme. Шум уровня error в постоянном режиме. **[код]**

### C-23. NIT — result-gate делает 41 итерацию, а не 40; BiMap клонируется на каждый verify с обвинением

`application.rs:947-948` (`0..=polls`, polls = 40) ⇒ 41 проверка, бюджет 1000 мс + 40×25 мс сна;
`:735` `(**bimap).clone()`. **[код]**

### C-24. NIT — `UpstreamResolver::cancel` снимает высоту из `inflight`, пока задача ещё летит

`cert_inlet.rs:3284-3288` vs guard `:2974-2989` — возможен дубль pull'а. Безвредно. **[код]**

### C-25. NIT — `hint_finalized` целится в отправителя backup-голоса, который может быть не членом комитета

`epoch_manager.rs:908-914`; в Hybrid цель игнорируется (`cert_inlet.rs:3259-3268`), в `Plane`
резолвере — недостижимая конфигурация (см. B-2). **[код]**

### C-26. NIT — `OrderBlock::read_cfg` зависит от непрерывности буфера

`order_block.rs:353-359` (`buf.chunk()`); задокументировано; при сегментированном `Buf` — ложный
`EndOfBuffer`/ошибка RLP. **[код]**

### C-27. NIT — `lock().unwrap()` на мьютексах в spawn-задачах

`cert_inlet.rs:3102`, `:3286`, `:3291`, `:3295`; `plane_upstream.rs:206`, `:273`, `:291`;
`outer.rs:188-189`. Отравленный мьютекс роняет задачу, `InflightGuard` при этом корректен. **[код]**

### C-28. NIT — панику marshal при ошибке архива (`panic!("failed to finalize")`) супервизор превращает в abort-all

`CW:marshal/core/actor.rs:1450`; `outer.rs:1575-1581`. Полный диск = падение узла; нет отдельного
сообщения оператору. **[код]**

---

## Часть B. Упрощения

| # | Что | Где | Цена |
|---|---|---|---|
| B-1 | Удалить `DeferReason::{CommitteeNotCommitted, NeedAttestation}` — не конструируются (grep по crate: только `fault.rs`) | `fault.rs:27-47` | нулевая; метрика `defer` теряет два никогда не встречавшихся лейбла |
| B-2 | Удалить `MarshalResolver::Plane` и `Option<U>` в `OuterEngine::start`/`DposLayerConfig::upstream`: node всегда даёт `Some` (`crates/node/src/dpos.rs:2026-2029`, `dpos.rs:2449-2453`); ветка `Plane` — единственный путь без захвата σ (C-03) | `outer.rs:64-193`, `:1426-1501`; `dpos.rs:941` | правка тестов, конструирующих `None`; `FollowerResolver::Noop` (`cert_inlet.rs:3166-3172`, «not a reachable production config») — туда же |
| B-3 | Три варианта `DeriveOutcome::{NeedAttestation, NeedParentVisible, NeedPrefixSeed}` несут одинаковый `Deferred` и одинаковый re-poke; отличаются только «свежими» побочными эффектами | `executor.rs:595-619`, `:1816-1857` | один вариант + `enum ParkReason`; логика `defer_if_needed` сокращается вдвое |
| B-4 | Шесть одинаковых сборок `RethCommitteeSource::new(RethStakingStateReader::new(..), chain_id, closure finalized_hash)` | `dpos.rs:1718-1732`, `:1799-1813`, `:2495-2505`, `:2580-2594`, `:3077-3091`, `:3284-3294`, `:3352-3366`, `:3605-3619` | один конструктор-хелпер; `finalized_hash`-замыкание в одном месте |
| B-5 | Два сидера граничного блока: `executor::seed_boundary_below_floor` берёт `b` и `b+1`, `outer.rs` — только `b`, с комментарием, что `b+1` больше не нужен | `executor.rs:2226-2281` vs `outer.rs:867-966` | одна функция над `BlockFetcher`; расхождение (b+1) снимается |
| B-6 | `LastCanonicalized.finalized_height` — хранимое состояние, которое после `reseed_forward` заведомо «over-claim» (`executor.rs:2295-2304`); `result_final` уже считается из `ordering_finalized` и `anchor` | `executor.rs:213-281`, `:3242-3259` | хранить только хэши FCU; высоты выводить; аккуратно с `update_head`-инвариантом |
| B-7 | `Inner` enum и `context` в `EpochEngine` нужны только byzantine-варианту (`engine.rs:112-127`, `:314`) | `engine.rs` | под `cfg(feature)`; в проде — просто `simplex::Engine` |
| B-8 | Один драйвер границы с `watch<u64>` вместо цикла на каждый вызов `enter_boundary` (C-15) | `dpos.rs:2149-2283` | средняя: сохранить семантику «нет give-up» и панику как счётчик |
| B-9 | `boundary_hook` получает `OrderBlock` целиком (клон до 4 MiB на блок, `application.rs:1026`), читает только `height` | `application.rs:262`, `dpos.rs:2285-2288`, `:3389-3396` | сигнатура `Fn(u64)`; нулевой риск |
| B-10 | `initial_head` вычисляется дважды: `derive_cold_start_heights` после прыжка (`dpos.rs:1912-1913`) и повторно `canonical_state.chain_info()` в `build` (`outer.rs:1036-1061`) с собственным дискриминатором | `outer.rs:1021-1061` | одно место; убрать `canonical_state` из `OuterBuilder` |
| B-11 | `CertInlet.schemes` — второй реестр схем рядом с `EpochSchemeProvider`, с собственной ретенцией и эвикцией | `cert_inlet.rs:401-403`, `:696-814`, `:871-873` | использовать провайдер marshal'а; цена — монотонность `register` (нельзя эвиктить после verify-fail) |
| B-12 | `probe_fast_left`/`FRONTIER_PROBE_FAST_BURST` гистерезис + два интервала | `executor.rs:396-413`, `:1519-1531` | заменить на «пробовать, пока tip заморожен», после переноса пробы в spawn (C-09) |
| B-13 | `ReJump.probe: Option` и `rotate: Option` для одного и того же upstream-хэндла; на follower `probe = None` только потому, что «инлет и так двигает tip» | `executor.rs:472-528` | сделать пробу свойством `CertUpstream` |

---

## Часть C. Покрытие

Продакшн-строки: от начала файла до первого `#[cfg(test)]` (для `cert_inlet.rs` плюс хвост
`2885-3301` после тестового модуля; для `byzantine.rs` весь файл собирается только под `test`/feature).

| Файл | Строк | Продакшн | Прочитан | Находок | Причина отсутствия находок (по коду) |
|---|---|---|---|---|---|
| executor.rs | 12187 | 3833 | целиком (тесты: фикстуры `:3995-4480`, guard#2 `:7430-7612`, список всех тестов) | C-02, C-03, C-06, C-09, C-17, C-21 | — |
| cert_inlet.rs | 3301 | 1377 | целиком (тесты: список + фикстура) | C-05, C-24, C-27 | — |
| epoch_manager.rs | 3185 | 1825 | целиком (тесты: список) | C-04, C-10, C-13, C-16, C-18, C-22, C-25 | — |
| application.rs | 2648 | 1206 | целиком (тесты: список) | C-23 | — |
| cold_start_jump.rs | 1997 | 957 | целиком (тесты: список) | C-01, C-08, C-10 | — |
| spec_exec.rs | 129 | 129 | целиком | C-03 | — |
| order_block.rs | 912 | 464 | целиком | C-07, C-26 | — |
| cert_follow.rs | 212 | 212 | целиком | C-10 | — |
| outer.rs | 2017 | 1745 | целиком (тесты: список) | C-07, C-18, C-27, C-28 | — |
| engine.rs | 327 | 327 | целиком | C-11, C-13, C-16 | — |
| dpos.rs | 5117 | 3954 | целиком (тесты: список) | C-01, C-12, C-15, C-19, C-20 | — |
| epocher.rs | 170 | 80 | целиком | 0 | чистая арифметика с `checked_*`; `containing` возвращает `None` ниже origin и при переполнении; совпадает с `epoch_of_block`/`is_epoch_boundary` из staking-reader (та же формула `(n−activation)/interval`) |
| weighted_vrf.rs | 739 | 246 | целиком | 0 | входы — σ предыдущего view (нотаризация/нуллификация/финализация несут один σ на раунд, `CW:voter/state.rs:310,328,366`), либо детерминированный fallback с prefix-free доменом; `saturating_add` держит `cum` монотонным; `total == 0` ⇒ равномерно; `weights: None` ⇒ ошибка, не деградация; смещение `mod` ≤ total/2^256 |
| plane_upstream.rs | 404 | 343 | целиком | C-01, C-05, C-27 | — |
| feed_sink.rs | 50 | 50 | целиком | 0 | безусловный `ack.acknowledge()` до/вместо отправки; ошибки `send` игнорируются намеренно (пассивный наблюдатель); не может создать backpressure или потерять ack |
| executed.rs | 169 | 64 | целиком | 0 (doc — E-3) | три исхода полностью покрыты тестами `:117-168`; `best_block_number` — атомик, ветка `Err` недостижима |
| fault.rs | 326 | 259 | целиком | 0 (B-1) | таксономия без логики; `From<eyre::Report>` ⇒ `Corruption` — громкая сторона |
| extra_data.rs | 233 | 137 | целиком | 0 | ровно 3 байта, версия, диапазон `accused` проверяются; `leader_index` без диапазона, но сверяется с `expected` в `production_record_ok` (`application.rs:227-235`), а `IndexExceedsWireFormat` закрывает >255 |
| scheme.rs | 78 | 78 | целиком | 0 | два адаптера над `EpochCommittee::from_pairs`/`build_verifier`; дубли ключей ⇒ `Err`/`None`, паники нет |
| digest.rs | 95 | 81 | целиком | 0 (doc — E-4) | фиксированные 32 байта, `Read` через `<[u8;32]>::read`; `Random` только в тестах (проверено grep'ом автора, повторно не проверял) |
| timeouts.rs | 148 | 107 | целиком | 0 | инварианты дублируют `CW:simplex/config.rs:161-197` и `voter/actor.rs:136-138` (проверено); значения фиксированы, вызывающий `expect` в `outer.rs:806-808` |
| sync_metrics.rs | 859 | 606 | целиком | C-14, C-21 | — |
| byzantine.rs | 339 | ~200 (только под feature) | целиком | 0 (doc — E-5) | не в продакшн-сборке (`lib.rs:34-35`); эквивокатор не проверяет отправителя и подпись входящего голоса — намеренно для devnet |
| lib.rs | 102 | 102 | целиком | 0 | реэкспорты и три константы; `REPLAY_BUFFER`/`WRITE_BUFFER` общие для marshal и движков |

Итого продакшн-строк: ≈ 18 400.

---

## Часть D. Требует эксперимента или контракта

| # | Вопрос | К каким находкам |
|---|---|---|
| D-1 | reth: после FCU `finalized = X`, где X никто не отдаёт по devp2p, отменяет ли следующий FCU с известными хэшами запущенный pipeline-backfill, или EL отвечает `SYNCING` бессрочно | C-01, C-06, C-08 |
| D-2 | commonware p2p: `update.latest.primary` в `buffered::Engine` — это весь набор из `track(epoch, …)` или только его «primary»-часть; какова семантика `Set{primary,…}` | C-07 |
| D-3 | commonware simplex: при кратком разрыве узел получает `Finalization(v)` без предшествующей `Notarization(v)`? (стенд: отрезать валидатор на 2–3 с и смотреть `dpos_executor_eager_finalized_derive_total{outcome="miss"}` + hold) | C-03 |
| D-4 | commonware resolver: наказывает ли `deliver == true` с неверным содержимым пира; сохраняет ли `PrioritySet` предпочтение самому быстрому между sweep'ами | C-05, C-01 |
| D-5 | Контракт: стоимость записи в реестр (`getRegistryWithKeys`) — если регистрация не требует стейка, C-04/C-07 достижимы без капитала | C-04, C-07 |
| D-6 | Контракт: может ли `getEpochCommitteeWithStakes(E)` измениться после первого чтения (эпох-заморозка) — на этом стоит `EpochSchemeProvider::register` «другой комитет ⇒ отказ» и кэш `CertInlet.schemes` | B-11, C-18 |
| D-7 | reth: `block_number(hash)` для заголовка, оставшегося в `HeaderNumbers` от отброшенного сайдчейна, — `Some`? Тогда `holds(l1)` (`cold_start_jump.rs:607-613`) может подтвердить чекпойнт на неканонической ветке | C-01 (L1-арм) |
| D-8 | Стенд: C-02 сценарий 2 — nullify после спекуляции при отставании executor'а на ≥K (искусственная задержка derive) → ожидаемый ложный `SafetyHalt` | C-02 |

---

## Часть E. Расхождения doc-комментариев с кодом

| # | Где | Комментарий утверждает | Код |
|---|---|---|---|
| E-1 | `executor.rs:3121-3130` | guard #2 — «ONLY code-proven result-divergence detector on the catch-up path», «caught IMMEDIATELY» | читает канонический хэш до FCU: `None` на догоне (проверка пуста), спекулятивный сиблинг при отставании (ложный halt) — C-02 |
| E-2 | `epoch_manager.rs:1188-1198`, `engine.rs:185-187`, `:236-237` | «the reconciler aborts this engine on its next reconcile» для `RotatedKey` | такой ветки нет (`epoch_manager.rs:1041-1074` снимает только по `share_probe`) — C-13 |
| E-3 | `executed.rs:11-12`, `:147` | «signer's 3-consecutive-error self-shutdown (`MAX_CONSECUTIVE_ON_FINALIZED_ERRORS`)» | константы нет в crate; `dpos.rs:2110-2115` заменил shutdown на gauge и retry-forever |
| E-4 | `digest.rs:1-5` | «The consensus digest IS the EVM block hash» | `Digest` = keccak над кодировкой `OrderBlock` (`order_block.rs:150-152`), не EVM-хэш |
| E-5 | `byzantine.rs:143-152` | ссылается на гейт `engine.rs: can_sign = member_signer.is_some() && can_sign_locally` | в `engine.rs` такого нет; гейт живёт в `Randomness::share_probe`/`signer_scheme` |
| E-6 | `order_block.rs:126-127` (`result`) | «`B256::ZERO` while `height < anchor + K`» | окно ключуется на `dpos_activation_block`, не на anchor узла (`application.rs:553`, `executor.rs:3138`, `Config` doc `:670-677`) |
| E-7 | `cold_start_jump.rs:42-47`, `:640-643` | landing «shares the tip's epoch (interval ≥ 32 ≫ K)» как основание читаемости комитета | не проверяется нигде; при `tip` в первых K блоках эпохи landing лежит в предыдущей эпохе. Комитет всё равно читаем (закоммичен на эпоху раньше), но обоснование в комментарии неверно |
| E-8 | `outer.rs:1002-1009` | marshal «transiently ack-drops an unregistered-epoch height and re-requests it via try_repair_gaps» | доставка без схемы отвечает `true` и не сохраняется (`CW:marshal/core/actor.rs:963-972`); повторный запрос — только если гап остаётся в `try_repair_gaps` [ГИПОТЕЗА: не прослежен] |
| E-9 | `executor.rs:530-538` | `Deferred` — «The ONLY park in the executor» (guard #2) | тот же слот паркует ещё `NeedParentVisible` и `NeedPrefixSeed` (`:3063-3068`, `:3088-3093`) |
| E-10 | `executor.rs:1080-1082` | «At cold-start … safe == finalized == head == anchor» | `head = initial_head` может быть выше finalized (migrated restart, `outer.rs:1054-1058`) |
| E-11 | `executor.rs:2226-2244` vs `outer.rs:869-876` | executor сидирует `b` и `b+1` «both-or-neither»; outer: «It used to fetch `b + 1` … that gate now compares against the agreement artifact» | два сидера расходятся по набору высот (B-5) |
| E-12 | `epoch_manager.rs:56-60` (`Role`) | «no epoch-boundary wait» для промоции | верно; но `spawn_engine == false` без записи в `deferred_spawns` (`:1402-1410`) даёт ожидание до следующего edge — не отражено |
| E-13 | `cert_inlet.rs:286-287` | `verified` «send_lossy … the durability-ack `-> bool` variant is a NEWER upstream rev» | в закреплённой ревизии `Mailbox::verified` действительно `send_lossy` (`CW:marshal/core/mailbox.rs:294`); совпадает — оставлено как проверенное |
| E-14 | `spec_exec.rs:60-61`, `timeouts.rs:5-6` | ссылки `voter/actor.rs:529-531` и `:136` | совпадают с закреплённой ревизией (`.report(Activity::Notarization)` ≈ `:529`, `panic!` при `leader > certification` `:136-138`) |
| E-15 | `plane_upstream.rs:21-26` | «A malicious peer can at most inflate the tip → … FAILS CLOSED at `verify_jump_authenticated`» | проверка читает комитет из синхронизированного с этого же пира состояния (C-01); «fail closed» не выполняется |

---

## Часть F. Расхождения с UNDERSTANDING.md

| # | UNDERSTANDING | Уточнение по коду |
|---|---|---|
| F-1 | §6.2 [испр. аудит]: «Если на высоте h спекулятивно исполнен другой блок, guard #2 видит его и объявляет ForkSafety» — подано как корректное поведение | это ложный halt (C-02, сценарий 2); а в обычном догоне без спекуляции guard пуст (сценарий 1) |
| F-2 | §5.4: источники σ на валидаторе — `Notarization` + backfill при `Hybrid`; «при резолвере `Plane` … захвата нет» | верно, но `Plane` в продакшне недостижим: node всегда передаёт `Some(upstream)` (`crates/node/src/dpos.rs:2026-2029`, `dpos.rs:2449-2453`) — B-2. Пропущено следствие для живого валидатора: финализация без нотаризации → hold до re-jump (C-03) |
| F-3 | §4 [испр. аудит]: `corroborate_frontier` считает отправителей без фильтра членства | подтверждено и доведено до последствия: f+1 записей реестра = verify-only навсегда (C-04) |
| F-4 | §8, строка FRONTIER: «верификация сертификата — в потребителе» | `deliver` ещё и не связывает ключ с содержимым (C-05); resolver считает fetch выполненным |
| F-5 | §12.4/§8 (Upstream RPC): порядок `sync_to` → аутентификация назван «круговым и поздним» | добавлено: на plane-пути «upstream» — любой из ≤4096 пиров, выбираемый по задержке; L1 на валидаторе не передаётся; последствие — persist-маркер halt (C-01) |
| F-6 | §11.3: `register_scheme` до `WeightedVrf::try_new` | подтверждено; добавлено, что `spawn_engine == false` не ставит `deferred_spawns` (C-16) |
| F-7 | §3: `SafetyHalt` «маркер пишется `std::fs::write` без fsync и после установки защёлки» | подтверждено (C-14) |
| F-8 | §1.4: `byzantine.rs` «devnet `VoteEquivocator` (feature)» | добавлено: doc-комментарий ссылается на несуществующий гейт в `engine.rs` (E-5); в проде модуль не собирается |
| F-9 | §12.7: `pending_boundary` single-slot недостижим при `interval ≥ 13` через `MAX_PENDING_ACKS = 16` | не перепроверялось (staking-reader вне объёма); в executor ack удерживается ещё и `awaiting_seed`-hold'ом (C-03), что только сужает окно — вывод не меняется |
| F-10 | §6.1: «loop ≤40×25ms» | 41 итерация (`0..=polls`), C-23 |
| F-11 | §11.8: `reseed_forward` in-memory `finalized_height = landing` при FCU `finalized = floor` | подтверждено; предложено вывести высоту вместо хранения (B-6) |
| F-12 | Отсутствует: партиции `consensus_epoch_{E}` не удаляются | C-11 |
| F-13 | Отсутствует: `enter_boundary` порождает цикл на каждый вызов | C-15 |
