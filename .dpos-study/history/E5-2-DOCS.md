# Э5 строка 5.2, Ф7 — доковый проход по `.claude/dpos_architecture/`

База `HEAD` `37691136`, ветка `djadjka/dpos-reth-2.2-squashed`, рабочее дерево строки 5.2
(не закоммичено). **Ни одной строки продакшн-кода или тестов не изменено.** Записаны только
`.claude/dpos_architecture/*.md` и этот файл. `git` использовался только на чтение
(`log`, `status`, `diff`, `show`, `merge-base`, `grep HEAD`).

Что читал в коде (это и есть источник каждой правки, не комментарии):
`beacon/seed_index.rs` целиком до тестов, `beacon/mod.rs`, `beacon/surface.rs` (трейты,
`certificate_verdict`, `LateFaults`, `impl Randomness for LiveBeacon`, `Absent`, `Canned`),
`beacon/plane.rs` (валидаторный мост `:930-1010`, весь follower-блок `:1014-1300`),
`beacon/metrics.rs`, `beacon/seed_journal.rs` (модульный док + `open`), `spec_exec.rs`,
`cert_inlet.rs` (`with_randomness`, `ingest`, `drain_late_verdicts`, `record_data_fault`,
`UpstreamResolver::spawn_finalized`), `dpos.rs` (`ReplaySeed*`/`seed_via_beacon`, места
`OuterBuilder`, `re_jump_threshold`), `executor.rs` (диффы + `beacon_over`),
`testbed/byzantine_roles.rs` (`impl Beacon`), `testbed/preconditions.rs:295-320`,
commonware `consensus/src/types.rs:405-425` (порядок `Round` — он несущий для
`take_while`/`peek` в `seed_index.rs`).

---

## §0 Инвентарь: сколько было, сколько осталось, почему

Команда (verbatim; `grep -rn`, **не** `git grep` — каталог в `.gitignore`):

```
cd .claude/dpos_architecture && grep -rnE 'SeedStore|quarantin|Quarantin|follower\.rs|observe_cert|observe_epoch|FollowerRandomness|certify\.rs|for_seeds|pin_terminal|terminal_at|promote_epoch|waiters|wait_for|on_invalid_seed|retain_terminal_from|KeyOnlyOracle|retain_quarantine_from|beacon::certify|quarantined_epochs|terminal_seed_at|promote_quarantined|seed_promoter' --include='*.md' .
```

**Было 189 попаданий в 11 файлах** (инвентарь оркестратора говорил 179 в 10; разница — не
дрейф, а шаблон: у меня в него попадают `04_cold_start…md` и лишние `wait_for`/`waiters`).
**Стало 143.** Из них:

| категория | шт. | почему осталось |
|---|---|---|
| ложные срабатывания шаблона | 31 | `observe_certificate` (живой метод), `terminal_at_or_below` (`epocher`, другой символ), `wait_for_activation_block`, «ack waiters» в `plane_upstream` |
| исторический лог `verified-against` (записи старше 5.2) | ~34 | это **датированный журнал**, а не описание текущего состояния. Правка записи 5.0/5.1/FLU-1203 стёрла бы историю; §1 объясняет решение |
| моя собственная новая запись 5.2 в `00_preamble.md` | 14 | называют удалённые символы **как удалённые** — это и есть содержание записи |
| живой текст секций 01-15 | 64 | из них все до одного — мои новые формулировки вида «`pin_terminal` больше нет», «`beacon/follower.rs` DELETED», «`observe_cert` — пустой дефолт», плюс два исторических пояснения в `02:283-284` про удалённый `keys.rs::on_invalid_seed` |

**Ни одного попадания, которое утверждало бы существующим то, чего в коде нет, не осталось.**

Отдельно: числа по файлам «было → стало» (по тому же шаблону):
`09` 57→29, `00_preamble` 37→51 (вырос на мою запись), `08` 34→23, `13` 24→16, `02` 14→6,
`03` 11→6, `12` 3→2, `06` 3→2, `15` 2→4 (вырос на блок про девнетный дрейф), `04` 2→3,
`01` 2→1.

---

## §1 Что переписано

### `09_followers.md` — переписан беконный половина §9.1, §9.4 gate 2

**Границы решения.** Постановка говорит «переписывается целиком». Файл — 1686 строк, и
§9.2 (cold start), §9.3 (L1 trust root), §9.5 (транспорт/serving), §9.6 (`UpstreamResolver`),
§9.6.1 (`CertUpstream`/frontier), §9.6.2 (`SafetyHalt`) — **не про бекон вообще**: там не было
ни одного попадания инвентаря и ни одного утверждения, которое строка 5.2 меняет. Переписан
весь блок, который был построен вокруг `beacon/follower.rs`: §9.1 строки ~40-370 и две точки
в §9.4. Это записано и в новой записи `verified-against`, чтобы следующий проход не искал
несуществующую правку в §9.6.

