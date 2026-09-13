# E5-0a-A — строка 5.0а, заход А: стендовый `CertInlet`

Ветка `djadjka/dpos-reth-2.2-squashed`, база HEAD `bec013a7`. Файлы на запись:
`crates/dpos/consensus/src/testbed/{stand.rs, mod.rs, cert_inlet_tests.rs}` (новый) —
ничего вне списка. Не коммитил, в индекс ничего не ставил.

## §0. Прямые ответы

1. **Форма `CertInletCfg`.** [KNOWN] `StandConfig` получил РОВНО одно новое поле —
   `pub cert_inlet: Option<CertInletCfg>` (`testbed/stand.rs:209`), `None` в
   единственном конструкторе `StandConfig::honest` (`:495`, тело с `:472`; `live` строится над ним
   через `..Self::honest`), ни одно существующее поле не тронуто. Сам тип —
   `CertInletCfg { nodes, source, tee }` (`stand.rs:227-239`; третье поле `tee` —
   проход 3, 5.0а-Д-5, В§0(2)). Предложение оркестратора было `{ nodes }`; остальные
   поля лежат ВНУТРИ нового типа, поэтому условие «ровно одно новое поле
   `StandConfig`» держится. Для ДВУХ плоскостных источников адресность по-прежнему НЕ
   в этом типе: отвечает собственная плоскость фронтира узла (`CountingUpstream` над
   продакшн-`PlaneUpstreamHandle`), а кто именно — существующие
   `upstream_only_link` / `upstream_source_only_for` (`stand.rs:161-181`), и тест (2)
   ниже пользуется именно ими. Третий источник (`PeerArchive { from }`, проход 3)
   называет донора сам — он читает его архив напрямую, и никакого линка в этом нет.
2. **Откуда marshal и как дождался слота.** [KNOWN, якоря перепинены на финальное
   дерево прохода 3 — A-10] Задача НЕ читает СВОЙ `marshal_slot`: она берёт ВТОРОЙ
   клон из уже собранного движка — `outer.marshal_mailbox()` (`stand.rs:2733`),
   ровно как продакшн-follower берёт два
   клона подряд (`consensus/src/dpos.rs:3578-3579`), а `MarshalMailbox` — это
   `marshal::core::Mailbox` (`outer.rs:294`), `marshal_mailbox()` возвращает клон
   (`outer.rs:1133`). Спавн стоит ПОСЛЕ `outer.build` и после
   `marshal_slot.set(...)` (`stand.rs:2678-2684`), поэтому «ждать слот» нечего:
   хэндл — факт, а не обещание. Это 5.0а-Д-1 (§1): вместо ожидания слота — порядок
   сборки, который делает ожидание невозможным.
   **Уточнение прохода 3:** это верно для СВОЕГО marshal-а. У источника
   `CertInletSource::PeerArchive` есть второй хэндл — marshal ДОНОРА, и его слот
   задача действительно ждёт поздней привязкой (`stand.rs:2819-2829`), потому что
   донор может быть ещё не собран. Слоты теперь создаются в `drive`, по одному на
   узел (`stand.rs:1504-1505`), и `build_node` берёт свой как `marshal_slots[i]`
   (`stand.rs:2243`) — см. часть В§0(4).
3. **Фидер.** [KNOWN] Две формы (`CertInletSource`, `stand.rs:309-338`), обе через
   один и тот же `CertUpstream`:
   * `NextAboveTier` (по умолчанию) — `up.get_finalization(Height::new(chain.tip() + 1))`
     (`stand.rs:2800-2803`): следующая высота над собственным тиром-F, прочитанная у
     `FakeChain`, а не у marshal-а (лишнее сообщение в его select-петлю на такт
     меняет прогон — Д-81, `stand.rs::StandConfig::marshal_tip_series`);
   * `Frontier` — `up.get_latest()` (`stand.rs:2804`): продакшн-вход inlet-а
     (`node/src/cert_inlet.rs:74`, петля `finalized_rx.recv()`), и единственный
     сертификат, который ещё получает узел с остановленным исполнением;
   * `PeerArchive { from }` (добавлен проходом 3, A-05) — `pair_at` по marshal-у
     ДОНОРА (`stand.rs:2819-2843`), без плоскости и без её гейта `deliver`. Три
     формы, не две; первая из двух, названных строкой плана, наконец построена.
   **ИСПРАВЛЕНО в проходе 3 (A-01) — прежний текст был неверен.** Оба плоскостных
   вызова идут в `PlaneUpstreamHandle::fetch_one` (`plane_upstream.rs:610-662`),
   который ждёт либо доставку, либо СВОЙ таймаут `FRONTIER_FETCH_TIMEOUT = 8 s`
   (`:169`, `:623-626`) — но `None` стоит этот таймаут ТОЛЬКО на таймаутном пути. На
   пути шага (5) `deliver` дропает запись вместе с её oneshot-sender-ом
   (`plane_upstream.rs:485-489`), и арм `answer = rx` (`:622`) резолвится в `Err`
   НЕМЕДЛЕННО, за нулевое виртуальное время (комментарий кода это и говорит,
   `:641-647`). Значит цикл ограничен round-trip-ом симулированной сети
   (`StandConfig::latency` = 10 мс), а не таймаутом. Измерено мною на
   keyless-прогоне: 3332 ingest-а + 840 дропов плоскости за 159,828 с виртуального
   времени ≈ 26 итераций/с ≈ 38 мс на итерацию ≈ два хопа плюс такты планировщика.
   Своего `sleep`/таймера на плоскостных армах по-прежнему нет, и подписаться не на
   что: «upstream теперь держит h» ни один шов стенда событием не публикует. Ложное
   обоснование, стоявшее тем же текстом в коде, переписано по факту
   (`stand.rs:2777-2793`). У ТРЕТЬЕГО арма (`PeerArchive`, проход 3) пацинг свой —
   `c.sleep(POLL)` на промахе, причина в В§2 (5.0а-Д-6).
4. **Тик tee.** [KNOWN; в проходе 3 это стало ОДНОЙ ИЗ ДВУХ разводок — A-03]
   Разводка `TeeWiring::Observed` — приём оркестратора дословно: стенд создаёт СВОЙ
   канал `(tee_tx, tee_rx)` (`stand.rs:2726-2732`), `tee_tx` уходит в
   `LiveFrontierTee { dkg_height_tx: tee_tx, plane_clock }` (`stand.rs:2760-2763`), и
   САМА задача inlet-а сразу после возврата `ingest` дренирует `tee_rx` через
   `try_recv` в цикле, пишет высоты в наблюдаемое и пересылает их в НАСТОЯЩИЙ
   `dkg_height_tx` тем же лоссовым `try_send` + `note_height_drop`, что и сам inlet
   (`stand.rs:2861-2868`). Отдельной задачи-реле нет. Дренаж стоит ПОСЛЕ `ingest` и
   корректен именно там: `ingest` дёргает tee синхронно и только на чистом исходе
   (`cert_inlet.rs:712-717` — после `observe_certificate` `:690` и `observe_cert`
   `:706`), поэтому всё, что лежит в канале после возврата, принадлежит только что
   поданному сертификату. **Чего эта разводка НЕ даёт** — продакшн-ПОРЯДКА: пересылка
   ложится после `verify_block`/`report_finalization` того же `ingest`
   (`cert_inlet.rs:736`/`:741`, `:744`/`:745`), тогда как в продакшне tee встаёт в
   очередь ДО них. Для порядка добавлена вторая разводка `TeeWiring::Production`
   (`stand.rs:2726-2732`, арм `Production`): tee получает НАСТОЯЩИЙ `dkg_height_tx`,
   порядок — продакшн по построению, а список высот при ней пуст. Почему обе сразу
   получить нельзя — В§0(2).
   **Чем доказано, что высота доходит до `DkgActor`:** (а) пересылка идёт в тот же
   `dkg_height_tx`, чей приёмник `dkg_height_rx` отдан в `ValidatorInputs::heights`
   (`stand.rs:2543`) — один клон канала, не второй канал; (б) в тесте (1)
   `dpos_dkg_height_drops_total == 0`, то есть на пересылке не потеряно ничего; (в)
   `dpos_dkg_clock_height = 72 >= max(tee) = 71`, а этот гейдж пишется ровно в одной
   точке — `on_height` актора после клампа (`beacon/actor.rs:1175`,
   `sync_metrics.rs:358`). НЕ эксклюзивно: на здоровом узле тот же канал кормит
   `FluentApp::report(Update::Tip)` (`application.rs:1081`), поэтому (в) говорит
   «часы видели эту высоту», а не «tee — единственный фидер» (§4).
