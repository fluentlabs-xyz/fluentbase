# Работа 1.8 — тесты контракта под правилом Д-12

Дата: 2026-09-08. `A:` — `/home/djadjka/Work/audit-482/pr482-study`, `B:` — `/home/djadjka/Work/fluentbase`.
Пути без префикса — `A:contracts/staking/src/`.

Правило приёмки Д-12: тест, зелёный при сломанном коде, не считается тестом. Применено
к каждому тесту классов «деньги» и «личность»: снималась именно та проверка, которую
тест охраняет по имени, и записывался исход прогона.

Сделано: **97 мутаций** контракта в копии дерева A (`cargo metadata` проверен до первого
прогона, `CARGO_TARGET_DIR` внутри копии, дерево A не мутировалось ни разу); **семь
правок тестов контракта** — два переписаны, пять новых; **два новых теста e2e** на
настоящем rWasm и **три мутации e2e** с пересборкой блоба; один тест e2e превращён из
печати в утверждение; одно изменение тестового хоста с тремя тестами на само себя.

Базовые линии, все прогоны мои [KNOWN]:

| ворота | до | после |
|---|---|---|
| `cargo test --features devnet-views` (`contracts/staking`) | 169 / 0 | **174 / 0** |
| `cargo test` без фичи (`contracts/staking`) | 168 / 0 | **173 / 0** |
| `cargo test -p fluentbase-e2e --release` | 120 / 0 / 9 ignored | **122 / 0 / 9 ignored** |
| `cargo test -p fluentbase-testing` | 3 / 0 | **6 / 0** |

`cargo clippy --all-targets` и `cargo fmt --check` чисты на всех трёх затронутых
крейтах.

Коммит в дереве A: **`eb09db32`** `test(staking): make every money and identity test
red under the check it names` — только `contracts/staking/src/tests.rs`,
`crates/testing/src/host.rs`, `e2e/src/staking_reserve.rs`. Два файла с чужими
незакоммиченными правками (`contracts/staking/README.md`, `contracts/staking/src/events.rs`)
не тронуты и в коммит не вошли.

**Состязательная проверка.** По готовности работы отдельный ревьюер прошёл диф с
заданием опровергать, а не подтверждать. Десять пунктов из двенадцати подтверждены по
исходникам; два реальных дефекта в моих правках он нашёл, и оба исправлены до коммита:

1. **Вакуумное утверждение в новом тесте.** `assert!(funding.borrow().pulls.is_empty())`
   в `an_epoch_with_a_committee_but_no_recorded_block_draws_no_pot` не могло упасть:
   `pulls` пополняется только на `SIG_ERC20_TRANSFER_FROM`, а путь начисления
   `transferFrom` не делает вовсе — проверено, все четыре вызова `safe_transfer_from`
   стоят вне закрытия (`staking.rs:1082`, `:1177`, `:1865`, `:1904`). Утверждение
   удалено, вместо него в коде стоит объяснение, почему его там быть не должно;
   несущий груз несёт контрольная нога (та же фикстура с одним блоком платит полный пот),
   и она бы упала, если бы резерв не покрывал пот.
2. **Односторонние утверждения в бюджетном цикле части 3.** `burnt_close.succeeded` и
   `headroom >= floor` оба выполняются и тогда, когда токен ПЕРЕСТАЛ жечь: работы
   меньше — остаток больше. `assert_token_refuses` вызывается только в безлимитной ветке
   и только при комитете 5, то есть для комитетов 21 и 51 — ради которых цикл и
   существует — факт ожога не проверялся ничем. Добавлены три утверждения: горящее
   закрытие потратило больше газа, чем честное; горящее ФОРФЕЙТНУЛО эпоху; честное её
   ПРОФИНАНСИРОВАЛО. Каждое показано красным отдельной мутацией фикстуры —
   `Refusal::BurnFuel` → `Refusal::Revert` роняет первое (строка 1104), читаемый резерв в
   горящей ветке роняет его же (`:1105`), снятие `setBlendReserve(OWNER)` в честной ветке
   роняет третье (`:1118`).

Третий дефект — мёртвое поле `Fixture.committee` в `e2e/src/staking_reserve.rs` —
существовало до этой работы (`git show 95385229^:e2e/src/staking_reserve.rs`, строка 304)
и не тронуто: чистка чужого кода вне правила приёмки. Комментарий «four zero-returns»
в новом тесте исправлен — ветки теперь перечислены, а не сосчитаны.

---

## 1. Реестр 149 тестов `tests.rs`

144 теста были до правки, 149 после: пять новых, два переписанных. Класс «личность (улики)» — двенадцать тестов маршрута улик, отложенных с работой 1.2; они перечислены, но не мутировались. Столбец «мутация» — номера мутаций, которые ЭТОТ тест убили.

