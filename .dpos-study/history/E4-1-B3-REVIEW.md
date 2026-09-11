# Ревью захода Б3 (строка 4.1) — стенд по ветке хэша, три стенд-теста, e2e-пин `commit_height`

Ревьюер: Opus 5, свежий контекст. База — `HEAD 17b796fa` (`git rev-parse HEAD`), объект —
незакоммиченное рабочее дерево по шести файлам: `testbed/{fakes,stand,mod}.rs`,
новый `testbed/committee_tests.rs`, новый `e2e/src/staking_commit.rs`, `e2e/src/lib.rs`.
Остальное в `git status` (`.dpos-study/history/E4-ORCHESTRATOR.md`, `target-contract/`)
— чужое, как часть захода не читалось. Постановка просила считать чужими ещё
`contracts/staking/**`, `crates/staking-abi/src/lib.rs` и `devnet/**` — **в моём снимке
`git status` они чистые** (правки либо закоммичены, либо откачены до начала ревью), так
что «чужие правки в `staking-abi`» я проверить не мог и ни на что не опирался.

Якоря `file:line` — по ИТОГОВОМУ дереву. Агенты не запускались; git — только на чтение;
единственный записанный файл — этот. Все мутации продакшн-кода и e2e откачены, `md5sum`
сверены (§0.1, §4).

---

## §0. Прямые ответы

### 1. Ворота — прогнал сам, verbatim в §4. Все зелёные, все цифры журнала совпали. [KNOWN]

`--lib` **671/0**; `--features dpos-devnet-byzantine testbed::` **43/0**; `testbed::` без
фичи **35/0**; `fluentbase-node --lib` **57/0**; `fluentbase-staking-reader` **63/0**
(+1 ignored doctest); `fluentbase-e2e --release a_committee_is_first_committable` **1/0**;
clippy обе формы — одна чужая `large_enum_variant` в `fluentbase-node`; clippy
`fluentbase-e2e --all-targets` — ноль строк; `cargo fmt --check` — **ноль** `Diff in` по
всему воркспейсу; `cargo doc … | grep -c "unresolved link"` = **8**.

**Детерминизм — три прогона `testbed::committee_tests` (`--test-threads=1 --nocapture`).**
Совпало побайтно во всех трёх: якоря
`[[(0,0),(1,1),(2,32),(3,64),(4,95),(5,128),(6,159),(7,168)], [… (3,63),(4,96),(5,127),(6,160) …],
[… (1,2),(2,31),(3,64),(4,96) …], [… (1,2),(2,31),(3,63),(4,95),(5,127),(6,159) …]]`,
то есть страддл эпохи 3 = **64, 63, 64, 63** как в журнале; `split_anchors=[1,2,3,4,5,6]`;
`straddled=[3]`; `tree_only=[None,None,None,Some((123, 0x5ad1…))]`;
`out_of_window{above}=72`; `not_readable=3`; `errors=4`; все карты `StakingReads`
идентичны; `heights` идентичны. [KNOWN]

**Дрожание есть — ровно в одном числе.** Счётчик
`dpos_committee_read_permanent_total{reason="weights_none"}` в тесте 3: **9 / 8 / 9**.
Ассерты на нём — `>= n` и «равен полной сумме по причинам», поэтому тест не флапает, но
прогон стенда НЕ детерминирован по числу вызовов `committee(2)`, и любой будущий ассерт
на точное число там будет флапать (находка B3-04). Время: три теста серийно **37.5 с**
(журнальные «22 с» — параллельный прогон; в общий `--lib` они добавляют ~1 с, 27.55 → 28.4 с). [KNOWN]

### 2. Продакшн-код не тронут — подтверждаю. [KNOWN]

`git diff HEAD --stat -- crates/ e2e/` = ровно четыре файла захода (+ два untracked);
`git diff HEAD -- crates/dpos/consensus/src/committee` — **0 байт**;
`md5sum committee/store.rs` = `87e882e228783ddba1765ff18d4daee7` = `git show HEAD:…`;
`committee/mod.rs` = `cf571f26353e7224d83f4b0c0cc18df6` = `git show HEAD:…`.
После моих мутаций (§4) обе md5 вернулись к этим же значениям, `git status --porcelain`
побайтно совпадает с состоянием до ревью.

### 3. Старые тесты — не тронуты, и по умолчанию идут прежним путём с ОДНОЙ оговоркой. [KNOWN]

`git diff HEAD -- testbed/tests.rs testbed/preconditions.rs` — **0 байт**. По диффу
`stand.rs`/`fakes.rs` нет ни одной удалённой строки с `assert` (проверил
`git diff … | grep '^-'` целиком — удалены только доки, импорты и переставленный блок
счётчиков).

Дефолтный путь `epoch_committee_snapshot` (`fakes.rs:1209-1254`):

* `by_branch = None` ⇒ `committee()` берёт прежний `(self.members)(epoch)` (`:1108-1111`);
* `tombstoned` пуст ⇒ `validator.tombstoned = any(…) = false`, а конструктор и так ставил
  `tombstoned: false` (`:1014`) — значение побайтно прежнее;
* предикат счётчика переехал с `validators.is_some()` на `validators.is_empty()`
  (`:1228-1233`) — **эквивалентен**: `committee()` возвращает `None` для пустого состава
  (`:1123-1125`), значит `Some(v) ⟺ v` непуст;
* **изменилось** одно: `weights` теперь может быть `None` не только по явному
  `weights_none_for`, но и по кольцу — `epoch + 16 ≤ current + 2`, т.е. `epoch ≤ current − 14`
  (`:1142-1145`).

Самый длинный старый прогон: `tests.rs:2842` — `min_height_of(&[1,2,3]) >= 6 * EPOCH_LEN`
при `EPOCH_LEN = 32`, то есть **эпоха 6**; прогоны с `epoch_len = 5` (`tests.rs:485`, `:517`,
`:580`, `:637`, `:3414`) целятся в высоты 16–24, то есть эпохи ≤ 5. До эпохи 14 не доходит
ни один, поэтому арм кольца в старых прогонах не исполняется. Плюс он и не мог бы им
навредить: `weights` из снимка читает ТОЛЬКО модуль (§0.6), а окно модуля
`[current−8, current+2]` по `WINDOW_FITS_THE_WEIGHT_RING` (`committee/mod.rs:88-93`,
`14 > 8`) не достаёт до `current−14`. Это находка о ФОРМУЛИРОВКЕ журнала, не о коде
(B3-10): «поведение по умолчанию не изменилось» строго говоря неверно — оно не изменилось
*наблюдаемо*.

### 4. Тест 1: что не вакуумно, а что — да. [KNOWN]

