# E5-0-REVIEW — ревью строки 5.0 против спеки и журнала

База `1e5f394e`, ветка `djadjka/dpos-reth-2.2-squashed`, объект — незакоммиченное рабочее
дерево (30 файлов под `crates/` + `.dpos-study/PLAN.md`). Спека: `history/E5-BEACON-DESIGN.md`
§5.1 (556-595) и §7 строка 5.0 (711-733). Журнал: `history/E5-0-BOUNDARY.md`.
Все ворота прогнаны мной в этой сессии.

## §0 Прямые ответы

**1. Ворота (verbatim, по одной строке).**

- `cargo test -p fluentbase-consensus --lib` → `test result: ok. 636 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 21.18s`
- `cargo test -p fluentbase-consensus --lib --features dpos-devnet-byzantine testbed::` → `test result: ok. 35 passed; 0 failed; 0 ignored; 0 measured; 609 filtered out; finished in 30.61s`
- `cargo test -p fluentbase-node --lib` → `test result: ok. 57 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 58.23s`
- `cargo clippy -p fluentbase-consensus -p fluentbase-node --all-targets` → ровно три строки: `warning: large size difference between variants`, `warning: `fluentbase-node` (lib) generated 1 warning`, `warning: `fluentbase-node` (lib test) generated 1 warning (1 duplicate)`
- `cargo clippy -p fluentbase-consensus --all-targets --features dpos-devnet-byzantine` → ноль строк `warning`/`error`, код выхода 0
- `rustfmt --edition 2021 --check` по всем изменённым `.rs` → пусто; `cargo fmt -p fluentbase-consensus -p fluentbase-node -- --check` → 0 строк `Diff in`
- `cargo doc -p fluentbase-consensus --no-deps 2>&1 | grep -c "unresolved link"` → `9`

Расхождений с журналом по воротам НЕТ. Девять ссылок: `E` ×4 (`beacon/mod.rs:2`,
`cert_inlet.rs:345`, `cold_start_jump.rs:795`, `executor.rs:508`), `subscribe`
(`dpos.rs:1023`), `crate::epoch_manager::Actor::enter` ×2 (`engine.rs:142`, `scheme.rs:47`),
`FakeMarshal` (`executor.rs:340`), `Scheme::verify_attestation` (`slasher/evidence.rs:510`) —
ни одна не про границу beacon-а. Предупреждение clippy — `large_enum_variant`; сам
`ValidatorUpstream` в диффе не встречается, подтверждаю как чужое.
Счёт `#[test]` + `#[tokio::test]` по каждому из 30 изменённых файлов HEAD == дерево
(30 из 30 совпали); отдельным счётом только `#[test]`: `executor` 118/118,
`epoch_manager` 13/13 — это и есть числа журнала Б§8.

**2. Поведение vote/cert-пути идентично HEAD — ДА для самого вердикта, но названо не всё.**
Тело вердикта перенесено дословно (сверка `surface.rs:305-351` против
`HEAD:cert_inlet.rs:3012-3042` и `HEAD:spec_exec.rs:63-89` — те же четыре исхода, те же две
`error!`-строки, тот же `unreachable!`), `seed`/`terminal_seed`/`oracle_for` — переименования,
гейт `mandatory_at` в `signer` вердикта не меняет (`carry.rs:115-129` уже отдавал `Some(None)`).
Изменений поведения, НЕ названных журналом, — **шесть**: F-2 (очистка `catchup_no_progress`
на `KeyAvailable`), F-3 (пробуждение epoch_manager-а ~1/с), F-4 (`Closed` → горячий цикл
executor-а), F-5 (дроп-гвардия супервизора не покрывает abort-до-первого-poll), F-10
(источник курсора `dkgQual` у follower-а), F-11 (`dkgQual` валидатора на нефинализированном
и, в откате, на генезисном хэше, с мемоизацией). Плюс F-9 (потеря поля `?check`) — журнал её
называет в Б§8 как наблюдение, но не как изменение и не чинит.

**3. Утечки за границу вне §5.1 и Д-1…Д-8 — шесть** (F-21 a-f): `agreement_partition`,
`absent_unregistered` в двух продакшн-конструкторах, `ArtifactFetch` вместо
`Arc<dyn ArtifactUpstream>`, `PinEffort`, тестовая дверь `beacon::testing` из 25+6 имён,
и три экспортированных типа без внешнего потребителя (`Observed`, `DataFault`,
`WithheldReason`). `pub(super) trait Randomness` (`surface.rs:443`) снаружи `beacon/`
действительно недостижим: `git grep 'Randomness'` вне `beacon/` даёт только имена
`StaticRandomness`/`WithholdingRandomness`/`FollowerRandomness` и комментарии.

**4. Изменённые утверждения тестов — шесть строк `assert*`, из них четыре чистых
переименования вызова** (`surface.rs:1583`, `surface.rs:1954-1955`, `follower.rs:1015`) —
**плюс полностью переписанный тест дренажа** `node/dpos.rs:2521-2560`, где сменились
фикстура (реальный `for_keys` над `BeaconKeys::with_persistence` → `Arc<dyn Any>` над голым
сендером), имя, оба текста сообщений и добавлен `ctx.sleep(50 ms)` перед отрицательной
половиной. Счёт `#[test]`/`#[tokio::test]` HEAD vs дерево совпал по всем 30 файлам
(см. §4).

**5. Дрейф документации `.claude/dpos_architecture/`.** Документация ОБНОВЛЕНА (`00_preamble`
несёт две новые записи `verified-against`, правлены 08, 09, 13, 15, 03, 01). Устаревших
ссылок против дерева — **три места в двух файлах**: `15_smoke_cases_…md:381` (F-15),
`09_followers.md:206-207` (F-16) и блок `09_followers.md:232-244` (F-17, семь якорей).

