# Ревью строки 5.2, заход А — факт σ: `SeedIndex` единственный владелец

Ревьюер: Opus 5, свежий контекст. База `HEAD = 37691136` (`git rev-parse HEAD`
подтверждён). Агентов не запускал. `git` только на чтение. Единственный
записанный файл — этот. Пути без префикса — от `crates/dpos/consensus/src/`;
`node/` = `crates/node/src/`; `beacon/` = `crates/dpos/consensus/src/beacon/`.

`md5sum -c c1-tree.md5` — 15/15 `ЦЕЛ` ДО работы и 15/15 `ЦЕЛ` после (последняя
сверка в §4). Дерево под ревью не менялось.

---

## §0 Прямые ответы

### (1) Ворота; ограничения среды; счёт тестов

**ОГРАНИЧЕНИЕ СРЕДЫ, ПРЯМО** [KNOWN]: `sed -i` по файлу репозитория **запрещён
классификатором** этой сессии (попытка мутации `testbed/preconditions.rs:353`
отклонена: «Blocked by classifier»); плюс оркестратор ограничил запись одним
файлом. Поэтому **мутационный эксперимент по R-016 я не ставил**, и находка C-01
опирается на чтение кода + арифметику ворот, а не на красный прогон. `cargo test`
среда разрешает, и я им пользовался (ниже).

**Мой собственный прогон** [KNOWN] (`CARGO_BUILD_JOBS=6`):

~~~
cargo test -p fluentbase-consensus --lib -- beacon::seed_index:: beacon::follower::tests::
  dpos::replay_seed_tests:: cert_inlet::tests::a_forged_seed_under_an_oracle_less_verifier_is_refused_at_the_ingress
  cert_inlet::tests::a_late_refusal_costs_the_upstream_a_rotation_once_the_key_lands
test result: ok. 28 passed; 0 failed; 0 ignored; 0 measured; 631 filtered out; finished in 5.78s
~~~

**Ворота `c1` verbatim** (файлы оркестратора, `c1-status.txt` — все `exit=0`,
строка `DONE`):

| ворота | verbatim | база (`b6`, дерево 5.1 = `HEAD`) |
|---|---|---|
| `--lib` | `test result: ok. 659 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 172.65s` | 656 |
| стенд с фичей | `ok. 55 passed; 0 failed; … 613 filtered out; finished in 225.25s` | 55 / 610 filtered |
| стенд без фичи | `ok. 46 passed; 0 failed; … 613 filtered out; finished in 170.20s` | 46 / 610 filtered |
| node | `ok. 55 passed; 0 failed; … 0 filtered out; finished in 58.50s` | 55 |
| reader | `ok. 64 passed; 0 failed; …` + `ok. 0 passed; 0 failed; 1 ignored; …` | то же |
| `--test slasher_integration` | `ok. 16 passed; 0 failed; … finished in 0.02s` | 16 |
| clippy (3 крейта, `--all-targets`) | ровно два ЧУЖИХ: `this MutexGuard is held across an await point` (`fluentbase-staking-reader` lib test), `large size difference between variants` (`fluentbase-node` lib) — **побайтно тот же набор, что в `b6-clippy.txt`**; в `fluentbase-consensus` ноль | то же |
| clippy с фичей | только `Checking`/`Finished`, ноль предупреждений | то же |
| `fmt --check` | `exit=0` | то же |
| doc | 6 строк `unresolved link` | 6 |

**Расхождения с журналом:**

1. [KNOWN] Журнал §3(е) утверждает: «`-p fluentbase-slasher` в этом воркспейсе
   НЕТ … число "slasher 16/0" из задания к этому дереву не привязывается». Это
   **неверное прочтение ворот**: ворота — `cargo test -p fluentbase-consensus
   --test slasher_integration` (`gates/run.sh`), это интеграционный ТЕСТ, а не
   крейт; файл `crates/dpos/consensus/tests/slasher_integration.rs` существует, и
   ворота зелёные 16/0. Находка C-16.
2. [KNOWN] Журнал гонял `cargo clippy --workspace --all-targets`, ворота —
   `-p fluentbase-consensus -p fluentbase-node -p fluentbase-staking-reader`.
   Результат совпал; расхождение только в охвате команды.
3. Журнальные числа `--lib` 659, стенд 55/46 — совпадают с `c1` побайтно.

**Счёт тестов — пересчитал сам** [KNOWN], `grep -c '#\[test\]'` по ВСЕМ изменённым
файлам (`git diff --name-only -- crates` + удалённый `certify.rs` + untracked
`seed_index.rs`); файлы без изменения счёта опускаю, их ноль сверх этих:

| файл | `HEAD:` | дерево | Δ |
|---|---|---|---|
| `beacon/certify.rs` | 14 | — (удалён) | −14 |
| `beacon/seed_index.rs` | — (новый) | 13 | +13 |
| `beacon/follower.rs` | 7 | 8 | +1 |
| `cert_inlet.rs` | 24 | 26 | +2 |
| `dpos.rs` | 31 | 32 | +1 |
| **итого** | | | **+3** |

656 + 3 = 659 ✔. Сходится и со второй стороны: `stand` без фичи 46 + 613 filtered
= 659, на `b6` 46 + 610 = 656.

**Четыре снятых теста — по `HEAD:`-телу, что доказывал каждый и куда ушло
свойство:**

| снятый тест (`HEAD:beacon/certify.rs`) | что доказывал | судьба свойства |
|---|---|---|
| `a_round_waiter_is_woken_by_its_own_round_and_not_by_another` (`:749`) | пробуждение по-раундовой ожидалки приходит на СВОЙ раунд, а не на любой record | **ушло классом**: `waiters`/`wait_for` удалены вместе с by-round pull-ом (FLU-1204 уже снял его потребителя). Замены не нужно. ✔ |
| `within_one_epoch_the_quarantine_bound_keeps_the_terminal_round` (`:781`) | внутри ОДНОЙ эпохи счётная граница карантина роняет старейшее, новейшее (= терминал) держит | **переехало частично**: счётную границу единой карты пинует `record_seed_is_bounded_and_idempotent` (`seed_index.rs:478`), но она пинует её на `Verified`-записях. На `Pending` счётной границы не пинует НИ ОДИН тест. |
| `the_quarantine_bound_keeps_the_newest_rounds_across_epochs` (`:802`) | то же между эпохами: старая эпоха уходит, новая остаётся | **переехало** в `the_protection_ages_out_with_the_scheme_retention_window` только по смыслу «старая эпоха теряет защиту»; сам факт «карантин ограничен по счёту и роняет старейшее» не пинуется. |
| `quarantine_eviction_is_per_epoch` (`:714`) | `retain_quarantine_from(9)` ВЫЧИЩАЕТ карантин эпох ниже фронтира — «за краем ретенции ключ прийти уже не может, а держимая σ — это память, которую может растить пир» | **ОСТАЛОСЬ БЕЗ ТЕСТА, потому что осталось без кода.** Замены у `retain_quarantine_from` нет: в дереве `Pending` не вычищается по эпохам ничем, только общей счётной границей. Журнал (§0 п. 8, §3) утверждает, что «обе их половины теперь несёт общее правило вытеснения» — для этой половины это неверно. **Находка C-05.** |

Три новых — `a_checked_entry_wins_over_a_held_one_in_both_arrival_orders`
(`seed_index.rs:656`), `the_protection_ages_out_with_the_scheme_retention_window`
(`:788`), `a_rehydrated_terminal_survives_the_window_it_is_replayed_beside`
(`:845`). Все три краснеют от своего дефекта по построению (проверял тела).

### (2) Строка плана, обязательство за обязательством

`PLAN.md:114`, по пунктам:

| обязательство | вердикт |
|---|---|
| `SeedIndex` — RAM-индекс, синхронный `seed`, журнал только для рестарта, единственный владелец | **сделано** `seed_index.rs:89` (`entries/persist/events`), `seed` синхронный `:255`, `open` асинхронный `seed_journal.rs:356`. Но «единственный владелец» — с оговорками, см. §0(4). |
| ретенция `SEED_RETENTION` + исключение терминального раунда как ПРАВИЛО | **сделано иначе и слабее**: правило есть (`oldest_evictable` `:392`), но оно state-blind и бюджет общий — C-03, C-04 |
| `SEED_RETENTION` не менять | **сделано**, `seed_index.rs:68` = 4096 [KNOWN] |
| карантин как состояние `Pending` | **сделано** (`Entry::Pending` `:82`), **но без per-epoch вычистки** — C-05 |
| вердикт внутри `observe_certificate` (синхронный `Refused`) | **сделано**, три продакшн-сайта получили читателя |
| поздний `DataFault` через `faults()` (Д-3 = (в)) | **сделано наполовину**: потребитель есть только у `CertInlet` с `LiveBeacon`; у follower-а `faults()` = `None` и `refused` выбрасывается в `info!` — C-06 |
| дедуп ERROR-строки защёлкой на эпоху | **сделано** (`follower.rs:379`; на плоскости защёлка от 5.1, `surface.rs:2104`), но без метрики — C-11 |
| удалить промоутер-задачу | **сделано** (`HEAD:plane.rs:960-985` → `LiveBeacon::settle_pending` `surface.rs:2031`), цена — C-08 |
| удалить `waiters` | **сделано** |
| удалить terminal-карту как отдельную карту | **сделано** |
| удалить `on_invalid_seed` | **было сделано 5.1** [KNOWN]: `git grep on_invalid_seed -- crates` в дереве — **ноль попаданий**; на `HEAD` два доккомментных (`surface.rs:170`, `:226`), оба переписаны этим заходом. Остаточные упоминания только в `.claude/` (§0(15)). ✔ |
| удалить **seed-половину и сам файл `follower.rs`** | **НЕ СДЕЛАНО** — §0(3), **C-02** |
| crash-replay через `observe_certificate`, `Pending ⇒ Defer`, возобновление по `KeyAvailable` | **сделано** `dpos.rs:741`, `:708`, `:1001` |
| `hint_finalized` как выход из hold НЕ вводить | **соблюдено** [KNOWN]: `git grep hint_finalized -- crates` даёт ровно два довода — `epoch_manager.rs:1577` и `executor.rs:441`, оба предсуществуют и оба про marshal-подсказку, не про выход из σ-hold. Ничего не добавлено. ✔ |
| затем R-016 порог абсолютный | **сделано**, и это **BLOCKER** — C-01, C-21 |
| читатели на трёх сайтах `let _observed` | **сделано**, сайтов действительно три (см. §0(16) п.1) |
| снять `for_seeds` вместе с фикстурой executor-а | **сделано** (`surface.rs` HEAD `:985` удалён; `executor.rs:4266` `beacon_over`) |
| «ОСТАТОК: три транзиентные копии σ в executor (`:134,239,724`)» | **ровно три, не прибавилось** [KNOWN]: `git grep 'crate::beacon::Seed' -- executor.rs` даёт поля структур только на `:134` (`Notarized`), `:239` (`ParkedSpec`), `:724` (`Deferred`); остальные попадания — параметры функций и тестовые. ✔ |
| R-129 стенд-тест | не делался, отложен в заход В — соответствует постановке |

### (3) `follower.rs` НЕ УДАЛЁН — главный вопрос захода

**(а) Что фактически осталось в файле** (1166 строк против 1042 на `HEAD` — файл
ВЫРОС), по функциям:

| символ | file:line | σ или ключ |
|---|---|---|
| `ResolvedFollowerInputs` | `:71` | сборка |
| `FollowerInputs` (pub) | `:100` | сборка |
| `build_follower` (pub) | `:113` | сборка |
| `FollowerBeacon` | `:150` | сборка |
| `build_resolved` | `:161` | сборка (строит `SeedIndex::new()` `:203` — σ) |
| `changed_bit` (pub(super)) | `:232` | комитет |
| `run_fetcher` | `:253` | **смешанная**: KEY-арм (fetch) + σ-арм (`settle_pending` `:269` + публикация `KeyAvailable` `:273`) |
| `FollowerRandomness.artifacts` | `:300` | ключ |
| `FollowerRandomness.keys` | `:303` | ключ |
| `FollowerRandomness.seed_namespace` | `:306` | **σ** |
| `FollowerRandomness.want_tx` | `:307` | ключ |
| `FollowerRandomness.seeds: SeedIndex` | `:310` | **σ** |
| `FollowerRandomness.reported_refusal` | `:316` | **σ** |
| `settle_pending` | `:326` | **σ** |
| `events` | `:343` | **σ** (делегирует `seeds.events()`) |
| `record_seed` | `:350` | **σ** |
| `hold_seed` | `:368` | **σ** + ключевой want одной строкой `:370` |
| `first_seed_refusal` | `:379` | **σ** |
| `seed_for` | `:387` | **σ** |
| `share_probe` | `:397` | ключ |
| `signer_scheme` | `:401` | ключ |
| `oracle_for` | `:413` | ключ |
| `ensure_key` | `:428` | ключ |
| `artifact_bytes` | `:443` | ключ |