Подразделы:

1. **Абзац FLU-1204 «follower is SEEDLESS»** — `SeedStore` → `SeedIndex`; ссылка на
   `SeedStore::promote_epoch` (`beacon/certify.rs:289`) заменена на «вердикт — одно тело
   `surface::certificate_verdict` (`beacon/surface.rs:415`), через которое ходит каждый
   `observe_certificate`» + `SeedIndex::settle_epoch` (`:321`); проставлены реальные якоря
   двух дверей `cert_inlet.rs:735` / `:3030` (старые `:3013`/`:3026` дрейфанули).
2. **Абзац FLU-1203 «σ не останавливается у двери»** — «served map / quarantine» заменено на
   одну карту с двумя состояниями `Entry::{Verified, Pending}` + вердикт
   `Observed::{Recorded, Pending, Refused, Inactive}`.
3. **Блок FLU-1167 (ключ у follower-а)** — **переписан**: `beacon/follower.rs` удалён,
   `build_follower`/`FollowerInputs`/`ArtifactFetch`/`run_fetcher` живут в `beacon/plane.rs`,
   follower собирает **тот же** `LiveBeacon`, отличаясь ровно тремя аргументами конструктора
   (пустой `CeremonyStore`, RAM-only сторы, замороженная geometry). Добавлено: в крейте ровно
   один `impl Randomness` — проверено командой `git grep 'impl Randomness for' -- crates`,
   один результат (`beacon/surface.rs:2194`).
4. **KEY WANT** — переписан: больше не едет на `observe_cert` (пустой дефолт без
   продакшн-вызывающих), а поднимается самим вердиктом `Pending` в
   `Randomness::hold_seed` (`beacon/surface.rs:2215`).
5. **σ-блок и «карантин»** — переписан целиком (см. §2 про то, что выброшено).
6. **Таблица «класс узла × impl Beacon»** — две продакшн-строки схлопнуты в одну;
   `impl Beacon`-блоков по-прежнему пять, якоря перепинены (`:495`, `:881`, `:1038`, `:1169`,
   `byzantine_roles.rs:367`); добавлена строка `WithholdingRandomness` (делегирует — прочитал
   тело); уточнена ловушка `Canned` (его `terminal_seed` читает **реальный** индекс, а `seed` —
   канонную карту, `beacon/surface.rs:1183`).
7. **Абзац `for_seeds`** — переписан: `for_seeds` удалён, замена —
   приватный `executor.rs:4266::beacon_over` поверх `keyless_index()`; добавлено, что
   единственное выживание старого имени — тест-алиас `beacon/mod.rs:170`.
8. **§9.4 gate 2** — два места: «`ensure_key` — синхронное чтение одного владельца
   (`beacon/follower.rs:428-440` на follower-е)» → одно тело `LiveBeacon::ensure_key`
   (`beacon/surface.rs:2467`); и абзац «общий провайдер нужен, чтобы `observe_cert` подрезал
   тот же стор» — **аргумент снят** и заменён двумя новыми причинами (общий индекс σ + канал
   `faults()`, который отдаётся ровно один раз).

**Что выброшено из `09` и почему** (полный список):

- **Абзац «THE TWO DOORS ARE NOT SYMMETRIC ON PRUNING»** — выброшен. Он был верен, пока
  вытеснение висело на `observe_cert`, которого дверь by-height не звала. Теперь `evict`
  (`beacon/seed_index.rs:393`) выполняется внутри `admit` на **каждой** вставке из **любой**
  двери. Вместо него записана асимметрия, которая осталась настоящей: у двери by-height нет
  рычага `RotateUpstream` (§9.1 → правило 34d).
- **Абзац «Retention moved with it: both seams now prune THREE maps»** (`retain_from`,
  `retain_quarantine_from`, `retain_terminal_from`, два драйвера) — выброшен целиком:
  ни одной из этих функций нет, и **шва** нет тоже. Заменён описанием правила вытеснения из
  трёх частей с явными ценами (пик `2 · SEED_RETENTION`; экзёмпция только для `Verified`;
  ретайр «закрытой» эпохи целиком).
