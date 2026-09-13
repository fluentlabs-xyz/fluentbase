# Э5 строка 5.1, Ф7 — доковый проход по `.claude/dpos_architecture/`

HEAD `129f2754`, ветка `djadjka/dpos-reth-2.2-squashed`. Ни одной строки кода не изменено;
записаны только `.claude/dpos_architecture/*.md` и этот файл.

## §0 Прямые ответы

### (1) Пересчитанный инвентарь

Команда (verbatim; `grep -rn`, не `git grep` — каталог в `.gitignore`):

```
cd .claude/dpos_architecture && grep -rnE 'BeaconKeys|keys\.rs|carry\.rs|resolve\.rs|key_journal|JOURNAL_RETENTION_EPOCHS|chain_key_epoch|KeySources|beacon_share_resolver|BeaconResolve|for_keys|AgreedKeys|mint_diverges_from_attested|frozen_dkg_qual|DkgQualFor|select_carry_scheme|KeySource|store_floor|InvalidSeed|CommitteePairFor|seed_material_refused_divergent' --include='*.md' .
```

Мой шаблон шире оркестраторского: добавлены `KeySource`, `store_floor`, `InvalidSeed`,
`CommitteePairFor`, `seed_material_refused_divergent`. Поэтому базовые числа не совпадают с
инвентарём в постановке (160 против 140) — это разница шаблона, не дрейфа.

| файл | было | стало |
|---|---|---|
| `03_epoch_machinery.md` | 47 | 15 |
| `00_preamble.md` | 40 | 51 |
| `08_node_integration_crates_node_bins_fluent.md` | 28 | 27 |
| `13_invariants_gotchas_rules.md` | 16 | 7 |
| `09_followers.md` | 15 | 3 |
| `00a_errata_2026_08_10_full_audit.md` | 5 | 6 |
| `02_ordering_core_orderblock_deferred_execut.md` | 4 | 5 |
| `12_consensus_critical_constants.md` | 1 | 3 |
| `15_smoke_cases_as_behavioral_spec_devnet_lo.md` | 1 | 1 |
| `06_staking_layer.md` | 1 | 1 |
| `05_identity_crypto.md` | 1 | 1 |
| `01_system_map.md` | 1 | 1 |
| **всего** | **160** в 12 файлах | **121** в 12 файлах |

Числа выросли там, где я НАПИСАЛ имя удалённой сущности, чтобы сказать «оно удалено» — это
цель прохода, а не остаток. Разбор 121 остатка:

- **40** — старые записи журнала `verified-against` в `00_preamble.md` (строки > 61). По правилу
  самого файла не трогал вовсе.
- **11** — моя НОВАЯ запись `verified-against` (строки 7–61), где имена перечислены как удалённые.
- **12** — `JOURNAL_RETENTION_EPOCHS`: имя ЖИВОЕ (`beacon/mod.rs:126`, алиас
  `crate::SCHEME_RETENTION_EPOCHS`). Дрейфом было только значение `= 1`, оно исправлено; остальные
  12 вхождений — предикаты вида `e + JOURNAL_RETENTION_EPOCHS < now`, верные как есть.
- **4** — ложные срабатывания шаблона, оставлены намеренно: `05_identity_crypto.md:23` и
  `08:1512` — это `keys.rs` крейта `bls` (`ValidatorBlsKeypair`); `08:970` — `keys.rs`/`output.rs`
  инструмента `devnet/local-dpos-smoke/genesis-bootstrap` (проверил: файлы существуют);
  `08:1442` — контрактный маппинг `epochBeaconKeys[E]` из исторического блока про откат 2026-08-17.
- **52** — мои же формулировки «X удалён / X был Y» с маркером `[REVISED|REWRITTEN 2026-09-13,
  Э5 row 5.1]` или внутри такого абзаца.
- **2** — чужая датированная проза, оставленная как история: `08:405` («ревизия 2026-08-21
  зафиксировала переезд `BeaconVerify` в `beacon/resolve.rs`») и исходное предложение пункта
  `00a:62`, к которому я дописал маркер `[CLOSED 2026-09-13]`.

### (2) Какие ПОДРАЗДЕЛЫ переписаны целиком (класс 1 «удалён механизм»)

1. **`03_epoch_machinery.md` §3.2, блок «Group-key writers W1/W3»** (был 90 строк). Описывал
   живьём: `BeaconKeys` как `Arc<RwLock<BTreeMap>>`, `set_pk` с ATTESTED-SOURCE-WINS,
   тиры `LocalDkg < Carried < Agreed`, `retain_from` с исключением `Agreed`,
   `BeaconKeys::notifier`/`subscribe`, писателей W1/W3/W4/W5. Механизма нет целиком: нет ни одного
   писателя ключа, нет тиров, нет ретенции на ключевой стороне. Переписан над новым владельцем
   (`MintIndex` + `ArtifactStore` + `KeyIndex`), исторические инциденты (soak 2026-07-14 v5@epoch77,
   исключение `Agreed` от 2026-08-20) сжаты в один явно помеченный исторический абзац.
