# E5-ORCHESTRATOR — состояние оркестратора этапа Э5 (строки 5.0а, 5.1, 5.2, 5.3, 5.4)

Ветка `djadjka/dpos-reth-2.2-squashed`, стартовый HEAD `bec013a7` (2026-09-12). Оркестратор — Opus 5; исполнители/ревьюеры/третьи проходы/оценщик — Opus 5 через Agent (`model: opus`, `subagent_type: general-purpose`), один агент на (строка, фаза), постановка — файлом в `history/E5-prompts/`. Правила сессии (verbatim, переживают компакцию):

- **Hard-stop** (единственные причины закончить с открытым этапом; каждый — в раздел «Hard-stop» со свидетельством): (1) ворота красные после ДВУХ проходов правки одной находки; (2) реализация невозможна без изменения решения в `DECISIONS.md` (П-2, П-3, П-9, Д-1, Д-3, Д-6, Д-7, Д-9 — все «принято») — записать предложение и остановить строку; (3) BLOCKER, подтверждённый мной по коду, на который проект не отвечает; (4) стенд-тест, доказавший неверность проекта на центральном пути.
- **Коммиты**: разрешены в точках Ф8; Conventional Commits; БЕЗ трейлеров-атрибуций любого вида; `git add` по явным путям (никогда `-A`/`.`); `.dpos-study/` — `git add -f`; `.claude/` не трекается. Запрещены `git push/stash/checkout/restore/reset/clean/rebase`. `devnet/` не трогать. **Никогда не ставить в индекс `.dpos-study/history/E4-ORCHESTRATOR.md`** (файл другой сессии, грязный). Перед коммитом `git status --short` — в индексе только файлы захода. Каждый коммит обязан собираться и проходить `cargo test -p fluentbase-consensus --lib --no-run`.
- **Провенанс**: реляция агента — утверждение, не факт; «проверено» — только команда, которую я запустил, или файл, который я открыл в этой сессии; квитанция в «Квитанции» ДО действия по чужому утверждению. Каждая находка SERIOUS+ , каждый REJECTED-вердикт и каждый FIXED на SERIOUS+ — ханк открываю сам; не менее одного «красного до правки» на заход воспроизвожу сам.
- **Ворота Ф3** (сам, скрипт `scratchpad/gates/run.sh <tag>`): `cargo test -p fluentbase-consensus --lib`; `… --features dpos-devnet-byzantine testbed::`; `… --lib testbed::` без фичи; `cargo test -p fluentbase-node --lib`; `cargo test -p fluentbase-staking-reader`; `cargo test -p fluentbase-consensus --test slasher_integration`; `cargo clippy -p fluentbase-consensus -p fluentbase-node -p fluentbase-staking-reader --all-targets` и с фичей; `cargo fmt --check`; `cargo doc -p fluentbase-consensus --no-deps 2>&1 | grep -c "unresolved link"`. Стенд не гонять параллельно с другой нагрузкой.
- **Контур строки**: Ф1 карта (сам) → Ф2 исполнение (Opus 5, свежий) → Ф3 ворота (сам) → Ф4 ревью (Opus 5, СВЕЖИЙ, только путь к журналу) → Ф5 квитанции (сам) → Ф6 третий проход (Opus 5, свежий) + Ф3 → Ф7 доки → Ф8 коммиты → Ф9 приёмка.
- **Порядок**: 5.0а → 5.1 → 5.2 → 5.3 → 5.4. Параллельно только 5.0а с 5.1.
- **Вне объёма**: Э6; `executor.rs` сверх `for_seeds`-фикстуры; `committee/` сверх формы фасада; `contracts/`; переписывание проекта §5 (отклонения — только Д-nn); `SCHEME_RETENTION_EPOCHS`/`SEED_RETENTION`/`deque_size`; удаление тестов `preconditions.rs`/`committee_tests.rs`.

## Текущее состояние

- Строка: **5.0а**, фаза **Ф8 (коммиты)**. Код закоммичен — `7790d1bc` `test(testbed): run the production CertInlet as a second marshal producer`. Ф3 на финальном дереве (тег `a3`) зелёная, Ф7 доки сделаны. Осталось: `docs(dpos)`-коммит `.dpos-study/` и Ф9 приёмка.
- Следующий шаг: `docs(dpos)`-коммит, затем Ф9 (стендовая половина Ex-21 — зелёная, проверено моим прогоном), затем строка **5.1**: Ф1 карта.
- Строки 5.1/5.2/5.3/5.4 — не начаты.

## Правила проходов (скопировано из `E4-ORCHESTRATOR.md:27-35`, решения пользователя — применять как есть)

- **2026-09-12 (решение пользователя)**: исполнитель НЕ гоняет полный набор ворот — только `cargo test -p fluentbase-consensus --lib` один раз в конце и ×3 по своим стенд-тестам; полный набор (стенд с фичей/без, node, reader, slasher, clippy ×2, fmt, doc) гоняет только оркестратор на Ф3. Причина — дубль тех же команд стоил ~15 мин на проход.
- Правки прохода — батчем, ОДИН прогон затронутых тестов, ворота один раз в конце.
- «Красный до правки» — только для теста, пинующего найденный дефект поведения (1–2 на проход); не для правок ассертов и косметики.
- Мутация — только для несущих ассертов, ≤ 2 на проход, по одному тесту (`--exact`).
- Детерминизм ×3 — один раз в конце.
- **Решение пользователя 2026-09-11**: размер заходов и форма ревью ОСТАЮТСЯ прежними — заходы не дробить дальше, ревью по-прежнему «полнота, не фильтр» (все вопросы, все находки с серьёзностью и «чем пытался опровергнуть», ворота ревьюер прогоняет САМ). Журнал короткий — без «инвентаризации до» и без раздела «по файлам».

