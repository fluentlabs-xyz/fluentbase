# Ревью 4.2 заход Б2 — «Правило единое», якорь холодного старта, отброс вне окна, снятие стадий

Ревьюер: Opus 5, свежий контекст, не писал этот код. База `HEAD = f8ec4939` (`git rev-parse HEAD`
прогнан мной). Объект — незакоммиченное рабочее дерево, 14 файлов под `crates/`
(`git diff HEAD --stat -- crates/` = +1301/−1931, совпадает с постановкой).
Пути без префикса — от `crates/dpos/consensus/src/`; `node/` = `crates/node/src/`.
Агентов не запускал. Git — только на чтение. Единственный записанный файл — этот.
Мутации продакшн-кода — три, все откачены, `md5sum -c gates/b2f-tree.md5` после отката без
расхождений (проверено мной дважды).

---

## §0. Прямые ответы

### (1) Ворота

Прочитал `gates/b2f-*.txt`, набор НЕ перегонял. `md5sum -c gates/b2f-tree.md5` — все
файлы `ЦЕЛ`, значит выводы относятся к тому же дереву, что я читал. `[KNOWN]`

`b2f-status.txt` verbatim: `lib exit 0 / stand exit 0 / node exit 0 / reader exit 0 /
slasher exit 0 / fmt exit 0 / stand-nofeat exit 0 / DONE`.

`test result` verbatim:

* `b2f-lib.txt:691` — `test result: ok. 682 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 169.38s`
* `b2f-stand.txt:55` — `test result: ok. 44 passed; 0 failed; 0 ignored; 0 measured; 646 filtered out; finished in 209.56s`
* `b2f-node.txt:67` — `test result: ok. 57 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 59.30s`
* `b2f-reader.txt:69` — `test result: ok. 63 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s`;
  `:76` — `test result: ok. 0 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s`
* `b2f-slasher.txt:23` — `test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s`
* `b2f-stand-nofeat.txt:44` — `test result: ok. 36 passed; 0 failed; 0 ignored; 0 measured; 646 filtered out; finished in 165.26s`

Число журнала «lib 682/0» сходится с `b2f-lib.txt` точно. `[KNOWN]`

ЧЕГО В НАБОРЕ НЕТ, а в каталоге есть: `b2f-clippy.txt`, `b2f-clippy-feat.txt`, `b2f-doc.txt`
(время 09:22–09:23, после последней строки `run-b2f.sh`). Их коды выхода НЕ записаны в
`b2f-status.txt`. `b2f-clippy.txt` содержит одно `warning: large size difference between
variants` на `node/dpos.rs:1990 ValidatorUpstream` — предупреждение ПРЕДСУЩЕСТВУЮЩЕЕ
(`git show HEAD:crates/node/src/dpos.rs` строки 1990-1993 те же). `b2f-clippy-feat.txt` чист.
`b2f-doc.txt` — 53 предупреждения против 55 в прошлом прогоне (`a2v-doc.txt`), то есть
рустдок-шум не вырос. Находка B2-14. `[KNOWN]`

### (2) Якорь холодного старта валидатора — центральный вопрос

**По коду дерева, случай за случаем.** `[KNOWN]`

| Случай | `resolve_cold_start_kind` | `(anchor_height, anchor_hash)` | file:line |
|---|---|---|---|
| graceful restart, архив непустой | `Restart` (`dpos.rs:1307-1308`) | `(archive_finalized, provider.block_hash(archive_finalized))` | `dpos.rs:2761-2762` |
| пустой архив, EL за эпохой 0, тег `finalized` ЕСТЬ | `ElFinalized` (`dpos.rs:1330`) | `(cs_finalized, cs_finalized_hash)` из `derive_cold_start_heights` | `dpos.rs:1755` |
| пустой архив, EL за эпохой 0, тега `finalized` НЕТ | `FreshMigration` | `(dpos_activation_block, wait_for_activation_block)` | `dpos.rs:1744-1753` |
| crash-survivor #12 | `Restart` → `DeferToElSync` | `(provider.best_block_number(), block_hash(best))` | `dpos.rs:1826-1841` |