2. **`03_epoch_machinery.md` §3.2, блок «ONE ladder / `BeaconKeys::get_pk`» + «own-DKG rung» +
   «follower residual PERMANENT»** (70 строк). Трёхтировая лестница с `KeySources` и
   `store_floor` — и есть тот случай, который нельзя было чинить подменой имён: рунгов больше не
   три, их два, и «флор» нечего исключать. Переписан как `ensure_field`-порядок из четырёх шагов
   `ensure_key`; аргумент own-DKG-рунга (неверный ключ терминален, отсутствующий лишь деградирует)
   сохранён и обобщён до П-3; follower-остаток перестал быть постоянным.
3. **`03_epoch_machinery.md` §3.2, булит «Provenance floor on rung 1»**. Флора нет; переписан как
   «свойство владельца, а не рунга», с переносом аргумента стоимости на мемо `MintIndex`.
4. **`03_epoch_machinery.md` §3.2, булит «No `dkgQual` reader» (follower hole #2)**. Было:
   `frozen_dkg_qual` с одним прод-вызовом на плоскости валидатора, у follower рунг ОТСУТСТВУЕТ.
   Стало: один и тот же `follower::changed_bit` для обоих классов узлов, мемоизация — у `MintIndex`.
5. **`02_ordering_core_orderblock_deferred_execut.md`, «THE VOTE-PATH KEY LADDER'S EPITAPH»,
   список трёх мест, куда ушла работа лестницы**. Все три были места CARRY-DIVERGENCE; класс
   недостижим. Переписан: что выжило от каждого из трёх и почему сравнивать больше нечего.
6. **`13_invariants_gotchas_rules.md` RULE 34** целиком. Правило состояло из тиров, arbitration и
   ретенции — ни одного из трёх нет. Переписано: два факта, один владелец, СОХРАНИВШИЙСЯ инвариант
   («член подписывает полиномом артефакта или не подписывает») с двумя местами его принуждения.
7. **`13_invariants_gotchas_rules.md`, правило «что значит ПРОВАЛИВШАЯСЯ σ»**. Поведение
   изменилось (см. §0(6)), не только якорь.
8. **`08_node_integration…md`, булит «Per-epoch signing key (rotation)»** — переписаны шесть его
   мест (`BeaconResolver`/`BeaconResolve`, арбитр carry, `CarryVerdict`, метрика
   `dpos_carry_forward_refused_total`, читатель бита, «W1-publishes», вход share-gate).
9. **`09_followers.md`, follower-овый ключевой путь** (три абзаца: что читает inlet, что делает
   `ensure_key`/`observe_cert`, что «нагружено» в per-cert вызове) и **булит рунгов inlet'а**.

Поправка к постановке: трёхтировая лестница лежит НЕ в `03 §3.3`. `§3.3` — это
`outer.rs::EpochSchemeProvider` (строка 852 до правок) и в нём нет ни одного попадания;
весь ладдер-текст сидит внутри §3.2. Переписывал там, где текст на самом деле.

### (3) Что оставлено как история и по какому признаку читается историей

- `00_preamble.md`, все записи ниже моей — по правилу файла. Признак: дата в начале записи.
- `08:405` — «2026-08-21 revision recorded `BeaconVerify` surviving a move … to `beacon/resolve.rs`.
  All five are now GONE». Читается историей: явная дата + «recorded» + «are now GONE».
- `00a_errata…md` целиком — датированный отчёт аудита 2026-08-10/08-20. Три его пункта всё же
  получили маркеры, потому что были не историей, а СПИСКОМ ЗАДАЧ, по которому агент стал бы
  действовать: `[SUPERSEDED]` у сводки про предикат RULE 34, `[CLOSED]` у пункта про residue-сайты
  §3.2, и вычерк + `[CLOSED BY DELETION]` у пункта «`beacon/keys.rs:20-22` всё ещё называет
  `record`/`lookup`» (файла нет). В списке «code comments that contradict this document» помечен
  `carry.rs:20` как VOID.
- `08:1329` (`CarryForwardMemo`, `confirm_carry_by_blocks`, `BeaconResolve::Unconfirmed` — «This
  DELETED the former two-rung span-proof system entirely») и `03:166` («were RETIRED — see §0a») —
  оставлены, признак истории есть (`DELETED`/`RETIRED` + ссылка на §0a).
- Четыре ложных срабатывания из §0(1) оставлены без правки: это другие `keys.rs` и контрактный
  `epochBeaconKeys`, к строке 5.1 отношения не имеют.

### (4) Новая запись `verified-against` (целиком, `00_preamble.md:7-61`)

```
  2026-09-13   **Э5 row 5.1 (П-3): the agreed artifact is the ONLY owner of `PK_E`.** FOUR
               FILES DELETED, 2301 lines — `beacon/keys.rs`, `beacon/key_journal.rs`,
               `beacon/carry.rs`, `beacon/resolve.rs` — and with them `BeaconKeys` and its
               three provenance tiers (`KeySource::{LocalDkg, Carried, Agreed}`), the
               `get_pk` ladder and `KeySources`/`store_floor`, `AgreedKeys`, `InvalidSeed`,
               `DkgQualFor`/`frozen_dkg_qual` and the `!(bit || committed)` guard,
               `select_carry_scheme`, `chain_key_epoch`/`chain_key_epoch_memoised`,
               `mint_diverges_from_attested`, `beacon_share_resolver`, `BeaconResolve`,
               `CommitteePairFor`, `for_keys`, the writers W1 and W3, and the metric
               `dpos_seed_material_refused_divergent_total`. WHAT STANDS IN THEIR PLACE, all
               in `beacon/artifact.rs`: `ArtifactStore` (`:423`) insert-only and NEVER
               evicted (`:411-421` says why a window would drop the valuable record first)
               as the only durable owner of the VALUE; `MintIndex` (`:615`, `minted_at`
               `:654`) as which epoch minted the key for the target, over the frozen
               `ChangedAt` bit (`:566`) whose `committed` leg is dropped as Д-7-resolved
               (`:554-566`); `KeyIndex` (`:744`, `key_at` `:761`, `sharing_at` `:771`,
               `holds_mint_of` `:779`) as the one read surface; `open_mint_memo` (`:1163`)
               as a DURABLE write-once `epoch → minted_at` memo that never records an
               undecided bit. `Beacon::ensure_key` is two rungs and takes no floor
               (`beacon/surface.rs:2376-2404`). The polynomial is read from
               `artifact.group_key`, so `CeremonyStore` holds the SHARE alone
               (`beacon/actor.rs:184`) and the share file holds the share alone
               (`beacon/share_state.rs:10-33`); `validate_share_on_poly` became a property
               of `adopt_share` rather than of one call site (`beacon/actor.rs:1048-1063`).
               The ceremony-start decision reads the module's bit (`maybe_start`,
               `beacon/actor.rs:1726`) with no roster pair. BEHAVIOUR CHANGES a reader must
               know: the promote VALUE-GATE and `WithheldReason::KeyDivergence` are gone
               (one value cannot diverge from itself) while the counter
               `epoch_engine_demoted_key_divergence_total` is still REGISTERED with no `inc`
               site (`beacon/metrics.rs:42`, `:218`) — a dead registration, so a panel over
               it reads a permanent zero; off the wire a σ that fails now ALWAYS convicts
               the sender (`certificate_verdict`, `beacon/surface.rs:351-390`), where it
               used to quarantine against an unattested key; retention is ONE window,
               `JOURNAL_RETENTION_EPOCHS` an alias of `SCHEME_RETENTION_EPOCHS = 8`
               (`beacon/mod.rs:126`, `lib.rs:26`), so the recompute-heal window widened
               1 → 8 epochs at roughly +3 MB of disk per node; a member with a share and no
               artifact asks for it every tick (`beacon/actor.rs:2348-2355`); and a
               ZERO-OVERLAP committee boundary is survivable, each half acquiring the
               other's key through `acquire_mint_artifacts` (`beacon/actor.rs:2475`).
               Gates (tag `b5`): consensus lib 656/0, stand 55/0 with the byzantine feature
               and 46/0 without, node 55/0, staking-reader 64/0, `slasher_integration` 16/0,
               clippy exactly 2 warnings (both pre-existing, both outside
               `fluentbase-consensus`) and 0 with the feature, `cargo fmt --check` clean,
               `cargo doc` unresolved link 6. Sections touched: 00 (this block), 00a
               (three superseded errata items), 01 (the module surface), 02 (the vote-path
               ladder's epitaph, rewritten), 03 (§3.2: the group-key writers, the ladder,
               the provenance floor, the pull rung, the follower bit reader — four
               subsections rewritten whole), 06 (§6 `dkgQual` reader), 08 (the file map, the
               key source, the rotation bullet, the retention window, the metric families),
               09 (the follower key path and the inlet rungs), 12 (`SCHEME_RETENTION_EPOCHS`,
               a new `JOURNAL_RETENTION_EPOCHS` row, `CARRY_WALK_CAP`), 13 (RULE 34
               rewritten, 34b's edge, the σ-failure rule, RULE 40 strengthened), 15 (the
               retired metric in the sim row). Journal:
               `.dpos-study/history/E5-1-A.md`; doc pass
               `.dpos-study/history/E5-1-DOCS.md`.
```

Цифру «2301 строки» проверил сам:
`git show --numstat 129f2754 -- …` даёт 478 + 417 + 1012 + 394 = 2301.
Гейты — ЧУЖИЕ (прогоны оркестратора, тег `b5`); я их не запускал и внёс как переданное.

### (5) Дрейф СВЕРХ списка удалённых имён

1. **`SCHEME_RETENTION_EPOCHS` был описан неверно дважды в одном предложении.** Дока
   (`03_epoch_machinery.md`, блок W1/W3, до правки): «`SCHEME_RETENTION_EPOCHS` is now
   `pub(crate)` (defined `outer.rs:246`)». Код: `lib.rs:26` — `pub const SCHEME_RETENTION_EPOCHS:
   usize = 8;`, а `git grep -n 'SCHEME_RETENTION_EPOCHS' -- outer.rs` пуст. Список потребителей там
   тоже был неверен: названы `cert_inlet.rs` и `outer.rs` (ни один не ссылается), не названы
   `committee/mod.rs`, `committee/store.rs`, `beacon/artifact.rs`, `beacon/follower.rs`,
   `beacon/surface.rs`, `beacon/verified_seed.rs`. Исправлено с маркером `[CORRECTED]`.
2. **Мёртвая регистрация метрики.** `beacon/surface.rs:1921` утверждает, что счётчик
   `dpos_engine_demoted_key_divergence_total` удалён вместе с гейтом. Он НЕ удалён:
   `beacon/metrics.rs:42` (поле) и `:218` (регистрация как
   `epoch_engine_demoted_key_divergence_total`), а `git grep 'engine_demoted_key_divergence.inc'`
   пуст. Это ровно та ловушка, о которой предупреждает `15_smoke_cases…md`: hard-zero-tripwire по
   отставной семье проходит вакуумно. Записал в 03 §3.2 и в новую запись преамбулы; сам счётчик —
   правка кода, не моя.
3. **`09_followers.md` держал вводящее в заблуждение «не удаляй это» на вызове, который больше
   ничего не пишет.** Было: «`ensure_key(epoch, Local)` … is the only thing that ever WRITES an
   artifact-sourced key into that store. Delete it and every such epoch stays keyless for the life
   of the process». В строке не было ни одного удалённого имени, поэтому grep её не находил.
   Сейчас `FollowerRandomness::ensure_key` (`beacon/follower.rs:428-440`) — синхронная проба
   `holds_mint_of`, ничего не пишущая; нагружено `observe_cert`'s `try_send` (`:456-461`).
   Переписано, иначе читатель защитил бы не то ребро.
4. **`02_…md` про «provenance floor `ensure_key` enforces»** — тоже без удалённого имени в строке;
   флора в `ensure_key` нет (`beacon/surface.rs:2376-2404`). Переписано вместе с эпитафией.
5. **`13_…md` RULE 34b ссылался на «the same `observe_epoch`/`observe_cert` seam the key store
   uses»** — висячая ссылка на удалённый объект: шов есть, но с 5.1 он чистит только σ-окна.
   Уточнено.
6. **`08_…md`: `beacon/mod.rs` назван «the nine-item public surface»** — в `pub use`
   (`beacon/mod.rs:128-134`) двадцать имён. К строке 5.1 не относится, НЕ правил (см. §0(8)).

### (6) Какие ПРАВИЛА в `13_` и константы в `12_` изменились по существу

`13_invariants_gotchas_rules.md`:

- **RULE 34 — переписано целиком.** Стало неверным по существу, а не по якорю: тиров
  (`KeySource`), arbitration'а (attested-source-wins) и ретенции ключей (`retain_from` тир-за-тиром
  + исключение `Agreed` в любом возрасте) больше не существует. На ключевой стороне ретенции НЕТ
  вообще: `ArtifactStore` намеренно не вытесняется (`beacon/artifact.rs:411-421`) — на долго
  стабильном комитете самая ценная запись САМАЯ СТАРАЯ, и без вытеснения `NotYet` из pull-шва
  никогда не ложь. Сохранившийся инвариант выписан отдельно с двумя местами принуждения:
  `adopt_share` (`beacon/actor.rs:1048-1063`) и `promote_gates` (`beacon/surface.rs:1931-1963`).
