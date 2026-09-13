# Строка 5.2, заход А — третий проход (правки по `E5-2-A-REVIEW.md`)

Исполнитель: Opus 5, свежий контекст. База `HEAD = 37691136` (`git rev-parse HEAD`
подтверждён в начале и в конце). Агентов не запускал. `git` только на чтение.
Пути без префикса — от `crates/dpos/consensus/src/`.

**Ограничения среды — ПРЯМО** [KNOWN]: их не было. `sed -i` по файлу репозитория
разрешён (проверил no-op подстановкой на `lib.rs`, md5 совпал с копией). `cargo
test` разрешён; все прогоны ниже — мои, `CARGO_BUILD_JOBS=6`, стенд ни разу не
шёл параллельно с другой сборкой.

Дерево после работы (`git status --short -- crates`): 18 изменённых, 2 удалённых
(`beacon/certify.rs` — прошлым проходом, `beacon/follower.rs` — этим), 1
untracked (`beacon/seed_index.rs`). Ни одного файла вне списка на запись.

---

## §0 Все находки ревью

| C-nn | вердикт | file:lines | что сделано / почему нет |
|---|---|---|---|
| **C-01** BLOCKER | **FIXED** | `dpos.rs:2483`, `:3244` | Откат R-016 на обоих путях: `crate::cold_start_jump::JUMP_THRESHOLD.min(interval)`. Оба доккомментария «R-016: ABSOLUTE» сняты, прежний текст восстановлен ДОСЛОВНО — `git diff` по обоим ханкам теперь пуст относительно `HEAD` (проверил `git diff -U2 … \| grep JUMP_THRESHOLD` — ни одной строки). |
| **C-02** SERIOUS | **FIXED** | `beacon/follower.rs` УДАЛЁН; `beacon/plane.rs:1014-1287` (продакшн), `:1327-2135` (тесты); `beacon/surface.rs:2050`, `:2106`, `:2153`; `beacon/oracle.rs` (−63 стр.); `beacon/mod.rs:128` | Опровержение ревью проверил сам и оно ДЕРЖИТСЯ (см. §1.2). Слияние выполнено: `FollowerRandomness` и `KeyOnlyOracle` удалены целиком, follower строит тот же `LiveBeacon`, что валидатор, над ПУСТЫМ `CeremonyStore`. Второго `impl Randomness` в крейте больше нет. Все девять follower-тестов переехали БЕЗ ИЗМЕНЕНИЯ ТЕЛ (кроме одного `metrics` → `metrics()`) и зелены. |
| **C-03** SERIOUS | **FIXED** | `seed_index.rs:94` (бюджет ПО СОСТОЯНИЮ), `:449-471` (`bound_pending`), `:473-490` (`bound_verified`) | Раздельные бюджеты: `SEED_RETENTION` считается по каждому состоянию отдельно, поэтому `Pending`-поток структурно не может вытеснить `Verified`. Цена названа в доке константы: пик — `2 · SEED_RETENTION` записей (те же ~400 КБ, что держали две карты на `HEAD`). Тест + мутация — §1.1. |
| **C-04** SERIOUS | **FIXED** | `seed_index.rs:512-529` (`oldest_evictable` фильтрует `Entry::Verified`) | Терминальная защита теперь CHECKED-ONLY: `Pending` не получает защиту и не даёт её. Тест + мутация — §1.1. |
| **C-05** SERIOUS | **FIXED** | `seed_index.rs:426-437` (`retire_closed_pending_epochs`), вызов `:403` | Поэпошная вычистка `Pending` восстановлена как ПРАВИЛО внутри индекса (замена `retain_quarantine_from` без внешнего привода): всё, что ниже `top_epoch − SCHEME_RETENTION_EPOCHS`, уходит целой эпохой. Тест + мутация — §1.1. |
| **C-06** SERIOUS | **FIXED** | `beacon/surface.rs:326-361` (`LateFaults`), `plane.rs` (follower строит `LiveBeacon`, у которого канал есть) | Поздняя половина Д-3 у follower-а теперь ЕСТЬ — и не копией, а тем же каналом: три поля `faults_tx/rx/armed` `LiveBeacon`-а вынесены в `LateFaults` (`new`/`report`/`take`), и после слияния (C-02) follower получает его по построению. Тест + мутация — §1.3. |
| **C-07** MODERATE | **FIXED** | `cert_inlet.rs:562` (`let late_charges = …`), `:748-762` (гейт сброса), `:847` (`drain_late_verdicts -> usize`) | Заряд, сделанный ВНУТРИ этого ingest-а, больше не стирается этим же ingest-ом: сброс серии пропускается, если дренаж что-то зарядил. Синхронный фолт переживал свой сертификат через `return`; теперь это верно и для позднего. Тест + мутация — §1.4. |
| **C-08** MODERATE | **RECORDED** | `beacon/plane.rs:973-977`; `surface.rs:2138-2147` | Не правил. Ревью само (§5 п.2) требует сохранить ПОРЯДОК (settle до публикации) и лечить только СТОИМОСТЬ; способ («ограничить число промоушенов на ребро» или `spawn_blocking`) — это изменение модели исполнения, а не правка по находке, и постановка третьего прохода его не заказывает (пункты 1-6 его не называют). Остаётся открытым: до `SEED_RETENTION` пороговых проверок синхронно внутри арма `select!`, задерживая сам `KeyAvailable`. **Слияние C-02 расширило область: тот же всплеск теперь возможен и на follower-е** (`plane.rs:run_fetcher`, арм `key_edge`) — там он был и раньше, но теперь это один и тот же код. |
| **C-09** MODERATE | **RECORDED** | `cert_inlet.rs:2933-2952`; `plane_upstream.rs:371-389` | Не правил, и не мог в рамках списка: рычагом был бы `RotateUpstream` у `UpstreamResolver`, а оба его конструктора (`crate::outer`, `node/dpos.rs`) вне списка на запись, и `plane_upstream.rs` тоже. Проверил сам: вердикт по-прежнему только `warn!`, `handler.deliver(..)` к этому моменту уже вернул `true`. Находка верна, остаётся открытой (см. §4). |
| **C-10** MODERATE | **FIXED** | вызов снят `cert_inlet.rs:758` (был); доки: `cert_inlet.rs:764-772`, `:2973-2977`, `dpos.rs:3719-3727`, `stand.rs:354`, `:2673`, `cert_inlet_tests.rs:99`, плюс `surface.rs:231` (док самого метода) | Продакшн-вызов пустого дефолта удалён; **продакшн-вызывающих `observe_cert` в крейте больше нет** (`git grep observe_cert -- crates`: остались только доки, тестовый враппер `byzantine_roles.rs:404-405` и `surface.rs:231`). `dpos.rs` содержал ДВА ложных утверждения подряд — подтвердил и переписал оба. Сайты в `crates/node/**` не трогал, перечислены в §4. |
| **C-11** MODERATE | **FIXED** | `beacon/metrics.rs:172-187` (`seed_verify_invalid`), регистрация `:331-337`; `beacon/oracle.rs:212-218`, `:225-228` | Синхронный `Refused` получил счётчик. Считаю на ОРАКУЛЕ, поэтому одна семья покрывает обе половины Д-3 и обе двери; «`Invalid` умышленно не считается» переписано с указанием, что защёлка per-epoch (5.2) и отсутствие громкой обработки на follower-е сломали прежний довод. Пинуется расширенным `the_refusal_line_is_latched_once_per_epoch` (`plane.rs:2029-2036`): «защёлка ограничивает СТРОКУ, но не СЧЁТ». |
| **C-12** MINOR | **RECORDED** (+док) | `spec_exec.rs:114-119` | Оставляю поведение: ревью само проверило, что `try_drain_parked` имеет ещё два привода (`executor.rs:2920`, `:3892`), то есть это задержка, а не вечный park. Я это перепроверил `git grep try_drain_parked` — три продакшн-сайта. Молчание доков было настоящей частью находки, поэтому вторая последствие названо прямо в коде. |
| **C-13** MINOR | **FIXED** | `seed_index.rs:281-300` (`SeedIndex::lock`), применён в `admit`/`seed`/`pending_epochs`/`settle_epoch`/`snapshot` | Правка КОРНЕВАЯ, а не «вернуть bool»: отравленный лок больше не роняет σ — guard восстанавливается (`PoisonError::into_inner`), ровно как в соседнем `ArtifactStore` (`artifact.rs:541-547`, довод дословно тот же: под локом простой `BTreeMap`, ни один путь не мутирует его больше чем одним оператором). Поэтому вердикт `Recorded`/`Pending` не может оказаться ложью, и `warn!` на каждый сертификат исчез. Тест + мутация — §1.5. |
| **C-14** MINOR | **RECORDED** | `surface.rs:327` (`LateFaults.tx`), дренаж `cert_inlet.rs:562` | Оставляю. Записи кладутся только при `armed` (приёмник взят) и только на ребре settle с `refused > 0`, то есть не чаще одной на (эпоха × ребро артефакта); рост возможен лишь у армленного инлета при полностью остановленном `ingest`. Ограничивать каналом — значит ТЕРЯТЬ обвинение, что прямо против Д-3 («факт, а не пробуждение»); метрика глубины — это новая семья ради случая, который сам является отдельной находкой (C-09/§4). Цена оставления: глубины не видно. |
| **C-15** MINOR | **RECORDED** | `seed_index.rs:208-212` | Оставляю: поведение `Pending ⇒ Pending` last-wins тождественно `HEAD:certify.rs:270` (сверил дословно), регрессии нет, а ревью само это подтверждает. Док автомата уже говорит, что отказанная запись ДРОПАЕТСЯ, то есть раунд снова можно спросить — это и есть ответ на «затёртую честную σ». |
| **C-16** NIT | **FIXED (в отчёте)** | `crates/dpos/consensus/tests/slasher_integration.rs` | Журнал §3(е) неверен. Ворота гоняют не крейт `-p fluentbase-slasher`, а таргет `cargo test -p fluentbase-consensus --test slasher_integration`; файл существует, мой прогон — 16/0 (§2). |
| **C-17** NIT | **RECORDED (исправлено здесь)** | `.dpos-study/history/E5-2-A.md` §5(1), §0(12) п.3 | Журнал захода в список на запись не входит, править его не могу. Обе формулировки признаю неверными: (1) «σ-половина `follower.rs` удалена целиком» была ложной на момент журнала — она удалена ЭТИМ проходом (C-02); (2) цена общего бюджета — не «`Pending` теряется раньше», а «`Pending` вытесняет `Verified`» (C-03). |
| **C-18** NIT | **FIXED частично** | `beacon/mod.rs:172-177` (док `keyless_index`); `executor.rs` — не трогал | `keyless_index` на тестовой границе теперь объяснён: это КОНСТРУКТОР (замена удалённого `for_seeds`), а не переименование, в отличие от алиаса `SeedStore` под ним. Вторая половина находки (доккомментарии в продакшн-теле `executor.rs` при букве «ТОЛЬКО `#[cfg(test)]`») — постановка ЭТОГО прохода разрешает в `executor.rs` доккомментарии явно (п. «Файлы на запись»), так что задним числом нарушения нет; я `executor.rs` не менял вовсе (md5 совпадает с `c1`). |
| **C-19** NIT | **FIXED** | `plane.rs:1211`, `:1215`, `:1258`, `:1268`, `:1277` | Имя `promoter` (переменная и параметр `Weak<…>`) переименовано в `provider`; «QUARANTINE PROMOTE на втором арме» в доке `run_fetcher` → «SETTLE of held σ». Задачи-промоутера в дереве нет ни под каким именем (`git grep promoter -- beacon/`: остаётся одно историческое упоминание `plane.rs:803`, которое ОПИСЫВАЕТ удаление). |
| **C-20** MINOR | **FIXED** | девять сайтов, все проверены | Снимается откатом, и я прошёл каждый: `dpos.rs:2477-2478` и `:3241-3242` восстановлены дословно (ханки пусты против `HEAD`); `executor.rs:663-671` («Only the tests construct a bare `JUMP_THRESHOLD`») — **байт-в-байт совпадает с `HEAD`, откат его РАЗ-инвертировал, править нечего** (проверил `sed -n '655,680p'` против `git show HEAD:…`); `cold_start_jump.rs:774`, `stand.rs:156`, `tests.rs:1343`, `:1591`, `preconditions.rs:304`, `:314` — все четыре файла не менялись этим заходом (`md5sum -c c1-tree.md5`: `cold_start_jump.rs`/`preconditions.rs` вообще не в дереве изменений) и после отката снова верны. Остаточное — в §4. |
| **C-21** MODERATE | **FIXED откатом; остаток назван** | `tests.rs:1385`, `:1618`, `:2616`, `:3492`, `:3709`; `preconditions.rs:162`, `:353`; `committee_tests.rs:201`, `:619` | Снимается ли ПОЛНОСТЬЮ — **нет, и вот что осталось.** Расхождение снято: продакшн снова считает `JUMP_THRESHOLD.min(interval)`, то есть ровно ту формулу, которую задают все девять стендовых сайтов, и `preconditions.rs:353` снова пинует настоящее продакшн-значение (`min(1024, 32) = 32 < 2·interval = 64`, гейт арминится). Остаток СТРУКТУРНЫЙ и предсуществует `HEAD`: стенд ЗАДАЁТ порог сам (`cfg.re_jump_threshold = Some(...)`), а не читает продакшн-вычисление, поэтому следующее изменение продакшн-формулы снова останется без красного теста. Закрыть это можно только тем, чтобы стенд брал порог из общего конструктора — а он вычисляется внутри `dpos.rs::launch*`, у которых нет тестируемой точки входа. Назвал как открытое (§4). |
| **C-22** MINOR | **RECORDED** (+док) | `surface.rs:145-157` | Оставляю поведение. Ревью само признаёт: безопасность цела (единственный вызывающий `epoch_manager.rs:159-163` называет раунд из СОГЛАСОВАННОГО терминального блока), а `HEAD`-док считал две формы эквивалентными по аргументу. Тест «`terminal_seed` не отвечает на не-терминальный раунд» написать НЕЛЬЗЯ так, чтобы он что-то значил: операция сведена к `seed` одной строкой, и такой тест пинул бы фикстуру. Потерянное свойство названо в доке прямо, включая то, что зафиксировать его теперь нечем. |
| **C-23** NIT | **FIXED** | `beacon/actor.rs:1408-1412`, `engine.rs:43-44` | Оба доккомментария больше не называют удалённый `beacon::certify`: ссылаются на `beacon::surface::certificate_verdict` — реальное место, где σ проверяется под аттестованным ключом эпохи. `git grep 'beacon::certify' -- crates` — ноль. |

