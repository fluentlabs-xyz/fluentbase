# E1-CONTRACT-2 — вторая контрактная сессия: остатки 1.3, 1.6, 1.8 и вестигии W6

Сессия 2026-09-11, ветка `djadjka/dpos-reth-2.2-squashed`, дерево
`/home/djadjka/Work/fluentbase`, контракт `contracts/staking/src`.
`CARGO_TARGET_DIR=/home/djadjka/Work/fluentbase/target-contract` (target делится с
другой сессией, поэтому свой каталог).

Правило свидетельства этой сессии: факт — это file:line, который я открыл, или
команда, которую я прогнал здесь. Комментарий и док-комментарий свидетельством
поведения не считаются (в этом контракте прошлый проход нашёл комментарий,
утверждавший обратное коду, — K-5). Тест — свидетельство, только если сказано, был
ли он красным до правки.

## §0 Прямые ответы

_Заполняется в конце._

## §1 Инвентаризация «до»

HEAD на входе: `89046c93 docs(dpos): record the committee wiring`. В дереве —
незакоммиченные правки ЧУЖОЙ сессии под `crates/dpos/`, `crates/node/` и
`.dpos-study/history/E4-ORCHESTRATOR.md`; их я не трогаю.

Ворота контракта на HEAD, verbatim (прогон мой, 2026-09-11):

| Ворота | Вывод |
|---|---|
| `cargo test` | `test result: ok. 175 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s` |
| `cargo test --features devnet-views` | `test result: ok. 176 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s` |
| `cargo clippy --all-targets -- -D warnings` | `Finished \`dev\` profile [optimized] target(s) in 7.62s` (без единого warning) |
| `cargo fmt --check` | чисто (только шесть `Warning: can't set … unstable features are only available in nightly channel`, они на stable печатаются всегда) |

Совпадает с записью `PLAN.md` §1 (175/0 и 176/0, 09-09).

Счёт `#[test]` по файлам (`grep -c '#\[test\]' src/*.rs`):

| файл | `#[test]` |
|---|---|
| `src/tests.rs` | 151 |
| `src/evidence.rs` | 21 |
| `src/math.rs` | 4 |
| `src/util.rs` | 1 |
| остальные 11 файлов | 0 |
| **итого** | **177** |

177 атрибутов против 175/176 прогнанных — расхождение объяснено, не списано:
в `tests.rs` два теста взаимно исключающие по фиче —
`devnet_view_selectors_match_their_pinned_ids` под `#[cfg(feature = "devnet-views")]`
(`tests.rs:1342-1343`) и `the_production_shape_answers_no_view_selector` под
`#[cfg(not(feature = "devnet-views"))]` (`tests.rs:1358-1359`), плюс
`production_liveness_views_read_the_new_namespace` под фичей (`tests.rs:7848-7849`).
`cargo test -- --list` подтверждает: без фичи `tests` 149, с фичей 150; `evidence` 21,
`math` 4, `util` 1 в обоих случаях.

Строк в `src/*.rs` — 17 897 (`wc -l`), из них `tests.rs` 10 383.

## §2 По пунктам

### П1 (1.6, K-16). Пол комитета при `initialize`

**Что было.** `validate_initialization` (`config.rs:131-145` на HEAD) проверяла cap
только на `!= 0` и `<= MAX_COMMITTEE_SIZE`; комментарий над проверкой прямо объявлял
это намеренным («Deliberately only the zero check here, not the committee floor the
setter enforces»). Гейт `cap < MIN_COMMITTEE_LENGTH` стоял только в сеттере
`set_active_validators_length` (`config.rs:384-390` на HEAD), с ошибкой
`ERR_ACTIVE_VALIDATORS_LENGTH_BELOW_COMMITTEE_FLOOR` и полезной нагрузкой
`(value, MIN_COMMITTEE_LENGTH as u32)`.

**Что сделано.** Тот же гейт, тот же код ошибки, та же пара в нагрузке, перед общей
проверкой `ERR_INVALID_CHAIN_CONFIG` — `config.rs:138-151`. Дизъюнкт
`active_validators_length == 0` из общей проверки убран: после гейта он недостижим
(ноль ниже пола). Комментарий переписан по коду.

**Файлы.** `src/config.rs` (гейт), `src/consts.rs` (док `MIN_COMMITTEE_LENGTH`:
абзац утверждал «`initialize` does not» — это стало ложью в том же изменении),
`src/tests.rs` (новый тест + три места, где фикстура задавала cap ниже пола).

**Тесты.**

- `initialize_refuses_a_committee_cap_below_the_floor` — новый. **Был ли красным до
  правки: да** — прогнан против кода с гейтом, заглушённым на `if false && …`:
  `test result: FAILED. 175 passed; 1 failed`, упал ровно он.
