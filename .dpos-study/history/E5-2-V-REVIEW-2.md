# Э5 5.2 заход В — финальное ревью (круг 4, Opus 5, свежий контекст)

Объект: рабочее дерево над `HEAD = 128e5224`, изменены только
`crates/dpos/consensus/src/testbed/{cert_inlet_tests.rs, stand.rs, tests.rs}`.
md5 сверены до начала и после каждой мутации:
`cert_inlet_tests.rs 68361290ae5298fe453f14a6df65e0d9`,
`stand.rs d347a051d7beed72d477a310ed399f52`,
`tests.rs c09a704f4a7939a8db9f35f9b13eac9b` — совпали. Пути без префикса — от
`crates/dpos/consensus/src/`; `CW` = `~/.cargo/git/checkouts/monorepo-27b478c9bb41d208/3c4e02c`.
Агенты не запускались. Git — только чтение (`git diff`, `git show`, `git archive`, `git grep`).

## §0 Прямые ответы

### 1. Ворота — все одиннадцать, мои прогоны (`CARGO_BUILD_JOBS=6`)

[KNOWN] verbatim из логов `scratchpad/rv2/*.log`:

```
$ cargo test -p fluentbase-consensus --lib
test result: ok. 676 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 174.82s
$ cargo test -p fluentbase-consensus --lib --features dpos-devnet-byzantine testbed::
test result: ok. 56 passed; 0 failed; 0 ignored; 0 measured; 629 filtered out; finished in 225.96s
$ cargo test -p fluentbase-consensus --lib testbed::
test result: ok. 47 passed; 0 failed; 0 ignored; 0 measured; 629 filtered out; finished in 170.97s
$ cargo test -p fluentbase-node --lib
test result: ok. 57 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 56.91s
$ cargo test -p fluentbase-staking-reader --lib
test result: ok. 64 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
$ cargo test -p fluentbase-consensus --test slasher_integration
test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
$ cargo clippy -p fluentbase-consensus -p fluentbase-node -p fluentbase-staking-reader --all-targets
   warning: large size difference between variants        --> crates/node/src/dpos.rs:1989:1
   warning: this `MutexGuard` is held across an await point --> crates/dpos/staking-reader/src/epoch_transition.rs:3017,3051
   — два чужих, в fluentbase-consensus ноль
$ cargo clippy -p fluentbase-consensus --all-targets --features dpos-devnet-byzantine
   0 warnings
$ cargo fmt --check ; echo $?
   0 (ни одного `Diff in`)
$ cargo doc -p fluentbase-consensus --no-deps 2>&1 | grep -c "unresolved link"
   6
$ make -C devnet/local-dpos-smoke harness-test
   2 failed, 2086 passed, 10 skipped in 4.67s
   FAILED …test_every_grepped_token_is_a_string_the_product_ACTUALLY_EMITS[EL-sync fast-forwarded the anchor]
   FAILED …test_the_two_jump_gates_are_the_values_the_product_uses
```

Расхождений с базой исполнителя нет — число в число. [KNOWN] Обе harness-ошибки
существуют и на чистом HEAD: прогон в `git archive HEAD` копии дал те же два
FAILED (`2 failed, 2084 passed, 12 skipped` — два лишних skip из-за отсутствия
gitignored-файлов в архиве).

Целевой тест, мой прогон с `--nocapture` (без фичи):

```
(5.2В/R-129) heights=[160, 160, 160, 127, 63] blocked=[0, 0, 0, 0, 0] sites=["consensus", "frontier"]
peers_blocked_families=18 block_warns=0 capture_live=true defers=33 ingests=160
drops=[("no_geometry", 0), ("out_of_window", 31), ("not_readable", 0), ("read_failed", 0)]
fired={"out_of_window"} rejected=[all 0] plane_calls=426
upstream_victim=UpstreamStats { latest_calls: 32, latest_delivered: 1, finalized_calls: 1,
finalized_delivered: 0, serve_requests: 440, deliveries_decoded: 32, deliveries_rejected: 0,
rejump_calls: 0 } virtual=159.916s
```