**6. Готово ли 5.0 к коммиту как есть — ДА ПОСЛЕ ПРАВОК.** Блокеров нет: ворота зелёные,
вердикт байт-в-байт, граница компилятором закрыта. До коммита стоит закрыть F-1 (мёртвые
`Notify` + доки, которые описывают удалённого потребителя как живого — это ровно тот класс,
который 5.1/5.2 потом примут за живой контракт), F-6 (осиротевший док на `UpstreamResolver`),
F-2/F-3 (дописать в журнал и в комментарий арма два неназванных изменения), F-13/F-14
(устаревшие имена символов в продакшн-комментариях — в этом репозитории это блокер ревью)
и F-15/F-16/F-17 (док-дрейф, тот же пункт CLAUDE.md). F-19 (строка PLAN сама себе
противоречит) — одна правка текста.

**7. Три самых опасных находки.**
F-1: `SeedStore::notify` и `BeaconKeys::notify` продолжают выстреливать пермит, которого в
продакшне никто не ждёт, а их доки всё ещё называют executor-а waiter-ом — мёртвый механизм
с живой документацией.
F-4: на `RecvError::Closed` арм executor-а проваливается сквозь фильтр и крутится без сна,
тогда как на HEAD он парковался навсегда.
F-11: `dkgQual` валидатора читается на `max(fin, live)` и в откате может прийти на
`block_hash(0)`, а `frozen_dkg_qual` мемоизирует `false` навсегда — на HEAD в этом окне
чтения не было вовсе.

**8. Где журнал вводит в заблуждение.**
(а) Б§0.2 «единственная правка внутри тела теста — `beacon/surface.rs:1954-1955`» — неверно
против дерева: есть ещё `surface.rs:1583`, `follower.rs:1015` и переписанный целиком тест
`node/dpos.rs:2521-2560` (F-25).
(б) Б§8 «ни одно утверждение теста не тронуто, подтверждено машинно» — верно только для
строк `assert*` и только для захода Б; смена фикстуры того же теста на `Arc<dyn Any>`
машинным грепом по `assert` не видна, а именно она и есть изменение утверждения.
(в) Б§6 «Файл 09 не входит в список документов этой задачи, поэтому НЕ тронут» — файл тронут:
`09_followers.md:208` и `:246` несут маркеры `[REVISED 2026-09-11, PLAN row 5.0 pass A]` (F-18).
(г) А§3, строка `consensus/dpos.rs::launch_follower`: «`dkgQual` follower-а читается на
`read_at()` (finalized-хэш) — тот же хэш, что и раньше». Хэш, может быть, и тот же, но
ИСТОЧНИК другой: было `provider.finalized_block_number()` + `block_hash`, стало
`CanonicalInMemoryState::get_finalized_num_hash()` (F-10). Утверждение о равенстве двух
поверхностей reth в любой момент времени ничем в изменении не обеспечено.
(д) А§3, строка `epoch_manager` (пробуждения): изменением названо только «share теперь будит
sweep». Второе изменение того же слияния — `catchup_no_progress = None` теперь выполняется и
на `KeyAvailable` — не названо (F-2); третье — сам арм не гейтирован и просыпается на каждом
`SeedRecorded` (F-3) — тоже.
(е) А§10 про дроп-гвардию супервизора: «`Abortable` владеет ею и дропается до `tree.abort()`
— то есть drop-glue выполняется и шесть детей аборчатся». Это верно только после первого
poll-а тела: `SupervisedChildren` конструируется ВНУТРИ async-блока (`plane.rs:193`), а
`commonware_runtime::Handle` не имеет `Drop` (F-5).
(ж) А§6 и §7 утверждают, что фичевый clippy на узле не собирается; Б§8 это же опровергает.
Проверил сам: фичевый clippy консенсуса собирается и чист
(команда и вывод — в §4); утверждение А устарело, и оно осталось в тексте А неисправленным.

**9. Где моя проверка была слабее всего.**
1. Ни одного живого узла и ни одного прогона девнет-смоука: F-12 (дренаж трёх писателей
   после того, как артефактный стор переехал внутрь `Arc<dyn Beacon>`) я разобрал только
   по коду и по владению; «все клоны умирают до `drain_shutdown_tasks`» — [ГИПОТЕЗА].
2. F-11 я довёл до «мемо может быть засеяно с генезисного хэша», но НЕ построил конкретный
   девнет, где `committee[E]` для `E ≥ DETERMINISTIC_BOOTSTRAP_EPOCH` закоммичен в генезисе;
   без этого эксплуатируемость — [ГИПОТЕЗА].
3. Девять `unresolved link` я сверил со списком журнала, но не собрал `cargo doc` на HEAD,
   то есть «ни одна не новая» — по совпадению со списком журнала, не по прямому сравнению.
4. `beacon/actor.rs` (7,4 тыс. строк) я читал только по диффу (одна строка) и по
   `DkgActor::new` в месте спавна — внутренности DKG-автомата не читал.
5. Хунки `testbed/stand.rs` (140 строк) просмотрел по диффу-статистике и по журналу, но
   построчно не сверял с `HEAD:` — стендовые ворота 35/0 прогнал сам, на них и опираюсь.
6. Текстовые проверки этого отчёта, после правок: строк, оканчивающихся на многоточие
   — 0; строк с нечётным числом обратных кавычек — 0 (две оставшиеся — ограждения блока
   кода в §4); удвоенных запятых — 0; пустых пар обратных кавычек — 0; пустых круглых
   скобок вне код-спанов — 0. Исправлено до сдачи: четыре код-спана, разорванных переносом
   строки, и три вертикальных черты внутри код-спанов в ячейках таблиц (они ломали
   разметку строк F-4, F-11, F-13, F-25).

## §1 Находки