- Тесты, которые правка сломала и которые пришлось поправить, — 14 штук, все по одной
  причине: их фикстура инициализировалась с cap ниже пола. Разобрано в §3 (Д-01).

**Ворота после П1, verbatim:**

    cargo test                        → test result: ok. 176 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
    cargo test --features devnet-views → test result: ok. 177 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
    cargo clippy --all-targets -- -D warnings → Finished `dev` profile [optimized] target(s) in 2.69s
    cargo fmt --check                 → чисто

ABI не затронут: ни одного нового/удалённого/переименованного селектора, сигнатура
`initialize` не менялась. Общие ворота по правилу Э2 не требуются; `cargo test -p
fluentbase-genesis-bootstrap` прогнан отдельно, потому что именно он строит генезис
через `initialize` (результат — ниже).

**sha:** `5386a635` (код), docs — следующим коммитом.

**Общие ворота.** ABI не тронут, но `initialize` — точка входа генезиса, поэтому
прогнан `cargo test` крейта `fluentbase-genesis-bootstrap` (он строит генезис именно
этим вызовом): `test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered
out; finished in 222.66s` (плюс `emit_pop_vectors` 0/0/2 ignored и doc-тесты 0/0).

**Документы.** `.claude/dpos_architecture/12_…md` — строка `MIN_COMMITTEE_LENGTH`
говорила «The contract refuses a cap under it» без указания, где именно; переписана
с пометкой `[REVISED 2026-09-11, Э1.6 / K-16]`, названы оба места. Блок
`verified-against` в `00_preamble.md` дополнен записью 2026-09-11. Правка
`.claude/dpos_architecture/` в коммит НЕ входит и войти не может: каталог целиком
игнорируется (`.gitignore:46 .claude`) и в индексе git отсутствует
(`git ls-files .claude/dpos_architecture/` — ноль файлов). Правка живёт на диске;
`-f` в этой сессии разрешён только для `.dpos-study/`.

## §3 Отклонения Д-nn

### Д-01 (П1). Три способа, которыми правка задела существующие тесты

Плановая строка 1.6 обещала только «добавить тот же гейт»; по факту гейт сломал
14 тестов. Разбор по коду, не по догадке:

1. `liveness_harness` (`tests.rs:7942`) принимает `cap` параметром и пишет его прямо
   в `InitializeCommand`; 11 из 26 её вызовов просят cap 2. Правка: инициализировать
   с `cap.max(MIN_COMMITTEE_LENGTH)`, а под-половый cap дописывать в
   `chain_config_storage().active_validators_length_accessor()` уже после
   `initialize`. Причина, почему это не ослабление: cap ниже пола и до правки не давал
   коммитируемого комитета (`commitEpochCommittee` держит тот же пол), и каждый из этих
   вызовов и так сажает комитет `commit_test_committee`, а не через реальный хендлер.
   Cap в этих тестах управляет шириной среза отбора, и ничем больше.
2. `exclusion_release_skips_tombstoned_and_non_active_validators` (`tests.rs:7648`)
   задавал cap 1. Поднят до `MIN_COMMITTEE_LENGTH` — тест зелёный без других правок,
   то есть значение cap в нём было безразличным.
3. `the_verdict_floor_is_a_stake_share_at_the_production_epoch_length`
   (`tests.rs:8577`) задавал cap 2 при двух валидаторах. Поднят до
   `MIN_COMMITTEE_LENGTH` — зелёный без других правок: cap только усекает отбор, а
   усекать двух валидаторов до четырёх нечего.

Решение `DECISIONS.md` это не меняет.

### Д-02 (П1). Код ошибки для cap 0 при `initialize` сменился

До правки cap 0 давал `ERR_INVALID_CHAIN_CONFIG`, теперь даёт
`ERR_ACTIVE_VALIDATORS_LENGTH_BELOW_COMMITTEE_FLOOR`. Это прямое следствие требования
«тот же код ошибки, что у сеттера» (сеттер на 0 отвечает именно полом). Ни один тест
в дереве не пинил cap 0 (`grep 'active_validators_length = ' src/tests.rs` — 13 мест,
нуля среди них нет). Решение `DECISIONS.md` это не меняет.

## §4 Мутации

_Заполняется в П4._

## §5 Оставлено как есть

_Заполняется по ходу._

## §6 Всплыло

- `devnet/local-dpos-smoke/genesis-bootstrap/src/bootstrap.rs:135` держит
  `seated >= 1`, а `:324` подаёт `seated` как `activeValidatorsLength`. После П1
  стенд с `committee_size` 1..3 будет падать на `initialize`, а не подниматься
  и вставать на первом `commitEpochCommittee`. Поведение по существу то же (цепь
  всё равно не жила), но сообщение другое; `devnet/` вне объёма этой сессии,
  не правил.

## §7 Где проверка слабее всего

_Заполняется в конце._