**Кто и когда писал тег `finalized` в reth.** `.claude/RETH_INTERNALS.md:290` (прочитан ДО
checkout'ов, как требует постановка): «Written ONLY by the engine on FCU
(`update_finalized_block`/`update_safe_block`, tree/mod.rs:3091-3154) … Persisted across
restart: `BlockchainProvider::with_latest` reloads both at construction
(blockchain_provider.rs:87-116)»; `:316` — «The pipeline never touches finalized/safe/canonical-head
trackers». FCU-писатели НА HEAD и в дереве: executor'ский finalize-FCU
(`executor.rs:3846` и соседние), стартовый FCU follower'а (`dpos.rs:2977-2983`) и
`RethElSync::drive_fcu` (`cold_start_jump.rs:465-472`). После Б2 у `drive_fcu` два входа —
`sync_to` (цель = заверенная пара) и `sync_to_checkpoint` (операторский хэш). То есть тег
пишет ЛИБО собственный executor, ЛИБО прыжок с проверенной целью, ЛИБО оператор. **Комментарий
`dpos.rs:1745-1754` это утверждает верно.** `[KNOWN]`

**Является ли якорь «проверенным» в смысле §5.2.** ДА, и ключ к этому — НЕ новый код:
`RethAnchor::height()` = `max(cursor, provider.finalized_block_number())`
(`committee/store.rs:709-722`). Курсор (`FinalizedCursor`) засевается величиной
`last_consensus_finalized_height` (`executor.rs:1220`), которая при ПУСТОМ архиве равна нулю;
пол по тегу reth — это то, что не даёт модулю комитета уехать на генезис и читать
`committee[epoch(0)+2]` под codeless ChainConfig. Значит на арме `ElFinalized` модуль читает
`committee[epoch(cs_finalized)+2]` ровно на `block_hash(cs_finalized)`. Курино-яичной петли нет.
`[KNOWN]`

**Тег `finalized` после devp2p-синка без единого FCU.** `None` ⇒ `derive_cold_start_heights`
отдаёт `(0, genesis_hash)` (`dpos.rs:165-167`) ⇒ `cs_finalized = 0` ⇒ условие
`cs_finalized >= activation + interval` (`dpos.rs:1309`) ЛОЖНО ⇒ `FreshMigration`, НЕ
`ElFinalized`. Арм «пустой архив + EL за эпохой 0» в этом сценарии **недостижим**. Дальше
`FreshMigration` якорится на `dpos_activation_block` (реальный блок, reth его держит), так что
**BLOCKER'а «якорится на генезис ⇒ codeless ChainConfig ⇒ крах» НЕТ**: путь до
`latest_finalized_hash = genesis_hash` в дереве отсутствует. `[KNOWN]`

Но узел и не поедет: сразу за этим стоит инвариант чистой остановки
`ensure!(head_hash == latest_finalized_hash, "fresh migration but reth head …")`
(`dpos.rs:1885-1893`), и при EL на миллионе он падает с сообщением про
«the sequencer was not production-gated at dposActivationBlock». Это поведение **побайтно то же
на HEAD** (`git show HEAD:…/dpos.rs`, строки 2093-2101 — сверил). Не регрессия захода, но новый
арм `ElFinalized` был введён именно ради «архив потерян / EL со снапшота / devp2p-синк», а
третью треть этого списка он не покрывает и диагностика остаётся вводящей в заблуждение —
**находка B2-04, MODERATE**. `[KNOWN]`

**Сравнение с HEAD и регрессия.** На HEAD этот же вход был `Restart` с якорем
`archive_finalized` (= генезис при пустом архиве) и жёстким требованием посадить прыжок —
`empty_archive_requires_landed_jump` + бесконечный retry под
`dpos_sync_degraded{reason=awaiting_upstream}` (`HEAD:dpos.rs:1874-1880`, `:2048-2070`).
Узел НЕ поднимался, пока кто-то не отдал фронтир. В дереве узел стартует на своём EL-теге и
лезет лестницей.

Для узла, чей EL-тег стоит в эпохе с ПОЛНОСТЬЮ ротированными комитетами, **путь по коду дерева
остаётся один и он ручной**. Механика: целевой fetch ступени и обычные by-height пуллы marshal'а
уходят только пирам из `participants`, а `participants` перезаписывается только из
`update.latest.primary` = `C[T−1] ∪ C[T] ∪ C[T+1]` (`.claude/COMMONWARE_INTERNALS.md:389`,
`:393`; §5.2 по `resolver/src/p2p/engine.rs:202-205`), то есть из СОБСТВЕННЫХ исторических
комитетов узла. При нулевом пересечении спрашивать некого — ни лестницей, ни контигуозным
догоном. У валидатора альтернативы нет вовсе: поля `l1_checkpoint` в `DposLayerConfig`
(`dpos.rs:924-1010`) не существует, оно есть только у `FollowerLayerConfig`
(`dpos.rs:2668-2681`). Остаётся оператор: восстановить datadir с НЕПУСТЫМ консенсус-архивом.
Журнал §0(9) говорит ровно это — **подтверждаю, и расширяю**: то же верно и для
follower'а/валидатора с ПОЛНОСТЬЮ ЗАПОЛНЕННЫМ, но устаревшим на ≥2 эпохи архивом
(`read_geometry` вернёт `Some`, арм `fresh_follower_entry` недостижим), чего журнал не пишет.
Находка B2-01. Является ли это регрессией — формально да по сравнению с HEAD только в одном:
на HEAD такой узел стоял припаркованным и НЕ поднимал движок, в дереве он поднимает движок в
давно мёртвой эпохе. Ни один из двух исходов не «работает». `[KNOWN по коду]`

### (3) #12 (crash-survivor `DeferToElSync`) — звено за звеном

Проверил каждое утверждение Д-99 по коду. `[KNOWN]`

* **Посев.** `executor.rs:1265` `last_tip_height: cfg.last_consensus_finalized_height`;
  `executor.rs:1289` `ordering_finalized: cfg.last_consensus_finalized_height.get()`. Разность
  0 на старте — ВЕРНО. Обе строки НЕ трогались заходом (`git diff` по файлу их не содержит),
  то есть это свойство HEAD, а не новое.
* **Гейт.** `executor.rs:2512-2519`: `height − ordering_finalized <= threshold ||
  pending_backfill.is_some() || !finalized_heights_to_backfill.is_empty()` ⇒ `return`. Спавн при
  непустом drain запрещён — ВЕРНО.
* **Drain доходит до архивного курсора при ГЛУБОКОМ разрыве.** `executor.rs:1176-1177`:
  `(cfg.last_execution_finalized_height + 1)..=cfg.last_consensus_finalized_height.get()`.
  **Cap'а нет ВООБЩЕ** — `MAX_COLD_RECOVER` ограничивает только пред-движковый replay
  (`dpos.rs`, `recover_finalized_tail_into_reth`), а не drain. То есть да, доходит; на HEAD
  диапазон был короче потому, что `last_execution_finalized_height` читался ПОСЛЕ прыжка
  (`HEAD:dpos.rs:2073-2079` с комментарием «Read AFTER the crash-survivor recovery + jump»), а
  в дереве прыжка перед этим чтением больше нет (`dpos.rs:1861-1866`).
* **`ordering_finalized`, засеянный курсором marshal'а при reth ниже него, не ломает derive** —
  ВЕРНО и это поведение HEAD, не новое (строка не менялась). Первый блок drain'а берётся из
  `(last_execution+1)`, а не от `ordering_finalized`.
* **Цена по времени против HEAD (называю, не оцениваю).** На HEAD глубина > `JUMP_THRESHOLD`
  закрывалась ОДНИМ devp2p-бэкфиллом (`sync_to` ⇒ staged pipeline, `RETH_INTERNALS.md:312`).
  В дереве тот же разрыв проходится ПОБЛОЧНО через `on_finalized_block` (derive + import +
  FCU) из архива marshal'а. Для разрыва в 65…1024 блоков разницы нет (прыжок был `Lagging`);
  разница начинается выше 1024.
* **Состояние взаимного ожидания.** Есть, и оно хуже, чем «ожидание»: арм `None`
  (`executor.rs:1438-1457`) — это `Fault::corruption`, то есть жёсткая остановка, а прыжок в
  этот момент запрещён гейтом. Достижимо, когда диапазон drain'а пересекает дыру в архиве
  (диапазон, который узел когда-то ПЕРЕПРЫГНУЛ: §5.2 «не хранится только перепрыгнутое»).
  На HEAD devp2p-фаст-форвард содержимое архива не спрашивал. Находка B2-02, MODERATE.
* **Снятие гейджа.** `executor.rs:1417-1436` — только в ветке УСПЕХА `Some(block)` и только
  когда drain опустел. Гейдж поднимается в `crash_recover_defer_or_fatal`
  (`dpos.rs:372-374`) и на HEAD снимался безусловно в `launch` (`HEAD:dpos.rs:2004-2008`).
  Два пути, на которых он теперь залипает навсегда: (а) пустой диапазон drain'а
  (`last_block_number() >= cursor`), (б) `dispatch_fault` на ПОСЛЕДНЕЙ высоте drain'а
  (тогда `else if` не выполняется). Находка B2-03, MODERATE.

### (4) Follower: три арма

**(а) datadir-арм без `sync_to`** (`dpos.rs:2861-2884`). Якорь `rf_hash`, при `rf_num <
activation` — `wait_for_activation_block`. Регрессия по времени против HEAD (называю):
HEAD делал `up.get_latest() ⇒ el.sync_to(&latest)` (`HEAD:dpos.rs:3074-3080`), то есть ОДИН
devp2p-фаст-форвард до живого tip'а перед стартом. В дереве follower садится на тег и лезет.
Сколько это стоит — зависит от источника: у WS-follower'а `get_finalization` идёт мимо
`participants` CW (`node/cert_follow/upstream.rs:100`), так что лестница по две эпохи за
посадку работает; у плоскостного — упирается в `participants` (см. (2)). Пока `committee(live)`
вне окна, живой поток inlet'а парковАн (`cert_inlet.rs:388-392` — «during a deep backfill the
module answers NotReadable for every cert»), то есть tip не двигается и steady-state прыжок не
триггерится: лестница здесь не «ускорение», а ЕДИНСТВЕННЫЙ путь.