- **Фраза «терминальные пины не имеют счётного ограничения вообще, одна запись на эпоху на
  всю жизнь процесса»** — выброшена: карты пинов нет, а экзёмпция ограничена окном
  `SCHEME_RETENTION_EPOCHS`, то есть память **ограничена**, чего старый текст не обещал.
- **Строка таблицы `FollowerRandomness`** — выброшена (тип удалён).
- **Утверждение «`ensure_key` отвечает одинаково на ОБОИХ effort-ах на follower-е»** —
  выброшено как **ошибка документа**, а не как устаревание: после слияния
  `build_resolved` кладёт в слот `acquire` тот же `TransportAcquire`, что и валидаторная
  плоскость, так что `Thorough` тратит один ограниченный фетч. Это явно помечено как
  исправление текста строки 5.1.

### `08_node_integration…md`

- Карта файлов: `certify.rs` → **DELETED**, преемник `seed_index.rs`; `follower.rs` →
  **DELETED**, куда именно переехали четыре символа.
- Таблица поверхности `Beacon`: строка `retention` переписана (пустые дефолты, у
  `observe_cert` нет вызывающих, удаление — за строкой 5.4).
- Абзац про `Randomness` под дверью: «две продакшн-реализации» → одна, с командой проверки.
- `Tasks.supervised`: **пять** детей, а не шесть — перечитал `spawn_supervisor`
  (`beacon/plane.rs:998-1006`): `dkg`, `beacon_resolver`, `agreement_launcher`,
  `agreement_write_back`, `event_bridge`. `seed_promoter` удалён.
- `for_seeds` → удалён; тест-дверь и алиас названы.
- **§8.11.2 переписан целиком** (заголовок тоже): был «`certify.rs` is now the `SeedStore`» —
  стал «σ-индекс это `beacon/seed_index.rs`». Внутри: два состояния, где были три карты;
  правило вытеснения вместо пина; четыре (а не три) синхронных читателя — добавлен
  crash-replay; вердикт как одно тело; `broadcast` вместо `notify_one`; и явно записано,
  что модульный док `seed_journal.rs` всё ещё несёт **устаревшее** обоснование (см. §3).
- Абзац про «churn > f»: «квaрантин» → `Entry::Pending`, «не может прочитать terminal pin» →
  «не находит σ на терминальном раунде `E-1`».
- `beacon::follower::changed_bit` → `beacon::plane::changed_bit` (`beacon/plane.rs:1231`),
  три места.

### `13_invariants_gotchas_rules.md` — см. §2 (там решения по инвариантам)

### `02_ordering_core…md`

- Три `SeedStore` → `SeedIndex`; «терминальный пин (`certify.rs::pin_terminal`)» → экзёмпция
  вытеснения (`seed_index.rs::oldest_evictable`) в двух местах (эпитафия propose-пути и
  абзац про удалённый by-round протокол).
- Пункт 3 эпитафии vote-ladder: «`NoKey` quarantines / speculation refuses without
  quarantining» → «`NoKey` HOLDS as `Entry::Pending` / speculation refuses without holding»,
  якоря вердикта перепинены на `beacon/surface.rs:415-492`.
- `02:283-284` (`keys.rs::on_invalid_seed`) **оставлено**: это описание удалённого файла в
  явно историческом абзаце «what the three states encoded».

### `03_epoch_machinery.md`

- «Проход не бесплатный: … `observe_epoch` re-attempts the key backfill» — **неверно**:
  `observe_epoch` теперь пустой дефолт и не стоит ничего. Переписано.
- §3.2 W1/W3: `Randomness::observe_epoch` больше нет в реализациях вообще.
- Абзац про edge `ArtifactStore::subscribe`: «два потребителя» → **один на класс узла**, и
  записано, почему это и есть суть (гонка `seed_promoter` vs мост).
- Список потребителей `SCHEME_RETENTION_EPOCHS`: `beacon/follower.rs` → `beacon/seed_index.rs`.
- Блок follower-а (`beacon::build_follower`): переписан на `beacon/plane.rs:1121`.
- Закрытый вопрос про boundary-σ: «`SeedStore::insert` пишет терминальный PIN из того же
  вызова» → правило вытеснения.

### `04_cold_start_restart…md` — **новый абзац**, не только переименование

Ходовая часть строки 5.2, которой в доке не было вообще: ступени (2) и (3) crash-replay больше
не читают σ из сертификата напрямую, а отдают финализацию в `Beacon::observe_certificate` и
действуют по вердикту — `Pending ⇒ ReplaySeed::Defer` (и **никогда** не фатально), `Refused`/
`Inactive ⇒ Absent`. Переписан и абзац «trust classes»: локальный сертификат теперь
**проверяется**, а без-проверочный класс сузился до индекса σ и тел блоков. Заодно записано,
что док-ссылка `dpos.rs:773` всё ещё зовёт `seed_from_cert` (см. §3).