5. **Наблюдаемые.** [KNOWN] `Outcome::cert_inlet: Vec<Option<CertInletFacts>>`
   (`stand.rs:892`), `None` — узел без inlet-а. Поля `CertInletFacts`
   (`stand.rs:405-412`), живые хэндлы — `CertInletObs` (`stand.rs:343-382`):
   * `ingests` — сколько сертификатов отдано в `ingest` (ПОПЫТКИ; каждый исход
     `ingest` — skip, `cert_inlet.rs:520`). Пинуется в (1) как `>= TARGET/2`, в (2)
     как слагаемое арифметики фолтов, в (3) как `> 0`.
   * `rotations` — вызовы `RotateUpstream`, единственный внешне видимый эффект
     `record_data_fault` при `consecutive_faults >= MAX_UPSTREAM_FAULTS = 3`
     (`cert_inlet.rs:758-772`, `:91`). Пинуется `== 0` в (1) и (3), `== faults/3` в (2).
   * `tee_heights` — высоты, прошедшие tee. Пинуется в (1) как ТОЧНОЕ равенство
     `(1..=ingests)` (значит: каждый ingest был чистым, и tee срабатывает ровно один
     раз на чистый ingest), в (2) — как разделитель «подделанные не teed», в (3) — как
     верхняя граница окна чтения комитета.
   * `defers` — счётчик `dpos_cert_inlet_committee_read_deferred_total{committee_not_committed}`,
     взятый не из экспозиции, а из самой `Family`, отданной inlet-у через
     `with_committee_read_deferred_metric` (`cert_inlet.rs:428`; follower в продакшне
     подключает её так же, `dpos.rs:3679`). Пинуется `== 0` во всех трёх тестах — и в
     (3) это ОБЪЯСНЁННЫЙ нуль, см. п. 9.
   * `consecutive_faults` **приватен** (`cert_inlet.rs:343`), геттер НЕ добавлялся.
     Наблюдается арифметически: в тесте (2) `defers == 0`, значит каждый ingest либо
     чистый (он teed), либо data-фолт, значит `faults = ingests − tee.len()` точно; все
     фолты идут одной непрерывной серией (фидер идёт строго вверх, а единственный
     by-height источник жертвы — форжер), значит счётчик сбрасывается ТОЛЬКО на пороге и
     `rotations == faults / 3` — равенство, а не `> 0`. Измерено: `faults = 6`,
     `rotations = 2`.
   * dkg-часы `PlaneClock` отдельным полем НЕ добавлялись: гейдж уже зарегистрирован
     на контексте узла (`sync_metrics.rs:321`) и читается существующим
     `Outcome::metric(i, "dpos_dkg_clock_height")` (`stand.rs:1055-1063`).
6. **Роль «валидатор с отстающим EL и живым inlet-ом».** [KNOWN] Нового варианта
   `Role` НЕ введено, и в `StandConfig`/`fakes.rs` для него ничего не добавлено.
   Существующая комбинация даёт всё: узел вне `committee[E]` при
   `PeerSet::Committee { upstream_link: true }` теряет консенсус-плоскость и
   сохраняет `FRONTIER_CHANNEL` (`stand.rs:453-463`, применение —
   `stand.rs:1624-1664`: `sever = true`, `sever_upstream = !upstream_link`), что тест
   `(4c)` уже пинует как «единственный путь к цепи — плоскость upstream».
   Отставание УДЕРЖИВАЕТСЯ само, без нового переключателя: вне `committee[2]` узел не
   держит `PK_2` (R-121/R-122), не может проверить ни одной σ эпохи 2, не исполняет ни
   одного блока эпохи 2 — и стоит на границе эпохи 1 весь прогон, а `re_jump_threshold`
   остаётся стендовым `u64::MAX`, поэтому ре-джамп, который иначе вынес бы его вперёд,
   не армируется. Измерено в тесте (3): `heights[4] < 64`, `jump_calls[4] == []`,
   `artifacts[4]` без эпохи 2.
7. **Тесты.** Живут в НОВОМ файле `testbed/cert_inlet_tests.rs`, подключённом из
   `testbed/mod.rs:86-90`. Обоснование одной строкой: `testbed/tests.rs` — рабочая
   поверхность строки 5.1, которая идёт параллельно по тому же дереву, а `mod tests`
   внутри `stand.rs` смешал бы харнес с тестами над ним.
   | тест | что утверждает | фальсификатор | «красный до правки» / мутация |
   |---|---|---|---|
   | `an_inlet_on_a_healthy_member_verifies_every_height_it_is_fed_and_tees_it` | второй продюсер в marshal безвреден (lockstep, без halt, без ERROR); `tee.len() == ingests` (все три фолт-ветки и ветка defer НЕ взяты); `tee == (1..=ingests)`; на пересылке в `dkg_height_tx` не потеряно ничего и часы `DkgActor` дошли до верхней teed-высоты | фолт/defer на честном прогоне; дыра или повтор в tee; дроп на пересылке; часы ниже teed-верха; любая ротация; потеря lockstep / halt / ERROR | мутация не нужна: до захода теста не было (`git grep CertInlet -- testbed` пуст на `bec013a7`), сам прогон и есть «после» |
   | `a_forged_seed_slot_costs_the_upstream_a_rotation_once_the_epoch_key_is_held` | подделанный σ-слот при ЕСТЬ-ключе валит BLS в `ingest`, это data-фолт, и на третьем подряд срабатывает `rotate()`: `rotations == faults / 3` ТОЧНО | роль не подделала ничего / подделала равное / тронула multisig-половину; жертва без артефакта эпохи 2; `rotations == 0`; ротаций не `faults/3`; `faults != ` число взятых подделок | измерено: мутация не требовалась — это НОВОЕ плечо (ротация по `record_data_fault`), которого в стенде не существовало; первый прогон дал `rotations = 2` при `faults = 6` |
   | `a_keyless_admission_is_all_the_plane_lets_an_outrun_inlet_see` | отстающий узел без `PK_2` ДОПУСКАЕТ сертификаты эпохи 2 с непроверенной σ (`dpos_cert_vote_only_admissions_total > 0`) и это НЕ стоит ротации; верхняя граница того, что он вообще видит, — `last(epoch(anchor)+2)`, и её ставит `deliver` плоскости, а не inlet | узел не стоит ниже границы эпохи 2 / ре-джампнул / держит `PK_2`; нет ни одного keyless-допуска; любая ротация; tee ВЫШЕ границы окна (тогда `deliver` пропустил неаутентифицируемое) или НИЖЕ верхней эпохи окна (тогда фидер остановило что-то другое); ноль дропов плоскости; halt комитета | **был красный**: первая версия утверждала `defers > 0` и упала на `defers == 0` при `ingests = 3332` и tee, обрывающемся на 127 при комитете на 160 — это и есть находка п. 9; ассерт переписан под механизм |
8. **Прогоны.** См. §2 — verbatim. `cargo test -p fluentbase-consensus --lib` —
   зелёный, `688 passed; 0 failed` = база 686 + 2 теста без фичи (третий под
   `dpos-devnet-byzantine`, в этом наборе не собирается). ×3 по новым тестам — детерминизм
   байт-в-байт. Остальные ворота у оркестратора.
9. **Что оказалось неверным по коду.** [KNOWN] Пункт 5(б) постановки — «фикстура
   R-129: `Committee::scheme(epoch)` отвечает `None` ⇒ `ingest` делает НЕ-фолтовый
   defer, пин на счётчик defer > 0» — **в стенде недостижим, и причина не в
   фикстуре**. Два независимых запрета:
   * *арифметика фидера by-height.* Окно чтения модуля — `[epoch(anchor) − 8,
     epoch(anchor) + 2]` (`committee/store.rs:218-224`), нижняя граница по
     `commit_height(E) = start(E−2)` (`committee/mod.rs:616-622`), а якорь модуля —
     это `chain.tip()` (`stand.rs::StandAnchor`), то есть та же высота, от которой
     фидер берёт `tip + 1`. Значит эпоха сертификата всегда `≤ epoch(anchor) + 1`, и
     ветка `:618-640` недостижима ПО ПОСТРОЕНИЮ.
   * *гейт плоскости.* С источником `Frontier` эпоха сертификата уходит сколь угодно
     высоко, но такой сертификат до inlet-а НЕ ДОХОДИТ: `FrontierHandler::deliver`
     классифицирует ровно те же три отказа на слой выше (шаг (4),
     `plane_upstream.rs:447-449`) и шаг (5) ДРОПАЕТ ответ, тикая
     `dpos_frontier_dropped_total{reason}` и разрешая ждущий `fetch_one` в `None`
     (`plane_upstream.rs:485-489`). Измерено: tee жертвы обрывается на 127 =
     `last(epoch(anchor)+2)` при комитете на 160, и `dpos_frontier_dropped_total` не
     нуль.
   В продакшне такого гейта перед inlet-ом нет — WS-поток отдаёт что пришло
   (`node/src/cert_inlet.rs:74`), — поэтому ветка defer остаётся продакшн-режимом.
   Тест (3) не выброшен: он ПИНУЕТ этот механизм (граница окна + счётчик дропов +
   `defers == 0` как следствие), так что нуль объяснён, а не замолчан. Что осталось
   строке 5.2 — §5.
   Мелочи того же рода: (а) постановка называет `MarshalSink` источником marshal-а «из
   `marshal_slot`» — продакшн берёт второй клон у движка (`dpos.rs:3578-3579`), слот
   в стенде нужен только serve-стороне плоскости; (б) `with_committee_read_deferred_metric`
   в списке билдеров постановки нет, но без него счётчик defer не наблюдаем вообще
   (`Family::default()` не зарегистрирована, `cert_inlet.rs:190-196`), а follower в
   продакшне его подключает (`dpos.rs:3679`) — подключил (5.0а-Д-3).
10. **Запрещённого сделано — ноль.** Писал только в
    `testbed/stand.rs`, `testbed/mod.rs`, `testbed/cert_inlet_tests.rs` (новый) и в
    этот журнал. `testbed/tests.rs`, `byzantine_roles.rs`, `preconditions.rs`,
    `committee_tests.rs`, `beacon/**`, `cert_inlet.rs`, `plane_upstream.rs`,
    `executor.rs`, `epoch_manager.rs`, `committee/**`, `node/**`, `p2p/**`, `bins/**`,
    `devnet/**`, `contracts/**`, `Cargo.toml`, `.claude/**` — не тронуты
    (`git status --short crates` ниже, §2). Ни одного `#[allow]`, `todo!`, таймера
    вместо события, поллинга; `unwrap` — только `Mutex::lock().unwrap()` и
    `.expect()` в тестовом коде. Не коммитил, `git` — только на чтение.