**(а) `branch_of` (`fakes.rs:1086-1092`)** сравнивает `at` с `chain.spec_hash_at(height)` —
это КАНОНИЧЕСКАЯ карта (`fakes.rs:633-635`), единственный тир, который видит
`executed_state_hash` (`fakes.rs:611-620`). Продовый читатель модуля берёт хэш ровно
оттуда (`store.rs:472-477` через `Anchor::executed_hash` → `StandAnchor`,
`stand.rs:1930-1936`), и в проде — тоже: `executed.rs:58-73` резолвит
`provider.block_hash(height)`, то есть канон. **Вывод: чтение МОДУЛЯ не может оказаться
`Speculative` ни на стенде, ни в проде — по построению.** Значит наблюдение (б)
(`speculative.is_empty()`, `:307-314`) для модуля тавтологично; поймать оно может только
НЕ-модульного потребителя, который резолвит хэш иначе (`JumpCommittees::build_at`
`fakes.rs:1369-1387` — на произвольном прыжковом хэше; холодный старт реплея через
`FakeChain::note_hash` `:595-597`). Журнал §7.2 говорит слабее («нечувствителен к
регрессии курсора»); точная формулировка — «недостижимо для модуля» (B3-05).

Что тест доказывает СВЕРХ «фейк работает»: наблюдение (в) (`:316-332`) — якорный хэш
каждой записи равен канону этого узла на той высоте, взятому из `Outcome::hashes`, а не из
фейка; плюс посылка 3 — эпохи 1–6 реально прочитаны минимум на двух разных высотах
(измерено). Это и есть содержательная часть; ассерт равенства (а) — как честно пишет
журнал §7.1 — вакуумен относительно ВЫБОРА канонической высоты.

**(б) Посылка «спекулятивная ветка реально существовала»** проверяется по `el_events`
(`tree_only_hash`, `:132-147`): ищется `Derived(h,x)`, не канонизированный СЛЕДУЮЩИМ же
событием. Измерено: только у узла 3, `Some((123, 0x5ad1…))`. Посылка доказывает, что
хэш вне канона в дереве БЫЛ; она не доказывает, что какой-либо читатель мог на него
попасть (и, по (а), модуль не мог).

**(в) Страддл `64, 63, 64, 63`** — измеренный литерал, и посылка 4 (`:257-275`) проверяет
его явно через `anchor.0 >= TOMBSTONE_FROM`, так что сдвиг якорей даст красное с текстом
«no epoch was read on both sides of the tombstone height 64». Сообщение при этом несёт
проглоченный перенос строки («so the live␣␣␣␣␣␣␣␣␣␣flag») — B3-13.

**(г) Мутация (в) повторена мной.** `fakes.rs:1226` — веса тумбстоуненного члена
занулены в ЗНАЧЕНИИ снимка. Результат (verbatim, §4):
`nodes 0 and 1 hold DIFFERENT committee records for epoch 3 (anchors (64, 0x06ba…) and (63, 0x25ff…)) left: (…, [1,1,1], true) right: (…, [0,1,1], true)`.
То есть ловится именно то, что заявлено. Откачено, md5 `77f27e4f…`.

### 5. Тест 2 (Д-53): арифметика верна, но одно плечо пинуется не тем, чем сказано. [KNOWN]

**Арифметика.** Окно `[epoch(anchor)−8, epoch(anchor)+2]` (`store.rs:178-183`),
`commit_height(E) = start(E−2)` для `E ≥ 3`, `1` для `E ∈ {1,2}`, `0` для `E = 0`
(`committee/mod.rs:579-586`). Для внутриоконной эпохи `E ≤ epoch(h)+2` ⇒ `E−2 ≤ epoch(h)`
⇒ `start(E−2) ≤ start(epoch(h)) ≤ h`. Единственное исключение — `h = 0` с `E ∈ {1,2}`.
**Да, плечо `NotReadable{ниже commit_height}` недостижимо для внутриоконной эпохи после
заморозки геометрии, кроме якоря 0.** Вывод журнала §0.4 подтверждаю; §5.6 проекта
(`E4-CORE-DESIGN.md:581`, «`NotReadable` без EVM: границы в первые K блоков после старта,
бэкфилл (R-123)») надо уточнить так: бэкфилл даёт `OutOfWindow{above}`; `NotReadable`
остаётся за (1) якорем 0 в первые блоки, (2) `Ok(None)` от `executed_hash`
(`store.rs:466-475`) — это и есть R-123, и (3) **пустым ответом контракта на внутриоконную
эпоху** (`store.rs:486-495`) — это плечо в §5.6 не названо ВООБЩЕ, а оно единственное из
трёх, которое стоит ДВУХ staticcall'ов.

**Отсюда главное замечание.** Ассерт `not_readable > 0` (`:466-470`) читает счётчик
`dpos_committee_not_readable_total`, который инкрементируется в ЧЕТЫРЁХ местах
(`store.rs:429, 449, 470, 490`), и комментарий теста «the OTHER no-EVM arm» верен только
для `:449`. Проверил мутацией: `committee/mod.rs:582` `e if e <= MAX… => 1` → `=> 0`
(то есть арифметическое плечо для эпох 1–2 снято) — тест **прошёл** ассерты `:467` и
`:474` и упал позже, на `:485`. Значит счётчик остаётся `> 0` и без названного плеча
(B3-02). Тест в целом чувствителен — но чувствительно наблюдение (в), а не (б).

**Ещё хуже с `above > 0` (`:451-460`) — он циркулярен (B3-01).** `unseen` вычисляется
ИЗ `parked.committee_records[3]` (`:415-420`), а это ответы, снятые фазой collect
(`stand.rs:1361-1388`), которая сама вызывает `node.committee.committee(e)` для
`0..=max_epoch+2` ВНУТРИ рекордера метрик (`drain` вызывается после `run_until`, а collect
— внутри). У припаркованного узла окно `hi = 4`, цикл идёт до 7 ⇒ ровно 3 отказа
`OutOfWindow{above}` появятся всегда, даже если прогон не спросил ни одной такой эпохи.
Комментарий «…and they really HAPPENED: without this the block above is a statement about
epochs nobody ever asked for» поэтому не выполняет свою роль. Фактически прогон спросил
(72 ≫ 3), но ассерт этого не различает. Дешёвая починка: сравнивать с числом проб фазы
collect (`above > 3`) или снимать счётчик до неё.

**Посылка «узел ниже `commit_height(E)` в момент запроса»** через `Outcome` доказана
только косвенно: `parked.heights[3] == last(2) == 95` (`:399-404`) плюс текст отказа
модуля. Для плеча `commit_height` посылка НЕ доказана (оно на этом прогоне и не
срабатывает — см. выше).

**Д-54 / «один снимок на эпоху».** `module_snapshot` считается ровно в порту
`committee::EpochReads` (`fakes.rs:1185-1202`) и ни в одном другом месте — проверил, что
у `FakeStaking` два impl'а `epoch_committee_snapshot` (`:1185` — порт модуля, `:1210` —
`StakingStateRead`, общее тело), и счётчик стоит только в первом. Правильно.
Вакуумности здесь нет: мутация (б) повторена мной — снят гейт `anchor_height < ready_at`
(`store.rs:447-451`), результат verbatim:
`node 1 paid Some(2) snapshots for committee[1] it holds once: … uncommitted: {1: 2} … module_snapshot: {0:1, 1:2, 2:1, …}` — ровно предсказание постановки. Откачено,
md5 `87e882e2…`.

Оговорка к наблюдению (в): для эпохи, которую прогон НЕ спрашивал, единственный снимок
делает сама фаза collect, так что «ровно 1» там верно по построению. Содержательным ассерт
остаётся для эпох, прочитанных в прогоне; тест B это чинит фильтром `read_back`
(`:528-537`), тест A — нет (B3-08).

