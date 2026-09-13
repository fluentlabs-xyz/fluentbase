# SYSTEM

You are operating autonomously. The user is not watching in real time and cannot answer questions mid-task, so asking "Want me to…?" or "Shall I…?" will block the work. For reversible actions that follow from the original request, proceed without asking.

**The deliverable is your assessment, not a change.** Report your findings and your proposed solution, and stop. Do not edit production code, tests, `devnet/`, or `.claude/dpos_architecture/`. The only file you write is the report named in the user message. `git` is read-only for you: no `add`, `commit`, `checkout`, `stash`, `restore`, `reset`, `clean`, `rebase`.

**Do not spawn subagents of any kind, and do not spawn one to verify your own work.** This task requires both sides of a boundary — the consensus layer's entry policy and reth's EL-sync behaviour — to be held in one context. An agent that reads only one side cannot see where the other misreads it, and that mismatch is exactly what this investigation is about.

**Scope.** The question is one specific hole and what the correct systemic answer to it is. It is not a re-audit of the cold-start machinery, not a redesign of the sync architecture, and not a review of the rows that produced the current code. If you notice something substantial outside this hole, note it in one line in the final section rather than pursuing it.

**Evidence rules.** Every factual claim must be traceable to a specific `file:line` you actually opened. A claim about reth or commonware behaviour must cite that dependency's source in the pinned checkout, not its documentation — read `.claude/RETH_INTERNALS.md` and `.claude/COMMONWARE_INTERNALS.md` first, but treat them as an index into the checkout, not as evidence. Mark anything inferred rather than seen with `[ГИПОТЕЗА]` and say what would settle it.

**Comments, doc-comments and `.claude/dpos_architecture/` are not evidence for behaviour.** In this codebase they drift, and the places where they drift predict defects: the last five passes each found a stale one, and this very investigation started from a doc-comment that asserted a property the code had stopped having. Read them to learn intent; establish behaviour from function bodies only. This exclusion is deliberate.

**Completeness over tidiness.** Report every option you considered, including ones you rejected and ones you are unsure about. Do not filter by how good an idea looks, do not suppress an option because you doubt it, and do not merge distinct options into one entry to keep the list short. Mark confidence instead. The user filters; you supply completeness.

Read whole lines: the Read tool, `cat`, or `sed -n`; never `cut -c` or `head -c`. Lines of 500–2000 characters are expected in this repository. Search with `git grep` over `crates`; note that `.claude/` is gitignored, so `git grep` there returns nothing always and you need `grep -rn`. Never read `**/.claude/session-reads/*.jsonl` — those are other sessions' transcripts.

Keep responses focused, brief, and concise. Only correct an earlier statement of your own when the error would change the user's decisions; state such corrections plainly and briefly, then continue.

# USER

Репозиторий `/home/djadjka/Work/fluentbase`, ветка `djadjka/dpos-reth-2.2-squashed`, `HEAD` = `37691136`. В рабочем дереве лежат незакоммиченные правки строки 5.2 — они к этой задаче отношения не имеют, `beacon/**` не читай ради неё. Пути без префикса — от `crates/dpos/consensus/src/`.

Отчёт пиши в `.dpos-study/history/E5-COLDSTART-RESEARCH.md`. Это единственный файл, который ты создаёшь.

## Дыра, которую надо закрыть

Воспроизведена мной дважды подряд, детерминированно, кейсом `make smoke-cert-follow`:

~~~
cert-follower: connected_peers=1, latest_block=0   (три минуты, не меняется)
WARN fluentbase_consensus::dpos: reth does not yet hold the DPoS activation block;
     polling (no give-up) — waiting for the sequencer to produce and persist it height=64
FAIL: cert-follower did not align with v0 past 97 (cert-follower=null|null, v0=0xfa)
~~~

Механизм, проверенный мной по коду (перепроверь каждый пункт сам, я мог ошибиться):

1. `dpos.rs:3000` определяет `mk_el_sync`. `git grep -n 'mk_el_sync'` даёт ровно два использования — `:3054` и `:3078`, **оба внутри ветки `None`** матча `read_geometry(&reader, rf_hash)` (свежий datadir без локально читаемой геометрии).
2. В ветке `Some((activation, interval))` привода EL нет ни одного. Якорь — `rf_hash`, и при `rf_num < activation` вызывается `wait_for_activation_block` (`dpos.rs:3028`).
3. `wait_for_activation_block` (`dpos.rs:280-320`) — чистый опрос: `provider.block_hash(activation)` … `ctx.sleep(POLL)`. Ни FCU, ни sync, ни обращения к пирам.
4. Девнетный follower попадает в ветку `Some`, потому что стакинговый `ChainConfig` читается прямо из генезиса (девнетный комитет эпохи 0 лежит в генезисе). Геометрия есть, datadir содержит только генезис, `activation = 64` — узел паркуется навсегда.
5. Блоки `1..activation−1` — до-DPoS секвенсерные: сертификатов нет, в marshal-архивах их нет, лестница/прыжок их не достают. Единственный источник — devp2p EL-синк, и его никто не вооружает.

## Почему это не чинится одной строкой

Удаление привода было **осознанным**. Комментарий на `dpos.rs:3008-3020` говорит прямо: «The `get_latest ⇒ sync_to` that used to stand here drove the EL onto a height a peer named, with nothing checking it (§5.2 lists it as one of the three unauthenticated entries); it is gone». Это строка 4.2 заход Б2 (коммиты `dacd1bfa`/`8a205eae`), и она исполняла правило проекта: `sync_to` вызывается только со входом, который узел может проверить.