- **RULE 34b — сменился ВЛАДЕЛЕЦ ребра.** Перепроверка карантина σ висит на
  `ArtifactStore::subscribe` (`beacon/artifact.rs:426-440`), не на `BeaconKeys::subscribe`;
  аргумент «никогда не общий хэндл, потому что `notify_one` будит РОВНО одного» — тот же.
- **Правило «что значит проваленная σ» — ПОВЕДЕНЧЕСКОЕ изменение.** Было: решение зависит от
  провенанса, против локально восстановленного ключа σ КАРАНТИНИТСЯ и никого не обвиняет.
  Стало: с провода провал ВСЕГДА обвиняет отправителя (`certificate_verdict`,
  `beacon/surface.rs:382-390`), потому что единственный ключ, против которого правило может
  провалиться, — заверенный кворумом `committee[minted_at]`. Отдельно сохранены два НЕ-обвиняющих
  плеча: `NoKey` (карантин, `:351-355`) и дверь спекуляции (отказ без карантина, `:365-368`).
  Заодно поменялось ОБОСНОВАНИЕ «не перепроверять при replay журнала»: старое («ключи прунятся до
  `entered − SCHEME_RETENTION_EPOCHS`, а окно σ мерится раундами») стало пустым — ключи не
  прунятся; выжившая половина — адресуемость (`beacon/artifact.rs:594-605`).