### 2. Может ли тест быть зелёным при сломанном свойстве — по ассертам

Свойство: на `out_of_window` шаг (5) `plane_upstream::deliver` (`plane_upstream.rs:486-490`)
дропает ответ и отвечает `true`; никакой `block`.

| ассерт | строка | конфигурация «ассерт держится, свойство нет» |
|---|---|---|
| `blocks.is_empty()` | `cert_inlet_tests.rs:1222` | [KNOWN] ЕСТЬ: слот фронтира получает не шпиона при сохранённом `at(...)` — моя мутация MA (ниже) зелёная; но это D-04 и его закрывают гейдж и WARN (MA+false — красный на `:1306`). Иной конфигурации нет: `false` ⇒ `block!` ⇒ `SpyBlocker::block` синхронно (`CW resolver/src/p2p/engine.rs:437`, `stand.rs:475-477`) |
| `excluded.is_empty()` (гейдж) | `:1306` | [KNOWN] окно: гейдж пишется только в `on_start` (`engine.rs:177`, единственный сайт), поэтому исключение в последнем арме до остановки движка невидимо. В этом прогоне движки живы до `ctx.encode()` (`stand.rs:2073`, до останова runtime). Одновременно нужно, чтобы шпион не был подключён И capture не жил — тройное совпадение |
| `block_warns.is_empty()` | `:1312` | [KNOWN] условно: только при `log_capture_live` (`capture.rs:41`, OnceLock на процесс). В крейте другой `set_global_default` нет (`git grep` по `src/` — только `capture.rs`), так что во всех трёх cargo-воротах capture жив; в моём прогоне `capture_live=true`. Зависит от текста CW (E-02, принято) |
| `rejected.is_empty()` | `:1261` | [KNOWN] ЕСТЬ: `FRONTIER_REJECTED` инкрементится ТОЛЬКО в `Self::reject` (`plane_upstream.rs:353-354`); `false`, минующий `reject` (M1/MC), оставляет множество пустым. Это «по причине», не «по факту» — факт ловят три остальных свидетеля. См. F-01: у стенда есть per-node счётчик факта, он не заассерчен |
| `fired == covered` | `:1358` | [KNOWN] нет конфигурации внутри семейства `dpos_frontier_dropped_total`; новый путь, считающий под другим семейством, невидим (докстрока это говорит, `:1136-1140`) |
| семейства `peers_blocked` | `:1281-1302` | [KNOWN] 15 семейств по имени + «хотя бы одно» `dkg_simplex_resolver`. Держится, пока в прогоне есть хоть один DKG (цель — эпоха 5, DKG с эпохи 2) |
| `sites` | `:1332` | [KNOWN] label-blind: перестановка меток между двумя `at(...)` (`stand.rs:2507`, `:2878`) оставит тест зелёным — метки нужны только для атрибуции в сообщении о падении. Не свойство, NIT |
| `out.blocked.len() == N` | `:1209` | тривиально, конфигурации нет |

**Мои мутации** (все с откатом, md5 после = объект; ни одна не совпадает с M1…M8):