| # | имя | стр. | класс | охраняемая проверка | заглушка | мутация | до правки | действие | после |
|---|---|---|---|---|---|---|---|---|---|
| 1 | `compact_storage_matches_solidity_struct_layouts` | 60 | прочее | `storage.rs` — раскладка полей в слоте | — | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 2 | `contract_storage_uses_separate_erc7201_namespaces` | 117 | прочее | `consts.rs:502-507` — корни ERC-7201 | — | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 3 | `solidity_bytes_calldata_reaches_staking_handlers` | 805 | ABI-вью | `consensus.rs:106-111`, `types.rs` — декодер calldata | мок PAIRING/предеплоев | M64 | красный | оставлен | красный |
| 4 | `register_validator_cast_calldata_registers_consensus_keys_atomically` | 867 | личность | `consensus.rs:215-224` — запись ключей и привязки; `staking.rs:1082` — источник взноса | мок PAIRING/предеплоев + дефолтный мок токена (`mock_external_return`) | M88, M91b | красный | оставлен | красный |
| 5 | `solidity_bytes_outputs_and_event_match_cast_vectors` | 970 | ABI-вью | `events.rs` — сигнатуры и кодирование | — | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 6 | `get_consensus_keys_matches_dynamic_struct_return_vectors` | 1110 | ABI-вью | `consensus.rs:244-257` — `write_validators_with_keys` | мок PAIRING/предеплоев | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 7 | `parameterized_custom_errors_use_solidity_abi` | 1167 | ABI-вью | `util.rs:21-33` — `revert_with` | `Harness` | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 8 | `derived_selectors_match_independent_hex_pins` | 1192 | ABI-вью | `consts.rs` — `derive_keccak256_id!` | — | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 9 | `devnet_view_selectors_match_their_pinned_ids` | 1263 | ABI-вью | `consts.rs` — селекторы devnet-вьюх | — | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 10 | `the_production_shape_answers_no_view_selector` | 1279 | ABI-вью | `lib.rs` — маршрутизация под `devnet-views` | `Harness` | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 11 | `production_liveness_event_signatures_match_the_solidity_abi` | 1296 | ABI-вью | `events.rs` — сигнатуры liveness-событий | — | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 12 | `initializes_registry_and_preserves_solidity_read_abi` | 1367 | отбор | `staking.rs:382-401` — `selected_validators` | мок PAIRING/предеплоев | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 13 | `epoch_number_is_rebased_to_initialization_block` | 1413 | конфиг | `util.rs:77-85` — `current_epoch_at_block` | `Harness` | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 14 | `governance_lifecycle_updates_active_registry` | 1432 | личность | `util.rs:59-65` — `ensure_governance`; `staking.rs:124-127` — привязка владельца | мок PAIRING/предеплоев | M84 | красный | оставлен | красный |
| 15 | `register_validator_rejects_a_subminimum_bond` | 1546 | деньги | `staking.rs:1059-1061` — минимум взноса | мок PAIRING/предеплоев | M21 | красный | оставлен | красный |
| 16 | `a_raised_validator_minimum_does_not_block_activating_an_earlier_registrant` | 1586 | деньги | `staking.rs:928-930` — ОТСУТСТВИЕ минимума при активации | мок PAIRING/предеплоев | M89 | красный | оставлен | красный |
| 17 | `an_owner_who_withdrew_his_whole_bond_cannot_be_activated` | 1659 | деньги | `staking.rs:928-930` — нулевой самостейк | мок PAIRING/предеплоев + дефолтный мок токена (`mock_external_return`) | M22, M57 | красный | оставлен | красный |
| 18 | `handlers_refuse_value_and_refuse_to_mutate_inside_a_static_frame` | 1746 | прочее | `util.rs:35-47` — `ensure_non_payable` и `ensure_mutable` | `Harness` | M101, M102 | теста не было | **новый тест** | красный |
| 19 | `staking_is_a_genesis_rwasm_contract_not_a_system_precompile` | 1801 | прочее | таблицы адресов SDK (не код контракта) | — | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 20 | `stores_chain_configuration_in_its_own_namespace` | 1807 | конфиг | `config.rs:33-129` — `apply_initial_config` | мок PAIRING/предеплоев | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 21 | `governance_updates_embedded_chain_configuration` | 1869 | конфиг | `config.rs` — сеттеры и `ensure_governance` | мок PAIRING/предеплоев | M84 | красный | оставлен | красный |
| 22 | `initialize_events_report_defaults_as_previous_values` | 1984 | конфиг | `config.rs:77-127` — события инициализации | мок PAIRING/предеплоев | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 23 | `initializer_rejects_mismatched_arrays_without_persisting_state` | 2076 | конфиг | `initializer.rs:82-88` — длины массивов | мок PAIRING/предеплоев | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 24 | `initializer_rejects_subminimum_active_validator` | 2105 | деньги | `initializer.rs:92-98` — минимум генезис-ставки | мок PAIRING/предеплоев | M90 | красный | оставлен | красный |
| 25 | `initializer_is_permissionless_for_atomic_deployment_but_one_shot` | 2132 | конфиг | `initializer.rs:29-33` — одноразовость | мок PAIRING/предеплоев | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 26 | `initializer_pulls_genesis_stake_from_declared_sponsor` | 2159 | деньги | `initializer.rs:75, :102-112` — кто платит генезис-ставки | `Harness` | M26 | красный | оставлен | красный |
| 27 | `initialize_and_registration_reject_bad_commission_and_duplicate_validator` | 2199 | деньги | `initializer.rs:89`, `staking.rs:1044`, `:85` — потолок комиссии; `staking.rs:94`, `:1047` — дубль валидатора | мок PAIRING/предеплоев | C1, C2 | красный | оставлен | красный |
| 28 | `register_validator_verifies_and_stores_consensus_keys_in_one_call` | 2245 | личность | `consensus.rs:209-224` — запись ключей, эпохи активации и привязки | мок PAIRING/предеплоев + дефолтный мок токена (`mock_external_return`) | M91, M91b | красный | оставлен | красный |
| 29 | `registration_rejects_replayed_bls_key_and_pop_without_partial_state` | 2315 | личность | `consensus.rs:123-130` и `:197-208` — BLS-ключ занят | мок PAIRING/предеплоев | C3, M91 | красный | оставлен | красный |
| 30 | `one_owner_cannot_register_a_second_validator` | 2404 | личность | `staking.rs:97-104` — владелец занят | мок PAIRING/предеплоев + дефолтный мок токена (`mock_external_return`) | M25 | теста не было | **новый тест** | красный |
| 31 | `registration_rejects_a_forged_proof_of_possession_without_partial_state` | 2469 | личность | `consensus.rs:146-148` — вердикт PoP | мок PAIRING/предеплоев | M61 | красный | оставлен | красный |
| 32 | `delegation_and_undelegation_follow_epoch_snapshots` | 2521 | деньги | `staking.rs:1141-1143` — WARMUP_DELAY; `:1286-1297` — очередь | `Harness` | M27 | красный | оставлен | красный |
| 33 | `leader_weights_are_frozen_at_the_selection_epoch_vintage` | 2607 | отбор | `consensus.rs:579`, `:626-630` — вес эпохи отбора | мок PAIRING/предеплоев | M27 | красный | оставлен | красный |
| 34 | `keys_activating_after_the_selection_epoch_are_filtered_before_the_cut` | 2669 | отбор | `consensus.rs:475` — `activation_epoch <= epoch` | мок PAIRING/предеплоев | M94, M95 | красный | оставлен | красный |
| 35 | `future_delegation_and_noop_commission_do_not_bypass_warmup` | 2744 | деньги | `staking.rs:1141-1143` — WARMUP_DELAY | мок PAIRING/предеплоев | M27 | красный | оставлен | красный |
| 36 | `commission_change_carries_forward_without_copying_future_stake_backward` | 2839 | деньги | `staking.rs:713-730` — `set_commission_from` | мок PAIRING/предеплоев | M27, M28 | красный | оставлен | красный |
| 37 | `sparse_snapshot_lookup_uses_sorted_materialized_epochs` | 2902 | прочее | `staking.rs:461-517` — сортированный индекс снапшотов | мок PAIRING/предеплоев | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 38 | `undelegation_rejects_a_later_pending_delegation_checkpoint` | 2941 | деньги | `staking.rs:1233-1235` — отложенная делегация | `Harness` | M27, M29 | красный | оставлен | красный |
| 39 | `delegation_into_a_tombstoned_validator_is_refused` | 3004 | личность | `staking.rs:1132-1138` — тумбстоун в `delegate_to` | `Harness` | M47 | красный | оставлен | красный |
| 40 | `a_tombstone_refuses_redelegation_without_stranding_the_claim` | 3067 | личность | `staking.rs:1132-1138` — тумбстоун на пути ределегации | `Harness` | M47, M48, M57 | красный | оставлен | красный |
| 41 | `undelegation_binds_the_minimum_to_the_remainder_not_the_withdrawal` | 3164 | деньги | `staking.rs:1278-1284` — минимум остатка | `Harness` | M30 | красный | оставлен | красный |
| 42 | `a_raised_delegation_minimum_does_not_govern_the_owner_self_stake` | 3249 | деньги | `staking.rs:1278` — исключение самостейка | `Harness` | M30, M31 | красный | оставлен | красный |
| 43 | `reward_views_split_blend_between_owner_and_delegators` | 3309 | деньги | `staking.rs:1347-1375` — `snapshot_payout` | мок PAIRING/предеплоев | M57, M60 | красный | оставлен | красный |
| 44 | `committee_commit_is_system_gated_and_returns_epoch_stakes` | 3356 | отбор | `consensus.rs:564-566` — системный вызов; `:610-630` — запись комитета | мок PAIRING/предеплоев | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 45 | `the_cap_setter_refuses_a_value_below_the_committee_floor` | 3434 | конфиг | `config.rs:384-390` — потолок ниже пола | мок PAIRING/предеплоев | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 46 | `a_committee_with_only_zero_weights_assigns_its_epoch_nothing` | 3475 | деньги | `staking.rs:2079`, `:2124`, `:2162`, `:2170`, `:2175` — пять нулевых веток | `install_stipend_token` | E1, M41 | красный | оставлен | красный |
| 47 | `an_epoch_with_a_committee_but_no_recorded_block_draws_no_pot` | 3513 | деньги | `staking.rs:2049-2053` — `recorded == 0` | `install_stipend_token` + `commit_test_committee` | M42 | теста не было | **новый тест** | красный |
| 48 | `an_eligible_set_one_short_of_the_floor_is_refused` | 3573 | отбор | `consensus.rs:581-587` — пол комитета | мок PAIRING/предеплоев | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 49 | `the_committee_seats_the_next_eligible_validator_instead_of_a_keyless_one` | 3616 | отбор | `consensus.rs:508-515` — фильтр до среза | мок PAIRING/предеплоев | M95 | красный | оставлен | красный |
| 50 | `a_refused_commit_writes_neither_committee_nor_cursor` | 3694 | отбор | `consensus.rs:581-587` + откат кадра | мок PAIRING/предеплоев + `Harness` | M95 | красный | оставлен | красный |
| 51 | `equal_stake_top_k_preserves_solidity_roster_order` | 3746 | отбор | `staking.rs:418-443` — `top_k_by_stake_at` | мок PAIRING/предеплоев | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 52 | `a_raised_minimum_does_not_empty_the_next_committee` | 3774 | отбор | `staking.rs:382-401` — отбор не читает минимум | мок PAIRING/предеплоев | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 53 | `the_genesis_commit_mints_a_record_although_nothing_changed` | 3833 | отбор | `consensus.rs:610` — `changed || target == 0` | мок PAIRING/предеплоев | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 54 | `a_short_successor_cannot_let_its_predecessor_read_the_wrong_weights` | 3894 | отбор | `consensus.rs:441-443` — штамп пары 0 | `commit_test_committee` | M43 | красный | оставлен | красный |
| 55 | `an_odd_member_count_leaves_no_stale_half_in_the_ring` | 3955 | отбор | `consensus.rs:403-405` — запись нуля во вторую половину | мок PAIRING/предеплоев | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 56 | `committee_changed_compares_positions_not_membership` | 4031 | отбор | `consensus.rs:329-331` — терм длины; `:339-343` — позиционное сравнение | `commit_test_committee` | M2 | зелёный при снятой проверке | **переписан** | красный |
| 57 | `a_peer_key_cannot_be_reassigned_so_the_sort_key_is_immutable` | 4082 | личность | `consensus.rs:112-119` и `:188-195` — peer-ключ занят | мок PAIRING/предеплоев | C4 | красный | оставлен | красный |
| 58 | `a_committee_stays_readable_far_past_the_retired_pruning_horizon` | 4147 | отбор | ОТСУТСТВИЕ прунинга комитетов | мок PAIRING/предеплоев | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 59 | `chain_config_guards_match_solidity_boundaries` | 4216 | конфиг | `config.rs` — границы сеттеров | мок PAIRING/предеплоев | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 60 | `dpos_activation_at_block_zero_remains_configurable` | 4301 | конфиг | `config.rs:25-30` — `ensure_dpos_not_active` | мок PAIRING/предеплоев | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 61 | `scheduling_activation_never_moves_the_epoch_backwards` | 4350 | конфиг | `math::epoch_at_block` — незанятая активация | мок PAIRING/предеплоев | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 62 | `undelegate_period_change_does_not_shorten_queued_principal` | 4403 | деньги | `staking.rs:1300-1305` — срок зафиксирован в очереди | `Harness` | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 63 | `lifecycle_transitions_preserve_next_epoch_snapshot_frontier` | 4441 | прочее | `staking.rs:962` — материализация E+1 при `disableValidator` | мок PAIRING/предеплоев | M93 | красный | оставлен | красный |
| 64 | `validator_owner_cannot_drop_below_minimum_while_delegators_remain` | 4501 | деньги | `staking.rs:1251-1267` — минимум самостейка | `Harness` | M27, M32 | красный | оставлен | красный |
| 65 | `sole_validator_owner_full_exit_deactivates_without_leaving_subminimum_dust` | 4545 | деньги | `staking.rs:1251-1267`, `:1315-1322` — полный выход | `Harness` | M32, M33 | красный | оставлен | красный |
| 66 | `stipend_pays_the_frozen_weights_not_the_stake_at_close_time` | 4626 | деньги | `staking.rs:2161` — замороженный вес | `install_stipend_token` | M39, M41 | красный | оставлен | красный |
| 67 | `closing_an_epoch_records_what_it_assigned` | 4677 | деньги | `liveness.rs:201` — нога начисления в `close_epoch` | дефолтный мок токена (`mock_external_return`) + `commit_test_committee` | M92 | красный | оставлен | красный |
| 68 | `the_accrued_total_is_exactly_the_sum_of_the_credits_it_wrote` | 4732 | деньги | `staking.rs:2188-2192` — сумма назначенного | дефолтный мок токена (`mock_external_return`) + `commit_test_committee` | M56 | красный | оставлен | красный |
| 69 | `re_accruing_an_epoch_rewrites_rather_than_doubles_it` | 4780 | деньги | `staking.rs:2184-2187` — присвоение, не накопление | `install_stipend_token` | M39, M44 | красный | оставлен | красный |
| 70 | `the_owner_and_delegator_claims_split_an_accrued_epoch_by_its_commission` | 4794 | деньги | `staking.rs:1347-1375` — деление по комиссии | `install_stipend_token` + `Harness` | M39, M48 | красный | оставлен | красный |
| 71 | `tombstoned_committee_member_earns_no_stipend_share` | 4850 | деньги | `staking.rs:2148-2154` — пропуск tombstoned-места | `install_stipend_token` | M39, M40 | красный | оставлен | красный |
| 72 | `a_permissionless_owner_claim_pays_the_owner_off_the_reserve` | 4892 | деньги | `staking.rs:1772-1774`, `:1893-1904` — получатель и источник | `install_stipend_token` + `Harness` | M39, M48, M49 | красный | оставлен | красный |
| 73 | `a_reward_and_a_matured_principal_claim_are_independent` | 4965 | деньги | `staking.rs:1599-1699` — два курсора и два источника | `install_stipend_token` | M35b, M48, M52, M53, M57 | красный | оставлен | красный |
| 74 | `claiming_rewards_does_not_rewrite_historical_self_stake` | 5124 | деньги | ОТСУТСТВИЕ перезаписи очереди в `consume_delegator_reward` | `Harness` | M103 | зелёный при снятой проверке | **переписан** | красный |
| 75 | `reward_claims_are_bounded_to_one_thousand_epochs` | 5193 | деньги | `staking.rs:1558-1564`, `:1758-1763` — окно MAX_EPOCHS_PER_CLAIM | `Harness` | M103, M50a, M50b | красный | оставлен | красный |
| 76 | `a_validator_registered_late_starts_its_reward_cursor_at_registration` | 5282 | деньги | `staking.rs:123` — курсор от `changed_at` | мок PAIRING/предеплоев | M51 | красный | оставлен | красный |
| 77 | `the_commit_orders_by_peer_key_and_membership_changes_mint_the_dkg_bit` | 5353 | отбор | `consensus.rs:596` — сортировка; `:631-634` — бит DKG | мок PAIRING/предеплоев | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 78 | `external_dependency_flows_fail_closed_before_calls` | 5466 | личность | `consensus.rs:106-111` — кодировка ключа; `evidence.rs` — форма улики | `Harness` | M64 | красный | оставлен | красный |
| 79 | `equivocation_seizes_active_and_pending_self_delegation` | 5511 | деньги | `consensus.rs:794-824` — состав конфискации | `Harness` | M54 | красный | оставлен | красный |
| 80 | `g2_compression_swaps_the_halves_and_reads_the_sign_from_c1` | 5823 | личность | `bls.rs:326-328` — перестановка половин и знак | — | M65 | красный | оставлен | красный |
| 81 | `the_y_sign_bit_is_strictly_above_half_the_field` | 5852 | личность | `bls.rs:225-227` — строгое сравнение с (p−1)/2 | — | M65, M66 | красный | оставлен | красный |
| 82 | `verify_refuses_an_infinity_point_on_every_side` | 5881 | личность | `bls.rs:214-219`, `:252-257` — три отказа на бесконечности | мок PAIRING/предеплоев | M68 | красный | оставлен | красный |
| 83 | `g1_compression_takes_the_sign_from_y_alone` | 5924 | личность | `bls.rs:300` — знак G1 | — | M66, M67 | красный | оставлен | красный |
| 84 | `compression_refuses_infinity_and_a_wrong_width` | 5948 | личность | `bls.rs:288-297`, `:313-322` — ширина и бесконечность | — | M69, M70 | красный | оставлен | красный |
| 85 | `a_namespace_that_would_outgrow_one_length_byte_is_refused` | 5974 | личность | `bls.rs:116-118` — однобайтовый префикс | мок PAIRING/предеплоев | M71 | красный | оставлен | красный |
| 86 | `a_dst_past_the_short_dst_limit_is_refused` | 5995 | личность | `bls.rs:169-171` — предел короткого DST | мок PAIRING/предеплоев | M72 | красный | оставлен | красный |
| 87 | `a_precompile_that_answers_wrongly_stops_the_registration` | 6035 | личность | `bls.rs:85-88` — статус и ширина ответа | мок PAIRING/предеплоев | M73 | красный | оставлен | красный |
| 88 | `a_refused_pairing_is_an_invalid_signature_and_not_a_broken_call` | 6086 | личность | `bls.rs:270-273` — отказ PAIRING = `false` | мок PAIRING/предеплоев | M61, M74 | красный | оставлен | красный |
| 89 | `each_slash_route_verifies_under_the_domain_its_kinds_name` | 6121 | личность (улики) | `consensus.rs:773-782`, `:968-983` — домены улик | мок PAIRING/предеплоев | M91 | красный | оставлен | красный |
| 90 | `a_blob_routed_through_the_wrong_entry_point_fails_verification` | 6171 | личность (улики) | то же, маршрут улик | мок PAIRING/предеплоев | M91 | красный | оставлен | красный |
| 91 | `equivocation_slash_tombstones_jails_and_seizes_the_self_stake_whole` | 6225 | личность (улики) | `consensus.rs:928-988` — маршрут улик | мок PAIRING/предеплоев | M80, M80b, M91 | красный | оставлен | красный |
| 92 | `a_seizure_stops_the_seized_bond_counting_as_stake` | 6353 | личность (улики) | `consensus.rs:816` — снятие веса (через маршрут улик) | мок PAIRING/предеплоев | M27, M91 | красный | оставлен | красный |
| 93 | `a_slash_survives_a_fund_that_refuses_the_seizure` | 6426 | личность (улики) | `consensus.rs:840-842` — `try_transfer` (через маршрут улик) | мок PAIRING/предеплоев | M91 | красный | оставлен | красный |
| 94 | `an_uncommitted_evidence_epoch_does_not_block_a_slash` | 6492 | личность (улики) | маршрут улик | мок PAIRING/предеплоев | M91 | красный | оставлен | красный |
| 95 | `the_index_slash_route_still_resolves_far_past_the_retired_pruning_horizon` | 6540 | личность | ОТСУТСТВИЕ прунинга комитетов на индексном маршруте | мок PAIRING/предеплоев | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 96 | `a_registered_but_never_activated_validator_can_be_slashed` | 6582 | личность (улики) | маршрут улик | мок PAIRING/предеплоев | M91 | красный | оставлен | красный |
| 97 | `a_nullify_finalize_conflict_is_slashable_through_its_own_entry_point` | 6659 | личность (улики) | маршрут улик | мок PAIRING/предеплоев | M91 | красный | оставлен | красный |
| 98 | `a_slash_with_nothing_to_seize_still_tombstones` | 6704 | личность (улики) | маршрут улик | мок PAIRING/предеплоев | M91 | красный | оставлен | красный |
| 99 | `a_slash_naming_an_unregistered_key_is_rejected` | 6802 | личность (улики) | маршрут улик | мок PAIRING/предеплоев | — | мутация не ставилась | оставлен | мутация не ставилась |
| 100 | `a_slash_whose_signatures_fail_verification_is_rejected` | 6825 | личность (улики) | маршрут улик | мок PAIRING/предеплоев | M91 | красный | оставлен | красный |
| 101 | `re_slashing_a_tombstoned_validator_is_refused` | 6851 | личность (улики) | маршрут улик | мок PAIRING/предеплоев | M91 | красный | оставлен | красный |
| 102 | `the_system_slash_entry_refuses_every_other_caller` | 6899 | личность | `consensus.rs:909-911` — системный вызов | `commit_test_committee` | M75 | красный | оставлен | красный |
| 103 | `a_system_verdict_naming_a_seat_the_committee_does_not_have_is_refused` | 6939 | личность | `consensus.rs:674-683` — `committee_member_at` | `commit_test_committee` | M76 | теста не было | **новый тест** | красный |
| 104 | `a_system_verdict_tombstones_jails_and_sends_the_whole_seizure_to_the_fund` | 6991 | личность | `consensus.rs:860-893` — тумбстоун, джейл, фонд | `commit_test_committee` + дефолтный мок токена (`mock_external_return`) | M80, M80b, M81 | красный | оставлен | красный |
| 105 | `the_committee_snapshot_reports_the_tombstone_against_its_own_member` | 7059 | личность | `consensus.rs:751-756` — позиционный флаг | `commit_test_committee` | M79 | красный | оставлен | красный |
| 106 | `a_second_system_verdict_against_the_same_validator_is_a_no_op` | 7113 | личность | `consensus.rs:918-924` — повтор вердикта | `commit_test_committee` + дефолтный мок токена (`mock_external_return`) | M77 | красный | оставлен | красный |
| 107 | `a_verdict_naming_a_validator_with_no_record_writes_no_tombstone` | 7157 | личность | `consensus.rs:865-874` — порядок чтения статуса | `commit_test_committee` | M78 | красный | оставлен | красный |
| 108 | `production_liveness_ships_disabled_on_a_fresh_chain` | 7193 | конфиг | `config.rs:73-76` — тир выключен на старте | мок PAIRING/предеплоев | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 109 | `production_liveness_setters_enforce_their_bounds` | 7235 | конфиг | `config.rs:565-624` — границы сеттеров | мок PAIRING/предеплоев | M84 | красный | оставлен | красный |
| 110 | `production_exclusion_refuses_at_the_committee_floor_and_leaves_no_trace` | 7338 | liveness | `staking.rs:293-295` — пол популяции | мок PAIRING/предеплоев | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 111 | `production_exclusion_bites_at_the_next_epoch_and_not_before` | 7369 | liveness | `staking.rs:288-290` — повтор штампа; `:296` — эпоха укуса | мок PAIRING/предеплоев | M18, M85, M87 | зелёный при снятой проверке | **переписан** | красный |
| 112 | `exclusion_release_skips_tombstoned_and_non_active_validators` | 7449 | liveness | `staking.rs:314-323` — охрана освобождения | мок PAIRING/предеплоев | M82, M85, M87 | красный | оставлен | красный |
| 113 | `governance_activation_does_not_cancel_a_running_exclusion` | 7526 | liveness | `staking.rs:943-945` — не перештамповывать под исключением | мок PAIRING/предеплоев | M83 | красный | оставлен | красный |
| 114 | `a_second_status_transition_does_not_rewrite_the_epoch_before_the_first` | 7609 | отбор | `staking.rs:214-242` — история из трёх переходов | мок PAIRING/предеплоев | M85, M86, M87 | красный | оставлен | красный |
| 115 | `production_liveness_views_read_the_new_namespace` | 7707 | ABI-вью | `liveness.rs:510-567` — devnet-вьюхи | мок PAIRING/предеплоев | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 116 | `record_production_belt_holds_and_the_epoch_cursor_precedes_the_overwrite` | 7876 | liveness | `liveness.rs:58-68` — пояс и порядок курсора | `commit_test_committee` | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 117 | `the_close_reports_the_epoch_that_ended_not_the_one_the_block_starts` | 7936 | liveness | `liveness.rs:73-75` — какая эпоха закрывается | `commit_test_committee` | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 118 | `an_uncommitted_committee_parks_the_block_instead_of_reverting` | 7987 | liveness | `liveness.rs:93-101` — парковка блока | мок PAIRING/предеплоев | M92 | красный | оставлен | красный |
| 119 | `a_partial_epoch_suppresses_judging_entirely` | 8063 | liveness | `liveness.rs:182-195` — неполная эпоха | `commit_test_committee` | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 120 | `the_recorder_takes_its_height_from_the_block_context` | 8102 | liveness | `liveness.rs:56-60` — высота из контекста | `commit_test_committee` | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 121 | `the_recorder_keeps_the_block_count_equal_to_the_sum_of_its_credits` | 8138 | liveness | `liveness.rs:103-121` — счётчики после обеих парковок | `commit_test_committee` | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 122 | `a_healthy_epoch_zero_is_complete_one_block_short_of_the_interval` | 8178 | liveness | `liveness.rs:171-175` — ожидание эпохи 0 | `commit_test_committee` | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 123 | `only_epoch_zero_gets_the_shortened_expectation` | 8209 | liveness | то же | `commit_test_committee` | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 124 | `an_epoch_zero_two_blocks_short_is_still_partial` | 8239 | liveness | то же | `commit_test_committee` | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 125 | `the_kill_switch_also_suppresses_verdicts` | 8274 | liveness | `liveness.rs:189-195` — рубильник | `commit_test_committee` | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 126 | `verdicts_come_from_the_frozen_weights_and_are_never_divided` | 8320 | liveness | `liveness.rs:308-359` — оба предиката | `commit_test_committee` | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 127 | `the_verdict_floor_is_a_stake_share_at_the_production_epoch_length` | 8375 | liveness | `liveness.rs:315-317` — пол вердикта | `commit_test_committee` | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 128 | `the_correlation_guard_keys_on_new_failures_and_frees_the_next_epoch` | 8457 | liveness | `liveness.rs:382-416` — гвардия корреляции | `commit_test_committee` | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 129 | `stamps_are_bounded_per_close_and_by_the_concurrent_budget` | 8510 | liveness | `liveness.rs:441-444` — два предела штампов | `commit_test_committee` | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 130 | `the_kill_switch_suspends_judging_but_never_releases` | 8575 | liveness | `liveness.rs:156`, `:189-195` — обе ноги | `commit_test_committee` | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 131 | `a_late_close_still_reads_its_own_epochs_weights` | 8735 | деньги | `consensus.rs:441-443` — штамп кольца | `Harness` | M92 | красный | оставлен | красный |
| 132 | `a_close_past_the_ring_forfeits_the_epoch_and_says_so` | 8785 | деньги | `consensus.rs:441-443`, `staking.rs:2132-2139` — промах кольца | `Harness` | M43, M92 | красный | оставлен | красный |
| 133 | `the_close_accrues_the_epoch_without_moving_any_money` | 8848 | деньги | `liveness.rs:196-201` — закрытие не двигает денег | `Harness` | M92 | красный | оставлен | красный |
| 134 | `an_unreadable_reserve_zeroes_the_epoch_without_failing_the_close` | 8901 | деньги | `util.rs:250-256` — отказ чтения ⇒ ноль | `Harness` | M36, M38, M92 | красный | оставлен | красный |
| 135 | `epochs_that_close_before_the_treasury_approves_burn_for_good` | 9013 | деньги | `util.rs:307` — терм allowance; `staking.rs:2119` — форфейт | `install_stipend_token` | M36, M37a, M39, M92 | красный | оставлен | красный |
| 136 | `an_approval_over_an_empty_reserve_covers_nothing` | 9075 | деньги | `util.rs:307` — терм баланса | `install_stipend_token` | M36, M37b, M39, M92 | красный | оставлен | красный |
| 137 | `the_close_asks_the_reserve_about_itself_and_not_about_the_contract` | 9134 | деньги | `staking.rs:2118` — адрес резерва | `install_stipend_token` | M39, M92 | красный | оставлен | красный |
| 138 | `the_delegator_views_report_the_reward_and_the_deposit_apart` | 9176 | деньги | `staking.rs:1416-1499` — две вьюхи, два курсора | `install_stipend_token` | M35b, M48, M52, M53, M57 | красный | оставлен | красный |
| 139 | `redelegation_takes_the_reward_and_leaves_the_withdrawal_queue_alone` | 9287 | деньги | `staking.rs:1848-1875` — ределегация только награды | `install_stipend_token` | M35b, M48, M52, M57, M59 | красный | оставлен | красный |
| 140 | `funding_the_reserve_after_the_close_does_not_revive_the_epoch` | 9418 | деньги | `staking.rs:2119` — форфейт окончателен | `Harness` | M36, M92 | красный | оставлен | красный |
| 141 | `a_tombstone_extinguishes_the_owner_commission_but_not_the_delegator_share` | 9559 | деньги | `staking.rs:1389-1395`, `:1748-1754` — гейт комиссии | `Harness` | M45, M46, M48, M49, M57, M60 | красный | оставлен | красный |
| 142 | `the_delegator_split_reproduces_the_seat_weight_frozen_two_epochs_back` | 9648 | деньги | `staking.rs:1451-1452`, `:1635-1636` — знаменатель E−2 | `Harness` | M27, M35b, M57 | красный | оставлен | красный |
| 143 | `a_seat_with_no_snapshot_at_its_selection_epoch_pays_its_whole_credit_to_the_owner` | 9703 | деньги | `staking.rs:1365-1367` — ветка нулевого знаменателя | мок PAIRING/предеплоев + `Harness` | M96 | теста не было | **новый тест** | красный |
| 144 | `a_clean_run_retires_the_kick_ladder_and_one_epoch_short_does_not` | 9784 | liveness | `liveness.rs:330-344` — сброс лестницы | `commit_test_committee` | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |
| 145 | `a_commission_rise_misses_two_epochs_while_a_cut_lands_on_the_next` | 9831 | деньги | `staking.rs:1369` — `min` двух ставок | `Harness` | M34, M35, M48, M60 | красный | оставлен | красный |
| 146 | `a_delegator_who_left_after_the_selection_still_earns_it_at_the_older_rate` | 9917 | деньги | `staking.rs:1451-1452` — знаменатель E−2 после выхода | `Harness` | M35, M35b, M48, M57, M60 | красный | оставлен | красный |
| 147 | `a_snapshot_materialized_after_its_epoch_passed_still_carries_the_old_rate` | 10039 | деньги | `staking.rs:601-603` — перенос ставки при материализации | `Harness` | M35, M60 | красный | оставлен | красный |
| 148 | `the_accrual_path_credits_an_epoch_that_is_still_divided_two_epochs_back` | 10110 | деньги | `staking.rs:2183`, `:1451-1452` — материализация и деление | `install_stipend_token` | M27, M35b, M39, M57, M60 | красный | оставлен | красный |
| 149 | `the_ladder_reset_also_lands_on_the_member_failing_that_same_epoch` | 10202 | liveness | `liveness.rs:330-344` — сброс на падающем участнике | `commit_test_committee` | — | не убит ни одной мутацией | оставлен | не убит ни одной мутацией |

