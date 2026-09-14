# E5-3-VG1 — финальное ревью захода В+Г1 строки 5.3 (Opus 5, свежий контекст, 2026-09-14)

База `HEAD = a45c458d`, ветка `djadjka/dpos-reth-2.2-squashed`, объект — незакоммиченное дерево.
md5 (первые 8) до начала и после каждой мутации: `actor.rs 77f2b43d`, `ceremony.rs edfb78c0`,
`testbed/stand.rs fc7e9627`, `testbed/tests.rs 56a995ff`, `testbed/committee_tests.rs 4a33669f`,
`dkg_transport.rs c3a67dac` — совпало; `git status` после ревью: те же 12 `M`, ничего лишнего.
Агентов не запускал. Пути без префикса — от `crates/dpos/consensus/src/beacon/`.

## §0 — прямые ответы

**1. Ворота (мои прогоны, `CARGO_BUILD_JOBS=6`, после `DONE` в `gates/p1-status.txt`)** [KNOWN]

| ворота | результат |
|---|---|
| `cargo test -p fluentbase-consensus --lib` | `688 passed; 0 failed` (182.6 s) |
| `… --lib --features dpos-devnet-byzantine testbed::` | `58 passed; 0 failed; 639 filtered out` |
| `… --lib testbed::` | `49 passed; 0 failed; 639 filtered out` |
| `cargo test -p fluentbase-node --lib` | `57 passed; 0 failed` |
| `cargo test -p fluentbase-staking-reader` | `64 passed; 0 failed` (+ `0 passed; 1 ignored` doc-target) |
| `cargo test -p fluentbase-consensus --test slasher_integration` | `16 passed; 0 failed` |
| `cargo clippy -p consensus -p node -p staking-reader --all-targets` | exit 0; 2 warning, оба чужие: `staking-reader/src/epoch_transition.rs:3024` (MutexGuard across await, тестовый код, дифф этого файла — только комментарий `:744-756`), `node/src/dpos.rs:1991` (large size difference; дифф файла — только комментарий `:1887-1893`) |
| `cargo clippy -p consensus --all-targets --features dpos-devnet-byzantine` | exit 0, 0 warning |
| `cargo fmt --check` | exit 0, `Diff in` = 0 (329 строк — только «unstable feature» предупреждения rustfmt) |
| `cargo doc -p fluentbase-consensus --no-deps` | exit 0, 58 warning; ни одно не в изменённых ханках (в 10 файлах диффа попадания только `beacon/mod.rs:2`, `:55`, `dpos.rs:1943`, `:3187` — вне ханков). Против `gates/base-doc.txt` (54, от 09-12) +4 — все в `plane.rs`/`surface.rs`, файлы этого диффа не трогают |

Измерения стенда (без фичи, `--nocapture`): stray `secondary=412 == stray_sends[4]=412, untracked=0, no_seat=0, epoch=0`;
тумбстоун `untracked=156, secondary=0, no_seat=0, observed=[[4]×5], proposals_refused_to_bind=136`;
запись `heights=[168×5], split_anchors=[2,3,4,5], straddled=[3]` (якоря эпохи 3: `64,64,63,64,64`),
`tree_only[3]=Some((113,…))`, два `JumpCall{Landed}` узла 3 (`from: 68`, `from: 116`).

**2. Таблица кадров** (получатель `now`, окно `[now, now+2]` + живые церемонии, `epoch_is_actionable` `actor.rs:2177-2182`;
отправитель на `now+d` во время своей эпохи шлёт кадры церемонии `now+d+1` и confirm на `target = now+d+1`) [KNOWN]

