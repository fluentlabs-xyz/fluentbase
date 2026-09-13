# SYSTEM

You are operating autonomously. The user is not watching in real time and cannot answer questions mid-task, so asking 'Want me to…?' or 'Shall I…?' will block the work. For reversible actions that follow from the original request, proceed without asking. Stop only for destructive actions or genuine scope changes the user must decide.

Before ending your turn, check your last paragraph. If it is a plan, a question, a list of next steps, or a promise about work you have not done ('I'll…', 'next I will…'), do that work now with tool calls. That includes retrying after errors and gathering missing information yourself. Do not stop because the context or session is long. End your turn only when the task is complete or you are blocked on input only the user can provide.

**The deliverable is a DECISION plus its reasoning, written to one file.** Report it and stop. Do not apply a fix, do not edit production code, tests, `devnet/`, or `.claude/dpos_architecture/`. The only file you write is the one named in the user message. `git` is read-only for you: no `add`, `commit`, `checkout`, `stash`, `restore`, `reset`, `clean`, `rebase`.

You are being asked to decide, not to survey. A document that lays out options and leaves the choice open is a failed deliverable. Choose, say what the choice costs, and say what would have to be true for the choice to be wrong. A step you have decided on is something to run, not to announce.

**Do not spawn subagents of any kind.** Two prior passes already ran in separate contexts and their reports are your input; a third delegated opinion adds nothing and would reintroduce the relay problem this decision exists to settle.

The number of tokens used to edit files is best minimized. When it will not affect the end result, surgically edit a file rather than rewrite the entire thing.

Remove mannered prose. Avoid the tic of negating a word to elevate it — "not a preference, a requirement", "not a bug, a design choice", "this isn't X, it's Y". Say the thing directly. One such construction in a document is a dial worth turning; five is a habit that makes every claim read like rhetoric rather than a finding.

Read whole lines: the Read tool, `cat`, or `sed -n`; never `cut -c` or `head -c`. Lines of 500–2000 characters are expected in this repository, and the two input reports have some over 1000. Before ending, run these checks on the file you wrote and fix what they find: lines ending in "…"; odd number of backticks per line (`awk '{n=gsub(/`/,"`"); if(n%2==1) print FILENAME": "NR}'`); `,,`, empty `` `` ``, empty `()`. Report the counts.

Search with `git grep` over `crates`; `.claude/` is gitignored, so `git grep` there returns nothing always and you need `grep -rn`. Never read `**/.claude/session-reads/*.jsonl` — those are other sessions' transcripts.

Write the document once. Do not draft it in your reasoning and then write it again; compose it directly into the file.

# USER

Репозиторий `/home/djadjka/Work/fluentbase`, ветка `djadjka/dpos-reth-2.2-squashed`, `HEAD` = `37691136`. Пути без префикса — от `crates/dpos/consensus/src/`.

Решение пиши в `.dpos-study/history/E5-COLDSTART-DECISION.md`. Это единственный файл, который ты создаёшь или меняешь.

## Что уже установлено (не перепроверяй целиком, но опирайся только на проверенное)

Воспроизведён дефект: follower, чей reth ниже блока активации DPoS, паркуется навсегда. Предикат — `read_geometry(rf_hash) = Some` И `rf_num < activation`. В этой ветке привода EL нет: `mk_el_sync` (`dpos.rs:3000`) используется только в ветке `None` (`:3054`, `:3078`), а `wait_for_activation_block` (`:280-320`) — чистый опрос без FCU. Блоки `1..activation−1` — до-DPoS секвенсерные, на ордеринг-плоскости их нет. Кейс `make smoke-cert-follow` падает детерминированно, дважды подряд.

Два прохода в отдельных контекстах уже отработали, читай их отчёты целиком:

- `.dpos-study/history/E5-COLDSTART-RESEARCH.md` — исследование, 15 вариантов, рекомендация **V2**.
- `.dpos-study/history/E5-COLDSTART-COUNTER.md` — контр-разбор, 13 атак, 5 ландят, вердикт «V2 годен с тремя правками».

**V2:** расщепить ветку `Some` вторым предикатом «держу ли блок активации»; в арме «не держу, но upstream есть» входить по сертификату на `activation+K`, проверенному BLS 2f+1 под `committee[epoch]`, читаемым локально на `rf_hash`, затем `sync_to` на эту заверенную цель; `wait_for_activation_block` — только для `upstream.is_none()`; `--dpos.l1-checkpoint` — второй вход.

**Три правки контр-разбора:** (1) checkpoint-вход ПЕРЕД сертификатным — иначе `assert_l1_checkpoint` (`dpos.rs:3118-3121`, считается от посадки) превращает вечный парк в фатальный отказ; (2) цель — первая ОБСЛУЖЕННАЯ высота из окна читаемых эпох, а не ровно `activation+K` — иначе каскадный follower (у tier-2 единственный апстрим tier-1 с окном `JUMP_THRESHOLD`) и зрелая сеть не закрыты; (3) отдельный арм «цепь ещё не дошла до активации».

**Что оркестратор проверил своими руками** (это факты, не реляция; остальное в обоих отчётах — заявления авторов):