**(б) fresh-арм** (`dpos.rs:2885-2950`). Форма checkpoint — ТОЛЬКО `hash` (`B256`), без высоты;
высота узнаётся из посадки `block_number(hash)` (`cold_start_jump.rs:626-646`). EL синкается к
хэшу без известной высоты обычным FCU `head=safe=finalized=hash` (`drive_fcu`,
`cold_start_jump.rs:465-472`); по `RETH_INTERNALS.md:223`/`:310` неизвестный disconnected
блок при gap > 32 и отсутствующем локально finalized запускает `BackfillAction::Start(finalized
hash)` — то есть pipeline ищет именно по ХЭШУ, высота не нужна. Механика верна. `[KNOWN]`
`assert_l1_checkpoint` со строками verbatim остался — функция в диффе НЕ появляется
(`git diff HEAD -- …/cold_start_jump.rs | grep assert_l1_checkpoint` пуст), вызов на месте
(`dpos.rs:2956-2958`). Обращаю внимание: на fresh-арме с checkpoint этот ассерт стал
тавтологией (узел только что синкнулся ИМЕННО на этот хэш) — на restart-арме он остаётся
содержательным.
Предикат отказа — ТОТ ЖЕ, что у `load_bls_keypair`: `is_deployed_network`
(`node/dpos.rs:2365`), вызовы `node/dpos.rs:2412` (BLS) и `node/cert_follow/mod.rs:238`
(cert-follow). Одна точка сборки `FollowerLayerConfig` (`node/cert_follow/mod.rs:221`), так что
«забыть» поле негде. Текст отказа (`dpos.rs:1359-1368`) называет флаг
`--dpos.l1-checkpoint`, chain_id и обе альтернативы. `[KNOWN]`
devnet-арм `get_latest ⇒ sync_to` (`dpos.rs:2904-2927`) — **единственный оставшийся
непроверенный `sync_to` в крейте**: `grep -n 'sync_to('` по продакшн-коду даёт ровно два
вызова — `cold_start_jump.rs:802` (внутри `jump_to_target`, цель из собственного архива) и
`dpos.rs:2927`; `sync_to_checkpoint` — один, `dpos.rs:2904`. Оба fresh-вызова стоят ЗА
`fresh_follower_entry(...)?`, то есть за предикатом. `grep get_latest` по продакшн-коду обоих
крейтов: два места — `dpos.rs:1177` (проба) и `dpos.rs:2913` (devnet-арм). `[KNOWN]`

**(в) второй прыжок follower'а удалён.** Глубокий разрыв закрывается лестницей Б1: проба
`frontier_probe` (`dpos.rs:1100-1183`) + `local_tracked_epoch` (`dpos.rs:1208-1218`,
`geometry.last(e) == fin ? e+1 : e`) → `executor::probe_frontier`
(`executor.rs:2228-2252`) → `marshal.hint_finalization(height, targets)` → marshal-резолвер →
`UpstreamResolver::spawn_finalized` (`cert_inlet.rs:2599-2660`) → `get_finalization` → marshal
`verify_delivered` → `Update::Tip` → `maybe_re_jump`. Проводка на месте.

### (5) A2-04 (отброс вне окна)

`deliver` при `OutOfWindow`/`NotReadable`/`Read(_)`/`no_geometry`: счётчик
`dpos_frontier_dropped_total{reason}` + `drop(waiters.remove(&key))` + `return true`
(`plane_upstream.rs:479-484`). Один общий арм на ОБА ключа — `Latest` и `Finalized{h}` — потому
что `key` в него не смотрит. Метрика переименована из `dpos_frontier_unauthenticated_total`
(`plane_upstream.rs:114-120`). `[KNOWN]`

Юниты инвертированы, все три: `an_out_of_window_latest_is_dropped_without_punishing_the_peer`
(`:1146`), `an_out_of_window_by_height_answer_is_dropped_without_punishing_the_peer` (`:1186`),
`an_unreadable_committee_is_dropped_without_punishing_the_peer` (`:1228`) — `rx.await.is_err()`
вместо `is_ok()`. **Мутация (сделана мной, откачена):** удалил `drop(...)+return true` из арма —
все три покраснели с текстом «the answer reached the caller — an unauthenticatable certificate
was admitted». Юниты чувствительны. `[KNOWN]`

`grep get_latest` — см. (4): проба и devnet-арм, больше ничего.

**Кто повторит by-height запрос.** CW: `Consumer::deliver(key, value) -> bool`, «true completes
the fetch» (`.claude/COMMONWARE_INTERNALS.md:389`), то есть `true` ЗАКРЫВАЕТ fetch — ответ
считается доставленным, ретрая от резолвера не будет. Повторный драйвер — проба
(`executor.rs:2228-2252`), которая называет ступень КАЖДЫЙ замёрзший тик, независимо от того,
сохранилось ли что-нибудь. Механизм есть, Д-72 действительно закрыт. `[KNOWN]`