11. **Слабое место — что НЕ проверил.** (а) Эксклюзивность tee: что dkg-часы
    двинула ИМЕННО пересылка, не проверено — на здоровом узле тот же канал кормит
    `application.rs:1081` (см. §4). (б) Ветка defer inlet-а (`cert_inlet.rs:618-640`)
    не исполнена ни разу — только доказано, почему стенд её не достаёт. (в) Ветки
    фолта `epoch_bind` (`:545-568`, на валидаторском inlet-е no-op) и
    `payload != digest` (`:573-586`) не исполнены: единственная исполненная фолт-ветка —
    BLS verify (`:648-673`). (г) `observe_certificate`/`observe_cert` вызываются, но их
    вердикт по-прежнему `let _observed` — потребителя 5.0а не добавляет, значит
    «σ захвачена» не утверждается. (д) Я не гонял полный набор ворот (по решению
    пользователя он у оркестратора): подтверждаю только `--lib` и свои тесты.

## §1. Отклонения

* **5.0а-Д-1 — marshal берётся у движка, не из слота.** Постановка §2: «`marshal`
  берётся из `marshal_slot` ПОСЛЕ `outer.build`; задача должна дождаться слота».
  Сделано иначе: `outer.marshal_mailbox()` вторым клоном (`stand.rs:2733`), спавн
  после `marshal_slot.set` (`:2682`). Причина: продакшн-follower делает ровно это,
  двумя клонами подряд (`consensus/src/dpos.rs:3578-3579`), а порядок сборки
  устраняет само ожидание — «дождаться» нечего, если хэндл уже есть. П и Д не
  меняет: слот остаётся у serve-стороны плоскости (`stand.rs::frontier_plane`),
  вторая дверь в marshal — та же.
* **5.0а-Д-2 — у `CertInletCfg` второе поле `source`.** Постановка §1 предлагала
  `{ nodes }`. Причина — находка §0 п. 9: с единственным источником by-height ветка
  defer недостижима по арифметике окна, а с единственным источником frontier
  недостижима непрерывная серия data-фолтов (ответы `Latest` в режиме
  `ForgeMode::SeedSlot` честны, `byzantine_roles.rs:889` — форжится только
  `FrontierKey::Finalized`), то есть тест (2) развалился бы. Нужны оба, и оба —
  формы ОДНОГО продакшн-входа: `Frontier` = продакшн-поток
  (`node/src/cert_inlet.rs:74`), `NextAboveTier` = то, о чём просит строка плана
  («`get_finalization(h)` выше своего tip-а»). Условие «ровно одно новое поле
  `StandConfig`» не нарушено: поле лежит внутри нового типа. Меняет ли П/Д: нет,
  это форма стендового шва.
* **5.0а-Д-3 — подключён `with_committee_read_deferred_metric`.** Постановка §2
  перечисляет билдеры и этого не называет. Причина: без него `defers` наблюдать
  нечем — `Family::default()` не зарегистрирована и инкремент инертен
  (`cert_inlet.rs:190-196`), а наблюдаемая по строке плана требуется. Форма —
  продакшн-follower-овская (`dpos.rs:3679`). `with_epoch_math`, `with_window`,
  `with_connection_token` НЕ подключены, как и велено (валидаторский inlet их не
  ставит, `node/src/cert_inlet.rs:84-88`).
* **5.0а-Д-4 — плечо «подделка допущена при отсутствии ключа» пинуется на ЧЕСТНОМ
  сертификате, не на подделанном.** Постановка §5(а) требует одним тестом: (1) при
  отсутствии `PK_E` подменённый серт допущен и узел продвинулся, (2) при пришедшем
  ключе тот же класс валит BLS и даёт ротацию. По коду (1) и (2) в одном прогоне на
  одном узле несовместимы: keyless-узел не может проверить ни одной σ эпохи 2,
  поэтому не исполняет ни одного её блока, поэтому его фидер НЕ входит в окно
  подделки (`FORGE_WINDOW = 64..=70`, `byzantine_roles.rs:480`) — он бесконечно
  переспрашивает первую высоту эпохи, а её роль никогда не подделывает (первый
  обслуженный ответ идёт в харвест, `byzantine_roles.rs:663-671`). Измерено: 3332
  ingest-а, tee = 1..127 с повторами, ни одной подделки. Разнесено на два теста:
  плечо (2) — `a_forged_seed_slot_costs_the_upstream_a_rotation_once_the_epoch_key_is_held`
  (жертва в комитете, ключ есть, шесть подделок подряд, две ротации); плечо (1) —
  `a_keyless_admission_is_all_the_plane_lets_an_outrun_inlet_see`
  (`dpos_cert_vote_only_admissions_total > 0`, `rotations == 0`). Что при этом НЕ
  доказано: что ПОДДЕЛАННЫЙ серт допускается при отсутствии ключа — это уже пинует
  существующий `a_forged_seed_slot_is_admitted_with_no_key_and_refused_when_the_key_lands`
  (`testbed/tests.rs:3288`) по пути фронтир+marshal; inlet добавляет к нему ротацию,
  а не допуск.

## §2. Прогоны (verbatim)

`git status --short crates .dpos-study` — в дереве только мои файлы плюс два чужих,
которые были изменены ещё до захода (`PLAN.md`, `E4-ORCHESTRATOR.md`; не трогал):

~~~
 M .dpos-study/PLAN.md
 M .dpos-study/history/E4-ORCHESTRATOR.md
 M crates/dpos/consensus/src/testbed/mod.rs
 M crates/dpos/consensus/src/testbed/stand.rs
?? crates/dpos/consensus/src/testbed/cert_inlet_tests.rs
~~~

**Детерминизм ×3** — `cargo test -p fluentbase-consensus --lib --features
dpos-devnet-byzantine testbed::cert_inlet_tests -- --nocapture --test-threads=1`,
три прогона подряд, строки байт-в-байт одинаковы:

~~~
--- RUN 1 ---
running 3 tests
test testbed::cert_inlet_tests::a_forged_seed_slot_costs_the_upstream_a_rotation_once_the_epoch_key_is_held ... (5.0а/Ex-21 keyed) heights=[102, 102, 102, 102, 102] forged=[65, 66, 67, 68, 69, 70] ingested_forged=[65, 66, 67, 68, 69, 70] teed_in_window=[64] ingests=102 rotations=2 defers=0 virtual=101.9s
test testbed::cert_inlet_tests::a_keyless_admission_is_all_the_plane_lets_an_outrun_inlet_see ... (5.0а/Ex-21 keyless) heights=[160, 160, 160, 63, 63] ingests=3332 keyless=1683 rotations=0 defers=0 teed_top=127 window_top=127 plane_dropped=840 virtual=159.828s
test testbed::cert_inlet_tests::an_inlet_on_a_healthy_member_verifies_every_height_it_is_fed_and_tees_it ... (5.0а/clean) heights=[72, 72, 72, 72] ingests=71 tee=[1..71] rotations=0 defers=0 dkg_clock=72 virtual=71.9s
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 694 filtered out; finished in 34.88s
--- RUN 2 ---   (идентично RUN 1)
--- RUN 3 ---   (идентично RUN 1)
~~~

Плюс более ранняя тройка (тот же набор, вывод фильтрован до итогов) —
`ok. 3 passed` × 3, `finished in 35.13s / 34.92s / 34.96s`.

Что именно измерено, к §0 п. 5:
* `(clean)`: 71 попытка, 71 чистый ingest, `tee == [1..71]`, `rotations = 0`,
  `defers = 0`, `dkg_clock = 72 >= 71`, дропов пересылки 0, ERROR-строк 0.
* `(keyed)`: подделаны высоты `65..70` (высоту 64 роль харвестит, не подделывает),
  ни одна из них не teed (`teed_in_window = [64]`), значит `faults = 102 − 96 = 6`,
  и `rotations = 2 = 6 / 3` — ТОЧНО порог `MAX_UPSTREAM_FAULTS`.
* `(keyless)`: комитет на 160 (эпоха 5), жертва стоит на 63 (эпоха 1), ре-джампов
  нет, артефакта эпохи 2 нет; 3332 ingest-а, `keyless = 1683` допусков с
  непроверенной σ, `rotations = 0`; верх tee `= 127 = last(1 + 2)` в точности равен
  верху окна чтения комитета, а `dpos_frontier_dropped_total{out_of_window,
  not_readable} = 840` — то самое, что дропнула плоскость и что inlet поэтому не
  увидел (§0 п. 9).

**ЦЕНА арма `Frontier`, цифрами (добавлено проходом 3, A-02/A-05; числа — мои
прогоны, часть В§0(6)).** 3332 ingest-а на 127 РАЗЛИЧНЫХ высот, то есть каждая
высота подана в `ingest` ≈ 26 раз, и каждая подача — полная BLS-проверка плюс два
сообщения в marshal (`cert_inlet.rs:744-745`). Причина не в фикстуре: `get_latest`
отдаёт то, что плоскость готова отдать, и у фидера нет своего курсора, так что
«одна и та же высота снова» — нормальный ответ. Тот же вопрос через новый вход
`PeerArchive` стоит **159 ingest-ов** (127 чистых + 32 defer-а, ни одного повтора —
курсор у фидера свой): 21× дешевле, и это ровно тот вход, на котором ветка defer
вообще достигается. `ingests` на арме `Frontier` поэтому нельзя читать как
«сколько сертификатов узел увидел» — только как «сколько раз он спросил».

**Полный `--lib` (обязательные ворота захода)** — `cargo test -p
fluentbase-consensus --lib`:

~~~
test result: ok. 688 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 171.96s
~~~