| id | серьёзность | файл:строки (дерево) | HEAD-якорь | что не так | чем пытался опровергнуть и почему не вышло | уверенность |
|---|---|---|---|---|---|---|
| F-1 | SERIOUS | `beacon/certify.rs:76,241`, `:52-57`, `:186-188`, `:730-732`; `beacon/keys.rs:155,282`, `:34-46` | `HEAD:beacon/certify.rs:455-461` (`notifier`), `HEAD:beacon/keys.rs:380-386` (`notifier`), `HEAD:executor.rs:1167` | Акцессоры `SeedStore::notifier`/`BeaconKeys::notifier` и трейт-методы `seed_edge`/`key_edge` удалены, но сами поля `notify` и вызовы `notify_one` остались. В продакшне их никто не ждёт; единственные оставшиеся waiter-ы — собственные тесты (`certify.rs:577,619`, `keys.rs:1000,1010`). Доки при этом по-прежнему описывают арм executor-а и правило «один waiter» как живые | Прогрепал каждый `.notified()` в `crates/`: продакшн-ожидатели — только `beacon_keys.subscribe()` (`plane.rs:942,983`), `share_notify` (`plane.rs:984`) и `keys.subscribe()` follower-а (`follower.rs:211`). Ни один из них не `self.notify` | по коду |
| F-2 | SERIOUS | `epoch_manager.rs:829-844` | `HEAD:epoch_manager.rs:764-770` (`share_n`), `:816-822` (`key_n`) | Слияние двух армов добавило ВТОРОЕ изменение поведения, не названное журналом: `self.catchup_no_progress = None` теперь выполняется и на классе `KeyAvailable`. На HEAD его чистил только share-арм | Сверил оба HEAD-арма построчно: `key_n` делал ровно `sweep_wake.send_replace` + `reconcile_live`, без сброса мемо. Эффект безвреден (лишняя попытка догона), но это отдельная дельта | по коду |
| F-3 | MODERATE | `epoch_manager.rs:829-832` | `HEAD:epoch_manager.rs:682-700` (набор армов) | Арм `beacon_events.recv()` не гейтирован, поэтому цикл epoch_manager-а просыпается на КАЖДОМ `SeedRecorded` (~1/с при целевом блок-рейте), чтобы сделать `continue`. На HEAD источника пробуждений с частотой раунда у этого цикла не было. Это тактовая частота там, где были только рёбра | Искал гейт: значение события лежит внутри future, `if`-условие арма его не видит, так что гейтом на месте не обойтись. Альтернатива (фильтр в мосте `plane.rs:986-1000` или отдельный канал на класс) существует и не применена | по коду |
| F-4 | MODERATE | `executor.rs:1494-1508` | `HEAD:executor.rs:1478-1482` | `matches!` отфильтровывает только два `Ok`-класса, `KeyAvailable` и `ParticipationChanged`. `Err(RecvError::Closed)` проходит фильтр, запускает `try_eager_finalized_derive`, и следующая итерация снова получает `Closed` немедленно — безостановочный горячий цикл, пока `awaiting_seed.is_some()`. На HEAD арм ждал `Notify` и парковался навсегда | Пытался доказать недостижимость: сендер живёт внутри того же объекта, который executor держит как `Arc<dyn Beacon>` (`certify.rs:128,166`; `surface.rs:622,668,1110`), так что сегодня `Closed` не наступает. Но это свойство ничем не зафиксировано — ни тестом, ни типом; любая будущая реализация с внешним сендером даст спин | по коду / [ГИПОТЕЗА] на достижимость |
| F-5 | MODERATE | `beacon/plane.rs:186-215` (`:193`) | нового аналога на HEAD нет (восемь хэндлов у узла) | `SupervisedChildren` конструируется ВНУТРИ тела задачи. `commonware_runtime::Handle` не имеет `impl Drop` (в `runtime/src/utils/handle.rs` есть только явный `abort()` на `:108-118`), поэтому супервизор, аборченный до первого poll-а, дропает голый `Vec<(&str, Handle<()>)>` и шесть детей остаются живыми | Искал `impl Drop for Handle` в пинованном чекауте — нет. Аргумент журнала (А§10) про drop-glue опирается на то, что обёртка уже существует, то есть на состоявшийся первый poll. Окно узкое (узел успевает много поработать до шатдауна), но оно есть | по коду / [ГИПОТЕЗА] на достижимость |
| F-6 | MODERATE | `cert_inlet.rs:2998-3018` | `HEAD:cert_inlet.rs:2993-3012` | Док удалённой `capture_certificate_seed` не удалён вместе с ней, а прилип к `pub struct UpstreamResolver` — публичный тип теперь документирован текстом о том, куда кладётся σ, и `cargo doc` это отрендерит | Проверил дерево напрямую: между `impl Drop for InflightGuard` и `pub struct UpstreamResolver` стоит именно этот блок `///`, без разрыва | по коду |
| F-7 | MODERATE | `spec_exec.rs:91-93`, `cert_inlet.rs:860-862`, `cert_inlet.rs:3127-3130`; тип — `surface.rs:236-249` | `HEAD:cert_inlet.rs:866`, `:3154`, `HEAD:spec_exec.rs:79-89` | `Observed` объявлен `#[must_use]`, и все три продакшн-сайта гасят его `let _ =`. §5.1 прямо отдаёт «синхронный `Refused` ⇒ data fault inlet-а» именно этой строке; PLAN строка 5.2 отдаёт проводку туда. В итоге вводится граничный тип с нулём читателей, и ничто — ни компилятор, ни тест — не фиксирует, что это осознанно | Смотрел, не читается ли вердикт где-то ещё: `git grep 'observe_certificate'` вне тестов даёт ровно три сайта, все с `let _ =`. Д-8 покрывает `faults()`, но не синхронный `Refused` | по коду |
| F-8 | MODERATE | `surface.rs:227-233`, `:319`, `:336-343` | `HEAD:spec_exec.rs:79-84` | `local` выводится из ТИПА сертификата (`Notarization ⇒ true`), а док `:229` называет этот вариант «Recovered locally from the round's notarization quorum». commonware репортит `Activity::Notarization` и для нотаризации, ПРИШЕДШЕЙ С ПРОВОДА: `try_broadcast_notarization` перечитывает её из state и репортит независимо от происхождения (`voter/actor.rs:492-531`), а провенанс из резолвера отмечается только флагом `resolved` (`:1007-1012`); та же ветка на реплее журнала (`:745-755`). То есть чужая нотаризация пойдёт в арм «locally recovered» | Поведение при этом ИДЕНТИЧНО HEAD — там catch-all `Err(check) => error!` делал то же самое, так что это не регрессия. Но 5.0 возвела ложный инвариант в именованное понятие границы. Практически узко: чужая нотаризация уже проверена батчером, который отвергает плохую σ, когда оракул резолвится | по коду (commonware-чекаут прочитан) |
| F-9 | MODERATE | `surface.rs:328-334` | `HEAD:spec_exec.rs:79-84` | Из локальной `error!`-строки пропало поле `?check`. Оно было единственным, что отличало `Invalid` от любого будущего третьего вида ошибки в логе | Журнал называет это в Б§8 последним пунктом как наблюдение о заходе А и не чинит. Восстановить поле — одна строка; `SeedCheck` уже в области видимости (`surface.rs:336`) | по коду |
| F-10 | MODERATE | `consensus/dpos.rs:3428-3430` | `HEAD:consensus/dpos.rs:3418-3423` | Курсор `dkgQual` follower-а сменил ИСТОЧНИК: было `provider.finalized_block_number()` → `provider.block_hash(fin)`, стало `CanonicalInMemoryState::get_finalized_num_hash()`. Журнал (А§3) называет это «тот же хэш, что и раньше» | Пробовал подтвердить эквивалентность: обе поверхности отвечают про finalized, но одна читает БД провайдера, другая — in-memory canonical state; совпадение в каждый момент старта ничем в изменении не обеспечено и в тестах не закреплено | по коду |
| F-11 | MODERATE | `node/dpos.rs:1370-1390`; `beacon/carry.rs:228-245` | `HEAD:node/dpos.rs:1500-1506` (`dkg_qual_at`) | `read_at()` возвращает `Some` и при `fin.is_none() && live > 0`, а откат на `block_hash(fin)` при `fin = 0` даёт ГЕНЕЗИСНЫЙ хэш — ровно то, что запрещает собственный guard двумя строками выше. `frozen_dkg_qual` мемоизирует `false` НАВСЕГДА, как только `committed` истинно на этом хэше. На HEAD `dkg_qual_at` в этом окне возвращал `None`, то есть чтения не было вовсе, и мемо засеять было нечем | Журнал выносит откат в §7 как предсуществующее `[СЛАБАЯ]` про читатель комитета. Для комитета он и правда предсуществует; для мемоизируемого бита — нет, там это новое окно | по коду; эксплуатируемость [ГИПОТЕЗА] |
| F-12 | MODERATE | `beacon/plane.rs:907-913`; `node/dpos.rs:516-543`, `:882-892` | `HEAD:beacon/plane.rs:657-665` | Артефактный стор переехал ВНУТРЬ `Arc<dyn Beacon>` (`LiveBeaconConfig.artifacts`). Значит дренаж артефактного писателя теперь зависит от того, что упадёт КАЖДЫЙ клон `Arc<dyn Beacon>`, а не только замыкание в реестре RPC. Комментарий порядка при этом по-прежнему рассуждает про ДВА писателя и `Arc<dyn Randomness>` | Замыкание RPC починено на `Weak` (`node/dpos.rs:1915-1919`, `consensus/dpos.rs:3529-3540`), это верно и проверено чтением. Но остальные держатели (`executor`, `epoch_manager`, `cert_inlet`, `outer`, `application`, `spec_exec`) освобождаются только вместе с аборченными задачами, и что это происходит ДО `drain_shutdown_tasks`, ни один тест не достаёт — журнал §9.4/§9.6 это признаёт | по коду; «успевает» — [ГИПОТЕЗА] |
| F-13 | MINOR | `cert_follow.rs:107`, `consensus/dpos.rs:3186`, `node/dpos.rs:377`, `:903`, `:526`, `:542`, `:884` | те же строки на HEAD | Устаревшие имена в продакшн-комментариях: `beacon::for_follower` (функция называется `build_follower`) в четырёх местах; `Arc<dyn Randomness>` в трёх (журнал признаёт только эти три); плюс «BOTH journal senders» (`:526`) и «there are two behind one handle now» (`:884`) — писателей теперь ТРИ | `git grep` по `for_follower` и по `Randomness` вне `beacon/` — все семь строк живы в дереве | по коду |
| F-14 | MINOR | `beacon/mod.rs:26`, `:29-31` | `HEAD:beacon/mod.rs:33-38` | Док парадной двери: «the three staking reads the node necessarily supplies (`CommitteeReads`, `ArtifactFetch`)» — заявлено три, перечислено два. Строкой выше «On the list and deliberately NOT public» — покорёженная правка HEAD-овского «On neither list, and deliberately», читается как противоположное тому, что имеется в виду | Перечитал оба текста рядом; третьего имени в списке нет ни в этом абзаце, ни в `pub use` ниже | по коду |
| F-15 | MINOR | `.claude/dpos_architecture/15_smoke_cases_as_behavioral_spec_devnet_lo.md:381` | — | «`WithholdingRandomness` re-implements a single operation and forwards fourteen» — против дерева `byzantine_roles.rs:367-471` содержит 13 методов, один из них (`signer`) переписан, остальные 12 делегируют. Тот же файл на `:194` говорит «twelve of the thirteen» — файл противоречит сам себе | Посчитал `fn` в теле `impl Beacon for WithholdingRandomness` — ровно 13 | по коду |
| F-16 | MINOR | `.claude/dpos_architecture/09_followers.md:206-207` | — | «Five `impl Beacon` blocks exist in the workspace, and that is the whole set (`beacon/surface.rs:434`, `:650`, `:772`, `:1952`; `beacon/follower.rs:395`)». В дереве блоки `impl Beacon` — `surface.rs:368` (бланкет), `:792`, `:1003`, `:1136` и `testbed/byzantine_roles.rs:367`; в `follower.rs` `impl Beacon` НЕТ вообще (там `impl Randomness for FollowerRandomness`) | Маркер `[REVISED]` строкой ниже оправдывает «якоря сдвинулись», но не неверный СПИСОК файлов: пропущен `byzantine_roles.rs`, назван `follower.rs` | по коду |
| F-17 | MINOR | `.claude/dpos_architecture/09_followers.md:232-244` | — | Блок без маркера `[REVISED]` подаёт якоря как текущие, и семь из них неверны: `absent_unregistered` «`beacon/surface.rs:262`» → `:620`; `absent(ctx)` «`:250`» → `:608`; «`application.rs:413`» → `:377`; «`cert_inlet.rs:505`» → `:500`; «`with_randomness` at `outer.rs:1145`» → `:1150`; «`crates/node/src/cert_inlet.rs:110`» → `:113`; «`dpos.rs:3684` … `dpos.rs:3642`» → `with_randomness` на `:3925` | Проверил каждый грепом по дереву | по коду |
| F-18 | MINOR | журнал `E5-0-BOUNDARY.md` Б§6, первый буллет | — | «Файл 09 не входит в список документов этой задачи, поэтому НЕ тронут» — файл тронут заходом А: `09_followers.md:208` и `:246` несут `[REVISED 2026-09-11, PLAN row 5.0 pass A]`, и первый из них ровно про ту таблицу, о которой Б говорит «нужна отдельная правка» | Прочитал `09_followers.md:205-262` целиком | по коду |
| F-19 | MINOR | `.dpos-study/PLAN.md:107` | `HEAD:.dpos-study/PLAN.md:107` | Строка 5.0 противоречит сама себе: колонка «Работа» по-прежнему содержит «`for_keys`/`for_seeds` удалены», а колонка справа честно говорит, что не удалены. Кроме того, правка строки — не только `[x]`: перепинены два якоря (`stand.rs:1782-1788`→`:1786-1792`, `byzantine_roles.rs:351-370`→`:367-471`) и колонка «Оценка» заменена статусной запиской | `git diff .dpos-study/PLAN.md` — обе половины видны в одном хунке | по коду |
| F-20 | MINOR | `.dpos-study/PLAN.md`, секция Э4 | `HEAD:.dpos-study/PLAN.md` | Тот же незакоммиченный дифф PLAN-а переписывает весь раздел Э4: строки 4.0 и 4.4 удалены, 4.1/4.2/4.3 заменены другими, заголовок раздела получил ссылки на проектные документы. К 5.0 это отношения не имеет и в журнале не упомянуто | `git diff .dpos-study/PLAN.md` — второй хунк | по коду |
| F-21 | MINOR | (a) `beacon/mod.rs:112` + `epoch_manager.rs:17,573`; (b) `mod.rs:113` + `application.rs:377`, `cert_inlet.rs:500`; (c) `follower.rs:92`; (d) `mod.rs:107-110` + `cert_inlet.rs:18`, `epoch_manager.rs:19`; (e) `mod.rs:133-157`; (f) `mod.rs:107-110` | §5.1 (556-595) | Шесть имён/классов имён покидают `beacon/` вне §5.1 и вне Д-1…Д-8: (a) `agreement_partition` — Д-6 покрывает только `agreement_intake`; (b) `absent_unregistered` как дефолт двух продакшн-конструкторов, тогда как §5.1 требует, чтобы `beacon::build` шёл РАНЬШЕ них; (c) `ArtifactFetch`-замыкание вместо `Arc<dyn ArtifactUpstream>`; (d) `PinEffort`, который §5.1 отправляет внутрь (Д-2 называет только `ensure_key`); (e) тестовая дверь из 25+6 имён там, где §5.1 предполагала два; (f) `Observed`/`DataFault`/`WithheldReason` без единого внешнего потребителя | (b) и (e) журнал раскрывает в Б§0.7 и Б§0.3, но ни то ни другое не входит в объявленный список Д-1…Д-8, который §0.4 подаёт как полный («Восемь отклонений от §5.1») | по коду |
| F-22 | NIT | `surface.rs:349` | `HEAD:spec_exec.rs:79-84` | `Err(SeedCheck::Valid) => unreachable!(…)` теперь стоит и на нотаризационном пути, где HEAD писал `error!`. Ветка мертва в обе стороны (`VerifiedSeed::check` отдаёт `Valid` только в `Ok`, `verified_seed.rs:48-52`), но паника заменила лог-строку на продакшн-пути | Проверил `check` — `SeedCheck::Valid => Ok(...)`, `other => Err(other)`; достичь `Err(Valid)` можно только сменой этой функции | по коду |
| F-23 | NIT | `consensus/dpos.rs:3542`; `beacon/follower.rs:139-146` | `HEAD:consensus/dpos.rs:3495-3500` | Follower берёт из `Tasks` только `supervised`, а `drain` роняет на месте. `build_follower` при этом СПЕЦИАЛЬНО спавнит пустую задачу под этот хэндл. Сегодня безвредно (`Handle` без `Drop`, тело пустое), но контракт типа («две ручки, которые узел должен beacon-у», `plane.rs:160-180`) на этом пути не соблюдён, и будущий журнальный писатель follower-а окажется недренированным молча | Смотрел, не регистрируется ли `drain` где-то ниже по follower-пути — нет: `drain_on_shutdown` на follower-е получает только то, что кладёт `launch_dpos_layer` | по коду |
| F-24 | NIT | `node/dpos.rs:2547` | — | Переписанный тест дренажа опирается на фиксированный `ctx.sleep(Duration::from_millis(50))` как на отрицательную половину — таймер там, где можно опросить сам хэндл. Журнал А§10 принимает это как исправление выродившегося теста | Альтернатива (poll хэндла через `noop_waker`, как в `certify.rs:577-600`) в этом же дереве уже применяется, то есть приём в репозитории есть | по коду |
| F-25 | NIT | журнал Б§0.2 | — | «единственная правка внутри тела теста — `Randomness::ensure_key(&canned, …)` → `Beacon::ensure_key(&canned, …)` в `beacon/surface.rs:1954-1955`» — против дерева правок в телах тестов как минимум три больше: `surface.rs:1583`, `follower.rs:1015` и целиком переписанный `node/dpos.rs:2521-2560` | `git diff HEAD -U0` с грепом по `assert` даёт шесть строк, и это только `assert*`; смена фикстуры в теле теста этим грепом не ловится | по запуску |