## Базовые ворота (мои прогоны на `bec013a7`)

| ворота | ожидание задания | мой результат на `bec013a7` (verbatim) |
|---|---|---|
| `cargo test -p fluentbase-consensus --lib` | 686/0 | `test result: ok. 686 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 169.46s` |
| `… --features dpos-devnet-byzantine testbed::` | 48/0 | `test result: ok. 48 passed; 0 failed; 0 ignored; 0 measured; 646 filtered out; finished in 214.85s` |
| `… --lib testbed::` без фичи | — | `test result: ok. 40 passed; 0 failed; 0 ignored; 0 measured; 646 filtered out; finished in 166.42s` |
| `cargo test -p fluentbase-node --lib` | 55/0 | `test result: ok. 55 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 56.40s` |
| `cargo test -p fluentbase-staking-reader` | 64/0 | `test result: ok. 64 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` + второй набор `0 passed; 1 ignored` |
| `--test slasher_integration` | 16/0 | `test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s` |
| clippy (consensus+node+reader, `--all-targets`) | 2 чужих | ровно два: `large size difference between variants` (node) и `this MutexGuard is held across an await point` (staking-reader) — оба с HEAD |
| clippy с фичей | 0 | 0 строк `warning`/`error` |
| `cargo fmt --check` | — | **чисто**, 0 `Diff in` (вывод — только семь nightly-предупреждений `.rustfmt.toml`, повторённые по крейтам). ВСПЛЫЛО: `PLAN.md` §1 всё ещё называет fmt красным на шести файлах (замер 09-09) — на `bec013a7` он зелёный |
| `cargo doc … grep -c "unresolved link"` | 6 | 6 |

Вывод — `scratchpad/gates/base-*.txt`, коды выхода — `base-status.txt` (все 10 `exit=0`).

Снимок объёма `beacon/` на `bec013a7` (моя команда, для закрывающего ответа 9): `ls crates/dpos/consensus/src/beacon | wc -l` = **28**; `cat crates/dpos/consensus/src/beacon/*.rs | wc -l` = **31313**.

## Карта 5.0а (Ф1, собрана мной по коду 2026-09-12 [KNOWN])

Один заход (решение пользователя 09-11 — не дробить). Цель строки плана: стендовый `CertInlet` как вторая продюсерская дверь в marshal узла стенда.

**Якоря, открытые мной в этой сессии:**
- `CertInlet` — `cert_inlet.rs:290` (поля), `new(marshal, committee, ctx)` `:404` (`M: MarshalSink`, `E: CryptoRngCore + Send`), билдеры `with_randomness` `:446`, `with_epoch_math` `:456`, `with_rotate` `:466`, `with_connection_token` `:479`, `with_window` `:488`, `with_tee` `:500`; `ingest` `:520` (инфаллибельна); `record_data_fault` `:758`; `MAX_UPSTREAM_FAULTS = 3` `:91`; `RotateUpstream = Arc<dyn Fn() -> BoxFuture<'static, ()>>` `:106`; `LiveFrontierTee{dkg_height_tx, plane_clock}` `:274`; `MarshalSink` `:213`, `impl MarshalSink for MarshalMailbox` `:231`.
- Тик tee — `cert_inlet.rs:713-717` (`tee.dkg_height_tx.try_send(uf.block.height)`, при ошибке `plane_clock.note_height_drop()`), стоит ПОСЛЕ `observe_certificate`/`observe_cert` и только на чистом ingest.
- Вердикт — `let _observed = self.randomness.observe_certificate(ObservedCertificate::Finalization(round, &uf.finalization))` `cert_inlet.rs:690`; комментарий рядом прямо говорит «PLAN row 5.2: … Until then nobody reads the verdict here» — значит потребителя вердикта 5.0а НЕ добавляет.
- Продакшн-сборка inlet-а — `crates/node/src/cert_inlet.rs:57-113`: `CertInlet::new(marshal, committee, c).with_tee(tee).with_rotate(rotate).with_randomness(beacon).with_connection_token(conn_gen)`, петля `finalized_rx.recv() ⇒ inlet.ingest(uf)`, `None ⇒ error! + break` (фатально).
- Вход: `UpstreamFinalized` — `cert_follow.rs:44`; трейт `CertUpstream` даёт `get_finalization(Height)`, `get_latest()`, `rotate()` (`fakes.rs:1625-1646` — реализация обёртки).
- Стенд: `frontier_plane(...) -> PlaneUpstreamHandle` `stand.rs:1751-1790` (продакшн `new_bridge` `stand.rs:1766`, `CountingHandler`, под фичей — `ForgedSeedProducer` `byzantine_roles.rs:574`); `CountingUpstream::new` `fakes.rs:1608`, обёртка над плоскостью — `stand.rs:2012-2041`; `marshal_slot: Arc<OnceLock<MarshalMailbox>>` `stand.rs:2011`, заполняется после `outer.build` (`stand.rs:2418` `marshal_slot.set(outer.marshal_mailbox())`); `upstream` **перемещается** в `outer.start(..., Some(upstream))` `stand.rs:2584` — inlet-у нужен КЛОН (`upstream.clone()` уже используется пробой `stand.rs:2078`); `(dkg_height_tx, dkg_height_rx)` `stand.rs:2233` (`mpsc::channel::<u64>(256)`), `dkg_height_tx` уходит в `OuterBuilder.dkg_height_tx` `stand.rs:2402` — inlet-у нужен КЛОН; `plane_clock` — там же; `Arc<dyn Beacon>` — `randomness` `stand.rs:2333-2345`.
- `StandConfig` — `stand.rs:90-205` (общая точка с 5.1; см. «Разделение»); `Beacon{Static, Live}` `stand.rs:209-222`; `Role` `stand.rs:318-380`; `Outcome` `stand.rs:552-660`; `NodeHandles` `stand.rs:1121-1144`; `FakeChain` `fakes.rs:326-359`.
- `git grep CertInlet -- crates/dpos/consensus/src/testbed` на `bec013a7` — пусто (моя команда).