### 6. Тест 3: `weights` читает только модуль — подтверждаю; рекордер видит весь прогон — подтверждаю; ассерт на ERROR — ложное красное, принимается. [KNOWN]

**Кто ещё читает `weights` из снимка.** `grep -n "weights\|tombstoned"` по
`epoch_transition.rs`, `scheme.rs`, `cert_inlet.rs` — **пусто**; `application.rs` и
`slasher/tombstone.rs` упоминают `weights: None` только в своих `#[cfg(test)]` фикстурах;
`slasher/tombstone.rs:45` читает `tombstoned`, не `weights`. Единственный второй читатель
в дереве — `WeightedVrf::try_new` (`weighted_vrf.rs:86-113`), но он получает снимок из
`record.snapshot_view()` (`epoch_manager.rs:1305`, `committee/mod.rs:174-195`), то есть из
записи МОДУЛЯ, а не вторым чтением контракта. На стенде `JumpCommittees::build_at`
(`fakes.rs:1369-1387`) и `FakeStaking::dkg_qual` (`:1155-1170`) читают снимок, но `weights`
не трогают, а `StaticRandomness` берёт `all_validators_snapshot()` (`:1049-1062`) с
захардкоженными `Some(vec![1; n])`. **Д-52 верно.**

**Д-55 / рекордер.** Глобального рекордера в дереве нет, поэтому вне
`with_local_recorder` все `metrics::counter!` уходят в noop; локальный рекордер —
потоко-локальный, детерминированный раннер однопоточный (`testbed/capture.rs:6-10` говорит
то же про подписчик логов). Эмпирика подтверждает: счётчики ненулевые и стабильные
(72/3 во всех трёх прогонах). Но рекордер видит и фазу collect (см. §0.5) — это не
«весь прогон», а «прогон плюс послепрогонный опрос».

**Ассерт «ровно 4 `error!` и ни одной другой ERROR-строки»** (`:646-660`) действительно
даёт ЛОЖНОЕ КРАСНОЕ от постороннего `error!`, не ложное зелёное — приемлемо, журнал §7.4
это признаёт. Отдельно: `logs` снимается в `stand.rs:1280`, то есть ДО фазы collect
(`:1361`), поэтому ERROR, выпущенный при послепрогонном опросе, в `Outcome::errors()` не
попадёт вовсе (B3-11) — это и защищает старые тесты, и прячет диагностику.

**Мутация `store.rs` повторена мной**: `let Some(weights) = snap.weights else` →
`… snap.weights.or_else(|| Some(vec![1u128; snap.validators.len()]))`. Результат verbatim:
`the weightless epoch must not produce a record: CommitteeFacts { …, weights: [1, 1, 1, 1], changed: false, anchor: (63, 0x52c4…) }`
— совпало с журналом до хэша якоря. Откачено, md5 `87e882e2…`.

Слабое место, которого журнал не называет: `committee_verifier_epochs`
(`store.rs:585-596`) перечисляет только схемы с `me().is_none()`, то есть VERIFY-ONLY.
Ассерт `:624-627` («не построил схему») не исключил бы схему подписанта (B3-06).
И измеренное `weights_none: {2: 4}` на КАЖДОМ узле означает, что модуль перечитывает
контракт на каждый `committee(2)`: постоянный отказ не мемоизируется (B3-12).

### 7. e2e-пин: граничная половина — твёрдая, генезисная — доказывает не то, что заявлено. [KNOWN]

Контекст прочитан целиком (`e2e/src/staking_commit.rs:137-193`): `with_full_genesis`
(все генезисные контракты, `e2e/src/lib.rs:122-128`), `chain_id` пинуется под BLS-векторы,
`ROSTER = 4 = MIN_COMMITTEE_LENGTH` (`staking_protocol.rs:25`), `INTERVAL = 100`,
`ACTIVATION = 1000`, `initialize` из `GENESIS_GOVERNANCE` на блоке `ACTIVATION − 1`.
`commit()` (`:90-110`) шлёт вызов от `SYSTEM_CALLER` напрямую — контракт проверяет
`contract_caller() != SYSTEM_CALLER` (`contracts/staking/src/consensus.rs:532`), так что
e2e «обходит» пре-исполнение узла, удовлетворяя проверке буквально; узел в пине не
участвует. Ревёрт различается ПО СЕЛЕКТОРУ `EpochNotYetCommittable(uint64,uint64)` и по
обоим аргументам (`:103-109`), и любой другой ревёрт паникует — это сделано правильно и
закрывает подмену отказом по полу комитета.

Сверка с контрактом: условие — `target > current + MAX_COMMITTEE_LOOKAHEAD_EPOCHS` ⇒
`ERR_EPOCH_NOT_YET_COMMITTABLE(target, current)` (`consensus.rs:536-545`), константа
`MAX_COMMITTEE_LOOKAHEAD_EPOCHS = 2` (`staking_protocol.rs:73`); `current_epoch` считается
`math::epoch_at_block(block, activation, interval)` (`util.rs:77-85`) — **клэмп в 0 через
`saturating_sub`** (`staking_protocol.rs:169-178`), то есть до активации `current = 0`. Всё
совпадает с тем, что пишет тест.

**Граница `1299` ревёрт / `1300` проходит — доказана.** Мутация (журнальная) повторена
мной: принятый вызов перенесён на `start(C) − 1`, результат verbatim
`epoch 5 is not committable at start(3) = 1300 / left: Err((5, 2)) / right: Ok(())`.
Горизонт `+2` — тоже (`Err((C+3, C))` на том же блоке).

**Генезисная половина — нет.** Док теста (`:201-206`) утверждает: «эпохи 1 и 2
закоммичены ПЕРВЫМ исполненным блоком и ничем раньше», и журнал §0.2 повторяет это как
доказанное. Тест этого не показывает: на блоке 0 `current_epoch` клампится в тот же 0, и
тот же обход 0→1→2→отказ 3 проходит. **Проверил мутацией: заменил
`with_block_number(1)` на `with_block_number(0)` — тест ПРОШЁЛ (1/0).** То есть
генезисная половина пинует горизонт контракта при `current = 0`, а не `commit_height(E≤2) = 1`.
Сама «единица» — свойство УЗЛА (генезис не проходит пре-исполнение, бутстрап делает ровно
один `commitEpochCommittee` для эпохи 0 — `committee/mod.rs:573-577` ссылается на
`devnet/local-dpos-smoke/genesis-bootstrap/src/bootstrap.rs:399-408`), и его этот пин не
трогает. Находка B3-03. Откачено, md5 `1fce2ef9…`.

Что тест НЕ доказывает — журнальный список (а) активация 0 запрещена контрактом,
(б) `node/evm.rs` не участвует, (в) реестр статичен — подтверждаю и дополняю:
(г) «блок 1, а не 0» (выше); (д) `commit_height(0) = 0` (эпоха 0 в фикстуре коммитится
тем же обходом на блоке 1, а не генезисом, так что «закоммичена АТ genesis» не пинуется).

