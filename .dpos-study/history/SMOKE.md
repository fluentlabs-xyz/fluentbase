# Смоуки `devnet/local-dpos-smoke`: что они проверяют на самом деле

Дата: 2026-09-04. Ветка `djadjka/dpos-reth-2.2-squashed`, HEAD `5b5e536c`, рабочее дерево
после Э0 (`E0-LOG.md`): контрактный блоб в `devnet/local-dpos-smoke/contracts/` — сборка с
carry-over (`sha256 988955e5…`), образ `fluent-dpos-smoke:local` `1960b8481e1f`.
Продакшн-код и существующие смоуки не менялись. Все ссылки — на файлы, открытые в этой
сессии; номера строк — состояние дерева на дату.

Сокращения: `asserts.py` = `dpos_harness/cases/smoke/asserts.py`, остальные модули харнесса
— от `devnet/local-dpos-smoke/dpos_harness/`. Контракт — `~/Work/audit-482/pr482-study/contracts/staking/src`.

---

## Часть 1. Что каждый смоук упражняет

Вердикты: **поведение** — падает при поломке названного кода; **фикстура** — сравнивает
собственную копию/литерал с собой; **не может упасть** — проходит при любом состоянии кода.
Отдельным столбцом — расхождение заявленного (имя, докстринг, `Makefile`, раздел 15
архитектурной документации) с фактическим.

Общее для 25 кейсов на `driver.run` (`driver.py:1267-1361`): подъём `StaticStack.bring_up_dpos`
(`static_stack.py:231-242`: секвенсер → флеш с exit-0 → `--dpos` → сходимость всех читателей
строго за якорем), затем тела по порядку, fail-fast, затем `safety_sweep` (`driver.py:1138-1222`:
четыре детектора батареи `safety-halt / fork-detected / result-divergence /
double-finalization`, `battery.py:316`), потом teardown. Сходимость везде — `converge.aligned_reading`
(`converge.py:106-171`): каждая голова не `null`/`0x0`, строго выше пола, и хэш узла на его
собственной высоте равен хэшу продюсера на той же высоте. То есть «rejoin/reconverge» во всех
кейсах означает «на цепи validator-0 и выше пола», не «на одном tip».