---

## 2. Мутации: что снято, что упало

97 мутаций контракта в копии дерева A. `M1`, `M2`, `M18` — повтор выживших из
`E1-REFLECTION.md`; `M21`–`M103` — новые; `C1`–`C5` и `E1` — комбинированные, они
отличают «проверка избыточна» от «нет теста». Каждая — снятие ровно одной проверки
(`if false &&`, замена на константу, снятие `min`, снятие насыщения) или, там где тест
охраняет ОТСУТСТВИЕ механизма, возврат этого механизма (`M52`, `M78`, `M89`, `M103`).
Колонка «после правки» заполнена только для мутаций, перепрогнанных после правок тестов.

| M | что снято | до правки тестов | упавшие тесты | после правки |
|---|---|---|---|---|
| M1 | consensus.rs selected_committee_at: drop the STATUS_ACTIVE term | ЗЕЛЁНАЯ | — | ЗЕЛЁНАЯ (перепроверено) |
| M2 | consensus.rs committee_changed: drop the length term | ЗЕЛЁНАЯ | — | КРАСНАЯ: `committee_changed_compares_positions_not_membership` |
| M18 | staking.rs apply_production_exclusion: drop the already-invisible refusal | ЗЕЛЁНАЯ | — | КРАСНАЯ: `production_exclusion_bites_at_the_next_epoch_and_not_before` |
| M21 | staking.rs register_validator: drop the minimum-bond gate | КРАСНАЯ (1) | `register_validator_rejects_a_subminimum_bond` | — |
| M22 | staking.rs activate_validator: drop the zero-self-stake gate | КРАСНАЯ (1) | `an_owner_who_withdrew_his_whole_bond_cannot_be_activated` | — |
| M23 | initializer.rs validate: drop the commission-rate ceiling | ЗЕЛЁНАЯ | — | ЗЕЛЁНАЯ (перепроверено) |
| M23b | staking.rs register_validator: drop the commission-rate ceiling | ЗЕЛЁНАЯ | — | ЗЕЛЁНАЯ (перепроверено) |
| M23c | staking.rs set_validator: drop the commission-rate ceiling | ЗЕЛЁНАЯ | — | ЗЕЛЁНАЯ (перепроверено) |
| M24 | staking.rs set_validator: drop the already-registered gate | ЗЕЛЁНАЯ | — | ЗЕЛЁНАЯ (перепроверено) |
| M25 | staking.rs set_validator: drop the owner-already-in-use gate | ЗЕЛЁНАЯ | — | КРАСНАЯ: `one_owner_cannot_register_a_second_validator` |
| M26 | initializer.rs: pull the genesis stakes from the CALLER, not the declared sponsor | КРАСНАЯ (1) | `initializer_pulls_genesis_stake_from_declared_sponsor` | — |
| M27 | staking.rs delegate_to: drop WARMUP_DELAY (book new stake at the current epoch) | КРАСНАЯ (9) | `a_seizure_stops_the_seized_bond_counting_as_stake`, `commission_change_carries_forward_without_copying_future_stake_backward`, `delegation_and_undelegation_follow_epoch_snapshots`, `future_delegation_and_noop_commission_do_not_bypass_warmup`, `leader_weights_are_frozen_at_the_selection_epoch_vintage`, `the_accrual_path_credits_an_epoch_that_is_still_divided_two_epochs_back`, `the_delegator_split_reproduces_the_seat_weight_frozen_two_epochs_back`, `undelegation_rejects_a_later_pending_delegation_checkpoint`, `validator_owner_cannot_drop_below_minimum_while_delegators_remain` | — |
| M28 | staking.rs set_commission_from: do not carry the rate into later snapshots | КРАСНАЯ (1) | `commission_change_carries_forward_without_copying_future_stake_backward` | — |
| M29 | staking.rs undelegate_from: drop the pending-delegation refusal | КРАСНАЯ (1) | `undelegation_rejects_a_later_pending_delegation_checkpoint` | — |
| M30 | staking.rs undelegate_from: drop the remainder minimum | КРАСНАЯ (2) | `a_raised_delegation_minimum_does_not_govern_the_owner_self_stake`, `undelegation_binds_the_minimum_to_the_remainder_not_the_withdrawal` | — |
| M31 | staking.rs undelegate_from: hold the owner's self-stake to the DELEGATION minimum too | КРАСНАЯ (1) | `a_raised_delegation_minimum_does_not_govern_the_owner_self_stake` | — |
| M32 | staking.rs undelegate_from: drop the owner min-validator-stake gate | КРАСНАЯ (2) | `sole_validator_owner_full_exit_deactivates_without_leaving_subminimum_dust`, `validator_owner_cannot_drop_below_minimum_while_delegators_remain` | — |
| M33 | staking.rs undelegate_from: drop the full-owner-exit deactivation | КРАСНАЯ (1) | `sole_validator_owner_full_exit_deactivates_without_leaving_subminimum_dust` | — |
| M34 | staking.rs snapshot_payout: drop the min() over the two commission vintages | КРАСНАЯ (1) | `a_commission_rise_misses_two_epochs_while_a_cut_lands_on_the_next` | — |
| M35 | staking.rs snapshot_payout: split the credit on the CLOSE epoch, not the selection epoch | КРАСНАЯ (3) | `a_commission_rise_misses_two_epochs_while_a_cut_lands_on_the_next`, `a_delegator_who_left_after_the_selection_still_earns_it_at_the_older_rate`, `a_snapshot_materialized_after_its_epoch_passed_still_carries_the_old_rate` | — |
| M35b | staking.rs claim walks: divide by the CLOSE-epoch total, not the selection vintage | КРАСНАЯ (6) | `a_delegator_who_left_after_the_selection_still_earns_it_at_the_older_rate`, `a_reward_and_a_matured_principal_claim_are_independent`, `redelegation_takes_the_reward_and_leaves_the_withdrawal_queue_alone`, `the_accrual_path_credits_an_epoch_that_is_still_divided_two_epochs_back`, `the_delegator_split_reproduces_the_seat_weight_frozen_two_epochs_back`, `the_delegator_views_report_the_reward_and_the_deposit_apart` | — |
| M36 | staking.rs assign_epoch_shares: drop the reserve-covers-the-pot gate (the forfeit) | КРАСНАЯ (4) | `an_approval_over_an_empty_reserve_covers_nothing`, `an_unreadable_reserve_zeroes_the_epoch_without_failing_the_close`, `epochs_that_close_before_the_treasury_approves_burn_for_good`, `funding_the_reserve_after_the_close_does_not_revive_the_epoch` | — |
| M37a | util.rs reserve_available: drop the allowance term (balance alone) | КРАСНАЯ (1) | `epochs_that_close_before_the_treasury_approves_burn_for_good` | — |
| M37b | util.rs reserve_available: drop the balance term (allowance alone) | КРАСНАЯ (1) | `an_approval_over_an_empty_reserve_covers_nothing` | — |
| M38 | util.rs erc20_scalar_read: score a failed/undecodable read as U256::MAX, not zero | КРАСНАЯ (1) | `an_unreadable_reserve_zeroes_the_epoch_without_failing_the_close` | — |
| M39 | staking.rs assign_epoch_shares: ask the token about THIS CONTRACT, not the reserve | КРАСНАЯ (9) | `a_permissionless_owner_claim_pays_the_owner_off_the_reserve`, `an_approval_over_an_empty_reserve_covers_nothing`, `epochs_that_close_before_the_treasury_approves_burn_for_good`, `re_accruing_an_epoch_rewrites_rather_than_doubles_it`, `stipend_pays_the_frozen_weights_not_the_stake_at_close_time`, `the_accrual_path_credits_an_epoch_that_is_still_divided_two_epochs_back`, `the_close_asks_the_reserve_about_itself_and_not_about_the_contract`, `the_owner_and_delegator_claims_split_an_accrued_epoch_by_its_commission`, `tombstoned_committee_member_earns_no_stipend_share` | — |
| M40 | staking.rs assign_epoch_shares: drop the tombstoned-seat skip | КРАСНАЯ (1) | `tombstoned_committee_member_earns_no_stipend_share` | — |
| M41 | staking.rs assign_epoch_shares: split on the LIVE stake, not the frozen weight | КРАСНАЯ (2) | `a_committee_with_only_zero_weights_assigns_its_epoch_nothing`, `stipend_pays_the_frozen_weights_not_the_stake_at_close_time` | — |
| M42 | staking.rs accrue_epoch: drop the never-recorded-epoch arm | ЗЕЛЁНАЯ | — | КРАСНАЯ: `an_epoch_with_a_committee_but_no_recorded_block_draws_no_pot` |
| M43 | consensus.rs read_weights: drop the ring stamp check | КРАСНАЯ (2) | `a_close_past_the_ring_forfeits_the_epoch_and_says_so`, `a_short_successor_cannot_let_its_predecessor_read_the_wrong_weights` | — |
| M44 | staking.rs assign_epoch_shares: accumulate the credit instead of assigning it | КРАСНАЯ (1) | `re_accruing_an_epoch_rewrites_rather_than_doubles_it` | — |
| M45 | staking.rs claim_validator_before: drop the tombstone refusal on the OWNER claim | КРАСНАЯ (1) | `a_tombstone_extinguishes_the_owner_commission_but_not_the_delegator_share` | — |
| M46 | staking.rs validator_owner_rewards: drop the tombstone zero in the VIEW | КРАСНАЯ (1) | `a_tombstone_extinguishes_the_owner_commission_but_not_the_delegator_share` | — |
| M47 | staking.rs delegate_to: drop the tombstone refusal on new stake | КРАСНАЯ (2) | `a_tombstone_refuses_redelegation_without_stranding_the_claim`, `delegation_into_a_tombstoned_validator_is_refused` | — |
| M48 | staking.rs pay_stipend: pay out of THIS CONTRACT's balance instead of the reserve | КРАСНАЯ (9) | `a_commission_rise_misses_two_epochs_while_a_cut_lands_on_the_next`, `a_delegator_who_left_after_the_selection_still_earns_it_at_the_older_rate`, `a_permissionless_owner_claim_pays_the_owner_off_the_reserve`, `a_reward_and_a_matured_principal_claim_are_independent`, `a_tombstone_extinguishes_the_owner_commission_but_not_the_delegator_share`, `a_tombstone_refuses_redelegation_without_stranding_the_claim`, `redelegation_takes_the_reward_and_leaves_the_withdrawal_queue_alone`, `the_delegator_views_report_the_reward_and_the_deposit_apart`, `the_owner_and_delegator_claims_split_an_accrued_epoch_by_its_commission` | — |
| M49 | staking.rs claim_validator_before: pay the CALLER instead of the owner | КРАСНАЯ (2) | `a_permissionless_owner_claim_pays_the_owner_off_the_reserve`, `a_tombstone_extinguishes_the_owner_commission_but_not_the_delegator_share` | — |
| M50a | staking.rs claim_validator_before: drop the MAX_EPOCHS_PER_CLAIM window | КРАСНАЯ (1) | `reward_claims_are_bounded_to_one_thousand_epochs` | — |
| M50b | staking.rs capped_delegator_reward_epoch: drop the MAX_EPOCHS_PER_CLAIM window | КРАСНАЯ (1) | `reward_claims_are_bounded_to_one_thousand_epochs` | — |
| M51 | staking.rs set_validator: start the owner reward cursor at zero, not at registration | КРАСНАЯ (1) | `a_validator_registered_late_starts_its_reward_cursor_at_registration` | — |
| M52 | staking.rs claim_delegator_reward_before: fold the matured principal back into the reward claim | КРАСНАЯ (3) | `a_reward_and_a_matured_principal_claim_are_independent`, `redelegation_takes_the_reward_and_leaves_the_withdrawal_queue_alone`, `the_delegator_views_report_the_reward_and_the_deposit_apart` | — |
| M53 | staking.rs withdraw_delegator_principal_before: pay the deposit off the RESERVE | КРАСНАЯ (2) | `a_reward_and_a_matured_principal_claim_are_independent`, `the_delegator_views_report_the_reward_and_the_deposit_apart` | — |
| M54 | consensus.rs seize_self_stake: leave the pending undelegation out of the seizure | КРАСНАЯ (1) | `equivocation_seizes_active_and_pending_self_delegation` | — |
| M56 | staking.rs assign_epoch_shares: announce the POT instead of the sum of the credits | КРАСНАЯ (1) | `the_accrued_total_is_exactly_the_sum_of_the_credits_it_wrote` | — |
| M57 | staking.rs delegate_claim_start: drop the max(cursor, first-entry) floor | КРАСНАЯ (10) | `a_delegator_who_left_after_the_selection_still_earns_it_at_the_older_rate`, `a_reward_and_a_matured_principal_claim_are_independent`, `a_tombstone_extinguishes_the_owner_commission_but_not_the_delegator_share`, `a_tombstone_refuses_redelegation_without_stranding_the_claim`, `an_owner_who_withdrew_his_whole_bond_cannot_be_activated`, `redelegation_takes_the_reward_and_leaves_the_withdrawal_queue_alone`, `reward_views_split_blend_between_owner_and_delegators`, `the_accrual_path_credits_an_epoch_that_is_still_divided_two_epochs_back`, `the_delegator_split_reproduces_the_seat_weight_frozen_two_epochs_back`, `the_delegator_views_report_the_reward_and_the_deposit_apart` | — |
| M59 | staking.rs available_for_redelegate: drop the compact-precision truncation | КРАСНАЯ (1) | `redelegation_takes_the_reward_and_leaves_the_withdrawal_queue_alone` | — |
| M60 | staking.rs touch_snapshot_at_or_before: do not carry the commission rate forward | КРАСНАЯ (6) | `a_commission_rise_misses_two_epochs_while_a_cut_lands_on_the_next`, `a_delegator_who_left_after_the_selection_still_earns_it_at_the_older_rate`, `a_snapshot_materialized_after_its_epoch_passed_still_carries_the_old_rate`, `a_tombstone_extinguishes_the_owner_commission_but_not_the_delegator_share`, `reward_views_split_blend_between_owner_and_delegators`, `the_accrual_path_credits_an_epoch_that_is_still_divided_two_epochs_back` | — |
| M61 | consensus.rs verify_consensus_keys: drop the proof-of-possession verdict | КРАСНАЯ (2) | `a_refused_pairing_is_an_invalid_signature_and_not_a_broken_call`, `registration_rejects_a_forged_proof_of_possession_without_partial_state` | — |
| M62 | consensus.rs verify_consensus_keys: drop the BLS-key-already-in-use gate | ЗЕЛЁНАЯ | — | ЗЕЛЁНАЯ (перепроверено) |
| M63 | consensus.rs verify_consensus_keys: drop the peer-key-already-in-use gate | ЗЕЛЁНАЯ | — | ЗЕЛЁНАЯ (перепроверено) |
| M64 | consensus.rs verify_consensus_keys: drop the key-encoding width/zero gate | КРАСНАЯ (2) | `external_dependency_flows_fail_closed_before_calls`, `solidity_bytes_calldata_reaches_staking_handlers` | — |
| M65 | bls.rs compress_g2_unchecked: do NOT swap the halves (EIP-2537 order kept) | КРАСНАЯ (2) | `g2_compression_swaps_the_halves_and_reads_the_sign_from_c1`, `the_y_sign_bit_is_strictly_above_half_the_field` | — |
| M66 | bls.rs fp_greater_half: make the y-sign compare non-strict (>= half) | КРАСНАЯ (2) | `g1_compression_takes_the_sign_from_y_alone`, `the_y_sign_bit_is_strictly_above_half_the_field` | — |
| M67 | bls.rs compress_g1_unchecked: drop the y-sign bit | КРАСНАЯ (1) | `g1_compression_takes_the_sign_from_y_alone` | — |
| M68 | bls.rs reject_infinity: accept the point at infinity | КРАСНАЯ (1) | `verify_refuses_an_infinity_point_on_every_side` | — |
| M69 | bls.rs compress_g1/g2: drop the infinity refusal on compression | КРАСНАЯ (1) | `compression_refuses_infinity_and_a_wrong_width` | — |
| M70 | bls.rs compress_g1/g2: drop the exact-width refusal | КРАСНАЯ (1) | `compression_refuses_infinity_and_a_wrong_width` | — |
| M71 | bls.rs union_unique: drop the one-byte namespace-length guard | КРАСНАЯ (1) | `a_namespace_that_would_outgrow_one_length_byte_is_refused` | — |
| M72 | bls.rs hash_to_g1: drop the short-DST limit | КРАСНАЯ (1) | `a_dst_past_the_short_dst_limit_is_refused` | — |
| M73 | bls.rs call_precompile: accept any status and any output width | КРАСНАЯ (1) | `a_precompile_that_answers_wrongly_stops_the_registration` | — |
| M74 | bls.rs verify: read a failed/short PAIRING answer as a VALID signature | КРАСНАЯ (1) | `a_refused_pairing_is_an_invalid_signature_and_not_a_broken_call` | — |
| M75 | consensus.rs slash_equivocation: drop the SYSTEM_CALLER gate | КРАСНАЯ (1) | `the_system_slash_entry_refuses_every_other_caller` | — |
| M76 | consensus.rs committee_member_at: drop the not-committed and index-range refusals | ЗЕЛЁНАЯ | — | КРАСНАЯ: `a_system_verdict_naming_a_seat_the_committee_does_not_have_is_refused` |
| M77 | consensus.rs slash_equivocation: drop the already-tombstoned no-op | КРАСНАЯ (1) | `a_second_system_verdict_against_the_same_validator_is_a_no_op` | — |
| M78 | consensus.rs apply_equivocation_penalty: write the tombstone BEFORE the record check | КРАСНАЯ (1) | `a_verdict_naming_a_validator_with_no_record_writes_no_tombstone` | — |
| M79 | consensus.rs get_epoch_committee_with_stakes: report every member as untombstoned | КРАСНАЯ (1) | `the_committee_snapshot_reports_the_tombstone_against_its_own_member` | — |
| M80 | consensus.rs apply_equivocation_penalty: leave the offender in the active set | КРАСНАЯ (2) | `a_system_verdict_tombstones_jails_and_sends_the_whole_seizure_to_the_fund`, `equivocation_slash_tombstones_jails_and_seizes_the_self_stake_whole` | — |
| M80b | consensus.rs apply_equivocation_penalty: drop the selection-invisible stamp | КРАСНАЯ (2) | `a_system_verdict_tombstones_jails_and_sends_the_whole_seizure_to_the_fund`, `equivocation_slash_tombstones_jails_and_seizes_the_self_stake_whole` | — |
| M81 | consensus.rs seize_self_stake: always burn, ignoring the configured slash fund | КРАСНАЯ (1) | `a_system_verdict_tombstones_jails_and_sends_the_whole_seizure_to_the_fund` | — |
| M82 | staking.rs release_production_exclusion: drop the tombstone and non-Active guards | КРАСНАЯ (1) | `exclusion_release_skips_tombstoned_and_non_active_validators` | — |
| M83 | staking.rs activate_validator: re-stamp visibility even under a running exclusion | КРАСНАЯ (1) | `governance_activation_does_not_cancel_a_running_exclusion` | — |
| M84 | util.rs ensure_governance: accept any caller | КРАСНАЯ (3) | `governance_lifecycle_updates_active_registry`, `governance_updates_embedded_chain_configuration`, `production_liveness_setters_enforce_their_bounds` | — |
| M85 | staking.rs set_selection_visible: drop the three-transition history shift | КРАСНАЯ (3) | `a_second_status_transition_does_not_rewrite_the_epoch_before_the_first`, `exclusion_release_skips_tombstoned_and_non_active_validators`, `production_exclusion_bites_at_the_next_epoch_and_not_before` | — |
| M86 | staking.rs set_selection_visible: drop the same-epoch re-stamp collapse | КРАСНАЯ (1) | `a_second_status_transition_does_not_rewrite_the_epoch_before_the_first` | — |
| M87 | staking.rs selection_visible_at: answer the live flag at every depth | КРАСНАЯ (3) | `a_second_status_transition_does_not_rewrite_the_epoch_before_the_first`, `exclusion_release_skips_tombstoned_and_non_active_validators`, `production_exclusion_bites_at_the_next_epoch_and_not_before` | — |
| C1 | ALL THREE commission-rate ceilings off (initializer + register + set_validator) | КРАСНАЯ (1) | `initialize_and_registration_reject_bad_commission_and_duplicate_validator` | КРАСНАЯ: `initialize_and_registration_reject_bad_commission_and_duplicate_validator` |
| C2 | BOTH already-registered gates off (set_validator + register_validator) | КРАСНАЯ (1) | `initialize_and_registration_reject_bad_commission_and_duplicate_validator` | КРАСНАЯ: `initialize_and_registration_reject_bad_commission_and_duplicate_validator` |
| C3 | BOTH BLS-key-already-in-use gates off (verify_consensus_keys + store_consensus_keys) | КРАСНАЯ (1) | `registration_rejects_replayed_bls_key_and_pop_without_partial_state` | КРАСНАЯ: `registration_rejects_replayed_bls_key_and_pop_without_partial_state` |
| C4 | BOTH peer-key-already-in-use gates off (verify_consensus_keys + store_consensus_keys) | КРАСНАЯ (1) | `a_peer_key_cannot_be_reassigned_so_the_sort_key_is_immutable` | КРАСНАЯ: `a_peer_key_cannot_be_reassigned_so_the_sort_key_is_immutable` |
| C5 | BOTH consensus-key writes off: store_consensus_keys' own already-set gate | ЗЕЛЁНАЯ | — | ЗЕЛЁНАЯ (перепроверено) |
| M88 | staking.rs register_validator: pull the bond from the VALIDATOR address, not the caller | КРАСНАЯ (1) | `register_validator_cast_calldata_registers_consensus_keys_atomically` | — |
| M89 | staking.rs activate_validator: hold the self-stake to the CURRENT minimum, not to zero | КРАСНАЯ (1) | `a_raised_validator_minimum_does_not_block_activating_an_earlier_registrant` | — |
| M90 | initializer.rs validate: drop the genesis minimum-stake gate | КРАСНАЯ (1) | `initializer_rejects_subminimum_active_validator` | — |
| M91 | consensus.rs store_consensus_keys: drop the bls_pubkey_owner write (key -> owner binding) | КРАСНАЯ (13) | `a_blob_routed_through_the_wrong_entry_point_fails_verification`, `a_nullify_finalize_conflict_is_slashable_through_its_own_entry_point`, `a_registered_but_never_activated_validator_can_be_slashed`, `a_seizure_stops_the_seized_bond_counting_as_stake`, `a_slash_survives_a_fund_that_refuses_the_seizure`, `a_slash_whose_signatures_fail_verification_is_rejected`, `a_slash_with_nothing_to_seize_still_tombstones`, `an_uncommitted_evidence_epoch_does_not_block_a_slash`, `each_slash_route_verifies_under_the_domain_its_kinds_name`, `equivocation_slash_tombstones_jails_and_seizes_the_self_stake_whole`, `re_slashing_a_tombstoned_validator_is_refused`, `register_validator_verifies_and_stores_consensus_keys_in_one_call`, `registration_rejects_replayed_bls_key_and_pop_without_partial_state` | — |
| M91b | consensus.rs store_consensus_keys: drop the activation_epoch write | КРАСНАЯ (2) | `register_validator_cast_calldata_registers_consensus_keys_atomically`, `register_validator_verifies_and_stores_consensus_keys_in_one_call` | — |
| M92 | liveness.rs close_epoch: drop the accrual leg entirely | КРАСНАЯ (10) | `a_close_past_the_ring_forfeits_the_epoch_and_says_so`, `a_late_close_still_reads_its_own_epochs_weights`, `an_approval_over_an_empty_reserve_covers_nothing`, `an_uncommitted_committee_parks_the_block_instead_of_reverting`, `an_unreadable_reserve_zeroes_the_epoch_without_failing_the_close`, `closing_an_epoch_records_what_it_assigned`, `epochs_that_close_before_the_treasury_approves_burn_for_good`, `funding_the_reserve_after_the_close_does_not_revive_the_epoch`, `the_close_accrues_the_epoch_without_moving_any_money`, `the_close_asks_the_reserve_about_itself_and_not_about_the_contract` | — |
| M93 | staking.rs disable_validator: drop the next-epoch snapshot materialization | КРАСНАЯ (1) | `lifecycle_transitions_preserve_next_epoch_snapshot_frontier` | — |
| M94 | consensus.rs active_peer_key_at: drop the key-activation-epoch gate | КРАСНАЯ (1) | `keys_activating_after_the_selection_epoch_are_filtered_before_the_cut` | — |
| M95 | consensus.rs selected_committee_at: drop the consensus-key filter | КРАСНАЯ (3) | `a_refused_commit_writes_neither_committee_nor_cursor`, `keys_activating_after_the_selection_epoch_are_filtered_before_the_cut`, `the_committee_seats_the_next_eligible_validator_instead_of_a_keyless_one` | — |
| M96 | staking.rs snapshot_payout: drop the zero-selection-total arm (pay the owner the lot) | ЗЕЛЁНАЯ | — | КРАСНАЯ: `a_seat_with_no_snapshot_at_its_selection_epoch_pays_its_whole_credit_to_the_owner` |
| M97 | staking.rs assign_epoch_shares: drop the zero-pot arm | ЗЕЛЁНАЯ | — | ЗЕЛЁНАЯ (перепроверено) |
| M98 | staking.rs assign_epoch_shares: drop the empty-committee arm | ЗЕЛЁНАЯ | — | ЗЕЛЁНАЯ (перепроверено) |
| M99 | staking.rs assign_epoch_shares: drop the zero-total-weight arm | ЗЕЛЁНАЯ | — | ЗЕЛЁНАЯ (перепроверено) |
| M100 | staking.rs assign_epoch_shares: drop the zero-frozen-weight skip | ЗЕЛЁНАЯ | — | ЗЕЛЁНАЯ (перепроверено) |
| E1 | assign_epoch_shares: ALL FIVE zero-return arms off at once | КРАСНАЯ (1) | `a_committee_with_only_zero_weights_assigns_its_epoch_nothing` | — |
| M101 | util.rs ensure_non_payable: accept a payable call | ЗЕЛЁНАЯ | — | КРАСНАЯ: `handlers_refuse_value_and_refuse_to_mutate_inside_a_static_frame` |
| M102 | util.rs ensure_mutable: accept a mutation inside a static frame | ЗЕЛЁНАЯ | — | КРАСНАЯ: `handlers_refuse_value_and_refuse_to_mutate_inside_a_static_frame` |
| M103 | staking.rs consume_delegator_reward: re-stamp the opening queue entry at the claim epoch (the pre-K-8 design) | КРАСНАЯ (1) | `reward_claims_are_bounded_to_one_thousand_epochs` — но НЕ `claiming_rewards_does_not_rewrite_historical_self_stake`, ради которого она ставилась | КРАСНАЯ: `claiming_rewards_does_not_rewrite_historical_self_stake`, `reward_claims_are_bounded_to_one_thousand_epochs` |