## §2 Поведение — по пунктам (а)-(д) и спецвопросы

**(а) Курсор чтения `dkgQual`.** Подтверждаю как изменение и подтверждаю направление.
HEAD: `dkg_qual_at` = `provider.finalized_block_number()?` → `block_hash(fin)`
(`HEAD:node/dpos.rs:1500-1506`), и `None`, если finalized-маркера нет.
Дерево: один `read_at()` = `max(fin, live)` с откатом (`node/dpos.rs:1370-1390`), общий
с чтением комитета. Аргумент журнала (бит монотонен, мемо морозит, курсор
cert-финализирован ⇒ можно только увидеть УЖЕ выставленный бит раньше) я проверил по
`carry.rs:228-245` и он держит ДЛЯ ВЫСТАВЛЕННОГО бита. Он не держит для отрицательного
ответа в окне `fin == None && live > 0` — там откат `.or_else(|| block_hash(fin))` с
`fin = 0` даёт генезисный хэш, а мемо запоминает `false` навсегда, если `committed` там
истинно. Это F-11.

**(б) Шатдаун одним дренажом вместо трёх.** Подтверждаю: `spawn_drain` (`plane.rs:227-246`)
ждёт трёх писателей `join_all`-ом внутри одного лимита узла, узел кладёт один хэндл
(`node/dpos.rs:853-856`). Строки лога на писателя сохранены (`plane.rs:236-240`). Новое, в
журнале не взвешенное: артефактный стор переехал внутрь `Arc<dyn Beacon>`, то есть условие
дренажа расширилось с «умер клон стора» до «умерли ВСЕ клоны beacon-а» — F-12.