- **MA** (тест-сторона, D-04 буквально): `stand.rs:2233` тип параметра → `impl Blocker<PublicKey = PeerPubkey>`, `:2507` → `{ let _ = blocker_spy.at(BLOCKER_SITE_FRONTIER); fluentbase_p2p::NoopBlocker }`. md5 `stand.rs` до `d347a051…`, под мутацией `447fa26f…`. [KNOWN] **ЗЕЛЁНЫЙ**: `sites=["consensus","frontier"]`, `blocked=[0,0,0,0,0]` — `sites` не видит подмену. Как и записано в D-04.
- **MA+false** (MA + `plane_upstream.rs:489` `return true` → `return false`; md5 `plane_upstream.rs` до `6232a651…`, под мутацией `222b7b04…`). [KNOWN] **КРАСНЫЙ** на `cert_inlet_tests.rs:1306`: `[(4, "node4_frontier_resolver_peers_blocked", 1.0)]`. Шпион слеп, ассерт (1) прошёл молча — поймал гейдж. Это и есть доказательство, что второй наблюдаемый не декоративен.
- **MB** (премисса): `cert_inlet_tests.rs:854` `HELD_LAG_NEVER` 4096 → 4 (md5 под мутацией `d8db8b18…`). [KNOWN] **КРАСНЫЙ** на `:903`: `node 4 executed into epoch 2 without a key: [160, 160, 160, 159, 159]` — разрез зажил, лаг не удержан, премисса это ловит до всех нулей.
- **MC** (`false` на шаге (5) без MA + печать WARN-свидетеля до первого ассерта; md5 `cert_inlet_tests.rs` под мутацией `d7d44672…`). [KNOWN] `block_warns=1 capture_live=true spy_calls=[0,0,0,0,1] first="commonware_resolver::p2p::engine: invalid data received peer=e695ff…"`; **КРАСНЫЙ** на `:1222` с `[(4, [("frontier", e695ff…)])] (block! WARN lines captured: 1)`. Три свидетеля согласны: одно исключение, узел 4, слот `frontier`, WARN-строка именно с тем target'ом, что в `BLOCK_WARN`.

Откат: `plane_upstream.rs 6232a651ba698ab2a91906c5fec08829`, `stand.rs d347a051…`, `cert_inlet_tests.rs 68361290…`, `tests.rs c09a704f…`; `git status` — только три файла объекта.

### 3. Честность покрытия

[KNOWN] Мой прогон: `fired={"out_of_window"}`, `defers=33 > 0`, `rotations=0`,
`drops out_of_window=31`. Атрибуция дропов жертве в тесте не заассерчена
(докстрока `:1108-1113` честно говорит), но арифметика прогона её даёт:
`upstream_victim.deliveries_decoded=32 = latest_delivered 1 + 31 дропов`, а MC
показывает первый `false` именно на слоте `frontier` узла 4. Имя, докстрока и
`COVERED` (`:1172`) согласованы с тем, что производит фикстура. Три
остальные причины видны нулями в печатной таблице, не суммированы.

### 4. D-12 — сосед байт в байт

[KNOWN] Код: `git diff HEAD` — вынос дословный, единственная новая строка в
фикстуре `cfg.metrics_snapshotter = snapshotter` с `None` у соседа, а дефолт и
был `None` (`stand.rs:561`); порядок `!out.timed_out` → три премиссы сохранён.
Прогон: HEAD-версия соседа из `git archive HEAD` (общий `target`) и рабочее дерево дают
одну строку: `heights=[160,160,160,127,63] ingests=160 teed=127 teed_top=127 window_top=127 defers=33 rotations=0 virtual=159.916s`.

### 5. `peers_blocked()` и семейства

[KNOWN] Парсер (`stand.rs:1232-1257`): `# HELP/# TYPE` дают ключ `#` и отсеиваются
суффиксом; гейдж без меток — `key value`. Семейств 18 = 5×3 именованных
(`resolver_resolver` — `stand.rs:3292` `with_label("resolver")` → `CW marshal/resolver/p2p.rs:70` `with_label("resolver")`;
`frontier_resolver` — `stand.rs:2250`; `beacon_log_resolver` — `beacon/plane.rs:327`) + 3
`dkg_simplex_resolver` (`beacon/dkg_engine.rs:333` → `CW consensus/src/simplex/engine.rs:104`).
Единственный сайт гейджа в чекауте — `engine.rs:177`. Премисса «хотя бы один» DKG
устойчива для этой фикстуры (комитет с эпохи 2 = три узла, цель — эпоха 5); какие именно
узлы дали три DKG-семейства, тест не утверждает — и не должен.

### 6. Граница и гигиена

[KNOWN] `git status` под `crates/`: только три файла объекта; `plane_upstream.rs`
после моих мутаций = `6232a651…` (исходный). `mod testbed` целиком под
`#[cfg(test)]` (`lib.rs:73-74`), все новые типы `pub(super)` — наружу ничего.
Новых `#[allow]` в диффе нет (`#[allow(clippy::too_many_arguments)]` на
`frontier_plane` — старый). `unwrap`/`expect` — только внутри `cfg(test)`.
`metrics-util` — `[dev-dependencies]` (`Cargo.toml:68,86`).

