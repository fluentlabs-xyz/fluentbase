# E5-3-A2 — финальное ревью захода А2 строки 5.3 (`Wiring` и хвосты автомата)

Ревьюер: Opus 5, свежий контекст, 2026-09-15. База `HEAD = f71f4d52`, объект — незакоммиченное
дерево. md5 (первые 8) сверены до и после каждой мутации: `actor.rs 9cbd4715`, `artifact.rs
85a74d45`, `ceremony.rs ad67f2f6`, `outcome.rs aed633bd`, `share_state.rs a46d50c8`, `plane.rs
bcad0998` — совпали с постановкой; `v1.md5` (ворота оркестратора) — тот же tree.
Пути без префикса — от `crates/dpos/consensus/src/beacon/`. Строки — по текущему дереву.
Агенты не запускались. Git — только чтение. Единственный записанный файл — этот.

## §0 — прямые ответы

### 0.1 Ворота (verbatim)

Мои прогоны (`scratchpad/rev/rv-*.txt`, после `DONE` в `gates/v1-status.txt`):

| ворота | результат |
|---|---|
| `cargo test -p fluentbase-consensus --lib` | `725 passed; 0 failed; 0 ignored` |
| `--features dpos-devnet-byzantine testbed::` (прогон 1) | `58 passed; 0 failed; 676 filtered out` |
| `testbed::` (прогон 1) | `49 passed; 0 failed; 676 filtered out` |
| `--features dpos-devnet-byzantine testbed::` (прогон 2) | `58 passed; 0 failed; 676 filtered out` |
| `testbed::` (прогон 2) | `49 passed; 0 failed; 676 filtered out` |
| `cargo clippy -p fluentbase-consensus -p fluentbase-node -p fluentbase-staking-reader --all-targets` | 2 чужих (`staking-reader/src/epoch_transition.rs:3024` MutexGuard across await; `node/src/dpos.rs:1991` large variant), 0 своих |
| `cargo fmt --check` | exit 0 (только nightly-option warnings rustfmt) |
| `make harness-test` (devnet/local-dpos-smoke) | `2 failed, 2086 passed, 10 skipped` — те же два чужих (`test_smoke_boundary_cases.py`) |

Ворота оркестратора `v1-*` над тем же md5 (прочитал, не запускал): node `57/0`, reader `0/0 (1 ignored)`,
slasher `16/0`, clippyf (с фичей) `0 warnings`, doc — 58 lib-warnings, **ни одного** в шести
файлах А2 (`grep beacon/{actor,artifact,outcome,share_state,ceremony}.rs v1-doc.txt` = 0);
базовые «doc 6» из постановки я воспроизвести не могу — считаю по `^warning`.

### 0.2 Эквивалентность `Wiring` продакшну

[KNOWN] `plane.rs:867-883` передаёт 13 полей; HEAD (`git show HEAD:…/plane.rs`, дифф `plane.rs`
hunk `@@ -856,31 +851,37 @@`) передавал ТЕ ЖЕ значения: `Some(logs)`→`resolver: logs`,
`Some(log_resolver_rx)`→`resolver_rx`, `.with_changed_bit(changed.clone())`→`changed: changed.clone()`,
`Some(share_dir)`→`share_dir`, `.with_plane_clock(plane_clock)`, `Some(outcome_at)`,
`.with_artifact_pull(pull_artifact)`, `.with_recorded_logs(recorded)`, `.with_share_confirms(confirms)`,
`.with_pinned_requests(pinned_rx)`, `.with_agreement_plane(agreement_request_tx, artifacts_rx)`,
`.with_body_lost(body_lost_rx)`. Внутри `new` (`actor.rs:1060-1061`) `set_recorded` → `set_pool`
— тот же порядок, что HEAD `with_recorded_logs`→`with_share_confirms` (HEAD `:1096`, `:1111`).
`DealerLogStore::new(.., Some(share_dir.clone()), ..)` (`:1049-1054`) = HEAD `share_dir.clone()` при
`Some`. `agreed_tx.clone()`→`agreed_tx` (`plane.rs:919`, дифф hunk `@@ -915,7 +916,7 @@`) — клон был только для удалённого replay.