### 8. Формула кольца — сходится по знаку и по границе; на стенде исполнить нельзя, и это доказуемо. [KNOWN]

Фейк (`fakes.rs:1142-1145`): `None` ⟺ `epoch + WEIGHT_RING_EPOCHS ≤ current + MAX_COMMITTEE_LOOKAHEAD_EPOCHS`,
т.е. `epoch ≤ current − 14`.
Док модуля (`committee/mod.rs:43-52`): веса целы ⟺ `epoch > epoch(anchor) + MAX − WEIGHT_RING`,
т.е. `epoch > current − 14`. **Дополнительны, граница совпадает.**
`const _: () = assert!(WEIGHT_RING_EPOCHS − MAX_COMMITTEE_LOOKAHEAD_EPOCHS > SCHEME_RETENTION_EPOCHS)`
(`committee/mod.rs:88-93`, `16 − 2 = 14 > 8`) — тот же знак.
Контракт: кадр `epoch mod 16`, читатель сверяет штамп пары 0 и отдаёт `None` при
несовпадении (`contracts/staking/src/consensus.rs` `read_weights`, `ring_base` `:318-321`),
кадр переиспользуется эпохой `E+16`, а новейшая закоммиченная на высоте `h` — `current+2`
(`:536-545`). Совпадает. Одно отличие: фейк берёт «новейшая = `current+2`» безусловно, а
контракт — из `last_committed_epoch_p1`, который может отставать; фейк поэтому строго
охотнее отдаёт `None` (B3-16, инертно).

**Исполнить кольцо на стенде «по-настоящему» нельзя, и это не дефект стенда, а теорема.**
Модуль читает только `[current−8, current+2]`, а арм кольца живёт при `epoch ≤ current−14`;
`WINDOW_FITS_THE_WEIGHT_RING` — ровно утверждение, что эти множества не пересекаются.
Короткий `interval` не помогает: он двигает обе границы одинаково. Что МОЖНО сделать
дёшево и что я бы предложил вместо «исполнить»: `#[test]` на чистой арифметике —
для всех `epoch ∈ [current − SCHEME_RETENTION_EPOCHS, current + MAX_COMMITTEE_LOOKAHEAD_EPOCHS]`
проверить, что `FakeStaking::weights_at` отвечает `Some` (одна таблица, без прогона).
Это пинует согласие фейка с `WINDOW_FITS_THE_WEIGHT_RING`, чего сейчас не пинует ничто.

### 9. `Outcome::committee_records` через трейт: порядок правильный, но фаза collect протекает в метрики. [KNOWN]

Порядок зафиксирован и верен: `staking_reads` — `stand.rs:1314`, `committee_records` —
`:1369`, `metrics = ctx.encode()` — `:1395`. Поэтому послепрогонное чтение не попадает в
счётчики `StakingReads`, на которых стоят тесты. Это работает.

Чего Д-56 не называет: `logs` снимается ещё раньше (`:1280`), а **метрики `metrics::counter!`
снимаются ПОЗЖЕ фазы collect — снаружи, через `Snapshotter`**, поэтому отказы фазы
collect в них попадают (§0.5, B3-01).

Ассерты, читающие эпоху, которую прогон не спрашивал, есть: тест 1, наблюдения (а) и (в),
идут по всем эпохам `0..=7`, а эпохи 0 и 7 у всех четырёх узлов имеют ОДИН якорь
(измерено: `(0,0)` и `(168,·)`), то есть там равенство — «один хэш, один ответ» (B3-08);
тест 2, наблюдение (в) — «ровно 1 снимок» для не прочитанной прогоном эпохи верно по
построению фазы collect. Ни одно из этих мест не ложно; они просто вакуумны, и посылка 3
теста 1 (`split_anchors` непуст) спасает только «хотя бы одну» эпоху, а не все.

### 10. Граница и гигиена — чисто. [KNOWN]

Все новые типы `pub(super)` (`Branch`, `BranchCommittees`, `Tombstones`, `CommitteeFacts`,
`CommitteeRefusal`, `FakeStaking::with_schedule`); ни одного `pub` наружу крейта.
В `committee_tests.rs` и `staking_commit.rs` — ни `#[allow]`, ни `todo!`, ни
`unimplemented!`, ни `sleep`/`Instant::now`/`thread::spawn` (grep пуст). Новые `unwrap()`
в диффе — только два `self.reads.lock().unwrap()`, как во всём фейке. `StandConfig` —
три новых поля с дефолтами в `honest()` (`:227-229`), `live()` наследует через
`..Self::honest`. `Outcome` конструируется в ЕДИНСТВЕННОМ месте (`:1396`), так что новые
поля не порождают второй конструктор. Размер: `committee_tests.rs` 690 строк,
`staking_commit.rs` 306; время — 37.5 с серийно / ~1 с прироста в общем наборе.
Приемлемо.

### 11. Где журнал вводит в заблуждение.

1. **§1, «Доказательство “поведение по умолчанию не изменилось”».** Не изменилось
   НАБЛЮДАЕМО; `weights_at` добавила новый дефолтный арм (B3-10). Собственная таблица §1
   это признаёт строкой ниже, заголовок — нет.
2. **§0.1, тест 2, «`dpos_committee_not_readable_total = 3` на якоре 0».** Счётчик не
   различает четыре плеча; я снял арифметическое — счётчик остался `> 0` (B3-02).
3. **§0.1, тест 2, «`out_of_window{above} = 72` доказывает, что спрашивали».** Ассерт
   `> 0` циркулярен (B3-01); само число 72 — да, доказывает, но не ассерт.
4. **§0.2, «Генезисное исключение … ровно то, что `commit_height` кодирует как `e <= 2 => 1`».**
   Пин не отличает блок 1 от блока 0 (B3-03, мутация).
5. **§7.2** формулирует слабее, чем есть: `speculative == 0` для модуля недостижимо ПО
   ПОСТРОЕНИЮ и в проде тоже (B3-05).
6. **§0.1, md5.** `e2e/src/staking_commit.rs` записан как `b14d39ed…`, в дереве —
   `1fce2ef9…` (B3-14): чек-сумма как расписка непроверяема задним числом.
7. **§6, «3-е плечо `NotReadable` (`Ok(None)`) стенду недоступно»** — верю (аргумент
   `tip() ≤ executed_tip()` правдоподобен по `fakes.rs:623-627, 673-676`), но сам этого
   не исполнял: **[ЛИКЕЛИ]**, не [KNOWN]. И §6 не называет ЧЕТВЁРТОЕ плечо — пустой
   ответ контракта (`store.rs:486-495`), которое как раз достижимо и стоит двух вызовов.

Что неверно в проекте (`E4-CORE-DESIGN.md`): §5.6 строка «`NotReadable` без EVM: границы
в первые K блоков после старта, бэкфилл (R-123)» — бэкфилл даёт `OutOfWindow`, а в списке
плеч `NotReadable` отсутствует пустой ответ контракта, который EVM-вызова стоит. §7
«Проверка 4.1» («тест “на бэкфилле `NotReadable` без вызова `epoch_committee_snapshot`”
(`StakingReads.uncommitted` не растёт)») называет не то плечо — это подтверждено
арифметикой, а не мнением.

---