**Файлы на запись (только):** `crates/dpos/consensus/src/testbed/{stand,fakes}.rs`, `crates/dpos/consensus/src/testbed/byzantine_roles.rs` (только если роль требует нового кадра — иначе не трогать), `.dpos-study/history/E5-0a-A.md`. Запрещено: `beacon/**`, `testbed/tests.rs` (держит 5.1), `cert_inlet.rs`, `node/**`, `committee/**`, `epoch_manager.rs`, `executor.rs`, `devnet/**`, `contracts/**`, `.claude/**`.

**Разделение с 5.1 (общая точка `StandConfig`):** 5.0а добавляет в `StandConfig` РОВНО ОДНО новое поле (`cert_inlet: Option<…>`) и НИ ОДНОГО изменения существующих полей; 5.1 `StandConfig` не трогает (её стенд-тесты живут в `testbed/tests.rs` и строятся на существующих полях). Это условие записано в постановки обеих строк.

**Инварианты §3 проекта, которые заход обязан не сломать:** I4 (любой узел, верифицирующий сертификаты эпохи E, получает `PK_E` в ограниченное время) — inlet зовёт `ensure_key(epoch, PinEffort::Local)` `cert_inlet.rs:596`; I7 (в `seed_for` только σ, проверенная под `PK_E`) — `observe_certificate` остаётся единственной дверью и вердикт по-прежнему не читается; I1/I2 заход не касается.

**Тест-потребители по строке плана:**
1. Стендовая половина Ex-21 — подменённый σ до `PK_E` ⇒ `DataFault`-ротация: сегодня плечо `CertInlet::ingest`/`ensure_key(Local)` в стенде отсутствует (R-008, `EXPERIMENTS.md` §3 п.1). 5.0а обязана дать НАБЛЮДАЕМЫЕ `record_data_fault`/`consecutive_faults`/счётчик `rotate()`; «до» для 5.2 — то есть на 5.0а ротация по данным должна быть видна, а чтение вердикта — нет.
2. Фикстура R-129 — узел получает финализацию эпохи, схем которой у него нет (ассерт на `requests_created`/`excluded`); строка 5.2 ставит сам тест, 5.0а даёт фикстуру.
3. Роль «валидатор с отстающим EL и живым inlet-ом» — покрытие догоняющего валидатора одним `clock` (нужно 5.4).

## Ворота Ф3 захода 5.0а-А (мои прогоны, дерево `gates/a1-tree.md5`)

| ворота | база `bec013a7` | дерево A1 (verbatim) | вердикт |
|---|---|---|---|
| `--lib` | 686/0 | `ok. 688 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 172.42s` | зелёные (+2) |
| стенд с фичей | 48/0 | `ok. 51 passed; 0 failed; 0 ignored; 0 measured; 646 filtered out; finished in 214.73s` | зелёные (+3) |
| стенд без фичи | 40/0 | `ok. 42 passed; 0 failed; 0 ignored; 0 measured; 646 filtered out; finished in 170.39s` | зелёные (+2) |
| node | 55/0 | `ok. 55 passed; 0 failed; …; finished in 58.94s` | без изменений |
| reader | 64/0 | `ok. 64 passed; 0 failed; …` + `0 passed; 1 ignored` | без изменений |
| slasher | 16/0 | `ok. 16 passed; 0 failed; …; finished in 0.02s` | без изменений |
| clippy ×3 крейта | 2 чужих | A1: **3** (`unused import: Role` сверх двух чужих) — **КРАСНЫЕ, рост**; A2 после правки: ровно два чужих (`large size difference between variants` в node, `this MutexGuard is held across an await point` в reader) | A1 красные, A2 зелёные |
| clippy с фичей | 0 | 0 строк `warning`/`error` (A1 и A2) | зелёные |
| `fmt --check` | чисто | 0 `Diff in` (A1 и A2) | зелёные |
| doc unresolved | 6 | 6 (A1; в A2 не гонялся — правка под `cfg(test)`) | зелёные |

Прогон **A2** (текущее дерево, `gates/a2-tree.md5`), после правки возврата №1: `--lib` `ok. 688 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 171.70s`; стенд с фичей `ok. 51 passed; 0 failed; 0 ignored; 0 measured; 646 filtered out; finished in 214.61s`; стенд без фичи `ok. 42 passed; 0 failed; 0 ignored; 0 measured; 646 filtered out; finished in 168.66s`. Ф3 **ЗЕЛЁНАЯ**.

## Агенты