Восемь из десяти методов `Randomness` и три из семи полей — σ. **Seed-половина не
удалена, она переписана на новое имя.** Удалены только `promote_quarantined` и
`retain_from` (`HEAD:follower.rs:332-340`, `:367`).

**(б) Правда ли слияние требует смены типа оракула** — [KNOWN] **нет, не в том
смысле, в каком это подано.** Типы: follower — `KeyOnlyOracle`
(`beacon/oracle.rs:251`), плоскость — `BeaconOracle` (`oracle.rs` через
`LiveBeacon::oracle_at`, `surface.rs:2344`). Оба отдаются как
`Arc<dyn SeedOracle>` — тип НА ГРАНИЦЕ один. По телу: `BeaconOracle::verify_seed`
(`:203-228`) и `KeyOnlyOracle::verify_seed` (`:281-293`) — **побайтно одинаковые
тела**; а `sign_partial`/`verify_partial`/`recover` `BeaconOracle` проходят через
`with_material` (`:117-122`), который на ПУСТОМ `CeremonyStore` возвращает `None`,
то есть ровно негативы `KeyOnlyOracle`. Конструктор пустого стора уже есть
(`surface.rs:939` `keyless_ceremony`, `:974` `keyless_index`). Так что оракул —
**не** блокер слияния. Настоящие различия, которые слияние обязано разрешить:
`ensure_key` (у плоскости ступень `Thorough` через `acquire`, `surface.rs:2361`; у
follower-а `want_tx` в последовательный троттлящий фетчер `follower.rs:253`),
`share_probe`/`signer_scheme` (жёсткий `Withheld`), RAM-only сторы. Это реальная
работа, но аргумент «слияние меняет тип оракула» её преувеличивает.

**(в) Остаток, который НЕ ключевой и всё-таки остался** — да, весь σ-столбец
таблицы (а): собственный `SeedIndex`, `record_seed`/`hold_seed`/`seed_for`,
собственный `settle_pending` (ДУБЛИКАТ `LiveBeacon::settle_pending`
`surface.rs:2031` минус `report_fault`), собственная защёлка.

**(г) Вердикт: НЕВЫПОЛНЕННЫЙ ПУНКТ СТРОКИ, не законное отклонение.** Строка
называет два действия — «удалить seed-половину» и «удалить сам файл». Ни одно не
выполнено. Причина, которую приводит журнал (§5 п.1), относится к ВТОРОМУ и
переоценена (см. (б)); ПЕРВОЕ она не оправдывает вовсе — seed-половина осталась не
из-за типа оракула, а потому что у follower-а есть свой индекс σ. Дополнительно:
отклонение записано только в §5 «что осталось», а не отдельной строкой `5.2а-Д-n`
в §1, как требует форма постановки (`5.2-A-impl-1.md`, §1). **C-02, SERIOUS.**

### (4) `SeedIndex` — владелец единственный?

`git grep` по владельцам/носителям σ в дереве (`BlsSignature` в роли seed, `Seed`,
`seed()`, `terminal_seed`):

| место | file:line | роль |
|---|---|---|
| `SeedIndex.entries` | `seed_index.rs:90` | **ВЛАДЕЛЕЦ** (RAM) |
| `SeedJournal` партиция `beacon-seed-ordinal` | `seed_journal.rs`, `dpos.rs:181` | durable-зеркало, только для рестарта |
| `FollowerRandomness.seeds: SeedIndex` | `follower.rs:310` | **ВТОРОЙ ЭКЗЕМПЛЯР того же типа** на другом классе узла (не второй тип, но второй владелец в процессе) |
| `executor::Notarized.seed` | `executor.rs:134` | транзитная копия (разрешено до 6.1) |
| `executor::ParkedSpec.seed` | `:239` | транзитная копия |
| `executor::Deferred.seed` | `:724` | транзитная копия |
| `executor::SeedOutcome::Present` | `:765` | транзит по значению внутри одного вызова |
| marshal-архив сертификатов | CW `marshal` | σ едет внутри сертификата; не «хранилище σ» |
| `testbed`/фикстуры | `beacon::testing::SeedStore` алиас, `mod.rs:171` | тестовые |

**Фактических владельцев — один тип и один экземпляр на процесс** (валидатор
строит `LiveBeacon` с одним `SeedIndex`, follower — `FollowerRandomness` с одним).
Три транзитные копии в executor-е разрешены строкой. **Свойство «один владелец»
достигнуто.** [KNOWN]

**`seed_journal.rs` — «только для рестарта»:**
- продакшн-читателей в горячем пути **нет** [KNOWN]: единственные читатели —
  `replay_window` и `terminal_per_epoch`, оба зовутся ровно из `open`
  (`seed_journal.rs:353-380`), который зовётся ровно из `plane.rs:718-725` до
  спавна потребителей. `Beacon::seed` журнала не касается. ✔
- вторым источником правды запись не стала: писатель один
  (`spawn_writer` над `UnboundedReceiver`, отправитель создаётся ВНУТРИ
  `with_persistence` `seed_index.rs:141-155` и наружу не выдаётся), в журнал уходит
  только `Entry::Verified` (`admit` `:226-229`: `if !verified { return; }` СТОИТ
  до блока persist), и рехидрация идёт через тот же `admit`. При расхождении
  «журнал против индекса» индекс не спрашивает журнал вовсе — расхождение
  ненаблюдаемо в рантайме. ✔

### (5) Правило вытеснения против удалённой карты

**(а) Свойства старой terminal-карты (`HEAD:beacon/certify.rs`) — по одному:**

| свойство `HEAD` | якорь | несёт / не несёт / несёт ИНАЧЕ |
|---|---|---|
| `pin_terminal` пишет «старший раунд на эпоху» | `HEAD:certify.rs:376-388` | **несёт иначе**: `oldest_evictable` (`seed_index.rs:392-405`) вычисляет то же по ключам карты вместо отдельной записи |
| пин кормится ТОЛЬКО из `insert`, то есть только ПРОВЕРЕННОЙ σ | `HEAD:certify.rs:201-203` (`insert` → `pin_terminal`), `quarantine` `:265` пина не трогает | **НЕ НЕСЁТ.** Новое правило state-blind: `oldest_evictable` перебирает `entries.keys()` и не смотрит на `Entry`. `Pending` может занять защиту. **C-04** |
| пин в ОТДЕЛЬНОЙ карте ⇒ счётная граница `SEED_RETENTION` его не касается | `HEAD:certify.rs:220-224` (`while map.len() > SEED_RETENTION` — над `seeds`, не над `terminal`) | **несёт иначе**: защита теперь ВНУТРИ бюджета (`the_epochs_terminal_round_outlives_the_retention_window` это и пинует), что само по себе корректно |
| `terminal_at` отвечает ТОЛЬКО на запиненный раунд, соседу — отказ | `HEAD:certify.rs:400-407` | **не несёт**: `terminal_seed` теперь дефолт над `seed` (`surface.rs:143-145`), отвечает на любой удержанный раунд. Безопасно (вызывающий называет раунд из согласованных данных), но свойство исчезло без теста — **C-22** |
| `retain_terminal_from(oldest)` — окно по эпохам, приводимое снаружи | `HEAD:certify.rs:409-413`, приводы `HEAD:surface.rs:2414,2430` | **несёт иначе**: окно `top_epoch − SCHEME_RETENTION_EPOCHS` считается ВНУТРИ (`evict` `:363-365`) |
| `retain_quarantine_from(oldest)` — per-epoch вычистка карантина | `HEAD:certify.rs:354-359` | **НЕ НЕСЁТ, замены нет.** **C-05** |
| карантин — отдельная карта с собственным бюджетом 4096 | `HEAD:certify.rs:80`, `:288-289` | **НЕ НЕСЁТ**: бюджет общий, `Pending` вытесняет `Verified`. **C-03** |

**(б) Раунд, на который старая карта отвечала, а новый индекс молчит.**
`epoch_manager.rs:163` (`boundary_base` → `Beacon::terminal_seed`) получает
терминал `E−1` теперь ИЗ ТОГО ЖЕ индекса, через дефолтную реализацию
`surface.rs:143`. Молчания по конструкции нет — наоборот, новый индекс отвечает
ШИРЕ (см. (а), строка `terminal_at`). Но появляются два случая, где он молчит там,
где карта отвечала: (i) защиту занял `Pending` и настоящий терминал вытеснен
(C-04); (ii) бюджет выел `Verified`-терминал раньше, чем эпоха ушла из окна
(C-03 — правда, терминал защищён, поэтому здесь риск ниже). Исход при «индекс
держит не тот раунд»: `seed(round)` → `None` → `BoundaryLookup::Missing` →
спавн движка ОТКЛАДЫВАЕТСЯ (`epoch_manager.rs:167`). Это безопасный, но
**молчаливый и бессрочный** исход — узел не подписывает эпоху. Неверную σ он не
отдаёт: индекс отвечает ровно на названный раунд.

**(в) Может ли правило защитить НЕ ТОТ раунд** — да, и последствие серьёзнее, чем
признаёт док `seed_index.rs:379-391`. Док обосновывает случай «терминал на раунд
ниже настоящего после жёсткого падения» и верно говорит, что НАЗЫВАТЬ терминал
правилу никто не даёт. Но он не разбирает случай, когда защиту занимает запись
СВЕРХУ настоящего терминала — а именно это и делает `Pending` при state-blind
переборе: защищается мусор, вытесняется настоящий `Verified`-терминал, и
`boundary_base` уходит в бессрочный `Missing`. На `HEAD` это было невозможно
структурно (пин кормился только из `insert`). **C-04.**

**(г) `floor = top_epoch − SCHEME_RETENTION_EPOCHS` от старшей эпохи В ИНДЕКСЕ —
приоритетный вопрос.** [KNOWN] Разобрал и **регрессией против `HEAD` это НЕ
считаю**, вопреки первому впечатлению:
- `HEAD` считал пол из двух источников: `observe_epoch(_, entered_frontier)`
  (`HEAD:surface.rs:2414`) и `observe_cert(epoch)` (`HEAD:surface.rs:2429`), где
  `epoch` — эпоха только что ЧИСТО принятого сертификата (`cert_inlet.rs:758`
  стоит ПОСЛЕ `return`-ов, в ветке «Verified: a clean ingest»). Действующим был
  более высокий пол, то есть фактически σ-фронтир — то же, что `top_epoch`.
- Поднять `top_epoch` «эпохой далеко впереди» без кворума нельзя: обе двери зовут
  `observe_certificate` только ПОСЛЕ проверки мультиподписи под `committee[E]`
  (`cert_inlet.rs:623-712` до `:733`; `:2920` под `handler.deliver(..) == true`),
  а crash-replay — по сертификату из собственного архива/верифицированного
  refetch. Форджённая эпоха из будущего требует > f.
- Остаточная разница: на `HEAD` `observe_cert` вызывался только на ЧИСТОМ ingest-е
  live-двери, а `Pending` — это тоже чистый ingest, так что и там пол двигала σ,
  чей ключ не проверен. Паритет.

Вывод по (г): **пол не стал хуже**; настоящие потери — (C-03) общий бюджет,
(C-04) state-blind защита, (C-05) исчезнувшая per-epoch вычистка.

### (6) Д-3 = (в), три сайта вердикта

| сайт | `Refused` | `Pending` | цена |
|---|---|---|---|
| `spec_exec.rs:92` | `warn!` + `return`, спекуляция пропускается (`:116-125`) | то же | — (спекуляция best-effort) |
| `cert_inlet.rs:731` | `warn!` + `record_data_fault().await` + `return` (в marshal не уходит, не тиируется) `:733-745` | не фолт, σ удержана | ротация на 3-й |
| `cert_inlet.rs:2922` (`UpstreamResolver`, by-height) | только `warn!` | не фолт | **ноль** |

**(а) Соответствует ли третий сайт Д-3 — это ДЫРА, и худшая из трёх.** [KNOWN]
Ратифицированный текст Д-3 (`DECISIONS.md:25`) требует «вердикт синхронно внутри
`observe_certificate` (`Refused`), поздний `DataFault` через `faults()`». На
by-height двери синхронный вердикт читается, но не имеет последствия; а поздний
`DataFault` до неё не относится (канал общий и его снимает `CertInlet`). При этом
именно эта дверь — ЕДИНСТВЕННАЯ, где форджённая σ вообще проходит: проверяющий
live-инлета несёт оракул (`committee::epoch_verifier` `committee/mod.rs:539-551`
передаёт `beacon.oracle_for(record.epoch)` в `build_verifier`), а
`plane_upstream::verifier_for` (`plane_upstream.rs:371-389`) строит
`build_verifier(.., None)` — «`oracle = None` is the whole point». Хуже: к моменту
вердикта `handler.deliver(key, value).await` уже вернул `true`, то есть сертификат
**уже положен в marshal-архив** и будет раздаваться дальше — ровно то отравление
архива, ради которого Д-3 выбирали. **C-09, MODERATE→SERIOUS по последствию.**