### 7. Докстрока против кода (`cert_inlet_tests.rs:1053-1152`, `stand.rs:411-433, 1192-1231`)

Проверил каждый якорь: [KNOWN] `plane_upstream.rs:140-143` (четыре причины) ✓;
`CW engine.rs:437-438` ✓; `p2p/src/lib.rs` «Bug A» — `crates/dpos/p2p/src/lib.rs:502-514` ✓;
`fetcher.rs:516` insert / `:242` filter / `:567` len ✓, удаления из `excluded` нет ✓;
`marshal/core/actor.rs:965-971` `send_lossy(true)` при `scheme == None` ✓;
`EpochSchemeProvider` переопределяет только `scoped` (`outer.rs:283-290`) ✓,
`Provider::all` дефолт `None` (`CW cryptography/src/certificate.rs:417-419`) ✓;
`beacon/plane.rs:330`, `beacon/dkg_engine.rs:343` `NoopBlocker` ✓; `engine.rs:174-178`
единственный сайт `peers_blocked` ✓; 5 с (`stand.rs:2259`) против 8 с
(`plane_upstream.rs:169`) ✓; `pop_active → add_retry` (`CW engine.rs:209-212`) ✓;
«`Self::reject` — единственный производитель `false`» — по `plane_upstream.rs:405-500`
пять `return Self::reject(..)` и два `true` ✓; `outer.rs:1267` Hybrid / `:1302` Plane /
`:1057` → `epoch_manager.rs:1750` → `engine.rs:290` ✓; `capture.rs:25` формат
`target: message` ✓. Неверных утверждений не нашёл. Одно слабое место — F-03.

### 8. Hard-stop

Решение в `DECISIONS.md` менять не нужно: заход не трогает продакшн, R-129 не
закрывает и говорит об этом. BLOCKER нет.

### 9. Вердикт

**КОММИТИТЬ** — все одиннадцать ворот число в число с базой, четыре свидетеля
согласуются между собой и с тремя моими мутациями (две красные там, где
должны, одна зелёная там, где записан принятый остаток D-04), сосед байт в
байт; единственная существенная находка (F-01) — усиление на одну строку, не
дефект.

### 10. Где проверка была слабее всего (по убыванию)

1. Мутации гонял только без фичи `dpos-devnet-byzantine`; с фичей — только чистые ворота.
2. Состав 18 семейств восстановлен арифметикой (15 + 3) и кодом меток, экспозицию по узлам глазами не смотрел.
3. Аномалия MC/MA+false: за 31 тик под `false` — ровно ОДНО исключение (шпион=1, WARN=1, гейдж=1); механизм, по которому остальные тики не дают `false`, не разобран (см. §3). На вердикт не влияет — тест ловит и одно.
4. HEAD-сосед гонял из `git archive`-копии с общим `target` — иная сборочная отпечатка, но прогон детерминирован и строка совпала.
5. `defers=33` — не проверял, откуда лишний по сравнению с `32` в записи R-129 от 09-13 (HEAD ушёл на два захода вперёд, обе версии сегодня дают 33).

## §1 Находки

