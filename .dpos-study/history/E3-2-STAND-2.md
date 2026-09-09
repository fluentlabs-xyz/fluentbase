# Э3.2 — стенд, сессия 2: шаг 3 оценки (upstream-плоскость) + две поправки контр-ревью (2026-09-09)

Дерево `~/Work/fluentbase`, ветка `djadjka/dpos-reth-2.2-squashed`, старт с `bff5abc6`.
Commonware `v2026.4.0` (`3c4e02c`), чекаут `monorepo-27b478c9bb41d208/3c4e02c` (далее `CW:`); в паниках cargo подставляет второй чекаут `monorepo-9732103c47eb4665/3c4e02c` (тот же коммит, как и в сессии 1).
Теги: `[KNOWN]` — прочитано/выполнено в этой сессии, `[LIKELY]` — согласуется, не проверено, `[ГИПОТЕЗА]` — вывод.
Компакции контекста за сессию не было; `TASK.md` в scratchpad записан до первого вызова инструментов, перечитывать не пришлось. Длинные строки читал `sed -n`, `cat -n` и инструментом Read; `cut -c`/`head -c` не использовал.

## 1. Итог

[KNOWN] Два коммита, оба по явным путям, без трейлеров:

| Шаг | Коммит | Что |
|---|---|---|
| A — таймаут frontier-fetch по `Clock` | `68e7f7f3` `refactor(consensus): time the frontier fetch with the runtime clock` | `plane_upstream.rs` (`PlaneUpstreamHandle<E: Clock>` с полем `context`, `new(ctx, mailbox, waiters)`, `tokio::select!` над `context.sleep` вместо `tokio::time::timeout`), `crates/node/src/dpos.rs` (три типа `PlaneUpstreamHandle<Context>` + `ctx.clone()` в `new` — см. отклонение ниже) |
| B+C — upstream-плоскость в стенде, две поправки | `69a14556` `test(consensus): give the testbed an upstream plane` | `testbed/{mod,fakes,stand,tests}.rs`: вторая simulated-сеть под `FRONTIER_CHANNEL`, прод-резолвер + `PlaneUpstreamHandle` + проба executor'а на каждом узле, счётчики обоих концов, `NoUpstream` удалён, `first_divergence` детерминирован (`Divergence::{Minority, Tie}`), тест (4a) с точным числом границ, тесты (A), (4c) живой, (4d), (5′), (C1) |

[KNOWN] Отклонение от списка путей коммита A: задание допускало `plane_upstream.rs` и `Cargo.toml`; `Cargo.toml` не понадобился (tokio с `time` остаётся ради тестов `epoch_manager.rs:2553`, `beacon/keys.rs:1011-1027`), но `crates/node/src/dpos.rs` понадобился — `PlaneUpstreamHandle` получил параметр типа и третий аргумент конструктора, а узел называет этот тип в трёх местах (`:1015`, `:2029`, `:2085`) и зовёт `new` в одном (`:1821`). Без этой правки `cargo check --workspace` красный, поэтому она в коммите A; другого способа дать хэндлу часы без контекста нет (`Clock` — трейт с RPITIT, объектно не стирается; `CW:runtime/src/lib.rs:512-527`).

[KNOWN] Не сделано и почему:
- (5′) в форме задания — «линков нет — `fetch_one` таймаутится» — не воспроизводится: в разрезе 2|2 у каждой половины есть свой пир, резолвер отвечает с той же стороны (§2, §3). Тест (5′) закрепляет то, что наблюдается; живой таймаут закреплён в (4b) и (A).
- Правки `.claude/dpos_architecture/` (00, 09, 13, 15, TOC) сделаны в дереве, в коммиты не входят (`.claude/` в `.gitignore`, `.gitignore:46` по сессии 1) — как и раньше.
- Этот отчёт и строка в `PLAN.md` не закоммичены (нет в списке разрешённых коммитов).
- Пункты §10 сессии 1 п. 8, 9, 11 и ручка `fcu_pace` — не тронуты по заданию (§6).

## 2. Гипотеза §2 #3 разведки — что показал тест на старом коде