**Достижимо ли, что marshal просит высоту вне окна.** `fin` — СВОЙ (`ordering_finalized`,
он же якорь модуля через `RethAnchor`, `committee/store.rs:709-722`). Ступень — `last(T+1)`,
`T ∈ {epoch(fin), epoch(fin)+1}`, значит эпоха ступени ≤ `epoch(fin)+2` = верх окна модуля, а
`commit_height(T+1) = start(T−1) ≤ fin` — читаемо. Ремонтные пуллы marshal'а идут из
`(last_processed_height, …]`, то есть не ниже `epoch(fin)`. **По построению ступень в окне**, и
пути к SERIOUS «отброшенный by-height без драйвера» я не нашёл. Остаточная дыра одна и она не
на лестнице: `dpos::refetch_verified_archive_hole` (`dpos.rs:414-462`) тянет ОДНУ высоту ниже
пола; если её эпоха уехала ниже `epoch(anchor)−8`, `deliver` теперь отбросит ответ и refetch
вернёт ошибку, где на HEAD он получал ответ и проверял его сам на `at_hash`. Достижимость
низкая (#8-дыра ≤ 64 блока от пола), поэтому MINOR, не SERIOUS — находка B2-17.

### (6) Стадии и варианты

* `jump_to_target` (`cold_start_jump.rs:759-768`) больше не берёт ни `CommitteeSource`, ни RNG —
  это КОМПИЛЯТОРНЫЙ факт, стадий нет. `[KNOWN]`
* **Прошла ли пара из `pair_at` структурную проверку и BLS.** `pair_at(tip)` берётся из
  архива, куда пишет только `store_finalization`. По `.claude/COMMONWARE_INTERNALS.md:181` путь
  `Resolver Deliver(Finalized{h})` даёт «BLS batch-verified: `verify_certificates` … `block.height()==h`,
  commitment==payload, `finalization.epoch()==bounds.epoch()` (:987-994). Failure → deliver `false`».
  ОДНАКО там же `:184`: «consensus-facing paths (`report`, `verified`, `proposed`) are 100%
  trust-the-caller». То есть структурно-BLS-гарантия резолверная, а не марашловая: вторая дверь в
  `store_finalization` — `Reporter::report`, и её закрывают уже НАШИ инварианты (inlet
  BLS-проверяет до `report`, `cert_inlet.rs`; засев границы идёт через `fetch_verified_boundary`,
  `outer.rs:820-834`; свой движок — simplex). Утверждение верно, но держится на дисциплине
  fluentbase, а не на инварианте CW; в доках `cold_start_jump.rs:14-20` это подано как свойство
  marshal'а. Находка B2-18, MINOR.
* `BadTarget`/`AuthFailed` — **0 идентификаторов** в коде: `grep -rn 'BadTarget\|AuthFailed'
  crates bins` даёт 16 попаданий, ВСЕ в комментариях/докстрингах. `cold_start_jump(` как функции
  нет вовсе. `[KNOWN]`
* L1 `Err ⇒ Stalled` — `cold_start_jump.rs:870` `Err(e) => return JumpOutcome::Stalled(e)`;
  юнит `an_l1_probe_error_is_stalled_not_a_fork_verdict` (`:1638-1688`) проверяет текст
  `"L1 checkpoint probe after jump failed"`. `[KNOWN]`
* Арм «нечитаемый комитет + L1 ⇒ Ok» — **снят полностью**: параметра `l1_checkpoint` у
  `verify_jump_authenticated` больше нет (`cold_start_jump.rs:714-737`), отказ безусловный.
  Оба оставшихся потребителя на HEAD передавали `None` (`HEAD:dpos.rs:1934`
  `recover_finalized_tail_into_reth(..., None, ...)` — единственный вызов;
  `HEAD:cert_follow.rs:219` — `None`), так что поведение потребителей не изменилось. `[KNOWN]`
* `refetch_verified_archive_hole` / `fetch_verified_boundary` не ослаблены: обе по-прежнему
  зовут `verify_jump_structural` + `verify_jump_authenticated` (`dpos.rs:450`, `:455`;
  `cert_follow.rs:216`, `:220`), проверка стала СТРОЖЕ (нет L1-отката).
* `JUMP_THRESHOLD` — продакшн-читатели: `dpos.rs:2343` и `:3081`
  (`JUMP_THRESHOLD.min(interval)` на обоих путях) и `node/cert_follow/mod.rs:167` (не про прыжок —
  это размер окна фида). Больше нигде.
* **Доки о `verify_jump_structural` фактически неверны.** `cold_start_jump.rs:26-28`:
  «survive as FUNCTIONS for the two by-height seams … **and for nothing else**»;
  `:676-683` — «The two callers left are …». По `grep` вызовов ЧЕТЫРЕ, из них продакшн-три:
  `dpos.rs:450`, `cert_follow.rs:216` и **`plane_upstream.rs:419` — шаг (2) самого `deliver`**
  (+ `testbed/byzantine_roles.rs:638`). Находка B2-16.

### (7) Стенд

**(а) Роль «пир без данных».** Роль НЕ писалась, и стенд действительно различает «нет данных»
от лжи ПРОДАКШН-кодом: `FrontierHandler::produce` (`plane_upstream.rs:518-529`) при промахе
роняет `response` не отправив, а `.claude/COMMONWARE_INTERNALS.md:389` подтверждает —
«dropping the sender = "don't have it"» ⇒ `Payload::Error` ⇒ `add_retry` без бана, `deliver`
не вызывается. Пин `deliveries_rejected == 0` (`tests.rs:1457-1462`) **НЕ вакуумен** —
проверено мутацией: я заменил роняние sender'а на `response.send(Bytes::new())` (то есть
«нет данных» стало «мусор на проводе»), и тест покраснел, распечатав
`deliveries_rejected: 3`, а лестница развалилась (`node 3 named ONE rung for the whole run`).
Мутация откачена, `md5sum` сверен. `[KNOWN]`
Чего пин не различает: «тот же пир / другой пир» — счётчики `UpstreamCounters`
(`fakes.rs:1530-1543`) по узлу, не по паре. Журнал §0(6) это признаёт.

**(б) Пин отброса вне окна.** **На стенде его НЕТ ни в каком виде.** `CountingHandler::deliver`
(`fakes.rs:1769-1778`) инкрементирует `deliveries_decoded` на КАЖДЫЙ `true`, то есть
отброшенный ответ считается там же, где принятый; счётчик `dpos_frontier_dropped_total` —
`metrics::counter!`, а `Outcome::metric` читает реестр commonware. Так что «какой узел / какой
счётчик» — ответа нет, инверсия держится ТОЛЬКО тремя юнитами `plane_upstream::tests`.
Журнал §0(6)/§4(7) это называет; фиксирую как B2-06. Ассерты лестницы и предусловий не
ослаблены: `testbed/preconditions.rs` в диффе отсутствует (`git status` его не показывает),
ладдер-тест только ПРИОБРЁЛ два ассерта (`unserved > 0`, `deliveries_rejected == 0`).

**(в) Переписанные ассерты «что доказывал / что доказывает».** Проверил все четыре блока
(`tests.rs:1296-1330`, `:2379-2419`, `:3327-3345`, `:2487-2496`) — формулировки соответствуют
коду: секция (4) в `the_rejump_runs_the_production_jump_and_lands_on_its_own_archive_pair`
честно пишет, что наблюдения больше нет и что отсутствие стадии — компиляторный факт; в
`a_lying_upstream_is_refused_at_the_frontier_and_never_lands_its_branch` ветка `Some("AuthFailed")`
убрана вместе с вариантом. Одно замечание: в «Falsifier» того же теста строка «a landing on a
branch the honest three did not execute (then the jump is reading the wrong state)» дублирует
предыдущий фальсификатор и не соответствует ни одному ассерту в теле — B2-19, NIT.

### (8) Юниты

**(а) Якорь пустого архива, красный до правки.** Журнал говорит «на HEAD эта строка не
компилировалась бы» — это ВЕРНО (варианта `ElFinalized` на HEAD нет), но поэтому же это не
демонстрация чувствительности. Воспроизвёл мутацией ≤ 2 (одна): заменил
`return Ok(ColdStartKind::ElFinalized);` на `Restart` — тест
`empty_archive_with_the_el_past_epoch_zero_anchors_at_the_el_finalized_tag` упал
(`left: Restart / right: ElFinalized`, `dpos.rs:3741`), остальные 7 в модуле зелёные. Откачено,
`md5sum` совпал с исходным. `[KNOWN]` Оговорка — B2-07: тест пинует ТОЛЬКО kind; сам якорь
(`cs_finalized_hash` в `launch`) не проверен ничем, кроме компилятора, хотя имя теста говорит
«anchors at the EL finalized tag».

**(б) `jump_to_target` со сломанным multisig ⇒ `Landed`.** Не вводит в заблуждение:
`cold_start_jump.rs:1555-1575` явно называет защиту («the property that makes this safe is that
such a pair never REACHES a jump») и даёт POSITIVE CONTROL по имени —
`plane_upstream::tests::a_multisig_that_fails_under_a_readable_committee_is_a_lie`, который
существует (`plane_upstream.rs:1249`). Документировано корректно.