**Что показали комбинированные мутации.** Девять одиночных мутаций пережили сюиту не
потому, что тестов нет, а потому, что проверка дублирована. `C1` (все три потолка
комиссии сразу), `C2` (оба гейта дубля валидатора), `C3`/`C4` (оба гейта занятого
BLS/peer-ключа) и `E1` (все пять нулевых веток `assign_epoch_shares` сразу) — все
КРАСНЫЕ. Значит свойство запинено, а вот КАКАЯ из копий его держит — не решено ничем, и
удаление любой одной копии проходит молча.

`C5` — единственная мутация, оставшаяся зелёной и не покрытая ни одиночным тестом, ни
комбинацией: гейт `ERR_CONSENSUS_KEYS_ALREADY_SET` (`consensus.rs:177-179`) недостижим,
пока стоят гейты `C2`, и док-комментарий над ним это прямо признаёт
(`consensus.rs:180-187`). Оставлен.

---

## 3. Заглушки: чего они не умеют отказать

### `install_stipend_token` (`tests.rs:652-737`)

Умеет отказать: `transferFrom` при `from != source`, при нехватке баланса и при
нехватке allowance; `balanceOf`/`allowance` отвечают ПРО АДРЕС, а не всем подряд;
`reports_failure` даёт и `false`, и отказ чтения. Это честная заглушка для ветки
резерва, и мутации `M36`, `M37a`, `M37b`, `M38`, `M39`, `M44` её убивают.