- **RULE 40 — УСИЛИЛОСЬ.** Было «артефакт — единственный источник `PK_E` для узла, который НЕ
  прогонял церемонию» (потому что `CeremonyStore` держал `(Output, Share)`). Стало: единственный
  источник ВООБЩЕ, включая члена комитета, — `CeremonyStore` держит только долю
  (`beacon/actor.rs:184`). Следствие для корollarий: персистенция теперь ОДНА
  (`beacon-artifact-metadata`), второй копии в share-файле нет, и названа цена по ликвидности.
  Добавлена третья популяция, которой раньше не было: валидатор-НЕ-член через
  `acquire_mint_artifacts` (`beacon/actor.rs:2475`), с окном до `now + 1`.

`12_consensus_critical_constants.md`:

- **`JOURNAL_RETENTION_EPOCHS`: 1 → 8.** Новая строка в таблице (её там не было вовсе; значение
  `= 1` жило в `08`). Теперь алиас `crate::SCHEME_RETENTION_EPOCHS` (`beacon/mod.rs:126`). Записана
  и цена: ~3.4 МБ QUAL-наборов при `n = 51` против ~430 КиБ, плюс ВОСЕМЬ эпох секретных
  `ReceivedDealing`-вью на диске вместо одной, все под `0600`.
- **`SCHEME_RETENTION_EPOCHS`**: убрано «bounds the key store's derived tiers» — ключевой ретенции
  нет; добавлено, что он теперь задаёт `JOURNAL_RETENTION_EPOCHS`; исправлены потребители и
  видимость (`pub`, `lib.rs:26`).