**(в) Отказ fresh-datadir.** Юнит есть:
`a_fresh_datadir_without_a_checkpoint_refuses_on_a_deployed_network` (`dpos.rs:3690-3713`),
проверяет и упоминание флага, и упоминание chain_id, и обе ветки `deployed`, и то, что
checkpoint выигрывает на обеих.

### (9) Граница и гигиена

`git diff HEAD -- crates/` по добавленным строкам: **ни одного** нового `#[allow]`, `todo!`,
`unimplemented!`, таймера или поллинга. Наоборот, СНЯТЫ два `#[allow(clippy::too_many_arguments)]`.
`pub`-поверхность изменилась в одном месте — `lib.rs:78` перестал реэкспортировать
`cold_start_jump` (функции больше нет); `ElSync` приобрёл обязательный метод
`sync_to_checkpoint` — это ломающее изменение публичного трейта, но крейт внутренний, все три
реализации в дереве. Единственный новый продакшн-`unwrap` — `self.waiters.lock().unwrap()`
(`plane_upstream.rs:481`), в стиле файла (mutex-poisoning). Правок вне списка постановки нет:
14 файлов `git status` ровно те, что перечислены. `node/cert_follow/mod.rs` — только проводка
(+6 строк, одно поле). `node/dpos.rs` — проводка плюс ОДНА новая функция `is_deployed_network`,
вынесенная из тела `load_bls_keypair` без изменения семантики (сверил тела). `[KNOWN]`

### (10) Где журнал вводит в заблуждение; §0(7) журнала; hard-stop'ы

**Журнал точен и необычно самокритичен.** Мест, где он вводит в заблуждение, два, оба мелкие:

1. §0(6): «Пин отброса вне окна (Б2.6 п.2) — держится зелёным: `preconditions::…` и
   `committee_tests::…`». Эти два теста пинуют НЕ отброс (первый — лестницу прыжков к своей паре,
   второй — отказ модуля без EVM); формулировка создаёт впечатление, что отброс на стенде
   наблюдаем. Ниже в том же абзаце это опровергается («сам СЧЁТЧИК … не наблюдаем»), но первая
   фраза остаётся неверной. B2-20, NIT.
2. §0(9): тупик описан только для follower'а «с существующим datadir'ом и **пустым архивом**».
   По коду он ровно тот же при НЕПУСТОМ, но устаревшем на ≥2 эпохи архиве — арм
   `fresh_follower_entry` недостижим в обоих случаях (условие — `read_geometry == None`), а
   плоскостной `participants` не зависит от заполненности архива. Область риска шире
   заявленной. B2-01.

**§0(7) журнала — пункт за пунктом.** (1) ПОДТВЕРЖДАЮ: `#12` не закрывается прыжком, посев и
гейт проверены выше. (2) ПОДТВЕРЖДАЮ: продакшн-`sync_to` два (`cold_start_jump.rs:802`,
`dpos.rs:2927`), ни один не «идёт через `Frontier`» в смысле §5.2, буква §5.2 описывает не тот
код. (3) ПОДТВЕРЖДАЮ: `cold_start_jump(` — ноль вхождений. (4) ПОДТВЕРЖДАЮ: §5.5 держит в списке
«остаётся» уже несуществующую функцию. (5) ПОДТВЕРЖДАЮ, с уточнением: строка §5.4 «`Latest` вне
окна … метрика» стала верной, но имя метрики в §5.4 не задано, а в коде оно СМЕНИЛОСЬ
(`dpos_frontier_unauthenticated_total` → `dpos_frontier_dropped_total`) — это ломает дашборды и
в журнале §1/§3 как отдельное решение не выделено. B2-21, MINOR.

**§0(9) журнала — с формулировкой согласен, с «путём, который остаётся» согласен** (проверил
отсутствие `l1_checkpoint` в `DposLayerConfig` и условие достижимости fresh-арма), с ОБЛАСТЬЮ
не согласен — см. выше.

**Hard-stop'ы оркестратора:**

* **(2) Требуется ли менять Д-2/П-4.** Д-2 менять не требуется — реализовано именно (а)
  «один шаг по проверенной финализации, повторяемый» и (б) «checkpoint только там, где проверить
  нечем». П-4 не затронут. ТРЕБУЕТ правки §5.2/§5.4/§5.5 (пять расхождений журнала §0(7), все
  подтверждены) — но это доки проекта, не решения. `[KNOWN]`
* **(3) Есть ли BLOCKER, на который проект не отвечает.** НЕТ. Самая тяжёлая находка (B2-01) —
  это Д-2(а) в явном виде, проект её предвидит (§5.4 строка «Ступень никем не обслужена») и
  журнал называет цену; §5.4 лишь переоценивает достижимость («недостижимо, пока хоть один
  член `committee(T+1)` жив»), потому что CW-адресация жёстче, чем «жив». Это ошибка
  ОЦЕНКИ РИСКА в проекте, а не неверность проекта на центральном пути.
* **(4) Опроверг ли стенд проект на центральном пути.** **НЕТ.** Свидетельство: полный набор
  зелёный на этом дереве (`b2f-status.txt`, все 7 кодов 0), включая оба пина предусловий
  (`stand exit 0` / `stand-nofeat exit 0` покрывают `testbed::preconditions::*`, файл
  предусловий не менялся), и обе мои мутации показали, что ключевые пины имеют зубы. Центральный
  путь (лестница → tip → прыжок на свою же пару) на стенде отрабатывает. `[KNOWN]`

---

## §1. Находки