**(б) Есть ли у этой двери ДРУГАЯ цена** — нет: ни исключения пира, ни метрики
(`SeedCheck::Invalid` умышленно не считается, `oracle.rs:214-217`), ни отказа
`deliver` (он уже прошёл). Должно было быть как минимум одно из: (i) отзыв записи
из архива / пометка её на перезапрос — прямой текст варианта (в) в
`DECISIONS.md:24` («при `Invalid` запись помечается и перезапрашивается»), которого
в коде нет; (ii) счётчик `dpos_seed_refused_total` с меткой двери; (iii) проброс
`RotateUpstream` в `UpstreamResolver::new` — но оба конструктора (`crate::outer`,
`node/dpos.rs`) вне списка файлов, и это честно названо в коде и в §5 журнала.

**(в) `Pending` на by-height двери: поднимает ли want.** [KNOWN] Зависит от класса:
- follower: **да** — `FollowerRandomness::hold_seed` (`follower.rs:368-372`) делает
  `want_tx.try_send(epoch)`. Это СТРОГО ШИРЕ `HEAD` (там want ехал на
  `observe_cert`, у которой единственный сайт — live-инлет), и заявление журнала
  здесь верное.
- валидатор: **нет** — `LiveBeacon::hold_seed` (`surface.rs:2098-2100`) только
  кладёт в индекс. Но это не регрессия: на `HEAD` by-height дверь тоже ничего не
  поднимала, а ключ валидатору приносят `ensure_key(Local)` в ingest-е
  (`cert_inlet.rs:623`) и `ensure_key(Thorough)` в `epoch_manager.rs:1950-1953`.
  σ не «зависает без просителя».

### (7) Поздний `DataFault`

**(а) «Потребитель ровно один» — структурно, да** [KNOWN]:
`git grep '\.faults()' -- crates` → ровно два попадания: `cert_inlet.rs:472` и
делегирующий враппер `testbed/byzantine_roles.rs:413`. Механизм —
`faults_rx: Mutex<Option<Receiver>>` + `.take()` (`surface.rs:2385`): второй
вызывающий получает `None`. Если приёмник не взят — `faults_armed` остаётся
`false` и `report_fault` (`:2007-2011`) выходит до `send`, то есть сообщения даже
не кладутся. Корректно. **Оговорка**: на follower-е `faults()` вообще не
переопределён и берёт дефолт `None` (`surface.rs:603`), поэтому `self.faults =
None` и дренаж — no-op; а `FollowerRandomness::settle_pending`
(`follower.rs:326-336`) `refused` вообще не передаёт никуда, только `info!`. **C-06.**

**(б) Дренаж на чужом ребре.** Журнал называет это безвредным («ротировать не от
кого»). **Не полностью**: сценарий, где ротация нужна ИМЕННО при остановленном
потоке, существует и он ровно тот, ради которого канал есть. Апстрим, который
отдаёт форджённые σ, обычно НЕ перестаёт слать сертификаты (поток здоровый — это
прямо записано в доке `faults` `cert_inlet.rs:389-390`), так что здесь журнал прав.
Но есть второй: апстрим отдал форджённое окно по BY-HEIGHT двери (gap-repair) и
live-поток при этом стоит (узел догоняет через resolver, а не через WS). Тогда
поздние фолты копятся в небуферизованно-растущем канале и не отрабатываются, пока
live-поток не оживёт. Практически — MINOR; называю как непокрытый случай, а не как
безвредный. Плюс C-14 (рост канала).

**(в) Порядок заряда и сброса — заряд МОЖЕТ быть стёрт тем же сертификатом**
[KNOWN]. `drain_late_verdicts().await` стоит в ГОЛОВЕ `ingest` (`cert_inlet.rs:557`),
а `self.consecutive_faults = 0` — на чистом ingest-е (`:750`). Синхронный фолт
делает `return` ДО `:750` и потому переживает; поздний — нет. Значит: `DataFault`
с `refused < MAX_UPSTREAM_FAULTS`, приехавший на ЧИСТОМ сертификате, стирается
этим же сертификатом **всегда**. Апстрим, подделывающий 1-2 раунда на эпоху и в
остальном чистый, не платит НИЧЕГО никогда. Док (`:823-831`) описывает это как
свойство серии, и это честно — но асимметрия «синхронный переживает, поздний нет»
в доке не названа, а она и есть механизм. **C-07.**

**(г) Переполнение канала `faults`.** Канал `UnboundedSender/Receiver`
(`surface.rs:1988`) — переполниться не может, теряется ничего. Взамен он растёт
без границы, если `ingest` не вызывается (см. (б)); каждая запись — 16 байт,
источник — один `settle_pending` на ребро артефакта, так что это не утечка
масштаба, а неограниченность без сигнала. Заметности нет: ни метрики глубины, ни
warn при росте. **C-14, MINOR.**

### (8) Автомат `admit` (`seed_index.rs:187-244`)

| свойство трёх карт на `HEAD` | несёт ли `admit` |
|---|---|
| I1 — две разные ПРОВЕРЕННЫЕ σ на раунд ⇒ первая остаётся + `error!` | **да**, `:197-203`, тождественно `HEAD:certify.rs:209-217`. Пинуется `a_second_differing_seed_for_one_round_is_refused` |
| «проверенное не перезаписывается непроверенным» | **да**, `:207` (`(Some(Verified(_)), false) => return`). На `HEAD` это несло РАЗДЕЛЕНИЕ КАРТ (структурно); теперь — одна ветка `match` (по аргументу). Пинуется `a_checked_entry_wins_over_a_held_one_in_both_arrival_orders` |
| «`Pending ⇒ Pending` last-wins» | **да**, `:208-212` (падает в `_ => {}` и перезаписывает). Тождественно `HEAD:certify.rs:270` (`map.insert`) — C-15, пред-существует |
| «отказанная удаляется, а не удерживается» | **да**, `settle_epoch` `:317-325`, и с корректной защитой «удалить только если это всё ещё тот самый `Pending`» |
| `Pending ⇒ Verified` (промоушен) | **да**, `_ => {}` + `fresh = !matches!(previous, Some(Verified(_)))` `:216` ⇒ промоушен считается свежим и пишется в журнал. Корректно |

**`persist`** — уходит `(Round, BlsSignature)` в `UnboundedSender`, созданный
внутри `with_persistence`. **`Pending` уйти не может** [KNOWN]: `admit` делает
`if !verified { return; }` на `:227-231` ДО блока persist `:239-243`. Плюс
рехидрация зовёт `admit(.., true, false)` — `persist=false`, обратной записи нет
(пинуется `rehydrated_entries_are_not_written_back_to_the_journal`).

**Порядок «промотировали ⇒ записали» против «записали ⇒ промотировали»** — в коде
первый: RAM-вставка `:218`, затем `evict`, затем broadcast `:236`, затем
`tx.send` `:241`. Падение между ними теряет ХВОСТ журнала, а не создаёт
непроверенную запись на диске: на рестарте раунд просто отсутствует, узел
перезапросит. Это и есть причина, по которой правило вытеснения обязано говорить
«старший УДЕРЖАННЫЙ», а не «терминальный» — док `:373-391` это признаёт
корректно. Обратного порядка («на диске есть, в RAM нет») достичь нельзя.

### (9) `settle_pending` синхронно в мосте (`plane.rs:973-977`)

**(а) Снимает ли гонку — да, и гонка была наблюдаема.** [KNOWN] На `HEAD` было ДВЕ
независимых подписки на один и тот же `ArtifactStore::subscribe()`:
`promoter_edge` (`HEAD:plane.rs:802`) и `bridge_key_edge` (`:803`). Оба
пробуждались одним insert-ом, в произвольном порядке. Потребитель, разбуженный
`BeaconEvent::KeyAvailable` из моста, перечитывает состояние запросом — и мог
прочитать индекс до того, как промоутер добежал до `promote_epoch`, увидев MISS на
σ, которая через миллисекунду стала бы servable. Наблюдаемость: `executor`
подписан на тот же broadcast (`executor.rs:1350`) и его ветка `KeyAvailable` →
`seed()` → `None` → высота остаётся HELD до следующего `SeedRecorded`. Теперь
`settle` — первое действие ветки, публикация — после. Гонка закрыта.

**(б) Цена.** Верхняя граница BLS-проверок под ОДНИМ `KeyAvailable` — число
`Pending`-раундов во всех эпохах, чей оракул отдался, то есть до `SEED_RETENTION`
= 4096 пороговых верификаций (`settle_epoch` `:305-316`). На это время мост
**не публикует ничего**: ни `KeyAvailable` (он после `settle`), ни
`ParticipationChanged` (второй арм того же `select!`, `plane.rs:978`). На `HEAD`
`KeyAvailable` публиковался НЕМЕДЛЕННО, а промоутер работал на своей задаче.
То есть журнальная формула «суммарная работа не изменилась, изменилось, какая
задача блокируется» верна, но недосказана: задержался и сам `KeyAvailable`, ради
которого потребители и просыпаются. **C-08.** Плюс: `settle_epoch` зовёт
`self.record` на каждый промоушен, каждый шлёт `BeaconEvent::SeedRecorded` в
broadcast глубиной `EVENT_BUFFER = 64` (`surface.rs:333`) — всплеск гарантированно
даёт `Lagged` каждому потребителю. Это по контракту допустимо («события —
пробуждения, не факты»), но означает, что всплеск промоушенов превращается в одно
пробуждение, а не в N.

**(в) Дёшев ли `NoKey` — [KNOWN] ДА, проверено по коду оракула, не по доку.**
`BeaconOracle::verify_seed` (`oracle.rs:218-227`) и `KeyOnlyOracle::verify_seed`
(`:281-292`) оба начинаются с `match self.keys.key_at(self.epoch)` и уходят в
`None => SeedCheck::NoKey` ДО `beacon::verify_seed`, то есть до спаривания.
Заявление журнала верно. Оговорка: каждый такой промах инкрементит
`seed_verify_no_key` — на `HEAD` промоутер делал ровно то же, счётчик не изменил
смысла.

**(г) Может ли `settle_pending` запаниковать или заблокироваться на отравленном
мьютексе — нет.** [KNOWN] `pending_epochs` (`:261-263`) и `settle_epoch`
(`:284-287`) оба через `let Ok(..) else`, отравление даёт пустой ответ + `warn!`.
Вложенных захватов одного мьютекса нет: `settle_epoch` отпускает лок перед BLS и
`self.record` берёт его заново. `oracle_for` → `mandatory_at` + `oracle_at`, паник
нет. `report_fault` — атомик + `send`. **Мост утащить нельзя.** Остаточный риск —
не паника, а то, что длинный СИНХРОННЫЙ CPU-всплеск сидит внутри async-задачи и
занимает воркер рантайма (в детерминированном рантайме — единственный).

### (10) crash-replay (E5-03)

**(а) Обе ветки прежней функции сохранены дословно** — [KNOWN] да. `HEAD:dpos.rs:438`
`crash_recover_defer_or_fatal`: ветка `!has_upstream` осталась на месте
(`dpos.rs:438-452`), остальное тело целиком переехало в новый
`crash_recover_defer` (`:466-487`) без изменений (`best_block_number` → `gap` →
`warn!` → три метрики → `Ok(RecoverOutcome::DeferToElSync { gap })`). Дифф это
показывает буквально: одна добавленная строка вызова и перенос без правок.

**(б) `Absent` на ОТКАЗАННОЙ σ и `Absent` на «сертификат чужого раунда» — один
исход на два факта.** Да, `dpos.rs:757` схлопывает `Refused | Inactive` и
`:745`/`:761` — «чужой раунд» / «нет σ». Последствие: после ДОКАЗАННОЙ лжи из
локального архива обход идёт к upstream-у (`:819-846`) и, если тот отдаёт
годную σ, узел её берёт. **Я считаю это корректным, а не дырой**: локальная
запись, не прошедшая аттестованный ключ, — это порча/подмена СВОЕГО архива, и
единственный разумный ответ — искать в другом месте; узел не «берёт σ из другого
источника после лжи ПИРА», он чинит свой архив. Настоящая цена схлопывания — в
наблюдаемости: `warn!` на `:840-844` называет оба факта одной строкой («absent, or
refused under the epoch key»), а отдельной ERROR-строки/метрики у отказа на
replay-пути нет (`certificate_verdict`'s `Invalid` печатает под защёлкой эпохи, и
на replay-е это ОДНА строка на эпоху). MINOR, включил в C-11.