Поведение продакшна против HEAD, кроме пяти правок, — отличий не нашёл. Проверил каждое место, где
HEAD ветвился на `None` (HEAD `:1151,1171,1179,1447,1799,1844,1890,2000,2114,2149,2162,2389,2467,
3098,3481,3569,3719` — список из `grep` по HEAD-копии): все стали безусловными, `Some`-ветвь HEAD
дословно. Различия: `recover` читает `stored` до маркера (`:3105`, был после) — лишь порядок чтения
замыкания; drain в `decide` считает `no_seat` ДО replay (`:2947-2953`), HEAD — после (только порядок
инкрементов счётчика); `sweep` дополнительно ретейнит `nondurable_dealings` (`:2173`).

Закрытые каналы. [KNOWN] `recv_or_park` (`:655`) на закрытом `pinned_rx`/`artifacts_rx`/`body_lost_rx`
= поведение HEAD (`None => pinned_rx = None` → `recv_or_never(None)` → `pending`), одно poll на
итерацию, без busy-loop. Обосновано: закрытый `artifacts_rx` не лишает актора артефактов —
`reconcile_with_store` (`:3824`) читает store каждый тик, pull-шов пишет в store. Закрытый
`resolver_rx` ⇒ `break` (`:1248-1256`) — не изменено, это решение 5.3-В (движок — supervised child,
узел падает с ним). Асимметрия осознанная и задокументирована в `Wiring` (`:742-746`).

### 0.3 Тесты после `Wiring`

[KNOWN] 15 сайтов `DkgActor::new(` в `actor.rs` (14 тестовых + `plane.rs:855`), 17 вызовов
`standalone_actor_wired/_at`, 31 `Wiring::standalone()/inert(`. 94 `#[test]` в `actor.rs`.

Фикстура изменилась по существу для КАЖДОГО теста, строившего актора с `share_dir = None`,
без `with_recorded_logs`, без `with_share_confirms` (= большинство из 94):
- `share_dir` реальный (`Wiring::inert`, `:4317`, `fresh_share_dir("standalone")`; каталог создаёт
  первая запись — `share_state.rs:305,554,681` `create_dir_all`): журналы, share-файлы,
  conflict-маркеры теперь пишутся; `append_journal` больше не «true без каталога».
- `ConfirmPool` реальный (`:4327`): каждый standalone-актор минтит и принимает `ShareConfirm`
  (HEAD: `pool()==None` ⇒ `mint`/`on_confirm` no-op, HEAD `:2431,2467`). В сим-сети `spawn_dealer_at`
  это новый трафик между узлами; `run_reveal_check` теперь ПРИКАЛЫВАЕТ это как факт
  (`:10886-10892`: чужой confirm в пуле node-0).
- `recorded_dkg_logs` реальный (`:4326`): `publish_recorded_logs` работает всегда.
- `agreement_tx` — parked-канал ёмкости 1 (`:4316`): первый `try_send` проходит, дальше `Full` →
  debug-строка (`:1981`), поведение не меняется.
- `changed`: `standalone_actor_at` ставит правило контракта только если бит — фикстурный
  `unreadable_bit` (`:4297`, `Arc::ptr_eq` `:6747`) — семантика HEAD сохранена; прямые сайты
  `DkgActor::new(.., Wiring::standalone())` остаются с `None` для всех эпох = HEAD `changed: None`.

Тест, чей ассерт стал слабее: **один, и он же — единственная переписанная семантика** —
`a_restart_reads_the_stored_artifact_back_into_the_actor` (`:11600`). HEAD-вариант
(`a_restart_replays…`, HEAD `:11329-11460`) утверждал «клин» (`assert_ne!(phase, "agreed")`) —
[KNOWN] при `outcome_at: None` (HEAD-тест `outcome_at` не выставлял; `grep` по телу — 0), т.е.
клин был свойством фикстуры, не продакшна. Новый тест утверждает сильнее по свойству («store
читается на тике» → `agreed` → `keyed`), но его doc-«фальсификатор» неверен — см. F-01.
Остальные ассерты — без ослабления: замены `Some(x)`→`x`, `Conflict{held,second}`→`{.., ..}`.