Не умеет отказать: обычный `transfer` — то есть трату СОБСТВЕННОГО баланса контракта —
всегда успешен, сколько бы контракт на самом деле ни держал. У заглушки нет
контрактной стороны книги: `transferFrom` в пользу `GENESIS_STAKING` ничего не
зачисляет, `transfer` ничего не списывает. Держит зелёными по мягкости:
`a_reward_and_a_matured_principal_claim_are_independent` (шаг 3 — вывод принципала при
пустом резерве) и `the_delegator_views_report_the_reward_and_the_deposit_apart` — оба
утверждают, что депозит выходит из баланса контракта, не проверив, что он там есть.
Переводится на честную заглушку без переписывания хоста: да — достаточно вести
`contract_balance` в том же `StipendFunding`. Не сделано: это правка фикстуры вне
правила приёмки Д-12; записано находкой F-3. На rWasm эта мягкость снята — в
`e2e/src/staking_reserve.rs` баланс контракта ведёт настоящий `universal-token`.

### Мок PAIRING как ПОЛИТИКА (`tests.rs:577-590`)

Не умеет отказать: он вообще ничего не решает про BLS12-381 — он принимает или
отвергает по namespace, вынутому из преобраза `expand_message_xmd`. Все негативные
ветки достижимы (`M61`, `M74` красные), но криптографического утверждения он не несёт.

Держит зелёными по мягкости — и это ПРОВЕРЕНО мутацией, а не выведено:
`M65` (перестановка половин G2 отменена) убивает только два теста,
`g2_compression_swaps_the_halves_and_reads_the_sign_from_c1` и
`the_y_sign_bit_is_strictly_above_half_the_field`. Три теста, которые сверяют
сохранённый сжатый ключ с ожидаемым —
`register_validator_cast_calldata_registers_consensus_keys_atomically`,
`register_validator_verifies_and_stores_consensus_keys_in_one_call`,
`get_consensus_keys_matches_dynamic_struct_return_vectors` — остаются ЗЕЛЁНЫМИ, потому
что их фикстура заполняет весь 256-байтовый ключ одним байтом `0x11`: обе половины `x`
совпадают, и перестановка не меняет ни байта. Переводится без хоста: да — разные
байты в половинах фикстуры. Не сделано (правка трёх фикстур вне правила приёмки);
записано находкой F-1.