Поэтому «вернуть `sync_to(get_latest())`» — костыль: он воскрешает ровно ту неаутентифицированную дверь, которую Э4 закрыл, и находки `E4-30` («третий `sync_to`») и `E4-01` в реестре — про неё.

**Твоя задача — найти решение, которое это правило соблюдает, либо аргументированно показать, что правило в этом конкретном случае неприменимо и почему.** Второе — законный ответ, но он требует доказательства, а не удобства.

## Что прочитать обязательно

- `dpos.rs`: `launch_follower` целиком (начало `:2916`), `wait_for_activation_block` (`:280`), `fresh_follower_entry` (`:1486`), `derive_cold_start_heights`, `read_geometry`.
- `cold_start_jump.rs`: `RethElSync`, `sync_to`, `sync_to_checkpoint`, `JUMP_THRESHOLD`, `holds`.
- `.dpos-study/history/E4-CORE-DESIGN.md`: §5.2 (правило единого входа), и в §0 — ответы 4 и 5, плюс `Д-2` (там прямо сказано: «обязательный checkpoint `(height, hash)` только для свежего datadir вне devnet (E4-05)»).
- `.dpos-study/REGISTER.md`: находки **E4-05** (follower без checkpoint), **E4-30** (третий `sync_to`), **E4-01**, **E4-28**.
- `.dpos-study/DECISIONS.md`: всё, что относится ко входу и к checkpoint.
- `devnet/local-dpos-smoke/docker-compose.cert-follow.yml` и `docker-compose.yml` (сервис `genesis-init`) — как именно девнет ставит геометрию в генезис и чем запускается follower.
- `.claude/RETH_INTERNALS.md` как индекс, затем сам чекаут reth: **чем именно у reth-2.2 арминуется devp2p-бэкфилл**, нужен ли ему FCU/sync-target, и что делает узел с `--trusted-only --disable-discovery` и одним пиром, которому никто не давал цели. Это половина ответа, и она должна стоять на исходниках reth, а не на доке.

## На что ответить

Отвечай по пунктам, каждый с `file:line` и меткой уверенности.

1. **Верен ли мой механизм по всем пяти пунктам.** Где я ошибся — скажи прямо со свидетельством.
2. **Какова точная область дыры.** При каких условиях узел паркуется: только follower или и валидатор; только `--cert-follow` или любой рестарт; только devnet или и прод. Условие сформулируй как предикат по коду, а не описанием. В частности: разбери случай «узел работал до активации, был остановлен, поднят после» — он попадает сюда в проде?
3. **Правильный ли дискриминатор у ветки.** Сегодня ветвление идёт по «читается ли геометрия локально». Вопрос, на который надо ответить: **то ли это свойство, которое различает случаи?** Кандидат, который я подозреваю, но не проверял: различать надо «держу ли я блок активации», а не «читаю ли геометрию». Проверь, подтверди или опровергни, и назови, какие ещё узлы попадут в каждую ветку при смене дискриминатора.
4. **Чем вообще можно проверить вход ниже активации.** Перечисли всё, что у узла есть локально до активации: хэш генезиса, состояние генезиса, `ChainConfig`, операторский `--dpos.l1-checkpoint`, L1 Rollup-контракт, набор доверенных пиров. По каждому: что именно он позволяет проверить, и хватает ли этого, чтобы посадка на блок активации была аутентифицированной. Отдельно ответь на неудобный вопрос: **до-DPoS секвенсерный участок цепи вообще чем-нибудь заверен?** Если ничем — какие следствия это имеет для §5.2 применительно к этому участку.
5. **Варианты решения — все, что рассмотрел, включая отвергнутые.** По каждому: механизм; какое правило проекта он соблюдает или нарушает; какие находки реестра (E4-nn) он затрагивает; цена в проде; цена в девнете; что ломается, **если его применить наполовину**; чем ты пытался его опровергнуть и почему не вышло. Не отбрасывай вариант за то, что он выглядит некрасиво — отбрасывай за свидетельство.
6. **Твоя рекомендация, одна.** С обоснованием, почему именно она, а не соседние. Назови файлы и функции, которые придётся тронуть, и скажи, попадает ли это в границу строки Э5 (`beacon/**` + перечисленные потребители) или требует правки, принадлежащей закрытому этапу Э4.
7. **Нужна ли правка `devnet/`.** Если да — какая именно и почему она полноценное решение, а не подгонка под зелёный прогон. Если нет — скажи это прямо.
8. **Как это проверяется живьём.** Назови кейс (существующий или новый), который отличит «починено» от «сегодня повезло». Отдельно: чем отличить негативный результат от сломанного стенда, если кейс не воспроизведёт дыру.
9. **Оставить как есть.** Непустой и конкретный перечень: что ты по дороге счёл кандидатом на правку и сознательно оставил, и почему.
10. **Где твоя проверка была слабее всего.** Ранжируй, назови, чем её добрать.

## Форма отчёта

Раздел на каждый из десяти пунктов. Для п. 5 — таблица `вариант | механизм | правило проекта | E4-nn | цена прод | цена девнет | наполовину | чем опровергал | уверенность`.

Уверенность — одна из трёх и всегда явная: `подтверждено кодом` / `подтверждено прогоном` / `опирается на допущение (назови его)`.

## В финальном ответе

Только: (а) верен ли мой механизм — да/нет и где нет; (б) рекомендация одной фразой; (в) таблица п. 5; (г) прямой ответ на вопрос — **является ли до-DPoS участок цепи заверенным хоть чем-то, и если нет, меняет ли это §5.2**; (д) где проверка была слабее всего.

<tone_preference>Кратко и по делу. Без преамбул, без пересказа задания, без предложений помочь дальше.</tone_preference>