| d | кадр церемонии `now+d+1` | где режется СЕЙЧАС | на HEAD (`actor_head.rs:2137-2149`) | повторная доставка |
|---|---|---|---|---|
| −2 | Commitment/Share/Ack/Reveal для `now−1` | `epoch` (`:2261`), если церемония `now−1` ещё не сметена; иначе диспатч + `has_seat` | `epoch`, иначе `not_member` (`committee_for`) | не нужна: получатель эту эпоху уже завершил |
| −2 | Confirm `now−1` | `confirm_window` (`:1670`) | то же | не переиздаётся (`confirmations.rs` edge-triggered) — как HEAD, счёт для уже вошедшей эпохи ничего не решает |
| −1 | дилинг для `now` | церемония жива → `has_seat` (`:2299-2306`) → `handle`; нет церемонии → `is_bufferable=false` (`epoch <= now`, `:2146`) — тихий дроп | то же, только с `not_member` перед телом | ретрансмит дилера каждый pre-seal tick (`DkgCeremony::retransmit`) |
| −1 | Ack/Reveal для `now` | церемония жива → `has_seat` → `handle`; нет → тихий дроп | то же | Reveal — refetch по хэшу (`fetch_missing_logs`) |
| −1 | Confirm `now` | окно → `committee_for(now)` → `position` → `pool.record` | то же через `beacon_member` | — |
| 0 | дилинг для `now+1` | церемония начата → `has_seat`; нет → буфер: запись читаема → `no_seat` для чужого (`:2368-2371`), нечитаема → буфер по эпохе, `no_seat` на дрейне (`:2004-2007`) | HEAD при НЕЧИТАЕМОЙ записи отказывал `not_member` (`is_some_and` на `None`) — сейчас буферизует: строго мягче | ретрансмит |
| 0 | Ack/Reveal для `now+1` | как d=−1 | то же | как выше |
| 0 | Confirm `now+1` | принят | принят | — |
| +1 | дилинг для `now+2` | буфер (как d=0, `is_bufferable` `:2146` допускает `now+2`) | то же | ретрансмит |
| +1 | Ack/Reveal для `now+2` | нет церемонии → тихий дроп | то же (или `not_member`) | Reveal — resolver; Ack не нужен получателю без церемонии |
| +1 | Confirm `now+2` | принят | принят | — |
| +2 | всё для `now+3` | `epoch` / `confirm_window` | то же | дилинг — ретрансмит; Reveal — resolver; Confirm — не переиздаётся, как HEAD (R-126 закрыт записью) |

Поверх — пред-декодный гейт (`dpos.rs:113-125`): `untracked`/`secondary`; E-01 — как с 4.3-А. **Ни одной клетки «нужный кадр без
повторной доставки», которой не было на HEAD** — единственное отличие семантики (d=0/+1, нечитаемая запись) в честную сторону.

**3. `no_seat`** [KNOWN]. Четыре сайта, одно правило «отправитель ∈ `committee[epoch]`», но два источника одного значения:
роcтер церемонии `DkgCeremony::roster` (`ceremony.rs:195`, = `committee` из `start` `:418`/`resume` `:919`, обе через
`committee_for(target)` в `maybe_start` `:1912`) — диспатч `:2299-2306`, дрейн `:2004-2007`; `committee_for(epoch)` напрямую —
буфер `:2368`, `on_confirm` `:1673-1681`. Честный кадр `no_seat` получить не может: церемония без ростера не существует
(`start` требует `next`); resume player-only даёт тот же ростер (`:927-930`); relayed Reveal в дереве нет (`handle` на Reveal
ничего не эмитит, `:2314`; резолверный путь `on_resolver_message` через `has_seat` не идёт — все три вызова `has_seat`
в `actor.rs:2004`, `:2303`). Мутации (каждая → откат, md5 `77f2b43d`): M2 диспатч (`:2303 → false`) →
`a_ceremony_frame_…_dispatch` красный на `:5841` (`0 != 1`); M3 дрейн (`:2004`) → тот же тест, `:5804`; M4 буфер (`:2368`)
→ тот же, `:5779`; M5 `on_confirm` (`:1678`) → `a_confirmation_…_is_counted` красный на `:5629`. Тест-соседи остаются зелёными
в каждой мутации — атрибуция по сайту точная.