| id | роль | строка | промпт-файл | файл результата | статус |
|---|---|---|---|---|---|
| I1 | исполнитель | 5.0а | `E5-prompts/5.0a-A-impl-1.md` | `history/E5-0a-A.md` | вернулся 2026-09-12; 4 отклонения (Д-1…Д-4), находка §0(9). **Возврат №1 на красных воротах** (clippy `unused import: Role`) — SendMessage, часть Б журнала |
| F1 | третий проход | 5.0а | `E5-prompts/5.0a-A-fix-1.md` | `history/E5-0a-A.md` часть В | запущен |
| R1 | ревьюер | 5.0а | `E5-prompts/5.0a-A-review-1.md` | `history/E5-0a-A-REVIEW.md` | вернулся 2026-09-12; 19 находок (1 SERIOUS, 7 MODERATE, 7 MINOR, 4 NIT); **прогонов ноль** — классификатор auto-mode запретил ему `cargo test` |

## Квитанции

Каждая строка — действие по чужому утверждению и то, что я открыл/запустил САМ до него.

| # | утверждение (чьё) | что я проверил сам | вывод |
|---|---|---|---|
| К-1 | Задание: Д-3/Д-6/Д-7/Д-9/Д-1 ратифицированы, hard-stop (2) не наступает | прочитал `.dpos-study/DECISIONS.md` целиком: Д-1 `:15` «принято 2026-09-12 … (а)», Д-3 `:25` «принято 2026-09-12 … (в)», Д-6 `:41` «принято 2026-09-12 — defer», Д-7 `:46` «принято 2026-09-12 — читать бит `changed` с якоря модуля», Д-9 `:56` «принято 2026-09-12 — beacon-owned `SeedIndex` + intake» | подтверждено; hard-stop (2) по этим пяти не наступает |
| К-2 | `E5-PRECONDITIONS.md` §0 п.5: `git grep CertInlet testbed/` пуст | запустил `git grep -n CertInlet -- crates/dpos/consensus/src/testbed` | пусто — подтверждено |
| К-3 | Проект §5.1/Д-3 (в): подменённый σ доходит до `ingest`, а не рубится раньше | открыл `plane_upstream.rs:405-490` (`deliver`) и `:371-390` (`verifier_for`): схема для `deliver` строится `build_verifier(&namespace, record.bls.bimap, epoch, None)` `:382-387` — **оракула эпохи нет**, значит σ-слот на `deliver` не проверяется; в `ingest` схема берётся `self.committee.scheme(epoch)` (`cert_inlet.rs:620`), а она с оракулом (`committee/mod.rs:326`, стенд строит через `committee::epoch_verifier(CHAIN_ID, beacon_slot)` `stand.rs:2002`) | механизм стендовой половины Ex-21 держится по коду; записан в постановку I1 |
| К-4 | Проект §7 5.0а: `upstream` стенда можно отдать inlet-у | открыл `stand.rs:2012-2041` (сборка `CountingUpstream`), `:2078` (`upstream.clone()` в пробе), `:2584` (`Some(upstream)` в `outer.start` — перемещение) | inlet-у нужен КЛОН; записано в постановку |
| К-5 | Строка 5.1: `for_keys` «только тесты» | запустил `git grep -n for_keys -- crates`: три вхождения, все в `beacon/surface.rs` (`:1009` определение, `:1864`, `:1881` — его собственный тестовый модуль) | подтверждено |
| К-6 | Строка 5.2: `for_seeds` уходит вместе с фикстурой executor-а | `git grep -n for_seeds -- crates`: `surface.rs:984`, дверь `mod.rs:157`, потребители `executor.rs:5012`, `:5051` (тесты), упоминание в доке `certify.rs:424` | подтверждено |
| К-7 | Строка 5.1 / предусловия Д3(а): решение «дилю ли я» бит `changed` не читает | открыл `beacon/actor.rs:1649-1690` — `maybe_start` берёт пару ростеров через `self.committee_pair_for` `:1680-1690` и решает по `next == cur`; `beacon/carry.rs:227-245` — `frozen_dkg_qual` с write-once мемо и стражем `!(bit || committed)` `:238-240` | подтверждено, якоря на месте |

## Карта 5.1 (Ф1 — черновик, якоря открыты мной 2026-09-12 [KNOWN]; дособрать перед запуском)

Два захода по строке плана: **(А)** артефакт — единственный владелец `PK_E`; **(Б)** Д-7 и `carry.rs`.