| id | серьёзность | file:lines | HEAD-якорь | что не так | чем пытался опровергнуть | уверенность |
|---|---|---|---|---|---|---|
| B2-01 | SERIOUS | `dpos.rs:1309-1331`, `:2861-2884`, `:2668-2681`; `cert_inlet.rs:388-392` | `HEAD:dpos.rs:1874-1880`, `:2048-2070`, `:3074-3080` | Узел (валидатор ИЛИ follower) с локальным `ChainConfig` и архивом, устаревшим ≥2 эпохи, теперь садится на собственный EL-тег и обязан лезть лестницей; целевой fetch и by-height пуллы уходят только в `participants = C[T−1..T+1]` (его СОБСТВЕННЫЕ исторические комитеты), а живой поток inlet'а парковАн `NotReadable`, пока окно не догонит. При полной ротации спрашивать некого. `l1_checkpoint` у валидатора нет вовсе (поле только в `FollowerLayerConfig`), а `fresh_follower_entry` достижим только при `read_geometry == None` — то есть у follower'а с ЛЮБЫМ (пустым или полным) устаревшим datadir'ом checkpoint недоступен. Журнал §0(9) называет это только для ПУСТОГО архива | Искал второй источник `participants` (нет: `update.latest.primary`, COMMONWARE_INTERNALS:389/§5.2); искал вход checkpoint'а для валидатора (`grep l1_checkpoint` по `DposLayerConfig` — пусто); проверял, спасает ли WS-upstream (спасает, но только у WS-follower'а, не у плоскостного дефолта); проверял, можно ли считать это не-регрессией (на HEAD узел парковался, а не лез — оба исхода нерабочие, но HEAD не поднимал движок в мёртвой эпохе) | высокая по механизму, средняя по частоте (зависит от скорости ротации) |
| B2-02 | MODERATE | `executor.rs:1176-1177`, `:1438-1457`, `:2512-2519` | `HEAD:dpos.rs:1955-1975`, `:2073-2079` | #12 теперь закрывается drain'ом без какого-либо cap'а, поблочно из архива marshal'а. Если диапазон `(reth tip .. cursor]` пересекает дыру в архиве (перепрыгнутый когда-то диапазон — §5.2 «не хранится только перепрыгнутое»), drain получает `None` и это `Fault::corruption` (жёсткая остановка), а прыжок в этот момент запрещён гейтом. На HEAD devp2p-фаст-форвард содержимое архива не спрашивал | Искал cap на drain (`MAX_COLD_RECOVER` ограничивает только пред-движковый replay, не `finalized_heights_to_backfill`); искал самолечение арма `None` (нет — комментарий bug 10 прямо говорит «cannot self-heal»); проверял достижимость: нужен откат/восстановление reth-датадира ниже прежней посадки прыжка при целом архиве — редко, но это ровно класс «EL со снапшота», ради которого #12 и существует | средняя |
| B2-03 | MODERATE | `executor.rs:1417-1436` | `HEAD:dpos.rs:2004-2008` | `dpos_sync_degraded{reason=crash_recover}` снимается только внутри ветки УСПЕХА drain'а и только когда очередь опустела. Два пути залипания навсегда: пустой диапазон drain'а (`last_block_number() >= cursor`) и `dispatch_fault` на последней высоте. На HEAD гейдж снимался безусловно в `launch` | Искал второй сайт `recover(SyncReason::CrashRecover)` — `grep` даёт ровно один; проверял, гарантированно ли диапазон непуст (нет: `last_block_number()` и `best_block_number()` — разные величины, анкер берёт `best`, drain — `last`) | средняя |
| B2-04 | MODERATE | `dpos.rs:165-167`, `:1309`, `:1885-1893` | `HEAD:dpos.rs:165-167`, `:2093-2101` (идентичны) | Новый арм `ElFinalized` недостижим ровно в одном из трёх сценариев, ради которых он введён: после devp2p-синка без единого FCU тег `finalized` отсутствует ⇒ `cs_finalized = 0` ⇒ `FreshMigration` ⇒ фатал инварианта чистой остановки с сообщением про «the sequencer was not production-gated», которое к причине отношения не имеет. Поведение идентично HEAD, поэтому не регрессия — но заход, который вводил арм под этот класс, дыру не закрыл и нигде её не назвал | Пытался найти путь, на котором `ElFinalized` получает `genesis_hash`: невозможен (`0 >= activation + interval` ложно при `interval > 0`), то есть BLOCKER'а «codeless ChainConfig» действительно НЕТ; проверял, не изменил ли заход инвариант чистой остановки — побайтно тот же | высокая |
| B2-16 | MINOR | `cold_start_jump.rs:26-28`, `:676-683`, `:706-712`; факт — `plane_upstream.rs:419` | `HEAD:cold_start_jump.rs:664-672` | Доки трижды утверждают, что `verify_jump_structural`/`verify_jump_authenticated` остались «для двух by-height швов **и ни для чего больше**». `verify_jump_structural` при этом — шаг (2) самого `FrontierHandler::deliver`, то есть у него ТРИ продакшн-вызывающих. Дрейф указывает на то, что при снятии стадий про третий вызов не вспомнили | `grep -rn 'verify_jump_structural('` — четыре вызова (три продакшн + роль стенда); проверял, не обёрнут ли вызов в `deliver` во что-то, что доками покрыто — нет | высокая |
| B2-17 | MINOR | `plane_upstream.rs:479-484`; потребитель `dpos.rs:414-462` | `HEAD:plane_upstream.rs:471-490` | `refetch_verified_archive_hole` тянет одну высоту НИЖЕ пола через `CertUpstream::get_finalization`; если её эпоха вне окна модуля, `deliver` теперь ОТБРАСЫВАЕТ ответ и refetch падает ошибкой. На HEAD ответ доходил, и шов проверял его сам на `at_hash` (восстановленный родитель) — то есть на более уместном хэше, чем окно `deliver`. Шов лишился входа, который его собственная проверка умела обработать | Пытался показать недостижимость: #8-дыра ≤ `MAX_COLD_RECOVER` (64) от пола, значит эпоха дыры ≈ эпоха якоря и в окне; но окно считается от якоря МОДУЛЯ (`max(cursor, EL-тег)`), а на этом пути курсор ещё не засеян, так что совпадение не гарантировано конструктивно | низкая (не нашёл конкретной достижимой траектории) |
| B2-18 | MINOR | `cold_start_jump.rs:14-20`, `:762-768` | — | Доки подают «пара из своего архива ⇒ уже 2f+1 и структурно связана» как свойство marshal'а («the single writer is `store_finalization` after `verify_delivered`»). По `.claude/COMMONWARE_INTERNALS.md:184` `report`/`verified` — trust-the-caller, то есть гарантию держат НАШИ инварианты (BLS в inlet'е, `fetch_verified_boundary` на засеве границ, свой simplex), а не marshal. Утверждение верно, обоснование — нет; будущая правка, добавившая третьего репортёра, снимет защиту незаметно | Проверял, нет ли в fluentbase репортёра без проверки: `outer.rs:820-834` (через `fetch_verified_boundary`), `cert_inlet.rs` (BLS до `report`), свой движок. Сегодня все три закрыты — поэтому MINOR, а не SERIOUS | высокая по факту, низкая по достижимости вреда |
| B2-05 | MINOR | `plane_upstream.rs:481-482`, `:625-646`, `:68-73` | `HEAD:plane_upstream.rs:534-553` | Комментарий арма и модульные доки говорят, что отброс «cancels the in-flight fetch». Не отменяет: `deliver` удаляет ВЕСЬ вход `waiters`, поэтому ветка `None` в `fetch_one` видит `waiters.get_mut(&key) == None`, ставит `empty = false` и `mailbox.cancel(key)` НЕ зовёт. Вреда нет (`true` и так закрывает fetch по CW), но описанный механизм не исполняется | Читал `fetch_one` целиком; проверял по COMMONWARE_INTERNALS:389, не нужен ли cancel («true completes the fetch» — не нужен), поэтому MINOR | высокая |
| B2-06 | MINOR | `fakes.rs:1769-1778`, `:1534-1542`; `plane_upstream.rs:114-120` | `HEAD:fakes.rs` (те же строки) | Центральное новое поведение §5.2 (отброс) на стенде НЕ НАБЛЮДАЕМО ни одним счётчиком: `CountingHandler` кладёт отброшенный ответ в `deliveries_decoded` вместе с принятым, а `dpos_frontier_dropped_total` — фасад `metrics`, который стенд не читает. Инверсия держится только тремя юнитами | Искал в `Outcome` любое поле, разделяющее принято/отброшено — нет; журнал §4(7) это признаёт, фиксирую как находку, потому что это единственная дыра наблюдаемости на новом центральном пути | высокая |
| B2-07 | MINOR | `dpos.rs:3717-3743` | `HEAD:dpos.rs:4021-4029` | `empty_archive_with_the_el_past_epoch_zero_anchors_at_the_el_finalized_tag` проверяет ТОЛЬКО возвращённый `ColdStartKind`; имя теста и его докстринг говорят про ЯКОРЬ (`cs_finalized_hash`), а проводка в `launch:1739-1766` не покрыта ничем, кроме компилятора. Мутация `ElFinalized → Restart` красит тест (проверено), мутация якоря в `launch` — нет | Мутировал `resolve_cold_start_kind` (красное, ожидаемо); искал второй тест, который трогает `launch` — `launch`/`launch_follower` на стенде не вызываются (`testbed/mod.rs`), в юнитах тоже нет | высокая |
| B2-14 | MINOR | `gates/b2f-status.txt`, `gates/run-b2f.sh` | — | В наборе ворот нет ни `clippy`, ни `doc`: `run-b2f.sh` их не гоняет, `b2f-status.txt` их кодов выхода не содержит, хотя файлы `b2f-clippy*.txt`/`b2f-doc.txt` в каталоге есть (позже по времени). «Clippy чист» этим прогоном не установлено; фактически в `b2f-clippy.txt` одно предупреждение (предсуществующее `large_enum_variant`), а `b2f-doc.txt` — 53 предупреждения против 55 в `a2v-doc.txt`, то есть не выросло | Сверил тела `ValidatorUpstream` на HEAD и в дереве — идентичны, предупреждение не заходом внесено; сверил счётчики doc-предупреждений между прогонами | высокая |
| B2-08 | NIT | удалён `HEAD:executor.rs:11726-11775` | `HEAD:executor.rs:11726-11775` | Регрессионный тест `re_jump_bad_target_resets_cross_url_streak` (инвариант критика r2 «ЛЮБОЙ executor-side `rotate()` сбрасывает streak») удалён без замены. Риск мал — единственный оставшийся ротирующий арм (`Stalled`, `executor.rs:1546-1550`) сбрасывает streak в той же ветке, — но инварианта как утверждения в тестах больше нет | Прочитал арм `Stalled` целиком и убедился, что сброс рядом с `rotate_upstream()`; искал другой тест на сброс — нет | высокая |
| B2-09 | NIT | `testbed/mod.rs:53-58` | `HEAD:testbed/mod.rs:53-58` | Доки стенда всё ещё описывают снятую конструкцию: «`cold_start_jump::cold_start_jump_with_threshold` (so `verify_jump_structural` and `verify_jump_authenticated` run for real)» — ни функции, ни стадий нет; и интра-док-ссылка `[fakes::JumpCommittees]` теперь висячая (тип удалён этим заходом). `cargo doc` этого не ловит, потому что `mod testbed` — `#[cfg(test)]` (`lib.rs:62-63`) | Сверил с HEAD: `cold_start_jump_with_threshold` был стайл-дрейфом уже на HEAD, а `JumpCommittees` ссылка сломалась ИМЕННО этим заходом; проверил `b2f-doc.txt` — предупреждения нет, подтверждая, что рустдок сюда не смотрит | высокая |
| B2-10 | NIT | `executor.rs:4918` | `HEAD:executor.rs:4932` | Докстринг тестового `Fixture` всё ещё обещает «#14/#1 tests assert the `engine_retry` / `auth_rotate` gauges», а `SyncReason::AuthRotate` этим заходом удалён (`sync_metrics.rs`) | `grep AuthRotate` — идентификатора нет, только эта прозаическая ссылка | высокая |
| B2-11 | NIT | `cold_start_jump.rs:2045-2049` | `HEAD:cold_start_jump.rs:2159-2175` | Сообщение `expect_err("an unreadable committee must refuse, with or without an L1 checkpoint")` описывает контраст, который тест больше не может поставить: параметра `l1_checkpoint` у функции нет, «с checkpoint» непредставимо | Читал тело теста: второй половины контраста действительно нет, докстринг про это честно пишет, а строка `expect_err` — нет | высокая |
| B2-12 | NIT | `dpos.rs:2937-2948` | `HEAD:dpos.rs:3093-3105` | На fresh-арме с checkpoint НИЖЕ активации `h = h.max(activation)` уводит чтение на высоту, которой reth не держит; `read_with_visibility_belt` (`dpos.rs:1382-1416`) через 10 с вернёт «reth does not hold the clamped landing {h}», не назвав причиной неверный checkpoint. Для входа, который оператор задаёт руками, диагностика слабее, чем у соседнего отказа `fresh_follower_entry` | Проверял, не бесконечен ли пояс (нет, `DEADLINE = 10s`), поэтому NIT, а не MINOR; проверял, есть ли ранняя проверка `h >= activation` — нет | высокая |
| B2-21 | MINOR | `plane_upstream.rs:114-120` | `HEAD:plane_upstream.rs:118-128` | Метрика переименована `dpos_frontier_unauthenticated_total` → `dpos_frontier_dropped_total`. Смена имени метрики — ломающее изменение для дашбордов/алертов; в журнале §1 (Д-99…Д-107) и §3 (семь групп) отдельным решением не выделена, в §5.4 проекта имя метрики не задано, так что сверить не с чем | Искал упоминание переименования в журнале — нашёл только внутри §0(3) как факт, без пометки «ломающее»; искал потребителей имени вне крейта (`grep` по `crates`/`bins` — нет; девнет не смотрел, он вне границ) | средняя |
| B2-19 | NIT | `tests.rs:3416-3419` | `HEAD:tests.rs:3470-3475` | В «Falsifier» теста `a_lying_upstream_is_refused_at_the_frontier_and_never_lands_its_branch` строка «a landing on a branch the honest three did not execute (then the jump is reading the wrong state)» — переписанный остаток фальсификатора про committee-read; она дублирует предыдущий пункт и не соответствует ни одному ассерту в теле | Прочитал тело теста целиком; ассерта про «ветку, которую честные три не исполнили», в нём нет | высокая |
| B2-20 | NIT | журнал `.dpos-study/history/E4-2-B2.md` §0(6) | — | «Пин отброса вне окна (Б2.6 п.2) — держится зелёным: `preconditions::a_node_more_than_two_epochs_behind…` и `committee_tests::a_node_below_the_chain_refuses…`» — ни один из этих тестов отброс не пинует (первый про лестницу прыжков, второй про отказ модуля без EVM). Абзацем ниже журнал сам это опровергает | Прочитал оба названных теста; ни `deliver`, ни счётчик отброса в них не фигурируют | высокая |