[KNOWN] Тест `a_frontier_fetch_with_no_peer_times_out_on_the_runtime_clock` (один узел на deterministic-runner, `FRONTIER_CHANNEL` на simulated-сети, прод-`commonware_resolver::p2p::Engine` + `new_bridge` с пустым `marshal_slot`, tracked set = сам узел, вызов `CertUpstream::get_latest` под `catch_unwind`) на коде `bff5abc6` (`plane_upstream.rs:280` = `tokio::time::timeout`):

```
thread '...a_frontier_fetch_with_no_peer_times_out_on_the_runtime_clock' panicked at crates/dpos/consensus/src/plane_upstream.rs:280:15:
there is no reactor running, must be called from the context of a Tokio 1.x runtime
(A) get_latest PANICKED after real 2.423729ms: there is no reactor running, must be called from the context of a Tokio 1.x runtime
```

Паника, не зависание и не «прошло»; на первом же poll (2,4 мс реального времени после старта runner'а). Гипотеза разведки подтвердилась дословно. Сессия 1 путь не достигала, потому что передавала `None::<NoUpstream>` — это тоже подтверждено: все тесты сессии 1 проходили на этом же коде (базовый прогон на `bff5abc6`: 617+3+5+13 / 0, 1 ignored).

[KNOWN] После правки тот же тест: `(A) get_latest -> None after virtual 8s, real 2.387899ms` — `None` ровно через `FRONTIER_FETCH_TIMEOUT` виртуального времени.

Попутно закрыты ещё два ожидания задания:
- **(5′) «линков нет — таймаут»: опроверглось.** [KNOWN] В разрезе `[0,1]|[2,3]` пробы срабатывают на всех четырёх узлах (по 8), и ВСЕ 8 отвечены — пиром с той же стороны разреза (каждый узел обслужил ровно столько запросов, сколько сделал сам: `serve_requests: 8`, `latest_delivered: 8`, `expired == 0`). Механизм по коду: резолвер берёт кандидатов из последнего tracked set минус себя, при таймауте (5 с в конфиге стенда и узла — `node/src/dpos.rs:1814`) штрафует и перевыбирает (`.claude/COMMONWARE_INTERNALS.md:395`, `CW:resolver/src/p2p/fetcher.rs`), а 5 с < 8 с окна `FRONTIER_FETCH_TIMEOUT`. Истекающий fetch появляется только у узла БЕЗ единого пира — это (4b): `latest_calls: 2, latest_delivered: 0` за 12 с виртуальной заморозки (первый вызов истёк через 8 с, второй выдан и висел на момент среза).
- **§2 #8 сессии 1 (`[LIKELY]` «догон через by-height резолвер»)**: [KNOWN] в (4a) — все узлы tracked, линки живые — проба узла 3 молчит (`latest_calls: 0`, `finalized_calls: 0`): выбывший узел кормит broadcast-плоскость по живым линкам, а не резолвер. Через upstream-плоскость он идёт только когда consensus-линков нет ((4c): 47 `Latest` + 12 by-height).

## 3. Тесты стенда

Все — `crates/dpos/consensus/src/testbed/tests.rs`, deterministic, seed 1, латентность 10 мс, потерь 0; реальное время — `Outcome.real_elapsed` одного прогона `--test-threads=1`, debug-профиль. Числа [KNOWN] из прогона перед коммитом B (`gatesB.txt`).

| Тест | Что проверяет | Что упало бы при нарушении | Вирт. | Реал. |
|---|---|---|---|---|
| (A) `a_frontier_fetch_with_no_peer_times_out_on_the_runtime_clock` — новый | один узел, пиров нет: `get_latest` → `None` ровно через 8 с виртуальных | паника (старый код); `Some`; вирт. ≠ 8 с; реальное ≥ 4 с (таймер на стенных часах) | 8 с | 2,8 мс |
| (1), (2), (3), (6), (7), (7′), (eq) — без изменений в ассертах; (3) сравнивает `Divergence::Minority { node: 2, height: 3 }` | как в сессии 1 | как в сессии 1 | те же | (1) 0,73 с; (2) 2,05 с; (3) 0,93 с; (7) 0,73+0,87 с; (eq) 0,75 с |
| (4a) `epoch_boundaries_pass_with_a_shrinking_committee_and_a_tracked_dropped_node_follows` — ассерт изменён | `epoch_len=5`, 4→3, все tracked, линки живые: трасса узла 0 содержит РОВНО первые блоки эпох 1, 2, 3 (`assert_eq!(.., vec![1,2,3])`), `[16,16,16,16]`; проба узла 3 молчит | граница не пройдена; четвёртая граница в трассе (предикат остановил бы позже); узел 3 стоит; две цепи | 16,1 с | 1,39 с |
| (4b) `a_node_outside_the_tracked_peer_set_stands_at_the_boundary` — конфиг и ассерты изменены | `PeerSet::Committee { upstream_link: false }` — сняты ВСЕ линки чужака: `[16,16,16,4]`; проба узла 3 стреляет и истекает по виртуальным часам: `latest_calls >= 2`, `latest_delivered == 0`, `finalized_delivered == 0` | узел 3 идёт без пиров; `Latest` пришёл без линка; ровно один вызов (зависший `fetch_one`) | 15,9 с | 1,23 с |
| (4c) `a_node_outside_the_tracked_peer_set_keeps_following_through_the_upstream_plane` — `#[ignore]` снят | `PeerSet::Committee { upstream_link: true }` — consensus-линки чужака сняты, frontier-линки к членам остались: `[16,16,16,16]`; **узел 3: `latest_calls 47 / latest_delivered 47`, `finalized_calls 12 / finalized_delivered 12`, `deliveries_decoded 59`, `rejump_calls 0`; узел 0 обслужил все 59 запросов (`serve_requests: 59`), узлы 1 и 2 — 0** | узел 3 стоит на 4 (`timed_out`); узел 3 на 16 с нулём доставок через плоскость (тогда догнал по линку и тест ничего не доказывает); ни один член не обслужил запрос; сработал re-jump | 16,1 с | 1,40 с |
| (4d) `a_node_outside_the_tracked_peer_set_with_its_links_intact_follows_the_chain` — новый | `PeerSet::CommitteeTrackedOnly` — tracked set сужен, линки все живые: РОВНО `[16,16,16,15]`; doc-комментарий объясняет, что это граница «simulated-сеть / authenticated-транспорт», а не прод-конфигурация | узел 3 стоит (тогда ручное снятие линков в `PeerSet::Committee` лишнее); другой вектор | 16 с | 1,38 с |
| (5) `a_two_two_partition_stalls_finalization_and_heals_into_one_chain` — без изменений | как в сессии 1: `[3,3,3,3]` на срезе и восстановлении, одна цепь до 12 | как в сессии 1 | 20 с | 1,22 с |
| (5′) `a_two_two_partition_is_not_bridged_by_the_upstream_plane` — новый | тот же разрез, обе плоскости: пробы стреляют (`probes > 0`, по 8 на узел), ни одна не истекает (`expired == 0` — отвечает пир с той же стороны), высоты на срезе = на восстановлении, одна цепь; реальное время < 8 с при разрезе 8,8 с виртуальных | tip двинулся в разрезе; проба не стреляет (тик executor'а мёртв); истёкший fetch (fallback не сработал); реальное время ~ виртуальному | 20 с | 1,21 с |
| (C1) `a_two_by_two_split_is_a_tie_not_a_minority` — новый | `first_divergence`: 2×2 → `Tie { height: 2, hashes: [a, b] }`; 3×1 → `Minority { node: 1, height: 2 }`; 1×1 → `Tie` | «победитель» среди равных; не та высота/хэши | — | ~0 |

Итог набора: `cargo test -p fluentbase-consensus --lib testbed -- --test-threads=1` — 14/0/0 (было 9 + 1 ignored); с `--features dpos-devnet-byzantine` — 15/0/0. Весь крейт: 622+3+5+13 / 0, 0 ignored в lib (было 617, 1 ignored).

[KNOWN] Изменение в (eq), не в ассертах: узел-эквивокатор теперь доходит до 5 (`heights=[6, 5, 6, 6]`), в сессии 1 стоял на 0 — `VoteEquivocator` подменяет движок, но executor узла 1 живой, его проба тянет честную цепь через upstream-плоскость. Ассерты (eq) не про узел 1, тест зелёный; в доке §15.a строка исправлена.

## 4. Прод-правка и доказательство нейтральности

| Файл | Изменено | Нейтральность |
|---|---|---|
| `plane_upstream.rs` | `use commonware_runtime::Clock`; `PlaneUpstreamHandle<E: Clock> { context: E, mailbox, waiters }`; `new(context, mailbox, waiters)`; в `fetch_one`: `let answer = tokio::select! { answer = rx => answer.ok(), () = self.context.sleep(FRONTIER_FETCH_TIMEOUT) => None }` — форма `beacon/artifact.rs:952-955` дословно; ветки `Some`/`None` = прежние `Ok(Ok(uf))`/`_` (та же очистка waiter'а и `cancel`); `impl<E: Clock> CertUpstream for PlaneUpstreamHandle<E>`; doc у `FRONTIER_FETCH_TIMEOUT` | [KNOWN] длительность та же (8 с), результат по таймауту тот же (`None`, waiter снят, `cancel` при пустом ключе); прежний `Err(Elapsed)` и `Ok(Err(RecvError))` оба шли в `_` → `None`, теперь `answer.ok()` даёт `None` в обоих случаях. В tokio-рантайме `Clock::sleep` = `tokio::time::sleep(duration)` — `CW:runtime/src/tokio/runtime.rs:762-764` (`impl Clock for Context`); `tokio::time::timeout` внутри — тот же `Sleep` + poll future, порядок арм в `select!` (`rx` первым) при одновременной готовности предпочитает ответ, как и `timeout` (он опрашивает future до дедлайна). `grep -rn 'tokio::time' crates/dpos/consensus/src` — в прод-коде 0, в тестах 4 (как было: `epoch_manager.rs:2553`, `beacon/keys.rs:1011,1023,1027`) |
| `crates/node/src/dpos.rs` | `:1015`, `:2029`, `:2085` — `PlaneUpstreamHandle<Context>`; `:1821-1825` — `PlaneUpstreamHandle::new(ctx.clone(), frontier_mailbox, frontier_waiters)` (`ctx: &Context`, tokio, строка `:1164`) | типовая подстановка, поведения нет |

Ворота A (на дереве коммита A: стендовые файлы временно = HEAD, свои версии в scratchpad, потом возвращены):

| Ворота | Результат |
|---|---|
| `cargo test -p fluentbase-consensus` | 617+3+5+13 / 0, 1 ignored (4c) + `print_corpus` + 1 doctest ignored — набор = база `bff5abc6` (тот же прогон до правок) |
| `cargo test -p fluentbase-node -p fluentbase-staking-reader -p fluentbase-p2p -p fluentbase-bls` | 0 failed во всех 15 бинарях |
| `cargo check --workspace` | Finished |
| `cargo clippy -p fluentbase-consensus --all-targets` | 0 warnings |
| `rustfmt --check` `plane_upstream.rs`, `node/src/dpos.rs` | чисто |

Ворота B/C (дерево коммита B): `cargo test -p fluentbase-consensus` 622+3+5+13 / 0 (lib 0 ignored); стенд 14/0/0 и 15/0/0 с фичей; четыре крейта 0 failed во всех 15 бинарях (EXIT 0); `cargo check --workspace` Finished; clippy без фичи 0 warnings, с фичей 0 warnings; `rustfmt --check` на `testbed/{mod,fakes,stand,tests}.rs` чисто.

Доки: `grep -rn 'FRONTIER_FETCH_TIMEOUT\|plane_upstream\|fetch_one\|tokio::time\|NoUpstream\|upstream: None' .claude/dpos_architecture/` — 5 попаданий до правок (`01_system_map.md:36` список модулей — верно; `08_…:2257` `decode_frontier` — верно; `09_followers.md:990,1014,1034,1071` — §9.6.1); устаревшим было одно — `09_…:1014` («bounded `FRONTIER_FETCH_TIMEOUT` (8 s)» без слова о часах, тип без параметра) — исправлено, плюс в §9.6.1 добавлен абзац «[stand, 2026-09-09]» с наблюдениями (4a)/(4b)/(4c)/(5′) и оговоркой к утверждению «nothing feeds its marshal». Ещё: `13_…` правило 23 — второе следствие (время в прод-коде только через `Clock`, grep-проверка); `15_…` §15.a — вводный абзац (две сети, плоскость, счётчики, `Divergence`), таблица (+A, 4b, 4c, 4d, 5′, C1; 4a точное число; eq — узел 1 на 5), список «не показывает»; `00_preamble.md` — запись `verified-against` за шаг 3; `TOC.md` — счётчики строк 00 (1135→1155), 09 (1307→1326), 13 (790→796), 15 (154→175). `04_cold_start…` — упоминания `PlaneUpstreamHandle` (`:14-20`) без сигнатуры, верны как были.

## 5. Что стенд НЕ показывает после этой сессии

- Reth: как в сессии 1 — R-006, R-015, R-031, R-043, R-074 (Э3.1); `FakeBeacon` всегда `Valid`.
- Контракт: путь «контракт → снимок → граница» подменён `boundary_relay` — шаг 5.
- Beacon: DKG, `AgreedArtifact`, σ через границу, `dkgQual`, дедлайны — `StaticRandomness`; R-002, R-008 — шаг 4.
- **Re-jump**: `ReJump.call` в стенде — счётный no-op `Lagging` за порогом `u64::MAX` (`rejump_calls == 0` во всех прогонах); прыжок с EL-sync (`cold_start_jump`) не моделируется — это reth. R-004 (вздутый `upstream_frontier` → прыжки на каждый tip) стендом НЕ воспроизводится: сама «проба → `fetch_max`» теперь живая (`executor.rs:1937-1939`, `upstream_frontier` в стенде — свежий `AtomicU64`), но следствие (прыжок) заглушено порогом. Воспроизводимого теста «до правки» для R-004 эта сессия не даёт; точка для него — §7.
- **R-009** (`deliver` не связывает ключ с содержимым): все доставки в этой сессии декодировались и были честными (`deliveries_rejected: 0` везде); ложный ответ не подавался. Воспроизводимого теста «до правки» нет; точка — §7. Статус записи в `REGISTER.md` не менял.
- Одна транспортная сеть: стенд держит frontier-канал на ВТОРОЙ simulated-сети, чтобы выразить «consensus-линков нет, frontier-линк есть». На authenticated-транспорте соединение одно на пира, и (4c) как конфигурация «провода» в проде не существует; в проде её аналог — зарегистрированный выбывший валидатор, чей marshal ничем не кормится (broadcast-плоскость его не достигает — находка `smoke-production-path` в §9.6.1 дока), и тогда единственный кормилец — проба. Почему в проде его не достигает broadcast, а в simulated ((4a)/(4d)) достигает — не выяснял ([ГИПОТЕЗА]: сертификаты `Recipients::All` на CERT_CHANNEL идут на подканал эпохи, у которого на выбывшем узле нет подписчика; в simulated `Recipients::All` = все зарегистрированные пиры любого tracked set, `CW:p2p/src/simulated/network.rs:633-641,702`).
- Атрибуция запросов по пирам: `serve_requests` считает `produce`, не зная отправителя; «ответил пир с той же стороны» в (5′) выведено из симметрии счётчиков (каждый узел обслужил ровно свои 8) и из (4b) (без линка доставок 0), а не измерено напрямую.
- Tracked set upstream-сети — все узлы, индекс 0, на весь прогон (в проде — `registry ∪ committee[E]` на одной сети); резолвер узла 0 при своей пробе может выбрать узел 3 кандидатом — в прогонах этого не видно (у членов `latest_calls: 0` во всех (4x)).
- R-020 write-behind, `authenticated::discovery`, слэшинг живьём, n=51, метка узла в логах — как в сессии 1.

## 6. Оставлено как есть

- `pin-project = "1.1"` в `consensus/Cargo.toml` (§10 п.11 сессии 1) — не используется, не трогал.
- `fcu_pace: Duration` — мёртвая ручка (identity), не трогал.
- §10 п.8 (два независимых пути к префиксу агрегата) и п.9 (неперечисленные упрощения: `QUOTA` без лимита, `disconnect_on_block: false`, `active_registry_peers → []`, `Address::ZERO`, `boundary_fetch/feed: None`) — не трогал; п.10 (`for_views(k)` = k leader-таймаутов) — doc-комментарий теста (5) не правил.
- `Clock::timeout` существует в трейте commonware (`CW:runtime/src/lib.rs:549-565`, сам — `select!` над `sleep`) и заменил бы `tokio::time::timeout` одной строкой; по заданию повторена форма `artifact.rs` (`select!`), не изобретал. Семантически одно и то же.
- В (4d) у узла 3 `finalized_calls: 12, finalized_delivered: 11` — один by-height fetch висел на момент среза (или истёк; не разбирал — не входит в ассерты).
- В (4b) у узла 0 `serve_requests: 1` при `latest_delivered: 0` у узла 3 — [ГИПОТЕЗА] запрос узла 3 ушёл до снятия линков на границе эпохи 1, ответ пришёлся на уже снятый линк. Не разбирал.
- Значения кадансов frontier-резолвера в стенде взяты из узла (`initial 100 мс`, `timeout 5 с`, `fetch_retry 500 мс`, `mailbox 256`, `node/src/dpos.rs:1811-1816`); QUOTA стенда — без лимита (как у остальных каналов стенда), не `FRONTIER_QUOTA` 16/с.
- `PeerSet::Committee` при `upstream_link: false` снимает frontier-линки только между чужаком и членами, как и consensus-линки; линки между двумя чужаками (их нет в текущих расписаниях) — та же логика (`inside` = оба члены).
- Точный вектор `[16,16,16,15]` в (4d) и «47/12/59» в (4c) закреплены как наблюдение seed 1; любое изменение планирования в крейте их сдвинет — это осознанно (задание просило точное ожидание для (4d); (4c) закреплён неравенствами).
- В `Outcome.upstream` нет разбивки по ключам (`Latest` vs `Finalized{h}`) на serve-стороне — не требовалось.
- Два чекаута одного коммита commonware; `.claude/session-reads` в grep; чужие fmt-хунки — как в сессии 1.

## 7. Найденное вне объёма

- [KNOWN] **Куда ставить роль 3.3 «ложный `Latest`»**: `stand.rs::frontier_plane` оборачивает прод-`FrontierHandler` в `fakes::CountingHandler` (Producer + Consumer). Роль = второй вариант обёртки, чей `produce(FrontierKey::Latest)` отдаёт пару `(fin, block)` не с `Identifier::Latest`, а с высоты `h−1` (R-009) или вообще из другого узла/эпохи; `deliver` прод-код примет (`plane_upstream.rs:201-214` — сверки ключа с высотой нет, [KNOWN] по коду). Обёртка Producer'а даётся через `Stand::node(i).role(..)` — `build_node` уже держит `role` в момент `frontier_plane`. Для R-004 (вздутый `Latest`): тот же Producer, отдающий `Finalized` с `block.height = tip + 10^6`; наблюдение — `upstream_frontier` (в стенде `Arc<AtomicU64>` в `build_node`, вынести в `NodeHandles`) и `rejump_calls` при пороге не `u64::MAX`, а прод-`min(JUMP_THRESHOLD, epoch_len)`.
- [KNOWN] Для шага 4 (beacon с живым DKG) upstream-плоскость уже даёт то, что `dpos.rs::launch` даёт beacon-плоскости: `marshal_slot` (тот же `OnceLock`, который читает DkgActor, `node/src/dpos.rs:1785-1790`) и `upstream_frontier` (`LiveFrontierTee` кормит им beacon-plane clock, `node/src/dpos.rs:765-816`).
- [KNOWN] Для шага 5 (фейк-стейкинг как машина состояний): `PeerSet::Committee` уже переотслеживает set по эпохам из расписания; при реальном `EpochTransition` источник членов для `track(epoch, ..)` надо брать из его снимков, не из `schedule(epoch)`.
- [KNOWN] Эквивокатор (`Inner::Equivocate`) с живым executor'ом теперь СЛЕДУЕТ за честной цепью через плоскость (5 из 6) — для ролей 3.3 «эквивокатор, который ещё и следует» это уже рабочая точка (сессия 1 §7 считала, что нужна другая точка подмены).
- [KNOWN] Резолвер frontier-канала выбирает одного пира и не ротирует, пока тот отвечает: в (4c) и (4d) все 58–59 запросов узла 3 обслужил узел 0, узлы 1 и 2 — ни одного (EMA по задержке, `CW:resolver/src/p2p/fetcher.rs:217-228` по `COMMONWARE_INTERNALS.md:395`). Это и есть предпосылка R-009 («самый быстрый пир»): в стенде она наблюдаема.
- [KNOWN] Утверждение дока §9.6.1 «a ROTATED-OUT validator … nothing feeds its marshal» на simulated-сети неверно для tracked-узла с живыми линками ((4a): проба молчит, узел идёт). Причина расхождения с прод-смоуком не установлена (§5).
- [LIKELY] Пробы стреляют и на здоровых узлах в lockstep-прогонах? Не проверял на (1)/(2) — счётчики там не печатаются; в (4a) у членов `latest_calls: 0` за 16 с, значит при 1 blk/s tip успевает двигаться между тиками.

## 8. Где проверка была самой слабой

1. (5′) «ответил пир с той же стороны» — по симметрии счётчиков, не по атрибуции (§5). Прямое доказательство — счётчик на `Consumer::deliver` с ключом и высотой ответа: высота ответа ≤ 3 у всех (обе стороны заморожены на 3) тоже ничего не различает. Нужен `produce` с пиром — у трейта его нет.
2. (4c) — модель «две сети» (§5): прод-фидельность конфигурации — оговорка, не факт.
3. Нейтральность select!-формы против `tokio::time::timeout` при ОДНОВРЕМЕННОЙ готовности `rx` и таймера — рассуждение по порядку арм, не тест; в проде это одна и та же длительность и один и тот же исход при любом порядке (доставка есть — `Some`).
4. (4a) «проба молчит, кормит broadcast» — `latest_calls: 0` доказывает молчание пробы; «кормит broadcast» — исключением (третьего пути нет: Hybrid отправляет все `Finalized{h}` в upstream, а `finalized_calls: 0`), сам broadcast-приём не измерял.
5. Ворота A гонялись на дереве «коммит A + HEAD-стенд»; стендовые файлы восстанавливались через `git show HEAD:… >` и обратно из scratchpad — `git status` после: только три стендовых файла `M`, `devnet/**` не тронут.
6. Числа в §3 — один прогон каждого (детерминизм подтверждён только для (6); остальные вектора — seed 1, второй прогон под фичей дал те же значения для 14 общих тестов — сверял глазами `gatesB.txt`, не diff'ом).
7. `RUST_BACKTRACE` не снимал: место паники старого кода — из сообщения rustc (`plane_upstream.rs:280:15`), стек не смотрел.

## 9. Handoff — шаг 4 (beacon с живым DKG)

- Заложено в `testbed/` под шаг 4: `marshal_slot: Arc<OnceLock<MarshalMailbox>>` на узел (`build_node`) — тот же объект, что `BeaconConfig` ждёт для DkgActor; `UpstreamCounters` и `CountingUpstream`/`CountingHandler` — образец, как оборачивать прод-объекты плоскости счётчиками без прод-правок; вторая simulated-сеть — образец для BEACON/BEACON_RESOLVER каналов (в `dpos.rs::launch` они на той же сети; для стенда решить, третья сеть или та же consensus-сеть — линки beacon-плоскости должны рваться вместе с consensus-линками, значит та же сеть).
- Временное (уходит с шагом 4): `StaticRandomness::build(CHAIN_ID, full_snapshot)`, `dkg_height_tx: None`, `agreement_intake: None`; с шагом 5: `boundary_relay`, `Committees::Schedule`, `SnapshotReader`, `PeerSet::Committee` по `schedule(epoch)`.
- Временное в upstream-плоскости: `ReJump.call` = no-op `Lagging` с порогом `u64::MAX` (когда появится фейк EL-sync — заменить на прод-`min(JUMP_THRESHOLD, epoch_len)` и настоящий `cold_start_jump_with_threshold`); `upstream_frontier` — локальный `AtomicU64` в `build_node`, не вынесен в `NodeHandles`.
- Сравнение узлов — только tier-F; `Divergence::Tie` — новый исход, в ассертах тестов пока только `None`/`Minority`.
- Прогон: `cargo test -p fluentbase-consensus --lib testbed -- --nocapture --test-threads=1` (~17 с); с эквивокатором — `--features dpos-devnet-byzantine`.

## 10. Контр-ревью

Не проводилось (по желанию в задании); все утверждения выше — свои.
