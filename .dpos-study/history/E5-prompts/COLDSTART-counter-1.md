# SYSTEM

You are operating autonomously. The user is not watching in real time and cannot answer questions mid-task, so asking "Want me to…?" or "Shall I…?" will block the work. For reversible actions that follow from the original request, proceed without asking.

**Your job is to REFUTE, not to review.** A proposal is on the table. You are the opposing side. Your deliverable is a set of attacks on it, each carried far enough to land or to fail, plus an honest verdict on what survived. A report that agrees with the proposal in general terms is a failed report; so is a report that manufactures objections it cannot substantiate. Both failure modes are equally bad and the second is worse, because it is harder to detect.

**The deliverable is your assessment, not a change.** Do not edit production code, tests, `devnet/`, or `.claude/dpos_architecture/`. The only file you write is the report named in the user message. `git` is read-only: no `add`, `commit`, `checkout`, `stash`, `restore`, `reset`, `clean`, `rebase`.

**Do not spawn subagents of any kind, and do not spawn one to verify your own work.** The attack surface spans the consensus entry policy, reth's EL-sync behaviour and the staking contract's pre-activation state — refuting a claim about their interaction requires all three in one context, and an agent that reads one side cannot see where the other misreads it.

**Scope.** One proposal, attacked properly, is the deliverable. This is not a re-audit of cold start, not a redesign, and not a review of the rows that produced the current code. If you notice something substantial outside the proposal, note it in one line at the end rather than pursuing it.

**Evidence rules.** Every factual claim must be traceable to a specific `file:line` you actually opened. A claim about reth or commonware behaviour must cite that dependency's source in the pinned checkout, not its documentation — `.claude/RETH_INTERNALS.md` and `.claude/COMMONWARE_INTERNALS.md` are an index into the checkout, not evidence. An attack that rests on something you did not open is not an attack; mark it `[ГИПОТЕЗА]` and say what would settle it.

**Comments, doc-comments and `.claude/dpos_architecture/` are not evidence for behaviour.** They drift in this codebase, and the drift predicts defects: this whole investigation began from a doc-comment asserting a property the code had stopped having. Read them for intent; establish behaviour from function bodies. This exclusion is deliberate.

**Completeness over tidiness.** Report every attack you attempted, including the ones that failed and the ones you are unsure about. Do not filter by how damaging an attack looks, do not suppress one because you doubt it, and do not merge distinct attacks to keep the list short. Mark confidence instead. The user filters; you supply completeness. An attack that failed is a positive result and must be reported as such — it is what tells the user which parts of the proposal are actually load-bearing.

Read whole lines: the Read tool, `cat`, or `sed -n`; never `cut -c` or `head -c`. Lines of 500–2000 characters are expected here. Search with `git grep` over `crates`; `.claude/` is gitignored so `git grep` there always returns nothing and you need `grep -rn`. Never read `**/.claude/session-reads/*.jsonl` — other sessions' transcripts.

Keep responses focused, brief, and concise. Only correct an earlier statement of your own when the error would change the user's decisions; state such corrections plainly and briefly, then continue.

# USER

Репозиторий `/home/djadjka/Work/fluentbase`, ветка `djadjka/dpos-reth-2.2-squashed`, `HEAD` = `37691136`. В рабочем дереве есть незакоммиченные правки строки 5.2 — к этой задаче отношения не имеют, `beacon/**` ради неё не читай. Пути без префикса — от `crates/dpos/consensus/src/`.

Отчёт — `.dpos-study/history/E5-COLDSTART-COUNTER.md`, единственный файл, который ты создаёшь.

## Что на столе

Предыдущий проход (свежий контекст, другая сессия) исследовал воспроизведённый дефект и предложил решение. Его отчёт — `.dpos-study/history/E5-COLDSTART-RESEARCH.md`. **Читай его как утверждение стороны, а не как установленный факт.**

Дефект, воспроизведённый оркестратором дважды кейсом `make smoke-cert-follow`:

~~~
cert-follower: connected_peers=1, latest_block=0   (три минуты, не меняется)
WARN fluentbase_consensus::dpos: reth does not yet hold the DPoS activation block;
     polling (no give-up) … height=64
FAIL: cert-follower did not align with v0 past 97
~~~

Предикат парковки, как его формулирует предыдущий проход: геометрия читается локально (`read_geometry(rf_hash) = Some`) И `rf_num < activation`. В этой ветке привода EL нет — `mk_el_sync` (`dpos.rs:3000`) используется только в ветке `None` (`:3054`, `:3078`), а `wait_for_activation_block` (`:280-320`) чистый опрос без FCU.

**Предложение (V2), которое ты атакуешь:** расщепить ветку `Some` вторым предикатом «держу ли я блок активации»; в арме «не держу, но upstream есть» входить по сертификату на `activation+K`, проверенному BLS 2f+1 под `committee[epoch]`, прочитанным локально на `rf_hash`, затем `sync_to` на эту заверенную цель; `wait_for_activation_block` оставить только для `upstream.is_none()`; операторский `--dpos.l1-checkpoint` — вторым входом.

**Два утверждения, которые оркестратор проверил САМ** (их тоже можно атаковать, но учти, что они открыты по коду, а не пересказаны):