### `12_consensus_critical_constants.md`

Строка `SEED_RETENTION`: якорь `beacon/certify.rs:40` → `beacon/seed_index.rs:94`; добавлено
главное, чего в таблице констант не было — **это бюджет НА СОСТОЯНИЕ**, и пиковая память
`2 · SEED_RETENTION`. Строка `SEED_PULL_TIMEOUT` (удалённая константа) — только про пин.

### `01_system_map.md`, `06_staking_layer.md`

`01`: `SeedStore` → `SeedIndex`; `terminal_seed` как дефолт; список удалённого дополнен
(`for_seeds`, `certify.rs`, `follower.rs`, «ровно один `impl Randomness`»).
`06`: `beacon::follower::changed_bit` → `beacon::plane::changed_bit`.
`06:603-605` (`terminal_at_or_below`) — **оставлено**: это метод `epocher`, не бекона.

### `15_smoke_cases…md` — см. §4

### `00_preamble.md`

Добавлена запись `verified-against` за 2026-09-13 в форме 5.0а/5.1: удалённые файлы со
строчными объёмами из `git diff --stat`, что стало на их месте с якорями, три продакшн-сайта
чтения вердикта, поздняя половина, `observe_epoch`/`observe_cert`, отдельным абзацем —
**R-016 не сделан и откачен намеренно** (`JUMP_THRESHOLD.min(interval)`, `dpos.rs:2483`,
`:3244`, свидетельство `testbed/preconditions.rs:303-315`), ворота тега `c2`, список
затронутых секций и примечание про границы правки `09`.

**Исторические записи лога не тронуты.** Это решение, а не пропуск: блок `verified-against` —
датированный журнал «что было верно на момент строки», и переписывание записи FLU-1203 под
сегодняшний код превратило бы его в фикцию, которая сама себя заверяет. Ровно этот режим отказа
уже записан в памяти проекта («doc-layers fail to fiction»).

---

## §2 Решения по инвариантам `13_…md`

### Правило 30 (HOLD в `awaiting_seed`) — **держится, уточнён механизм**