`Fixture`/`Drop`: [KNOWN] после `--lib` (725) и ~6 прогонов `beacon::` в мутациях —
`ls -d /tmp/beacon-dkg-restart-standalone-*` = **0**. Утечки в `/tmp` есть только у тестов, которые
называют каталог сами и чистят в конце (`nondurable-*` 1660, `append-*` 656 — HEAD-era; плюс
`conflict-keyless-*`, `drain-retry-bad-*`, `equivocator-drain-*` — от УПАВШИХ прогонов мутаций,
время совпадает с MR3/MR6; на зелёном прогоне чистятся) — §3.

### 0.4 Пять правок

**F-01 (`Conflict{key}`).** [KNOWN] Подписание стоит на всех путях: `conflict()` (`:1864-1889`)
→ `stop_signing` (`:1910`, drop share + маркер) и `Conflict{key: Some(held)}`; `recover` маркер
(`:3106-3129`) → `drop_share` + `key = stored.map(value_digest)`; `recover` divergent
(`:3131-3143`) → `stop_signing` + `key: Some(held)`; `apply_artifact` на `Conflict` (`:1646-1679`)
меняет ТОЛЬКО `key`, фаза не покидает `Conflict`, share не адоптится (нет пути к `adopt_share`).
`needs_artifact()` (`:426-437`) включает `Conflict{key: None}`; `reconcile_with_store` (`:3828`)
исключает `Conflict{key: Some}` и для `Conflict{key: None}` вызывает `apply_artifact`
(`:3839-3842`); `carries(NoArtifact) = needs_artifact()` (`:510`) ⇒ защёлка уходит с ключом.
Мутация **MR3** (`:3840` `apply_artifact` убран): тест `a_conflict_restarted…` (`:13571`) FAIL на
`key: Some` второго тика. Подпись под `Conflict` — нет (BLOCKER-класс не найден).

**E-10 (`ceremony_refusal`).** [KNOWN] Одна функция `:2998-3018`, спрашивается на live
(`:3525`) и на drain (`:2947-2953`, до `handle`, после `enter` — evidence восстановлена в `enter`
`:1373-1381` из `c.equivocations()`). Честный дилер: evidence кладётся только `note_equivocation`
(`:2611-2637`) из `c.equivocation(dealer)` — пара двух РАЗНЫХ валидных подписанных логов (ceremony
`:1011-1039` при resume, `:2617` live); честный там оказаться не может без ошибки в самой ceremony
(HEAD-код, вне А2). Кворум при одном забаненном: n=4, f=1 — три дилинга/лога = `quorum(4)=3`
(`00_preamble.md:346` то же), self-ack есть (`ceremony.rs:977-978`), reveal забаненного ≤ f.
Мутация **MR4** (`:3009` убран `| DkgBody::Ack(_)`): `beacon::` **275/275 зелёные** → ветка `Ack`
не покрыта — F-02.

**`validate_share_on_poly(.., me, ..)`.** [KNOWN] Один продакшн-вызов — `share_on_artifact`
(`:1826`, `&self.me_key.public_key()`); оба adopt-пути (`:1795`, `:2058`) идут через него. Типы:
`Share.index: Participant` (commonware `group.rs:850`, `utils/src/lib.rs:53` newtype u32,
`From<Participant> for usize` `:76`), `Set::position → Option<usize>` (`ordered.rs:83`, binary
search = seat); первая проверка `outcome.players() == committee` (`outcome.rs:112`) держит оба в
одном индексном пространстве (`:112`). Тест `outcome.rs:220` самопроверяет фикстуру.