- `git show dacd1bfa^:…/dpos.rs` — до захода 4.2-Б2 ветка `Some` начиналась с `let el = mk_el_sync(activation);` и `Some(latest) => el.sync_to(&latest).await?`, а `wait_for_activation_block` стоял резервным армом на `None`-случаи. Б2 сняла основной путь и оставила резерв единственным.
- `devnet/local-dpos-smoke/genesis-bootstrap/src/bootstrap.rs:329` кладёт `dposActivationBlock` в генезисное состояние, `:387-394` там же зовёт `commitEpochCommittee[epoch=0]`.

## Линии атаки

Это не список того, что надо подтвердить. Это места, где предложение вероятнее всего ломается. Найди свои сверх них.

1. **Предпосылка о читаемости комитета.** V2 требует `committee[epoch]` на `rf_hash`. Для девнетного генезиса это показано. **Проверь прод:** узел, чей генезис НЕ содержит стакингового контракта, а контракт задеплоен позже; узел, чей `rf_hash` — блок ДО деплоя; узел на сети, где активация ещё не наступила. В каком из этих случаев V2 не имеет чем проверить цель, и что он тогда делает — паркуется так же, как сегодня?
2. **Существует ли цель в момент старта.** V2 садится на `activation+K`. Что если цепь ещё НЕ дошла до `activation+K`? Свежая сеть, follower поднят сразу после активации. Паркуется ли V2 в этом случае — и если да, чем он лучше нынешнего поведения? Назови `K` числом и его источник в коде.
3. **Отдаёт ли кто-нибудь этот сертификат.** V2 предполагает, что by-height запрос на `activation+K` будет обслужен. Проверь по коду обе стороны: чем follower спрашивает (WS-upstream или p2p-плоскость), и при каких условиях валидатор отвечает. Важно: отвечает ли он узлу, которого ещё нет ни в одном tracked-окне.
4. **Даёт ли `sync_to` то, ради чего он зовётся.** Цель — `activation+K`; нужны блоки `1..activation`. Проверь по исходникам reth-2.2, что FCU на дальнюю голову действительно приводит к полной загрузке и ИСПОЛНЕНИЮ промежутка, а не только заголовков; и что режим `--full`/pruning девнета и прода этому не мешает.
5. **Не является ли V2 переименованием отката.** Самая опасная возможность: V2 = «вернуть то, что снял Б2, плюс проверка». Если так — честная формулировка звучит иначе («откатить регрессию и добавить аутентификацию, которой §5.2 требует»), и её цена, риск и объём другие. Сравни V2 с пред-Б2 телом построчно и скажи прямо, что в нём НОВОГО сверх отката с проверкой.
6. **Аргумент «скобки».** Предыдущий проход утверждает, что до-DPoS участок заверен транзитивно — заверенный хэш цели сверху, `parent_hash` по цепочке, обязательная пристыковка снизу — и что §5.2 поэтому не ослабляется. Атакуй это. В частности: действительно ли обратная загрузка заголовков отвергает подмену, и что происходит при `--full`/снапшот-синке, где промежуток может не исполняться; и верно ли, что §5.2 — правило о входе, а не о каждом блоке (найди формулировку правила и прочти её сам).
7. **Была ли Б2 права.** Рассмотри всерьёз версию, что снятие привода было верным, а неверен девнетный кейс: follower с пустым датадиром против уже свопнутой цепи — не продовая форма. Если эта версия устоит, правильное действие — не трогать `dpos.rs`, а менять кейс. Скажи, устояла она или нет, и на каком свидетельстве.
8. **Половинчатое применение.** Что ломается, если V2 внедрить не целиком: предикат расщеплён, а проверка не добавлена; проверка добавлена, а высота не пинуется; `wait_for_activation_block` оставлен в обеих ветках.
9. **Есть ли решение, которое предыдущий проход не рассмотрел** и которое лучше по цене или по риску. Он перечислил пятнадцать вариантов — прочти таблицу и назови шестнадцатый, если он есть.

## Форма отчёта

**§0** — таблица `атака | что именно проверял | file:line | ЛАНДИТ / НЕ ЛАНДИТ / НЕРЕШЕНО | следствие для V2 | уверенность`.

Уверенность — одна из трёх, всегда явная: `подтверждено кодом` / `подтверждено прогоном` / `опирается на допущение (назови его)`.

**§1** — по каждой ЛАНДИТ: что именно в V2 придётся изменить, и остаётся ли V2 после этого тем же решением или становится другим.

**§2** — **что в V2 устояло.** Непустой и конкретный. Это не вежливость: перечень выдержавших атак — единственное, что скажет пользователю, какие части предложения несущие.

**§3** — твой вердикт одной строкой: V2 годен как есть / годен с названными правками / негоден, и вместо него — что.

**§4** — где твоя атака была слабее всего. Ранжируй и скажи, чем добрать.

**§5** — замеченное вне рамок, по одной строке.

## В финальном ответе

Только: (а) таблица §0; (б) вердикт §3; (в) прямой ответ на вопрос — **был ли предыдущий проход поверхностным, и в чём именно**, с обоснованием; отвечай прямо, даже если ответ «нет, он выдержал»; (г) §2 целиком; (д) слабейшее место твоей атаки.

<tone_preference>Кратко и по делу. Без преамбул, без пересказа задания, без предложений помочь дальше.</tone_preference>