## §1. Находки

| id | серьёзность | file:lines | что не так | чем пытался опровергнуть | уверенность |
|---|---|---|---|---|---|
| B3-01 | MODERATE | `testbed/committee_tests.rs:415-420, 451-460`; `testbed/stand.rs:1361-1388` | Ассерт `above > 0` циркулярен: `unseen` берётся из `committee_records`, которые фаза collect вычисляет САМА, вызывая `committee(e)` для `0..=max_epoch+2` внутри активного рекордера метрик. У припаркованного узла окно `hi = 4`, цикл идёт до 7 ⇒ ≥ 3 отказа `OutOfWindow{above}` появятся всегда, даже если прогон не спросил ни одной такой эпохи. Комментарий «…and they really HAPPENED» не выполняет заявленной роли | Проверил, что `drain(&snap)` стоит ПОСЛЕ `with_local_recorder`, а collect — внутри `run_until`, т.е. считается; посчитал число проб collect (3) против измеренных 72 — прогон действительно спрашивал, но ассерт этого не различает; искал ассерт до collect — нет | высокая |
| B3-02 | MODERATE | `testbed/committee_tests.rs:462-470`; `committee/store.rs:429, 449, 470, 490` | `not_readable > 0` не пинует «второе плечо без EVM»: один счётчик на четыре места, из них `store.rs:486-495` (пустой ответ контракта) срабатывает ПОСЛЕ двух staticcall'ов. Опорный ассерт `epoch_one.anchor.0 >= 1` говорит лишь, что запись взята не на генезисе | Мутация `committee/mod.rs:582` `e <= 2 => 1` → `=> 0` (арифметическое плечо снято): тест прошёл `:467` и `:474` и упал на `:485`. Проверил, что плечо `store.rs:429` (нет геометрии) на стенде недостижимо — watch засеян в `stand.rs:1956` | высокая |
| B3-03 | MODERATE | `e2e/src/staking_commit.rs:201-206, 220-247` | Генезисная половина пина доказывает горизонт контракта при `current = 0`, а не `commit_height(E ≤ 2) = 1`: «единица» — свойство узла (бутстрап делает один commit на генезисе), контракт её не знает | Мутация `with_block_number(1)` → `(0)`: тест ПРОШЁЛ 1/0. Прочитал `current_epoch_at_block` (`contracts/staking/src/util.rs:77-85`) и `epoch_at_block` (`staking_protocol.rs:169-178`) — клэмп одинаков на 0 и на 1 | высокая |
| B3-04 | MODERATE | `testbed/committee_tests.rs:670-678` (наблюдаемое); стенд в целом | Прогон не детерминирован по числу вызовов `committee(2)`: `dpos_committee_read_permanent_total{reason="weights_none"}` = **9 / 8 / 9** на трёх идентичных прогонах. Сегодняшние ассерты (`>= n`, равенство полной сумме) это переживают; любой ассерт на точное число будет флапать | Три прогона `--test-threads=1`; всё остальное (якоря, `above`, `not_readable`, `errors`, карты `StakingReads`, высоты) побайтно одинаково | высокая |
| B3-05 | MINOR | `testbed/committee_tests.rs:304-314`; `testbed/fakes.rs:611-620, 633-635, 1086-1092`; `committee/store.rs:472-477`; `executed.rs:58-73` | Наблюдение (б) (`speculative.is_empty()`) недостижимо для чтения МОДУЛЯ по построению: якорный хэш всегда приходит из канонической карты — и на стенде, и в проде (`provider.block_hash`). Счётчик может поймать только не-модульного потребителя. Журнал §7.2 формулирует это слабее | Искал путь, на котором модуль получил бы не-канонический хэш: `Anchor::executed_hash` — единственный источник, единственная реализация на стенде читает `spec_hash_at`; в проде `block_hash` — тоже канон | высокая |
| B3-06 | MINOR | `testbed/committee_tests.rs:619-627`; `committee/store.rs:585-596` | «схемы тоже нет» проверяется через `committee_verifier_epochs`, а `verifier_epochs()` перечисляет ТОЛЬКО verify-only схемы (`me().is_none()`). Схема уровня подписанта для отказанной эпохи в ассерт не попала бы | Прочитал `verifier_epochs`; проверил, что `upgrade_scheme` не может создать запись без записи комитета (`store.rs:526-545`) — то есть дыра сегодня незаполнима, но ассерт её не закрывает | высокая |
| B3-07 | MINOR | `testbed/committee_tests.rs:43-50, 61-72, 187-188` | Канонический арм `branching_rotation()` обязан совпадать с `rotate_four_three_four()` член в член («the stand builds its peer-set expectations from the plain schedule»), и это записано ТОЛЬКО комментарием. Плюс `branching_rotation` захардкодил `vec![0,1,2,3]`, а плоское расписание берёт `(0..n)` — при смене `n` они разъедутся молча | Искал ассерт совпадения — посылка 1 (`:206-214`) проверяет только «канон ≠ спекуляция», не «канон = расписание стенда» | высокая |
| B3-08 | MINOR | `testbed/committee_tests.rs:287-332`, `:479-491` | Часть ассертов идёт по эпохам, у которых якорь одинаков у всех узлов (измерено: эпоха 0 — `(0,0)`, эпоха 7 — `(168,·)`), т.е. там равенство записей вакуумно; в тесте 2 «ровно 1 снимок» для не прочитанной прогоном эпохи верно по построению фазы collect. Посылка 3 спасает «хотя бы одну» эпоху, а цикл идёт по всем | Сверил измеренные якоря по трём прогонам; проверил, что тест B чинит это фильтром `read_back` (`:528-537`), а тест A — нет; журнал §7.8 признаёт общий случай, но не то, что ассерт (а) смешивает две категории | высокая |
| B3-09 | MINOR | `testbed/fakes.rs:1142` | `weights_at` возвращает «кольцо провернулось» `None` и тогда, когда `epoch_at_block` вернул `None` (нулевой интервал): две разные причины дают один ответ, и модуль интерпретирует его как «контракт отдал невозможное» | Недостижимо: `epoch_len > 0` форсируется `NonZeroU64::new(cfg.epoch_len).expect` (`stand.rs:2081`). Поэтому MINOR, а не выше | высокая |
| B3-10 | MINOR | журнал `E4-1-B3.md` §1 (заголовок «поведение по умолчанию не изменилось»); `testbed/fakes.rs:1142-1145` | Дефолтное поведение фейка ИЗМЕНИЛОСЬ: появился арм `epoch ≤ current − 14 ⇒ weights: None`, которого до захода не было. Наблюдаемо оно не изменилось | Нашёл самый длинный старый прогон (`tests.rs:2842`, `6 * EPOCH_LEN` при 32 ⇒ эпоха 6; прогоны с `epoch_len = 5` — эпохи ≤ 5): до 14 не доходит ни один; и `weights` из снимка не читает никто, кроме модуля, а окно модуля до `current−14` не достаёт | высокая |
| B3-11 | MINOR | `testbed/stand.rs:1280` против `:1361` | `logs` снимается ДО фазы collect, поэтому любой `error!`/`warn!`, выпущенный послепрогонным опросом через продакшн-трейт, невидим для `Outcome::errors()`. Это же и защищает старые тесты — но диагностика фазы collect пропадает молча | Проверил порядок в `drive`; убедился, что 4 ERROR-строки теста 3 выпущены В ПРОГОНЕ (иначе ассерт `errors().len() == n` не сошёлся бы) | высокая |
| B3-12 | MINOR | `committee/store.rs:191-207, 480-503` (продакшн); измерено `weights_none: {2: 4}` на каждом узле | Постоянный отказ чтения не мемоизируется: запись не создаётся, `reported` гасит только ЛОГ, поэтому каждый следующий `committee(E)` снова шлёт два staticcall'а. Ни один ассерт этого не пинует, и журнал этого не называет — хотя «сколько EVM-вызовов» и есть предмет 4.1 | Прочитал `failed()` и `committee()`: между `NOT_READABLE`/`READ_PERMANENT` и кэшем нет отрицательной записи; измеренное `{2: 4}` на узел это подтверждает | высокая |
| B3-13 | NIT | `testbed/committee_tests.rs:270`, `:535` | Два сообщения ассертов несут проглоченный перенос строки: «so the live␣×10 flag», «{read_back:?} of␣×10 {unseen:?}» | Прочитал обе строки целиком; `cargo fmt --check` их не ловит | высокая |
| B3-14 | NIT | журнал `E4-1-B3.md` §0.1 | Записанная md5 `e2e/src/staking_commit.rs` = `b14d39edee1674e7f5560d05be20c8f6`, в дереве `1fce2ef9fa7d22e1578a0e370c3cbaa8`. Расписка об откате мутации непроверяема задним числом (md5 `fakes.rs` и `store.rs` совпали) | Посчитал md5 сам; допускаю правку файла ПОСЛЕ мутации — но тогда расписка должна была быть пересчитана | высокая |
| B3-15 | NIT | `testbed/fakes.rs:1070` против `:1142` | `committed_at` пишет активацию литералом `0`, `weights_at` рядом — через `DPOS_ACTIVATION_BLOCK`. Два написания одной константы в одном типе | Проверил, что `DPOS_ACTIVATION_BLOCK = 0` (`fakes.rs:55`), т.е. сегодня это одно и то же | высокая |
| B3-16 | NIT | `testbed/fakes.rs:1143-1144`; `contracts/staking/src/consensus.rs:536-545` | Фейк считает новейшую закоммиченную эпоху равной `current + 2` безусловно; контракт отвечает по `last_committed_epoch_p1`, который может отставать. Фейк строго охотнее отдаёт `None` | Инертно: арм вне окна модуля (§0.8), и никто, кроме модуля, `weights` не читает | высокая |