**`restart_replay` удалён.** [KNOWN] Рестарт с артефактом на диске: тик → `decide_window`
(`:2230`) → `recover` читает `stored` (`:3105`), журнал `Present` → `resume_from_journal(..,
&agreed)` → `(true, Some(set), _) => Agreed` (`:3244`) → тот же тик `drive_finalization`
(`:2304`) и `fetch_missing_logs` (`:2360`); плюс второй рельс — `reconcile_with_store`
(`:3767`→`:3845` `apply_artifact`, `Sealed → Agreed` `:1708-1715`). Окно: `decidable_epochs`
(`:629`) = `[now−R, now] ∪ {now+1}`, R = `JOURNAL_RETENTION_EPOCHS` — то же окно, в котором
HEAD-replay через `on_artifact→decide` вообще мог что-то сделать. Переписанный тест доказывает:
после первого тика `agreed` при пустом store, после прихода тела — `keyed`, повторное чтение
идемпотентно. Чего он НЕ доказывает — что это делает именно `recover`: **MR5** (`:3199`
`agreed = None`) — тест зелёный; **MR5b** — все 95 `beacon::actor::` зелёные (F-01).

**DB-08 / F-02 / F-03.** [KNOWN] `journal_or_defer` (`:3027-3049`) на live (`:3546`) и на drain
(`:2971`); очередь `nondurable_dealings` (`:961`), ретрай `:2419-2428` (запись в ЛЮБУЮ позицию
журнала безопасна — `resume` собирает `ReceivedDealing` без учёта порядка, `ceremony.rs:986-988`).
Мутация **MR6** (`:2421` ретрай выбрасывает очередь): оба DB-08 теста FAIL. F-02: флаг
`EpochSlot.body_lost` (`:458`) ставится только в `Dealing` (`:1513-1526`), читается ровно в seal
(`:2258`), применяется ровно один раз — `Dealing` больше не входит (слот `contains_key`-guard в
`decide` `:2906`); при `agreed: Some` побеждает `Agreed` (`:2260`), флаг игнорируется — верно
(набор уже есть). Pull — тем же тиком `drive_acquisition` (`:2352`→`:3801-3806`). Мутация
**MR2** (`:2273` ветка выключена): тест `:13921` FAIL (`sealed` вместо `acquiring_…`). F-03:
`BodyMissing` снимается `clear_stall` при `all_held` (`:2704-2709`), `QuorumMissing` поднимается
только при `all_held && !ready`, обе уходят с фазой (`carries` `:496-503`). Мутация **MR1**
(`:2705` убран `clear_stall`): тест `:13979` FAIL (`{QuorumMissing, BodyMissing}`).

Мои мутации сверх M1–M6 исполнителя: MR1, MR2, MR3, MR4, MR5/MR5b, MR6 (`scratchpad/rev/MR*.txt`,
каждая — дифф + результат + `restored md5: 9cbd4715`).

### 0.5 Гигиена

[KNOWN] `#[allow]` новых нет (единственный — HEAD-era `too_many_arguments` на `new`, `:1014`;
аргументов всё ещё 12). Новых `unwrap`/`expect` вне тестов нет (grep по `+`-строкам диффа:
единственный продакшн `expect("just entered Dealing")` `:2963` — перенесённый HEAD-код, все
остальные — в `mod clock_tests` от `:4246`). `pub struct Wiring` — `mod actor;` приватный
(`mod.rs:68`), наружу не экспортируется (`git grep Wiring` вне `beacon/` — только `TeeWiring`,
чужое). Мёртвый код: `recv_or_never`, 8 builders, `restart_replay`, `journal_epochs` — удалены;
`ArtifactStore::epochs` под `#[cfg(test)]` (`artifact.rs:670-671`); `git grep` по всем именам —
ничего живого (`with_plane_clock` в `application.rs/outer.rs` — другой тип). `#[cfg(test)]
fixture: Option<Fixture>` в продакшн-типе — приемлемо (§2).

### 0.6 Доки

[KNOWN] `00_preamble.md:7-24,25-59`, `08_…md:1030,1033-1092,1100,1103`, `13_…md:502-504,
661-663,960-961`, `12_…md:71`, `CHANGELOG:16-33` — описывают код. Символьные якоря А2 верны
(`Wiring` 737, `ceremony_refusal` 2998, `journal_or_defer` 3027, `clear_stall` 1496,
`needs_artifact` 426, `nondurable_dealings` 961, `journal_failures` 1184, `recv_or_park` 655,
`body_lost` 458, `inert/standalone` 4311-4349). Номера строк в НОВОМ тексте F-01 строки 08 —
устарели (F-03 ниже). `git grep` по builders/`restart_replay`/`recv_or_never`/`gossip-only` —
живого нет: `gossip-only` `:1242,7374,7438` — отрицания про выход резолвера; `restart_replay`
`:11595` — «удалён в А2» в doc-комментарии теста.