- **`CARRY_WALK_CAP`**: замена — не `carry.rs::chain_key_epoch`, а `MintIndex::minted_at`. Ходок
  ПО-ПРЕЖНЕМУ без кэпа и ПО-ПРЕЖНЕМУ на пути сертификата (оракул зовёт `key_at`/`sharing_at`:
  `beacon/oracle.rs:118`, `:218`, `:282`). Изменилась амортизация: биты кэшируются на процесс
  (`:621-624`), ответы мемоизируются write-once И ДОЛГОВЕЧНО (`:1163`), мемоизированная НИЖНЯЯ
  эпоха отвечает за верхнюю по монотонности (`:661-665`). `DPOS_AUDIT.md` B10 по форме остаётся,
  по стоимости — нет.

### (7) Текстовые счётчики (мои команды)

Скрипт: для каждой строки — оканчивается ли на многоточие (оба написания); чётность обратных
кавычек (нечётные строки группируются в цепочки, цепочка нечётной длины = непарная); наличие
двойной запятой; наличие пустой пары обратных кавычек (две подряд, не внутри тройной);
наличие пустых круглых скобок вне код-спана — индекс проверяется против покрытия код-спанов,
иначе скобки вокруг код-спана дают ложняк.

По ВСЕМ 20 файлам каталога:

- строк, оканчивающихся многоточием: **0**
- строк с двойной запятой: **0**
- пустых пар обратных кавычек: **0**
- строк с нечётным числом обратных кавычек: **318** — все складываются в пары (перенесённые
  код-спаны) либо являются ограждениями кодовых блоков.
- «непарных» цепочек нечётных строк: **36** — все разделены пустой строкой внутри пары; ни одна
  не в написанном мной тексте (проверил построчно).
- пустых круглых скобок вне код-спана: **21** — все предсуществующие и все до одной артефакт
  перенесённого код-спана (открывающая кавычка на предыдущей строке); ни одной в моём тексте.

Этот отчёт прогнан тем же скриптом: многоточий 0, двойных запятых 0, пустых пар кавычек 0,
пустых скобок вне код-спана 0, нечётных строк 8 — четыре пары (два ограждения и два
перенесённых код-спана).

### (8) Что осталось недоделанным и почему

1. **Старые записи `verified-against` в `00_preamble.md` (40 попаданий) не тронуты** — по правилу
   самого файла. Это не остаток, а решение.