| Смоук | Путь исполнения | Критерий успеха | Что должно сломаться, чтобы упал | Вердикт | Расхождение с заявленным |
|---|---|---|---|---|---|
| smoke-tx (`asserts.py:53-100`) | `cast send` перевод + `MockBlendToken.approve` → receipt.status → `wait_finalized_ge(maxblk)` на v0 → баланс и `allowance` | статусы 0x1, оба блока финализированы за 60 с, delta баланса ровно 0.1 ETH, allowance == 12345 | EVM не исполняет/не пишет SSTORE; tx-блоки не финализируются; reth не обновляет `finalized` | поведение | нет. Читает только v0 (`ctx.rpc`), т.е. финализация — с точки зрения одного узла |
| smoke-epoch (`asserts.py:120-172`) | poll `aligned_reading` пяти читателей ≥ `epoch_target` (`verdicts.py:89-98`: первый блок эпохи `min_cross` границ вперёд) → `getEpochCommittee(cur)` непуст → 60 с пейсинг 45..66 (`verdicts.py:84-86`) | все пять на цепи v0 и ≥ цели за 220 с; комитет непуст; 45 ≤ Δ ≤ 66 | остановка на границе эпохи; пустой комитет; уход от 1 blk/s в любую сторону | поведение | нет |
| smoke-vrf (`asserts.py:177-270`) | ждать `epoch_start(2)+8` → окно 8 блоков на 5 узлах (`verdicts.py:134-182`: читаемо, не ноль, побайтно одинаково, все различны) → счёт `ACTIVE_LINE` ≥5 на каждом валидаторе и рост за 3 блока → recent mixHash ⊂ залогированным v0 → `PrevRandaoProbe.snapshot()` == header.mixHash → `beacon_digest_fallback` не растёт, `beacon_seed_active` растёт | все шесть | beacon падает в `order.digest()` (ноль или один и тот же mixHash), узлы расходятся в seed, значение не доходит до EVM, метрики | поведение | нет. E1 (follower в наборе) действительно в `BEACON_NODES` (`driver.py:95-96`) |
| smoke-vrf-boundary (`asserts.py:397-453`, `beacon.py:127-253`) | окно ±6 вокруг `epoch_start(3)` на 5 узлах → ветка по `getEpochCommittee(3)` vs `(2)`: на этом стенде комитет не меняется, поэтому всегда carry-forward-ветка: рехидрейт-строка на всех 4, ни одной из 4 стадий плана для эпохи 3, non-zero mixHash внутри эпохи 3, `dpos_dkg_artifact_rejected_total == 0` | окно + карри-ветка | carry-forward ключа ломается (нулевой mixHash в эпохе 3); план спавнит церемонию на неизменном комитете; хранилище артефактов в памяти; отказ артефакта | поведение | частично. Заявлено «наблюдение плоскости соглашения» (`verdicts.py:362-391`); на стенде по умолчанию ветка `assert_agreement_plane` (четыре стадии, view==1, `pinned`) никогда не исполняется — комитет не ротируется. Ветка с церемонией живёт только в юнитах харнесса |
| smoke-base | `[assert_tx, assert_epoch, assert_vrf, assert_vrf_boundary]` на одном подъёме (`base.py:57-58`) | все четыре | см. выше | поведение | нет |
| smoke-deferred (`asserts_fault.py:78-241`) | 6 замеров `latest/finalized/safe` в полосе `[K, K+2]`, `safe−fin==K` и `latest−safe<K` хотя бы раз (`verdicts_fault.py:113-186`) → `consensus_getLatest` cgap ∈{K,K+1}, ровно K хотя бы раз → skew ≤1 → **срез артефакта N+K по фиксированному смещению** (`WIRE_HEADER_FIELDS`, `verdicts_fault.py:64-91`) == `eth hash(N)` → троттлинг v1 до 0.15 CPU на 45 с, +20 блоков, victim догоняет | все | overclaim финальности, мёртвая спекуляция, расхождение тиров, result не совпадает с исполнением, остановка при одном медленном EL | **поведение, но на момент части 1 красный по ошибке харнесса** (исправлено, часть 6.1). `WIRE_HEADER_FIELDS` содержал `fee_recipient` (20 Б); поле удалено из `OrderBlock::write` 2026-09-04 (`order_block.rs:274-284`: parent, height, proposal_view, timestamp, gas_limit, result). Смещение `result` в харнессе 168 hex, в проводе 128. `evaluate_result_commitment` найдёт хэш «в другом месте» и упадёт с `LAYOUT CHANGED`. Проверено на живом стенде `xp6` (артефакт высоты 132 по `consensus_getFinalization`, 104 байта): derived-хэш блока N лежит с байта 64 (hex 128), срез харнесса с hex 168 даёт `483ff787…` ≠ хэш. Юнит `tests/test_smoke_fault_verdicts.py:199-218` парсит `order_block.rs` и должен краснеть; `pytest` на хосте не установлен, т.е. после Э0.3 его никто не запускал ([LIKELY] по коду теста, не запускался) | докстринг раздела 15 (`15_smoke…md:49`) сам помечает числа K/K+1 как «UNVERIFIED»; фактически кейс не может дойти до них — падает раньше на срезе |
| smoke-peers (`asserts_fault.py:246-320`) | `p2p_network_tracker_directory_connected{peer}` на :19100 == committee−1 → `net_peerCount(v1)>0` → `restart v1` → те же условия плюс хотя бы один connect-timestamp выше pre-restart максимума и `finalized > pre` | все | discovery не соединяет комитет; devp2p не проброшен под `--dpos`; после рестарта соединение не пересоздаётся | поведение | нет; «chain advanced» — слабая половина (один блок), настоящая — свежий timestamp |
| smoke-vrf-fault (`asserts_fault.py:343-448`) | ждать `epoch_start(2)+8` → stop v3 → +10 блоков → окно на 3 валидаторах + follower → start v3 → догон до `a_hi` → mixHash victim == v0 на всём разрыве → `promoted to Signer` для эпохи ≥ share_epoch и нет `share-gate` (`verdicts_fault.py:1026-1064`) | все | beacon не переживает f=1; victim при догоне падает в fallback/форкается; share не перезагружается (verify-only) | поведение | нет. B3 читается по промоут-строке — прямое наблюдение |
| smoke-crash-survivor (`asserts_fault.py:795-869`) | `docker kill` v3 → `finalized ≥ pre+3` (мягко +12) → `docker start` → `aligned_reading([v0, v3], floor=head-while-down)` за 600 с | victim на цепи v0 выше высоты, достигнутой без него | узел клинится на отсутствующем блоке EL после SIGKILL; форк | поведение | нет |
| smoke-full-restart (`asserts_fault.py:917-992`) | stop всех 4 (timeout 40) → exit code 0 каждого → start → 5 читателей строго выше `pre` на цепи v0 → блок с timestamp ≥ момента рестарта за 120 с | все | не флешится (137), не сходятся с диска, «вернулись на хвосте и не производят» | поведение | нет |
| smoke-fault | `[deferred, peers, vrf_fault, crash_survivor, full_restart]` (`fault.py:144-146`) | все | см. выше | поведение (был красный из-за deferred, часть 6.1) | — |
| smoke-vrf-dkg-live-heal (`asserts_fault.py:467-772`, interval 64) | stop v3 до открытия DEAL-фазы (`epoch_start(1)−K`) → пейсинг на 3 → граница эпохи 2 на выживших + окно → start → догон → `ceremony started` от нового процесса ИЛИ heal-строка с `want==dealers` → `dpos_dkg_artifact_pull_ok_total>0` → share по одной из двух дорог → `dpos_seed_verify_ok_total≥1` → промоут → mixHash == v0 → всё ещё финализирует → `producedAt(2, v3) > 0` после конца эпохи 2 | все | live-epoch pull артефакта (FLU-1166), reveal-fallback, промоут, подпись | поведение | нет. Единственное живое покрытие reveal-fallback подтверждается самим свидетелем `want==dealers` |
| smoke-cert-follow (`asserts_follow.py:120-504`) | follower по WS → align за якорь+interval → stop/start с разрывом → back-fill → MITM-байтфлип: tamper-follower жив (`eth_blockNumber`), 45 с без прогресса `finalized`, строка отказа → фаза 4a: `PK_epoch obtained` + `dpos_follower_artifact_adopted_total` + `dpos_cert_vote_only_admissions_total` плоский 30 с при движущемся v0 и растущем follower → 4b: seed-slot MITM «CLEARED», ≥5 `BLS verify FAILED`, vote-only плоский → пейсинг | все; фазы 3/4b SKIP с exit 0 без сети (`asserts_follow.py:230-234, 387-391`) | follower принимает подделанный сертификат, не получает ключ, принимает пустой seed-слот, принимает vote-only после ключа | поведение | докстринг (`:142-156`) сам фиксирует: отказ в 4b больше не различает keyed/keyless (любой оракул отказывает пустому слоту), различает только плоский счётчик. SKIP-путь: на хосте без pip кейс зелёный на двух позитивных фазах |
| smoke-cert-keyless (`asserts_follow.py:509-672`) | follower → `vote_only_admissions ≥ 1` (предусловие) → ключ по строке + счётчику → 30 с плоский vote-only при контролях → sweep-строка после adoption | все | ключ не доходит до оракула; сертификаты принимаются vote-only после ключа | поведение | нет; предусловие делает кейс невакуумным (`verdicts_follow.py:619-654`) |
| smoke-cert-cascade (`asserts_follow.py:677-781`) | deploy `MockRollup` + `setCheckpoint(1, fin_hash)` → tier-1 с `--cert-follow.l1-rpc-url` align + строка «L1 Rollup checkpoint verified» → tier-2 align через tier-1 → bogus Rollup с `deadbeef` → строка `NOT in the local chain` + нет прогресса | все | follower следует без проверки L1-чекпойнта; каскад не сервит; bogus не отказывается | поведение, **с моком внешней системы** | «L1» — сам devnet, контракт — `MockRollup` (ABI: `getBatch`, `lastFinalizedBatchIndex`, `setCheckpoint`; читатель узла `cert_follow/l1.rs:34-35` вызывает те же две вьюхи). Паритет с настоящим rollup-контрактом на Ethereum здесь не проверяется — только чтение узла |
| smoke-tx-cascade (`asserts_follow.py:803-940`) | sentry (L2) → downstream (L3, только к sentry) → tx в L3 → receipt на v0 → финализация → receipt и state на L3 → `evaluate_no_isolated_warning` | все, кроме последнего | tx-gossip через `--trusted-only` не доходит до продюсера; L3 не синкает обратно | поведение; последняя проверка **не может упасть** (`verdicts_follow.py:971-997`: монитор `tx-route` в дереве отсутствует, строка не эмитится никем) | кейс сам печатает `NOT COVERED — tx-route ISOLATION`. Расхождения с заявлением нет, но в `smoke-all` это гарантированно зелёная проверка |
| smoke-liveness (`asserts_onchain.py:129-313`) | ждать DKG эпохи 2 → 4 цикла (v3 3I+1, v2 I+1, v1 5, v3 I+1): promote-ledger до стопа → stop → `producedAt` baseline через K+1 → +gap → delta victim == 0 при росте `blocksInEpoch` (SKIP на цикле 3) → start → `aligned_reading`+peers>0 выше `pre+gap` → промоут для эпохи ≥ floor | все | производство кредитуется стопнутому; не rejoin'ится; rejoin'ится, но verify-only | поведение | **имя/описание vs факт**: «catch-up spectrum … walks boundary-by-boundary» (`liveness.py:1-8`, докстринг `:138-149`); по признанию самого кода (`verdicts_onchain.py:246-266`) три цикла из четырёх делают re-jump, идёт только 5-блочный. Раздел 15 (`:39`) повторяет «rejoin over devp2p», про re-jump молчит |
| smoke-byzantine (`asserts_onchain.py:373-449`) | overlay `FLUENT_DPOS_BYZANTINE=equivocate` на v3 → `getValidatorStatus == 3` за 200 с → строка `severing its transport` на v0 → +3 блока за 90 с | три | слэшер/улика/тумбстоун/разрыв транспорта; остановка сразу после джейла | поведение, **но окно кончалось до фатальной границы** (продлено, часть 6.2) | Докстринг (`:393-399`) и `evaluate_post_jail_liveness` (`verdicts_onchain.py:626-652`), раздел 15 (`:41`): «committee-shrink boundary … cannot be exercised on this stand: три выживших ревёртят `ERR_COMMITTEE_TOO_SMALL`». Это ровно R-112, наблюдённое на этом же стенде: до Э0.2 сеть умирала через 35 с после тумбстоуна (`EXPERIMENTS.md` §5.3 E1), а смоук был зелёным, потому что смотрит 3 блока. После Э0.2 текст устарел дважды: коммит не ревёртит, а переносит комитет. Ни та, ни другая версия поведения на границе E+3 смоуком не наблюдается |
| smoke-cert-catchup (`asserts_onchain.py:472-715`, interval 64, netem 3000 мс) | ждать DKG → snapshot 4 счётчиков строк → stop v2 (exit 0) → +28 → start + `tc netem` → rejoin выше `pre+gap` → `PARKING derive` вырос (`executor.rs:1825`), `OuterEngine exited cleanly` не вырос (`node/dpos.rs:624`), `cannot derive beacon prev_randao` не вырос | rejoin + парк ≥1 + два отсутствия | executor завершает процесс вместо парковки; парк не срабатывает | поведение; негатив `OLD_FATAL` **не может упасть** (строки нет в дереве, кейс это знает: `verdicts_onchain.py:732`) | нет; вакуумный негатив задокументирован как tripwire |
| smoke-vrf-dkg-restart-midwindow (`asserts_onchain.py:720-882`, interval 64) | poll: журнал e2 есть И share e2 нет, до `boundary−30` → `restart v3` → `finalized < boundary` после старта → граница → `ceremony resumed from journal` + `share computed` для e2 → mixHash == v0 → `producedAt(2)>0`, нет слэша, статус ≠ 3 → финализирует | все | resume из журнала не работает (узел shareless в e2) | поведение | нет |
| smoke-rejump-signer (`asserts_boundary.py:296-344`, interval 64, gap 160) | snapshot лога+метрик → stop v2 → ждать `pre+gap` в первых 8 блоках эпохи → start → rejoin → промоут для эпохи посадки → строки `landing`/`entering the landing epoch` → промоут именно в эпохе посадки, ≥1 `proposing order block` в ней → :9100 жив → `epoch_engine_spawn_deferred_total` +≤8 → отчёт о сидинге | все | член после re-jump verify-only до следующей эпохи (пре-фикс сигнатура — промоут в E+1) | поведение | нет |
| smoke-weighted-vrf (`asserts_onchain.py:905-960`, interval 64, ×9) | стейки 9:1:1:1 прочитаны → 2 эпохи → `producedAt` всех суммируется в `blocksInEpoch`, каждый лёгкий ≥1, heavy ≥ 1.5×light_max → beacon-метрики | все | электор не взвешивает или исключает индекс; счётчики не сходятся | поведение | имя: VRF не проверяется (докстринг `:915-919` сам говорит), только взвешивание |
| growth (`cases/growth.py:345-548`, sim-стек 6/4) | register_activate ×2 с raise_cap → cap растёт, joiner в live-комитете → post-growth `finalized` +interval за 8·interval → `dpos_dkg_pinned_idx_out_of_range_total==0` на каждом валидаторе (не −1 на всех) → нет строки `names indices outside the committed committee` | все | finalize-stall при росте; pinned idx вне комитета | поведение (строка есть: `beacon/actor.rs:1504-1507`) | нет |
| quorum (`cases/quorum.py:171-327`, n=7) | stop v1..v3 (f+1) → плато `baseline_height` ровно flat за 14 с → start → каждый victim выше плато → `finalized > plateau` | все | финализация при потере кворума (safety); victim не возвращается; не восстанавливается | поведение | нет; докстринг честно: «не доказывает, что узел голосует» |
| seed-continuity (`cases/seed_continuity.py:429-612`, n=7) | stop v1 → 600 с → карта view→leader из логов → контроль: лидер view 1 каждой эпохи == оффлайн-предсказание по **константной** базе (`:562`) → ≥20 successors-of-nullified, ≥5 несовпадений | контроли ≥3 и все верны, потом B | — | **не мог пройти** на момент части 1 ([LIKELY] по коду; починено и прогнано, часть 6.3). Докстринг `:26-33, :552-561`: база с FLU-1204 — σ терминального раунда предыдущей эпохи, который не виден ни в логах, ни в RPC; скрипт предсказывает по константной базе, контроль для σ-эпох расходится → `RC_INCONCLUSIVE` (4). В `cli.py:SUITE` кейс стоит, т.е. `smoke-all` из-за него всегда ненулевой | заявлено «two-sided case»; фактически измерение заблокировано, о чём сам файл и говорит |
| turnover (`cases/turnover.py:243-469`, 8/4) | register_setkeys ×4 + batch activate → 4 делегации в начале эпохи → target = landed+4 → предпосылка: все 4 места сменились одним коммитом → `finalized` +interval → `epoch_engine_spawn_deferred_total` и `dpos_fallback_seed_constant_total` не выросли и прочитаны | все | halt на границе с нулевым пересечением; σ не наследуется (constant base) | поведение | докстринг `:19-25` устарел: «`commit_epoch_committee` reverts below [MIN] … chain stop no transaction can repair» — после Э0.2 переносит; на суть кейса не влияет |
| smoke-production-path (`asserts_prod.py:300-429`, prod-стек 6/5) | argv v0 без WS → комитет == 5 → регистрация v5 + delegate → сходимость → v5 в ahead-комитете → сходимость за границу → `promoted to Signer` у v5 → комитет ротирован → вытесненный продолжает следовать 8 с → watchdog-строка отсутствует → финализирует | все | внепроцессный join, авто-промоут, демоция | поведение | шаг 7 (liveness-ejection) снят (`:399-413`); OK-строка это отражает |
| smoke-vrf-rotation (`asserts_prod.py:478-649`) | baseline-окно эпохи ≥2 → регистрация → ротация найдена по ahead-комитету → DKG-строки v5 для E_new из follower-фазы → граница → relive-окно на 7 узлах → рост head → share-файл v5 → рост ACTIVE_LINE у всех членов → стабильная эпоха: окно carry-forward | все | beacon не переживает смену комитета; joiner без share; carry на стабильной | поведение | нет |
| smoke-vrf-dkg-halt (`asserts_prod_dkg.py:207-446`) | ротация → 2 стайера pre-seal (журнал есть, seal нет, share нет) → stop → tear журналов (readback `ffffffff`) → start → Torn-строки, без re-deal → climb до `boundary−1` → head заморожен 30 с → ни у кого share/промоута для E_new → нет паник → заморожен ещё 15 с, `finalized < boundary` | все | shareless change-epoch комитет ПЕРЕСЕКАЕТ границу | поведение (пиняет останов как норму) | нет; это негативный контроль durability |
| smoke-vrf-dkg-durability (`asserts_prod_dkg.py:463-706`) | фаза 1: v3+v4 post-seal → stop → head flat 12 с (контроль) → start → `finalized > pre_stall` → share-файлы → окно → без слэша; фаза 3: ротация → tear v3 → chain live через границу → Torn/без re-deal/без share/без промоута → 4 финализировали → окно с v3 → без паник/эквивокации → пейсинг | все | recovery share с диска; sit-out на Torn ломает кворум/цепь | поведение | нет |
| smoke-sim / -quick / -ram (`sim/`) | стохастический churn под гейтом `gate_accept` + батарея | нет нарушений инвариантов | много | поведение (soak) | гейт правилами 4b/7 (`sim/actions.py:291-316`) **запрещает** тумбстоун и добровольный выход, уводящие комитет ниже `min_committee`, т.е. sim по построению не заходит в R-111/R-112 |
| xp-up/xp-e5/xp-e2/xp-byz | стенд на N + скрипты E1/E2/E5 | нет вердикта, только наблюдение и логи | — | не смоук (инструмент) | в `Makefile:362-406` описан как «carries the scenarios», вердиктов не выносит |