Якоря, которые я уже открыл сам:
- `beacon/mod.rs:56-86` — подмодули приватны (`mod x;`), `:118-124` — `pub use`, `:127-129` — `pub(crate) use dkg_engine::agreement_partition`, `surface::absent_unregistered`; `:131-171` — тестовая дверь `#[cfg(test)] pub(crate) mod testing` (Д-13 журнала 5.0). `for_keys` в двери НЕТ — он `pub(crate)` в `surface.rs:1009`, и его единственные потребители — два теста в том же файле (`:1864`, `:1881`) [К-5].
- `beacon/plane.rs:120-146` — трейт `CommitteeReads`: `read_at`, `committee(epoch, at)`, `committee_bls(epoch, at)`, `dkg_qual(epoch, at) -> Option<(bool, bool)>`, provided `committee_pair(target)` (`:143-146`, два `committee` на одном `read_at`). `qual_read_at` в трейте УЖЕ НЕТ (док `:109-119` объясняет, почему он был и почему ушёл) — Д-14 журнала 5.0 закрыт 4.1.
- `beacon/plane.rs:471-540` — `ValidatorInputs` (семь generic-параметров p2p; `committees: Arc<dyn CommitteeReads>` `:504`, `heights: mpsc::Receiver<u64>` `:507`, `geometry: watch` `:521`, `partition_prefix` `:538`); `journal_partition` `:543`.
- `beacon/actor.rs:1649` — `maybe_start`; пара ростеров через `self.committee_pair_for` `:1680-1690`, решение по `next == cur`; бит `changed` здесь НЕ читается [К-7].
- `beacon/carry.rs` — `DkgQualFor` `:40`, `CarryVerdict` `:44`, `chain_key_epoch` `:74`, `chain_key_epoch_memoised` `:110`, `select_carry_scheme` `:162`, `DkgQualProbe` `:185`, `frozen_dkg_qual` `:227` со стражем `!(bit || committed)` `:238-240` и write-once мемо `:241-243`.
- `beacon/keys.rs` — `InvalidSeed` `:56`, `KeySource` `:73`, `BeaconKeys` `:148` (`with_persistence` `:189`, `cached_only` `:216`, `cached_at_least` `:231`, `attested` `:250`, `set_pk` `:272`, `retain_from` `:365`, `on_invalid_seed` `:397` — потребитель 5.2, `subscribe` `:414`), `AgreedKeyAt` `:433`, `AgreedKeys` `:447`, `KeySources` `:496`, `memoise_carry` `:615`.
- `beacon/resolve.rs` — `mint_diverges_from_attested` `:41`, `beacon_share_resolver` `:69`; тест `a_stable_committees_attested_mint_outlives_the_retention_window` `:304` (по проекту §7 переписывается над новым владельцем).
- `beacon/actor.rs:173` — `pub type CeremonyStore = Arc<RwLock<BTreeMap<u64, (CeremonyOutput, Share)>>>`; `adopt_share` `:1014`, вызовы `:1624` и `:2392`; гейт `validate_share_on_poly` стоит ТОЛЬКО на recompute-пути (`:2358`, дока `:2314`) — §5.5 требует сделать его обязательным перед ЛЮБЫМ `adopt_share`; `validate_share_on_poly` определён `beacon/outcome.rs:99`.
- `beacon/share_state.rs` — копия артефакта в share-файле: формат v2 `(output, share, artifact)` (`:10`, `:22-23`, `:74`, `:92-94`), `inner_frame` `:94`, разбор `:128-149`, `encode` `:221-234`, `persist` `:300-316`, загрузка `:322-355`; окно замены `:689-690`.