---

## §1 SERIOUS-и-выше: ханк, тест, однострочная мутация

Каждая мутация ниже ПОСТАВЛЕНА И ПРОГНАНА мной: тест краснел, файл восстанавливался
из копии с проверкой md5.

### 1.1 C-03 / C-04 / C-05 — одна правка правила вытеснения

Ханк — `seed_index.rs:391-529`: `evict` больше не «oldest-first до одного бюджета», а
три шага, каждый со своим свойством:

~~~rust
fn evict(entries: &mut BTreeMap<Round, Entry>) {
    let Some(top_epoch) = entries.keys().next_back().map(|round| round.epoch().get()) else { return };
    let floor = top_epoch.saturating_sub(crate::SCHEME_RETENTION_EPOCHS as u64);
    retire_closed_pending_epochs(entries, floor);   // C-05
    if entries.len() <= SEED_RETENTION { return; }  // дешёвый общий выход
    bound_pending(entries);                          // C-03, поэпошно
    bound_verified(entries, floor);                  // C-03 + C-04
}
~~~

Вторая карта НЕ восстановлена: свойства несёт правило.

- **(а) защиту получает только `Verified`** — `oldest_evictable` (`:512`) перебирает
  `entries.iter().filter(Entry::Verified)`, поэтому «высший раунд эпохи» считается
  среди ПРОВЕРЕННЫХ, и `Pending` ни защиты не получает, ни её не отнимает.