---

## §2. Поведение по ханкам

* `cold_start_jump.rs` — (i) `ElSync` получил `sync_to_checkpoint` (`:363`), FCU-драйв вынесен в
  общий `drive_fcu` (`:437-576`), логи переформулированы без смены семантики; (ii)
  `JumpOutcome` потерял два варианта; (iii) `jump_to_target` потерял `committees`/`ctx`,
  обе стадии и `#[allow(too_many_arguments)]`; (iv) `verify_jump_authenticated` потеряла
  `l1_checkpoint` и L1-откат, отказ безусловный; (v) L1-`Err` переехал `AuthFailed → Stalled`;
  (vi) `cold_start_jump` и `FakeUpstream` удалены, шесть юнитов переписаны под `jump_to_target`,
  два заменены инвертированными, один (`unreadable_committee_with_l1_defers_to_checkpoint`) удалён.
* `dpos.rs` — (i) новый `ColdStartKind::ElFinalized` и его арм в `launch`; (ii) удалены
  `cold_start_jump_eligible`, `JumpDisposition`, `classify_jump_outcome`,
  `cold_start_jump_self_heal`, `empty_archive_requires_landed_jump`, `crash_recover_deferred`,
  `jumped_marshal_floor` на обоих путях; (iii) новый чистый `fresh_follower_entry` +
  `FreshFollowerEntry`; (iv) follower'ский restart-арм якорится на `rf_num/rf_hash`, fresh-арм —
  на checkpoint или (вне deployed) на `get_latest`; (v) `l1_checkpoint` убран из цепочки
  `recover_finalized_tail_into_reth → recover_walk_block/recover_replay_seed →
  refetch_verified_archive_hole`; (vi) `marshal_floor` на обоих путях перестал зависеть от
  посадки прыжка.