### 0.7 Hard-stop

`DECISIONS.md`: Д-6 (локальный бан, ничего on-chain) — соблюдён (`refuse` + счётчик); Д-7 (бит
`changed`, не сравнение) — `mints_at` `:3871-3876`; П-3 (`validate_share_on_poly` перед любым
`adopt_share`) — `:2058`. BLOCKER — нет.

### 0.8 Вердикт: **КОММИТИТЬ.**

Не прочитал: `ceremony.rs` кроме `resume`/`try_ack`/`withhold_ack`/`retransmit`; `dkg_agree.rs`,
`dkg_engine.rs` (производители `artifacts_rx`/`body_lost_rx`, предусловие канала — на слово
дока `:809-814`); `log_store.rs`, `confirmations.rs` кроме `set_*`/`pool`/`mint`-шапки; тела
~80 нетронутых тестов `actor.rs` (только hunks диффа); журналы `E5-3-A2*.md`, `E4/E5-ORCHESTRATOR`
(по правилу).

### 0.9 Слабее всего (по убыванию)

1. `recover`-рельс (`(true, Some(set)) => Agreed` `:3244` и D-10 `preferred` через `&agreed`
   `:3202`) не приколот НИ ОДНИМ тестом актора — MR5b 95/95 зелёные; свойство держится вторым
   рельсом. А1-era дыра, но А2-тест объявляет ложный фальсификатор (F-01).
2. Ветка `Ack` бана E-10 не покрыта (MR4) и стоит reveal-слота — по спецификации, но без теста
   и без строки в доке о цене (F-02).
3. Якоря строк в новом тексте F-01 (`08_…md:1030`) сдвинуты на 11–87 строк (F-03).

## §1 Находки

| id | серьёзность | file:lines | что не так | чем пытался опровергнуть | уверенность |
|---|---|---|---|---|---|
| F-01 | LOW | `actor.rs:11586-11598` (doc), `:3199`, `:3244`, `:3202` | Doc теста `a_restart_reads_…` называет фальсификатором «`recover` ignoring the stored artifact»; фактически тест зелёный при `agreed = None` в `recover` — свойство держит `reconcile_with_store` (`:3845` → `:1708`). Рельс `recover` (Agreed-row + D-10 preferred) не приколот тестами актора. | MR5 (один тест) и MR5b (`beacon::actor::` 95/95) — оба зелёные; `git grep AgreedSet::of` — нет другого теста, читающего `agreed` в `recover`. Фикс: ассерт после `actor.decide(2, ..)` (как в `:12008`) до `drive_acquisition`, или честный doc. | [KNOWN] |
| F-02 | LOW | `actor.rs:3007-3016` | Ветка `DkgBody::Ack(_)` бана не покрыта; refuse ack'а эквивокатора на СВОЙ дилинг не защищает от второй полиномы (риск только у Commitment/Share), а стоит reveal-слота в своём логе (`unsent` не чистится → reveal на seal). По Д-А2-1 — спецификация; но ни теста, ни слова о цене в `08_…md:1065-1074`. | MR4: `beacon::` 275/275 зелёные без ветки. `ceremony.rs:668-670` `withhold_ack`, `:679-690` `retransmit` при отсутствии ack. | [KNOWN] |
| F-03 | LOW | `08_…md:1030` (F-01 row), `:1031`, `:1103`; `13_…md:661` | Якоря в НОВОМ тексте А2 устарели: `apply_artifact :1621`→1632, `reconcile_with_store :3737`→3824, `recover key :3008`→3128, `conflict() :1824`→1864, `stop_signing :1843`→1910, `debug_assert_settled :1435`→1445, `put_back :1428`→1438, `share_on_artifact :1779`→1819, `evict_conflict :1159`→1209, `decidable_epochs :616`→629. Символы существуют, дрейф только строк. | `grep -n` по каждому символу в текущем `actor.rs`. | [KNOWN] |
| F-04 | NIT | `actor.rs:420` | `held_digest` arm `Conflict{held,..} => Some(held)` недостижим: оба вызывающих (`:1646` перед `:1680`; `:3839` перед `:3844,3853`) разбирают `Conflict` раньше. Безвредно; вводит в заблуждение при чтении. | `grep -n held_digest` — 3 вызова, все после `Conflict`-ветки. | [KNOWN] |
| F-05 | NIT | `actor.rs:13571-14225` (6 новых тестов) | Новые тесты повторяют HEAD-паттерн «`remove_dir_all` в конце» — при панике каталог утекает (`/tmp/beacon-dkg-restart-{conflict-keyless,drain-retry-bad,equivocator-drain}-*` после MR3/MR6). Не ошибка кода; `Fixture::Drop` чистит только свой scratch. | `ls -ld --time-style` — время утечек = время упавших мутаций; на зелёных прогонах чисто. | [KNOWN] |