- **(б) `Pending` не вытесняет `Verified`** — выбран **раздельный бюджет**, а не
  приоритет. **Цена, названная:** пик памяти — `2 · SEED_RETENTION` записей вместо
  `SEED_RETENTION` (док `:84-93`); это ровно тот пик, который держали две карты
  `HEAD` по `SEED_RETENTION` каждая, то есть возврата к худшему нет, но и экономии
  от слияния карт в памяти нет — слияние окупается ОДНИМ владельцем, не байтами.
- **(в) `Pending` уходит ПОЭПОШНО** — `retire_closed_pending_epochs` снимает целую
  эпоху ниже окна (префикс карты, `take_while`), а счётный бюджет `bound_pending`
  роняет старейшую pending-ЭПОХУ целиком. Исключение названо и обосновано в доке
  `:441-448`: ПОСЛЕДНЯЯ оставшаяся pending-эпоха подрезается со СТАРОГО конца, а не
  сносится целиком, потому что это эпоха, чей ключ ещё может прийти, а раунд, который
  спросит граница, — самый ВЫСОКИЙ из держимых; «остатка, который никогда не
  разрешится», здесь не возникает. Цена этого исключения тоже в доке: узел,
  бесключевой дольше `SEED_RETENTION` раундов ОДНОЙ эпохи, теряет её старейшие
  держимые σ и переспрашивает их после прихода ключа.