**(в) `GeometryUnfrozen`.** Починка на месте и верна: ветка стоит ВНУТРИ арма
`mandatory_at && material.is_none()` (`surface.rs:2385-2416`), то есть уточняет причину, а не
вердикт. Сверил с `HEAD:beacon/surface.rs:1954-1962` — там арм тот же, только без
разветвления причины. Вердикт совпадает всегда. Новая метрика
`engine_demoted_geometry_unfrozen` зарегистрирована (`beacon/metrics.rs:51,216-221`).
Замена `BoxFuture` на `watch` (`plane.rs:490-499`, `:798-816`; узел — `node/dpos.rs:1503-1512`,
`:1619-1628`) действительно убирает гонку одноразового `Notify`.

**(г) `SeedStore` и `SeedRecorded` из `record`.** Подтверждаю: сендер живёт в сторе
(`certify.rs:92-96,128,166`), `record` шлёт `BeaconEvent::SeedRecorded` (`certify.rs:242`),
`LiveBeacon::events()` отдаёт именно его (`surface.rs:2345-2349`), и промоутер карантина
пишет через тот же `record`. То есть заявленное свойство (пробуждение от КАЖДОГО писателя)
в коде есть. Обратная сторона — F-1: `notify_one` строкой выше остался без продакшн-waiter-а.