2. **Код-комментарии, которые теперь лгут, я НЕ правил** (правила прохода: ни строки кода).
   Найденные живые (не «used to») места: `beacon/actor.rs:2463` («the ladder's own
   `chain_key_epoch` walk»), `beacon/artifact.rs:60` и `:1252` («the `PK_epoch` rungs» / «the two
   `PK_epoch` key rungs»), `lib.rs:16-19` («the DERIVED tiers of the beacon's cross-epoch key
   store»), `beacon/plane.rs:292` («`pull_keys` is a `fetch` on the very mailbox»),
   `beacon/actor.rs:1723` («Phase 5 reuses the prior epoch's `BeaconKey`»),
   `beacon/actor.rs:701` и `:8995` (`CommitteePairFor`), и тестовая метка
   `node/src/dpos.rs:2605` `with_label("key_journal_writer")`. Рекомендация — по конвенции §0a:
   править первым же изменением, которое тронет эти файлы.
3. **Мёртвая регистрация `epoch_engine_demoted_key_divergence_total`** (см. §0(5).2) — правка кода.
   В доках я предупредил читателя, но счётчик остался.
4. **«nine-item public surface» в `08`** — стало двадцать имён; к строке 5.1 не относится, а
   править чужой дрейф вне задачи я не стал (правило «не улучшать секции, которых дрейф не
   касается»). Зафиксировано здесь, чтобы не потерялось.
5. **Гейты в новой записи преамбулы — переданные, не мои.** Я не прогонял ни один тест: проход
   доковый. Из чисел постановки я проверил сам только «2301 строки».
6. **Один якорь в моём тексте указывает на диапазон, а не на строку** (`beacon/mod.rs:84-116` для
   аргумента «under-retention is SAFE») — это комментарий длиной в 30 строк, единственной строки у
   него нет.

## §1 Таблица правок

| файл | что было | что стало | якорь живого кода |
|---|---|---|---|
| `00_preamble.md` | — | одна новая запись `verified-against` сверху (строки 7–61) | якоря внутри записи |
| `00a_errata….md:29` | сводка «RULE 34 retention predicate (KeySource::Agreed exempt at any age)» как действующая правка | + `[SUPERSEDED 2026-09-13]`: правило переписано, тиров и ключевой ретенции нет | `beacon/artifact.rs:411-421` |
| `00a_errata….md:60-65` | список из трёх residue-сайтов §3.2 как незакрытая задача | + `[CLOSED 2026-09-13]`: все три переписаны; share-verify probe — единственный оставшийся promote-гейт | `beacon/surface.rs:1931` |
| `00a_errata….md:118` | в списке «code comments that contradict this document» стоит `carry.rs:20` | помечен VOID — файла нет | — |
| `00a_errata….md:123-125` | «`beacon/keys.rs:20-22` всё ещё называет `record`/`lookup`» | вычеркнуто, `[CLOSED BY DELETION]` | — |
| `01_system_map.md:36` | «`absent`/`for_keys`/`for_seeds` … the key store is created inside `beacon::build`» | `for_keys` удалён; внутри `beacon::build` создаётся АРТЕФАКТНОЕ хранилище — единственный владелец `PK_epoch`; четыре файла названы удалёнными | `beacon/artifact.rs:423`, `beacon/plane.rs:451-465` |
| `02_…md:271-300` | эпитафия лестницы: три «gated sites», все три — carry-divergence, с `beacon_share_resolver`, `mint_diverges_from_attested`, `keys.rs::on_invalid_seed`; провенансный флор; метрика `..._refused_divergent_total` | переписано: класс недостижим; что выжило от каждого из трёх; обе метрики-заместителя удалены, живут `dpos_seed_verify_{ok,no_key}_total` | `beacon/surface.rs:2148`, `:2078`, `:351-390`; `beacon/oracle.rs:14-24`, `:455-470` |
| `03_…md:148-155` | «the resolver moved to `beacon/resolve.rs`» | `resolve.rs` удалён; резолв — приватные `material` + `share_probe`, делегат `can_participate` | `beacon/surface.rs:2078`, `:2148`, `:418` |
| `03_…md:163-171` | арбитраж бита = `beacon/carry.rs::chain_key_epoch` | арбитр = `MintIndex::minted_at`, правило выписано | `beacon/artifact.rs:654`, `:572` |
| `03_…md:180-193` | spawn-гейт (2) = promote VALUE-GATE с `KeySource::Agreed`/`BeaconKeys::attested` | гейт (2) = promote SHARE self-probe (`BadShare`); VALUE-GATE и `KeyDivergence` удалены; счётчик остался зарегистрированным без `inc` | `beacon/surface.rs:1931`, `:1954-1960`, `:2277`; `beacon/metrics.rs:42`, `:218` |
| `03_…md:222-311` | блок «Group-key writers W1/W3»: store, тиры, `set_pk`, `retain_from`, `notifier` | ПЕРЕПИСАН: писателей нет, два факта, один владелец; ретенция только у схем/журнала/σ; ребро `subscribe` переехало в `ArtifactStore`; исправлены видимость и место `SCHEME_RETENTION_EPOCHS` | `beacon/artifact.rs:423`, `:566`, `:615`, `:654`, `:744`, `:411-421`, `:426-440`, `:512-519`; `lib.rs:26`; `beacon/plane.rs:740`, `:451-465` |
| `03_…md:330-400` | «ONE ladder `BeaconKeys::get_pk`» + own-DKG-рунг + постоянный follower-остаток | ПЕРЕПИСАН: `ensure_key` — четыре шага, два рунга, без флора; долговечное мемо и что оно НЕ закрывает; follower получил своё хранилище + приобретение | `beacon/surface.rs:2376-2404`; `beacon/artifact.rs:804-806`, `:1163`, `:1156-1162`; `beacon/follower.rs:113`, `:122`, `:135`, `:224`, `:25-33`; `cert_follow.rs:121`; `dpos.rs:1514` |
| `03_…md:490-503` | булит «Provenance floor on rung 1» | ПЕРЕПИСАН: флора нет, свойство у владельца; стоимость ходока снята мемо | `beacon/surface.rs:2376-2404`; `epoch_manager.rs:1950-1953`; `beacon/artifact.rs:1163` |
| `03_…md:637-660` | «the ladder's PULL rung … `chain_key_epoch(E)`», «write-back does NOT stop at `BeaconKeys`» | ключ приобретения = `minted_at`; неразрешимый бит режет ДО приобретения; write-back отдаёт артефакт актору | `beacon/artifact.rs:1686`, `:175`, `:182`; `beacon/surface.rs:2390-2392`; `beacon/actor.rs:2512-2520`; `beacon/plane.rs:451-465` |
| `03_…md:682-694` | follower hole #2: `frozen_dkg_qual` с одним прод-вызовом, у follower рунг отсутствует | ПЕРЕПИСАН: один `changed_bit` для обоих классов, мемоизация у `MintIndex`, `dkg_qual_for`/`held_keys`/`pull_keys` больше нет | `beacon/follower.rs:224`, `:122`; `beacon/artifact.rs:615-630`, `:672-676` |
| `03_…md:757` | «re-spawn idempotent (W1 `BeaconKeys::set_pk` idempotent)» | идемпотентность структурная: `ArtifactStore::insert` FIRST-WINS | `beacon/artifact.rs:483-490` |
| `06_staking_layer.md:790` | `dkgQual` «read by `beacon/carry.rs`» | читается `follower::changed_bit`, мемоизируется `MintIndex`; нога `committed` отброшена (Д-7) | `beacon/follower.rs:224`; `beacon/artifact.rs:615` |
| `08_…md:325` | «`beacon/carry.rs` reads the frozen bit unchanged» | читатель и мемо названы, `carry.rs` удалён, ПРАВИЛО не изменилось | `beacon/follower.rs:224`; `beacon/artifact.rs:615` |
| `08_…md:409-413` | «`beacon_share_resolver` also survived … `dkg_qual_for` still feeds `beacon/carry.rs`» | не выжил: `resolve.rs` и трипвайр удалены, метрика удалена; бит читает `changed_bit` | `beacon/surface.rs:2078`; `beacon/artifact.rs:771`, `:654`; `beacon/follower.rs:224` |
| `08_…md:710-717` | источник ключа = «own DKG `Sharing` ИЛИ артефакт, через `BeaconKeys::get_pk`» | источник ОДИН — артефакт минтящей эпохи через `KeyIndex`; своя `Sharing` больше не источник | `beacon/artifact.rs:744`, `:761`, `:771` |
| `08_…md:802-803` | файловая карта: `beacon/{keys.rs,key_journal.rs,carry.rs}` как живые | помечены удалёнными, работа расписана по `artifact.rs`; добавлена строка про `follower.rs`/`share_state.rs` | `beacon/artifact.rs:615`, `:423`, `:744`, `:1306`, `:1163`; `beacon/share_state.rs:10` |
| `08_…md:1032-1036` | «`absent`, `for_keys` и `for_seeds` are `#[cfg(test)]`» | `for_keys` удалён; тестовый тир — `MintFixture` + `ArtifactStore` | `beacon/mod.rs:157-180` |
| `08_…md:1024-1029` | перечень типов границы без уточнения вариантов `WithheldReason` | `WithheldReason` — ровно три варианта, `KeyDivergence` ушёл | `beacon/surface.rs:53-61` |
| `08_…md:1268-1341` | булит «Per-epoch signing key (rotation)»: `BeaconResolver`/`BeaconResolve`, `select_carry_scheme`, `CarryVerdict`, `dpos_carry_forward_refused_total`, `DkgQualFor`, «W1-publishes», promote VALUE-GATE | шесть мест переписаны: приватный `material`, арбитр `minted_at`, плоский `Option` вместо трёх вердиктов, метрика удалена, читатель/кэш бита названы, publish'а нет, вход share-gate — `Withheld` | `beacon/surface.rs:2078`, `:72`; `beacon/artifact.rs:654`, `:572`, `:1163`, `:621-624`, `:554-566`; `beacon/actor.rs:235-240`, `:2475` |
| `08_…md:1786-1790` | «recompute-heal retention … `JOURNAL_RETENTION_EPOCHS = 1`» | 1 → 8, алиас `SCHEME_RETENTION_EPOCHS`; предикаты те же, число другое; названа цена и почему направление безопасно | `beacon/mod.rs:126`, `:84-116`; `lib.rs:26` |
| `08_…md:2616-2618` | `dpos_seed_material_refused_divergent_total` как живая семья для скрейпа | УДАЛЕНА: не скрейпить, не строить панель; класс недостижим | `beacon/oracle.rs:14-24`, `:455-470` |
| `09_…md:94-100` | ключ «from the ONE key ladder», рунги inlet'а = store + held artifacts | один владелец, локальная проба `holds_mint_of` | `beacon/artifact.rs:744` |
| `09_…md:110-125` | fetch-задача пишет ключ в `BeaconKeys` под `KeySource::Agreed`; ключ приобретения — `carry.rs::chain_key_epoch` | ПЕРЕПИСАНО: `ensure_key` — одна проба на обоих усилиях; успех ФАЙЛИТ артефакт, писать нечего; ключ — `minted_at` | `beacon/follower.rs:428-440`, `:456-461`, `:230-248`; `beacon/artifact.rs:654` |
| `09_…md:134-140` | «per-cert `ensure_key` … the only thing that ever WRITES … delete it and every epoch stays keyless» | вызов ничего не пишет; защищать надо `observe_cert`'s `try_send` | `beacon/follower.rs:428-440`, `:413-426` |
| `09_…md:178-190` | перепроверка карантина на `BeaconKeys::subscribe()` | на `ArtifactStore::subscribe()`, с тем же аргументом про `notify_one` | `beacon/artifact.rs:426-440` |
| `09_…md:243`, `:271-282` | `for_keys` как живой тестовый конструктор поверх заглушки резолвера | `for_keys` удалён; у `for_seeds` деградировано ПРИОБРЕТЕНИЕ (`acquire: None`) | `beacon/mod.rs:159-162` |
| `09_…md:448-472` | рунги inlet'а, `BeaconKeys::get_pk` как писатель, carry-курсор мемоизируется как `KeySource::Carried`, W5 заменён «artifact rungs at `Agreed`» | вызов inlet'а — проба, филят три пути; мемо — `epoch → minted_at`, без тира; замены W5 нет | `beacon/surface.rs:2376-2404`; `beacon/follower.rs:428-440`; `beacon/actor.rs:2475`; `beacon/artifact.rs:654`, `:1163`, `:771` |
| `12_…md:67` | `SCHEME_RETENTION_EPOCHS` «bounds the key store's derived tiers» | ключевой ретенции нет; он же задаёт `JOURNAL_RETENTION_EPOCHS`; исправлены потребители и видимость | `lib.rs:26`; `beacon/mod.rs:126`; `epoch_manager.rs:1701`, `:321` |
| `12_…md` (новая строка) | строки `JOURNAL_RETENTION_EPOCHS` не было | добавлена: 8, было 1, с обоснованием и ценой | `beacon/mod.rs:126`; `beacon/share_state.rs:655`; `beacon/actor.rs:1151` |
| `12_…md:71` | `CARRY_WALK_CAP`: замена — `carry.rs::chain_key_epoch`, некэшируемый ходок с пути голосования | замена — `MintIndex::minted_at`; ходок без кэпа и на пути сертификата, но амортизирован кэшем бит + долговечным мемо | `beacon/artifact.rs:654`, `:621-624`, `:1163`, `:661-665`; `beacon/oracle.rs:118`, `:218`, `:282` |
| `13_…md` RULE 34 | тиры, attested-source-wins, `retain_from` тир-за-тиром, исключение `Agreed` | ПЕРЕПИСАНО: два факта, один владелец, сохранившийся инвариант + два места принуждения, ключевая ретенция отсутствует намеренно | `beacon/artifact.rs:654`, `:423`, `:744`, `:411-421`, `:342-348`; `beacon/actor.rs:1048-1063`; `beacon/surface.rs:1931-1963`; `epoch_manager.rs:1701`, `:1225`; `cert_inlet.rs:706`; `beacon/follower.rs:321`; `beacon/surface.rs:2414` |
| `13_…md` RULE 34b | ребро перепроверки = `BeaconKeys::subscribe` | = `ArtifactStore::subscribe`; шов чистит только σ-окна | `beacon/artifact.rs:426-440` |
| `13_…md` σ-failure | решение по провенансу, `BeaconKeys::on_invalid_seed` | решает `certificate_verdict`; с провода провал всегда обвиняет; два не-обвиняющих плеча; обоснование replay'я переформулировано | `beacon/surface.rs:321`, `:351-355`, `:365-368`, `:382-390`; `beacon/artifact.rs:594-605` |
| `13_…md` RULE 40 | артефакт — единственный источник для того, кто НЕ прогонял церемонию | УСИЛЕНО: единственный источник вообще; персистенция одна; третья популяция — валидатор-не-член | `beacon/actor.rs:184`, `:2405-2500`, `:2475`; `beacon/share_state.rs:10-33`; `beacon/plane.rs:740`; `beacon/artifact.rs:744`, `:615`, `:423` |
| `15_…md:53` | «`dpos_seed_material_refused_divergent_total` replaces …» | вычеркнуто: сам заместитель удалён, hard-zero по нему прошёл бы вакуумно | `beacon/oracle.rs:14-24` |