| id | серьёзность | file:lines | что не так | чем пытался опровергнуть | уверенность |
|---|---|---|---|---|---|
| F-01 | MODERATE | `cert_inlet_tests.rs:1236-1268`; `fakes.rs:1526-1539, 1805-1815`; `stand.rs:2244` | Per-node счётчик ФАКТА `deliver ⇒ false` на фронтире — `out.upstream[i].deliveries_rejected` — не заассерчен ни для одного узла. Он считается в `CountingHandler` (`fakes.rs:1808-1815`), обёрнутом в `frontier_plane` безусловно (`stand.rs:2244`), и потому: (а) per-node, в отличие от процессного `rejected`; (б) слеп к обходу `Self::reject` — в отличие от `rejected` (§0.2); (в) НЕ зависит от того, что было положено в слот blocker'а — то есть закрывает остаток D-04 для фронтирной половины без шпиона вовсе. Одна строка: `for u in &out.upstream { assert_eq!(u.deliveries_rejected, 0, ..) }`. В моём прогоне на жертве `deliveries_rejected: 0` | Искал, не зависит ли счётчик от blocker'а — нет: `CountingHandler::deliver` считает возврат `inner.deliver` (`fakes.rs:1808-1815`); под MA (`NoopBlocker` в слоте) счётчик был бы ненулевым при `false`. Искал, не покрыто ли уже: `git grep deliveries_rejected cert_inlet_tests.rs` — пусто | высокая |
| F-02 | MINOR | `cert_inlet_tests.rs:1248-1268` | Ассерт «по причине» (`rejected.is_empty()`) слеп к `false`, минующему `Self::reject` (`plane_upstream.rs:353-354` — единственный инкремент); тест выживает за счёт трёх остальных свидетелей. Докстрока `:1249-1250` объявляет `reject` единственным производителем как факт кода, а не как допущение ассерта | Проверил сегодняшний код (`:405-500`): утверждение верно; MC показывает слепоту ассерта при его нарушении. Закрывается F-01 | высокая |
| F-03 | NIT | `cert_inlet_tests.rs:1098-1103` | Докстрока теста ссылается на журнал `.dpos-study/history/E5-2-V.md §0(4), mutation 2` как на измерение — не-кодовый путь в кодовой докстроке, дрейфует независимо | Журнал существует в дереве (force-added); текст говорит «journal», не выдаёт за код | средняя |
| F-04 | NIT | `stand.rs:2507`, `:2878`; `cert_inlet_tests.rs:1327-1337` | `sites` label-blind: перестановка `BLOCKER_SITE_*` между слотами оставляет тест зелёным; метки — только для атрибуции в сообщении о падении (MC: `("frontier", peer)`) | По коду `BlockerSpy::at` (`stand.rs:449-455`): метка не проверяется против места | высокая |

## §2 Оставить как есть

- D-04/D-05/D-06/E-01/E-02/E-08 — записи верны, подтверждены (MA — D-04 буквально; `capture_live=true` во всех воротах — E-01 не бьёт; target `commonware_resolver::p2p::engine` совпал в MC — E-02 держится сегодня).
- Условная ветка `else { eprintln! }` на `:1316` — конвенция модуля, capture в крейте единственный.
- «Хотя бы один» `dkg_simplex_resolver` (`:1293-1302`) — для этой фикстуры устойчиво; именовать узлы-дилеры значит завязать тест на расписание DKG, чего строка не просит.
- Рельс `dropped <= plane_calls` (`:1389-1393`) — слабый по записи, печатается, не несёт доказательства.
- Порядок «свойство до покрытия» (`:1204-1207`) — верный: ноль безусловен, премиссы дают право его читать.
- `Outcome::peers_blocked` с `panic!` на неатрибутируемое семейство (`stand.rs:1246-1249`) — правильно: молчаливый пропуск = «ничего не исключено».
- Вынос фикстуры (D-12) — дословный, сосед не изменён по прогону.

## §3 Вне рамок

- [KNOWN] Под `false` на шаге (5) за 31 пробный тик — ровно одно исключение на жертве (MC: spy=1, WARN=1, гейдж=1). Фетч нетаргетный (`plane_upstream.rs:619` `mailbox.fetch(key)`), `excluded` фильтруется в `get_eligible_peers` (`CW fetcher.rs:242`), значит ретрай должен уйти к другому пиру и тоже получить `false`. Почему этого не происходит — не разобрано; стоит один вопрос, если когда-нибудь будет строка про цену `false` на фронтире.
- Незакоммиченная правка `.dpos-study/REGISTER.md` (R-129, статус 09-14) цитирует `engine.rs:176-178`, докстрока стенда — `:174-178`; оба покрывают строку 177. Не объект ревью.