### Заглушки MODEXP / MAP_FP_TO_G1 / G1ADD (`tests.rs:523-536`, `:592`)

Не умеют отказать: не выражают «не на кривой», «не в подгруппе», «грязный паддинг
EIP-2537», «неканоническая координата» — то есть ровно то, за что настоящий PAIRING
отвергает точку. Держат зелёным: ничего. Негативные ветки покрыты переключателями
`truncate`/`refuse`/`zero`, и `M69`, `M70`, `M73` красные. Непереводимо в юнит-харнессе
в принципе; криптографическое утверждение живёт в `e2e/src/staking.rs`,
`e2e/src/staking_bls.rs` на настоящих предеплоях.

### `static_call` → тот же обработчик, что и `call`, с гейтом `static_depth` (`crates/testing/src/host.rs`)

Что сделано в этой работе: гейт распространён на `emit_log`. Это третий эффект, который
EIP-214 запрещает в статическом кадре, и — в отличие от двух записей — единственный, до
которого может дотянуться САМ КОНТРАКТ: чтение резерва (`util.rs`, `erc20_scalar_read`)
и пять чтений предеплоев (`bls.rs`, `call_precompile` и `verify`) идут внутри
`static_call`, и событие, добавленное на любой из этих путей, ревертило бы на настоящей
цепи и молча проходило бы здесь. Конкретный класс дефекта, названный в реестре;
на себя есть три теста (`host.rs`, `a_log_emitted_inside_a_static_call_fails_the_test`,
`the_same_log_is_allowed_through_a_plain_call`,
`the_log_gate_closes_when_the_static_call_returns`). Ни один из 174 тестов контракта от
него не покраснел — то есть сегодня контракт логов внутри статических кадров не пишет,
и гейт работает на будущее, а не чинит наличный зелёный.

Чего гейт по-прежнему не умеет: увидеть мок, который мутирует только собственное
захваченное Rust-состояние. Такие моки в файле ЕСТЬ и работают внутри статических
кадров прямо сейчас: `mock_precompile_reply` для SHA-256 пишет `current_namespace`, а
для PAIRING — `seen.borrow_mut().push(...)`, и оба вызываются из `bls.rs` через
`static_call`. Это бухгалтерия харнесса, а не состояние, видимое контракту, так что
здесь она безвредна — но это и есть класс, который гейт закрыть не может и не сможет.
Вложенный CALL обратно в контракт он тоже не различает.

### `Harness` (`tests.rs:146-262`)

Не умеет: откатывать ЛОГИ на реверте (откатывается только хранилище); не выставляет
`contract_value` и `contract_is_static`. Второе держало зелёными `ensure_non_payable` и
`ensure_mutable` — оба открывают КАЖДЫЙ обработчик крейта, и до этой работы их снятие
(`M101`, `M102`) не роняло ни одного из 173 тестов. Закрыто новым тестом
`handlers_refuse_value_and_refuse_to_mutate_inside_a_static_frame`; правка хоста для
этого не понадобилась — `ContractContextV1::value` и `::is_static` публичны и доступны
через `sdk.context_mut()`.

Первое остаётся: каждое `assert!(harness.sdk.take_logs().is_empty())` после реверта
(`a_slash_whose_signatures_fail_verification_is_rejected`,
`re_slashing_a_tombstoned_validator_is_refused`,
`a_second_system_verdict_against_the_same_validator_is_a_no_op`) проходит потому, что
ревертящие пути ничего не испускали до реверта, а не потому, что харнесс это проверил.

### `commit_test_committee` / `record_test_production` / `store_test_consensus_keys`

Не умеют: пройти через `commitEpochCommittee` — сортировку по peer-ключу, запись
кольца, бит DKG. Держат зелёным: всю группу выплат и вердиктов по индексу. Настоящий
коммит гоняют `an_odd_member_count_leaves_no_stale_half_in_the_ring`,
`the_commit_orders_by_peer_key_and_membership_changes_mint_the_dkg_bit`,
`keys_activating_after_the_selection_epoch_are_filtered_before_the_cut` — согласованность
«что пишет коммит = что читает закрытие» запинена только там, где оба конца настоящие.

---

## 4. Часть 2 (F2) и часть 3 (F1) — на настоящем rWasm

### Чем отвечает токен, и почему двойник менялся

Три формы ОТКАЗА (реверт, горелка, усечённое слово) остались на самодельном
EVM-двойнике: там отказ — это и есть предмет, и двойник проверяет собственный отказ
перед использованием (`assert_token_refuses`).

Две новые формы — «прочитали успешно, но доступно меньше пота» — на НАСТОЯЩЕМ
`universal-token`, том самом, что стоит за BLEND в проде. Почему не двойник: он
отвечает константой `2^80−1` всем, о ком не отказывается, и не ведёт книги. Он не может
выразить ни «баланс меньше пота» (одна константа на все адреса), ни — что важнее для
случая K-32 — «баланс УПАЛ, потому что претензия его вытащила». Поднимать двойник до
книги значило бы написать ERC-20 в EVM-ассемблере под потолок `deploy_runtime`;
настоящий токен даёт то же самое и вдобавок делает читаемые пути (`balanceOf`,
`allowance`, `transferFrom`) отгружаемыми, а не имитированными. Оба фикстур-строителя
живут в одном файле и делят один `initialize_calldata`.

### Новые тесты e2e

**`a_reserve_that_answers_with_less_than_the_pot_forfeits_the_epoch`.** Два случая:
(а) баланс на одну базовую единицу меньше пота при щедром allowance;
(б) allowance на одну единицу меньше пота при щедром балансе. Каждый утверждает:
`reserve_available()` действительно равен `POT − 1` (анти-вакуум: читается с самого
токена, а не предполагается); закрытие эпохи 0 УСПЕШНО; ровно одно
`EpochBlendRewardsCommitted{0, 0}` с декодированной полезной нагрузкой;
`getEpochRewards(0) == 0`; `getValidatorFee` == 0 и обе претензии за эпоху 0 не двигают
ни одного токена резерва; после пополнения ИМЕННО ТОЙ половины, которой не хватало,
эпоха 1 финансируется, а эпоха 0 остаётся нулём.

**`a_claim_between_two_closes_puts_the_reserve_under_the_pot_and_burns_the_epoch`** —
K-32 живьём. Резерв ровно на один пот; эпоха 0 закрывается и начисляется; посторонний
адрес (`STRANGER`) шлёт `claimValidatorFee` — комиссия уходит ВЛАДЕЛЬЦУ (утверждается
приростом его баланса), а вызывающему не достаётся ничего (утверждается нулевым
балансом); резерв падает под пот (утверждается чтением); эпоха 1 закрывается под потом
и сгорает; после пополнения эпоха 2 финансируется, а эпоха 1 не воскресает.

### Мутации новых тестов e2e

Прогнаны в копии дерева A с `cargo clean -p fluentbase-contracts` перед каждым
прогоном; дайджест блоба печатался и двигался на каждой мутации (базовая линия
`cd54cf8d`, мутанты `e9de20ef`, `08958985`, `cd54cf8d`), так что ни один прогон не шёл
против устаревшего артефакта.

| мутация | результат |
|---|---|
| `EM1` — снять терм allowance (`Ok(core::cmp::min(balance, allowance))` → `Ok(balance)`) | 5 passed, **1 failed**: `a_reserve_that_answers_with_less_than_the_pot_forfeits_the_epoch`, и падает именно на ноге «approval one unit short of the pot». Нога «balance short» остаётся зелёной — то есть две половины `min` запинены ПОРОЗНЬ, а не одной ассерцией |
| `EM2` — снять форфейт (`if reserve_available(sdk, reserve)? < pot`) | 0 passed, **6 failed** — весь файл, включая K-32 («a reserve one claim under the pot forfeits the closing epoch whole») |
| `EM3` — пять холодных записей в хранилище ПОСЛЕ чтения резерва | 5 passed, **1 failed**: `the_fuel_burning_read_against_the_production_system_call_budget`. Запас упал 420 484 → **309 984**, закрытие ещё ВЫЖИВАЕТ, и валит его именно порог — то есть тест ловит добавленную работу задолго до того, как она сможет уронить блок |

### Часть 3 — F1 превращена в утверждение

Было: печать `SURVIVED`/`FAILED` и явный отказ утверждать исход горящей ветви.
Стало — два утверждения на каждом из трёх размеров комитета (5, 21, 51):

1. `burnt_close.succeeded` — потому что «FAILED» здесь означает остановку цепи:
   закрытие идёт предысполнительным системным вызовом, и его провал не чинится
   транзакцией.
2. `SYSTEM_CALL_BUDGET − frame_gas >= BURNT_CLOSE_HEADROOM_FLOOR`.

Замер (мой прогон, воспроизведён трижды): горящее закрытие тратит **29 579 516** газа
из 30 000 000 — одинаково при комитете 5, 21 и 51 — остаётся **420 484**. Порог
поставлен на **400 000** и объяснён в самом тесте: он не регрессионная защита на
420 484 (это число никто не выбирал), а растяжка на ПОРЯДОК ног. Причина, по которой
остаток сегодня достаточен, записана рядом со ссылками, которые я открыл в этой сессии:
`erc20_scalar_read` передаёт `fuel: None`, а EVM удерживает 1/64 —
`A:crates/revm/src/syscall.rs:423` передаёт `call_stipend_reduction(gas.remaining())`,
это `gas_limit − gas_limit / 64` (revm-rwasm `8674122`,
`crates/context/interface/src/cfg/gas_params.rs:605-606`, делитель 64 засеян на `:211`;
бюджет 30M — `crates/handler/src/system_call.rs:65`) [KNOWN, все четыре места открыты].
Чтение резерва — последняя нога: `close_epoch` кончается на `staking::accrue_epoch`, а
ветка форфейта в `assign_epoch_shares` возвращается до обхода комитета — поэтому число
не двигается с размером комитета.

Колпак топлива НЕ поставлен: вариант А, решение пользователя.

---

## 5. Находки

### F-1. Три теста личности не пинят формат сжатия ключа: их фикстура одноцветная

- **Утверждение.** `register_validator_cast_calldata_registers_consensus_keys_atomically`,
  `register_validator_verifies_and_stores_consensus_keys_in_one_call` и
  `get_consensus_keys_matches_dynamic_struct_return_vectors` сверяют сохранённый сжатый
  ключ с `compressed_key_of(0x11)` — но их вход заполнен ОДНИМ байтом на все 256, так
  что обе половины `x` совпадают и перестановка половин EIP-2537 ↔ zcash не меняет ни
  байта результата.
- **Опора.** Мутация `M65` (`bls.rs:326-327`, перестановка отменена): 2 упавших теста,
  и это ровно те два, что подают РАЗНЫЕ половины —
  `g2_compression_swaps_the_halves_and_reads_the_sign_from_c1` и
  `the_y_sign_bit_is_strictly_above_half_the_field`. Три названных остались зелёными.
  `tests.rs:427-432` (`compressed_key_of`), `tests.rs:2202-2206` и др. (фикстуры).
- **Уверенность:** [KNOWN] — мутация моя, прогон мой.
- **Чем пытался опровергнуть.** Искал, не пинит ли формат что-то ещё на этих путях:
  `M91` (снять запись `bls_pubkey_owner`) и `M91b` (снять запись `activation_epoch`)
  красные, то есть ЗАПИСЬ привязки покрыта — не покрыт именно ФОРМАТ ссылки.
- **К какому тесту/работе.** 1.10; T-1 из `AUDIT-CONTRACT.md`.
- **Дублирует:** T-1, но T-1 говорил это про удалённую заглушку верификатора; здесь то
  же свойство показано мутацией уже про инлайн-`bls.rs`. **Связано:** F-2.
- **Последствие, если не трогать.** Ошибка в порядке слов ключа ловится ровно одним
  тестом. Он есть и он красный — но три теста, которые ЧИТАЮТСЯ как проверка привязки
  «сжатый ключ ↔ владелец», её не проверяют, и это ложная уверенность в реестре.
  Лечится сменой байтов в фикстуре, без правки хоста.

### F-2. Пять «нулевых» веток `assign_epoch_shares` взаимно избыточны; ни одна не запинена в одиночку

- **Утверждение.** `pot.is_zero()` (`staking.rs:2079`), `len == 0` (`:2124`),
  `weight.is_zero()` в первом проходе (`:2162`), `total_weight.is_zero()` (`:2170`) и
  `weight.is_zero()` во втором проходе (`:2175`) дают один и тот же наблюдаемый исход —
  ноль. Снятие любой ОДНОЙ не роняет ни одного теста; снятие всех пяти сразу роняет
  один.
- **Опора.** `M97`, `M98`, `M99`, `M100` — все ЗЕЛЁНЫЕ; `E1` (все пять) — КРАСНАЯ, падает
  `a_committee_with_only_zero_weights_assigns_its_epoch_nothing`.
- **Уверенность:** [KNOWN].
- **Чем пытался опровергнуть.** Прошёл по коду, почему исход один и тот же: при `len == 0`
  `read_weights` отдаёт `Some(vec![])`, обход пуст, `total_weight` ноль; при нулевом
  `total_weight` второй обход пропускает каждое место по `weight.is_zero()`; при нулевом
  поте доля `pot * w / W` ноль и её пропускает `share.is_zero()`. Кода, который выдал бы
  разный исход, не нашёл.
- **К какому тесту/работе.** 1.7 / 1.9.
- **Дублирует:** нет. **Связано:** F-13.
- **Последствие, если не трогать.** Ничего не ломается сегодня. Но «покрыто тестом» для
  этих пяти веток означает «покрыт их дизъюнкт», и удаление любой одной — например при
  чистке — пройдёт молча вместе с её комментарием, объясняющим, ЗАЧЕМ она была.

### F-3. `install_stipend_token` не ведёт баланс самого контракта