| находка | тест (`seed_index.rs`) | однострочная мутация | прогон |
|---|---|---|---|
| C-03 | `a_pending_flood_cannot_evict_a_verified_seed` `:944` | в `bound_verified` (`:478`): `while verified > SEED_RETENTION {` → `while entries.len() > SEED_RETENTION {` | **красный**: `a held flood evicted the checked σ of the epoch below: Round { epoch: Epoch(1), view: View(0) }`, 15 passed / 1 failed |
| C-04 | `a_pending_round_above_the_terminal_does_not_take_its_protection` `:983` | в `oldest_evictable` удалить строку `.filter(\|(_, entry)\| matches!(entry, Entry::Verified(_)))` | **красный**: `left: None, right: Some(95903c6d…)`, 15 passed / 1 failed |
| C-05 | `a_closed_epochs_pending_rounds_are_retired_as_a_unit` `:1018` | в `evict` удалить строку `retire_closed_pending_epochs(entries, floor);` | **красный**: `left: [1, 2], right: [2]`, 15 passed / 1 failed |

Все три теста самопроверяют предпосылку до вердикта (флуд действительно превышает
бюджет; держимый раунд действительно не обслуживается; соседняя эпоха внутри окна
остаётся нетронутой — «ушло как ЕДИНИЦА, а не по раунду»).

### 1.2 C-02 — слияние `follower.rs`

**Опровержение ревью я проверил сам, и оно ДЕРЖИТСЯ** — но не по той причине, что
тела `verify_seed` одинаковы (это верно, но это только один метод из четырёх). Держит
его `with_material` (`oracle.rs:117-122`):

~~~rust
fn with_material<T>(&self, f: impl FnOnce(&Sharing<MinSig>, &Share) -> T) -> Option<T> {
    let (minted_at, sharing) = self.keys.sharing_at(self.epoch)?;
    let held = self.ceremony.read().ok()?;
    let share = held.get(&minted_at)?;        // ← пустой CeremonyStore ⇒ None
    Some(f(&sharing, share))
}
~~~

На follower-е `sharing_at` может быть `Some` (артефакт скачан), но ШАРЫ нет никогда,
поэтому `sign_partial → None`, `verify_partial → false`, `recover → None` — ровно те
перманентные негативы, которые `KeyOnlyOracle` давал «по типу». А `verify_seed` в
обеих реализациях начинался с `keys.key_at(self.epoch)` и в `beacon::verify_seed` не
заглядывает в ceremony вовсе. То есть follower — это `LiveBeacon` с ПУСТЫМ
`CeremonyStore`, а не другой тип.