**(д) `certificate_verdict`.** Сверил построчно с двумя HEAD-телами. Финализационная ветка
(`local = false`) воспроизводит `HEAD:cert_inlet.rs:3022-3042` дословно, включая консультацию
`on_invalid_seed` и три её исхода и `error!`-текст. Нотаризационная ветка (`local = true`)
воспроизводит `HEAD:spec_exec.rs:73-89` с двумя отличиями: потеряно поле `?check` (F-9) и
`SeedCheck::Valid` теперь `unreachable!` вместо `error!` (F-22). Оба исхода `Refused`.

**Спецвопрос: гейт `mandatory_at` в `signer`.** Держит. `surface.rs:2456-2458` заменяет
безусловный `self.material(epoch)` HEAD-а (`HEAD:beacon/surface.rs:1988`) на
`mandatory_at.then(|| material).flatten()`. На продакшн-пути `material` идёт через
`(self.resolver)(epoch)` (`surface.rs:2315-2320`), резолвер — `beacon_share_resolver`
(`plane.rs:912-919`), а он для `epoch < DETERMINISTIC_BOOTSTRAP_EPOCH` получает
`Some(None)` из `carry.rs:115-129` → `NoUsableMint` → `BeaconResolve::Absent` → `None`.
Оба пути дают `None`, вердикт не меняется. Заявление журнала верно.

**Спецвопрос: пропавшее `?check`.** Верно, F-9. Было `round`, `?check`
(`HEAD:spec_exec.rs:79-84`), стало только `round` (`surface.rs:328-334`).

**Спецвопрос: `local` из ТИПА сертификата.** Есть путь, где чужая нотаризация попадает в
`local`-арм — F-8, с якорями в commonware. Поведение при этом не отличается от HEAD, так
что это не регрессия, а ложный инвариант, возведённый в имя.

**Спецвопрос: `Observed` нигде не читается.** Да, F-7. Три продакшн-сайта, все `let _ =`,
тип `#[must_use]`. Это дропнутый сигнал в том смысле, что §5.1 назначала его потребителя
уже в 5.0; PLAN строка 5.2 переназначила проводку, но пока результат — граничный тип с
нулём читателей и без единой защёлки, что так и задумано.

**Спецвопрос: crash-replay.** Перевод чисто механический: `replay_seed_source` и
`recover_replay_seed` сменили тип параметра и имена двух методов
(`consensus/dpos.rs:564-590`, `:629-650`, `:743`, `:831`), ветки `Inactive`/`Held`/`Wanted`,
счётчик `crash_recover_stray_seed` и `ReplaySeed::Defer` не тронуты. Против
`HEAD:consensus/dpos.rs:564-590` и `:627-650` — расхождений нет. Переход на
`observe_certificate` с `Pending ⇒ Defer` (§5.1) здесь не сделан и честно отложен в 5.2.