Что дособрать перед запуском (моё, не агента): форма нового `signer`-гейта `mandatory_at` (`surface.rs`), артефактная половина `follower.rs` и throttle `PULL_MIN_INTERVAL`, `artifact.rs` (`ArtifactStore`, `verify_artifact`, write-behind), тесты стенда, которые заход обязан переписать (C7 `testbed/tests.rs`).
| К-8 | I1 §0(9)(i): by-height-фидер по арифметике окна не может дать эпоху выше `epoch(anchor)+1`, поэтому ветка defer inlet-а недостижима | открыл сам: `committee/store.rs:218-224` — `window = [anchor_epoch − SCHEME_RETENTION_EPOCHS, anchor_epoch + MAX_COMMITTEE_LOOKAHEAD_EPOCHS]`; `committee/mod.rs:616-622` — `commit_height(E) = start(E−2)`; `testbed/fakes.rs:621-623` — `FakeChain::tip()` = `finalized_tip` (тир-F), тот же курсор, что у якоря модуля | **ПОДТВЕРЖДЕНО мной** |
| К-9 | I1 §0(9)(ii): `deliver` дропает ровно тот класс, который заставил бы inlet сделать defer | уже открыто в К-3: `plane_upstream.rs:445-449` (шаг 4, три отказа модуля) + `:486-490` (шаг 5 — дроп, `FRONTIER_DROPPED`, `waiters.remove` ⇒ `fetch_one` в `None`, возврат `true`) | **ПОДТВЕРЖДЕНО мной** |
| К-10 | Моя собственная ошибка в постановке I1, п. 5(б): я назвал фикстуру R-129 «ветка defer inlet-а» | открыл `.dpos-study/REGISTER.md:997-1006`: R-129 — про **marshal-резолвер** (`CertProvider::scoped == None` ⇒ CW `verify_delivered` ⇒ `deliver == false` ⇒ `block!` честного пира), а не про inlet; достижимость — «через R-128 (форк контракта) и „запись читается, схему собрать нельзя“» | постановка была неверна; фикстура R-129 остаётся ОТКРЫТОЙ для 5.2, маршрут назван: `StandConfig::weights_none_for`/`reverts_for` (уже есть, `stand.rs:108-111`) + узел, получающий финализацию отравленной эпохи по marshal-каналу |
| К-11 | I1 §0(3): «`None` уже стоил 8 с виртуального времени, повтор — не спин» | открыл `plane_upstream.rs:610-662`: `fetch_one` = `tokio::select!{ rx, context.sleep(FRONTIER_FETCH_TIMEOUT=8s) }` (`:169`, `:623-626`). НО путь дропа `deliver` возвращает `None` **немедленно**: `deliver` снял всю запись `waiters` (`:488`), sender дропнут, `rx` резолвится `Err` — и это прямо описано комментарием `:641-647` («The OTHER way in here is a `deliver` DROP»). В keyless-тесте этот путь взят 840 раз при 3332 ingest-ах | утверждение журнала **завышено**: цикл тактуется сетевым round-trip'ом, не таймаутом. Не дефект (round-trip стоит виртуального времени), но вводит в заблуждение — в карту Ф6 |
| К-12 | Тест keyless считает `epoch_at_block(heights[VICTIM], 0, EPOCH_LEN)` с активацией-литералом `0` | открыл `testbed/fakes.rs:51` — `pub(super) const DPOS_ACTIVATION_BLOCK: u64 = 0` | арифметика ВЕРНА; литерал вместо константы — нит в карту Ф6 |
| К-13 | R1 A-03 (SERIOUS): высота доходит в настоящий `dkg_height_tx` ПОСЛЕ `verify_block`/`report_finalization`, а в продакшне tee встаёт в очередь ДО них | открыл `cert_inlet.rs:712-717` (tee `try_send`) против `:735`/`:743` (`verify_block`) и `:739`/`:744` (`report_finalization`) — тик tee действительно стоит ПЕРВЫМ в продакшн-порядке; в стенде пересылка в настоящий канал делается задачей ПОСЛЕ возврата `ingest` (`stand.rs:2648-2653`) | **ПОДТВЕРЖДЕНО мной.** Материально: §0.7(а) проекта требует сравнить МОМЕНТ прихода высоты через tee против marshal-Tip — стенд в текущем виде этого различить не может. Ф6: дренировать `tee_rx` параллельно с `ingest` (`select!` в том же цикле, без отдельной задачи) |
| К-14 | R1 A-04 (MODERATE): «отстающий валидатор» сделан узлом, НАВСЕГДА выкинутым из `committee[2]` — он не дилит и валидатором не является; механизма «`FakeChain` держит EL ниже» нет | открыл `testbed/fakes.rs:326-359` (поля `FakeChain` — удержания тира-F нет) и `testbed/stand.rs:489-510` (`Partition{a, b, after_height, duration}`, `PartitionCfg::for_views`), `:639-644` (`PartitionObservation{heights_at_cut, heights_at_heal}`); `PeerSet::Committee` с `Stand::partition` несовместим (`stand.rs:363-365`), но дефолт `PeerSet::AllNodes` — совместим | **ПОДТВЕРЖДЕНО мной.** Роль «ДОГОНЯЮЩИЙ валидатор» строится существующими ручками: временный раздел члена комитета ⇒ он отстаёт, остаётся членом, догоняет; лаг наблюдаем `PartitionObservation`. Ф6 |
| К-15 | R1 A-05 (MODERATE): путь «архив ЧУЖОГО узла» (первый вход, названный строкой плана) гейта `deliver` не имеет | открыл `plane_upstream.rs:249-257` — `pub trait FrontierMarshal: MarshalSink { latest_pair(), pair_at(height) -> Option<(Cert, OrderBlock)> }`, `:259-268` — `impl FrontierMarshal for MarshalMailbox`; в стенде `marshal_slot: Arc<OnceLock<MarshalMailbox>>` (`stand.rs:2011`) — тот же приём поздней привязки уже есть | **ПОДТВЕРЖДЕНО мной.** Фидер над `pair_at` чужого узла даёт: (а) буквально названный строкой плана вход; (б) достижимость ветки defer inlet-а ⇒ фикстуру R-129 из колонки «Закрывает» строки; (в) снятие конфаунда «гейт плоскости». Ф6 |
| К-16 | Мой собственный «красный до правки» (правило: ≥ 1 на заход воспроизводит оркестратор) | сам заменил `assert_eq!(f.defers, 0, …)` на `assert!(f.defers > 0, …)` в `cert_inlet_tests.rs:485` и прогнал `cargo test -p fluentbase-consensus --lib … a_keyless_admission… --exact`: `FAILED. 0 passed; 1 failed; … finished in 20.53s`, panic-вывод `CertInletFacts { ingests: 3333, rotations: 0, tee_heights: […127], defers: 0 }`. Файл восстановлен, `md5sum -c gates/a2-tree.md5` — все три ЦЕЛ | **ПОДТВЕРЖДЕНО мной своим прогоном**: ветка defer через плоскость не берётся ни разу. Тот же вывод подтвердил A-02: `tee_heights` содержит ~3333 записи, каждая высота повторена ~26 раз — арм `Frontier` переингестит один и тот же серт десятками раз |
| К-17 | R1 A-11 (MINOR): «ротация» плоскости — no-op для by-height фидера | открыл `plane_upstream.rs:684-692` — `PlaneUpstreamHandle::rotate` = `mailbox.cancel(FrontierKey::Latest)` | **ПОДТВЕРЖДЕНО мной.** Для `NextAboveTier` это полный no-op, и равенство `rotations == faults / 3` держится именно на её бездействии — это обязано быть НАЗВАНО в тесте и журнале, иначе читатель верит, что стенд моделирует продакшн-failover. Ф6 |
| К-18 | R1 A-15 (NIT): `cfg.committees = Committees::All;` — уже дефолт | открыл `stand.rs:374-400` (`StandConfig::honest`: `committees: Committees::All`) и `:402-414` (`live` = `..Self::honest`) | факт верен, но вердикт **REJECTED**: явное присваивание делает предпосылку фикстуры независимой от дефолта `honest` и читается на месте. Не убирать |
| К-19 | R1 A-08 (MINOR): `Beacon::Static` + `cert_inlet` достижимы, приёмник настоящего `dkg_height_tx` дропнут, каждая пересылка тикает `dpos_dkg_height_drops_total` | открыл `stand.rs:2474-2488` (`inlet_inputs` захватывает `dkg_height_tx.clone()` ДО строки `let dkg_height_tx = matches!(cfg.beacon, Beacon::Live).then_some(dkg_height_tx)`) и `sync_metrics.rs:337`, `:366` (смысл счётчика — «канал полон») | **ПОДТВЕРЖДЕНО мной.** Ф6: либо гейт «inlet только при `Beacon::Live`», либо громкий отказ конфигурации |
| К-20 | Ревьюер R1 не прогнал НИ ОДНОЙ команды | из его отчёта: классификатор auto-mode запретил ему `sed -i` и любой `cargo test`; ворота он прочитал из моих файлов | Отклонение от решения пользователя 09-11 («ворота ревьюер прогоняет САМ»). Все его поведенческие утверждения — чтение кода; поэтому SERIOUS+ и REJECTED-кандидаты я открыл сам (К-13…К-19). Записано в «Всплыло» |