Сделано:
- `beacon/follower.rs` **удалён** (1276 строк). `FollowerRandomness` (собственный
  `SeedIndex`, `settle_pending`, защёлка, восемь σ-методов) и `KeyOnlyOracle`
  (`oracle.rs`, −63 строки) удалены целиком. **Второго `impl Randomness` в крейте
  нет.**
- Продакшн-половина переехала в `beacon/plane.rs:1014-1287` — `build_follower`,
  `FollowerInputs`, `ArtifactFetch`, `changed_bit`, `run_fetcher`, `WANT_MAILBOX` —
  и строит `LiveBeacon::build(LiveBeaconConfig { ceremony: keyless_ceremony(), … })`.
- **Want-нога.** Единственное, чего `LiveBeacon` не умел: поднимать «мне нужен
  `PK_epoch`» из `hold_seed`. Добавлено как `want: OnceLock<mpsc::Sender<u64>>`
  (`surface.rs:2050`) + `wire_want` (`:2106`), а НЕ поле `LiveBeaconConfig`, и это
  вынужденно: `LiveBeaconConfig` конструируется в `epoch_manager.rs:2083`, `:2128` —
  файл ВНЕ списка на запись, новое поле сломало бы его компиляцию. Write-once
  корректно, потому что единственный вызывающий пишет туда до того, как `Arc` кому-то
  отдан; вторая запись — `error!`. На валидаторе `want` не выставлен, и `hold_seed`
  пропускает push.
- **Ключ на follower-е берётся тем же `ensure_key(Thorough)`,** что у валидатора:
  `run_fetcher` want-арм больше не дёргает `acquire`/`keys` напрямую. Это строго
  лучше прежнего: `LiveBeacon::ensure_key` сначала проверяет `holds_mint_of` и
  сетевой круг не тратит, если ключ уже есть.
- `geometry: watch::channel(Some((0, 1))).1` — обосновано в коде: единственный
  читатель watch-а, уточнение `GeometryUnfrozen` в `share_probe`, на классе без
  церемонии дал бы ложную историю; `NoUsableShare` — правда.
- **Девять follower-тестов переехали в `plane.rs::follower_tests` с НЕТРОНУТЫМИ
  телами** (изменены только импорты модуля и одно обращение `fb.provider.metrics` →
  `fb.provider.metrics()`), и все девять зелёные против слитой реализации. Это и есть
  главное свидетельство, что слияние поведения не поменяло: тесты писались против
  `FollowerRandomness` и проходят против `LiveBeacon` без единой правки ассертов.
- Граница крейта не изменилась ни на одно имя: `beacon/mod.rs:128` —
  `pub use plane::{build_follower, ArtifactFetch, FollowerInputs}`. Единственный
  продакшн-вызывающий — `dpos.rs:3426` (файл в списке), **и он не правился**;
  `crates/node/**` называет `build_follower` только в доккомментариях.

Тест, ловящий регрессию слияния, и мутация — в §1.3 (follower-ские фолты) плюс
`a_verified_certificates_seed_is_filed_and_served` / `a_seed_that_arrives_before_the_key_is_held_and_then_promoted`
(`plane.rs`), которые ловят подмену оракула: если бы пустая ceremony ломала
`verify_seed`, первый покраснел бы на `seed(round) == Some(σ)`.

### 1.3 C-06 — поздний отказ у follower-а доходит до потребителя

Ханк — `surface.rs:305-360`: три поля `LiveBeacon` (`faults_tx`, `faults_rx`,
`faults_armed`) вынесены в тип `LateFaults` с `new`/`report`/`take`; `Beacon::faults`
у `LiveBeacon` — `self.faults.take()`. После слияния (C-02) follower получает канал
по построению, а не второй копией.

Тест: `plane.rs::follower_tests::a_late_refusal_reaches_the_followers_fault_consumer`
(`:2052`). Форма: потребитель берёт `faults()` ПЕРВЫМ (канал армится), σ соседнего
раунда вклеивается в сертификат при ОТСУТСТВУЮЩЕМ ключе (`Observed::Pending`,
ассертится), проверяется что ничего не зафайлено пока вердикт открыт, затем приходит
артефакт → settle → `try_recv() == Ok(DataFault { epoch, refused: 1 })`. Тест
самопроверяет подмену (`assert_ne!` до сплайса) и «не более одного потребителя»
(второй `faults()` → `None`).

**Мутация (одна строка):** удалить `self.faults.report(DataFault { epoch, refused });`
из `FollowerRandomness::settle_pending` (на момент постановки мутации код ещё жил в
`follower.rs`). **Прогон: красный** — `right: Ok(DataFault { epoch: 10, refused: 1 })`,
8 passed / 1 failed. После слияния эквивалентная мутация — та же строка в
`LiveBeacon::settle_pending` (`surface.rs:2153`).

### 1.4 C-07 — симметрия синхронного и позднего заряда

Ханк — `cert_inlet.rs`:

~~~rust
let late_charges = self.drain_late_verdicts().await;   // :562, возвращает usize
…
if late_charges == 0 {                                  // :760
    self.consecutive_faults = 0;
}
~~~

Тест: `cert_inlet.rs::tests::a_sub_threshold_late_charge_survives_the_certificate_it_rode_in_on`
(`:2306`). Один форджённый раунд удержан при бесключевом окне ⇒ один поздний заряд
(ниже порога); чистый сертификат его доставляет и ОБРАБАТЫВАЕТСЯ (`vec!["verified",
"report"]` ассертится), ротаций 0; затем два синхронных фолта ⇒ `1 + 2 =
MAX_UPSTREAM_FAULTS` ⇒ ровно одна ротация.

**Мутация (одна строка):** `if late_charges == 0 {` → `if true {`.
**Прогон: красный** — `right: 1` (ротаций стало 0), 0 passed / 1 failed.

### 1.5 C-13 — отравленный лок не теряет факт

Ханк — `seed_index.rs:281-300`: один `SeedIndex::lock`, восстанавливающий guard
(`PoisonError::into_inner`), на всех пяти сайтах вместо `let Ok(..) else { warn; return }`
и `.lock().ok()?`. Довод в доке — тот же, что у `ArtifactStore` (`artifact.rs:541-547`):
под локом простой `BTreeMap`, ни один путь не мутирует его больше чем одним
оператором, поэтому половинчатого состояния быть не может; терять σ или ронять узел —
обе альтернативы хуже.

Тест: `seed_index.rs::tests::a_poisoned_index_still_files_and_still_answers` (`:1063`).
Отравляет лок настоящим unwind-ом (`catch_unwind` + guard), **самопроверяет, что
отравление состоялось** (`index.entries.lock().is_err()`), затем требует, чтобы
`record` был обслуживаемым, а `hold` — держимым, то есть чтобы вердикты `Recorded` и
`Pending` были правдой.

**Мутация (одна строка):** в `SeedIndex::lock` заменить
`.unwrap_or_else(std::sync::PoisonError::into_inner)` на `.expect("index lock")`.
**Прогон: красный**, 0 passed / 1 failed.

---

## §2 Ворота — все десять, verbatim

Прогон `d3` (мой, `gates/run.sh`, `CARGO_BUILD_JOBS=6`, последовательно), на ФИНАЛЬНОМ
дереве. Все `exit=0`, последняя строка `DONE`.

| ворота | команда | verbatim |
|---|---|---|
| `--lib` | `cargo test -p fluentbase-consensus --lib` | `test result: ok. 665 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 174.51s` |
| стенд С ФИЧЕЙ | `… --lib --features dpos-devnet-byzantine testbed::` | `test result: ok. 55 passed; 0 failed; 0 ignored; 0 measured; 619 filtered out; finished in 223.87s` |
| стенд БЕЗ фичи | `… --lib testbed::` | `test result: ok. 46 passed; 0 failed; 0 ignored; 0 measured; 619 filtered out; finished in 171.37s` |
| node | `cargo test -p fluentbase-node --lib` | `test result: ok. 55 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 57.01s` |
| reader | `cargo test -p fluentbase-staking-reader` | `test result: ok. 64 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s` + `test result: ok. 0 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s` |
| slasher | `cargo test -p fluentbase-consensus --test slasher_integration` | `test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s` |
| clippy БЕЗ фичи | `cargo clippy -p fluentbase-consensus -p fluentbase-node -p fluentbase-staking-reader --all-targets` | `warning: this MutexGuard is held across an await point` / `warning: fluentbase-staking-reader (lib test) generated 1 warning` / `warning: large size difference between variants` / `warning: fluentbase-node (lib test) generated 1 warning` / `warning: fluentbase-node (lib) generated 1 warning (1 duplicate)` — ровно ДВА ЧУЖИХ, в `fluentbase-consensus` ноль |
| clippy С ФИЧЕЙ | `cargo clippy -p fluentbase-consensus --all-targets --features dpos-devnet-byzantine` | ноль предупреждений |
| fmt | `cargo fmt --check` | `exit=0`, пустой вывод |
| doc | `cargo doc -p fluentbase-consensus --no-deps` | 6 × `unresolved link`: `` `E` `` ×2, `` `crate::epoch_manager::Actor::enter` ``, `` `FakeMarshal` `` ×2, `` `Scheme::verify_attestation` `` |

**Баланс счёта тестов по файлам** (`grep -c '#\[test\]'`, `HEAD:` против дерева):

| файл | `HEAD:` | дерево | Δ |
|---|---|---|---|
| `beacon/certify.rs` | 14 | — (удалён) | −14 |
| `beacon/follower.rs` | 7 | — (удалён) | −7 |
| `beacon/seed_index.rs` | — (новый) | 17 | +17 |
| `beacon/plane.rs` | 1 | 10 | +9 |
| `cert_inlet.rs` | 24 | 27 | +3 |
| `dpos.rs` | 31 | 32 | +1 |
| **итого** | | | **+9** |