688 = база 686 (HEAD `bec013a7`, прогон оркестратора) + 2 моих теста, которые
собираются БЕЗ фичи; третий (`a_forged_seed_slot_costs_the_upstream_a_rotation_once_
the_epoch_key_is_held`) под `#[cfg(feature = "dpos-devnet-byzantine")]`, поэтому в
`--lib` без фичи его нет. С фичей набор стенда даёт `694 filtered out` + мои 3, то
есть все три видны только там. Ни один существующий тест не покраснел и ни одно
число в существующих тестах не менялось — это и есть отрицательный контроль §5(в)
постановки: при `cert_inlet: None` inlet не поднимает никто.

**`cargo fmt --check`** — чисто (после `cargo fmt -p fluentbase-consensus`; в дереве
переформатированы только мои три файла, см. `git status` выше).

**`cargo clippy -p fluentbase-consensus --lib --tests --features
dpos-devnet-byzantine`**:

~~~
(ни одной строки `warning:` / `error:` — фильтр `grep -E "^(warning|error)"` пуст)
~~~

**ВАЖНО (исправлено в части Б):** эта команда гоняет ОДНУ конфигурацию — С фичей — и
поэтому НЕ доказывала «мой код не добавил предупреждений». Без фичи он их добавлял
ровно одно (`unused import: Role`), и это поймали ворота оркестратора. Актуальное
состояние и обе команды — часть Б ниже. Базовые два чужих (`large_enum_variant` в
`node`, `MutexGuard` в `staking-reader`) — в других крейтах и этими командами не
задеты.

## §3. Логические коммиты

1. `test(testbed): stand up the production CertInlet as a second marshal producer`
   — `testbed/stand.rs` (поле `StandConfig::cert_inlet`, `CertInletCfg`,
   `CertInletSource`, `CertInletObs`/`CertInletFacts`, задача в `build_node`, сбор в
   `Outcome`).
2. `test(testbed): cert-inlet tests — clean path, tee, data-fault rotation, keyless admission`
   — `testbed/cert_inlet_tests.rs` + `mod` в `testbed/mod.rs`.
3. `docs(dpos): journal of Э5 5.0а` — `.dpos-study/history/E5-0a-A.md`.

Группы 1 и 2 по отдельности НЕ собираются: `mod cert_inlet_tests;` без полей
`Outcome`/`StandConfig` не компилируется, а поля без читателя дают
`dead_code`-предупреждение (проверено: до появления тестов сборка выдавала
`field cert_inlet is never read`). Значит 1+2 — один коммит либо 1 с `#[allow]`,
чего постановка не разрешает; выбран один коммит.

## §4. Где проверка слабее всего

1. **Tee не пинован эксклюзивно.** `dpos_dkg_clock_height >= max(tee)` истинно и
   без пересылки: тот же `dkg_height_tx` кормит `FluentApp::report(Update::Tip)`
   (`application.rs:1081`). Что пинуется точно — «пересылка не лоссовая»
   (`dpos_dkg_height_drops_total == 0`) и «tee срабатывает ровно один раз на чистый
   ingest» (`tee.len() == ingests`). Эксклюзивный пин требует узла, у которого
   ordering-tip НЕ двигается, а inlet двигается — это строка 5.4 (сравнение «с tee»
   против «без tee»), и она же его и должна поставить.
2. **`consecutive_faults` — арифметика, не значение.** Равенство
   `rotations == faults / 3` держится только пока все фолты идут одной серией; это
   свойство ФИКСТУРЫ (единственный by-height источник — форжер, фидер строго вверх),
   и тест его проверяет отдельным ассертом `faults == |взятые подделки|`, но само
   поле по-прежнему невидимо.
3. **Ветка defer не исполнена.** См. §0 п. 9: доказано, почему недостижима, а не
   что делает.
4. **Один seed.** Все три теста на `seed = 1`. Детерминизм проверен ×3 на одном
   seed-е, устойчивость по seed-ам — нет.
5. **`payload != digest` и `epoch_bind`** — фолт-ветки, которые стендовый inlet не
   берёт ни разу.

## §5. Что осталось строкам 5.1 / 5.2 / 5.4

* **5.1** — `StandConfig` в 5.0а расширен ровно одним полем и ни одно существующее
  не переименовано, так что конфликта по типу нет; новый файл `cert_inlet_tests.rs`
  не пересекается с `tests.rs`.
* **5.2** — (а) читатель вердикта `observe_certificate` (`cert_inlet.rs:690`,
  `let _observed`) и потребитель `faults()`: 5.0а даёт им стенд, в котором
  keyless-допуск уже виден счётчиком, а ротация — наблюдаема; после 5.2 второе плечо
  `a_forged_seed_slot_…` меняет исход на `DataFault` + ротацию, и наблюдаемая для
  этого уже есть (`Outcome::cert_inlet[i].rotations`). (б) **Сам тест R-129 и его
  фикстура.** 5.0а её НЕ закрыла и объяснила, почему (§0 п. 9): чтобы inlet увидел
  сертификат эпохи, которую он не может прочитать, нужен источник БЕЗ гейта
  `deliver` — то есть шов, эквивалентный WS-потоку, либо per-node отказ комитета
  (`reverts_for`/`weights_none_for` в стенде глобальны, а глобальный отказ означает,
  что эпоха вообще не наступает: `committee_tests.rs:919-924` гоняет до
  `last(epoch−1)`). Плюс сама R-129 — про MARSHAL-резолвер (`deliver == false` на
  `scheme(E) == None` и вечное исключение честного пира), и её наблюдаемые
  (`requests_created`/`excluded`) 5.0а не добавляла.
* **5.4** — (а) тест «догоняющий валидатор дилит на живом фронтире без tee»: роль
  для него готова (§0 п. 6 — комбинация конфигурации, без нового `Role`), сравнение
  «с tee / без tee» остаётся; (б) эксклюзивный пин tee (§4 п. 1); (в) снятие tee и
  переход на `ordering_tip` watch.
* **Не сделано сознательно** (постановка §6): чтение вердикта `observe_certificate`,
  потребитель `faults()`, любые правки `cert_inlet.rs`/`beacon/**`, тест R-129.

---

# Часть Б — возврат Ф3→Ф2 №1 (clippy без фичи)

## Б.1 Что было

Ворота оркестратора (`cargo clippy -p fluentbase-consensus -p fluentbase-node
-p fluentbase-staking-reader --all-targets`, **БЕЗ фичи**) дали ТРИ предупреждения
вместо базовых двух:

~~~
warning: unused import: `Role`
warning: `fluentbase-consensus` (lib test) generated 1 warning (run `cargo clippy --fix --lib -p fluentbase-consensus --tests` to apply 1 suggestion)
warning: large size difference between variants
warning: `fluentbase-node` (lib) generated 1 warning
warning: `fluentbase-node` (lib test) generated 1 warning (1 duplicate)
warning: this `MutexGuard` is held across an await point
warning: `fluentbase-staking-reader` (lib test) generated 1 warning
~~~

Третье — моё: `testbed/cert_inlet_tests.rs:19` импортировал `Role` в безусловном
`use super::stand::{…}`, а единственный его потребитель —
`a_forged_seed_slot_costs_the_upstream_a_rotation_once_the_epoch_key_is_held` под
`#[cfg(feature = "dpos-devnet-byzantine")]`. По правилу ворот этого этапа рост
clippy — дефект, не нит. [KNOWN: прогон оркестратора, реляция; своим прогоном ниже
подтвердил обратное направление — после правки без фичи ноль].

## Б.2 Почему моя команда этого не поймала

Я гонял **одну** конфигурацию:

* моя (часть А §2): `cargo clippy -p fluentbase-consensus --lib --tests --features
  dpos-devnet-byzantine` — С ФИЧЕЙ, и под фичей `Role` используется, поэтому
  `unused_imports` не срабатывает;
* его: `cargo clippy … --all-targets` **БЕЗ фичи** — конфигурация, в которой тела
  feature-gated тестов не компилируются вовсе, а безусловный `use` остаётся.

То есть дефект лежал ровно в разнице «с фичей / без фичи», а я проверил только одну
сторону. Ошибка метода, не кода: «ноль предупреждений» в части А §2 было утверждением
про одну конфигурацию, а прочиталось как про обе — в §2 это теперь исправлено
явной пометкой.

## Б.3 Что исправлено

Ровно одна правка, `testbed/cert_inlet_tests.rs`:

* `Role` убран из безусловного `use super::stand::{…}` (`:17-20`);
* внесён в тело того теста, который его использует, рядом с уже стоявшим там
  `FORGE_WINDOW`: `use super::{byzantine_roles::FORGE_WINDOW, stand::Role};`
  (`:218`). Идиома — соседних feature-gated тестов стенда: `testbed/tests.rs:3561`
  (`use super::byzantine_roles::LATEST_INFLATION;` внутри тела теста под
  `#[cfg(feature = "dpos-devnet-byzantine")]`), там же `:3758`, `:4102`. Вариант «второй
  `#[cfg(feature)] use` на уровне файла» не взят: в `testbed/` его нет ни разу, а
  in-body `use` есть трижды.

Остальные имена того же `use` проверены — все нужны БЕЗ фичи, и это доказано не
глазами, а прогоном Б.4(1) (без фичи ноль `unused_imports`): `CertInletCfg`,
`CertInletSource` — во всех трёх тестах; `CertInletFacts`, `Outcome` — в хелпере
`inlet()`; `Progress` — в `reached()`; `Committees` — в
`drop_the_last_two_from_epoch_two()`, которую зовёт НЕ-gated тест (3); `PeerSet` — в
тесте (3); `Stand`, `StandConfig` — всюду; `std::sync::Arc` — в
`drop_the_last_two_from_epoch_two()`.