## Карта Ф6 для 5.0а (мои решения по 19 находкам R1; расклад — в `E5-prompts/5.0a-A-fix-1.md`)

**FIXED (13):** A-03 (SERIOUS, центральная — конкурентный дренаж `tee_rx`, чтобы пересылка шла в продакшн-порядке ДО `verify_block`/`report_finalization`); A-04 (настоящая роль догоняющего ЧЛЕНА комитета через `Stand::partition`, а не навсегда исключённого узла); A-05 (вход из архива ЧУЖОГО узла через `FrontierMarshal::pair_at` — буквально первый вход строки плана, он же делает ветку defer достижимой и даёт фикстуру R-129); A-06 (`with_carry_forward_fail_metric` как точный свидетель «ключ был в момент verify»); A-01 (комментарий и журнал про такт цикла); A-11 (назвать, что `rotate` плоскости — no-op для by-height); A-17 (`dkg_height_drops == 0` и в keyed-тесте); A-08 (`Beacon::Static` + inlet); A-09 (процессность `frontier_dropped`); A-10 (перепинить якоря журнала); A-14 (константы вместо литералов); A-18 (`errors()`/lockstep в keyed); A-19 (ассерт отрицательного контроля); A-16 закрывается через A-05.

**RECORDED (4):** A-02 (цена арма `Frontier` — 3333 ingest-а на ≤127 высот, измерено мной; это плата за пин гейта плоскости, другого способа через плоскость нет); A-07 (`tee == (1..=ingests)` — пин соотношения скоростей; набор {первый==1, строго возрастает, len==ingests, последний==ingests} ему эквивалентен, ослаблять нечего); A-12 (`Outcome` — второй общий с 5.1 тип); A-13 (`UpstreamCounters` смешивают трафик inlet-а и зонда).

**REJECTED (1):** A-15 — `cfg.committees = Committees::All;` действительно дефолт (`stand.rs:374-400`, `:402-414`, открыл сам), но явное присваивание делает предпосылку фикстуры независимой от дефолта и читается на месте. Не убирать.

«Красный до правки» этого прохода — A-05: ассерт `defers > 0` красный на плоскостном фидере (воспроизведено МНОЙ, К-16) и обязан стать зелёным на архивном входе.

## Коммиты

| sha | что | пути |
|---|---|---|
| `7790d1bc` | `test(testbed): run the production CertInlet as a second marshal producer` | `testbed/{stand,mod,cert_inlet_tests}.rs` — ровно три, `git status --short` перед коммитом показывал их и НИЧЕГО больше под `crates`; `E4-ORCHESTRATOR.md` в индекс не попадал |

## Ворота Ф3 на финальном дереве 5.0а (тег `a3`, мои прогоны) и мои проверки третьего прохода

| ворота | база `bec013a7` | `a3` | вердикт |
|---|---|---|---|
| `--lib` | 686/0 | `ok. 692 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 172.97s` | зелёные |
| стенд с фичей | 48/0 | `ok. 55 passed; 0 failed; 0 ignored; 0 measured; 646 filtered out; finished in 217.34s` | зелёные |
| стенд без фичи | 40/0 | `ok. 46 passed; 0 failed; 0 ignored; 0 measured; 646 filtered out; finished in 171.78s` | зелёные |
| node | 55/0 | `ok. 55 passed; 0 failed; …; finished in 60.31s` | без изменений |
| reader | 64/0 | `ok. 64 passed; 0 failed; …` + `0 passed; 1 ignored` | без изменений |
| slasher | 16/0 | `ok. 16 passed; 0 failed; …; finished in 0.04s` | без изменений |
| clippy ×3 крейта | 2 чужих | набор строк ПОБАЙТНО тот же, что база (`diff` сделан мной; расходится только то, какая из двух строк `fluentbase-node` несёт «(1 duplicate)» — порядок сборки) | зелёные |
| clippy с фичей / `fmt --check` / doc | 0 / чисто / 6 | 0 / 0 `Diff in` / 6 | зелёные |

Счёт тестов: файл `testbed/cert_inlet_tests.rs` даёт 7 тестов, 6 из них компилируются без фичи — отсюда +6 в `--lib` и в стенде без фичи, +7 в стенде с фичей.