`656 + 9 = 665` ✔ — сходится с `--lib`. Сходится и со второй стороны: стенд без фичи
`46 + 619 filtered = 665`.

Против ворот `c1` оркестратора (659): `+6` — четыре новых теста в `seed_index.rs`
(три по C-03/04/05 и один по C-13), один в `plane.rs::follower_tests` (C-06) и один в
`cert_inlet.rs` (C-07). Девять follower-тестов переехали и в дельте не участвуют
(`−7` из `follower.rs` и `+8` в `plane.rs` из тех же девяти + один новый).

Прочие ворота против `c1` — байт-в-байт: clippy без фичи ровно два ЧУЖИХ
предупреждения (`MutexGuard held across an await` в `fluentbase-staking-reader`
lib-test, `large size difference between variants` в `fluentbase-node`; в
`fluentbase-consensus` ноль), clippy с фичей ноль, `fmt --check` чисто, doc — 6
`unresolved link` (те же шесть: `E` ×2, `crate::epoch_manager::Actor::enter`,
`FakeMarshal` ×2, `Scheme::verify_attestation`).

---

## §3 Отклонения

**Новых отклонений `5.2а-Д-n` нет.** Все шесть пунктов постановки выполнены в
рамках списка файлов; пункт 6 не потребовал остановки (§1.2).

Ближе всего к отклонению подошёл want-канал слияния: поле в `LiveBeaconConfig`
сломало бы `epoch_manager.rs:2083`/`:2128` (запрещённый файл), и это решено внутри
списка — `OnceLock` + `wire_want` в `surface.rs`. Записываю как принятую цену, а не
как отклонение: внешнее поведение не изменилось, а форма «write-once, выставляется
строителем до выдачи `Arc`» задокументирована в самом коде.

Ратифицированные П-2, Д-3, Д-9 не менялись. Д-3 стал ПОЛНЕЕ, чем был: поздняя
половина теперь есть на обоих классах узла (C-06), синхронная получила счётчик
(C-11), и вердикт синхронной двери больше не может быть ложью (C-13). Пробел Д-3 на
by-height двери (C-09) остаётся и назван в §4.

---

## §4 Что осталось незакрытым

1. **C-09 — by-height дверь без рычага.** Единственная дверь, чей проверяющий
   строится без оракула (`plane_upstream.rs:371-389`), и единственная с нулевой ценой
   за форджённую σ; к моменту вердикта `handler.deliver(..)` уже вернул `true`, то
   есть сертификат уже в marshal-архиве. Рычаг (`RotateUpstream` у
   `UpstreamResolver`, либо «пометить и перезапросить запись» из текста варианта (в)
   `DECISIONS.md:24`) требует `crate::outer` и `node/dpos.rs` — оба вне списка.
   Это отдельная строка, не остаток 5.2.
2. **C-08 — стоимость синхронного settle на ребре.** До `SEED_RETENTION` пороговых
   проверок внутри арма `select!`, задерживающих сам `KeyAvailable`. Порядок
   (settle → публикация) правильный и трогать его нельзя. После слияния C-02 тот же
   код обслуживает и follower-а — площадь выросла, механизм тот же.
3. **C-21, структурный остаток.** Стенд ЗАДАЁТ порог re-jump сам
   (`Some(JUMP_THRESHOLD.min(EPOCH_LEN))` на девяти сайтах), а не читает
   продакшн-вычисление, которое живёт внутри `dpos.rs::launch*` без тестируемой точки
   входа. Расхождение снято откатом, но следующее изменение продакшн-формулы снова
   останется без красного теста.
4. **C-14** — канал поздних вердиктов неограничен и его глубина не наблюдаема
   (см. §0).
5. **C-22** — «`terminal_seed` не отвечает на не-терминальный раунд» больше не
   пинуется ничем и запинуть это нечем, пока операция сведена к `seed`.
6. **Доккомментарии в ЗАПРЕЩЁННЫХ файлах, ставшие ложными этим проходом** (править
   не могу, перечисляю):
   - `crates/node/src/cert_inlet.rs:50` — «this inlet's `observe_cert` prune a store
     nothing else reads»; продакшн-вызова больше нет.
   - `crates/node/src/dpos.rs:784` — «`observe_cert` prunes what the key ladder
     reads»; то же.
   - `crates/node/src/dpos.rs:931` — «the quarantine promoter» в списке
     супервизируемых детей; задачи нет с прошлого прохода.
   - `crates/node/src/dpos.rs:374`, `:866` — «via `beacon::build_follower`»: имя
     живо, но модуль другой (`beacon::plane`).
   - `testbed/fakes.rs:1171` — `beacon::follower::changed_bit`; теперь
     `beacon::plane::changed_bit`.
   - `epoch_manager.rs:517`, `:853`, `:2281`, `:2835`; `executor.rs:9475`, `:5222`;
     `order_block.rs:249`; `plane_upstream.rs` — дрейф с прошлого прохода, не мой.
   - `devnet/local-dpos-smoke/dpos_harness/cases/smoke/asserts_follow.py:147`,
     `verdicts_follow.py:225`, `:528` — ссылки на `beacon/follower.rs:355-367`.
     **Файла больше нет.** Строки, на которые ссылается вердикт, при этом живы —
     правило `mandatory_at(epoch).then(..)` переехало в `LiveBeacon::oracle_for`
     (`surface.rs`).
   - Отдельно, НЕ моё и предсуществует `HEAD`: `verdicts_follow.py:237`
     `CF_KEY_LINE = "cert-follow: PK_epoch obtained and verified…"`, тогда как код
     пишет `"beacon: PK_epoch obtained and verified…"` (`artifact.rs:944`, и на
     `HEAD:943` тот же текст). Проверил обе стороны; расхождение старое.