**(в) Может ли `Pending ⇒ Defer` стать вечным — нет** [KNOWN]. `Defer` уходит в
`crash_recover_defer` → `RecoverOutcome::DeferToElSync { gap }` (`:487`), то есть
узел отдаёт догон devp2p EL-sync-у, а не ждёт ключа. Это изменение живучести
против `HEAD` (там был слепой derive из архива), и оно ровно то, что предписывает
проект §5.4 («Рестарт без ключа эпохи»). Эпоха — та, чей сертификат в архиве;
выход — EL-sync + последующий `KeyAvailable`.

**(г) Пин раунда сохранён и означает то же** — [KNOWN] да: `seed_via_beacon`
`:745-747` (`if finalization.proposal.round != round { return Absent }`) —
тождественно `HEAD:seed_from_cert:684-686`; плюс четвёртая ветка нового теста
(`the_replays_certificate_seed_is_checked_under_the_epoch_key`) это пинует.

### (11) R-016

**(а) Появился ли у hold собственный выход — ДА, но он закрывает НЕ ВСЮ работу,
которую нёс re-jump.** [KNOWN] Выход есть: `Pending` в индексе → `settle_pending`
на ребре `ArtifactStore::subscribe()` → `record` → `BeaconEvent::SeedRecorded` →
арм `executor.rs:1689-1700` перезапускает eager-derive. Тащит его ребро артефакта
(`plane.rs:973`, у follower-а `follower.rs:269`). Если артефакт не придёт никогда
(> f молчат, либо узел отрезан), hold вечен — и это принятый BFT-остаток,
проект §5.4 («Локального артефакта нет ⇒ Acquiring без предела»).

**(б) Какие ещё пути опирались на re-jump как на выход** — `git grep
re_jump|ReJump|JUMP_THRESHOLD -- crates`, продакшн-сайты:
1. `executor::maybe_re_jump` (`executor.rs:2516-2589`), гейт `:2532`:
   `height − ordering_finalized <= threshold ⇒ return`. Это **не только σ-hold**:
   он же — выход для узла, чья ИСПОЛНИТЕЛЬНАЯ сторона встала (отрезанная
   консенсус-плоскость, отставший EL), у которого marshal-tip заморожен на
   двухэпохальном потолке. Этот путь предусловие строки не покрывает вообще.
2. `ReJump::probe` (`executor.rs:1042`, `dpos.rs:1227`) — фронтир-проба, порогом не
   гейтится, не затронута.
3. `cold_start_jump::jump_to_target` — получает `jump_threshold` параметром из тех
   же двух сайтов (`dpos.rs:2533`, `:3293`), то есть изменился синхронно.
4. `node/cert_follow/mod.rs:167` — `JUMP_THRESHOLD` как размер окна, не порог.

**(в) Devnet: какой сценарий ломается и есть ли кейс, который это поймает —
ЕСТЬ, и он сломается.** [KNOWN] `devnet/local-dpos-smoke/dpos_harness/cases/smoke/
verdicts_boundary.py:255-272` (`boundary_gap`) прямо говорит: «the gap must sit
ABOVE `min(1024, interval)` and BELOW 1024. At interval 64 that band is (64,
1024)», и берёт `vo.deep_gap(64) = 2*64 + 32 = 160`
(`verdicts_onchain.py:1114-1117`). Кейс `rejump_signer.py` (`smoke-rejump-signer`,
`EPOCH_BLOCK_INTERVAL=64`) существует РОВНО для того, чтобы steady-state re-jump
сработал и потом проверить пост-джамповое boundary-seeding. С порогом 1024
разрыв 160 < 1024 ⇒ `maybe_re_jump` не арминится ⇒ кейс перестаёт проверять то,
ради чего написан. Плюс `asserts_onchain.py:627` и `verdicts_boundary.py:261`
письменно фиксируют формулу `min(JUMP_THRESHOLD=1024, epochBlockInterval)` как
предмет утверждения.