Больше ничего не менялось: ни тестов, ни полей, ни имён, ни `stand.rs`, ни `mod.rs`.

## Б.4 Прогоны (verbatim, мои, в заданном порядке)

1. `cargo clippy -p fluentbase-consensus --all-targets` (БЕЗ фичи), фильтр
   `grep -E "^(warning|error)"`:

~~~
=== CLIPPY no-feature ===
[no-feature done, exit=1]
~~~

   `exit=1` — это код `grep`, «ни одного совпадения», то есть от крейта `consensus`
   ноль строк `warning:`/`error:`. Ровно это и требовалось.

2. `cargo clippy -p fluentbase-consensus --all-targets --features dpos-devnet-byzantine`,
   тот же фильтр:

~~~
=== CLIPPY with feature ===
[clippy-feature done]
~~~

   Тоже ни одной строки.

3. `cargo fmt --check` (фильтр `grep -v "^Warning"` убирает только шум о
   nightly-опциях `rustfmt.toml`):

~~~
=== FMT ===
[fmt done]
~~~

   Ни одного `Diff in …` — чисто, `cargo fmt` повторно НЕ запускался (правка легла
   уже отформатированной).

4. `cargo test -p fluentbase-consensus --lib`:

~~~
test result: ok. 688 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 172.74s
~~~

   Те же 688/0, что и в части А (172.74 s против 171.96 s — только время).

Полный набор ворот не гонял — он у оркестратора. `git` только на чтение, не коммитил;
дерево по-прежнему: `M testbed/mod.rs`, `M testbed/stand.rs`, `?? testbed/cert_inlet_tests.rs`.

## Б.5 Урок

Для теста, часть которого под `cfg(feature)`, «clippy зелёный» — утверждение о ПАРЕ
конфигураций, и проверять надо обе. В части А §2 я записал результат одной команды
как свойство кода; это тот же класс, что «empty output is not evidence»: негативный
результат не факт, пока не знаешь, что его ограничивало. Для строк 5.1/5.2/5.4:
любой новый feature-gated тест стенда требует clippy И без фичи, И с ней.

---

# Часть В — проход 3 по ревью `E5-0a-A-REVIEW.md`

Свежий контекст, 2026-09-12. База HEAD `bec013a7` (не менялась). Писал только в
`testbed/stand.rs`, `testbed/cert_inlet_tests.rs` и в этот журнал; `testbed/fakes.rs`
НЕ понадобился (см. В§0(3)/(4) — партиция и слоты marshal-а уже существуют).
`testbed/mod.rs` не менял (его `mod`-строка стоит с прохода 1). Не коммитил, в индекс
ничего не ставил, `git` — только на чтение.

## В§0. Прямые ответы

### (1) Итог по 19 находкам

**FIXED — 14:** A-01 (комментарий кода + §0(3) переписаны по факту),
A-03 (две разводки tee, `TeeWiring`), A-04 (новый тест «догоняющий ЧЛЕН» на
`Stand::partition`), A-05 (третий источник `CertInletSource::PeerArchive` + новый
тест; «красный до правки» — ниже), A-06 (`with_carry_forward_fail_metric` подключён,
наблюдаемая `carry_forward_fails`, ассерт в keyed-тесте), A-08 (громкий отказ
конфигурации на сборке + `#[should_panic]`-тест), A-09 (процессность счётчика названа
в коде теста, вместе с тем, почему вывод не рушится), A-10 (все якоря §0(2),(3),(4)
перепинены на финальное дерево), A-11 (`rotate` = `cancel(Latest)`, то есть no-op для
by-height фидера — назван в доккомменте keyed-теста), A-14 (`EPOCH_LEN` теперь
ПРИСВАИВАЕТСЯ в каждом тесте, активация — `fakes::DPOS_ACTIVATION_BLOCK`, эпоха —
`beacon::testing::DETERMINISTIC_BOOTSTRAP_EPOCH`), A-16 (положительный контроль
`defers = 32` даёт новый архивный вход), A-17 (`dpos_dkg_height_drops_total == 0` в
keyed-тесте, как премисса арифметики), A-18 (`errors()` + `assert_lockstep_except(&[])`
в keyed-тесте), A-19 (`only_these_ran_inlets` — ассерт в каждом из шести прогонных
тестов).
**RECORDED — 4:** A-02 (цена арма `Frontier` цифрами — §2 и В§4), A-07 (доккоммент
теста (1) теперь говорит, что точное равенство пинует СООТНОШЕНИЕ СКОРОСТЕЙ),
A-12 (`Outcome` — второй общий с 5.1 тип, В§5), A-13 (`UpstreamCounters` смешаны,
В§5).
**REJECTED — 1:** A-15.

### (2) A-03 — центральная правка: что выбрано и почему НЕ конкурентный дренаж

**Предписанная форма невозможна, и это свойство кода, а не формы дренажа.** Между
тиком tee (`cert_inlet.rs:712-717`) и первым обращением к marshal-у в `ingest` НЕТ НИ
ОДНОЙ точки `await`: за тиком идут только комментарии и `match self.window_tx.clone()`
(`:734`), а первый `await` — это `verify_block` (`:736` в арме с окном, `:744` без
него), за ним `report_finalization` (`:741`/`:745`). Значит никакая другая future в
той же задаче — арм `select!` над стендовым каналом в том числе — не может быть
опрошена между тиком и marshal-ом: конкурентный дренаж физически не получает
управления в этом промежутке. Единственный способ, которым он мог бы успеть, — если
`verify_block` вернёт `Pending`; но `MarshalSink for MarshalMailbox` — это
`marshal::core::Mailbox::verified` → `send_lossy`
(CW `utils/src/channel/fallible.rs:156-158`) → `tokio::sync::mpsc::Sender::send`
(CW `utils/src/channel/mod.rs:7` реэкспортирует именно tokio), который при наличии
места в мэйлбоксе завершается БЕЗ уступки. То есть конкурентный дренаж в лучшем случае
дал бы НЕДЕТЕРМИНИРОВАННЫЙ порядок (зависящий от заполненности мэйлбокса), а в обычном
случае — тот же самый «после», что и сейчас. Для эксперимента §0.7(а) недетерминированный
порядок хуже честного «этой разводкой нельзя».

**Что сделано вместо (5.0а-Д-5, не молча — это названный оркестратором альтернативный
вариант):** РАЗВОДКА стала конфигурируемой, `CertInletCfg.tee: TeeWiring`
(`stand.rs:238`, тип — `:266`), два значения, и каждый тест указывает своё:

* `Observed` (`stand.rs:2726-2732`, арм `Observed`) — прежний стендовый канал с
  дренажем: ЕСТЬ список высот, порядок «после marshal-а»;
* `Production` (там же, арм `Production`) — tee получает НАСТОЯЩИЙ `dkg_height_tx`,
  то есть `LiveFrontierTee` в точности продакшн-сборки: `try_send` выполняет сам
  inlet на своей строке (`cert_inlet.rs:713`), ДО `verify_block`/`report_finalization`.
  Порядок продакшн-ный ПО ПОСТРОЕНИЮ, а не по наблюдению; список высот при этой
  разводке пуст (у `tokio::sync::mpsc` один приёмник, и он у актора beacon-а), и тик
  наблюдается только гейджем `dpos_dkg_clock_height` и `dpos_dkg_height_drops_total`.

Обе сразу получить нельзя именно поэтому: второго приёмника у канала не существует, а
любой промежуточный релей — это ещё один такт планировщика и снова «после».

**Что теперь МОЖЕТ измерить 5.4, чего не могла.** До правки стенд ставил tee и
marshal в ОДИН порядок (оба «после `ingest`»), поэтому «часы двинул tee» и «часы
двинул marshal-Tip» были в нём неразличимы — а §0.7(а) состоит ровно в разнице этих
двух моментов. С `TeeWiring::Production` высота попадает в очередь `DkgActor`-а до
того, как marshal вообще узнал о сертификате, так что 5.4 может (а) сравнить «с tee /
без tee» на одном фидере, (б) поставить догоняющего члена (В§0(3)) и посмотреть, успел
ли он напечатать share к дедлайну. Работающая фикстура этой разводки пинуется тестом
`the_production_tee_wiring_feeds_the_dkg_clock_with_no_drain_of_ours`: `ingests = 23`,
`tee_heights` пуст, дропов 0, `dkg_clock = 24 >= 23`. Чего он НЕ утверждает —
эксклюзивности (на здоровом узле тот же канал кормит `application.rs`), это по-прежнему
работа 5.4.

### (3) A-04 — догоняющий ЧЛЕН: форма и что наблюдается

Форма — `Stand::partition(&[0,1,2], &[3]).after_height(8).for_views(8)` на
`StandConfig::live(4, 1)` при дефолтном `PeerSet::AllNodes` (с `PeerSet::Committee`
партиция несовместима — оба владеют линками, `stand.rs:461-463`), inlet на узле 3,
источник `NextAboveTier`, разводка `Observed`. Нового `Role` и правок `fakes.rs` не
потребовалось: разрез физический (драйвер снимает линки ОБЕИХ плоскостей,
`stand.rs:1575-1590`), трое из четырёх — кворум, они продолжают финализировать, а
четвёртый встаёт. Тест —
`a_catching_up_committee_member_is_fed_by_its_inlet_while_its_el_is_behind`.