Итог части 1: в `cli.py:CASES` 30 записей — 2 агрегата и 28 самостоятельных кейсов, плюс
sim и `xp-*`. Из 28: 27 проверяют поведение (одна из них, `smoke-deferred`, а через неё
`smoke-fault` и `smoke-all`, сейчас падает по ошибке харнесса; одна, `smoke-byzantine`,
заканчивает наблюдение до границы, на которой найдена R-112), одна (`seed-continuity`) не
может пройти по построению; два негатива (`OLD_FATAL` в cert-catchup, `tx-route ISOLATED` в
tx-cascade) не могут упасть и задокументированы как tripwire; sim проверяет поведение, но его
гейт исключает путь пола; `xp-*` — не смоуки. Все утверждения таблицы — по открытым в сессии
файлам ([KNOWN]), кроме двух помеченных [LIKELY].

**Проверка собственной фикстуры/мока.** Прямых случаев «литерал против своей копии» в
Python-харнессе не найдено: все вердикты читают цепь, логи или метрики. Такие случаи
живут в Rust-юнитах, на которые опираются два смоука по документации, и в реестре они
уже есть: `guard2_convergence_mismatch_engages_safety_halt` (`executor.rs:7429-7460`)
проходит на `FakeDeriver` (`executor.rs:4117-4145`), канонизирующем при derive, чего reth
не делает (R-006); `epoch_committee_return_arity_is_pinned` (`reader.rs:1343-1391`)
кодирует своим же `sol!`-типом и декодирует им же (R-119);
`commit_epoch_committee_selector_is_pinned` / `next_epoch_to_commit_selector_is_pinned`
(`evm.rs:1619-1675`) сверяют `SIGNATURE` со строкой и `SELECTOR` с hex внутри узла;
`fluent_namespace_layout_is_stable` (`bls/src/lib.rs:123-129`) — префикс со своим
литералом. Единственный мок внешней системы в смоуках — `MockRollup` (cert-cascade),
см. строку таблицы.