«ONLY exit is the seed-record notify (`SeedStore`'s per-record `Notify`)». Инвариант стоит:
единственный выход по-прежнему ровно один. Но механизм назван неверно — это `broadcast`
`BeaconEvent::SeedRecorded`, а не `Notify`, и так было уже с 5.0. Переписал с пометкой, что
часть комментариев в `executor.rs` до сих пор говорит `Notify` (§3).

### Правило 34, хвост про SEAMS — **снят**

Было: «что осталось от старых швов ретенции — это σ-половина: `LiveBeacon::observe_epoch` и
`FollowerRandomness::retain_from`, и два их драйвера снаружи бекона неизменны».
**Не держится ни в какой форме.** Ни одной из двух функций нет; `Beacon::observe_epoch` и
`observe_cert` — пустые дефолты без реализаций, у `observe_cert` нет ни одного продакшн-
вызывающего. Переписано в «швов не осталось», с явным запретом «не подключайте новый вызов ни
к одному из двух» и с указанием, кто их ещё называет (`epoch_manager.rs:1225`,
`testbed/byzantine_roles.rs:400` — оба вне списка записи 5.2).

### Правило 34b («непроверенная σ не может дойти до `seed` и не выразима») — **держится, ПЕРЕПИСАНО**

Само правило стоит и стало **сильнее**, но его обоснование было полностью про две карты:
«`NoKey` идёт в ОТДЕЛЬНУЮ карантинную карту — не флаг на общей, потому что `record` —
last-wins и флаг оставил бы непроверенное значение структурно способным перезаписать
проверенное». Карты больше нет; свойство несёт машина состояний `SeedIndex::admit`
(`:213`). Переписал полностью:

- одна карта, два состояния; кто кого не может перезаписать и почему (плюс новая ветка:
  два **различающихся** `Verified` на один раунд отвергаются и логируются — это то, на чём
  стоит fork-safety правила 28);
- settle: **один** потребитель на класс узла, оба зовут `settle_pending` **перед** публикацией
  `KeyAvailable`; отдельным предложением — «отдельная задача `seed_promoter` удалена и не
  должна вернуться», с описанием гонки, которая была;
- ретенция: правило из трёх частей вместо «счётный бэкап эпохного окна», каждая часть с тем
  свойством, которое иначе молча теряется;
- **новый абзац «двух вещей экзёмпция НЕ делает»**: она не решает, какой раунд отвечается
  (старый `terminal_at` отвечал только на пин и отказывал соседям — это свойство **потеряно**,
  и я записал, почему безопасность от этого не страдает: единственный вызывающий берёт раунд
  из согласованного терминального БЛОКА, `epoch_manager.rs:159-163`), и она не **называет**
  терминальный раунд (жёсткий kill между вставкой и sync-ом журнала).

### Правило 34c («σ приезжает на сертификате или никак») — **держится, одна оговорка снята**

Оговорка «Only ONE of the two doors PRUNES» **снята** — см. §1, выброшенное. Заменена на
асимметрию, которая осталась (рычаг ротации), со ссылкой на новое 34d. Пункт (b) «эпоха, чей
движок не стартует, потому что `boundary_base` читает пин, а пин пишется этим же захватом
(`… → SeedStore::insert → pin_terminal`)» переписан на цепочку через `Entry::Verified` и
защиту вытеснением.

### Правило 34d — **новое**

Позднюю половину вердикта (`LateFaults` → `Beacon::faults()` → дренаж в голове
`CertInlet::ingest` → `record_data_fault`) в `13` не описывал никто. Записал как правило, с
двумя асимметриями, которые легко потерять: заряд делается **до** суждения об этом
сертификате и переживает его (иначе чистый сертификат стирал бы обвинение, которое сам
принёс — C-07); и у двери by-height рычага ротации нет вообще, это **записанный остаток**.

### Правило 34 (ключевая часть) и правило 35 — **не трогал**, к строке 5.2 отношения не имеют.

---

## §3 Дефекты В КОДЕ, которые я нашёл, а ревью пропустило

Не правил ничего. Все якоря — из файлов, которые открывал в этой сессии.

### 3.1 `beacon/mod.rs:36-41` — модульный док утверждает ровно то, что строка 5.2 отменила

```
//! Four of the vocabulary names — [`WithheldReason`], [`PinEffort`],
//! [`Observed`], [`DataFault`] — are here because a signature above names them,
//! not because anything outside reads them yet: ... PLAN rows 5.1-5.2 give
//! the last two their readers.
```

Строка 5.2 **и есть** та строка, которая дала им читателей: `Observed` читается на трёх
продакшн-сайтах (`spec_exec.rs:123`, `cert_inlet.rs:736`, `cert_inlet.rs:3032`) и в
`dpos.rs:749`, а `DataFault` — в `cert_inlet.rs` (`self.faults`, поля `epoch`/`refused` в
`drain_late_verdicts`). Док остался в будущем времени и теперь говорит неправду о двух из
четырёх имён. Файл — внутри списка записи строки 5.2. **Почему дефект:** это единственный
текст, который объясняет, зачем каждое имя стоит в `pub use`; читатель, доверившийся ему,
удалит `Observed`/`DataFault` из поверхности как «ещё никем не читаемые».

### 3.2 `beacon/mod.rs:44-47` — «две ПРОДАКШН-реализации» там, где осталась одна

```
//! ... the module-internal [`surface::Randomness`] trait the two PRODUCTION
//! implementations still speak while the internals move
```

`git grep 'impl Randomness for' -- crates` даёт **одну** строку —
`beacon/surface.rs:2194` (`LiveBeacon`). Удаление второй реализации — главный результат этой
строки, и он противоречит тексту в том же файле, который строка правила. **Почему дефект:**
дословно тот же класс, что 3.1, но хуже: это утверждение о количестве, и оно опровергается
однострочным грепом, который сама постановка строки 5.2 предписывает выполнять.

### 3.3 `beacon/seed_journal.rs:7-12` — устаревшее обоснование ORDERING-CRITICAL, которое соседний файл уже переписал

```
//! `SeedIndex::record` is ORDERING-CRITICAL (`crate::spec_exec`): it must stay
//! synchronous and precede the executor send, because the certify gate's
//! `false`-on-missing-seed verdict is cross-node deterministic only if every
//! honest node has recorded the round before its own `certify` scan reaches it.
```

Certify-гейта нет, и `spec_exec.rs:56-84` **явно** это фиксирует: «Re-derived from scratch,
because the justification this comment used to carry was stale: it cited
`certify.rs seed_certify_verdict`, a function deleted with the certify gate», после чего
выводит настоящее обоснование в трёх шагах (voter awaits `report()` inline; исполнитель
деривит из σ того же раунда; промах — это HOLD, а не неверный derive). Строка 5.2 **открывала
этот файл** и переименовала в нём `SeedStore` → `SeedIndex` на строке 7 — то есть отредактировала
предложение и оставила в нём мёртвое обоснование. **Почему дефект:** два файла в одном крейте
дают взаимоисключающие причины для одного и того же контракта, и устаревшая из них — та,
которая говорит «precede the executor send», а переписанная явно проверила и **отвергла** эту
половину («That half is not load-bearing»).

### 3.4 `beacon/log_resolver.rs:128-131` — комментарий обещает запись, которой больше нет

```
/// more: σ rides the certificate, `capture_certificate_seed` files it at the
/// certificate's own round on BOTH cert doors, and `SeedStore::insert` writes
/// the per-epoch terminal pin from that same call — so a node that pulls a
/// boundary finalization obtains exactly the σ this request existed to fetch.
```

Обоснование, почему тег `TAG_SEED_RETIRED = 2` можно держать отставным, опирается на
«`SeedStore::insert` пишет per-epoch terminal pin из того же вызова». На `HEAD` это было
правдой (`certify.rs:201-203`: `insert` звал `pin_terminal`). Сейчас нет ни `insert`, ни
`pin_terminal`, ни карты пинов; выживание терминального раунда обеспечивает **правило
вытеснения**, и оно ограничено окном `SCHEME_RETENTION_EPOCHS`, чего пин не был. Заодно
`capture_certificate_seed` не существует в дереве вовсе — но это **предсуществующий** дрейф,
он уже был на `HEAD` (проверил `git grep -n 'capture_certificate_seed' HEAD -- crates`: одна
строка, этот же комментарий). **Почему дефект:** вывод («узел, тянущий boundary-финализацию,
получает ровно ту σ, ради которой существовал запрос, пин включительно») теперь держится по
другой и **более слабой** причине, а это единственное место, где обоснован отказ от
переиспользования проводного тега.

### 3.5 `executor.rs:9475-9476` — ссылка на удалённый файл и переименованный тест

```
// the arm's WAKEUP (no lost notification) is covered by certify.rs's
// `seed_store_record_notifies_without_a_lost_wakeup`.
```

Файла `beacon/certify.rs` нет; теста с таким именем нет. Преемник —
`beacon/seed_index.rs:634::seed_index_record_notifies_without_a_lost_wakeup` (на `HEAD` тест
звался как в комментарии — проверил `git show HEAD:…/certify.rs | grep`, строка 567). **Почему
дефект:** это единственная запись о том, **где** покрыта половина свойства, которую этот тест
намеренно не покрывает («drives the arm's BODY directly»); потерянная ссылка означает, что
удаление покрытия пройдёт незамеченным.

### 3.6 `executor.rs:1689-1690`, `:2479-2482`, `:5219` — механизм назван неверно в строках, которые строка 5.2 редактировала

Все три говорят «`Notify`» / «per-record `Notify`» про то, что на самом деле —
`broadcast::Sender<BeaconEvent>` с вариантом `SeedRecorded` (`beacon/seed_index.rs:132`,
`:256`). Сам дрейф **предсуществует** строке 5.2 (broadcast появился в строке 5.0), но эти
строки — в диффе строки 5.2: в них менялось `SeedStore` → `SeedIndex` **в том же
предложении**. **Почему дефект:** «`notify_one`-пермит переживает отсутствие ждущего» и
«broadcast буферизует от момента подписки» — это **разные** контракты, и второй требует
подписаться до первого чтения. Комментарий, обещающий первый, — это инструкция написать
код с потерянным пробуждением. Ровно эту ловушку `beacon/surface.rs:236-243` описывает как
то, «что раньше было бесплатно, а теперь должно быть организовано».

### 3.7 `dpos.rs:773` — док-ссылка на несуществующий символ

```
/// cold-start jump landing. Every source is round-pinned by [`seed_from_cert`],
```

Функция называется `seed_via_beacon` (`dpos.rs:749`). `seed_from_cert` в дереве больше нет
(второе вхождение, `dpos.rs:4689`, корректно — оно говорит «предшественник»). **Почему
дефект:** `[[…]]` — интра-док-ссылка, то есть это ещё и нерезолвящаяся ссылка; и это
единственное место, где названо, **что именно** обеспечивает round-pinning всех четырёх
источников replay-обхода.

### 3.8 `beacon/plane.rs:1041` — буллет отрицает поле, которое конструктор ставит намеренно

```
//   * no DKG, no agreement plane, no muxes, no geometry watch — the `Withheld`
//     verdicts follow from the empty share store rather than from a second type.
```

`build_resolved` (`beacon/plane.rs:1200`) передаёт `geometry: watch::channel(Some((0, 1))).1`,
и в том же месте объясняет, что это **несущее** решение: follower не замораживает своей
геометрии, поэтому уточнение `GeometryUnfrozen` в `share_probe` не должно срабатывать.
Последствие ошибки ограничено (метка `WithheldReason`, не поведение: оба пути дают
`Withheld`), поэтому это нит, а не баг. **Почему всё-таки дефект:** буллет — это список
«чего у follower-а нет», и читатель, снявший «geometry watch» по его указанию, поменяет
причину отказа на ту, которую соседний комментарий называет «ложной историей для узла,
который держит mint и потерял share».

### 3.9 `beacon/mod.rs:177` — «below it» про то, что выше

```
/// a rename of anything above — unlike the `SeedStore` alias below it.
```

Алиас `pub(crate) use super::seed_index::SeedIndex as SeedStore;` — строка **170**, то есть
**выше** комментария (строки 172-178). Чистый нит, но в файле, который сама строка 5.2
переписывала.

### 3.10 Не дефект, но проверено и записано здесь, чтобы не перепроверяли

- **Мёртвых счётчиков нет.** Прогнал все 37 полей `Counter`/`Gauge`/`Family` в крейте против
  сайтов инкремента — каждое имеет не-декларативные употребления. Новый
  `seed_verify_invalid` инкрементится в `beacon/oracle.rs:226` (у оракула, т.е. покрывает обе
  половины Д-3) и читается тестом `beacon/plane.rs:2033`. Это отличие от строки 5.1, где
  доковый проход нашёл `epoch_engine_demoted_key_divergence_total` с регистрацией без `inc`.
- **`seed_events` — тот же broadcast, что и у индекса** (`beacon/plane.rs:935`:
  `seed_store.events().clone()`), так что `KeyAvailable` доходит до подписчиков
  `Beacon::subscribe()`. Проверял специально: разные каналы здесь были бы молчаливой потерей
  класса пробуждений.
- **`wire_want` зовётся ровно один раз** (`beacon/plane.rs:1206`, follower), у валидатора
  `want` не подключён — и это задокументировано в коде (`plane.rs:1034`), а не случайность.
- **Порядок `Round` — `(epoch, view)`** (commonware `consensus/src/types.rs:409-414`,
  derive `Ord` по полям в этом порядке). Это несущее допущение для `take_while` в
  `retire_closed_pending_epochs` и для `peek`-логики в `oldest_evictable`; в коде оно
  утверждается комментарием, я проверил его в чекауте.
- **R-016 действительно откачен**: `crate::cold_start_jump::JUMP_THRESHOLD.min(interval)` в
  обоих путях запуска (`dpos.rs:2483`, `:3244`), свидетельство в
  `testbed/preconditions.rs:303-315` («A gate at or above `2·interval` therefore wedges an
  execution-stalled node PERMANENTLY after §5.2. Production cannot configure one»). Ни одна
  секция дока абсолютного порога не обещает — проверил грепом по `JUMP_THRESHOLD`.

---

## §4 Расхождения документа с девнетом

### 4.1 `verdicts_follow.py` ждёт строку лога, которой код больше не пишет — **ПОДТВЕРЖДЕНО, и предсуществует `HEAD`**

- ожидание: `devnet/local-dpos-smoke/dpos_harness/cases/smoke/verdicts_follow.py:237`
  `CF_KEY_LINE = "cert-follow: PK_epoch obtained and verified against committee[epoch]"`
- код: `crates/dpos/consensus/src/beacon/artifact.rs:944`
  `"beacon: PK_epoch obtained and verified against committee[epoch]"`
- **когда разошлось:** коммит `129f2754` «refactor(beacon)!: make the agreed artifact the only
  owner of the epoch key» (строка 5.1) — в его диффе по `beacon/follower.rs` видно удаление
  строки `"cert-follow: PK_epoch obtained and verified against committee[epoch] — …"`.
  `git merge-base --is-ancestor 129f2754 HEAD` → да. `git diff HEAD --stat -- devnet/` →
  пусто, то есть строка 5.2 `devnet/` не трогала.
  **Вывод: расхождение предсуществует `HEAD` и создано строкой 5.1, не 5.2.** Отчёт третьего
  прохода здесь прав.
- **насколько это больно:** это жёсткий FAIL, а не skip. Четыре сайта в
  `cases/smoke/asserts_follow.py` (`:326`, `:400`, `:608`, `:663`) делают
  `ctx.poll(has_key, …)` и затем `ctx.check(case, got, msg)` по литералу. Два follower-кейса
  будут падать на живом прогоне. Корроборация `CF_ADOPTED_FAMILY =
  dpos_follower_artifact_adopted_total` этого не спасает — комментарий самого харнесса
  говорит, что счётчик требует двух девнетных предусловий и «не имеет права быть тем
  свидетелем, на котором держится вердикт».
- **записано** в `15_smoke_cases…md` отдельным блоком `[DRIFT, NAMED …]` с явной пометкой,
  что `devnet/` — не файл этой строки.

### 4.2 `asserts_follow.py:318` — обоснование выбора узла ссылается на отставной механизм

«Read on `cert-follower` and not on the phase-3 tamper node: that one refuses every
certificate, so it **never calls `observe_cert`** and could never obtain a key». После строки
5.2 `observe_cert` — пустой дефолт, а KEY WANT едет на вердикте `Pending`. **Выбор узла
остаётся правильным** (узел, отвергающий каждый сертификат, не поднимет и want), но **причина
названа неверно**. Записано там же.

### 4.3 Что НЕ разошлось

Кейс Э3.3 (`a_forged_seed_slot_is_admitted_with_no_key_and_refused_when_the_key_lands`) — это
крейтовый тест стенда, не девнет; он зелёный по воротам `c2` (стенд с фичей 55/0). В доке
обновил имя сайта отказа (`SeedIndex::settle_epoch`, `beacon/seed_index.rs:321`) и добавил,
что этот же отказ теперь порождает `DataFault`.

---

## §5 Где моя проверка слабее всего

1. **Якоря вне `crates/dpos/consensus`.** Строки вида `bins/fluent/src/node_modes.rs:96`,
   `crates/node/src/dpos.rs:1514`, `outer.rs:1105/1145/1439` в таблицах `09` я **не**
   перепроверял — они не входят в инвентарь дрейфа строки 5.2, но соседние якоря того же
   абзаца (`dpos.rs:2424`/`3306`) оказались просроченными на ~200 строк, так что вся эта
   таблица, вероятно, дрейфует и дальше. Перепинил только те, которые сам открыл
   (`dpos.rs:2616`, `:3426`, `:3457`).
2. **`09_followers.md` §9.2-§9.6.2 не читал построчно.** Проверил только грепом инвентаря
   (ноль попаданий) — это аргумент «нет упоминаний удалённых символов», а не «содержание
   верно». Если строка 5.2 изменила что-то в поведении `SafetyHalt` или frontier-резолвера
   косвенно, я бы этого не увидел.
3. **Новое правило вытеснения проверено чтением, не прогоном.** Я прочитал `evict`,
   `bound_pending`, `bound_verified`, `oldest_evictable`, `retire_closed_pending_epochs` и
   убедился в отсутствии бесконечных циклов и в корректности префиксных допущений (после
   проверки порядка `Round` в чекауте commonware). Но тесты `seed_index.rs` (строки 620-1100)
   я **не** читал целиком и ничего не запускал — ворота дал оркестратор.
4. **Незакрытый вопрос, который я НЕ утверждаю как дефект.** Окно ретенции теперь
   отсчитывается от **собственной** наивысшей эпохи индекса (`evict`, `beacon/seed_index.rs:
   395-399`), а не от фронтира, который узел получал снаружи (`highest_entered_epoch` /
   фронтир инлета). Это смена источника доверия: якорь окна — теперь максимальная эпоха,
   названная **любым** принятым сертификатом. Я проследил, что до `hold` доходят только
   сертификаты, прошедшие multisig-проверку против `committee[epoch]`, прочитанного из
   собственного состояния узла, — то есть «эпоха из будущего» означает, что узел отстал, а не
   что его обманули. Но я **не** прошёл по всем путям верификации до конца и не проверял,
   может ли валидаторный инлет (у которого `epoch_bind == None`) принять сертификат эпохи,
   чья committee читается, но чей номер сильно выше фронтира. Помечаю как UNPROVEN, не как
   находку. Последствие в худшем случае ограничено 8 эпохами и касается только `Pending` и
   экзёмпции терминалов ниже `top − 8`.
5. **`00a_errata_2026_08_10_full_audit.md` и `14_known_cross_repo_drift…md` не проверял по
   содержанию** — только грепом инвентаря (ноль попаданий).
6. **Ворота не гонял** (по указанию оркестратора: код не трогал). Все утверждения о
   зелёности в записи `verified-against` — это ретрансляция чисел тега `c2`, а не мой прогон.
   В самой записи они помечены как «Gates (tag `c2`)», чтобы провенанс читался.