Наблюдается (мой прогон): `heal = [lag 8 vs majority 22]` — на заживлении узел 3 был
на 14 блоков ниже; `artifacts[3]` содержит эпоху 2, то есть он ОСТАЛСЯ членом (дилил
её DKG); `in_window = [9, 9, 10, 10, 11]` — высоты, teed-ые его inlet-ом СТРОГО между
его тиром-F на заживлении и тиром-F большинства, то есть сертификаты, которых ему в тот
момент не хватало; `ingests = 85`, `rotations = 0`, `defers = 0`,
`carry_forward_fails = 0`, дропов пересылки 0; финиш — `min_height >= 96` по ВСЕМ узлам
плюс `assert_lockstep_except(&[])`, то есть он догнал.

Чем отличается от keyless-теста: там узел НАВСЕГДА вне `committee[2]` — не дилит, ключа
не держит, консенсус-плоскости у него нет; это ушедший, а не догоняющий. Его
доккоммент теперь это и говорит первым абзацем («It is not the plan row's validator
with a lagging EL»), сам тест не переписан. Чего новый тест НЕ утверждает: что догнал
его именно inlet — его собственный marshal-ремонт идёт по тому же диапазону, и
разделение этих двух — «с tee / без tee» 5.4; названо в доккомменте.

Отмечу для 5.4: во время разреза inlet не кормит НИЧЕГО (разрез физический), окно этого
теста — после заживления, пока узел ещё позади. Если 5.4 нужен «живой inlet при мёртвой
консенсус-плоскости», это уже другая фикстура (`PeerSet::Committee{upstream_link:true}`
или `upstream_source_only_for`), и она с партицией не сочетается.

### (4) A-05 — архивный вход: форма, «красный до правки», цена, что осталось R-129

Форма: третий вариант `CertInletSource::PeerArchive { from }` (`stand.rs:332-337`).
Фидер держит СВОЙ курсор `walk` (`stand.rs:2774`) и читает пару у marshal-а донора
через продакшн-трейт `FrontierMarshal::pair_at` (`plane_upstream.rs:256`, impl для
`MarshalMailbox` `:278-283`), собирая `UpstreamFinalized` сам
(`stand.rs:2819-2843`) — без плоскости, без резолвера, без гейта `deliver`. Хэндл
донора — поздней привязкой, тем же приёмом, что `marshal_slot`: вектор слотов на все
узлы создаётся в `drive` (`stand.rs:1504-1505`), `build_node` берёт свой как
`marshal_slots[i]` (`stand.rs:2243`), а задача ждёт слот донора паузой `POLL`
(`stand.rs:2820-2829`). Курсор двигается ТОЛЬКО на успешной паре, поэтому вход идёт
вверх без повторов.

**«Красный до правки» verbatim** (мой прогон; тест уже написан, источник ещё
плоскостной `Frontier`, всё остальное — то же):

~~~
running 1 test
test testbed::cert_inlet_tests::a_donors_archive_hands_the_inlet_an_epoch_it_cannot_read_and_it_defers ...
thread '...' panicked at crates/dpos/consensus/src/testbed/cert_inlet_tests.rs:881:5:
the inlet never deferred: every certificate the donor's archive handed it was inside its own read window, so this source is gated after all: CertInletFacts { ingests: 3333, rotations: 0, tee_heights: [1, 1, 1, … (1..127, каждая ~26 раз) …, 127], tee: Observed, defers: 0, carry_forward_fails: 0 }
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 690 filtered out; finished in 20.58s
~~~

**Зелёный после правки** (тот же тест, источник `PeerArchive { from: 0 }`):

~~~
test testbed::cert_inlet_tests::a_donors_archive_hands_the_inlet_an_epoch_it_cannot_read_and_it_defers ... (5.0а/peer-archive defer) heights=[160, 160, 160, 63, 63] ingests=159 teed=127 teed_top=127 window_top=127 defers=32 rotations=0 virtual=159.828s
ok
~~~

Цена: **159 ingest-ов против 3332/3333** плоскостного на тех же 127 различных высотах
(127 чистых + 32 defer-а, ни одного повтора) — 21×. Плюс новый вход даёт то, чего у
плоскостного нет ни при какой цене: `defers = 32` при `rotations = 0`, то есть ветка
`cert_inlet.rs:618-640` исполнена, и исполнена как НЕ-фолт.

**Чего не хватает тесту R-129 (его пишет 5.2, здесь только фикстура).** R-129 —
про MARSHAL-резолвер, а не про inlet (`REGISTER.md:997-1006`): «`deliver == false` при
`scheme(E) == None` и вечное исключение честного пира». Эта фикстура даёт ему (а) узел,
чьё окно чтения ниже эпохи сертификата, и (б) способ подать такой сертификат в обход
плоскости. Не хватает трёх вещей: (1) наблюдаемых резолвера — `requests_created` /
`excluded` по пиру (стенд их не собирает ни для marshal-резолвера, ни для
фронтир-резолвера); (2) пути через `FrontierHandler::deliver` с ПРОВЕРКОЙ, что он
ответил `false` именно по причине `NotReadable`, — сегодня наблюдаем только суммарный
`dpos_frontier_dropped_total{reason}`, процессный (A-09); (3) второго честного пира,
чтобы «исключён навсегда» отличалось от «пиров больше нет». Ни одно из трёх в 5.0а не
входило.

### (5) A-06 — что теперь свидетельствует «ключ был»

`with_carry_forward_fail_metric` подключён (`stand.rs:2767`), счётчик — поле
`CertInletObs::carry_forward_fails` / `CertInletFacts::carry_forward_fails`
(`stand.rs:381`, `:411`), Arc-backed `Counter`, как и `defers`. Инкремент — ровно одна
строка: `cert_inlet.rs:666-668`, «BLS verify провалился И `key_known`». Прогон:
`carry_forward_fails = 6` при `ingested_forged = [65..70]` — равенство стоит ассертом.
Это прямое свидетельство того, что жертва судила КАЖДУЮ подделку, уже держа `PK_2`, в
момент суда; прежняя премисса (`artifacts[VICTIM]` на конец прогона) говорила только
«ключ у неё когда-то появился». В трёх тестах, где ключа не должно быть или подделок
нет, счётчик пинуется нулём.

### (6) Прогоны verbatim (мои, финальное дерево)

`cargo test -p fluentbase-consensus --lib`:

~~~
test result: ok. 692 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 174.50s
~~~

692 = 688 (дерево до этого прохода) + 4 новых теста без фичи: догоняющий член,
архивный defer, продакшн-разводка tee, отказ конфигурации. Наборы стенда я не гонял
(они у оркестратора): ожидать +4 и там — 55 с фичей, 46 без, из 51/42.

`cargo clippy -p fluentbase-consensus -p fluentbase-node -p fluentbase-staking-reader
--all-targets` **БЕЗ фичи**, `grep -E "^(warning|error)"`, `exit=0`:

~~~
warning: large size difference between variants           (fluentbase-node, lib + lib test)
warning: this `MutexGuard` is held across an await point  (fluentbase-staking-reader, lib test)
~~~

Те же три крейта **С фичей** (`--features dpos-devnet-byzantine`, то есть у каждого
пакета своя одноимённая фича), `exit=0` — те же ДВА чужих предупреждения и ни строки
от consensus. Плюс `cargo clippy -p fluentbase-consensus --all-targets --features
dpos-devnet-byzantine` — ноль строк вообще.

**Ловушка, на которую я наступил и которую стоит знать 5.1/5.2/5.4:**
`--features fluentbase-consensus/dpos-devnet-byzantine` (с квалификацией пакета) на
трёх крейтах НЕ СОБИРАЕТСЯ и это НЕ мой дефект: включается фича consensus-а, но не
одноимённая фича `fluentbase-node`, и `node/src/dpos.rs:2279-2280` перестаёт
инициализировать поле `byzantine` — `error[E0063]: missing field byzantine in
initializer of DposLayerConfig`. Правильная форма — неквалифицированная
`--features dpos-devnet-byzantine`.

`cargo fmt --check` (после `cargo fmt -p fluentbase-consensus`; переформатированы
только мои файлы, `git status` не изменился): `grep -c "Diff in"` = **0**, `exit=0`.

**Детерминизм ×3** — `cargo test -p fluentbase-consensus --lib --features
dpos-devnet-byzantine testbed::cert_inlet_tests -- --nocapture --test-threads=1`, три
прогона подряд, вывод БАЙТ-В-БАЙТ одинаков (сравнил `diff`-ом, отбросив только строки
`finished in` / `Finished` / `Compiling` / `Running unittests`):

~~~
running 7 tests
test …::a_catching_up_committee_member_is_fed_by_its_inlet_while_its_el_is_behind ... (5.0а/catch-up member) heights=[96, 96, 96, 96] heal=[lag 8 vs majority 22] in_window=[9, 9, 10, 10, 11] ingests=85 rotations=0 defers=0 cff=0 virtual=95.948s
test …::a_cert_inlet_is_refused_on_a_node_whose_beacon_drops_the_height_channel - should panic ...
test …::a_donors_archive_hands_the_inlet_an_epoch_it_cannot_read_and_it_defers ... (5.0а/peer-archive defer) heights=[160, 160, 160, 63, 63] ingests=159 teed=127 teed_top=127 window_top=127 defers=32 rotations=0 virtual=159.828s
test …::a_forged_seed_slot_costs_the_upstream_a_rotation_once_the_epoch_key_is_held ... (5.0а/Ex-21 keyed) heights=[102, 102, 102, 102, 102] forged=[65, 66, 67, 68, 69, 70] ingested_forged=[65, 66, 67, 68, 69, 70] teed_in_window=[64] ingests=102 rotations=2 defers=0 carry_forward_fails=6 virtual=101.9s
test …::a_keyless_admission_is_all_the_plane_lets_an_outrun_inlet_see ... (5.0а/Ex-21 keyless) heights=[160, 160, 160, 63, 63] ingests=3332 keyless=1683 rotations=0 defers=0 teed_top=127 window_top=127 plane_dropped=840 virtual=159.828s
test …::an_inlet_on_a_healthy_member_verifies_every_height_it_is_fed_and_tees_it ... (5.0а/clean) heights=[72, 72, 72, 72] ingests=71 tee=[1..71] rotations=0 defers=0 dkg_clock=72 virtual=71.9s
test …::the_production_tee_wiring_feeds_the_dkg_clock_with_no_drain_of_ours ... (5.0а/production tee) heights=[24, 24, 24, 24] ingests=23 tee_heights=0 dkg_clock=24 virtual=23.9s
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 694 filtered out; finished in 53.87s / 53.92s / 53.86s
~~~