**Спецвопрос: `Tasks` против §5.1 `Tasks{supervised, drain}`.** Форма соблюдена частично.
`supervised` — явный супервизор (`plane.rs:186-215`) с правилом «смерть любого ребёнка
завершает supervised» через `select_all`; шесть детей перечислены поимённо
(`plane.rs:1024-1033`). `drain` спавнится из контекста СБОРКИ, вне линии спавна движка
(`plane.rs:227-232`), что и требует §5.1; тест `the_journal_writer_survives_the_engine_abort_only_outside_its_spawn_lineage`
(`node/dpos.rs:2579`) на месте. Отличия от §5.1: третье поле `agreement_intake`
(объявлено, Д-6) и дроп-гвардия, не покрывающая abort-до-первого-poll (F-5).

## §3 Граница — таблица имён

Из `beacon/mod.rs` выходит 20 `pub` + 2 `pub(crate)` плюс тестовый тир
`#[cfg(test)] pub(crate) mod testing` (25 имён + 6 под фичей). Ниже — только то, что реально называется
вне `beacon/` в НЕтестовом коде, и вердикт против §5.1.

| имя | где объявлено | продакшн-потребители вне `beacon/` | §5.1 / Д |
|---|---|---|---|
| `Beacon` | `mod.rs:107` ← `surface.rs` | `spec_exec.rs:18`, `cert_inlet.rs:18`, `executor.rs:719,979`, `epoch_manager.rs:19,513`, `outer.rs:536,750`, `application.rs:257,417`, `consensus/dpos.rs:10,1072`, `node/cert_inlet.rs:87` | §5.1 ✓ |
| `BeaconEvent` | `mod.rs:107` | `executor.rs:1503-1504`, `epoch_manager.rs:830` | §5.1 ✓ (Д-4 — без нагрузки) |
| `ObservedCertificate` | `mod.rs:107` | `spec_exec.rs:18,93`, `cert_inlet.rs:18,861,3129` | §5.1 ✓ |
| `Observed` | `mod.rs:107` | НЕТ | вне §5.1 как отдельный экспорт — F-21(f), F-7 |
| `DataFault` | `mod.rs:107` | НЕТ (продюсер `plane.rs:965`, потребителя нет) | Д-8 |
| `PinEffort` | `mod.rs:107` | `cert_inlet.rs:18`, `epoch_manager.rs:19` | §5.1 отправляет внутрь — F-21(d) |
| `ShareProbe`, `SignerVerdict` | `mod.rs:107` | `epoch_manager.rs:19,1094,1150,1221` | §5.1 ✓ |
| `WithheldReason` | `mod.rs:107` | НЕТ (используется только через `ShareProbe::Withheld(reason)`) | F-21(f) |
| `Seed` | `mod.rs:106` ← `seed.rs` | `spec_exec.rs:18`, `executor.rs:134,208,573,614,…`, `application.rs:18`, `consensus/dpos.rs:10`, `node/derive.rs:10` | §5.1 ✓ (чистая производная) |
| `prev_randao_from_seed`, `witness_fallback_seed`, `constant_fallback_seed` | `mod.rs:106` | `node/derive.rs:10`, `epoch_manager.rs:18`, `weighted_vrf.rs:8,11,77` | §5.1 ✓ (явно оставлены снаружи) |
| `build`, `ValidatorInputs` | `mod.rs:105` ← `plane.rs` | `node/dpos.rs:1869,1871` | Д-1 |
| `build_follower`, `FollowerInputs` | `mod.rs:104` ← `follower.rs` | `consensus/dpos.rs:3524,3526` | Д-1 |
| `Tasks` | `mod.rs:105` | `node/dpos.rs:1901-1903`, `consensus/dpos.rs:3524` | §5.1 ✓, но третье поле — Д-6 |
| `CommitteeReads` | `mod.rs:105` | `node/dpos.rs:1365,1441`, `consensus/dpos.rs:3421,3480` | §5.1 ✓ |
| `ArtifactFetch` | `mod.rs:104` | `consensus/dpos.rs:3492` | §5.1 требовала `Arc<dyn ArtifactUpstream>` — F-21(c) |
| `agreement_partition` (`pub(crate)`) | `mod.rs:112` | `epoch_manager.rs:17,573` | вне §5.1 и вне Д — F-21(a) |
| `absent_unregistered` (`pub(crate)`) | `mod.rs:113` | `application.rs:377`, `cert_inlet.rs:500` | §5.1 требует обратного порядка сборки — F-21(b) |
| `testing::*` (25 + 6) | `mod.rs:133-157` | только `#[cfg(test)]`, проверено компилятором | §5.1 предполагала два имени — F-21(e) |

Удалены из двери по сравнению с HEAD и НЕ заменены снаружи: `Randomness`, `BeaconResolve`,
`BeaconResolver`, `CommitteePairFor`, `CommitteeSource`, `frozen_dkg_qual`, `AgreedKeys`,
`BeaconKeys`, `ArtifactSource` (переехал в `dpos::ArtifactSource`), `for_follower`,
`FollowerBeacon`, `FollowerRandomnessConfig`, `BeaconConfig`, struct `Beacon`,
`absent`/`for_keys`/`for_seeds` (под `cfg(test)` либо внутрь).
`JOURNAL_RETENTION_EPOCHS` сужен до приватного (`mod.rs:104`).

Приватность подмодулей: все 27 объявлений — `mod x;` (`beacon/mod.rs:45-75`).
`pub(super) trait Randomness` — `surface.rs:443`. Вне `beacon/` имя `Randomness` не
называется нигде, включая комментарии, КРОМЕ семи строк из F-13 (три с `Arc<dyn Randomness>`
в `node/dpos.rs` и четыре с `beacon::for_follower`) и имён другого символа
(`StaticRandomness`/`WithholdingRandomness`/`FollowerRandomness`).