BLOCKER-классы постановки: продакшн-поведение против HEAD не из пяти правок — не найдено;
подпись под `Conflict` — нет; честный дилер как `equivocator` — нет; доля с чужим индексом —
закрыто `outcome.rs:116-118` + тест `:220`; регрессия живости на стенде — 58/0 ×2, 49/0 ×2.

## §2 Оставить как есть

- Д-А2-2 (`log_store`/`confirmations` внутренние `Option`, `confirmations.rs:95,103`) — за
  актором всегда `Some` (`:1060-1061`); Д-А2-3 (`standalone().changed = |_| None`) = HEAD
  `changed: None` на прямых сайтах; Д-А2-11 (свой `Fixture` без `tempfile`).
- `#[cfg(test)] fixture: Option<Fixture>` в `Wiring`/`DkgActor` (`:825-826`, `:1002-1004`):
  альтернатива — возвращать `(Wiring, Fixture)` и держать `Fixture` в каждом из 15 сайтов, причём
  для `spawn_*` — на всё время рантайма; продакшн платит одним `fixture: None` (`plane.rs:882`).
  Приемлемо.
- `recv_or_park` на закрытом plane-канале (`:655`): = HEAD; актор остаётся функционален через
  store-рельс; `resolver_rx` ⇒ `break` — решение 5.3-В.
- Refuse `Ack` эквивокатора (F-02) — по Д-А2-1; цена в пределах f. Фикс — тест + строка в 08, не
  код.
- Фикстурный `agreement_tx` ёмкости 1 (`:4316`): `Full` → debug (`:1981`), `announced` не
  ставится — только лог-строка.
- `retry_nondurable_journals` выбрасывает очередь эпохи без ceremony (`:2421-2423`): после
  finalize share есть, для `Acquiring(Logs)` точка восстанавливается из reveal дилера (ack был
  withheld) — ограничено и безопасно.
- Порядок `no_seat` до replay на drain (`:2947-2953`) — только порядок счётчика.

## §3 Вне рамок (по строке 5.3)

- Утечки `/tmp/beacon-dkg-restart-{nondurable,append}-*` (HEAD-era, 2300+ каталогов) и паттерн
  «чистка только на успехе» в тестах с собственным каталогом — общий `Drop`-guard для тестовых
  каталогов.
- 58 rustdoc-warnings крейта (private-item links в `plane.rs`, `surface.rs`, `cert_inlet.rs`…) —
  ни одного в файлах А2.
- 2 чужих clippy (`epoch_transition.rs:3024`, `node/dpos.rs:1991`), 2 чужих harness-теста
  (`test_smoke_boundary_cases.py`).
- Тестовое покрытие `recover`-строк `Agreed`/D-10 (F-01) — А1-era, закрыть отдельным тестом.
- `announce_agreement_targets` шлёт `try_send(epoch)` каждый тик для каждой `Sealed`/`Agreed`
  эпохи (`:1965-1966`) — дедуп на стороне plane; HEAD-поведение.