Ни одно существующее число не поменялось: clean 71/tee 1..71, keyed 102/2/6 подделок,
keyless 3332/1683/127/840 — те же, что в части А.

### (7) Что оказалось неверным в решениях оркестратора

1. **A-03, предписанная форма** — конкурентный дренаж `ingest`-future и `tee_rx`
   продакшн-порядка НЕ даёт: между тиком tee и первым marshal-вызовом в `ingest` нет
   точки `await` (`cert_inlet.rs:712-717` → `:734` → `:736`/`:744`), а
   `verify_block` на реальном мэйлбоксе не уступает при свободном канале
   (`send_lossy` → tokio `mpsc::send`). Взят названный в той же строке альтернативный
   вариант, но не «вместо списка высот», а КАК ВТОРАЯ РАЗВОДКА — см. В§0(2),
   5.0а-Д-5.
2. **A-08, условие отказа шире, чем названо.** Приёмник `dkg_height_rx` дропается не
   только при `Beacon::Static`: арм `(Beacon::Live, Role::AbsentBeacon)`
   (`stand.rs:2473`) тоже не передаёт его никуда (`absent()` входов не берёт).
   Ассерт поэтому проверяет ОБА условия (`stand.rs:2600-2606`).
3. **A-14, константы `EPOCH_LEN` в `testbed` не существует.** Дефолт стенда — литерал
   `epoch_len: 32` внутри `StandConfig::honest` (`stand.rs:476`), не `const`. Вместо
   ввода константы в `stand.rs` (лишняя строка в теле `honest` = лишняя точка
   конфликта с параллельной 5.1) каждый тест теперь ПРИСВАИВАЕТ `cfg.epoch_len =
   EPOCH_LEN` — та же логика, по которой A-15 отклонён: явное присваивание делает
   предпосылку фикстуры независимой от дефолта. Две другие константы существуют и
   взяты (`fakes::DPOS_ACTIVATION_BLOCK`, `beacon::testing::DETERMINISTIC_BOOTSTRAP_EPOCH`).
4. **A-15 — REJECTED, с причиной оркестратора** (проверено им: `stand.rs:374-400`,
   `:402-414` на его дереве; на финальном — `:472-498`, `:500-512` — `Committees::All` действительно дефолт): строка
   `cfg.committees = Committees::All;` оставлена, её комментарий дополнен тем, почему
   явное присваивание лучше наследования дефолта.

Ничего из решений не отклонено кроме A-15 (что и было предписано) и формы A-03 (с
указанием file:line и без молчаливой подмены).

### (8) Запрещённого сделано — ноль

`git status --porcelain` на конец прохода — те же пять строк, что и на входе:

~~~
 M .dpos-study/PLAN.md                     ← чужое, не трогал
 M .dpos-study/history/E4-ORCHESTRATOR.md  ← чужое, не трогал
 M crates/dpos/consensus/src/testbed/mod.rs      ← прохода 1, в этом проходе не менял
 M crates/dpos/consensus/src/testbed/stand.rs
?? crates/dpos/consensus/src/testbed/cert_inlet_tests.rs
~~~

`testbed/tests.rs`, `byzantine_roles.rs`, `preconditions.rs`, `committee_tests.rs`,
`fakes.rs`, `cert_inlet.rs`, `plane_upstream.rs`, `beacon/**`, `committee/**`,
`epoch_manager.rs`, `executor.rs`, `node/**`, `devnet/**`, `contracts/**`,
`Cargo.toml`, `.claude/**` — не тронуты. Ни одного `#[allow]`, `todo!`; `unwrap` —
только `Mutex::lock().unwrap()` (`stand.rs:2863`, `:389`); таймер один и он назван
(5.0а-Д-6). Не коммитил, в индекс не ставил.

## В§1. Таблица по всем 19 находкам

| id | исход | что сделано |
|---|---|---|
| A-01 | FIXED | Комментарий фидера переписан по факту (`stand.rs:2777-2793`): цикл ограничен round-trip-ом сети (10 мс), а не `FRONTIER_FETCH_TIMEOUT`; путь мгновенного `None` назван (`plane_upstream.rs:485-489` против арма `:622`, комментарий `:641-647`); измеренная частота — 26 итераций/с (3332 + 840 за 159,828 с). Журнал §0(3) переписан там же |
| A-02 | RECORDED | Цена арма `Frontier` в §2 и В§4 цифрами: 3332 ingest-а на 127 различных высот (≈26× на высоту), каждая — BLS + два сообщения в marshal; архивный вход тех же 127 высот стоит 159 ingest-ов |
| A-03 | FIXED | `CertInletCfg.tee: TeeWiring{Observed, Production}` (`stand.rs:238`, `:266-286`, разводка `:2726-2732`). Конкурентный дренаж отклонён с доказательством по коду (В§0(2), 5.0а-Д-5); продакшн-порядок пинует новый тест `the_production_tee_wiring_…` |
| A-04 | FIXED | Новый тест `a_catching_up_committee_member_is_fed_by_its_inlet_while_its_el_is_behind`: член комитета, отрезанный `Stand::partition` (лаг 14 блоков на заживлении), догоняет, остаётся членом (артефакт эпохи 2), inlet кормит его в окне догона (`in_window = [9,9,10,10,11]`). Keyless-тест не переписан — только его доккоммент теперь говорит «НЕ-ЧЛЕН» |
| A-05 | FIXED | `CertInletSource::PeerArchive{from}` + `FrontierMarshal::pair_at` по marshal-у донора, вектор слотов в `drive`, новый тест с `defers = 32`. «Красный до правки» verbatim — В§0(4) |
| A-06 | FIXED | `with_carry_forward_fail_metric` подключён, `CertInletFacts::carry_forward_fails` пинуется `== 6` в keyed-тесте и `== 0` в трёх других |
| A-07 | RECORDED | Доккоммент теста (1) переписан: точное равенство `tee == (1..=ingests)` — пин СООТНОШЕНИЯ СКОРОСТЕЙ плюс отсутствие дыр, один сид, ×3 байт-в-байт; повтор высоты законен и наблюдался в других тестах (`in_window=[9,9,10,10,11]`, keyless 3332/127) |
| A-08 | FIXED | Громкий отказ на сборке узла (`stand.rs:2589-2606`), условие ШИРЕ названного: `Beacon::Live` И роль не `AbsentBeacon`. Пинуется `#[should_panic]`-тестом |
| A-09 | FIXED (текст) | В keyless-тесте названо, что `counter_of` процессный и в сумму входит узел 3; сказано, на чём держится вывод (свой `teed_top` жертвы у края своего окна), и что per-node аналог того же гейта — новый архивный тест (`defers` считает сам inlet жертвы) |
| A-10 | FIXED | Якоря §0(2),(3),(4) перепинены на финальное дерево ПОСЛЕ последней правки: `:2733` (marshal), `:2678-2684` (спавн после `set`), `:2800-2803`/`:2804`/`:2819-2843` (три фидера), `:2726-2732`/`:2760-2763`/`:2861-2868` (tee), `:2543` (`heights: dkg_height_rx`) |
| A-11 | FIXED (текст) | В доккомменте keyed-теста: `rotate` = `mailbox.cancel(FrontierKey::Latest)` (`plane_upstream.rs:684-692`), для by-height фидера полный no-op, и равенство `rotations == faults/3` держится ИМЕННО на её бездействии; сказано, что это пин порога, а не продакшн-failover-а |
| A-12 | RECORDED | В§5: второй общий с 5.1 тип — `Outcome` (`stand.rs:892`) |
| A-13 | RECORDED | В§5: `UpstreamCounters` смешивают трафик inlet-а и зонда; новый архивный вход их не трогает вовсе (читает marshal донора напрямую) |
| A-14 | FIXED | `cfg.epoch_len = EPOCH_LEN` в каждом тесте; активация — `fakes::DPOS_ACTIVATION_BLOCK`; `EPOCH_2_START = DETERMINISTIC_BOOTSTRAP_EPOCH * EPOCH_LEN`. Почему константы `EPOCH_LEN` нет — В§0(7)(3) |
| A-15 | REJECTED | Строка `cfg.committees = Committees::All;` оставлена; причина оркестратора (явное присваивание делает предпосылку независимой от дефолта `honest`) вписана в комментарий на месте |
| A-16 | FIXED через A-05 | Положительный контроль счётчика `defers`: 32 на архивном входе. Связь записана в доккомментах обоих тестов |
| A-17 | FIXED | `dpos_dkg_height_drops_total == 0` в keyed-тесте, с комментарием, что это премисса точности `faults = ingests − tee.len()` |
| A-18 | FIXED | `out.assert_lockstep_except(&[])` + `assert!(out.errors().is_empty())` в keyed-тесте — оба зелёные (жертва в лockstep: `heights=[102×5]`) |
| A-19 | FIXED | Хелпер `only_these_ran_inlets(&out, &[…])` — ассерт в каждом прогонном тесте: inlet поднялся ровно у названных узлов и ни у кого больше |