* `executor.rs` — сняты арм `BadTarget`, арм `AuthFailed` и гейдж `auth_rotate`; добавлено
  снятие `crash_recover` на опустошении стартового drain'а; удалены три теста.
* `plane_upstream.rs` — отброс вместо passthrough, переименована метрика, три юнита
  инвертированы.
* `sync_metrics.rs` — сняты `AwaitingUpstream`/`AuthRotate`, `ALL` 12 → 10.
* `cert_inlet.rs`, `cert_follow.rs`, `lib.rs`, `outer.rs` — доки и снятие одного аргумента.
* `testbed/{fakes,stand,tests}.rs` — удалены `JumpCommittees`/`JumpCommitteeReads` и
  `Outcome::jump_committee_reads`; `JumpElSync::sync_to_checkpoint` паникует; ладдер-тест
  приобрёл два ассерта; C9 и R-001-var-Б переписаны.
* `node/` — поле `deployed_network`, вынесенный `is_deployed_network`, доки.

## §3. Граница

Запрещённого не нашёл. 14 файлов = список постановки. `testbed/preconditions.rs`,
`testbed/byzantine_roles.rs`, `epoch_manager.rs`, `committee/**`, `beacon/**`, `p2p/**`,
`staking-reader/**`, `bins/**`, `contracts/**`, `Cargo.toml` не тронуты (`git status --short`).
`devnet/**` и `.dpos-study/history/E4-ORCHESTRATOR.md` — чужие, в диффе `-- crates/` отсутствуют.
Новых `#[allow]`/`todo!`/таймеров/поллеров нет; два `#[allow(clippy::too_many_arguments)]` СНЯТЫ.

## §4. Ворота (verbatim)

См. §0(1) — шесть строк `test result` и `b2f-status.txt` приведены дословно оттуда.
Дополнительно (прогнано МНОЙ на этом дереве, точечно):

* `cargo test -p fluentbase-consensus --lib dpos::cold_start_kind_tests` — под мутацией
  `ElFinalized → Restart`: `test result: FAILED. 7 passed; 1 failed; 0 ignored; 0 measured;
  674 filtered out; finished in 0.00s`. После отката мутация снята, дерево сверено `md5sum`.
* `cargo test -p fluentbase-consensus --lib plane_upstream::tests` — под мутацией «снять отброс»:
  `test result: FAILED. 10 passed; 3 failed; 0 ignored; 0 measured; 669 filtered out;
  finished in 0.01s`. Откачено.
* `cargo test -p fluentbase-consensus --lib --features dpos-devnet-byzantine
  testbed::tests::the_ladder_names_successive_rungs_and_the_lagging_node_reaches_every_one --
  --exact` — под мутацией «`produce` отвечает пустыми байтами вместо роняния sender'а»:
  `test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 689 filtered out;
  finished in 159.83s`, печать `deliveries_rejected: 3`. Откачено.

Все три мутации откачены, `md5sum -c gates/b2f-tree.md5` после отката — расхождений нет.

## §5. Оставить как есть

* **Безусловный отказ `verify_jump_authenticated` при нечитаемом комитете** (снятие L1-отката) —
  строго сильнее прежнего и оба потребителя и так передавали `None`. Не возвращать параметр.
* **`deliver` возвращает `true` на отброс.** Правильно: `false` в CW — вечное исключение пира
  (`COMMONWARE_INTERNALS.md:393`), и платить им за СВОЁ отставание нельзя. Мутация подтвердила,
  что `false` здесь разваливает лестницу.
* **`JumpElSync::sync_to_checkpoint` паникует, а не возвращает `Ok`** (`fakes.rs:1360-1372`) —
  тихая заглушка позволила бы будущей фикстуре поверить, что стенд покрывает путь,
  которого он не покрывает. Оставить панику.
* **`sync_to_checkpoint` берёт только хэш, высоту узнаёт из посадки** — конфигу не нужна пара
  `(height, hash)`, а неканонизируемый хэш даёт стоп, а не посадку не туда. Правильная форма.
* **`from = ordering_finalized`, а не `marshal_floor`, в `maybe_re_jump`** (`executor.rs:2553`) —
  вместе с посевом `:1289` это ровно то, что делает #12-прыжок невозможным по построению.
* **Удаление `SyncReason::AwaitingUpstream`** вместе с петлёй «retry until landed» — петля
  существовала только ради требования посадить прыжок, которого больше нет.
* **Сохранённый `verify_jump_structural` как шаг (2) `deliver`** — он там нужен (bind
  payload↔digest до любого чтения комитета); чинить надо ДОКИ (B2-16), а не вызов.

## §6. Вне рамок (для В, 4.3, Э7)

* **Вход weak-subjectivity для RESTART-датадира и для валидатора** — Д-2(б); сегодня у
  валидатора checkpoint'а нет в конфиге вовсе, и это единственный выход из B2-01.
* `is_live_epoch` по tip'у и снятие корроборации (`epoch_manager.rs`) — заход В, как записано.
* Роль «EL сел не туда» (`InvalidTarget ⇒ Fault::corruption` вживую) — Э7.
* **Живой прогон `sync_to_checkpoint`** — ни разу не исполнялся (стенд паникует, юниты
  `unreachable!()`), приёмка 4.2 на девнете.
* **Наблюдаемость отброса на стенде** (B2-06) — требует моста от фасада `metrics` к
  `Outcome`; отдельная работа по инфраструктуре стенда.
* **Правка §5.2/§5.4/§5.5 `E4-CORE-DESIGN.md`** под пять расхождений журнала §0(7) плюс
  переоценка строки §5.4 «Ступень никем не обслужена ⇒ недостижимо» (CW-адресация жёстче) —
  доковая работа, блокирует закрытие строки 4.2.