- **Утверждение.** Заглушка ведёт книгу резерва (баланс и allowance списываются при
  `transferFrom`), но не ведёт книгу контракта: `transfer` всегда успешен, а
  `transferFrom` в пользу `GENESIS_STAKING` ничего не зачисляет.
- **Опора.** `tests.rs:706-734` — ветка `SIG_ERC20_TRANSFER` возвращает `true` без
  всякой проверки; в `StipendFunding` полей контрактной стороны нет (`:402-417`).
- **Уверенность:** [KNOWN] по коду заглушки.
- **Чем пытался опровергнуть.** Искал тест, который на юнитах утверждал бы, что вывод
  принципала упирается в фактический баланс контракта: `M53` (платить принципал с
  резерва) красная, то есть ИСТОЧНИК запинен, а достаточность — нет.
- **К какому тесту/работе.** 1.7; T-2.
- **Дублирует:** T-2 частично. **Связано:** F-1.
- **Последствие, если не трогать.** «Незаполненный резерв не держит депозит заложником»
  на юнитах доказано против книги, в которой у контракта бесконечный баланс. На rWasm
  эта половина теперь честная (`staking_reserve.rs` на настоящем токене), на юнитах —
  нет.

### F-4. Гейт `ERR_CONSENSUS_KEYS_ALREADY_SET` не покрыт ничем и недостижим

- **Утверждение.** `consensus.rs:177-179` не убивается ни одиночной мутацией (`C5`
  ЗЕЛЁНАЯ), ни в комбинации: до него не доходит ни один путь, потому что гейты дубля
  валидатора (`C2`) отсекают повторную регистрацию раньше.
- **Опора.** `C5` ЗЕЛЁНАЯ; док-комментарий `consensus.rs:180-187` признаёт то же самое
  про два соседних рекчека.
- **Уверенность:** [KNOWN] по мутации; [ГИПОТЕЗА] что путей нет вообще — я перебрал
  вызывающих `store_consensus_keys` (их два: `register_validator` и `initialize`), и оба
  зовут её сразу после `verify_consensus_keys`.
- **Чем пытался опровергнуть.** Комбинация `C2` (снять оба гейта дубля) — красная, то
  есть свойство запинено; но падает `initialize_and_registration_...`, а не что-то, что
  дошло бы до `C5`.
- **К какому тесту/работе.** 1.10.
- **Дублирует:** T-7 («ветки без покрытия»). **Связано:** F-13.
- **Последствие, если не трогать.** Ничего: это пояс на записи, и он честно так назван
  в коде. Записан, чтобы следующая чистка не сочла его живой проверкой.

### F-5. `ensure_non_payable` и `ensure_mutable` не имели ни одного теста — ЗАКРЫТО

- **Утверждение.** Оба открывают КАЖДЫЙ обработчик крейта (`util.rs:35-47`), и до этой
  работы снятие любого из них не роняло ни одного из 173 тестов.
- **Опора.** `M101` и `M102` — обе ЗЕЛЁНЫЕ на первом прогоне; после нового теста
  `handlers_refuse_value_and_refuse_to_mutate_inside_a_static_frame` обе КРАСНЫЕ.
- **Уверенность:** [KNOWN].
- **Чем пытался опровергнуть.** Проверял, не нужен ли для этого хост: не нужен —
  `ContractContextV1::value` и `::is_static` публичны и доступны через
  `sdk.context_mut()`.
- **К какому тесту/работе.** T-6 — подтверждено мутацией, а не унаследовано.
- **Дублирует:** T-6. **Связано:** F-3.
- **Последствие, если не трогать.** Было: платёж, приложенный к любому вызову, и мутация
  внутри статического кадра проходили бы мимо тестов. Закрыто.

### F-6. `claiming_rewards_does_not_rewrite_historical_self_stake` был вакуумным — ЗАКРЫТО

- **Утверждение.** Тест поднимался на `Harness::new(0)`. Нулевой блок активации — это
  сентинел «не заряжено», при котором эпоха пинится на нуле; значит претензия, которую
  тест гонит, шла по окну нулевой ширины, не двигала курсор и ничего не потребляла. Его
  собственный комментарий («Settling a frontier is what makes the claim advance at all»)
  его фикстурой не выполнялся.
- **Опора.** Мутация `M103` (вернуть до-K-8 поведение: перештамповать открывающую запись
  очереди на эпоху претензии) на первом прогоне уронила только
  `reward_claims_are_bounded_to_one_thousand_epochs`, а этот тест — нет. После смены
  активации на `DEFAULT_EPOCH_BLOCK_INTERVAL` и добавления анти-вакуумной ассерции на
  курсор `M103` роняет оба.
- **Уверенность:** [KNOWN].
- **Чем пытался опровергнуть.** Проверил соседний тест
  `reward_claims_are_bounded_to_one_thousand_epochs` — там комментарий про сентинел стоит
  прямо в коде (`tests.rs:4967-4970`), то есть ловушка была известна и один тест её
  обошёл, а другой нет.
- **К какому тесту/работе.** 1.7.
- **Дублирует:** нет. **Связано:** F-17.
- **Последствие, если не трогать.** Было: тест на деньги, который не выполнял ни одного
  шага пути, который называет. Закрыто.

### F-7. `committee_changed` без терма длины принимает укоротившийся комитет как неизменённый — ЗАКРЫТО

- **Утверждение.** Позиционный обход идёт по НОВОМУ срезу, поэтому преемник, который
  является строгим префиксом действующего комитета, сравнивается равным до конца и
  читается как «не изменился». Тогда запись переиспользуется, а индекс запоминает
  короткую длину против ДЛИННОЙ записи.
- **Опора.** `M2` (снять терм длины, `consensus.rs:329-331`) ЗЕЛЁНАЯ до правки, КРАСНАЯ
  после добавления случая «на одного короче».
- **Уверенность:** [KNOWN].
- **Чем пытался опровергнуть.** Проверил случай роста: при `N > L` обход читает
  `incumbent.at(i)` за концом записи, получает нулевой адрес и честно отвечает
  «изменился» — терм длины нужен ровно для укорочения, и это записано в комментарии
  теста.
- **К какому тесту/работе.** 1.1; R1.5b из `E1-REFLECTION.md`.
- **Дублирует:** R1.5b — да, это её закрытие. **Связано:** F-8, F-13.
- **Последствие, если не трогать.** Было: единственная проверка, отделяющая «комитет
  сократился» от «комитет тот же», не запинена ничем.

### F-8. Повторный штамп исключения: тест не различал два разных отказа — ЗАКРЫТО

- **Утверждение.** `production_exclusion_bites_at_the_next_epoch_and_not_before`
  утверждал, что повторный `apply_production_exclusion` отказывает. Но его фикстура
  держала пять валидаторов при поле `MIN_COMMITTEE_LENGTH = 4`, так что после первого
  исключения популяция на эпохе укуса падала до 4 и повтор отказывал по ПОЛУ, а не по
  «уже невидим». Проверка «уже невидим» была свободна.
- **Опора.** `M18` (снять отказ по невидимости, `staking.rs:288-290`) ЗЕЛЁНАЯ до правки.
  После расширения набора до шести валидаторов (популяция после первого исключения — 5,
  ровно `MIN_COMMITTEE_LENGTH + 1`) `M18` КРАСНАЯ. В тест добавлена анти-вакуумная
  ассерция `eligible_population_at_least(1, MIN_COMMITTEE_LENGTH + 1)`, чтобы сокращение
  фикстуры на одного участника не вернуло всё как было молча.
- **Уверенность:** [KNOWN].
- **Чем пытался опровергнуть.** Ожидания `selected_at(0)` и `selected_at(1)` при шести
  валидаторах не изменились — шестой стоит ниже среза и в комитет не входит; то есть
  расширение фикстуры не переписало свойство, а только вернуло различимость.
- **К какому тесту/работе.** 1.1.
- **Дублирует:** нет (в `E1-REFLECTION.md` M18 назван выжившим без разбора причины).
  **Связано:** F-7.
- **Последствие, если не трогать.** Было: без этой проверки повторный штамп в одном
  закрытии испускал бы второе `ProductionExclusionApplied` и тратил ход лестницы на
  исключение, которого не было.

### F-9. Гейт «владелец уже занят» не имел теста — ЗАКРЫТО

- **Утверждение.** `staking.rs:97-104` — единственное, что не даёт одному владельцу
  зарегистрировать второго валидатора; `owner_validators` — карта один-к-одному, и
  вторая регистрация перезаписала бы её.
- **Опора.** `M25` ЗЕЛЁНАЯ до правки, КРАСНАЯ после нового теста
  `one_owner_cannot_register_a_second_validator`.
- **Уверенность:** [KNOWN].
- **Чем пытался опровергнуть.** Комбинация `C2` (оба гейта дубля ВАЛИДАТОРА) красная, но
  ключ у них другой — валидатор, не владелец; ни один из них этот случай не ловит.
- **К какому тесту/работе.** 1.5 / pre-Э1.
- **Дублирует:** K-40 из `AUDIT-CONTRACT.md` касается соседнего свойства (адрес
  валидатора — произвольный параметр), не этого. **Связано:** F-13.
- **Последствие, если не трогать.** Было: снятие гейта оставляет первого валидатора
  недостижимым через своего же владельца, с уже забранным взносом, и ни один тест этого
  не видит.

### F-10. Ветка «эпоха без единого записанного блока» не имела теста — ЗАКРЫТО

- **Утверждение.** `accrue_epoch`'s `recorded == 0` (`staking.rs:2049-2053`) — это
  ЕДИНСТВЕННОЕ, что не даёт эпохе с закоммиченным комитетом, живыми весами, ненулевым
  потом и покрывающим резервом выписать полный пот за нулевую работу. Ни один из пяти
  нулевых возвратов внутри `assign_epoch_shares` в этом состоянии не срабатывает.
- **Опора.** `M42` ЗЕЛЁНАЯ до правки: единственный тест этой формы,
  `an_uncommitted_committee_parks_the_block_instead_of_reverting`, комитета не коммитит
  вовсе, так что уходит в ветку `len == 0`. КРАСНАЯ после нового теста
  `an_epoch_with_a_committee_but_no_recorded_block_draws_no_pot`.
- **Уверенность:** [KNOWN].
- **Чем пытался опровергнуть.** Новый тест содержит контрольную ногу: та же фикстура с
  одним записанным блоком платит полный пот — значит ноль в первой ноге даёт счётчик
  блоков, а не что-то ещё.
- **К какому тесту/работе.** 1.7.
- **Дублирует:** нет. **Связано:** F-2.
- **Последствие, если не трогать.** Было: остановившийся рекордер или преактивационный
  префикс покупал бы стипендию целой эпохи за нулевую работу, и снятие ветки прошло бы
  молча.

### F-11. Обе ветки отказа `committee_member_at` не имели теста — ЗАКРЫТО

- **Утверждение.** `ERR_EPOCH_COMMITTEE_NOT_COMMITTED` и `ERR_SIGNER_INDEX_OUT_OF_RANGE`
  (`consensus.rs:674-683`) не покрывались ничем. Незакоммиченная эпоха читает индексную
  пару `(record 0, length 0)` — и без первого отказа разрешила бы место 0 против того,
  что лежит в записи 0, то есть против ЧУЖОГО комитета.
- **Опора.** `M76` (снять оба отказа) ЗЕЛЁНАЯ до правки, КРАСНАЯ после нового теста
  `a_system_verdict_naming_a_seat_the_committee_does_not_have_is_refused`. Новый тест
  утверждает `committee_at(9) == (0, 0)` явно, чтобы состояние было видно, а не
  предполагалось.
- **Уверенность:** [KNOWN].
- **Чем пытался опровергнуть.** Единственный тест индексного маршрута,
  `the_index_slash_route_still_resolves_far_past_the_retired_pruning_horizon`, гоняет
  только положительный случай.
- **К какому тесту/работе.** 1.2 (системный маршрут, не маршрут улик — он в объёме).
  T-7 называл ровно эти две ветки.
- **Дублирует:** T-7 — да, это её закрытие мутацией и тестом. **Связано:** нет.
- **Последствие, если не трогать.** Было: вердикт, назвавший несуществующее место,
  тумбстоунил бы валидатора, которого он не называл, из предысполнительного системного
  вызова, который нечем откатить.

### F-12. Ветка нулевого знаменателя в `snapshot_payout` не имела теста — ЗАКРЫТО, но её достижимость в проде остаётся открытой

- **Утверждение.** `if selection_total.is_zero() { return Ok((ZERO, total_reward)); }`
  (`staking.rs:1365-1367`) решает, КУДА уходит кредит целой эпохи, когда у места нет
  снапшота на эпохе отбора: сегодня — весь владельцу. Ни один тест этого не касался.
- **Опора.** `M96` ЗЕЛЁНАЯ до правки, КРАСНАЯ после нового теста
  `a_seat_with_no_snapshot_at_its_selection_epoch_pays_its_whole_credit_to_the_owner`.
  Состояние достижимо обычным путём: валидатор, зарегистрированный в эпохе 5,
  материализует первый снапшот на 6, а эпоха 7 отбирается с эпохи 5 — тест это и делает
  через настоящий `registerValidator`.
- **Уверенность:** [KNOWN] что ветка достижима в юнит-харнессе и что поведение — «всё
  владельцу». [ОТКРЫТО] может ли такое место реально СИДЕТЬ в комитете эпохи 7 в проде:
  это вопрос R9.1 из `E1-REFLECTION.md`, и я его не закрывал.
- **Чем пытался опровергнуть.** Проверил вторую половину: делегатский обход на этой же
  эпохе платит ноль (`delegator_fee_of == 0`), потому что `delegate_claim_start`
  сдвигает первую награду на `first_reward_epoch_for(6) = 8`. То есть «всё владельцу» —
  не потеря, а адресация; но адресация НИКЕМ не была выбрана явно.