**4. Стенд-гейт** [KNOWN]. Окно заполняется в `TrackSink::track` (`stand.rs:1541`) ДО `manager.track`, как
`OracleHandle::track` (`p2p/src/lib.rs:314-321`); первый `record` — на `cold_start` в `build_node` (`stand.rs:2505-2510`) до
спавна стрей-дилера (`:2827`, первый кадр через 1 s). `TombstoneSet` один: `stand.rs:2473` → предикат окна `:2475-2479`,
`OuterBuilder` `:3030`, наблюдатель `:3363`; `TombstoneSet(Arc<RwLock<..>>)` (`slasher/tombstone.rs:25`). Все роли под
`Beacon::Live` идут через один `bcr = GatedReceiver::new(bcr, window, "beacon", true)` (`stand.rs:2921`) — византийские
обёртки под фичей заворачивают только sender (`TwoRevealSender`, `:2883`), единственная регистрация `BEACON_CHANNEL`
— `:2818`. Тесты фильтруют по обеим меткам (`counter_where` `stand.rs:859`, `beacon_refusals` `tests.rs:3212`).
Мутация M1 (`:2921` → без гейта, откат md5 `fc7e9627`): stray — `left (0,0) != right (412,0)` (`tests.rs:3184`), при этом
`no_seat=332` (потребитель ловит то, что гейт должен был); тумбстоун — «no beacon frame was refused as `untracked`»
(`tests.rs:3345`). Оба теста без гейта красные.

**5. Фикстура пяти узлов** [KNOWN]. Докстрока `committee_tests.rs:155-221` против кода `:223-431`: премиссы 1 (`:248-258`),
2a (`:263-275`), 2 (`:280-287`), 3 (`:291-313`), 4 (`:319-345`) и наблюдения a/b/c (`:355-410`) — те же, что в HEAD-версии
(дифф внутри тела — только `live(5,1)`, `rotate_five_four_five`, `partition(&[0,1,2,4],&[3])`, текст 2a и `eprintln`).
Измерено: `straddled=[3]`, узел 0 ∈ `C[3]` по построению `rotate_five_four_five` (`:85-91`: `3|4 => [0,1,2,4]`, иначе все) —
премисса 4 не вакуумна. `rotate_four_three_four` остался (`:67-74`) и используется только `:505`, `:655` (backfill/окно),
без `cfg.tombstoned` — не задет. Тест доказывает то же: одна запись на эпоху при разных якорях под живым тумбстоуном;
квоты в докстроке верны (`quorum(4)=3`, `quorum(5)=4`, N3f1).

**6. `refuse`/`undecodable`** [KNOWN]. `record_ingress_drop` в биконе — единственный вызов `actor.rs:2223` внутри `refuse`
(`:2213-2224`); `git grep record_ingress_drop -- beacon/` даёт только его. `undecodable` — три сайта декода
(`:2238`, `:2258`, `:2269`), `epoch: None` — первые два (нет эпохи). Что тихо и не считается: ack/reveal без церемонии,
нечитаемая запись в `on_confirm` `:1673-1675`, дубликат — как в доке `:2200-2204`; ПЛЮС три случая, которых в доке нет —
см. F-01.

**7. `break`** [KNOWN]. После `break` (`:936`) `run` возвращается: дропаются `heights`-receiver, сетевой receiver, `self`
целиком; актор ничего не спавнит (единственные `.spawn` в `actor.rs` — тесты `:3269`, `:3403`, `:3508`, `:6120`; прод-код
кончается на `:3086`). Единственный держатель sender'а — `LogHandler::new(log_resolver_tx)` (`plane.rs:737`) →
`BeaconFetchHandler::new(log_handler, ..)` (`:325`) → `consumer: handler.clone(), producer: handler` в `ResolverEngine`
(`:331-332`), `resolver_handle` = `engine.start` (`:342`) — дитя `spawn_supervisor` (`:1001-1008`), чей handle — в
`supervised` узла как `("beacon", ..)` (`node/dpos.rs:797`) → `supervise` отменяет токен (`:642-660`). `None` штатно
недостижим: движок не выходит, пока жив mailbox (актор держит клон `self.resolver`) и сеть (сеть — тоже supervised). В
тестовом харнессе `None` достижим только намеренным дропом `resolver_tx` (`:6101-6134`); остальные фикстуры передают `None`
(`recv_or_never` паркует, `:321-326`). Остаточек — F-03.