7. **`.claude/dpos_architecture/`** этим проходом не правился — отдельная фаза Ф7.
   Слияние C-02 добавляет к её списку весь `09_followers.md` (тип follower-а,
   `promote_epoch`, `retain_*`, `for_seeds`) и `13_invariants_gotchas_rules.md:623`
   («a SEPARATE quarantine map — not a flag on the shared one»): этот инвариант
   заход снял, и Ф7 должна его РЕШИТЬ, а не переименовать.

### Что покрыто ТОЛЬКО девнетом

`make smoke-cert-follow` я не гонял (`devnet/` держит другая сессия), приёмку берёт
оркестратор. По слиянию C-02 девнетом покрыто следующее, и юнитами — нет:

1. **Реальная сборка follower-а.** Девять переехавших тестов строят `build_resolved`
   над канонными замыканиями; настоящий путь `dpos.rs:3426 → build_follower →
   LiveBeacon` не исполняется ни одним юнитом и ни одним стенд-тестом. Стенд
   `--cert-follow` узлы вообще не поднимает — это записано в самом стенде
   (`testbed/tests.rs:3436-3438`: «nothing here constructs a `--cert-follow`
   provider»).
2. **`TransportAcquire` над настоящим `CertUpstream::get_epoch_artifact`** (WS): в
   юнитах транспорт — замыкание `Upstream::fetch`.
3. **Want → `ensure_key(Thorough)` → adopt по живому линку.** Юниты проверяют, что
   want поднимается и что фетч не выполняется inline; что он ДОХОДИТ по WS — девнет.
4. **Смена INFO-строки** `"cert-follow: settled held seeds"` →
   `"beacon: settled held seeds"` (обе половины теперь одна). Проверил `git grep` по
   `devnet/`: ни один вердикт эту строку не читает, так что приёмка на ней не
   завязана — но операторские дашборды увидят переименование.
5. **`share_probe`/`signer_scheme` на follower-е.** Теперь это тела `LiveBeacon`
   поверх пустого `CeremonyStore` (`Withheld(NoUsableShare)`), а не отдельный тип.
   На этом классе их не вызывает НИКТО (epoch_manager там не запускается), поэтому
   не покрыто ни юнитом, ни девнетом — обосновано только чтением (§1.2).

---

## §5 Где моя работа слабее всего

1. **Слияние C-02 обосновано тестами, которые писались против старого типа.** Это
   сильное свидетельство, но не полное: девять тестов не покрывают
   `share_probe`/`signer_scheme` (п. 5 выше) и не трогают ни одного пути, где
   `LiveBeacon` отличается от удалённого типа НЕ пустотой ceremony — например
   `geometry`. Я выбрал `Some((0, 1))` по рассуждению о единственном читателе watch-а,
   а не по прогону.
2. **C-03, выбор «раздельные бюджеты» против «приоритет вытеснения» — суждение.**
   Приоритет дал бы тот же пик памяти, что и общий бюджет, но душил бы `Pending` на
   полном `Verified`-окне; я выбрал предсказуемость `HEAD` (бюджет на состояние) и
   назвал цену, но ЗАМЕРА, показывающего, что пик в 2·4096 записей приемлем на
   реальном узле, у меня нет.
3. **Исключение «последняя pending-эпоха подрезается по раунду» ослабляет букву
   пункта (в) постановки.** Я считаю, что оно правильное (§1.1(в)), и написал
   почему, но формально в этом одном случае `Pending` уходит не поэпошно. Тестом это
   исключение не пинуется — его пинует только общий бюджет.
4. **C-01 я откатывал, а не воспроизводил.** Клин воспроизвёл оркестратор; я
   восстановил текст дословно и проверил, что ханки пусты против `HEAD`, но
   мутационный прогон `preconditions.rs:353 → Some(JUMP_THRESHOLD)` сам не ставил —
   решение по пункту 1 было не предметом обсуждения, и лишний красный прогон ничего
   бы не добавил к уже снятой правке.
5. **Девнет не гонял вовсе** — см. выше. Для класса `--cert-follow` это самая
   дорогая дыра в этом проходе, потому что именно он перестроен целиком.
6. **`.claude/dpos_architecture/` я не открывал** сверх того, что назвало ревью:
   список Ф7 в §4 п. 7 — это пересказ таблицы ревью плюс мой вывод про `09_followers.md`,
   а не мой собственный обход каталога.