- **К какому тесту/работе.** 1.9; R9.1.
- **Дублирует:** R9.1 частично. **Связано:** F-2.
- **Последствие, если не трогать.** Теперь запинено. Если решение «весь кредит владельцу»
  неверно — тест покраснеет и заставит его пересмотреть, а не пройдёт молча.

### F-13. Дублированные проверки: удаление любой одной копии проходит молча

- **Утверждение.** Четыре набора: три потолка комиссии (`initializer.rs:89`,
  `staking.rs:1044`, `:85`), два гейта дубля валидатора (`staking.rs:94`, `:1047`), два
  гейта занятого BLS-ключа (`consensus.rs:123-130`, `:197-208`), два гейта занятого
  peer-ключа (`:112-119`, `:188-195`). В каждом наборе снятие ЛЮБОЙ одной копии не
  роняет ни одного теста; снятие всего набора роняет один.
- **Опора.** `M23`, `M23b`, `M23c`, `M24`, `M62`, `M63` — ЗЕЛЁНЫЕ; `C1`, `C2`, `C3`,
  `C4` — КРАСНЫЕ.
- **Уверенность:** [KNOWN].
- **Чем пытался опровергнуть.** Читал док-комментарий над рекчеками в
  `store_consensus_keys` (`consensus.rs:180-187`): он утверждает, что они недостижимы и
  ни один тест до них не доходит. Мутация показывает обратную сторону того же факта —
  когда снят ПЕРВЫЙ гейт, тест ловит именно ВТОРОЙ. Оба чтения верны; ни одно из них не
  говорит, какая копия обязана остаться.
- **К какому тесту/работе.** 1.5, 1.10.
- **Дублирует:** нет. **Связано:** F-2, F-4, F-9.
- **Последствие, если не трогать.** Это законная причина оставить тесты как есть
  (проверка избыточна), но НЕ повод считать каждую копию защищённой: чистка, снимающая
  одну, пройдёт зелёной вместе с комментарием, объясняющим, зачем она.

### F-14. Безопасность горелки держится на порядке ног, и теперь у этого есть тест

- **Утверждение.** Запас 420 484 газа — не спроектированный колпак, а 1/64, которую EVM
  не отдаёт дочернему кадру. Он достаточен только потому, что чтение резерва — последняя
  нога закрытия.
- **Опора.** Замер (три прогона, одно и то же число при комитете 5/21/51); мутация `EM3`
  (пять холодных записей после чтения) роняет запас до 309 984 и валит порог, при этом
  закрытие ещё выживает — то есть тест ловит добавленную работу задолго до остановки
  блока. Ссылки на 63/64 и на бюджет 30M открыты в этой сессии (см. §4).
- **Уверенность:** [KNOWN].
- **Чем пытался опровергнуть.** Прогнал под 30M при комитете 5, 21 и 51 — цифра
  идентична, потому что после форфейта комитет не обходится.
- **К какому тесту/работе.** 1.7 (R7.10), F1 из `E1-CLOSEOUT.md`.
- **Дублирует:** F1 — да, это её закрытие утверждением. **Связано:** нет.
- **Последствие, если не трогать.** Было: число ничем не запинено, и любая работа после
  чтения резерва получала бы 1/64 остатка вместо 30M — молча.

### F-15. F2 закрыт: форфейт при реально коротком резерве бежал на rWasm

- **Утверждение.** Обе половины `min(balance, allowance)` и сам форфейт проверены на
  настоящем рантайме и настоящем BLEND, а не на моке.
- **Опора.** `a_reserve_that_answers_with_less_than_the_pot_forfeits_the_epoch` — зелёный;
  `EM1` роняет ровно ногу allowance, `EM2` роняет обе.
- **Уверенность:** [KNOWN].
- **Чем пытался опровергнуть.** Анти-вакуум: `reserve_available()` читается с самого
  токена и сверяется с `POT − 1` до закрытия; без него «эпоха дала ноль» не отличалось
  бы от «фикстура не профинансировала резерв».
- **К какому тесту/работе.** 1.7; F2 из `E1-CLOSEOUT.md`.
- **Дублирует:** F2 — да, это её закрытие. **Связано:** F-16.
- **Последствие, если не трогать.** Было: самая денежно-значимая ветка контракта
  подтверждена только моком.

### F-16. K-32 воспроизведён живьём, серией из одной претензии

- **Утверждение.** Посторонний адрес одной беспермиссионной претензией опускает резерв
  под пот перед границей, и закрывающаяся эпоха сгорает целиком.
- **Опора.** `a_claim_between_two_closes_puts_the_reserve_under_the_pot_and_burns_the_epoch`;
  `EM2` роняет и его.
- **Уверенность:** [KNOWN].
- **Чем пытался опровергнуть.** Утверждается, что комиссия ушла ВЛАДЕЛЬЦУ (прирост его
  баланса) и что вызывающему не досталось ничего (нулевой баланс) — то есть это
  гриферство ценой газа, а не кража; и что после пополнения эпоха 2 платит, а сгоревшая
  эпоха 1 не воскресает.
- **К какому тесту/работе.** 1.5 (K-32 оставлена), 1.7 (W2).
- **Дублирует:** §6.3 из `E1-CLOSEOUT.md` («претензия — одна, а не серия»): серия и не
  нужна, при резерве ровно на один пот хватает одной. **Связано:** F-15.
- **Последствие, если не трогать.** Вектор записан в аудите как «кодом не чинится»;
  теперь он ещё и запинен, так что изменение, случайно его закрывающее или
  усугубляющее, станет видимым.

### F-17. Два теста «денег»/«личности» не краснеют ни от какой мутации ПРОВЕРКИ

- **Утверждение.** `undelegate_period_change_does_not_shorten_queued_principal` и
  `the_index_slash_route_still_resolves_far_past_the_retired_pruning_horizon` охраняют
  ОТСУТСТВИЕ механизма (перерасчёт срока созревания по живому конфигу; прунинг
  комитетов). Снимать в коде нечего.
- **Опора.** Ни одна из 97 мутаций их не уронила. По коду: `undelegate_from` кладёт
  `maturity_epoch` в очередь один раз (`staking.rs:1300-1305`), а
  `set_undelegate_period` пишет только поле конфига (`config.rs:486-488`) — пути, который
  мог бы переписать очередь, нет; прунинга комитетов в крейте нет вовсе.
- **Уверенность:** [KNOWN] по мутациям и по коду.
- **Чем пытался опровергнуть.** Для третьего теста этого класса,
  `claiming_rewards_does_not_rewrite_historical_self_stake`, механизм ЕСТЬ в истории
  (до-K-8 дизайн), я его вернул мутацией `M103` — и он оказался вакуумным по другой
  причине (F-6). Для этих двух аналогичного исторического механизма в дереве не нашёл.
- **К какому тесту/работе.** 1.7, 1.5.
- **Дублирует:** нет. **Связано:** F-6.
- **Последствие, если не трогать.** По букве Д-12 это тесты, которые не краснеют при
  сломанном коде. По существу — регрессионные сторожа на удалённые механизмы, и
  единственный способ сделать их красными это вернуть механизм. Оставлены; см. §6.

---

## 6. Оставлено как есть

Тесты, которые я рассматривал к переписыванию или удалению и оставил, с причиной по
каждому.

1. **`a_committee_with_only_zero_weights_assigns_its_epoch_nothing`** — четыре из пяти
   веток, которые он охраняет, поодиночке не краснеют (`M97`–`M100`). Оставлен и НЕ
   расщеплён на пять: избыточность здесь в КОДЕ, а не в тесте — все пять веток дают один
   исход (F-2), и пять тестов на один наблюдаемый исход были бы пятью копиями одного
   утверждения. Комбинация `E1` показывает, что дизъюнкт запинен.
2. **`initialize_and_registration_reject_bad_commission_and_duplicate_validator`** —
   `M23`/`M23b`/`M23c`/`M24` зелёные поодиночке, `C1`/`C2` красные. Причина оставить:
   проверка избыточна (три копии одного потолка, две копии одного гейта), и тест
   утверждает СВОЙСТВО, а не конкретную копию. Записано находкой F-13.
3. **`registration_rejects_replayed_bls_key_and_pop_without_partial_state`** и
   **`a_peer_key_cannot_be_reassigned_so_the_sort_key_is_immutable`** — то же: `M62`/`M63`
   зелёные, `C3`/`C4` красные. Проверка избыточна.
4. **`undelegate_period_change_does_not_shorten_queued_principal`** — ни одна мутация не
   краснит. Оставлен, потому что снимать нечего: он охраняет отсутствие перерасчёта, а
   пути перерасчёта в коде нет (F-17). Удалять не стал: это регрессионный сторож на
   дизайн, который однажды существовал.
5. **`the_index_slash_route_still_resolves_far_past_the_retired_pruning_horizon`** — то
   же основание (сторож на удалённый прунинг). Оставлен.
6. **`staking_is_a_genesis_rwasm_contract_not_a_system_precompile`** — рассматривал к
   удалению как проверку структуры чужого крейта. Оставлен: он пинит НЕ структуру
   стандартной библиотеки, а факт, от которого зависит учёт топлива этого контракта —
   что `GENESIS_STAKING` не сидит ни в `EXECUTE_USING_SYSTEM_RUNTIME_ADDRESSES`, ни в
   таблице метрируемых предеплоев. Если это перестанет быть правдой, изменится поведение
   контракта, а не только SDK.
7. **`contract_storage_uses_separate_erc7201_namespaces`** — рассматривал: пять первых
   ассерций сравнивают слот аксессора с константой, из которой он же построен. Оставлен:
   это не константа против своей копии, а пин того, что КОНКРЕТНЫЙ аксессор резолвится в
   КОНКРЕТНЫЙ корень (перепутанные корни в `storage.rs` он ловит), плюс цикл на
   неалиасинг пяти независимо выведенных из строк keccak-констант.
8. **`derived_selectors_match_independent_hex_pins`**, **`devnet_view_selectors_match_their_pinned_ids`** —
   рассматривал как тавтологии. Оставлены: `SIG_*` выводятся макросом
   `derive_keccak256_id!` из строк сигнатур (`consts.rs:20-25`), а правая часть — литералы
   из `cast`; это сверка вывода с внешним источником, не константы с собой.
9. **`sole_validator_owner_full_exit_deactivates_without_leaving_subminimum_dust`** —
   его собственный комментарий признаёт, что последние два чтения (`tests.rs`, цикл по
   `selected_at`) «добавляют мало». Оставлены как есть: тест в целом краснеет от `M32` и
   `M33`, а вычищать признанно-слабые ассерции — это правка стиля, вне правила приёмки.
10. **`a_committee_stays_readable_far_past_the_retired_pruning_horizon`** — класс «отбор»,
    сторож на удалённый прунинг; мутациям не подвергался, потому что снимать нечего.
    Оставлен.

---

## 7. Вне объёма

- Двенадцать тестов маршрута улик (permissionless `slashEquivocation*` по уликам) —
  отложены с работой 1.2, Д-4 не решено; перечислены в реестре с классом
  «личность (улики)», не мутировались.
- `evidence.rs` и его 21 собственный `#[test]` — не читал, вне объёма 1.8.
- `math.rs` (4 теста) и `util.rs` (1 тест) — не входят в 144/149 списка `tests.rs`, не трогал.
- Классы «отбор», «liveness», «конфиг», «ABI-вью» — мутировались только там, где
  пересекаются с деньгами и личностью; правило Д-12 к ним не применялось.
- Все e2e-файлы, кроме `staking_reserve.rs` — не менял и не мутировал.
- Живой девнет и смоук-стенд — не поднимал.
- Дерево B — изменён только этот отчёт.

---

## 8. Где проверка была самой слабой

1. **74 теста не краснели ни от одной мутации, и для большинства это не приговор, а
   отсутствие замера.** Я мутировал проверки классов «деньги» и «личность»; тесты
   liveness, отбора и конфига остались зелёными потому, что их проверок я не снимал.
   Отличить «избыточен» от «не измерен» для них по этой работе нельзя.
2. **Мутации — только снятие проверок и три возврата удалённых механизмов.** Я не
   мутировал арифметику: границы циклов, знаки сравнений (кроме `fp_greater_half`),
   off-by-one в бинарных поисках, порядок операндов в делении. Тест, зелёный при
   сдвинутом индексе, этой работой не пойман.
3. **Порог 400 000 в части 3 выбран мной, а не измерен как граница.** Я знаю, что
   420 484 — фактический остаток и что 110 500 газа лишней работы его валят; где именно
   между 400 000 и 420 484 проходит настоящая граница безопасности, не измерял.
4. **`install_stipend_token` осталась мягкой по балансу самого контракта** (F-3). Я
   назвал это находкой, но не починил, и два теста продолжают утверждать про выход
   депозита из баланса, которого заглушка не ведёт.
5. **Фикстуры трёх тестов личности остались одноцветными** (F-1). Мутация показала
   мягкость, но менять фикстуры я не стал — это вне правила приёмки, а значит реестр
   до сих пор содержит три строки, которые читаются сильнее, чем есть.
6. **Взаимная избыточность проверена только там, где я её заподозрил.** Пять комбинаций
   (`C1`–`C5`, `E1`) — это все, что я поставил; систематического перебора пар не было,
   и другие наборы взаимно прикрывающих проверок могли остаться незамеченными.
7. **Мутации e2e — три, и все по одному файлу.** `staking_cost.rs`, `staking.rs`,
   `staking_bls.rs` под Д-12 не проверялись вовсе.
8. **Достижимость ветки F-12 в проде не установлена.** Я запинил её ПОВЕДЕНИЕ, не
   доказав, что валидатор без снапшота на эпохе отбора может реально сидеть в комитете
   этой эпохи (R9.1 остаётся открытым).