Ни одного BLOCKER и ни одного SERIOUS. Продакшн-код побайтно равен `HEAD`; все три
стенд-теста и e2e-пин зелёные, воспроизводимые и чувствительные (каждый упал на своей
мутации, включая две мои собственные).

---

## §2. Поведение по ханкам

| ханк | file:lines | поведение |
|---|---|---|
| импорт `WEIGHT_RING_EPOCHS` | `fakes.rs:36-38` | нет |
| `Branch`, `BranchCommittees`, `Tombstones` | `fakes.rs:848-884` | новая поверхность, `pub(super)`, дефолтов не меняет |
| `StakingReads` + 4 карты | `fakes.rs:900-928` | только наблюдение; `speculative` считается ВСЕГДА (в т.ч. в старых прогонах с `resume_from` — по доку `:906-914`), но ни один старый тест его не читает |
| `FakeStaking` + 3 поля | `fakes.rs:969-986`, `:1017-1023` | `None`/`None`/пусто в `new` ⇒ прежнее поведение |
| `with_schedule` | `fakes.rs:1030-1044` | новая точка настройки; не вызывается нигде, кроме `build_node` |
| `branch_of` | `fakes.rs:1086-1092` | новый; сравнение с канонической картой |
| `committee(epoch, at, height, branch)` | `fakes.rs:1101-1129` | при `by_branch = None` и пустых тумбстоунах — побитово прежний ответ (доказательство в §0.3) |
| `weights_at` | `fakes.rs:1135-1147` | **единственный ханк, меняющий дефолт**: арм кольца. Ненаблюдаем (§0.3) |
| счётчик в порту модуля | `fakes.rs:1188-1200` | только наблюдение |
| перестановка блока счётчиков | `fakes.rs:1226-1244` | предикат `is_some()` → `!is_empty()` эквивалентен; блокировка мьютекса переехала после вычисления весов |
| `StandConfig` + 3 поля | `stand.rs:97-113`, `:227-229` | дефолты = прежнее поведение |
| `CommitteeFacts` / `CommitteeRefusal` | `stand.rs:371-396` | новые типы, `anchor` — диагностическая нога, в сравнении не участвует |
| `Outcome` + 2 поля | `stand.rs:482-500` | новое наблюдение; единственный конструктор `:1396` |
| `NodeHandles.committee` | `stand.rs:959-960` | держит уже существующий `Arc`, второго модуля не создаёт |
| фаза collect | `stand.rs:1361-1393` | **новый побочный эффект на КАЖДОМ прогоне стенда**: продакшн-`committee()` вызывается `n × (max_epoch+3)` раз после прогона. `staking_reads` и `logs` уже сняты, а метрики — ещё нет (B3-01) |
| `with_schedule` в `build_node` | `stand.rs:1648-1653` | прокидывание конфигурации |
| `mod committee_tests;` | `testbed/mod.rs:88` | новый модуль, без фичи |
| `mod staking_commit;` | `e2e/src/lib.rs:61-62` | `#[cfg(test)]`, новый модуль |

---

## §3. Граница

Наружу крейта не вышло ничего. Новые типы: `Branch`, `BranchCommittees`, `Tombstones`,
`CommitteeFacts`, `CommitteeRefusal` — все `pub(super)` внутри `testbed`. Новые методы:
`FakeStaking::with_schedule` (`pub(super)`), `FakeStaking::branch_of`/`committee`/
`weights_at` (приватные). Поля структур `pub` внутри `pub(super)`-типов — как во всём
`testbed`. `committee_tests.rs` не объявляет ни одного `pub`-элемента.
`e2e/src/staking_commit.rs` — все элементы приватные, `sol!`-тип `EpochNotYetCommittable`
объявлен локально и не попадает в `fluentbase-staking-abi` (обосновано в доке `:43-48`;
согласен — узел этот ревёрт не декодирует).

Мёртвой поверхности нет: `speculative` читает тест 1, `weights_none` — тест 3,
`tombstoned_seen` — тест 1, `module_snapshot` — тест 2, `committee_verifier_epochs` —
тест 3, `committees_by_branch`/`tombstoned` — тест 1, `weights_none_for` — тест 3.

---

## §4. Ворота (прогон ревьюера, `CARGO_BUILD_JOBS=6`), verbatim

~~~
$ cargo test -p fluentbase-consensus --lib
test result: ok. 671 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 28.39s

$ cargo test -p fluentbase-consensus --lib --features dpos-devnet-byzantine testbed::
test result: ok. 43 passed; 0 failed; 0 ignored; 0 measured; 636 filtered out; finished in 44.22s