- `git show dacd1bfa^:…/dpos.rs` — до захода 4.2-Б2 ветка `Some` начиналась с `let el = mk_el_sync(activation);` и `Some(latest) => el.sync_to(&latest).await?`; `wait_for_activation_block` был резервным армом на `get_latest()==None` и на `upstream.is_none()`. Б2 сняла основной путь и оставила резерв единственным.
- `E4-CORE-DESIGN.md:514` — «`sync_to` — единственный писатель EL-тегов на этом пути, и это не меняется; **меняется только его вход**». `:516` — «Три сегодняшних входа: прыжок, **follower на существующем datadir**, follower на свежем datadir… **Первые два идут через `Frontier`**… Третий — единственный, где проверить нечем». Проект написал «пропустить вход через проверенную цель»; Б2 реализовала «снять вход».
- `dpos.rs:3118-3121` — `assert_l1_checkpoint` стоит после match-а, безусловно при заданном checkpoint, и считается от посадки.
- `order_block.rs:20` — `K = 3`; `:173-179` — `result_target` отдаёт `PreActivation` при `height < anchor + K`. Сертификат ровно на `activation+K` несёт хэш ровно блока `activation`; ниже — нулевой `result`.
- `devnet/local-dpos-smoke/genesis-bootstrap/src/bootstrap.rs:329` кладёт `dposActivationBlock` в генезисное состояние; `:386-394` — там же ровно ОДИН `commitEpochCommittee` (эпоха 0). Утверждение исследования «на генезисе читаются комитеты 0..2» неверно.

## Контекст, который делает это решением, а не задачей

Идёт этап Э5 (`.dpos-study/PLAN.md`, блок «### Э5.»), оркестратор ведёт его по строкам 5.0а → 5.1 → 5.2 → 5.3 → 5.4. Прочитай `.dpos-study/history/E5-ORCHESTRATOR.md` — там правила этапа, hard-stop'ы, текущая фаза и квитанции К-1…К-75.

Существенное:

- Строки 5.0а и 5.1 закрыты и закоммичены. **Строка 5.2 доведена до зелёных ворот и НЕ закоммичена** (`--lib` 665/0, стенд с фичей 55/0, без фичи 46/0, всё остальное на базовых числах — прогоны оркестратора, тег `c3`). Заход В строки 5.2 (стенд-тест R-129) не начат. Строки 5.3 и 5.4 не начаты.
- Главный результат 5.2 — удаление `beacon/follower.rs` целиком со слиянием в `LiveBeacon`. **Единственное живое покрытие этого слияния — `make smoke-cert-follow`**, и именно он падает из-за дефекта, о котором идёт речь. Юнитами путь `dpos.rs:3426 → build_follower → LiveBeacon` не исполняется, стенд `--cert-follow` узлы не поднимает.
- Правка V2 лежит в `crates/dpos/consensus/src/dpos.rs` — продакшн-код **строки 4.2 закрытого этапа Э4**, вне границы строки 5.2 (её список файлов — `beacon/**` плюс перечисленные потребители).
- Правило этапа, заданное пользователем дословно: «Если агент или ревью поднимет что-то существенное вне строки, это идёт одной строкой в «Всплыло» состояния, и этап идёт дальше». Это правило говорит «записать и продолжить».
- Пользователь отдельно разрешил менять `devnet/`, с условием: решения должны быть обоснованные и полноценные, а не костыль ради зелёного прогона.
- Отдельный открытый хвост: харнесс `verdicts_follow.py:237` ждёт лог-строку `"cert-follow: PK_epoch obtained…"`, а код пишет `"beacon: PK_epoch obtained…"` — префикс сменила строка 5.1, когда приобретение ключа переехало в `artifact.rs` и стало общим для обоих классов узлов. Четыре сайта `asserts_follow.py` делают `poll`/`check` по литералу, то есть это жёсткий FAIL фазы 4a.

## Что решить

Реши каждый пункт и обоснуй. Не перечисляй варианты без выбора.

1. **Порядок работ.** Чинить холодный старт сейчас, вскрывая закрытую строку 4.2 Э4, — или довести 5.2 до коммита и захода В, а регрессию оформить отдельной позицией Э4? Правило этапа говорит второе; приёмка 5.2 при этом остаётся открытой, и удаление `follower.rs` уходит в коммит без единой живой проверки. Взвесь это и выбери.
2. **Коммитить ли 5.2 до починки.** Если да — что именно записать в строку `PLAN.md` и в приёмку, чтобы «покрытия нет» не потерялось.
3. **Объём правки, если чинить.** V2 с тремя правками контр-разбора — целиком, частично или иначе. Если частично, скажи, какая часть отложена и почему её отсутствие не оставляет узел в состоянии без выхода.
4. **Правка `devnet/`.** Нужна ли, какая именно, и почему она полноценное решение. Отдельно реши судьбу расхождения лог-строки: править харнесс, править код, или заменить проверку по логу на проверку по метрике (`dpos_follower_artifact_adopted_total` на `:9100` — проверь сам, существует ли она и инкрементится ли).
5. **Как это проверяется живьём.** Назови кейс, который отличит «починено» от «сегодня повезло», и скажи, чем отличить негативный результат от сломанного стенда.
6. **Чей это этап.** Регрессия принадлежит Э4. Реши, оформляется она как повторное вскрытие строки 4.2, как новая строка Э4, или как строка Э6/Э7 — и назови, где её место в `PLAN.md`.

## Форма документа

Раздел на каждый из шести пунктов. В каждом: **РЕШЕНИЕ** одной фразой, затем обоснование, затем **«чем я плачу»** (что теряется от этого выбора), затем **«при чём я был бы неправ»** — конкретное условие, при котором решение надо пересмотреть, а не общая оговорка.

В конце документа — раздел **«Порядок исполнения»**: нумерованная последовательность шагов, которую оркестратор может выполнять, не принимая новых решений.

Если по ходу окажется, что какой-то из установленных фактов выше неверен, — это ценный результат: скажи прямо, с `file:line`, и перестрой решение вокруг настоящего факта.

## В финальном ответе

Только: шесть решений по одной фразе каждое; раздел «Порядок исполнения» целиком; одна фраза о том, какое из шести решений наименее уверенное и почему; счётчики проверок текста, которые ты прогнал.

Ограничение на объём: документ должен быть написан один раз, прямо в файл. Не держи его черновик в рассуждениях.
