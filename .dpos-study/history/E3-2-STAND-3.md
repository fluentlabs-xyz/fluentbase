# Э3.2 — стенд, сессия 3: шаг 4 оценки (beacon-плоскость с живым DKG) (2026-09-09)

Дерево `~/Work/fluentbase`, ветка `djadjka/dpos-reth-2.2-squashed`, старт с `888c0eda`.
Commonware `v2026.4.0` (`3c4e02c`), чекаут `monorepo-27b478c9bb41d208/3c4e02c` (далее `CW:`); в паниках cargo подставляет второй чекаут `monorepo-9732103c47eb4665/3c4e02c` (тот же коммит).
Теги: `[KNOWN]` — прочитано/выполнено в этой сессии, `[LIKELY]` — согласуется, не проверено, `[ГИПОТЕЗА]` — вывод.
Компакций контекста за сессию: 0; `TASK.md` в scratchpad записан до первого вызова инструментов. Длинные строки читал `sed -n`, `cat -n` и инструментом Read; `cut -c`/`head -c` для чтения кода не использовал (`cut -c1-N` — только для обрезки вывода grep/тестов в отчётах).

## 1. Итог

**Исход пробы A — составилось.** [KNOWN] `beacon::build` на N=4 поднялся в стенде с первой попытки без единой прод-правки: throwaway-тест (`probe_a_live_beacon_composes`, удалён) дал `heights=[70,70,70,70]` за 69,9 с виртуальных / 4,8 с реальных, на каждом узле артефакт эпохи 2 (1682 байта), σ с высоты 64 (= `epoch_start(2)`, round `(2, view 1)`) на всех четырёх, `dkg_ceremony_ok_total = 1` на каждом узле. Единственное предупреждение плоскости: `dkg agree: too few members confirm they hold the pinned dealer logs … epoch=2 view=1 covering=1 quorum=3` — первый view агремента не набрал подтверждений, следующий набрал (артефакт есть). Первый ключ по коду — на ЭПОХУ 2, не 1: `DETERMINISTIC_BOOTSTRAP_EPOCH = 2` (`beacon/actor.rs:123`), эпохи 0–1 бессидовые (`carry.rs:115-125` возвращает `Some(None)` ниже bootstrap); формулировка задания «ключ эпохи 1» прочитана как «первый ключ эпохи».

[KNOWN] Коммиты, по явным путям, без трейлеров:

| Шаг | Коммит | Что |
|---|---|---|
| C — прод-правка (одна, нейтральная) | `40fe8f3e` `refactor(consensus): prefix the beacon plane journals per node` | `beacon/plane.rs`: `journal_partition(prefix, base)`; три журнала `beacon::build` (`beacon-key-ordinal`, `beacon-seed-ordinal`, `beacon-artifact-metadata`) открываются под `BeaconConfig.partition_prefix`; тест литералов `plane::tests::journal_partitions_are_the_production_names_under_the_empty_prefix`; doc-комментарий поля |
| B — стройка | `08870d69` `test(consensus): run a live DKG in the testbed` | `testbed/{mod,fakes,stand,tests}.rs`: `Beacon::{Static,Live}`, `StandConfig::live`, `Role::AbsentBeacon`, `cold_start_epoch`, σ/артефакты/метрики в `Outcome`, тесты B1–B5 + B4′ |
| после контр-ревью | `a860e588` `test(consensus): tighten the live-DKG testbed assertions` | `testbed/{stand,tests}.rs`: `verify_seed` на КАЖДОМ узле; `errors().is_empty()` в B1–B4; сняты три структурных ассерта (`artifacts[3].is_empty()`, повторный `digest()`, `dkg_ceremony_ok == 0` в фазе 2 B4); B4′ на `resume_from`, lockstep фазы 1; B5 сравнивает и само поле σ; doc `Role::AbsentBeacon` — что он моделирует |

[KNOWN] Не сделано и почему: правки `.claude/dpos_architecture/` (00, 08, 12, 13, 15, TOC) сделаны в дереве, в коммиты не входят (`.claude/` в `.gitignore`); этот отчёт и строка в `PLAN.md` не закоммичены (нет в списке разрешённых коммитов); R-002/R-008 не поставлены (по заданию — 3.3, точки в §7); `REGISTER.md` не тронут.

## 2. Препятствия пробы A

[KNOWN] Таблица препятствий компиляции/паники — пуста: проба скомпилировалась с первого раза (одно предупреждение `dead_code` о ещё не использованном `Role::AbsentBeacon`) и прошла без паник. Проверено прогоном `cargo test … probe_a_live_beacon_composes` (`ok`, 4,89 с). Что при этом оказалось не так, как читалось по журналам, и что потребовало правок ПОСЛЕ пробы:

| # | Место | Что мешает | Обходится тестом? | Правка |
|---|---|---|---|---|
| 1 | `beacon/plane.rs:522,560,579` (до правки) — `key_journal::open(.., KEY_JOURNAL_PARTITION)`, `seed_journal::open(.., SEED_JOURNAL_PARTITION, ..)`, `artifact::open(.., ARTIFACT_JOURNAL_PARTITION)` | [KNOWN] константы `dpos.rs:116-125` подставляются ВНУТРИ `beacon::build` без префикса — 4 плоскости на одном in-memory `Storage` писали ОДИН `beacon-key-ordinal`/`-seed-ordinal`/`-artifact-metadata` (та же коллизия, что §2 #2 сессии 1 для `consensus_epoch_{E}`). Проба прошла и так — честные узлы пишут одинаковые значения, коллизия невидима (как и в сессии 1 до реплея). Сессия 1 §4 записала «константы передаются в `beacon::build` из прод-кода» — неточно: они импортируются в `plane.rs:55` | нет (имена внутри функции) | прод, нейтральная (§4): `journal_partition(&partition_prefix, ..)`; прод передаёт `""` (`node/src/dpos.rs:1968`) |
| 2 | `stand.rs::build_node` boundary relay — cold-start arm всегда `(0, committee[0])` | [KNOWN] на реплее (B4) узлы поднялись, передеривили 1..70 и встали в эпохе 0 без движка эпохи 2: `[70,70,70,70]`, без halt и ошибок, таймаут; INFO-лог реплея: только `epoch entered (signer) epoch=Epoch(0)` ×4, ни одной строки для эпох 1–2 (hook `report(Update::Block)` во время догона из архива не стреляет для границ 31/63 — relay ждёт границу, которой не будет). Прод здесь делает `EpochTransition::cold_start(hash, fin)` в эпохе финализированного блока | да | стенд: `StandConfig.cold_start_epoch` (0 для свежей цепи; B4 ставит `70 / 32 = 2`) |
| 3 | `Outcome.artifacts` — сравнение артефактов между узлами байтами | [KNOWN] 1682-байтные артефакты совпадают до байта 1584 и различаются в 49-байтном хвосте на каждой паре узлов: половина `Finalization` — агрегат над тем подмножеством ≥ кворума голосов, которое собрал каждый инстанс (ловушка из памяти «не сравнивать битмап сертификата»); половина `DkgProposal` (эпоха, пиннутый набор логов, `PK_E` + полином, подтверждения — `dkg_agree.rs:447-452,558-570`) — одна | да | стенд: сравнивать `decode_artifact(..).0.encode()` и `digest()`; доки §8/§13 |
| 4 | имена метрик | [KNOWN] prometheus-client дописывает `_total` к зарегистрированному имени: `node0_dkg_ceremony_ok_total_total 1`; счётчики демоута зарегистрированы как `epoch_engine_demoted_*_total` (`metrics.rs:203-217`), не `engine_demoted_*` | да | `Outcome::metric` ищет `key` и `key_total` |

Что подтвердилось без правок: [KNOWN] `share_dir` на `std::fs` обходится каталогом в `std::env::temp_dir()` (как `actor.rs:3224` `fresh_share_dir`); ничего в `beacon::build` не прибито к `tokio::Context` (сигнатура `build<E: BufferPooler + Clock + CryptoRngCore + Metrics + Spawner + Storage + …>`, `plane.rs:415-431`) — deterministic `Context` подходит; стенных часов в beacon нет: `grep -n 'Instant::now\|SystemTime::now\|thread::sleep' beacon/*.rs` — пусто; имена журналов в проде не изменились: `dpos.rs:116,119,125` = `beacon-seed-ordinal`, `beacon-key-ordinal`, `beacon-artifact-metadata` — закреплены тестом литералов; ловушка (3) из памяти (свой namespace у agreement-плоскости) — обеспечивается прод-кодом (`dkg_engine.rs:289-300` `dkg_namespace(fluent_namespace(chain_id))`), стенду делать нечего; `SharedMux` (`outer.rs:48`) — те же `Arc<tokio::sync::Mutex<MuxHandle>>`, что стенд уже держит для `OuterEngine::start`; vote-backup форвардер стенда уже фильтровал `subchannel < DKG_SUBCHANNEL_BASE` (сессия 1).

## 3. Тесты B1–B6

Все — `testbed/tests.rs`, `StandConfig::live(4, seed)`, `epoch_len = 32` (окно сделки `32 − DKG_MARGIN_BLOCKS(20) = 12` блоков, `actor.rs:115`), латентность 10 мс, потерь 0. Реальное время — `Outcome.real_elapsed`, `--test-threads=1`, debug-профиль, один прогон [KNOWN].

| Тест | Что проверяет | Что упало бы | Вирт. | Реал. |
|---|---|---|---|---|
| (B1) `four_nodes_agree_the_epoch_key_and_carry_the_seed_across_the_boundary` | до 72: на всех узлах артефакт эпохи 2 с байтово равным `DkgProposal` (и `PK_2.encode()` равен между узлами), артефактов для 0/1/3 нет, 4 dealer-лога в пине; высоты 1..63 без σ на всех; с 64 по 72 на каждом узле `Seed` равен узлу 0 (round `(2, view h−63)`; `Seed: PartialEq` по раунду и BLS-подписи — равенство значений, не байтов сериализации), на КАЖДОМ узле проходит `verify_seed(PK_2, seed_namespace(fluent_namespace(CHAIN_ID)), round, σ)`, `prev_randao_from_seed` равен; `dkg_ceremony_ok=1`, `_fail=0`, `epoch_engine_demoted_no_polynomial=0` на каждом; ни одной ERROR-строки | разные proposal/`PK_2`; σ отсутствует или отличается хоть на одном узле на h≥64; σ не проходит под `PK_2`; σ до 64; демоут; проваленная церемония; ERROR | 71,9 с | **4,81 с** |
| (B2) `three_boundaries_with_committee_rotation_keep_dkg_qual_honest` | комитеты: эпохи 0–2 все четыре, 3–4 `[0,1,2]`, 5 — все четыре; до 168: артефакты 2, 3, 5 на ВСЕХ узлах (узел 3 не член committee[3], но артефакт 3 у него есть — подтянут через `pull_keys`), для 4 — ни у кого; `PK_2≠PK_3≠PK_5` (все три пары); σ каждой высоты эпох 3 И 4 проходит под `PK_3`, σ эпохи 4 НЕ проходит ни под `PK_2`, ни под `PK_5`; эпохи 5 — под `PK_5`; σ равны между узлами; `dkg_ceremony_ok = [3,3,3,2]` (узел 3: эпохи 2 и 5) | артефакт эпохи 4 (перековка на стабильном комитете); повтор ключа; σ эпохи 4 не под `PK_3`; узел встал на границе (ловушка (1)) | 167,9 с | 12,05 с |
| (B3) `one_absent_dealer_does_not_stop_the_key` | узел 3 = `Role::AbsentBeacon` (`beacon::absent`, без share/лога/места в агременте/регистрации `BEACON_CHANNEL`); трое: артефакт эпохи 2 с ТРЕМЯ логами в пине, `PK_2` равен, `ceremony_ok=1`; цепь на троих до 72 с согласованной σ; узел 3 стоит на 63, halt нет, ERROR нет. Имя теста — из задания; модель — «валидатор без beacon-модуля» (верификатор с эпохи 0), не «член, не сдавший dealing» (тот получил бы чужие dealings, восстановил share над пином и подписывал бы дальше — `actor.rs`, doc `drive_finalization`); для церемонии эффект тот же — на одного dealer'а меньше | трое не сковали; четыре лога в пине; halt; узел 3 прошёл 64 (деривил без σ в обязательной эпохе) | 71,9 с | 4,27 с |
| (B4) `restart_replays_key_and_seed_journals` | стоп на 70 → `Checkpoint` → все узлы заново над тем же Storage и теми же share-каталогами, `cold_start_epoch=2`: proposal артефакта 2 == фазы 1; σ на 64..70 == фазы 1 (`Seed` equality); хэши фазы 1 — префикс фазы 2; цепь до 82 с σ (71..82 равны между узлами, под `PK_2`); `epoch_engine_demoted_no_polynomial=0`; ERROR нет; нет строки `re-agreeing`; `dkg_ceremony_ok` фазы 2 печатается, НЕ ассертится (actor стартует только `now+1` = 3, на стабильном расписании это carry — ноль от расписания, не от рестарта) | узел не поднялся; другой proposal; σ отсутствует или на другом раунде (другое ЗНАЧЕНИЕ σ на том же раунде недостижимо — σ уникальна по `(round, PK)`); демоут (share не перечитан); ERROR; две цепи | 70 + 12 с | 4,68 + **1,38 с** |
| (B4′) `restart_without_the_share_dirs_parks_the_chain_verify_only` — негативный контроль к B4 | то же, но `share_root` стёрт между фазами: фаза 1 в lockstep, 1..70 передеривлены (вектор высот), все узлы `Withheld(NoUsableShare)` (`epoch_engine_demoted_no_polynomial ≥ 1`), цепь стоит на `[resume_from; 4]` (= 70), halt нет, `timed_out`; реплей журналов здесь НЕ ассертится (это B4) | цепь пошла без share (тогда B4 ничего не доказывает про перечитывание); нет демоута; halt | 70 + 60 с | ≈4,7 + ≈1,5 с |
| (B5) `a_live_dkg_run_reproduces_the_seed_trace_byte_for_byte` | трасса `(height, view, leader, digest, hash, σ)` узла 0 (6083 байта, σ есть на 70) — seed 1 трижды побайтно одна; seed 2 — другая, и её σ на 70 — другая (поле σ живое в сравнении; различие ожидаемо — другие ключи ⇒ другой `PK_2`) | любые стенные часы/OS-случайность в плоскости (DKG-раздача, view агремента, восстановленная σ) | 4×70 с | 4,73/4,40/4,39/4,39 с |
| (B6) время | реальное время B1 — 4,81 с при 71,9 с виртуальных (< 10 с); grep стенных часов в `beacon/**` пуст (§2) | — | — | — |

Старые тесты (числа — по дереву `a860e588`): [KNOWN] 14 старых + (eq) — без изменений ассертов, зелёные (`Beacon::Static` по умолчанию в `StandConfig::honest`); в (6) `trace_bytes` теперь включает σ (Static раздаёт σ на любой раунд) — ассерты те же, doc-комментарий обновлён. Итог набора: стенд без фичи 20/0/0 (14 + 6), с `--features dpos-devnet-byzantine` 21/0/0; крейт целиком `cargo test -p fluentbase-consensus`: **629+3+5+13 / 0**, lib 0 ignored (было 622: +6 стенда, +1 тест литералов `plane::tests`).

B3, ответ по коду ДО прогона [KNOWN]: dealer-кворум при n=4 под `N3f1` — `n−f = 3` (`ceremony.rs:1206,1418` комментарии тестов «n=5 ⇒ dealer quorum n−f = 4»; `Dealer::finalize` даёт `TooManyReveals` только при reveals > `max_reveals` — `CW:cryptography/src/bls12381/dkg.rs:1551-1566`); entry bar агремента при n=4 — голый кворум 3 (`dkg_engine.rs:1516-1518`); агремент — simplex на 4 местах, 3 живых ⇒ финализирует. Ожидалось: трое сковывают над 3-логовым пином, цепь идёт; узел 3 без источника σ (`absent::seed_for = None`, `mandatory_at(2) = true` — `surface.rs:169-171`, default impl) ⇒ `OwnRoundSeed::Missing` (`executor.rs:2936`; парковка — `:1796`) ⇒ паркуется на 64. Прогон дал ровно это: `heights=[72,72,72,63]`, `dealers=3`, halt пуст. Расхождений нет. Оговорка: узел 3 с `absent` — верификатор с эпохи 0 (`Absent::signer_scheme = Withheld` всегда, разведка §2 #4), то есть в эпохах 0–1 подписывают трое из четырёх; «член, который просто не сдал share» (подписывает до 2, выбывает на 2) — другая роль, не ставил.

## 4. Прод-правки

Одна. [KNOWN]

| Файл | Изменено | Нейтральность |
|---|---|---|
| `beacon/plane.rs` | `pub(crate) fn journal_partition(prefix, base) -> format!("{prefix}{base}")`; три вызова `open` получают `&journal_partition(&partition_prefix, KEY/SEED/ARTIFACT_JOURNAL_PARTITION)`; doc-комментарий `BeaconConfig.partition_prefix` расширен; `mod tests` с тестом литералов | прод передаёт `partition_prefix: String::new()` — `node/src/dpos.rs:1968` (единственный прод-вызов `beacon::build`; `dpos.rs:3512,3554` — follower-путь без `beacon::build`); `format!("{}{}", "", base) == base` — закреплено тестом `journal_partitions_are_the_production_names_under_the_empty_prefix` (`"beacon-key-ordinal"`, `"beacon-seed-ordinal"`, `"beacon-artifact-metadata"`, и `"node3-beacon-seed-ordinal"`); `grep -rn 'KEY_JOURNAL_PARTITION\|SEED_JOURNAL_PARTITION\|ARTIFACT_JOURNAL_PARTITION' crates bins` — определения в `dpos.rs:116-125` и три вызова в `plane.rs`, других потребителей нет; `key_journal::open`/`artifact::open` трактуют ПУСТУЮ строку как «без журнала» (`key_journal.rs:238`, `artifact.rs:634`) — с `""`-префиксом строка не пуста, поведение прежнее. Полные ворота ниже |

Ворота (все [KNOWN], команды задания, дерево = оба коммита):

| Ворота | Результат |
|---|---|
| `cargo test -p fluentbase-consensus` | 629+3+5+13 / 0; lib 0 ignored (`print_corpus` и 1 doctest ignored — как на базе) |
| `cargo test -p fluentbase-consensus --features dpos-devnet-byzantine --lib testbed` | 21 / 0 |
| `cargo test -p fluentbase-node -p fluentbase-staking-reader -p fluentbase-p2p -p fluentbase-bls` | EXIT 0, 0 failed во всех 15 бинарях (46+1+1+3+1+3+2+57+32+1+59 passed; ignored — как на базе) |
| `cargo check --workspace` | Finished |
| `cargo clippy -p fluentbase-consensus --all-targets` без фичи / с фичей | первый прогон — 4 warnings в моих тестовых файлах (`redundant closure`, 2× `get(..).is_none()`, `>= y + 1`) — исправлены до коммита; второй прогон без фичи и с фичей — 0 warnings |
| `rustfmt --check` `testbed/{mod,fakes,stand,tests}.rs`, `beacon/plane.rs` | чисто (exit 0; предупреждения rustfmt о nightly-опциях `rustfmt.toml` — как в сессиях 1–2) |

Доки: `grep -rn 'StaticRandomness\|beacon::build\|BeaconConfig\|share_dir\|AgreedArtifact\|carry-forward\|carry_forward' .claude/dpos_architecture/` — 45 попаданий; устаревшими были: `15_…:132` («randomness = `StaticRandomness` … step 4») — переписано; `08_…:2214` (префикс только для `dkg_epoch_{E}`) — дополнено тремя журналами; `08_…:2261`, `12_…:64` (`ARTIFACT_JOURNAL_PARTITION` без префикса) — дополнены. Добавлено: `08_…` после `AgreedArtifact = (DkgProposal, Finalization…)` — абзац «[stand, 2026-09-09]» о несовпадении сертификатной половины; `13_…` правило 23 — третье следствие (стенные часы в beacon отсутствуют, числа B1/B5); `15_…` §15.a — заголовок «steps 1, 3 and 4», вводный абзац про `Beacon::{Static,Live}`/`cold_start_epoch`/`AbsentBeacon`/наблюдения, строка (6) с σ, шесть новых строк таблицы, список «не показывает»; `00_preamble.md` — запись `verified-against` за шаг 4; `TOC.md` — счётчики 00 (1155→1174), 08 (2435→2446), 13 (796→800), 15 (175→203). Остальные попадания (`03_…:104,134`, `09_…:77,200,214,367-370`, `08_…:789-1705`, `00a`, `01`, `12:21`) — верны как были, не трогал.

## 5. Что стенд НЕ показывает после сессии, и что стало постановимым

- Контракт: `dkgQual` и пара комитетов — из расписания по правилу контракта `committee[e] != committee[e−1]` (`carry.rs:3-8`); что закоммитил бы КОНТРАКТ — не читается (шаг 5). `dkg_qual_at` — константа `B256::ZERO`: «прочитано на финализированном хэше» не моделируется, `frozen_dkg_qual` (`carry.rs:223-243`) отвечает сразу для всех эпох расписания — в проде эпоха становится «committed» на границе E−1; ранняя читаемость `committee[target]` в стенде не помешала (actor стартует только `now+1`, `actor.rs:1253`), но сценарий «комитет ещё не закоммичен, когда actor вошёл в E» (комментарий `actor.rs:1231-1240`) стенд не воспроизводит.
- Один клок высоты: только ordering tip `FluentApp::report(Update::Tip)` → `dkg_height_tx` (`application.rs:1029-1034`); поллер `fin + K` и inlet-tee (`node/src/dpos.rs:1647-1653, 809-818`) отсутствуют — «задержанный клок съедает окно» (`actor.rs:2927-2933`) не моделируется.
- Дедлайны DKG под реальной латентностью/потерями: 10 мс, 0 потерь; ловушка (4) из памяти (> f без share клинит эпоху) — не ставилась (B3 — ровно f=1 без share, эпоха идёт).
- Шифрованный конверт share (`share_seal_key: None`), `reconcile_journals` при стёртом каталоге — B4′ стирает каталог целиком, torn-journal (`JournalLoad::Torn`, `actor.rs:1673`) не ставился.
- Reth/контракт/re-jump/R-020/devp2p — как в сессиях 1–2.
- **Постановимые R-записи (постановка — 3.3):**
  - **R-002** (два `Reveal` одного dealer'а): точка — обёртка `Sender` над половиной `BEACON_CHANNEL`, которую `build_node` кладёт в `BeaconConfig.beacon_channel` (`stand.rs`, `register(BEACON_CHANNEL)`), роль `Role::TwoReveals`. Второй лог НЕЛЬЗЯ переподписать из обёртки: `SignedDealerLog::sign` — приватная (`CW:cryptography/src/bls12381/dkg.rs:1203`), единственный публичный производитель — `Dealer::finalize::<N3f1>()` (`:1551`), и `Dealer::new(info, me)` публичен (`:1688`), `Info::new` тоже (`:686`). Значит обёртка строит ВТОРОЙ независимый `Dealer` над тем же `Info` (комитет эпохи из расписания, тот же `dkg_namespace`) и ключом узла, без акков, `finalize` → второй валидно подписанный конфликтующий лог (с `TooManyReveals`, если reveals > f), и шлёт оба `DkgBody::Reveal` (кодек `dkg_msg.rs:145`). Наблюдение, которое тест должен ловить: (а) `PK_2` совпал на всех узлах и цепь прошла 64 — агремент пиннул один из логов и R-002 в этой форме не воспроизводится; (б) `dkg_ceremony_fail_total > 0` / `dkg_agree_bar_unmet` / узлы с разным `PK_2` / стоп на 64 — воспроизведение R-002. Ставить надо оба исхода как взаимоисключающие ассерты с печатью.
  - **R-008** (сертификат с подменённым σ): σ лежит в фиксированном слоте подписи сертификата — `vote 48 ‖ flag 1 ‖ seed 48` = 97 байт (`bls/src/combined_scheme.rs:48-50,93-104,136-155`, `write_seed_slot`/`read_seed_slot`). Две точки: (1) follower через upstream-плоскость (конфигурация (4c)) — обёртка `Producer` в `stand.rs::frontier_plane` (сейчас `fakes::CountingHandler::produce`) подменяет у `FrontierKey::Finalized{h}` хвост сертификата (flag=1, σ' другого раунда или случайная точка G1); наблюдение: `deliveries_rejected > 0` у узла 3, `seeds[3][h] != σ'`, узел 3 не деривит от σ' (иначе `Divergence::Minority{node:3}`) — и, для случая «ключа ещё нет» (собственно R-008: `SeedCheck::NoKey` принимает любой σ), стартовать follower без артефакта эпохи (у `Role::AbsentBeacon` есть только `absent`; нужна `for_follower`-роль) и смотреть, кладёт ли он подделку в архив; (2) член комитета — обёртка `Sender` над `CERT_CHANNEL`-половиной `register(CERT_CHANNEL)` в `build_node`, подмена слота в исходящем сертификате; наблюдение: остальные отвергают (`seed_verify_*` метрики `metrics.rs:145-166`) и цепь идёт.
  - Дополнительно постановимы теперь: ловушка (4) (`> f` узлов с `AbsentBeacon` — ожидание по коду: нет dealer-кворума ⇒ `dkg_ceremony_fail`, цепь встаёт на 64 — принятый BFT-остаток), torn-journal рестарт (`JournalLoad::Torn`), рестарт до seal-дедлайна (`resume_from_journal` с реконструкцией dealer'а, `actor.rs:1669-1671`), выбытие узла с share на границе смены (demote-heal `drive_recompute`).

## 6. Оставлено как есть

- `Role::AbsentBeacon` = `beacon::absent` — верификатор с эпохи 0, не «подписант, не сдавший share» (§3, B3).
- `dkg_qual_at` = `Some(B256::ZERO)`, `geometry` = `Some((0, epoch_len))` немедленно — прод-шаг «geometry frozen by the plane EpochTransition» (`node/src/dpos.rs:1968-1971`) подменён константой.
- `Beacon` handles (`dkg_handle`, `resolver_handle`, …) не супервизируются в стенде: `Handle` drop = detach (`COMMONWARE_INTERNALS.md:409-411`, `CW:runtime/src/utils/handle.rs`), задачи живут до конца runner'а; паника внутри плоскости валит runtime (как и у всех задач стенда).
- В фазе 2 B4 предупреждение `result-final hash unresolved; keeping previous finalized cursor result_final=67` ×66 на узел (`executor.rs:3275`): [ГИПОТЕЗА] маршал при догоне из архива отдаёт tip (70) первым, `ordering_finalized` сразу 70, и каждый из последующих 66 блоков до дерайва 67 предупреждает; не входит в ассерты, не разбирал.
- `StandConfig::live` создаёт каталог в `std::env::temp_dir()/fluent-testbed-{pid}-{n}` и НЕ удаляет его после прогона (как `actor.rs:3224`); B4′ удаляет свой сам.
- Метрики фазы 2 (B4) считаются с нуля: `Runner::from(checkpoint)` даёт свежий реестр ([KNOWN] по наблюдению — `dkg_ceremony_ok = 0` в фазе 2 при 1 в фазе 1); не проверял по коду CW.
- `dkg_ceremony_ok == 0` в фазе 2 B4 — слабое свидетельство (actor стартует только `now+1`); настоящая улика перечитывания share — прогресс за 70 и B4′.
- Второй view агремента при n=4 (WARN `too few members confirm … view=1 covering=1`) — штатно, не разбирал, почему первый view не набрал подтверждений (10 мс латентности против первого тика).
- `Outcome.artifacts` собирается для эпох `0..=max_epoch+2` через `ArtifactSource` — то, что отдаёт `consensus_getEpochArtifact`, не сам `ArtifactStore`.
- Сессия 1 §4 фраза «константы передаются в `beacon::build` из прод-кода» — архив, не правил.
- Два независимых префикса агрегат-журналов `{prefix}dkg_epoch_{E}`: создаёт beacon-плоскость из `BeaconConfig.partition_prefix` (в стенде безусловно `node{i}-`), подметает epoch manager из `engine_partition_prefix` (в стенде `""` при `shared_engine_partitions: true`) — комбинация `Beacon::Live` + `shared_engine_partitions` нигде не используется, при ней уборка промахнётся молча (контр-ревью #9; то же, что §10 п.8 сессии 1). Не связывал.
- `dpos.rs:120-125` doc `ARTIFACT_JOURNAL_PARTITION` «Public because … opened … in the node crate» устарел: все три использования внутри `consensus` (`plane.rs:55,589,870`); `pub` лишний (контр-ревью #3, до этой сессии). Не трогал.
- Ничто в репозитории не закрепляет, что прод передаёт ПУСТОЙ префикс (`node/src/dpos.rs:1972`): тест литералов проверяет константы, не вызывающую сторону (контр-ревью #2). Тест на это жил бы в `crates/node` — вне объёма.
- `Outcome::metric` не матчит семейства с лейблами (`name{k="v"}`) и не различит counter `foo` от gauge `foo_total` (контр-ревью #12) — сейчас не срабатывает.
- `FakeChain::seed_at` схлопывает «не деривилось» и «деривилось без σ»; `trace_bytes` читает σ по высотам трассы (ordering-блоки из `hook_rx`), которые могут превышать executed-tip — там σ станет `0xFF` (контр-ревью #11); для B5 это детерминированно.
- `pin-project`, `fcu_pace`, §10 п.8/9 сессии 1 — как раньше.

## 7. Вне объёма

- Шаг 5 (фейк-стейкинг под реальным `EpochTransition`): должен взять на себя из стенда — `boundary_relay` целиком (включая новый `cold_start_epoch` — прод-`cold_start(hash, fin)` делает это сам), `Committees::Schedule` → снимки состояния, `dkg_qual_probe`/`committee_pair_for`/`committee_source` из `build_node` (сейчас три замыкания над `schedule`) → чтения `StakingStateRead` на хэше, `dkg_qual_at` → реальный финализированный хэш, `geometry` → `frozen_geometry()` после `cold_start`, `PeerSet::Committee` → `track(epoch, ..)` из снимков ET.
- Роль «подписант без share» для B3-варианта и `> f` отсутствующих dealer'ов (ловушка (4)).
- R-002/R-008 — точки в §5.
- Вынос общей сборки `dpos.rs::launch`/`build_beacon_plane`/`build_node` в generic-функцию (разведка §2 #7) — стенд теперь повторяет и сборку beacon.

## 8. Где проверка была самой слабой

1. B2 «узел 3 подтянул артефакт 3 через `pull_keys`» — вывод из того, что у не-члена артефакт есть; сам pull (`dkg_artifact_pull_ok`) не читал из метрик.
2. B4 «σ из журнала»: σ 64..70 в фазе 2 равны фазе 1 — но executor мог получить их и из сертификатов архива (spec-exec reporter `record_seed`, `spec_exec.rs:99`) при передериве; какой источник сработал — не разделял, и разделить равенством нельзя: σ уникальна по `(round, PK)` (`seed.rs:18-22`), «другая σ» на том же раунде честным путём недостижима (находка контр-ревью #17). Что журнал реплеился — INFO-лог `rehydrated the seed store from disk entries=7` (в throwaway-прогоне с capture на INFO). Негативный контроль есть только для share (B4′), для seed-журнала — нет.
3. B3: узел 3 «паркуется на `Missing`» — по коду и по высоте 63; сам лог парковки не смотрел.
4. Числа реального времени — один прогон каждого; B5 показывает разброс 4,39–4,73 с.
5. Ворота гонялись одним скриптом в фоне; `git status` после — только пять моих файлов вне `devnet/**`.
6. `Runner::from(checkpoint)` и свежий реестр метрик — по наблюдению, не по коду.

## 9. Handoff для шага 5

- Точки в `testbed/stand.rs::build_node`: `schedule`-замыкания `roster`/`committee_for`/`committee_pair_for`/`committee_source`/`dkg_qual_probe` (блок `(Beacon::Live, _)`), `boundary_relay` (cold-start arm с `cfg.cold_start_epoch`), `soft_enter`, `SnapshotReader` (`fakes.rs`) — всё, что читает `schedule`, заменяется чтением фейк-стейкинга на хэше.
- Наблюдения, которые уже есть и должны сохраниться: `Outcome.seeds/artifacts/metrics`, хелперы `pk_of`/`dealers_of`/`artifact_on_every_node`/`seed_agreed_at`/`seedless_on_every_node` в `tests.rs`.
- Прогон: `cargo test -p fluentbase-consensus --lib testbed -- --nocapture --test-threads=1` (~75 с реальных, из них live-тесты ~45 с); с эквивокатором — `--features dpos-devnet-byzantine`.
- Каталоги `fluent-testbed-*` в `/tmp` копятся — при желании чистить в `Stand::drop`.

## 10. Контр-ревью

Агент на Opus (свежий контекст, git read-only, без правок, без субагентов; ~12 мин, 81 вызов инструментов). Его отчёт — пересказ; ниже отмечено, что перепроверил сам.

Подтверждено агентом и мной: нейтральность `40fe8f3e` по всем четырём пунктам (единственный прод-вызов `beacon::build` — `node/src/dpos.rs:1943-1974`, `partition_prefix: String::new()` на `:1972` [KNOWN, открывал]; `journal_partition("", ..)` = константы [KNOWN, тест]; пустые ветки `key_journal.rs:238`/`artifact.rs:634` недостижимы при непустой базе [KNOWN]; заимствования `&partition_prefix` заканчиваются до move в `AgreementPlaneConfig` [KNOWN — иначе не скомпилировалось бы]); `FakeDeriver` записывает σ атомарно с хэшем — `land` и `insert` в одном вызове без `.await` между ними (#10, [KNOWN, `fakes.rs`]); `Outcome::metric` не путает `node1`/`node10` — точное сравнение (#12, [KNOWN]); `Checkpoint` не несёт реестр метрик — `CW:runtime/src/deterministic.rs:465-476` (агент; я — по наблюдению `dkg_ceremony_ok = 0` в фазе 2, код CW не открывал — [LIKELY]).

Таблица агента по задаче 2 (перепроверил по `tests.rs` до правок): B1/B2/B3/B4 — `PK_E` сравнивается байтами (`proposal.encode()`, плюс `pk_of(..).encode()` в B1), σ — между узлами по значению `Seed`, `verify_seed` — только на узле `nodes[0]` (остальные транзитивно); B4′ и B5 — ни `PK_E`, ни σ между узлами не сравнивают (B5 сравнивает трассу узла 0 между прогонами). Всё верно.

Находки и решения:

| # | Находка (кратко) | Перепроверил | Решение |
|---|---|---|---|
| 2 | тест литералов не закрепляет, что прод передаёт `""` | [KNOWN] `node/src/dpos.rs:1972` | отклонено как правка (тест жил бы в `crates/node`); в §6 |
| 3 | doc `ARTIFACT_JOURNAL_PARTITION` устарел, `pub` лишний | [KNOWN] grep: три использования в `consensus` | не трогал (до сессии); в §6 |
| 4 | `geometry` — константа, ветка «unfrozen» и задержка заморозки недостижимы | [KNOWN] `stand.rs`, `plane.rs:683-692` | принято как ограничение; §5 |
| 5 | расписание отвечает на любую будущую эпоху — гонка «комитет ещё не закоммичен» (`actor.rs:1231-1240`) невоспроизводима | [KNOWN] | принято; §5 (шаг 5) |
| 6 | `dkgQual` в B2 выведен из того же `roster`, что и решение о сделке — «honest» тавтологично по построению; реально проверяется «минт там, где бит, перенос там, где нет» | [KNOWN] `stand.rs` `dkg_qual_probe` | принято; в §5 и здесь: B2 доказывает согласованность ПЛОСКОСТИ с битом, не согласованность бита с контрактом |
| 7 | `dkg_qual_at = Some(ZERO)` вырезает путь `at_finalized() == None` (`carry.rs:228-241`) | [KNOWN] | принято; §5 |
| 8 | один фидер клока вместо трёх | [KNOWN] | было в §5 |
| 9 | два префикса `dkg_epoch_{E}` обязаны совпадать; `Beacon::Live + shared_engine_partitions` промахнётся | [KNOWN] `outer.rs`/`plane.rs:742` | принято; §6 |
| 11 | `seed_at` схлопывает `None`/`Some(None)`; `trace_bytes` выше tip даёт `0xFF` | [KNOWN] | принято; §6 |
| 12 | слепые зоны `metric()` (лейблы; counter vs gauge) | [KNOWN] | принято; §6 |
| 13 | `cold_start_epoch` — подстановка ответа, который прод вычисляет (`epoch_transition.rs:733-740`); floor-пиннинг не проверяется | [LIKELY] — `epoch_transition.rs` не открывал | принято как ограничение; §5/§7 (шаг 5) |
| 14 | `AbsentBeacon` — нода без beacon-модуля, не «dealer, который не раздал»; имя теста и коммит переоценивают модель | [KNOWN] `surface.rs:632-693`, `actor.rs` doc `drive_finalization` | принято: doc роли и теста переписаны, в §3 оговорка; имя теста оставлено (из задания) |
| 15 | `artifacts[3].is_empty()` вакуозен (стенд подставляет пустую карту) | [KNOWN] `stand.rs` | **принято, исправлено**: ассерт снят; попытка заменить на `metric(3, dkg_ceremony_ok) == None` упала — `beacon::absent` регистрирует те же семейства (`surface.rs:252`); оставлен комментарий, что наблюдение отсутствия — три лога в пине и парковка на 63 |
| 16 | `dkg_ceremony_ok == 0` в фазе 2 B4 гарантирован carry-forward (`actor.rs:1253,1646`), не рестартом | [KNOWN] | **принято, исправлено**: ассерт снят, значение печатается; doc переписан |
| 17 | «σ из seed-журнала» не изолирована от сертификатов архива; «другая σ» на том же раунде недостижима | [KNOWN] `seed.rs:18-22`, `spec_exec.rs:99` | принято: doc B4 и §8 переписаны; негативный контроль для seed-журнала не добавлял (нужна отдельная ручка на партицию) |
| 18 | doc B4′ обещает три проверки журналов, которых нет; `vec![70;4]` против `max` без lockstep | [KNOWN] | **принято, исправлено**: lockstep фазы 1, `vec![resume_from; 4]`, doc сужен |
| 19 | ни один live-тест не ассертит `errors().is_empty()` | [KNOWN] | **принято, исправлено**: добавлено в B1–B4 (B4′ — нет: парковка штатна); все зелёные ⇒ ERROR-строк в live-прогонах нет [KNOWN] |
| 20 | `digest()`-ассерт тавтологичен после `encode()` (`dkg_agree.rs:457-459`) | [KNOWN] | **принято, исправлено**: снят |
| 21 | негативная крипто-проверка эпохи 4 почти тавтологична после `assert_ne!` ключей | [KNOWN] | отклонено как правка: она проверяет саму σ, не ключи; оставлена |
| 22 | doc «byte-equal» при value-equal; `verify_seed` только на узле 0 | [KNOWN] | **принято, исправлено**: `verify_seed` на каждом узле, doc «value equality» |
| 23 | каталоги `fluent-testbed-*` не убираются | [KNOWN] | было в §6/§9 |
| 24 | B5: различие seed 1/2 не изолирует поле σ | [KNOWN] | **принято, исправлено**: `assert_ne!(σ₁(70), σ₂(70))` |

Принято 21 из 24 (#2, #21 отклонены как правки, #10 — подтверждение отсутствия проблемы); в коде исправлено 7 (#15, #16, #18, #19, #20, #22, #24) + doc `AbsentBeacon` (#14) — коммит `a860e588`; остальные принятые — оговорки в этом отчёте и в доке §15.a. Вердикт агента (пересказ): прод-часть нейтральна; ядро тестов настоящее (`PK_E` байтами между узлами через прод-кодек, σ по значению между узлами, настоящая `verify_seed`, пара B4/B4′ с негативным контролем), но три подмены — `dkgQual` из того же источника, что и решение о сделке; мгновенные `roster`/`geometry`; `AbsentBeacon` как нода без модуля — и falsifier-блоки шире ассертов. С последним согласен и сузил ассерты/доки; первые три — ограничения стенда до шага 5, записаны в §5.