**Мои прогоны по утверждениям третьего прохода** (`cargo test … testbed::cert_inlet_tests -- --nocapture --test-threads=1`, с фичей и без):
- `a_donors_archive_hands_the_inlet_an_epoch_it_cannot_read_and_it_defers` → `ingests=159 teed=127 teed_top=127 window_top=127 defers=32 rotations=0` — ветка defer (`cert_inlet.rs:618-640`) исполнена 32 раза и как НЕ-фолт. A-05 закрыта.
- `a_forged_seed_slot_costs_the_upstream_a_rotation_once_the_epoch_key_is_held` → `forged=[65..70] ingested_forged=[65..70] ingests=102 rotations=2 defers=0 carry_forward_fails=6`. A-06 закрыта.
- `a_catching_up_committee_member_is_fed_by_its_inlet_while_its_el_is_behind` → `heal=[lag 8 vs majority 22] in_window=[9,9,10,10,11] ingests=85 rotations=0 defers=0 cff=0`. A-04 закрыта.
- `the_production_tee_wiring_feeds_the_dkg_clock_with_no_drain_of_ours` → `ingests=23 tee_heights=0 dkg_clock=24`. A-03 закрыта.
- **Найдено мной сверх ревью:** числа keyless-теста зависят от состава прогона в процессе — без фичи `ingests=3333, plane_dropped=881`, с фичей `3332, 840`. `stand::counter_of` (`stand.rs:697`) процессный, поэтому печатаемые числа не атрибутируемы одному тесту; ассерты — неравенства и выдерживают. Записано в `EXPERIMENTS.md` §1 и в `15_smoke_cases…md` §15.a.1.

## Ф7 — что я правил в документации (сам)

- `.claude/dpos_architecture/15_smoke_cases_as_behavioral_spec_devnet_lo.md`: `:201` — клауза «the node's poller and inlet feeders do not exist here» была ЛОЖНОЙ после 5.0а, переписана; `:208` — якорь `StandConfig.beacon` перепинен `stand.rs:146` → `:416`; добавлена секция **§15.a.1** (форма стендового inlet-а, три фидера, две разводки tee, таблица семи тестов с фальсификаторами, граница точности процессных счётчиков).
- `.claude/dpos_architecture/00_preamble.md`: новая запись `verified-against` 2026-09-13 на 5.0а.
- `.claude/dpos_architecture/09_followers.md:1309` («rotated-out validator … has no cert-inlet») — проверил, это утверждение о ПРОДАКШНЕ, а не о стенде; не дрейф, не трогал.
- `.dpos-study/PLAN.md`: строка 5.0а → `[x] … сделано 2026-09-13 — 7790d1bc` с тем, что НЕ сделано внутри строки; §1 ворота (`node`, `staking-reader`, `consensus`, `fmt`) перемерены на `7790d1bc` с датой 09-13. Запись «`cargo fmt --check` красный на шести чужих файлах» (09-09) была устаревшей — на `bec013a7` и на `7790d1bc` он чист (мои прогоны `base` и `a3`).
- `.dpos-study/REGISTER.md`: R-008 — новый статус 09-13 (стендовая половина Ex-21 закрыта на обоих плечах, с механизмом «плоскость верифицирует без оракула»); R-129 — новый статус 09-13 (фикстура частично построена, чего не хватает самому тесту).
- `.dpos-study/EXPERIMENTS.md`: §1 — новая секция 2026-09-13 (шесть пунктов с цифрами моих прогонов); §3 п.1 — Ex-21 переписан: стендовая половина ЗАКРЫТА, открыта только девнетная приёмка и она после 5.2; §1 строка про «Ex-21/Ex-19 не покрыты стендом целиком» уточнена.

**Текстовые проверки перед `docs(dpos)`** (мои команды): строки на многоточие в конце — 0 во всех правленых файлах; нечётное число обратных кавычек — 0 в `PLAN.md`, `REGISTER.md`, `EXPERIMENTS.md`, этом файле и в двух постановках; в `E5-0a-A.md` 26 строк и в `E5-0a-A-REVIEW.md` 2 — это 13 и 1 ПАРА код-спанов, разорванных переносом строки (проверил попарно), не дефект; в `5.0a-A-impl-1.md` была ОДНА непарная — мой собственный код-спан на имя `testbed/tests.rs`, у которого закрывающую кавычку съели две звёздочки; исправлен. Две запятые подряд — 0; пустые пары обратных кавычек — 0; пустых круглых скобок вне кода — 0 (все вхождения внутри код-спанов: `Arc<dyn Fn()`, `sig::<abi::…>()`, `Ok(())` и нотация кодека в `REGISTER.md`).


## Отклонения от проекта (Д-nn)

| Д | строка | что иначе | причина (file:line) | меняет П/Д |
|---|---|---|---|---|
| О-1 | этап | Строки идут СТРОГО последовательно; разрешённая планом параллель 5.0а ‖ 5.1 не используется | Ф3 — ворота по рабочему дереву; два исполнителя, пишущих одновременно, делают дерево неатрибутируемым объектом и для ворот, и для ревью (`E4-ORCHESTRATOR.md:11` — контур Э4 последователен по строкам). Цена — время, не свойство | нет |

## Всплыло

- Ревьюер R1 не смог запустить ни одной команды: классификатор auto-mode в его сессии запретил `sed -i` и `cargo test`. Решение пользователя 09-11 требует, чтобы ворота ревьюер гонял сам. Обход на этот заход: ворота он прочитал из моих файлов, а все SERIOUS+/REJECTED я открыл сам. Для следующих ревью вписывать в постановку разрешённые формы команд заранее.
- В рабочем дереве под `crates/dpos/consensus/src/.claude/session-reads/` лежат журналы чужих сессий (gitignored) — обычный `grep -rn` по `crates/` в них попадает и выдаёт мусор. Правило сессии: поиск по коду — только `git grep`.

## Hard-stop

Нет.