---

## Часть 2. Дублирование

**Настоящее дублирование (один путь, один вердикт):**

- `smoke-tx/epoch/vrf/vrf-boundary` ≡ тела `smoke-base`; `smoke-deferred/peers/vrf-fault/crash-survivor/full-restart` ≡ тела `smoke-fault`. Одни и те же объекты-функции (`base.py:57`, `fault.py:144`), пиняется `tests/test_cli_case_all.py`. `cli.py:SUITE` держит только агрегаты. Standalone-цели полезны для отладки, не для покрытия.
- Пейсинг 45..66/60 с измеряется в `smoke-epoch`, `smoke-vrf-dkg-live-heal` (на 3 выживших), `smoke-cert-follow` (с прокси и двумя follower'ами), `smoke-vrf-dkg-durability` (с torn-членом). Инструмент один (`verdicts.evaluate_pacing`), конфигурации разные, поэтому это оправдано; дублируется только `smoke-epoch` против `smoke-base`.
- Окно beacon (`evaluate_beacon_window`) — 9 вызовов в 7 кейсах. Одна функция; каждый вызов — на своей конфигурации (f=1 down, после рестарта, через границу, после ротации, с torn-членом). Оправдано.
- `still finalizing` (два чтения через 6 с) в конце 5 prod-кейсов — самый слабый критерий в дереве; дублирует пейсинг, где он есть. `durability` добавил пейсинг именно потому, что этот критерий пропускает цепь на трети скорости (`asserts_prod_dkg.py:496-500`).

**Намеренное повторение на разных конфигурациях (оправдано, чем):**

- `smoke-vrf-fault` (graceful stop, seeded epoch, 4 узла) / `smoke-crash-survivor` (SIGKILL) / `smoke-liveness` (4 глубины разрыва) / `smoke-cert-catchup` (парк под netem) / `smoke-rejump-signer` (gap > re-jump gate): всё «узел вышел и вернулся», но разные ветки кода узла: горячий share-reload, crash-recovery без флеша, re-jump+сидинг границы, парк guard #2, промоут в эпохе посадки. Различаются проверяемыми строками/счётчиками, не только gap.
- `smoke-vrf-dkg-live-heal` (victim до DEAL) / `restart-midwindow` (внутри окна, до финализа) / `durability` фаза 1 (после seal+finalize) / `halt` (2 pre-seal torn) — четыре точки одного окна церемонии, каждая ведёт в другую ветку `maybe_start` (`NoFile`/`Present`/store-hit/`Torn`). Разделение доказано в `vrf_dkg_restart_midwindow.py:432-443`.
- `quorum` (f+1=3 из 7, stall+resume) vs `durability` фаза 1 (2 из 5, stall+resume): одно свойство, но второе привязано к post-seal состоянию DKG. Частичное дублирование контроля stall — оправдано как контроль внутри чужого кейса.
- `smoke-cert-follow` фаза 4a vs `smoke-cert-keyless`: одно и то же чтение «ключ получен, vote-only плоский». Различие — предусловие keyless-окна (`vote_only ≥ 1`), которого у 4a нет. 4a без предусловия — слабее и дублирует keyless; keyless стоит оставить, 4a можно свести к контролю перед 4b.

---

## Часть 3. Непокрытое: реестр против смоуков

Только записи, подтверждённые на стенде (`эксперимент+`) или наблюдённые в ходе
экспериментов, плюс класс DUPLICATES. Для каждой: поймал бы ли хоть один существующий
смоук, и что именно мешает.

| Запись | Статус в реестре | Ловит ли существующий смоук | Почему нет |
|---|---|---|---|
| R-111 (undelegate → ревёрт коммита → все узлы мертвы) | BLOCKER, подтверждено E2/E5, исправлено Э0.2 | **нет** | ни один кейс не делает `undelegate` полного self-stake; `voluntary_exit` sim'а снимает 46e18 из 50e18 (`orchestrator.py:151-152`, `writes.py:902-910`) — частичный вывод, `full_owner_exit` не наступает, видимость не снимается; сверх того гейт исключает выход ниже пола правилом 7 (`sim/actions.py:314-316`) |
| R-112 (тумбстоун при V=4 → тот же ревёрт) | SERIOUS, подтверждено E1/E3/E4, исправлено Э0.2 | **нет** | `smoke-byzantine` смотрит 3 блока после джейла, фатальная граница на 64..96 блоков дальше; sim — правило 4b |
| R-113 (нет floor-проверки у трёх писателей штампа) | MODERATE, половина закрыта | нет | инвариант контракта; наблюдаемое следствие — то же, что R-111 |
| R-001 (прыжок к неаутентифицированному хэшу; Э-1, Э-23 подтверждены) | BLOCKER | нет | нужен пир консенсусной плоскости, отвечающий чужой парой (часть 3 EXPERIMENTS, заготовка не построена) |
| R-006 (guard #2: пуст на догоне / ложный halt) | SERIOUS, сц. 2 не воспроизвёлся | нет | сценарий 1 требует расходящегося блока (византийский предложенец). `smoke-cert-catchup` упражняет парк guard #2 (ветка absent body), не ветку result-mismatch |
| R-007 (σ только из Notarization; hold) | SERIOUS, механизм подтверждён Э-6 | нет | ни один кейс не читает `dpos_executor_eager_finalized_derive_total{outcome="miss"}`; `seed_hold_stalled_total` читает только батарея sim, и только как warn-отчёт (`battery.py:1157-1213`, `_inv_seed_watch`: «REPORT ONLY»); `docker pause` нигде не используется |
| R-010 (fee_recipient не проверяется) | SERIOUS, поле удалено Э0.3 | нет (и уже нечего) | до Э0.3 ни один смоук не читал `miner`; после — `smoke-deferred` из-за этого же удаления красный (часть 1) |
| R-085 (дубли серий метрик) | MINOR, подтверждено Э-17, исправлено Э0.4 | нет | ни один кейс не считает серии экспозиции; `assert_beacon_metrics` берёт первое значение семейства (`metric_first_val`) — на дублях читал бы одно из семи |
| R-070 (счётчики executor'а не экспортируются) | MINOR, половина закрыта Э0.4 | нет | ни один кейс не требует наличия семей `reth_dpos_executor_*` |
| R-020 (kill -9 без upstream; 0/20) | MODERATE | частично | `smoke-crash-survivor` = один SIGKILL v3, у которого есть upstream (plane-native); момент неуправляем |
| R-024 (окно деалинга при interval ≤ 20; печать закрывается сразу) | MODERATE | нет | все DKG-кейсы на interval 64 |
| R-046 (pending_boundary; interval=12, не воспроизвелось) | MINOR | нет | нет кейса с interval < 32 |
| R-101 (заморозка геометрии до активации) | MODERATE | нет | ни один кейс не переносит активацию после старта узлов |
| R-114/R-119/R-117/R-118/R-115/R-116 (DUPLICATES, 5 фатальных групп) | SERIOUS/MODERATE | **нет ни у одного** | по построению: пока копии равны, стенд здоров. Юниты пиняют копию против копии (часть 1). Класс требует теста на СОГЛАСИЕ двух объявлений, не на поведение |

Фатальные группы DUPLICATES (Corruption ⇒ abort-all у всех узлов при расхождении): №3
(селекторы `recordProduction`/`commitEpochCommittee`), №4 (арность
`getEpochCommitteeWithStakes`), №9 (`SYSTEM_CALLER`), №10 (формула эпохи), №11 (горизонт 2).
Ни одна не покрыта тестом согласия — покрыты новым `agreement_check.py` (часть 4).

---

## Часть 4. Написанные сценарии

Каталог: `devnet/local-dpos-smoke/scripts/xp/` — там, где лежит стенд на N узлов и скрипты
экспериментов части 5 (`Makefile:362-406`). Существующие файлы не тронуты; make-цели не
добавлены (это правка `Makefile`), поэтому запуск — прямой:

```
python3 scripts/xp/floor_case.py 4 exit            # R-111: E2, один честный выход
python3 scripts/xp/floor_case.py 4 byz             # R-112: E1, эквивокатор при V=4
python3 scripts/xp/floor_case.py 6 decay           # R-111: E5, V 6→5→4→3
python3 scripts/xp/agreement_check.py [--stand N]  # DUPLICATES, 12 оффлайн + 6 живых
python3 scripts/xp/metrics_dupcheck.py --stand N   # R-085 / R-070
```

(запускать из `devnet/local-dpos-smoke`; каждый `floor_case` сам поднимает и сворачивает
свой стенд `xp<N>`, `--reuse`/`--keep-up` для отладки.) Предлагаемые make-цели — в конце
части.

### 4.1 `floor_case.py` — популяция ниже `MIN_COMMITTEE_LENGTH`

Переиспользует `genN.py`, `bringupN.sh`, `byz_overlay.sh`, актуацию из `e2exit.sh`/`exits.py`
и гейт готовности из `exits.py` (первый прогон E5 был негоден из-за его отсутствия).

Что делает: после готовности DPoS снимает снапшот фатальных строк, строк
`epoch_committee_carried_over` и счётчика `dpos_epoch_committee_carried_over_total` на
каждом честном узле; актуирует (undelegate полного self-stake / оверлей эквивокации до
строки `severing its transport` с полем `epoch=`); считает фатальный блок
`F = activation + (E+1)·interval` (первый блок эпохи после снятия видимости; там коммитится
`committee[E+3]` по выборке из `E+1`) и ждёт `finalized ≥ F + 2·interval`, падая немедленно,
если любой честный контейнер вышел.

Что ловит (в каком порядке): (1) смерть узлов на F — до Э0.2 три/четыре узла умирали за
3–35 мс друг от друга; (2) **невакуумность**: на каждом честном узле обязана появиться
строка `epoch_committee_carried_over … eligible=V members=N` с `V` = ровно та популяция,
до которой кейс довёл реестр, и счётчик должен вырасти — без этого прогон, в котором
выборка не опустилась ниже пола, прошёл бы все liveness-гейты даром; (3) семантика
переноса на контракте: `getEpochCommittee(E+3) == getEpochCommittee(E+2)` побайтно и
`getDkgQual(E+3) == false` (`consensus.rs:731-783`); (4) радиус поражения N-1 из
EXPERIMENTS §3.4: full-node жив, `finalized > F` и его хэш совпадает с v0 на его высоте;
(5) темп финализации за 60 с после пола. В режиме `decay` после каждого нефатального
выхода (V ≥ 4) дополнительно контроль: переноса НЕТ (свежий короткий комитет сел).

Проверено:

| Режим | Сборка контракта | Результат |
|---|---|---|
| `4 exit` | текущая (carry-over) | **PASS**: выход в блоке 264 (эпоха 6), фатальный блок 288, `committee[9] == committee[8]` (4 места, включая вышедший 0xfaa2…), dkgQual false, все 4 + full-node живы, 60 blk/60 s |
| `4 exit` | до фикса (`bc42042a` — исходник прежнего блоба по `E0-LOG.md`; собрана в этой сессии в отдельном worktree, `sha256 30ecb41f…`, подставлена через `XP_CONTRACTS_DIR`) | **FAIL, как и должен**: выход в блоке 74, фатальный блок 96, при `finalized=92` все четыре валидатора `exited/0`, последняя строка `OuterEngine exited cleanly (unexpected)` — та же смерть, что E2 в `EXPERIMENTS.md` §5.3 |
| `4 byz` | текущая | **PASS**: тумбстоун в эпохе 0 (строка `severing its transport` на v0), фатальный блок 96, `committee[3] == committee[2]` (4 места, включая тумбстоунного), 3 честных + full-node живы, 57 blk/60 s |
| `6 decay` | текущая | **PASS**: выходы v5 (блок 75, эпоха 0), v4 (134, эпоха 2), v3 (197, эпоха 4); после первых двух контроль «переноса нет» (свежий комитет из 4 сел, `activeValidatorsLength=4` на стенде genN); третий даёт V=3, фатальный блок 224, `committee[7] == committee[6]`, все 6 + full-node живы, 60 blk/60 s. Первый прогон упал на ошибке самого кейса (ожидал `members=N`, а комитет всегда `cap=4`); исправлено и перезапущено, что и есть тот прогон |

### 4.2 `agreement_check.py` — класс DUPLICATES

Каждая проверка сравнивает два НЕЗАВИСИМЫХ объявления; ни одна не сравнивает копию с
собой. Оффлайн (без стенда): G3 — 11 `sol!`-сигнатур узла (`evm.rs`, `reader.rs`) объявлены
теми же строками в `consts.rs` контракта И их 4-байтные селекторы (little-endian) есть в
отгружаемом `.rwasm`; G4 — арность возврата `getEpochCommitteeWithStakes` по `sol!` узла vs
кортеж `write_returns` обработчика (`consensus.rs:1000`); G1 — префикс `FLUENT_DPOS_V1_`
(bls vs контракт), суффиксы (checkout commonware `scheme/mod.rs:123-125` vs
`consensus.rs:1028-1030`), `to_be_bytes` на обеих сторонах; G5–G8, G11, G13, G14 — числовые
литералы обеих сторон; G9 — `SYSTEM_CALLER` vs `fluentbase_types::SYSTEM_ADDRESS`
(`crates/types/src/lib.rs:54`); G12 — формула `(n−1)/3` у `N3f1::max_faults`
(`utils/src/faults.rs:89-95`) и `math::fault_tolerance` (`math.rs:32-38`). Живая половина
(`--stand N`/`--rpc`): L3 — `eth_call` каждой сигнатуры достигает своего обработчика
(вьюхи отвечают; системные вызовы от постороннего ревёртят `OnlySystemCall()` `0x7a8b265e`,
от `0xff…fe` — чем угодно, кроме `UnknownMethod()` `0x940e693c`/`OnlySystemCall`); L9 —
гейт системного вызывающего открыт ровно для адреса узла; L2 — peer-ключи `committee[e]`
строго возрастают побайтно (порядок `Participant` commonware: `derive(Ord)` на
`ed25519::PublicKey`, `scheme.rs:123`, `Set::from_iter_dedup` → `sort()`,
`ordered.rs:58-61`); L4 — четырёхмассивный ABI узла декодирует живой ответ; L10 —
`currentEpoch()` на выборке высот == `epoch_of_block` узла (`reader.rs:312-318`); L11 —
`nextEpochToCommit() == currentEpoch + 3` на каждой высоте ≥ activation (согласие литерала
`+ 2` в `evm.rs:988` и `MAX_COMMITTEE_LOOKAHEAD_EPOCHS` в действии).

Проверено: оффлайн 12/12 согласны; на `xp4` живые 6/6 согласны (18 проверок, 0 unread).
«Падает до фикса» здесь не применимо — расхождений в дереве нет; чем отличит: любое из 18
сравнений при сдвиге одной стороны (например, снижение `MAX_COMMITTEE_LOOKAHEAD_EPOCHS`
до 1 — L11 и G11; переименование в контракте — G3 по блобу и по исходнику; три массива
вместо четырёх — G4 и L4). Ограничения: контрактный исходник берётся из соседнего
worktree (`STAKING_CONTRACT_SRC`), не из этого дерева; блоб проверяется на наличие
селектора, не на диспетчеризацию — её закрывает L3, которому нужен стенд; порядок
commonware читается из checkout, пиннутого `Cargo.lock`.

### 4.3 `metrics_dupcheck.py` — R-085 / R-070

Скрейпит оба реестра каждого валидатора стенда и считает одинаковые ключи
`name{labels}` (вычисление Э-17, `e17_dupcheck.py`), плюс требует семей
`reth_dpos_executor_*` на реестре reth. Проверено на `xp4`: 0 коллизий на 8 экспозициях
(8654/8654, 3744/3744 …), по 3 семьи executor'а на каждом reth-реестре. До Э0.4 давало 223
коллидирующие серии (55,6 % сэмплов) — измерено в Э-17 на том же стенде; «падает до
фикса» подтверждено этим измерением, не повторным прогоном (образ до Э0.4 не
сохранён).

### Предлагаемые make-цели (не применены)

```
floor-exit:   ; @python3 scripts/xp/floor_case.py 4 exit
floor-byz:    ; @python3 scripts/xp/floor_case.py 4 byz
floor-decay:  ; @python3 scripts/xp/floor_case.py 6 decay
agreement:    ; @python3 scripts/xp/agreement_check.py $(if $(N),--stand $(N),)
metrics-dup:  ; @python3 scripts/xp/metrics_dupcheck.py --stand $(N)
```

---

## Часть 5. Что стоило бы написать, но нельзя без правки продакшн-кода или дорогого стенда

| Сценарий | Запись | Что нужно |
|---|---|---|
| Пир консенсусной плоскости, отвечающий на `Latest`/`Finalized{h}` чужой парой (Э-3/Э-4/Э-7) | R-001, R-004, R-009 | клиент commonware-p2p с ключами стенда (заготовка из EXPERIMENTS §3.3, 1–2 дня); прозрачный прокси невозможен — плоскость аутентифицирована и шифрована |
| Расходящийся по результату блок для guard #2 (сц. 1 R-006) и византийский dealer с двумя `Reveal` (Э-22) | R-006, R-002 | новый режим `--dpos.byzantine` в `crates/` — правка продакшн-кода |
| σ-hold > f узлов одновременно (последствие R-007) | R-007 | комитет ≥ 7 и одновременная пауза двух узлов; на 4 узлах неотделимо от потери кворума. Стенд `xp7` есть, но нужен ещё и способ наблюдать `awaiting_seed` (только reth-реестр, `dpos_executor_seed_hold_stalled_total`) — можно написать без правки кода, не написан из-за неподтверждённости последствия |
| Три точных момента kill -9 (R-020) | R-020 | инструментированный узел (синхронизация в критических секциях) |
| Follower с upstream, подменяющим σ до прихода `PK_E` (Э-21) | R-008 | `cert-mitm-proxy.py` уже умеет `seed-slot` (очистка); нужен третий режим — подстановка σ предыдущего сертификата (docstring `asserts_follow.py:158-165` описывает); это правка скрипта-сайдкара, не продакшн-кода, но и не «одна команда» без разбора WS-кадра |
| DKG при n=51 и WAN (Э-12), ресурсные Э-8/9/13/16 | R-024, R-013, R-037, R-038, R-067 | стенд на 51 узел + netem; или bench-крейт |
| Перенос активации governance после старта узлов (R-101) | R-101 | стенд `bare` с runtime-деплоем и двумя `setDposActivationBlock` — на prod-стеке возможно без правки кода, не написан: следствие (раскол по геометрии) нужно ещё смоделировать |
| `smoke-byzantine` до границы E+3 на стенде по умолчанию | R-112 | это `floor_case.py 4 byz`; переписать существующий кейс нельзя по условию задачи |
| Исправление `WIRE_HEADER_FIELDS` в `verdicts_fault.py` | — | правка существующего смоука; без неё `smoke-deferred`/`smoke-fault`/`smoke-all` красные на текущем дереве |
| `seed-continuity`: источник σ терминального раунда для предсказателя | — | лог-строка с `sigma.signature` на границе — правка продакшн-кода |

---

## Часть 6. Правки смоуков (2026-09-04, второй заход)

Три работы внутри `devnet/local-dpos-smoke`; `crates/` не тронуты. Юнит-набор харнесса
теперь запускается одной командой `make harness-test` (throwaway-контейнер
`python:3.12-slim`, `pip install pytest pyyaml`, монтируется корень репозитория, потому что два
теста читают `crates/`; запуск от пользователя хоста, так как два теста пишут в дерево).
Требует сети для одного `pip install`; итог на этом дереве — 2083 passed, 10 skipped.

### 6.1 `WIRE_HEADER_FIELDS` после Э0.3

- `verdicts_fault.py`: из списка убран `fee_recipient`; смещение `result` теперь 128 hex,
  минимум провода 192 (было 168/232). Добавлены `WIRE_TYPE_BYTES`, `ORDER_BLOCK_SOURCE`
  (путь к `order_block.rs`, переопределяется переменной окружения того же имени),
  `wire_layout_from_source()` — парсер структуры `OrderBlock` и тела `write` (порядок берётся
  из `write`, ширины из типов полей; первое не-fixed выражение завершает префикс; отсутствие
  `result` или неизвестный тип — `ValueError`), и `evaluate_wire_layout()` — вердикт
  «копия == исходник».
- `asserts_fault.py::_assert_result_commitment`: перед срезом читает `order_block.rs` и
  проверяет раскладку через `evaluate_wire_layout`; при расхождении кейс падает с
  сообщением о харнессе («NOT sliced»), не с фиктивной result-divergence; нечитаемый исходник
  — тоже падение.
- Юниты (`test_smoke_fault_verdicts.py`): числа 128/192, парсер на заглушке исходника,
  добавленное поле → drift, убранное поле → drift, неизвестный тип/пустой исходник/нет
  `result` → не проходит. Существующий `test_the_field_list_matches_the_rust_codec` теперь
  зелёный.

Чем проверено: (1) живой артефакт `xp6` (часть 1): хэш блока N лежит с байта 64; (2)
`ORDER_BLOCK_SOURCE=<копия с добавленным `fee_recipient: B256`>` и `<копия без
`timestamp`>` — обе копии дают `drifted`, реальный исходник — `(True, '')`, через ту же
функцию `_read_order_block_source`, которой пользуется кейс; (3) `make harness-test` зелёный;
(4) `smoke-deferred`/`smoke-fault` — см. итог полного набора ниже.

Не выбран вариант «генерировать список из исходника при импорте»: харнесс тогда не
импортируется без `crates/`, а дефект был не в наличии копии, а в отсутствии выполняемой
сверки — она теперь выполняется в самом кейсе и в контейнерном юните.

### 6.2 `smoke-byzantine` до границы пола

- `asserts_onchain.py::assert_byzantine`, шаг 4: снапшот фатальных строк и строк
  `epoch_committee_carried_over` на честных узлах берётся до ожидания джейла; эпоха джейла
  читается из поля `epoch=` строки `severing its transport` (`node/src/dpos.rs`: эпоха
  finalized-блока, на котором прочитан тумбстоун); цель ожидания —
  `epoch_start(E+2) + interval` (`vo.carry_wait_target`; покрывает фатальный первый блок
  эпохи E+1 при обоих прочтениях эпохи и ещё одну эпоху перенесённого комитета), бюджет
  `3·interval + 120` с. Во время ожидания каждый честный контейнер обязан быть `running`
  (`ctx.ps_state`, новый read в `driver.py`); выход хотя бы одного — немедленный FAIL с
  именем ветки R-112 и последними фатальными строками (`vo.evaluate_honest_alive`).
  Затем: на каждом честном узле дельта строк `epoch_committee_carried_over` содержит
  `eligible=3 members=4` и все узлы называют одну целевую эпоху (`vo.evaluate_carried_over`,
  анти-вакуумный гейт); `getEpochCommittee(t) == getEpochCommittee(t−1)` побайтно и
  `getDkgQual(t) == false` (`vo.evaluate_carried_committee`).
- Переписаны: докстринг кейса (`byzantine.py`), докстринг `assert_byzantine`, докстринг
  `evaluate_post_jail_liveness`, заголовок пакета `cases/smoke/__init__.py`, комментарий
  `Makefile` о «permanently shrunk», строка `case-byzantine` в разделе 15 документации.
- `docker-compose.yml`: `${SMOKE_CONTRACTS_DIR:-./contracts}:/contracts:ro` — переменная, чтобы
  поднимать стенд по умолчанию на другом наборе артефактов (так проверено «до фикса»).
- Юниты: `test_smoke_onchain_verdicts.py` (эпоха из строки, цель ожидания, парсер строк
  переноса, три вердикта в обе стороны), `test_smoke_onchain_cases.py` (здоровый мир проходит
  сквозь границу; узлы `exited` на границе → FAIL с «R-112»; граница без строки переноса →
  FAIL; перенесённый комитет не копия → FAIL).

Чем проверено на стенде по умолчанию (4 валидатора, interval 32):

| Сборка контракта | Результат |
|---|---|
| текущая (carry-over) | PASS: джейл прочитан в эпохе 0, фатальный блок 96, `committee[3]` перенесён из `[2]`, dkgQual false, все честные узлы живы, finalized 162 ≥ 160; safety sweep OK |
| `bc42042a` (`SMOKE_CONTRACTS_DIR` на блоб до фикса) | FAIL: на блоке 96 `commitEpochCommittee(epoch 3) did not succeed: Revert 0x0a87ec8d(3,4)` → `Corruption` → v0/v1/v2 `exited` в пределах 21 мс; вердикт называет ветку R-112 |

Длительность: наблюдение добавляет до трёх интервалов (≤96 блоков ≈ 100 с); полный кейс
≈ 6–7 мин с подъёмом. Остаётся в `SUITE`.

Попутно: правка `.dpos-study/*.md` сбрасывает кэш слоя `COPY` образа (в `Dockerfile` исключены
только `.git`, `dist`, `devnet`), поэтому первый `make smoke-*` после правки документа
пересобирает `fluent` (~5 мин на тёплом cargo-кэше). Не менял — вне трёх работ.

### 6.3 `seed-continuity`

σ наблюдаем после FLU-1204: он едет в каждом сертификате финализации beacon-активной эпохи —
хвост `seed_flag(1 Б) ‖ σ(48 Б)` (`combined_scheme.rs::write_seed_slot`; тот же хвост чистит
`cert-mitm-proxy.py` в режиме `seed-slot`), а `consensus_getFinalization {"height": h}` отдаёт
этот сертификат для любой финализированной высоты. База seedless-ветви эпохи E —
`sha256(σ)` терминального раунда эпохи E−1 (`beacon/seed.rs::witness_fallback_seed`,
`epoch_manager.rs::boundary_base`), то есть σ из сертификата последнего блока эпохи E−1.
Константная база — только когда предшественник не beacon-активен
(`mandatory_at`: `epoch ≥ DETERMINISTIC_BOOTSTRAP_EPOCH = 2`).

- `cases/seed_continuity.py`: `terminal_seed_from_certificate()` (хвост 49 байт, флаг обязан
  быть 1 — сертификат без σ отвергается, а не хэшируется), `epoch_base_kind()`,
  `terminal_seed_base()` (RPC-чтение терминального блока), и выбор базы per-epoch по той же
  ветке, что `seedless_base` узла. Нечитаемый σ → `RC_INCONCLUSIVE`, не тихий откат на
  константу. Докстринг модуля переписан.
- Юниты (`test_case_seed_continuity.py`): срез хвоста, отказ на флаге 0/коротком
  сертификате, выбор ветви по эпохе.

Живой прогон (`make case-seed-continuity`, стенд sim n=7, окно 600 с): **PASS** — 18
позитивных контролей совпали (view 1 эпох 2..20; эпоха 2 на константной ветви, 3..20 на
witness-ветви — база каждой прочитана из сертификата терминального блока, например эпоха 11:
высота 447, view 37), 71 из 79 лидеров после нуллифицированного view отличаются от
предсказания (доля совпадений 0,10 при ожидаемой ~1/7). До правки тот же кейс на том же коде
давал INCONCLUSIVE: контроли witness-эпох расходились по построению. Кейс остаётся в `SUITE`
как рабочий гейт. Что кейс не покрывает — как и прежде: корректность σ как пороговой подписи
и межузловое согласие лидера при медленном (не мёртвом) лидере.

### 6.4 Полный набор после правок (`make smoke-all`, 2026-09-04)

Один прогон `SUITE` (21 кейс), затем три отдельных перепрогона упавших кейсов на том же
коде. Логи: `scratchpad/smoke-all.log`, `scratchpad/reruns.log`.

| Кейс | Вердикт | Что показал |
|---|---|---|
| smoke-base | PASS | |
| smoke-weighted-vrf | PASS | |
| smoke-fault (вкл. deferred) | PASS | сверка `WIRE_HEADER_FIELDS` с `order_block.rs` прошла (6.1) |
| smoke-cert-cascade | PASS | |
| smoke-tx-cascade | PASS | |
| smoke-byzantine | PASS | committee[3] перенесён из committee[2], финализация 162 ≥ 160 (6.2) |
| smoke-cert-catchup | PASS | |
| smoke-vrf-dkg-restart-midwindow | PASS | |
| growth | PASS | |
| quorum | PASS | |
| seed-continuity | PASS | 18 контролей, 71/79 расхождений (6.3) |
| turnover | PASS | |
| smoke-rejump-signer | FAIL ×2 | validator-2 промоутился в посадочную эпоху 5 (landing=452, запас 59 блоков), 0 предложений за эпоху |
| smoke-liveness | FAIL ×2 | 1-й прогон: цикл 2 (v2 down) — цепь не дошла до 247 сразу после «v3 SIGNING again»; перепрогон: цикл 3 (v1 down) — finalized 279 → 279 после того, как v3 и v2 оба вернулись и «SIGNING again» |
| smoke-vrf-dkg-live-heal | FAIL ×2 | validator-3 пересчитал долю (`share recompute-heal epoch=2 want=3 dealers=3`), prev_randao байт-в-байт со survivors, посажен на 273/274 при 47/46 блоках запаса, `producedAt=0` |
| smoke-cert-follow | ERROR | follower не поднялся: хост-порт 29100 занят процессом VS Code (pid 3639366); оверлей публикует `29100:9100` (`docker-compose.cert-follow.yml:83`) |
| smoke-cert-keyless | ERROR | та же причина (тот же оверлей) |
| smoke-prod (4 кейса production-path) | ERROR | bring-up падает на `forge create contracts/staking/mocks/MockBlendToken.sol:MockBlendToken`: в соседнем checkout `solidity-contracts` (HEAD `ae429e0`, 2026-09-02) файла нет (`find` по `MockBlendToken.sol`/`BLS12381Verifier.sol` пуст, `forge build` там зелёный) — путь в `TOKEN_CONTRACT` харнесса отстал от репозитория контрактов |

Итог: 12 PASS / 3 FAIL / 6 ERROR.

**Три FAIL воспроизводятся детерминированно** (два прогона из двух) и имеют одну сигнатуру:
вернувшийся после остановки валидатор промоутируется в Signer (лог «SIGNING again», доля
пересчитана, prev_randao согласован), но за целую эпоху не ведёт ни одного view. В
`smoke-liveness` это проявляется косвенно: пока вернувшийся член — «мёртвый вес», остановка
ещё одного валидатора оставляет 2 живых голоса из 4, и цепь стоит. Ни один из трёх кейсов не
пересекается с правками 6.1–6.3 (харнесс fault/deferred, byzantine, seed-continuity); их
верификаторы не менялись, упавшие утверждения — про поведение узла (`crates/`), которое по
границам задачи не трогается. Что это — регрессия узла после Э0.x или давняя дыра, — здесь не
установлено: прогонов на более старом бинаре не делалось.

**Шесть ERROR — окружение, не харнесс и не узел.** Два первых снимаются освобождением порта
29100 (или параметризацией хост-порта в оверлее), четыре последних — указанием
`SOLIDITY_CONTRACTS_DIR` на checkout, где ещё есть `contracts/staking/mocks/`, либо
переводом bring-up на вендоренный артефакт `MockBlendToken.json` вместо `forge create` из
исходника. Ни то ни другое в объём трёх работ не входило и не делалось.

**Что теперь красное на старом коде и зелёное на новом:**
- `smoke-deferred`/`smoke-fault` — с прежним `WIRE_HEADER_FIELDS` (с `fee_recipient`) сверка
  с `order_block.rs` и юнит-тесты красные; с исправленным — зелёные. Искусственный сдвиг в
  копии исходника (добавленное `fee_recipient: B256`, убранный `timestamp`) даёт «drifted».
- `smoke-byzantine` — на блобе `bc42042a` (до Э0.2) FAIL с именованием R-112 (v0/v1/v2
  `exited`, ревёрт `CommitteeTooSmall(3,4)` на блоке 96); на текущем коде PASS.
- `seed-continuity` — на текущем коде до правки INCONCLUSIVE (witness-контроли расходились
  по построению), после — PASS.

Стенды свёрнуты: `docker ps -aq` пуст.