**8. `deque_size` юнит** [KNOWN]. `two_bodies_from_one_sender_…` (`dkg_transport.rs:185-264`) строит движок через
`build_body_engine` (`:220`), три тела одного отправителя с разными digest'ами (`:243-246`), `get(d0)` есть после второго,
`None` после третьего, `d1`/`d2` есть (`:249-263`). Мутация M6 `deque_size: 2 → 3` → красный на `:257` («a third body … must
evict the first»); откат md5 `c3a67dac`. Претензия дока к `propose` подтверждена: `dkg_agree.rs:1427` `certified_value`,
`None` arm → `build_proposal` (`:1453-1458`).

**9. Доки** [KNOWN]. `git grep beacon_member|window_unset|not_member -- crates/` — пусто. В `.claude/dpos_architecture/`
попадания только в блоках, помеченных историей (`00_preamble.md:59-63`, `:94-99`; `08:2792`, `:2831`; `13:1232-1233`, `:1294`).
`12:22` — `INGRESS_LOOKAHEAD_EPOCHS` с якорями `:157`/`:168` (совпадают с кодом); `15:428` — переименованная фикстура;
`00:9-45` — четвёртый круг. Устаревших утверждений не нашёл.

**10. Граница и гигиена** [KNOWN]. `git diff HEAD --stat -- crates` — ровно 10 файлов; прод-изменения только в `actor.rs`,
`ceremony.rs` (+`roster`/`has_seat`), `dkg_transport.rs` (комментарий), остальное — комментарии и `cfg(test)`. `#[allow]` в
добавленных строках нет; все `unwrap`/`expect` в добавленных строках — тесты (`actor.rs` после `:3087`, `testbed/*` под
`#[cfg(test)] mod testbed` `lib.rs:73-74`). Реэкспорты для `StrayDealer` — внутри `#[cfg(test)] pub(crate) mod testing`
(`beacon/mod.rs:167-168`) — публичная поверхность не расширена. Следов окна в акторе/`ValidatorInputs` нет
(`git grep TrackedWindow -- beacon/` пусто; `plane.rs` в диффе отсутствует).

**11. Hard-stop.** Менять решение в `DECISIONS.md` не нужно: реализованное правило (гейт — единственная классификация,
эпоха — окно `[now, now+2]`, место — потребитель) совпадает с пересмотренным решением 2. BLOCKER — нет.

**12. Вердикт: КОММИТИТЬ** — ни одна из четырёх BLOCKER-категорий не сработала (таблица кадров без новых потерь, стенд
краснеет там, где прод режет, `break` штатно недостижим, пришпиливание не тронуто), все 11 ворот зелёные, 6 мутаций
пойманы ожидаемыми тестами; находки — только документационные.

**13. Слабее всего** (по убыванию): (i) стенд не моделирует транспортное отсечение тумбстоуна (`blocker.block`,
`node/dpos.rs:1715`) — заявлено в докстроке, но «тумбстоуненный узел следует за цепью» — свойство стенда, не прода;
(ii) E-02: гейт по окну ET и потребитель по `committee_for` — два чтения, могут разойтись на боундари (записано);
(iii) `refuse` как «единственное место отказа» — при трёх незадокументированных тихих дропах (F-01); (iv) тест
`the_resolver_engines_exit_stops_the_actor` — проверяет только `run` вернулся, не реакцию супервизора (та — в `plane.rs`,
тестом не покрыта, но и не менялась).

## §1 Находки