$ cargo test -p fluentbase-consensus --lib testbed::
test result: ok. 35 passed; 0 failed; 0 ignored; 0 measured; 636 filtered out; finished in 28.82s

$ cargo test -p fluentbase-node --lib
test result: ok. 57 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 58.74s

$ cargo test -p fluentbase-staking-reader
test result: ok. 63 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
test result: ok. 0 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s

$ cargo test -p fluentbase-e2e --release a_committee_is_first_committable
test staking_commit::a_committee_is_first_committable_two_epochs_before_its_own_first_block ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 131 filtered out; finished in 0.33s

$ cargo clippy -p fluentbase-consensus -p fluentbase-node --all-targets
      1 warning: `fluentbase-node` (lib) generated 1 warning
      1 warning: `fluentbase-node` (lib test) generated 1 warning (1 duplicate)
      1 warning: large size difference between variants

$ cargo clippy -p fluentbase-consensus -p fluentbase-node --all-targets --features dpos-devnet-byzantine
      1 warning: `fluentbase-node` (lib) generated 1 warning (1 duplicate)
      1 warning: `fluentbase-node` (lib test) generated 1 warning
      1 warning: large size difference between variants

$ cargo clippy -p fluentbase-e2e --all-targets 2>&1 | grep -cE '^(warning|error)'
0

$ cargo fmt --check 2>&1 | grep -c '^Diff in'
0

$ cargo doc -p fluentbase-consensus --no-deps 2>&1 | grep -c "unresolved link"
8
~~~

### Три прогона детерминизма

~~~
$ for i in 1 2 3; cargo test -p fluentbase-consensus --lib testbed::committee_tests -- --nocapture --test-threads=1

RUN 1
(4.1/backfill) parked=[168, 168, 168, 95] unseen=[5, 6, 7] read_back=[5, 6] above=72 not_readable=3 caught_up=[168, 168, 168, 168] reads=StakingReads { committed: {0: 4, 1: 4, 2: 5, 3: 5, 4: 6, 5: 6, 6: 3}, uncommitted: {1: 1}, unknown_state: 0, speculative: {}, weights_none: {}, tombstoned_seen: {}, module_snapshot: {0: 1, 1: 1, 2: 1, 3: 1, 4: 1, 5: 1, 6: 1} }
(4.1/weights) heights=[63, 63, 63, 63] weights_none=9 errors=4 reads=StakingReads { committed: {0: 4, 1: 4, 2: 4, 3: 1}, uncommitted: {1: 1}, unknown_state: 0, speculative: {}, weights_none: {2: 4}, tombstoned_seen: {}, module_snapshot: {0: 1, 1: 1, 2: 1} }
(4.1/record) heights=[168, 168, 168, 168] anchors=[[(0, 0), (1, 1), (2, 32), (3, 64), (4, 95), (5, 128), (6, 159), (7, 168)], [(0, 0), (1, 1), (2, 32), (3, 63), (4, 96), (5, 127), (6, 160), (7, 168)], [(0, 0), (1, 2), (2, 31), (3, 64), (4, 96), (5, 127), (6, 160), (7, 168)], [(0, 0), (1, 2), (2, 31), (3, 63), (4, 95), (5, 127), (6, 159), (7, 168)]] split_anchors=[1, 2, 3, 4, 5, 6] straddled=[3] tombstoned_seen=[{2: 1, 3: 4, 4: 5, 5: 5, 6: 3}, {3: 2, 4: 5, 5: 5, 6: 3}, {2: 1, 3: 4, 4: 5, 5: 5, 6: 3}, {3: 2, 4: 6, 5: 6, 6: 3}] tree_only=[None, None, None, Some((123, 0x5ad1850b65cfc6cc5562068b22d69654214be7f2dbb5f8b7c487ae8ee5ea2ffe))]
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 668 filtered out; finished in 37.79s

RUN 2  — идентично RUN 1, КРОМЕ: weights_none=8
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 668 filtered out; finished in 37.52s

RUN 3  — идентично RUN 1 (weights_none=9)
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 668 filtered out; finished in 37.48s
~~~

### Мутации ревьюера (все откачены, md5 до = md5 после)

Бэкапы: `fakes.rs 77f27e4f3d7d42b424b200bb6fb45a47`,
`store.rs 87e882e228783ddba1765ff18d4daee7`,
`committee/mod.rs cf571f26353e7224d83f4b0c0cc18df6`,
`staking_commit.rs 1fce2ef9fa7d22e1578a0e370c3cbaa8`.

~~~
### MUT-A  fakes.rs:1226 — тумбстоун занесён в ЗНАЧЕНИЕ (вес 0 помеченному)
panicked at testbed/committee_tests.rs:293:13:
assertion `left == right` failed: nodes 0 and 1 hold DIFFERENT committee records for epoch 3 (anchors (64, 0x06ba35b1…) and (63, 0x25ff9874…))
  left: ([1fd3c299…, e695ffdb…, f1cbb2ce…], [1, 1, 1], true)
 right: ([1fd3c299…, e695ffdb…, f1cbb2ce…], [0, 1, 1], true)
test result: FAILED. 0 passed; 1 failed
→ откат: 77f27e4f3d7d42b424b200bb6fb45a47  (совпало)

### MUT-C  committee/mod.rs:582 — `e if e <= MAX_COMMITTEE_LOOKAHEAD_EPOCHS => 0` (снято арифм. плечо)
panicked at testbed/committee_tests.rs:485:13
test result: FAILED. 0 passed; 1 failed
  (ассерты :467 `not_readable > 0` и :474 ПРОШЛИ — см. B3-02)
→ откат: cf571f26353e7224d83f4b0c0cc18df6  (совпало)

### MUT-B  committee/store.rs:447-451 — снят гейт `anchor_height < ready_at`
panicked at testbed/committee_tests.rs:485:13:
node 1 paid Some(2) snapshots for committee[1] it holds once: StakingReads { committed: {0: 4, 1: 4, 2: 5, 3: 5, 4: 5, 5: 5, 6: 3}, uncommitted: {1: 2}, unknown_state: 0, speculative: {}, weights_none: {}, tombstoned_seen: {}, module_snapshot: {0: 1, 1: 2, 2: 1, 3: 1, 4: 1, 5: 1, 6: 1} }
test result: FAILED. 0 passed; 1 failed
→ откат: 87e882e228783ddba1765ff18d4daee7  (совпало)

### MUT-D  committee/store.rs — `snap.weights.or_else(|| Some(vec![1u128; snap.validators.len()]))`
panicked at testbed/committee_tests.rs:614:14:
the weightless epoch must not produce a record: CommitteeFacts { members: [11d786d2…, 1fd3c299…, e695ffdb…, f1cbb2ce…], weights: [1, 1, 1, 1], changed: false, anchor: (63, 0x52c4893b…) }
test result: FAILED. 0 passed; 1 failed
→ откат: 87e882e228783ddba1765ff18d4daee7  (совпало)

### MUT-E  e2e/src/staking_commit.rs:282 — принятый вызов перенесён на start(C) − 1
panicked at e2e/src/staking_commit.rs:283:5:
assertion `left == right` failed: epoch 5 is not committable at start(3) = 1300
  left: Err((5, 2))
 right: Ok(())