## В§2. Новые отклонения (после 5.0а-Д-4)

* **5.0а-Д-5 — вместо конкурентного дренажа две РАЗВОДКИ tee.** Решение прохода 3
  предписывало держать `ingest` как future и дренировать `tee_rx` конкурентно
  (biased `select!`). Сделано иначе: `TeeWiring::{Observed, Production}`. Причина по
  коду: между тиком tee (`cert_inlet.rs:712-717`) и первым marshal-вызовом (`:736` /
  `:744`) в `ingest` нет ни одной точки `await` (между ними только комментарии и
  `match self.window_tx.clone()` `:734`), поэтому конкурентная future не получает
  управления; а `verify_block` на реальном мэйлбоксе не уступает при свободном канале
  (`send_lossy` → tokio `mpsc::send`, CW `utils/src/channel/fallible.rs:156-158`,
  `mod.rs:7`). Конкурентный дренаж дал бы недетерминированный порядок — для
  эксперимента §0.7(а) это хуже, чем честная вторая разводка. Меняет ли П/Д: нет,
  это форма стендового шва; но 5.4 обязана выбирать разводку осознанно, и тип
  заставляет её это сделать (поля с дефолтом «как раньше» достаточно, чтобы старые
  тесты не менялись, но каждый тест указывает значение явно).
* **5.0а-Д-6 — у арма `PeerArchive` свой пацинг `c.sleep(POLL)` на промахе**
  (`stand.rs:2827`, `:2839`). Постановка прохода 1 разрешала «уступить такт событием
  или `ctx.sleep`, назвав выбор». Событию тут не на что подписаться: чтение
  `pair_at` — ЛОКАЛЬНОЕ для донора, сетевого round-trip-а, у которого плоскостные
  армы занимают пацинг, нет, а «донор финализировал h» стенд событием не публикует
  (у `FakeChain` нет ни `Notify`, ни watch). Без паузы это не безобидный спин:
  детерминированный рантайм двигает время на 1 мс за итерацию и перескакивает в
  idle ТОЛЬКО когда готовых задач нет (`COMMONWARE_INTERNALS.md:423`), то есть
  всегда-готовая задача и приколачивает часы к 1 мс/итерация, и заливает
  select-петлю marshal-а донора сообщениями (та же опасность, что Д-81, с другой
  стороны). `POLL` — существующая константа стенда (частота выборки драйвера,
  `stand.rs:87`), идиома `c.sleep(BACKOFF)` в задаче стенда уже есть
  (`stand.rs:3017`). Меняет ли П/Д: нет.
* **5.0а-Д-7 — четыре новых теста, а не два.** Таблица решений требовала новый тест
  под A-04 и новый под A-05. Добавлены ещё два, и оба по механической причине, а не
  для полноты: (а) `the_production_tee_wiring_…` — без него вариант
  `TeeWiring::Production` не конструирует НИ ОДИН тест, а это
  `warning: variant is never constructed`, то есть рост clippy, то есть возврат на
  воротах (я видел этот warning ровно в такой форме на `PeerArchive`, пока новый тест
  ещё ходил через плоскость); (б) `a_cert_inlet_is_refused_…` — отказ A-08 без теста
  остаётся утверждением о коде, а `#[should_panic]` стоит 0,30 с и пинует его.
  Меняет ли П/Д: нет.

## В§3. Раскладка коммитов (уточнение §3 с учётом прохода 3)

1. `test(testbed): stand up the production CertInlet as a second marshal producer`
   — `testbed/stand.rs`: поле `StandConfig::cert_inlet`, `CertInletCfg` (три поля),
   `CertInletSource` (три варианта, включая `PeerArchive`), `TeeWiring`,
   `CertInletObs`/`CertInletFacts` (с `carry_forward_fails` и `tee`), вектор
   marshal-слотов в `drive` + параметр `build_node`, задача inlet-а с тремя фидерами
   и двумя разводками tee, отказ конфигурации.
2. `test(testbed): cert-inlet tests — clean path, tee wirings, data-fault rotation, keyless admission, peer-archive deferral`
   — `testbed/cert_inlet_tests.rs` (семь тестов) + `mod` в `testbed/mod.rs`.
3. `docs(dpos): journal of Э5 5.0а` — `.dpos-study/history/E5-0a-A.md`.

Группы 1 и 2 по отдельности по-прежнему НЕ собираются, и теперь причин две: без тестов
поля `Outcome`/`StandConfig` дают `dead_code`, а вариант `TeeWiring::Production` и
вариант `CertInletSource::PeerArchive` дают `variant is never constructed` (видел
этот warning на `PeerArchive` в промежуточном состоянии прохода). Значит 1+2 — один
коммит, `#[allow]` не нужен.

## В§4. Где проверка слабее всего (после прохода 3)

1. **Порядок tee против marshal-а по-прежнему НЕ ИЗМЕРЕН, он только СТАЛ
   измеримым.** `TeeWiring::Production` даёт продакшн-порядок по построению, но ни
   один тест 5.0а не сравнивает момент «часы двинулись» с моментом
   «marshal-Tip пришёл»: гейдж `dpos_dkg_clock_height` читается на конец прогона, и
   на здоровом узле тот же канал кормит `application.rs`. Это работа 5.4, и теперь у
   неё есть обе разводки и догоняющий член.
2. **Цена плоскостного арма, цифрами (A-02).** `Frontier`: 3332 ingest-а на 127
   различных высот — каждая ≈26 раз, каждая подача — полная BLS-проверка плюс
   `verify_block` + `report_finalization` в marshal (`cert_inlet.rs:744-745`).
   Причина структурная: у плоскостного фидера нет своего курсора, `get_latest`
   отвечает тем, что есть. Новый архивный вход этой цены не платит вовсе (159
   ingest-ов, курсор свой, повторов нет) — но и гейт плоскости он, по построению, не
   пинует; тесты нужны оба.
3. **Один сид.** Все семь тестов на `seed = 1`; детерминизм ×3 проверен, устойчивость
   по сидам — нет. Точное равенство `tee == (1..=ingests)` (A-07) — самое чувствительное
   к этому место.
4. **Эксклюзивность тика tee** — как в части А: не доказана ни при одной разводке.
5. **`consecutive_faults`** — по-прежнему арифметика, не значение; но теперь
   премисса арифметики (нет потерь на пересылке) стоит ассертом (A-17), а премисса
   «ключ был» — счётчиком (A-06).
6. **Ветки `payload != digest` и `epoch_bind`** не исполнены ни разу (вторая на
   валидаторском inlet-е и не может быть).
7. **Отказ конфигурации проверен только на `Beacon::Static`**, не на
   `(Live, AbsentBeacon)`: второй арм того же ассерта тестом не покрыт (лишний
   прогон стенда ради второй половины одного `matches!`).
8. **Партиция и `PeerSet::Committee` несовместимы** (`stand.rs:461-463`), поэтому
   «догоняющий член» и «живой inlet при мёртвой консенсус-плоскости» — два разных
   теста, и второго в 5.0а нет.

## В§5. Что уходит в 5.1 / 5.2 / 5.4 (дополнение к §5)

* **5.1** — общих типов ДВА, не один: `StandConfig` (одно новое поле,
  `stand.rs:209`) и `Outcome` (`stand.rs:892`, поле `cert_inlet`) — A-12. Плановое
  условие параллельности названо только по первому; при слиянии смотреть оба. Новые
  типы (`CertInletCfg`, `CertInletSource`, `TeeWiring`, `CertInletFacts`) и файл
  `cert_inlet_tests.rs` с `tests.rs` не пересекаются.
* **5.2** — (а) читатель вердикта `observe_certificate` и потребитель `faults()`:
  наблюдаемые готовы, плюс теперь есть `carry_forward_fails`, который различает
  «форжер» и «ключ из чужого минта» (A-06), и `defers` с положительным контролем;
  (б) **фикстура R-129 построена** (`CertInletSource::PeerArchive`, `defers = 32`), но
  самого теста R-129 не хватает трёх вещей — наблюдаемых marshal-резолвера
  (`requests_created`/`excluded`), пути через `deliver` с причиной отказа и второго
  честного пира; список в В§0(4); (в) второе плечо keyed-теста после Д-3=(в) меняет
  формулировку с «BLS verify FAILED» на «синхронный `Refused` ⇒ `record_data_fault`»;
  (г) `UpstreamCounters` смешивают трафик inlet-а и зонда (A-13) — `served_heights` на
  арме `Frontier` растёт до тысяч; архивный вход их не трогает.
* **5.4** — (а) выбор разводки tee теперь ЕСТЬ и он в типе (`TeeWiring`); эксперимент
  §0.7(а) строится на `Production` + догоняющем члене, обе фикстуры зелёные;
  (б) эксклюзивный пин tee (В§4 п. 1, 4); (в) снятие tee и переход на `ordering_tip`
  watch; (г) если 5.4 нужен живой inlet при МЁРТВОЙ консенсус-плоскости — это не
  партиция (несовместима с `PeerSet::Committee`), а `upstream_source_only_for` или
  `PeerSet::Committee{upstream_link:true}`, и такой фикстуры в 5.0а нет.
* **Общее** — `PlaneUpstreamHandle::rotate` отменяет только `FrontierKey::Latest`
  (`plane_upstream.rs:684-692`): для валидатора, чей inlet кормится by-height,
  data-fault-ротация плоскости не делает НИЧЕГО (A-11, теперь сказано в
  доккомменте теста). Если 5.2 считает ротацию средством защиты от плохого
  upstream-а на плоскости — это отдельная проверка, в 5.0а её нет.