| id | серьёзность | file:lines | что не так | чем пытался опровергнуть | уверенность |
|---|---|---|---|---|---|
| F-01 | MINOR (док) | `actor.rs:2200-2204` | Список «что НЕ считается» в доке `refuse` неполон: тихо и без счёта также (а) `on_confirm` без пула `:1635-1637`, (б) расхождение envelope/signed epoch `:1638-1646`, (в) Commitment/Share, которые `is_bufferable` отвергает (`epoch <= now`, `dealing_closed`, ключ уже в `store`, `:2131-2150`) — выпадают из `else if` `:2358` молча. Измерено в M1: без гейта 412 стрей-кадров → `no_seat=332`, `epoch=0`, 80 кадров не посчитаны нигде | искал четвёртый сайт `record_ingress_drop` — нет; проверил, что (в) не ловится ни одной меткой по коду `:2358-2385` | высокая |
| F-02 | NIT | `actor.rs:936`, `sync_metrics.rs:336-344` | После `break` `heights`-receiver закрыт, и до реакции супервизора каждый tick `dkg_height_tx.try_send` (`cert_inlet.rs:782`, `stand.rs:3259`) тикает `dpos_dkg_height_drops_total`, чей док говорит «канал полон». Окно — микросекунды до `shutdown_token.cancel()`; `error!` перед `break` уже говорит правду | искал путь, где актор жив, а resolver мёртв — нет (§0.7) | высокая |
| F-03 | NIT (док) | `committee_tests.rs:179` | «The four premises and three observations are the four-node ones» — в коде пять помеченных премисс (1, 2a, 2, 3, 4), как и на HEAD; фраза верна по смыслу, но счёт не совпадает с метками | сверил с HEAD-версией: 2a была и там | высокая |
| F-04 | INFO | `testbed/tests.rs:3078-3083`, `stand.rs:2836-2839` | Стрей-дилер шлёт на эпоху `chain.tip()/epoch_len + 1` (не через ET-геометрию) и в namespace `FLUENT_DPOS_V1_stray`. Для гейт-теста безразлично (кадр не декодируется), но обещание докстроки «a frame its actor can decode and answer» держится только пока активация = 0 и namespace не проверяется до `no_seat` | M1 показал `no_seat=332`, т.е. кадры доходят до потребителя и отвечаются `no_seat`, как обещано | средняя |

BLOCKER/SERIOUS/MODERATE — нет.

## §2 Оставить как есть

- Буфер при НЕЧИТАЕМОЙ записи (`actor.rs:2358-2385`): дилинг принимается по эпохе; HEAD отказывал `not_member`. Это мягче в
  честную сторону (start-race — ровно этот случай), слоты по-отправительски ограничены гейтом (окно ET, `members_only`),
  эвикция каждый tick `:1388-1389`.
- `has_seat` — `pub fn` при `pub(crate) struct` (`ceremony.rs:806`) — единый стиль всех методов `DkgCeremony` (`:411`…`:794`).
- `INGRESS_LOOKAHEAD_EPOCHS = 2`, `now+3` не расширять — запись R-126 (`actor.rs:145-156`) стоит.
- `refuse` с `epoch: None` только на двух сайтах до чтения эпохи — верно.
- Пред-существующие остатки E-01/E-02/E-06, D-14, D-18 — записи верны, не поднимаю.
- Асимметрия `epoch_is_actionable` (живые церемонии ниже `now`) vs `on_confirm` (только окно) — задокументирована `:1624-1633`.
- Стенд не моделирует `blocker.block` — сказано в докстроках (`stand.rs:3416-3420`, `tests.rs:3240-3242`).

## §3 Вне рамок

- Прод-EVIDENCE-гейт (`node/dpos.rs:1818`) в стенде отсутствует — не заход В.
- `dpos_dkg_height_drops_total` семантика после смерти актора (F-02) — общая для `heights.recv() → None` `:903`, не этого захода.
- +4 doc-warning'а в `plane.rs`/`surface.rs` относительно `base-doc.txt` (09-12) — другие строки Э5.
- Автомат состояний, `Option`-швы — другие заходы.