test result: FAILED. 0 passed; 1 failed
→ откат: 1fce2ef9fa7d22e1578a0e370c3cbaa8  (совпало)

### MUT-F  e2e/src/staking_commit.rs:221 — `with_block_number(1)` → `with_block_number(0)`
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 131 filtered out; finished in 0.35s
  (ЗЕЛЁНЫЙ — см. B3-03)
→ откат: 1fce2ef9fa7d22e1578a0e370c3cbaa8  (совпало)
~~~

Итоговая проверка дерева после ревью: `git status --porcelain` побайтно совпадает с
исходным (`M` на четырёх файлах захода + два `??` + два чужих), md5 всех восьми
затронутых файлов равны исходным.

---

## §5. Оставить как есть

1. **`BranchCommittees` с четвёртым параметром `Branch` (Д-51).** Замыкание строится до
   узлов и не имеет доступа к `FakeChain` читающего узла — вычислить «канон или нет» само
   оно не может. Сигнатура — надмножество заданной. Согласен.
2. **`weights: None` на уровне контракта, а не порта модуля (Д-52).** Проверено: второго
   читателя весов из снимка нет (`WeightedVrf` берёт их из записи модуля). Отдать `None`
   только модулю значило бы смоделировать состояние, которого на цепи не бывает.
3. **Отдельный счётчик `module_snapshot` (Д-54).** `committed` считает и `EpochTransition`,
   и две лишних выборки фейкового `dkg_qual`; «один снимок на эпоху» на нём невыразимо.
   Счётчик стоит ровно в порту `EpochReads` — это ровно то, что нужно.
4. **`CommitteeFacts::anchor` как диагностическая нога (Д-59).** Без неё посылка «разные
   высоты» была бы предположением. В сравнение не входит — правильно.
5. **`with_schedule` вместо расширения `new` (Д-58).** Шесть позиционных аргументов уже
   есть; три новых — путь к транспозиции.
6. **Тумбстоуны как посылка теста 1, без своего теста (Д-57).** Потребителя флага нет ни в
   модуле (решение Д-1(а)), ни на стенде; отдельный тест пинал бы фейк. Мутация MUT-A
   показывает, что ручка нагружена.
7. **Ассерт «ровно n ERROR-строк и ни одной другой».** Ложное красное, не ложное зелёное;
   для «громко и ровно один раз на узел» другого поузлового свидетельства нет.
8. **Два прогона в тесте 2 вместо одного.** «До догона» и «после» — две разные цепи;
   склеивать их в один прогон значило бы завязаться на тайминг.
9. **e2e-ревёрт, объявленный локальным `sol!` и не в `staking-abi`.** Узел его не
   декодирует; объявление в тесте делает пин конкретным по ПРИЧИНЕ отказа.
10. **Порядок фазы collect (`staking_reads` раньше `committee_records`).** Обязателен и
    соблюдён. (Дыру в метриках чинить надо не порядком, а ассертом — B3-01.)

---

## §6. Вне рамок — что нужно для закрытия 4.1, и что дальше

### Что стенд теперь РЕАЛЬНО пинует из реестра

* **R-075** (две авторитетные таблицы одного комитета) — **пинуется**: одна запись на
  эпоху у четырёх узлов на четырёх разных высотах, побайтно, плюс якорь каждой записи
  сверен с `Outcome::hashes`. Это главный результат захода.
* **R-027** (пустой комитет при чтении = `Permanent` drop улики) — **пинуется частично**:
  тест 2 показывает, что пустой/невидимый комитет даёт ТРАНЗИЕНТНЫЙ отказ
  (`is_err_and(|r| r.transient)`, `:441-447`), т.е. улика не дропается. Слэшер в цепочку
  не включён — до его перевода на модуль это свидетельство о модуле, не о слэшере.
* **R-123** (заголовок есть, состояния нет) — **НЕ пинуется**. Плечо `Ok(None)` от
  `executed_hash` стенду недоступно (`StandAnchor::height() = chain.tip()` не обгоняет
  `executed_tip()`), и заход это честно пишет. Единственный свидетель — юниты
  `committee::tests`.
* **R-040** (by-height fetch не связывает эпоху раунда с высотой) — **не пинуется** этим
  заходом; предмет 4.2 (`deliver`).
* **R-019** (`MAX_COMMITTEE_SIZE` проверяется один раз при старте) — **не пинуется**:
  состав на стенде не растёт; ветвящийся фейк это теперь УМЕЕТ (спекулятивный арм может
  вернуть состав другого размера), но ни один тест этого не делает.
* **R-076** (эпоха без схемы у follower'а) — **не пинуется**: follower на стенде нет.

### Для закрытия 4.1 (по убыванию цены/пользы)

1. Починить B3-01 и B3-02 — два ассерта, которые сегодня не пинуют то, что обещают в
   собственных комментариях. Стоимость — по строке каждый.
2. Добавить арифметический `#[test]` согласия `FakeStaking::weights_at` с
   `WINDOW_FITS_THE_WEIGHT_RING` (§0.8): единственный способ «исполнить» кольцо, не врущий
   про то, что он исполняет.
3. Переписать генезисную половину e2e-пина или её док (B3-03): либо признать, что пинуется
   горизонт при `current = 0`, либо добавить пин на узловой стороне (бутстрап + первый
   исполненный блок), где «единица» вообще живёт.
4. Ассерт совпадения двух расписаний теста 1 (B3-07) — 4 строки, снимает целый класс
   молчаливого разъезда.
5. Уточнить §5.6 проекта: перечислить ЧЕТЫРЕ плеча `NotReadable`, из которых одно стоит
   двух staticcall'ов (§0.5).

### Для 4.2

* `speculative` — готовый детектор, но сегодня недостижимый по построению (B3-05).
  Чтобы он что-то ловил, стенду нужен ИСТОЧНИК состояния, не совпадающий с локальной
  цепью (роль-лжец на `Latest`/`Finalized`), а не другой курсор. Это и есть 4.0(а)/R-001
  вариант А, и ветвящийся фейк под него уже готов — не хватает только роли.
* `module_snapshot` даёт прямое измерение «лишних чтений не стало» после удаления
  `CommitteeSource`/`LiveFrontierTee`.
* Фаза collect (`stand.rs:1361-1388`) теперь вызывает продакшн-`committee()` на КАЖДОМ
  прогоне стенда. Перед 4.2 стоит решить, считается ли это частью прогона: сегодня
  `staking_reads` и `logs` от неё защищены, а метрики — нет.

### Для 4.3

* `dpos_committee_out_of_window_total{above} = 72` за один прогон на одном отставшем узле
  — оценка снизу на поток отказов, который маска `Ingress` будет производить. Но из этих
  72 три приходят от фазы collect, а не от прогона; для оценки брать 69.
* Тумбстоун-ручка (`StandConfig::tombstoned`) даёт живой флаг без потребителя. Когда 4.3
  заводит `TombstoneSet`-поллер, у неё появляется первый настоящий читатель — тест 1 можно
  будет расширить с «запись не изменилась» до «связь разорвана, а запись не изменилась».