## §4 Ворота — вывод команд

```
$ cargo test -p fluentbase-consensus --lib
test result: ok. 636 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 21.18s

$ cargo test -p fluentbase-consensus --lib --features dpos-devnet-byzantine testbed::
test result: ok. 35 passed; 0 failed; 0 ignored; 0 measured; 609 filtered out; finished in 30.61s

$ cargo test -p fluentbase-node --lib
test result: ok. 57 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 58.23s

$ cargo clippy -p fluentbase-consensus -p fluentbase-node --all-targets
      1 warning: `fluentbase-node` (lib) generated 1 warning
      1 warning: `fluentbase-node` (lib test) generated 1 warning (1 duplicate)
      1 warning: large size difference between variants

$ cargo clippy -p fluentbase-consensus --all-targets --features dpos-devnet-byzantine
(0 строк warning/error, exit 0)

$ git diff HEAD --name-only -- crates/ | grep '\.rs$' | xargs rustfmt --edition 2021 --check
(пусто; только предупреждения про nightly-опции из rustfmt.toml)

$ cargo fmt -p fluentbase-consensus -p fluentbase-node -- --check | grep -c '^Diff in'
0

$ cargo doc -p fluentbase-consensus --no-deps 2>&1 | grep -c "unresolved link"
9
```

Счёт тестов по файлам (`#[test]` + `#[tokio::test]`, HEAD vs дерево) — совпал по всем 30
изменённым файлам; несовпадений ноль. Отдельно только `#[test]`: `executor.rs` 118/118,
`epoch_manager.rs` 13/13 — это те же числа, что приводит Б§8.

## §5 Оставить как есть

- **Два конструктора вместо одного enum-а `BeaconInputs` (Д-1).** Причина в журнале
  проверяема: `ValidatorInputs` несёт семь generic-параметров p2p (`plane.rs:452-458`),
  у `FollowerInputs` — ноль (`follower.rs:99-107`). Единственный enum заставил бы
  follower-сайт (`consensus/dpos.rs:3524`) назвать их турбофишем. Граница от этого не шире.
- **`observe_epoch`/`observe_cert` на трейте (Д-2).** Они по-прежнему ведут ретенцию
  (`surface.rs:2564-2596`), и без них она пропадёт. Отложено в 5.1/5.2 честно.
- **`ensure_key` на трейте (Д-2).** Лестница ключей зовёт его явно
  (`epoch_manager.rs:1713-1716`); чисто реактивное приобретение — работа 5.1.
- **`broadcast` вместо хранимого пермита (Д-3).** Правило «подписаться ДО первого чтения»
  записано в трейте (`surface.rs:203-210`) и соблюдено обоими потребителями
  (`executor.rs:1177`, `epoch_manager.rs:673`). Порядок, при котором пробуждение теряется,
  из этих двух мест непостроим — проверил: `awaiting_seed` заполняется только в
  `on_finalized_block`, а подписка взята до цикла.
- **`share_notify` как единственный waiter моста.** Мост — единственный ждущий
  `notify_one` (`plane.rs:984`), так что схлопывания рёбер между потребителями нет;
  `notified()` пересоздаётся на СТАБИЛЬНОМ хэндле, то есть пермит объектный и не теряется.
- **Общий 5-секундный таймаут на трёх писателей.** Работа на писателя ограничена сотнями
  68-байтных append-ов и одним fsync; принял аргумент журнала.
- **`FollowerCommitteeReads::committee` возвращает `None` всегда**
  (`consensus/dpos.rs:3439-3445`). Follower не ведёт церемонию, `committee_pair` через него
  тоже отдаёт `None`, и это верный ответ, а не заглушка.
- **`unreachable!("Valid is the Ok arm")` как таковой** — перенесён из HEAD, дока честная;
  претензия только к тому, что он теперь стоит и на нотаризационном пути (F-22).
- **`BeaconEvent` без полезной нагрузки (Д-4)** и **отсутствие `Stalled` (Д-5)** —
  производителей нет до 5.3, вариант без конструктора был бы мёртвой поверхностью.
- **Метрика `engine_demoted_geometry_unfrozen`** — новое семейство, зарегистрировано один
  раз (`metrics.rs:216-221`), дублирующего владельца нет.
- **Отсутствие `#[allow(...)]`, `todo!`, `unimplemented!` и новых `unwrap`/`expect` на
  продакшн-путях.** Проверил `git diff HEAD -U0 | grep '^+'` — все добавленные `expect`
  лежат в тестах (`follower.rs:1024`, `surface.rs:1870,1873`), единственный добавленный
  таймер — в тесте (F-24).

## §6 Замечено вне рамок 5.0

- `.dpos-study/PLAN.md` в этом же незакоммиченном дереве переписывает весь раздел Э4 — работа, к строке 5.0 отношения не имеющая (F-20).
- `cert_follow.rs:95` ссылается на `beacon::decode_artifact`, которого в двери нет ни на HEAD, ни в дереве (`decode_artifact` живёт в `beacon::testing`) — предсуществующий мёртвый якорь.
- `plane_upstream.rs:186` ссылается на `beacon::log_resolver::LogHandler` как на код-спан — предсуществует, но после приватизации подмодулей это ссылка в закрытую комнату.
- `application.rs:257` — поле `randomness` у `FluentApp` не читается ни на HEAD, ни в дереве (только `clone()` и сеттер); предсуществующий мёртвый вес, журнал его называет.
- `beacon/certify.rs:412-461` — `waiters`/`wait_for`/`prune_waiters` теперь `#[cfg(test)]`, то есть по-рабочему мертвы; PLAN 5.2 их удаляет, но до тех пор `record` продолжает обходить `waiters` на каждом вызове (`certify.rs:228-235`).
- `surface.rs:596-598` — док `absent` ссылается на `beacon/follower.rs:162`, где теперь стоит `struct FollowerBeacon`, а не построение `FollowerRandomness`.