Хуже кейса — общий вывод, который уже измерен в репозитории:
`testbed/preconditions.rs:304-315` содержит МЕРЕНЫЙ прогон и вывод дословно:
«A gate at or above `2·interval` therefore wedges an execution-stalled node
PERMANENTLY after §5.2. Production cannot configure one: it computes
`JUMP_THRESHOLD.min(interval)`, which is `≤ interval < 2·interval` for every
interval». После этой правки продакшн **именно такой порог и конфигурирует** для
любого `interval ≤ 512`. Арифметика подтверждается и из кода: достижимый разрыв
ограничен двухэпохальным потолком (`epoch_manager.rs:837-840`: «`live ≤ epoch(fin)
+ 3` on a node whose execution has stalled (4.2 Б2's two-epoch ceiling)»), то есть
`tip − fin` порядка `2-3·interval`. **C-01, BLOCKER.**

В проде (эпохи ≫ 1024) изменение — no-op: `min(1024, interval) = 1024`. То есть
правка ничего не даёт проду и снимает выход на всех коротких интервалах.

**(г) Стендовые тесты задают порог сами — и именно поэтому потеряли покрытие**
[KNOWN]. Все сайты ставят СТАРУЮ продакшн-формулу:
`testbed/tests.rs:1385`, `:1618`, `:2616`, `:3492`, `:3709`;
`testbed/preconditions.rs:162`, `:353`; `testbed/committee_tests.rs:201`, `:619` —
все `Some(JUMP_THRESHOLD.min(EPOCH_LEN))`. Новая продакшн-формула не проверяется
**ни одним** тестом. Заявление журнала §0(7) «продакшн-правка их чисел не двигает»
верно буквально и вводит в заблуждение по сути: это не отсутствие регрессии, это
отсутствие покрытия — и тест `preconditions.rs:353`, чей доккоммент измерил именно
этот клин, остаётся ЗЕЛЁНЫМ при сломанном свойстве. **C-21**, и это же делает
C-01 блокером по рубрике («тест, зелёный при сломанном свойстве»).

### (12) Дедуп ERROR-защёлки

[KNOWN] `follower.rs:379-385`: `reported_refusal: Mutex<BTreeSet<u64>>`
(`:316`), `seen.insert(epoch)` — защёлка **на эпоху**, не глобальная. Первую
строку ДРУГОЙ эпохи не съедает (ключ — эпоха). Отравление лока → `true`, то есть
не глушит («A poisoned latch must not silence a real witness») — правильная
сторона отказа. Пинуется `the_refusal_line_is_latched_once_per_epoch`
(`follower.rs:1082`), тест самопроверяет подмену (`assert_ne!` до вердикта) и
проверяет соседнюю эпоху.

**Не сбрасывается никогда и не прунится** — множество растёт на одну `u64` за
эпоху за жизнь процесса. Практически безразлично; называю для полноты.

**Метрика — НЕ полная, и это и есть цена** [KNOWN]. Счётчика у синхронного
`Refused` нет вовсе: `beacon/metrics.rs:161-170` знает только `seed_verify_ok` и
`seed_verify_no_key`, а `oracle.rs:213-217` прямым текстом объясняет, что
`Invalid` умышленно не считается — «уже имеет громкую атрибутируемую обработку на
каждом сайте». На плоскости эта обработка есть (фолт + ротация). На follower-е
её нет (C-06), и до 5.2 единственным свидетельством была строка в секунду; теперь
одна строка на эпоху и ноль счётчиков. **C-11.**

### (13) Пустые дефолты `observe_epoch`/`observe_cert` (отклонение 5.2а-Д-1)

**`observe_epoch` — аргумент ДЕРЖИТСЯ** [KNOWN]: единственный вызывающий
`epoch_manager.rs:1225`, файл в списке ЗАПРЕЩЁННЫХ (`5.2-A-impl-1.md` п.3).
Удалить метод = править запрещённый файл. Обе реализации удалены, нога удалена.

**`observe_cert` — аргумент ДЕРЖИТСЯ ЧАСТИЧНО, пункт выполнен не до конца.**
Держатель, на которого ссылается журнал, — `testbed/byzantine_roles.rs:404`; этот
файла **действительно нет** в разрешённом списке (`testbed/{tests,
cert_inlet_tests, committee_tests, preconditions, stand, fakes}.rs`), так что
снять метод с трейта нельзя, и это честно. **Но продакшн-ВЫЗОВ
`cert_inlet.rs:758` (`self.randomness.observe_cert(epoch);`) стоит в РАЗРЕШЁННОМ
файле и остался.** Его удаление трейта не касается и было доступно.

**Вызов пустого тела в продакшне — это мёртвый код**, причём с активно
вводящими в заблуждение соседями: комментарий прямо над ним (`cert_inlet.rs:752-757`)
объясняет несуществующую ретенцию, `cert_inlet.rs:2865-2869` и `dpos.rs:3735-3738`
(оба ПИСАБЕЛЬНЫЕ файлы) строят на нём аргументы — «`observe_cert` prunes what
`ensure_key` reads», «`observe_cert` is also the key-delivery TRIGGER». Оба
утверждения теперь ложны. **C-10.**

### (14) Граница и гигиена

**`pub use` beacon/mod.rs — имя за именем** [KNOWN], `HEAD` против дерева:

| тир | `HEAD` | дерево | Δ |
|---|---|---|---|
| `pub use follower::{build_follower, ArtifactFetch, FollowerInputs}` | есть | есть | — |
| `pub use plane::{build, CommitteeReads, Tasks, ValidatorInputs}` | есть | есть | — |
| `pub use seed::{constant_fallback_seed, prev_randao_from_seed, witness_fallback_seed, Seed}` | есть | есть | — |
| `pub use surface::{Beacon, BeaconEvent, DataFault, Observed, ObservedCertificate, PinEffort, ShareProbe, SignerVerdict, WithheldReason}` | есть | **идентично** | — |
| `pub(crate) use dkg_engine::agreement_partition`, `surface::absent_unregistered` | есть | есть | — |
| `testing::` `certify::SeedStore` | `:163` | — | **удалено** |
| `testing::` `seed_index::SeedIndex as SeedStore` | — | `:171` | **добавлен АЛИАС** (док `:165-170`, держатели названы: `epoch_manager.rs:2076,2080,2129,2836,2848` — запрещённый файл; проверил, все пять существуют) |
| `testing::` `for_seeds` | `:167-169` | — | **удалено** ✔ |
| `testing::` `keyless_index` | — | `:173-175` | **добавлено** — новое имя на тестовой границе, в доке mod.rs не объяснено (NIT, C-18) |

Крейт-внешняя граница (`pub use`) **не изменилась ни на одно имя**. Новых `pub`
нет. `#[allow]` — счёт не изменился ни в одном затронутом файле (`plane.rs` 1,
`dpos.rs` 5, `seed_index.rs`/`surface.rs`/`follower.rs`/`cert_inlet.rs`/
`spec_exec.rs` — 0). `todo!`/`unimplemented!` — ноль. Таймеров вместо событий,
поллинга — не добавлено (settle едет на ребре `Notify`, не на тике). Новые
`unwrap`/`expect` — все в `#[cfg(test)]` (проверил `git diff -U0 | grep '^+'`,
14 попаданий, все тестовые); на продакшн-пути `seed_index.rs` локи берутся через
`let Ok(..) else`, `unwrap` нет вовсе.

**`executor.rs` — заявлен как тронутый только в `#[cfg(test)]`.** Семантически
продакшн-поведение **не изменено** [KNOWN]: правки вне тестового модуля — только
доккомментарии (`:752`, `:1689`, `:2479`, `:2984`, `:9068`, `:9475`), переименование
`SeedStore`→`SeedIndex` в прозе. Фикстура `beacon_over` (`:4266`) строит
`LiveBeacon` с `keyless_index()`/пустым ceremony/`ArtifactStore::new()` — то же,
что делал удалённый `for_seeds`, то есть подмены поведения через фикстуру нет.
Формально буква «ТОЛЬКО `#[cfg(test)]`-модуль» нарушена комментариями — NIT (C-18).

**`lib.rs` — заявлен как «только доккоммент константы»** [KNOWN]: сверил, дифф —
одна строка в доке `SCHEME_RETENTION_EPOCHS` (`lib.rs:26`). ✔

### (15) Дрейф доков

**В `crates` (`git grep`)** — места, называющие удалённые/переименованные символы.
Помечаю (П) = файл был ПИСАБЕЛЬНЫМ для захода, (З) = запрещённым.

| file:line | что называет |
|---|---|
| `beacon/actor.rs:1410` (П) | «`beacon::certify`, which σ-verifies…» — O-7 подтверждён |
| `engine.rs:44` (З) | «(`beacon::certify` records why)» — O-7 подтверждён |
| `beacon/log_resolver.rs:129` (П) | «`SeedStore::insert` writes» — и тип, и метод удалены |
| `beacon/surface.rs:615` (П) | «its own `SeedStore` since the follower seed store landed» |
| `beacon/surface.rs:1975` (П) | «gives the promoter its late-verdict sink» |
| `beacon/surface.rs:2087-2088` (П) | «`SeedStore::record` … and the promoter records through it too» |
| `beacon/artifact.rs:2246` (П) | «(one `observe_epoch` per epoch entered)» |
| `beacon/follower.rs:212,215,257,267,276` (П) | переменная/параметр `promoter` — задачи с таким именем больше нет |
| `cert_inlet.rs:752-757` (П) | «the beacon's key store keeps its own, below» + вызов `observe_cert` |
| `cert_inlet.rs:2865-2869` (П) | «Only ONE of the two doors prunes what it files: `observe_cert`, which carries the retention window» — ложно |
| `cert_inlet.rs:2916-2917` (П) | «QUARANTINE is keyed on a round the RESPONDER chose» |
| `dpos.rs:2477-2478` (П) | «Rule Y: … same epoch-relative threshold» — прямо НАД абсолютным порогом |
| `dpos.rs:3252-3254` (П) | «Epoch-relative re-jump gate … `min(serving-window, 1 epoch)`» — O-5 подтверждён, стоит прямо над `:3255-3257` |
| `dpos.rs:3735-3738` (П) | «`observe_cert` prunes what `ensure_key` reads» + «is also the key-delivery TRIGGER» — оба ложны |
| `executor.rs:663-671` (З, продакшн-тело) | `ReJump::threshold`: «`min(JUMP_THRESHOLD, epoch_block_interval)` … Only the tests construct a bare `JUMP_THRESHOLD`» — теперь **ровно наоборот** |
| `executor.rs:9475` (З) | «covered by certify.rs's …» |
| `executor.rs:5222` (З) | «`SeedStore::record`» (через алиас — терпимо) |
| `cold_start_jump.rs:774` (З) | «`jump_threshold` is `min(JUMP_THRESHOLD, epoch_block_interval)` on both» |
| `order_block.rs:249` (З) | «resolved from the local `SeedStore`» |
| `epoch_manager.rs:517` (З) | «`Beacon::observe_epoch` and a participation probe, on every block» |
| `epoch_manager.rs:853` (З) | «`observe_epoch` re-attempts the previous epoch's key backfill» |
| `epoch_manager.rs:2281` (З) | «leave quarantine» — журнал это назвал |
| `epoch_manager.rs:2835` (З) | «reads the production `SeedStore` pin» — пина нет |
| `testbed/stand.rs:156` (П) | «production's own gate, `JUMP_THRESHOLD.min(epoch_block_interval)`» |
| `testbed/stand.rs:354` (П) | «(`observe_certificate` and `observe_cert`)» — O-6 подтверждён |
| `testbed/stand.rs:2673` (П) | «`observe_cert` pruning a store nobody reads» — O-6 подтверждён |
| `testbed/tests.rs:1343` (П) | «is production's own, `min(JUMP_THRESHOLD, interval)`, and that is load-bearing» — **ложно и load-bearing** |
| `testbed/tests.rs:1591` (П) | «`JUMP_THRESHOLD.min(epoch_block_interval)`, `consensus/src/dpos.rs:2466`» |
| `testbed/cert_inlet_tests.rs:99` (П) | «after `observe_certificate` + `observe_cert`» |
| `testbed/preconditions.rs:304,314` (П) | «`min(JUMP_THRESHOLD, interval)` (`consensus/dpos.rs`, both launch paths)» + «Production cannot configure one» — **теперь может и конфигурирует** |
| `node/cert_inlet.rs:50` (З) | «this inlet's `observe_cert` prune a store nothing else reads» — O-6 |
| `node/dpos.rs:784` (З) | «`observe_cert` prunes what the key ladder reads» — O-6 |
| `node/dpos.rs:931` (З) | список супервизируемых детей называет «the quarantine promoter» — задачи нет |

**В `.claude/dpos_architecture/` (`grep -rn`, `git grep` там пуст по gitignore) —
отдельной таблицей, вход Ф7. Править НЕ надо.**

| file:line | что называет |
|---|---|
| `00_preamble.md:39` | «used to quarantine against an unattested key; retention is ONE window» |
| `00_preamble.md:666-668` | «`SeedStore`/`BeaconKeys` … (`beacon/certify.rs:237`, `beacon/keys.rs:272-280`)» |
| `00_preamble.md:704` | сигнатура `certificate_verdict(cert, oracle_for, record, quarantine, on_invalid)` |
| `00_preamble.md:707` | «`crate::beacon::certify::SeedStore` no longer compiles» |
| `00_preamble.md:711,713` | `SeedStore`, `for_seeds` в списках достижимого |
| `00_preamble.md:747,749,751` | `quarantine_seed`, `on_invalid_seed`, `SeedStore::notifier`, `for_seeds` |
| `00_preamble.md:757-758` | «`broadcast::Sender<BeaconEvent>` owned by `SeedStore` … (the quarantine promoter included)» |
| `00_preamble.md:1061-1062` | «the follower quarantines it … `promote_epoch` refuses» |
| `00_preamble.md:1470` | «resolves σ from its OWN `SeedStore`» |
| `00_preamble.md:1494-1496` | «`SeedStore::pin_terminal`/`terminal_at`, `retain_terminal_from`), exempt from `SEED_RETENTION`» |
| `00_preamble.md:1540,1544,1547` | «`record_seed`/`quarantine_seed` empty … RAM-only `SeedStore`; the quarantine's promote trigger» |
| `00_preamble.md:1558` | «`observe_cert` has ONE call site (`cert_inlet.rs:885`…)» |
| `00_preamble.md:1576-1577` | «`SeedStore::terminal_at` + `SeedJournal::terminal_per_epoch`» |
| `00_preamble.md:1585-1586,1589-1590` | `SeedStore::{quarantine, promote_epoch, retain_quarantine_from, quarantined_epochs}`, `with_persistence(rehydrated)` (арность изменилась ещё до 5.2) |
| `00_preamble.md:1646` | «`certify.rs` и σ quarantine question are untouched» |
| `00_preamble.md:2181` | `SeedStore` |
| `01_system_map.md:29` | «resolves σ from its own `SeedStore`» |
| `02_ordering…:42,148,153,162,266,282-283,299,308,310,520,788,801` | `SeedStore` ×5, `certify.rs::pin_terminal` ×2, `keys.rs::on_invalid_seed`, «quarantines» |
| `03_epoch_machinery.md:119,257,280,998-999` | `observe_epoch` ×2, «two consumers are the quarantined-σ promoter», `SeedStore::insert` |
| `08_node_integration…:104,112,812-813,826,838,878,881,933,950,1005-1012,1035,1039,1453,2115-2121,2135-2158` | самый плотный узел дрейфа: весь §8.11.2 описывает `certify.rs` как «now the `SeedStore`», отдельную карантинную карту, `pin_terminal`/`terminal_at`, `observe_epoch/observe_cert` как TRANSITIONAL, `for_seeds`, «quarantine promoter» в списке супервизируемых |
| `09_followers.md:40-42,56-58,104-105,120,126,147,161-169,178-181,196-204,247,252-256,276-292,494` | ~30 упоминаний: `SeedStore` как тип follower-а, `promote_epoch`, `observe_cert`'s `try_send` («the edge that must not be deleted» — удалена), `FollowerRandomness::promote_quarantined`, `retain_from` + `retain_quarantine_from` + `retain_terminal_from`, `for_seeds` |
| `12_consensus_critical_constants.md:98` | «`SEED_RETENTION` … `beacon/certify.rs:40` — the `SeedStore`'s served-map window» |
| `13_invariants_gotchas_rules.md:349,577,611-613,620-654,683-687,695,706-708,720` | правило «`NoKey` идёт в ОТДЕЛЬНУЮ карантинную карту — не флаг на общей» (`:623`) теперь **прямо противоречит коду**; `seed_promoter`, `retain_terminal_from`, `pin_terminal`, `on_invalid_seed` |
| `15_smoke_cases…:212,406` | верхнеуровнево, имя теста живо — правки минимальны |

Особо: `13_invariants_gotchas_rules.md:623` («a SEPARATE quarantine map — not a flag
on the shared one») — это ИНВАРИАНТ, записанный как правило, и заход его снял.
Ф7 должен не переименовать его, а решить.

### (16) Где журнал вводит в заблуждение

**§0(10), три заявленных опровержения чужих якорей — проверил каждое сам:**

1. «Сайтов вердикта в продакшне ТРИ, а не два» — **ПОДТВЕРЖДАЮ** [KNOWN].
   `git show HEAD:…/cert_inlet.rs | grep -n '^#\[cfg(test)\]'` → `:778`, тестовый
   модуль `mod tests {` `:779`, закрывающая скобка в нулевой колонке `:2400`.
   Сайт `HEAD:2641` лежит в `UpstreamResolver::spawn_finalized` — вне модуля,
   продакшн. Якорь оркестратора О был неверен.
2. «Плечо стенда НЕ обязано менять причину» — **ПОДТВЕРЖДАЮ** [KNOWN].
   `committee::epoch_verifier` (`committee/mod.rs:539-551`) передаёт
   `beacon.oracle_for(record.epoch)` в `build_verifier`, поэтому подменённая σ при
   держащемся ключе падает в `verify` и до `observe_certificate` не доходит.
   Причина осталась «BLS verify FAILED», числа не изменились.
3. «Удаление `observe_epoch`/`observe_cert` С ТРЕЙТА невозможно внутри списка» —
   **ПОДТВЕРЖДАЮ ДЛЯ ТРЕЙТА, ОПРОВЕРГАЮ ФОРМУЛИРОВКУ «сделано максимум
   возможного»**: продакшн-вызов `cert_inlet.rs:758` и три доккомментария в
   ПИСАБЕЛЬНЫХ файлах (`cert_inlet.rs:2865`, `dpos.rs:3735`, `stand.rs:354/:2673`,
   `cert_inlet_tests.rs:99`) были доступны и остались. См. §0(13), C-10.

**Остальные пункты §0 и §1 журнала:**

| пункт | вердикт |
|---|---|
| §0(1) форма индекса и единственный писатель | **подтверждаю** |
| §0(2) «опасность исчезла вместе с пином: индекс ничего не НАЗЫВАЕТ» | **вводит в заблуждение**: исчезла ОДНА опасность (пин на раунд ниже), появилась другая (защита на раунд ВЫШЕ, занятая `Pending`) — C-04 |
| §0(4) потребитель `faults()` — «никто больше в процессе `faults()` не зовёт» | **подтверждаю** структурно; не сказано, что на follower-е потребителя нет вовсе — C-06 |
| §0(5) «`wc -l beacon/` 30104 → 30090» | не проверял построчно; продакшн-половины сверил выборочно, порядок верен |
| §0(5) «want-нога СТРОГО ШИРЕ прежнего» | **подтверждаю** (для follower-а) |
| §0(6) crash-replay | **подтверждаю** целиком |
| §0(7) «Предусловие строки выполнено пунктами 1-2» | **ОПРОВЕРГАЮ**: предусловие сформулировано про σ-hold, а re-jump нёс вторую работу (execution-stalled узел) — C-01 |
| §0(7) «стендовые тесты порог задают сами, поэтому правка их чисел не двигает» | **вводит в заблуждение**: это не «нет регрессии», это «нет покрытия» — C-21 |
| §0(8) «три теста границы карантина … обе их половины теперь несёт общее правило вытеснения» | **ОПРОВЕРГАЮ** для `quarantine_eviction_is_per_epoch` — C-05 |
| §0(11) «запрещённого сделано ноль» | **подтверждаю по существу**; буква «только `#[cfg(test)]` executor.rs» нарушена доккомментариями — C-18 |
| §0(12) п.3 «начнёт терять `Pending`-записи раньше» | **недосказано в опасную сторону**: главная цена общего бюджета — `Pending` вытесняет `Verified`, а не наоборот — C-03 |
| §1 5.2а-Д-1 | **подтверждаю для `observe_epoch`; для `observe_cert` — не до конца** (C-10) |
| §1 5.2а-Д-2 (расщепление `crash_recover_defer_or_fatal`) | **подтверждаю**, обе ветки дословны |
| §1 5.2а-Д-3 (приёмник в `with_randomness`) | **подтверждаю** |
| §1 5.2а-Д-4 (`spec_exec` отказывается спекулировать) | **подтверждаю** по существу; не названо, что `return` пропускает и `try_drain_parked` — C-12 |
| §3(е) «`-p fluentbase-slasher` в воркспейсе нет» | **ОПРОВЕРГАЮ** — C-16 |
| §5 «σ-половина `follower.rs` удалена целиком» | **ОПРОВЕРГАЮ** — C-02/C-17 |

### (17) Слабые места, названные исполнителем, + четвёртое

1. **Follower без девнет-прогона** — согласен, но серьёзность выше заявленной:
   на этом классе изменились ТРИ вещи сразу (want переехал, появилась защёлка,
   исчезли две карты) и ДВЕ из них сделали follower-а хуже наблюдаемым
   (C-06, C-11). **MODERATE→SERIOUS для приёмки**: `make smoke-cert-follow`
   обязателен перед закрытием строки.
2. **Дренаж на чужом ребре** — **MINOR**, но не «безвреден»: есть непокрытый
   случай by-height-форджа при стоящем live-потоке (§0(7)(б)). Настоящая проблема
   рядом — не ребро, а порядок заряда и сброса (C-07), который исполнитель
   называет в §4, но не в §0(12).
3. **Общий бюджет `SEED_RETENTION`** — **SERIOUS**, и по другой причине, чем
   назвал исполнитель: опасно не «Pending теряется раньше», а «Pending вытесняет
   Verified» (C-03) и «Pending занимает терминальную защиту» (C-04).
4. **ЧЕТВЁРТОЕ, не названное: исчезла per-epoch вычистка `Pending`** — у
   `retain_quarantine_from` нет замены ни в коде, ни в тесте (C-05). Это то самое
   свойство, которое снятый тест `quarantine_eviction_is_per_epoch` и пинует, и
   которое журнал считает перенесённым.
   *(Пятое, если считать: у R-016 нет ни одного теста на новую формулу — C-21.)*

### (18) Hard-stop оркестратора

**(2) Требуется ли менять П-2, Д-3 или Д-9 — НЕТ, но Д-3 реализован не полностью.**
- **Д-9** (`DECISIONS.md:56`): «beacon-owned `SeedIndex` + intake: RAM-индекс с
  единственным писателем, журнал только для рестарта; `hint_finalized` не
  вводить» — **выполнено** (§0(1), §0(4)). Менять не нужно.
- **П-2** (`DECISIONS.md:85`) вытеснен Д-9 («ни П-2, ни B-1 дословно»); отклонений
  от него, требующих правки, нет.
- **Д-3** (`:25`): синхронный `Refused` + поздний `DataFault` + карантин как
  `Pending` — реализовано на двух классах из трёх дверей и одном классе узла из
  двух. Пробелы (C-06, C-09) — это НЕДОДЕЛКИ реализации, а не противоречие
  решению; решение менять не надо, надо доделать или явно записать остаток.
  Отдельно: фраза варианта (в) «при `Invalid` запись помечается и
  перезапрашивается» в ратифицированном статусе не воспроизведена, и в коде её
  нет — если владелец считает её частью решения, это hard-stop; если статус
  (`:25`) исчерпывающий — нет. **Вопрос владельцу, не мой вердикт.**

**(3) Есть ли BLOCKER, на который проект не отвечает — ДА, один: C-01.**
Проект (`E5-BEACON-DESIGN.md` §7 строка 5.2, `PLAN.md:114`) предписывает «затем
R-016 порог абсолютный» с предусловием «только после того, как σ-hold перестанет
зависеть от re-jump». Предусловие про σ-hold выполнено. Но проект нигде не
разбирает ВТОРУЮ работу re-jump-а — выход для узла с остановленным ИСПОЛНЕНИЕМ на
двухэпохальном потолке — и потерю этого выхода при `2·interval ≤ 1024` не
предписывал. Измеренный вывод лежит в самом репозитории
(`testbed/preconditions.rs:304-315`) и говорит «wedges an execution-stalled node
PERMANENTLY».

**(4) Опроверг ли стенд проект на центральном пути — нет.** Стенд зелёный
(55/46), и центральный путь (владелец σ, вердикт, crash-replay) он подтверждает.
Опровержение пришло не от прогона, а от ЧТЕНИЯ стендовой фикстуры: она пинует
старую продакшн-формулу порога, поэтому проверить новую физически не может (C-21).

### (19) Где моя проверка была слабее всего

1. **R-016 (C-01) не подтверждён красным прогоном.** Мутация
   `testbed/preconditions.rs:353` → `Some(JUMP_THRESHOLD)` — ровно тот
   эксперимент, который превратил бы вывод в [KNOWN] по прогону; `sed -i` по
   репозиторию запрещён классификатором, а оркестратор ограничил запись одним
   файлом. Вывод построен на чтении: `dpos.rs:2494/:3257` + гейт
   `executor.rs:2532` + `boundary_gap` девнета + мереный абзац
   `preconditions.rs:304-315`. **Одна команда закрывает это:**
   `cfg.re_jump_threshold = Some(crate::cold_start_jump::JUMP_THRESHOLD);` в
   `preconditions.rs:353`, затем
   `cargo test -p fluentbase-consensus --lib testbed::preconditions::a_node_more_than_two_epochs_behind`.
2. **Достижимость C-04** (`Pending` ВЫШЕ верифицированного терминала эпохи) —
   механизм [KNOWN], достижимость [ГИПОТЕЗА]: нужен сертификат/нотаризация эпохи
   `E−1` на вью выше терминального блока, принятый при отсутствующем ключе.
   Кандидат — нотаризация вью, которая потом нуллифицировалась (`spec_exec.rs:92`
   → `speculation=true` → `NoKey` → `hold`). Я не прошёл commonware до конца, чтобы
   доказать, что такая нотаризация доезжает после терминального блока эпохи.
3. **Стендовые и девнет-сценарии я не гонял** — ни `testbed::`, ни
   `make smoke-*`. Все стендовые числа — из ворот оркестратора, не мои.
4. **Follower целиком проверен только чтением и юнитами.** Ни один мой прогон не
   касался `cert-follow` в сборке.
5. **`wc -l`/размерные утверждения журнала** я выборочно не пересчитывал.

---

## §1 Находки

| id | серьёзность | file:lines дерева | HEAD-якорь | что не так | чем пытался опровергнуть | уверенность |
|---|---|---|---|---|---|---|
| C-01 | **BLOCKER** | `dpos.rs:2494`, `:3257`; гейт `executor.rs:2532` | `HEAD:dpos.rs:2399`, `HEAD:dpos.rs:3160` (`JUMP_THRESHOLD.min(interval)`) | Абсолютный порог 1024 снимает выход re-jump-а не только у σ-hold, но и у узла с ОСТАНОВЛЕННЫМ ИСПОЛНЕНИЕМ, чей marshal-tip заморожен двухэпохальным потолком: достижимый разрыв `tip−fin ≈ 2-3·interval` (`epoch_manager.rs:837-840`), поэтому при `interval ≤ ~341` гейт не арминится никогда. `testbed/preconditions.rs:304-315` содержит МЕРЕНЫЙ вывод «a gate at or above `2·interval` wedges an execution-stalled node PERMANENTLY after §5.2. Production cannot configure one» — после правки продакшн именно такой и конфигурирует. Предусловие строки (σ-hold получил свой выход) покрывает только одну из двух работ re-jump-а | Искал второй выход для execution-stalled узла: `ReJump::probe` порогом не гейтится, но она только КОРМИТ marshal, а не двигает `ordering_finalized`; `git grep re_jump\|ReJump\|JUMP_THRESHOLD` — других продакшн-потребителей порога нет. Проверял, что в проде это no-op (эпохи ≫ 1024 ⇒ `min` = 1024) — да, потеря только на коротких интервалах. Мутационный прогон поставить не смог (см. §0(19) п.1) | высокая по механизму; красным прогоном не подтверждено |
| C-02 | SERIOUS | `beacon/follower.rs` целиком (1166 стр.), σ-часть: `:203`, `:269`, `:306`, `:310`, `:316`, `:326-336`, `:343-393` | `HEAD:beacon/follower.rs` (1042 стр.), `PLAN.md:114` «удалить … seed-половину и сам файл `follower.rs`» | Ни seed-половина, ни файл не удалены; файл ВЫРОС на 124 строки. Удалены только `promote_quarantined` и `retain_from`. Восемь из десяти методов `Randomness` и три из семи полей — σ. Отклонение записано только в §5 журнала, а не строкой `5.2а-Д-n` в §1, как требует форма | Проверял главный аргумент журнала (слияние меняет тип оракула): `BeaconOracle::verify_seed` и `KeyOnlyOracle::verify_seed` (`oracle.rs:218-227` / `:281-292`) — одинаковые тела, а `with_material` (`:117-122`) на пустом `CeremonyStore` даёт те же негативы; пустые конструкторы уже есть (`surface.rs:939`, `:974`). Аргумент не пустой (`ensure_key`, want-канал, `share_probe`), но слабее заявленного | высокая |
| C-03 | SERIOUS | `seed_index.rs:68` (один `SEED_RETENTION`), `:220-226` (`evict` над обоими состояниями) | `HEAD:certify.rs:76-84` (три карты), `:220-224` (бюджет served), `:288-289` (бюджет quarantine) | Один бюджет на два состояния: `Pending` вытесняет `Verified`. Вытеснение идёт oldest-first по раунду, поэтому у узла, бесключевого для эпохи `E`, растущий хвост `Pending` эпохи `E` выметает `Verified` эпохи `E−1`, которые его executor ещё не исполнил ⇒ бессрочный hold. На `HEAD` это невозможно структурно: карты и бюджеты раздельны | Проверял, не спасает ли что-то: `evict` состояние не различает (`:227` вызывается безусловно, `oldest_evictable` смотрит только ключи); ни один тест не пинует счётную границу на `Pending`. Проверял, редок ли случай: §5.4 проекта сам называет «Локального артефакта нет ⇒ все σ эпохи `Pending`, `Acquiring` без предела» штатным | высокая |
| C-04 | SERIOUS | `seed_index.rs:392-405` (`oldest_evictable`), `:355-371` (`evict`) | `HEAD:certify.rs:376-388` (`pin_terminal`, кормится ТОЛЬКО из `insert` `:201`, т.е. только `VerifiedSeed`) | Терминальная защита теперь STATE-BLIND: перебираются `entries.keys()`, `Entry` не смотрится. `Pending` на вью выше настоящего терминала эпохи `E−1` получает защиту, а `Verified`-терминал вытесняется ⇒ `boundary_base` (`epoch_manager.rs:163`) → `Missing` → спавн движка эпохи `E` отложен бессрочно, молча. На `HEAD` карта пина не могла содержать непроверенное значение | Проверял, не спасает ли `seed()`: нет — он вернёт `None` на настоящий терминал. Проверял, отдаст ли индекс НЕВЕРНУЮ σ: нет, отвечает только на названный раунд (безопасность цела, потеряна живучесть). Док `:379-391` разбирает противоположный случай («пин на раунд НИЖЕ») и этот не покрывает | механизм высокая, достижимость [ГИПОТЕЗА] (см. §0(19) п.2) |
| C-05 | SERIOUS | `seed_index.rs` — замены нет ни в одной функции | `HEAD:certify.rs:354-359` `retain_quarantine_from`, приводы `HEAD:surface.rs:2422-2432`; снятый тест `HEAD:certify.rs:714` | Per-epoch вычистка `Pending` исчезла БЕЗ ЗАМЕНЫ. `Pending`-раунды эпохи, чей ключ прийти уже не может, не удаляются ничем, кроме общей счётной границы, и до тех пор едят бюджет (⇒ C-03). Свойство снятого теста («за краем ретенции держимая σ — это память, которую может растить пир») не несёт ни один живой тест; журнал §0(8) утверждает обратное | Проверял, не несёт ли его `the_protection_ages_out_with_the_scheme_retention_window`: нет — он про ТЕРМИНАЛЬНУЮ защиту, а не про вычистку `Pending`; `record_seed_is_bounded_and_idempotent` — про `Verified`. Проверял `settle_pending`: он `Pending` не удаляет, только промотирует или дропает ОТКАЗАННЫЕ | высокая |
| C-06 | SERIOUS | `beacon/follower.rs:326-336` (`refused` уходит в `info!`), отсутствие `fn faults` в `follower.rs` ⇒ дефолт `surface.rs:603` = `None` | `HEAD:plane.rs:982` (`report_fault` у промоутера — только плоскость; у follower-а его и не было) | На классе `--cert-follow`, чей собственный док (`follower.rs:359-363`) называет `Pending` ОРДИНАРНЫМ состоянием, поздняя половина Д-3 отсутствует целиком: `faults()` = `None` ⇒ `CertInlet::with_randomness` ставит `self.faults = None` ⇒ дренаж no-op; а `FollowerRandomness::settle_pending` `refused` никуда не передаёт. Лгущий апстрим follower-у не платит ничем, кроме одной ERROR-строки на раунд | Искал другой рычаг у follower-а: `run_fetcher` ротации не имеет, `TransportAcquire` отказ σ не видит; `git grep '\.faults()'` — два попадания, оба не follower | высокая |
| C-07 | MODERATE | `cert_inlet.rs:557` (дренаж в ГОЛОВЕ `ingest`) против `:750` (`consecutive_faults = 0` на чистом ingest) | `HEAD:cert_inlet.rs` — дренажа не было вовсе | Поздний заряд асимметричен синхронному: синхронный фолт делает `return` ДО сброса (`:744`) и переживает, поздний — нет. `DataFault` с `refused < MAX_UPSTREAM_FAULTS`, приехавший на ЧИСТОМ сертификате, стирается этим же сертификатом ВСЕГДА. Апстрим, подделывающий 1-2 раунда на эпоху и в остальном чистый, не платит никогда | Проверял, ловит ли это тест: `a_late_refusal_costs_the_upstream_a_rotation_once_the_key_lands` подаёт `refused = 3 ≥ MAX_UPSTREAM_FAULTS` одним сообщением, то есть ровно тот случай, где сброс не достигается. Док `:823-831` описывает поведение, но не называет асимметрию | высокая |
| C-08 | MODERATE | `beacon/plane.rs:973-977` (`settle.settle_pending()` в KEY-арме, до публикации); `surface.rs:2031-2054`; у follower-а `follower.rs:269` | `HEAD:plane.rs:947-985` (отдельная задача `seed_promoter`), `HEAD:plane.rs:1013` (`KeyAvailable` публиковался немедленно) | До `SEED_RETENTION` = 4096 пороговых BLS-проверок исполняются СИНХРОННО внутри арма `select!` моста. На это время мост не публикует ни `KeyAvailable` (он после), ни `ParticipationChanged` (второй арм). На `HEAD` `KeyAvailable` уходил сразу. Гонку это действительно закрывает (§0(9)(а)), но цена — задержка самого пробуждения, а не только «другая задача блокируется», как сказано в журнале §4 | Проверял дешевизну `NoKey`-ветки — подтвердилась по коду оракула, так что платят только ПРОМОТИРУЕМЫЕ раунды; но их и может быть 4096. Проверял возможность deadlock/panic — нет (§0(9)(г)) | высокая |
| C-09 | MODERATE | `cert_inlet.rs:2920-2940` (`warn!` без рычага); `plane_upstream.rs:371-389` (`build_verifier(.., None)`) | `HEAD:cert_inlet.rs:2640-2643` (`let _observed`, вердикт не читался) | By-height дверь — ЕДИНСТВЕННАЯ, чей проверяющий строится без оракула, то есть единственная, где форджённая σ вообще проходит (у live-инлета `committee::epoch_verifier` `committee/mod.rs:539-551` оракул несёт). И именно у неё цена нулевая. Хуже: `handler.deliver(..)` вернул `true` ДО вердикта, т.е. сертификат уже в marshal-архиве и будет раздан — ровно отравление архива, ради которого выбирали Д-3; текст варианта (в) (`DECISIONS.md:24`) требует «запись помечается и перезапрашивается», чего нет | Проверял, не ловит ли marshal σ сам: ловит только если его схема несёт оракул; на этом пути `verifier_for` его явно не даёт («`oracle = None` is the whole point»). Проверял, доступен ли рычаг в рамках списка файлов: `UpstreamResolver::new` зовут `crate::outer` и `node/dpos.rs` — оба вне списка, ограничение реально | высокая |
| C-10 | MODERATE | `cert_inlet.rs:758` (вызов пустого дефолта), доки `cert_inlet.rs:752-757`, `:2865-2869`, `dpos.rs:3735-3738`, `testbed/stand.rs:354`, `:2673`, `testbed/cert_inlet_tests.rs:99` — ВСЕ в писабельных файлах | `HEAD:surface.rs:2429-2434` (`observe_cert` двигал `retain_quarantine_from`/`retain_terminal_from`) | Отклонение 5.2а-Д-1 оправдывает сохранение МЕТОДА на трейте (держатель `byzantine_roles.rs:404` вне списка) — это верно. Но продакшн-ВЫЗОВ и шесть доккомментариев, строящих аргументы на несуществующей ретенции, стоят в РАЗРЕШЁННЫХ файлах. `dpos.rs:3735-3738` утверждает две ложные вещи подряд: «`observe_cert` prunes what `ensure_key` reads» и «is also the key-delivery TRIGGER» | Проверил список разрешённых файлов в `5.2-A-impl-1.md` п.3: `cert_inlet.rs`, `consensus/dpos.rs`, `testbed/{tests,cert_inlet_tests,stand}` — все писабельны; `byzantine_roles.rs` и `epoch_manager.rs` — нет. Разделение подтвердилось | высокая |
| C-11 | MODERATE | `beacon/follower.rs:379-385` (новая защёлка); `beacon/metrics.rs:161-170`; `oracle.rs:213-217` | `HEAD:beacon/follower.rs:372-374` (`fn first_seed_refusal(_) -> bool { true }` — строка на КАЖДЫЙ отказ) | У синхронного `Refused` нет счётчика вообще (`Invalid` умышленно не считается, комментарий `oracle.rs:213-217` обосновывает это «громкой атрибутируемой обработкой на каждом сайте»). На плоскости такая обработка есть (фолт+ротация); на follower-е её нет (C-06) — и заход туда добавил ещё и защёлку. Итог для follower-а: одна строка на эпоху и ноль метрик, тогда как до 5.2 было по строке на отказ. Условие «счёт остаётся полным, глушится только строка» НЕ выполнено | Искал компенсирующий счётчик: `git grep 'seed_verify\|no_key_total\|seed_invalid'` — только `seed_verify_ok` и `seed_verify_no_key`. Проверил, что per-round ERROR из `settle_epoch` (`seed_index.rs:314`) НЕ защёлкнута — да, и стенд-тест на неё опирается; то есть поздняя половина наблюдаема, синхронная — нет | высокая |
| C-12 | MINOR | `spec_exec.rs:116-125` (`return` на `Refused`/`Pending`) | `HEAD:spec_exec.rs:92-101` (сообщение отправлялось всегда) | `return` пропускает не только спекуляцию, но и `Command::SpecNotarized`, чей обработчик после `spec_execute` зовёт `try_drain_parked` (`executor.rs:2476`). Запаркованная не-по-порядку нотаризация не дренируется до первого НЕ-`Pending` раунда | Проверил, есть ли у `try_drain_parked` другой привод: есть (`repoke_deferred` на доставках), так что это задержка, а не вечный park. Отклонение 5.2а-Д-4 по существу верное, просто неполно описанное | средняя |
| C-13 | MINOR | `seed_index.rs:189-195` (`admit`, отравленный лок ⇒ `warn!` + drop) | `HEAD:certify.rs:204-210` (та же форма для `insert`) | `admit` возвращает `()`, поэтому `certificate_verdict` отдаёт `Observed::Recorded` для σ, которая НЕ подана. Инлет считает ingest чистым, executor будет держать высоту. Плюс `warn!` не защёлкнут — на отравленном индексе это строка на каждый сертификат. Форма унаследована от `HEAD`, но на `HEAD` вердикт был `let _observed`, то есть от него ничего не зависело; теперь зависит | Проверил crash-replay: там `Recorded` перепроверяется через `beacon.seed(round)` (`dpos.rs:751-758`) и промах уходит в `Absent` — единственный сайт, который это ловит. Инлет и `spec_exec` не ловят | высокая |
| C-14 | MINOR | `surface.rs:1988` (`unbounded_channel`), `cert_inlet.rs:829-842` (дренаж только в `ingest`) | — (канал введён 5.0, потребителя не было) | Канал не может переполниться, но растёт без границы, пока `ingest` не вызывается; глубины не видно (нет gauge), при росте не предупреждает | Проверил гейт `faults_armed` (`surface.rs:2007`): до взятия приёмника ничего не кладётся, так что рост возможен только у армленного инлета при остановленном потоке — узкий случай | высокая |
| C-15 | MINOR | `seed_index.rs:208-212` (`Pending ⇒ Pending` last-wins) | `HEAD:certify.rs:270` (`map.insert`, то же) | Любой поздний отправитель затирает ранее удержанную честную σ того же раунда; при settle затёртая отказывается и дропается. Поведение тождественно `HEAD` — называю только потому, что новый док объявляет автомат `admit` носителем свойств, которые несло разделение карт, а это свойство не несёт ни то, ни другое | Сверил с `HEAD` дословно — идентично; регрессии нет | высокая |
| C-16 | NIT | — | — | Журнал §3(е): «`-p fluentbase-slasher` в этом воркспейсе НЕТ … число "slasher 16/0" к этому дереву не привязывается». Ворота — `cargo test -p fluentbase-consensus --test slasher_integration` (`gates/run.sh`), файл `crates/dpos/consensus/tests/slasher_integration.rs` существует, ворота зелёные 16/0 (`c1-slasher.txt`) | Прочитал `run.sh` и `c1-status.txt` | высокая |
| C-17 | NIT | — | — | Две формулировки журнала неверны по существу: §5(1) «σ-половина `follower.rs` удалена целиком» (см. C-02) и §0(12) п.3, где общий бюджет подан как «Pending теряется раньше» вместо опасного направления (см. C-03) | Сверил §5/§0(12) с кодом `follower.rs` и `seed_index.rs` | высокая |
| C-18 | NIT | `executor.rs:752`, `:1689`, `:2479`, `:2984`, `:9068`, `:9475`; `beacon/mod.rs:173-175` (`keyless_index` на тестовой границе) | — | Формально: `5.2-A-impl-1.md` п.3 разрешал в `executor.rs` «ТОЛЬКО `#[cfg(test)]`-модуль», а правки задели доккомментарии продакшн-половины (семантика не изменена — проверил каждую). Отдельно: `keyless_index` — новое имя в `beacon::testing`, в доке `mod.rs` не объяснено, в отличие от алиаса `SeedStore` | Прочитал каждую из шести правок — все прозаические. Проверил, что `beacon_over` строит то же, что удалённый `for_seeds` — да | высокая |
| C-19 | NIT | `beacon/follower.rs:212`, `:215`, `:257`, `:267`, `:276` | `HEAD:beacon/follower.rs:209` и далее | Имя `promoter` (переменная и параметр `Weak<FollowerRandomness>`) пережило удаление задачи-промоутера; в `plane.rs` она удалена, здесь — нет. Читается как ссылка на несуществующую сущность | — | высокая |
| C-20 | MINOR | `dpos.rs:2477-2478`, `:3252-3254`; `executor.rs:663-671`; `cold_start_jump.rs:774`; `testbed/stand.rs:156`; `testbed/tests.rs:1343`, `:1591`; `testbed/preconditions.rs:304`, `:314` | те же строки на `HEAD`, где они были ВЕРНЫ | Дрейф доков, созданный правкой R-016 и не убранный. `dpos.rs:3252-3254` (O-5 оркестратора — подтверждаю) и `dpos.rs:2477-2478` («Rule Y … same epoch-relative threshold») стоят НЕПОСРЕДСТВЕННО над новыми строками и им противоречат. `executor.rs:663-671` инвертирован: «Only the tests construct a bare `JUMP_THRESHOLD`» — теперь наоборот. `testbed/tests.rs:1343` называет `min(...)` «production's own … and that is load-bearing». Пять из девяти сайтов — в ПИСАБЕЛЬНЫХ файлах | Прочитал каждый сайт целиком; O-5 подтверждён дословно | высокая |
| C-21 | MODERATE | `testbed/tests.rs:1385`, `:1618`, `:2616`, `:3492`, `:3709`; `testbed/preconditions.rs:162`, `:353`; `testbed/committee_tests.rs:201`, `:619` — все `Some(JUMP_THRESHOLD.min(EPOCH_LEN))` | те же строки на `HEAD`, где они ЗЕРКАЛИЛИ продакшн | Изменение продакшн-ЖИВОСТИ не покрыто ни одним тестом: каждый стендовый сайт задаёт СТАРУЮ продакшн-формулу. `preconditions.rs:353`, чей доккоммент измерил клин при `гейт ≥ 2·interval`, остаётся зелёным при сломанном свойстве — это и делает C-01 блокером по рубрике. Четыре из девяти сайтов — в писабельных файлах, то есть покрытие можно было добавить в рамках захода | Искал тест, ставящий `Some(JUMP_THRESHOLD)` без `.min`: `git grep re_jump_threshold -- crates` — таких нет; `stand.rs:156` документирует значение как «production's own gate, `JUMP_THRESHOLD.min(epoch_block_interval)`» | высокая |
| C-22 | MINOR | `surface.rs:143-145` (`terminal_seed` — дефолт над `seed`) | `HEAD:surface.rs:2128-2136` (`terminal_seed_at` читал ТОЛЬКО пин, без fall-through на окно), `HEAD:certify.rs:400-407` | Свойство «на соседний раунд закрытой эпохи ответа НЕТ» исчезло. Безопасность цела (вызывающий называет раунд из согласованных данных, `epoch_manager.rs:159-163`), и `HEAD`-док сам считал две формы эквивалентными по аргументу — но теперь они эквивалентны буквально, и ни один тест не пинует, что `terminal_seed` не отвечает на не-терминальный раунд | Проверил вызывающих: `epoch_manager.rs:163` и `:2874` (тест) — оба называют раунд из терминального блока. Практической разницы не нашёл, поэтому MINOR | высокая |
| C-23 | NIT | `beacon/actor.rs:1410`, `engine.rs:44` | `HEAD` те же | O-7 оркестратора — ПОДТВЕРЖДАЮ: удалённый модуль `beacon::certify` назван в двух доккомментариях. `actor.rs` писабелен, `engine.rs` — нет | `git grep 'beacon::certify' -- crates` | высокая |

---

## §2 Поведение по ханкам против `HEAD`

### 2.1 `beacon/seed_index.rs` (untracked, 882 стр., в диффе отсутствует)

Замещает `HEAD:beacon/certify.rs` (935 стр.). Поведенчески:

- **Структура**: `SeedIndex { entries: Arc<Mutex<BTreeMap<Round, Entry>>>, persist:
  Option<UnboundedSender<(Round, BlsSignature)>>, events: broadcast::Sender }`
  (`:88-105`) против `HEAD` `SeedStore { seeds, quarantined, terminal, waiters,
  persist, events }` (`HEAD:certify.rs:76-124`). Четыре карты → одна.
- **`Entry::{Verified, Pending}`** (`:79-84`) — состояние вместо карты.
- **`admit` (`:187-244`) — единственный путь вставки**; три входа над ним:
  `record` (`:168`), `hold` (`:175`), `with_persistence` (`:141`). Автомат —
  §0(8). Поведенческое отличие от `HEAD`: `HEAD` держал «непроверенное не
  перезапишет проверенное» структурно (разные карты), дерево — веткой `match`.
- **`seed` (`:255-260`)** = `HEAD:lookup` + отказ на `Pending`. Отвечает точно на
  раунд, `Pending` — промах.
- **`pending_epochs` (`:263-274`)** = `HEAD:quarantined_epochs`. `dedup()` без
  `sort()` — **корректно**, проверил: `Round` в commonware выводит `Ord` по
  `(epoch, view)` (`consensus/src/types.rs:410-416` в чекауте `3c4e02c`,
  док прямо: «ordered first by epoch, then by view»), поэтому `BTreeMap`
  отдаёт эпохи группами.
- **`settle_epoch` (`:281-327`)** = `HEAD:promote_epoch`, с той же семантикой
  «отказанное дропаем» и той же защитой от гонки при повторном взятии лока.
- **`evict` (`:355-371`) + `oldest_evictable` (`:392-405`)** заменяют
  `pop_first` + `pin_terminal` + `retain_terminal_from` + `retain_quarantine_from`.
  Потери разобраны в §0(5)(а): C-03, C-04, C-05.
- **Тесты**: 13 против 14, разбор в §0(1).

### 2.2 `beacon/surface.rs`

- `Beacon::terminal_seed` → дефолт над `seed` (`:143-145`); `Randomness::terminal_seed_at`
  удалён. C-22.
- `Beacon::observe_epoch`/`observe_cert` → пустые дефолты (`:206`, `:218`);
  обе реализации `LiveBeacon` и обе у `StaticRandomness`/`Absent`/`Canned`
  удалены. C-10, отклонение 5.2а-Д-1.
- `certificate_verdict`: параметр `quarantine` → `hold`, тела арм не изменились.
- `Randomness::quarantine_seed` → `hold_seed`.
- `LiveBeacon::report_fault` — `pub(crate)` → приватный (плюс к инкапсуляции);
  новый `LiveBeacon::settle_pending` (`:2031-2054`).
- `for_seeds` удалён (`HEAD:985-1006`).
- Тестовая фикстура `Canned.store` — тип `SeedStore` → `SeedIndex`; её
  `terminal_seed` теперь читает `store.seed`, а не `store.terminal_at`.

### 2.3 `cert_inlet.rs`

- Новое поле `faults` (`:389-404`), берётся в `with_randomness` (`:466-474`).
- `drain_late_verdicts` (`:829-842`), вызов в голове `ingest` (`:557`). C-07.
- Синхронный `Refused` читается (`:731-746`): `warn!` + `record_data_fault` +
  `return` — сертификат не уходит ни в marshal, ни в окно.
- By-height дверь (`:2920-2940`): вердикт читается, но только `warn!`. C-09.
- `self.randomness.observe_cert(epoch)` (`:758`) — остался, тело пустое. C-10.
- Два новых теста (`:2140-2270`), плюс замены `lookup`→`seed`,
  `quarantined_epochs`→`pending_epochs` в существующих.

### 2.4 `dpos.rs`

- `crash_recover_defer_or_fatal` расщеплён (`:438` / `:466`), обе ветки дословны.
- `ReplaySeed::Defer` (`:651-656`), `CertSeed` (`:708-717`), `seed_via_beacon`
  (`:741-762`) вместо `seed_from_cert`; вызовы в `recover_replay_seed`
  (`:817-823`, `:836-846`); ветка `Defer` в вызывающем (`:1001-1013`).
- R-016 ×2 (`:2494`, `:3257`). C-01, C-20.
- Новый тест `the_replays_certificate_seed_is_checked_under_the_epoch_key`
  (`:4710-4785`) с четырьмя ветками.

### 2.5 `beacon/plane.rs`

- Удалены `promoter_edge` (одна из двух подписок) и вся задача `seed_promoter`
  (`HEAD:947-985`), и её строка в списке супервизируемых (`HEAD:1013`).
- `settle.settle_pending()` в KEY-арме моста, ПЕРЕД публикацией (`:973-977`). C-08.

### 2.6 `beacon/follower.rs`

- Удалены `promote_quarantined` и `retain_from`; `FollowerRandomness` получил
  собственный `SeedIndex` (`:310`), `settle_pending` (`:326`), защёлку (`:316`,
  `:379`); want переехал в `hold_seed` (`:370`).
- `run_fetcher` KEY-арм зовёт `settle_pending` и затем шлёт `KeyAvailable`
  (`:269-273`).
- Восемь тестов против семи. Файл не удалён. C-02, C-06, C-11.

### 2.7 Прочие

`spec_exec.rs` — читатель вердикта (`:116-125`), C-12. `beacon/mod.rs` — §0(14).
`beacon/seed_journal.rs`, `verified_seed.rs`, `artifact.rs`, `lib.rs` — только
переименования в прозе и типах. `executor.rs` — фикстура + доккомментарии, C-18.
`testbed/{tests,cert_inlet_tests}.rs` — доккомментарии + две строки `logs_containing`
под новый текст ERROR (`held seed does not verify`).

---

## §3 Граница — таблица имён

См. §0(14). Итог: **крейт-внешняя граница (`pub use`) не изменилась ни на одно
имя**; в тестовом тире `certify::SeedStore` → `seed_index::SeedIndex as SeedStore`
(алиас с объяснённым держателем), `for_seeds` удалён, `keyless_index` добавлен.
Новых `pub` нет.

---

## §4 Ворота

`c1` verbatim — §0(1), таблица. Мой точечный прогон — §0(1), 28/0 за 5.78 s.
Полный набор ворот я не перегонял (доверенный вход оркестратора, помечено как
относящееся к его прогонам, не к моим).

Финальная сверка дерева — `md5sum -c c1-tree.md5`:

~~~
crates/dpos/consensus/src/beacon/artifact.rs: ЦЕЛ
crates/dpos/consensus/src/beacon/follower.rs: ЦЕЛ
crates/dpos/consensus/src/beacon/mod.rs: ЦЕЛ
crates/dpos/consensus/src/beacon/plane.rs: ЦЕЛ
crates/dpos/consensus/src/beacon/seed_index.rs: ЦЕЛ
crates/dpos/consensus/src/beacon/seed_journal.rs: ЦЕЛ
crates/dpos/consensus/src/beacon/surface.rs: ЦЕЛ
crates/dpos/consensus/src/beacon/verified_seed.rs: ЦЕЛ
crates/dpos/consensus/src/cert_inlet.rs: ЦЕЛ
crates/dpos/consensus/src/dpos.rs: ЦЕЛ
crates/dpos/consensus/src/executor.rs: ЦЕЛ
crates/dpos/consensus/src/lib.rs: ЦЕЛ
crates/dpos/consensus/src/spec_exec.rs: ЦЕЛ
crates/dpos/consensus/src/testbed/cert_inlet_tests.rs: ЦЕЛ
crates/dpos/consensus/src/testbed/tests.rs: ЦЕЛ
~~~

Плюс `testbed/preconditions.rs` — `949cfc27e7286711defe3cbc01b625a1`, `ЦЕЛ`
(мутацию поставить не дали, файл не тронут).

---

## §5 Оставить как есть

1. **Автомат `admit` вместо двух карт.** Свойства I1, «проверенное не
   перезаписывается», «Pending last-wins», «отказанное дропается» несёт
   корректно и пинуется тестами. Возврат к двум картам не нужен; лечить надо
   ВЫТЕСНЕНИЕ (C-03/C-04/C-05), а не автомат.
2. **`settle_pending` В МОСТЕ, ПЕРЕД публикацией.** Гонка «потребитель разбужен
   раньше промоутера» была реальной и наблюдаемой (§0(9)(а)); порядок правильный.
   Чинить надо стоимость (C-08 — например, ограничить число промоушенов на ребро
   или уйти в `spawn_blocking`), а не порядок.
3. **Расщепление `crash_recover_defer_or_fatal` (5.2а-Д-2).** Обе ветки дословны,
   мотив («вызвать с подложенным `has_upstream = true` значило бы написать в коде
   неправду») правильный. Оставить.
4. **Приёмник `faults()` внутри `with_randomness` (5.2а-Д-3).** «Не более одного
   потребителя» стало структурным фактом, а не соглашением. Оставить.
5. **`spec_exec` отказывается спекулировать вместо обнуления seed (5.2а-Д-4).**
   Это прямо закрывает класс «spec-exec seed-blind divergence». Оставить (C-12 —
   доработка, не откат).
6. **Проверка σ на crash-replay (`seed_via_beacon`) и `Pending ⇒ Defer`.** E5-03
   закрыт по существу, тест четырёхветочный и самопроверяющий подмену. Оставить.
7. **Пустые дефолты `observe_epoch`/`observe_cert` на трейте** — как компромисс
   до 5.4 приемлемы; убрать надо ВЫЗОВ и доки в писабельных файлах (C-10), а не
   переигрывать отклонение.
8. **Алиас `beacon::testing::SeedStore`** с явным доком и перечисленными
   держателями — правильная форма временного имени. Оставить до 5.4.
9. **`SEED_RETENTION = 4096` не тронут.** Оставить; лечить надо разделение
   бюджета, а не величину.

---

## §6 Замечено вне рамок 5.2

1. **Для 5.4**: `Beacon::terminal_seed` сводится к `seed` одной правкой
   `epoch_manager.rs:163` + `:2874` + `testbed/byzantine_roles.rs:372`; тогда же
   снимаются `observe_epoch`/`observe_cert` с трейта и алиас
   `beacon::testing::SeedStore` (`beacon/mod.rs:171`). Список держателей в доках
   точный, проверил все пять якорей `epoch_manager.rs`.
2. **Для 5.4 / отдельной строки**: у `UpstreamResolver` нет `RotateUpstream`
   (C-09). Конструкторы — `crate::outer` и `node/dpos.rs`. Вместе с рычагом стоит
   решить и вопрос «пометить и перезапросить запись в архиве», который текст
   варианта (в) Д-3 называет, а код не делает.
3. **Для 6.1**: три транзиентные копии σ (`executor.rs:134`, `:239`, `:724`) —
   ровно три, не прибавилось.
4. **Для Ф7 (доки)**: `.claude/dpos_architecture/13_invariants_gotchas_rules.md:623`
   записывает как ИНВАРИАНТ то, что заход снял («a SEPARATE quarantine map — not a
   flag on the shared one»). Это не переименование, это решение. Там же
   `09_followers.md:147` называет `observe_cert`'s `try_send` «the edge that must
   not be deleted» — ребро удалено (корректно заменено на `Pending`), но
   формулировку надо снять явно, а не молча.
5. **Вне 5.2, но рядом**: `node/dpos.rs:931` перечисляет «the quarantine promoter»
   среди супервизируемых детей `BeaconPlane` — задачи нет, список неверен.
   Файл запрещён этому заходу.
6. **Общее наблюдение по девнету**: `smoke-rejump-signer`, `smoke-cert-catchup` и
   `asserts_onchain.py:627` жёстко закодировали формулу порога как предмет
   утверждения. Любое решение по C-01 должно идти вместе с правкой этих трёх
   мест — иначе девнет-приёмка (Ex-21) упрётся в них первой.
