#[cfg(test)]
mod tests {
    use crate::{
        fvm::types::WasmStorage,
        helpers_fvm::{fvm_transact, fvm_transact_commit},
    };
    use alloc::{vec, vec::Vec};
    use core::str::FromStr;
    use fluentbase_sdk::{
        journal::{JournalState, JournalStateBuilder},
        runtime::TestingContext,
        Bytes34,
        ContractContext,
    };
    use fuel_core::{
        database::{database_description::on_chain::OnChain, Database, RegularStage},
        executor::test_helpers::{create_contract, setup_executable_script},
        txpool::types::TxId,
    };
    use fuel_core_executor::{executor::ExecutionData, refs::ContractRef};
    use fuel_core_storage::{
        rand::rngs::StdRng,
        structured_storage::StructuredStorage,
        tables::Coins,
        transactional::{Modifiable, WriteTransaction},
        Mappable,
        StorageAsMut,
        StorageInspect,
        StorageMutate,
    };
    use fuel_core_types::{
        blockchain::{
            block::PartialFuelBlock,
            header::{ConsensusHeader, PartialBlockHeader},
        },
        entities::coins::coin::CompressedCoin,
        fuel_asm::{op, RegId},
        fuel_crypto::rand::{Rng, SeedableRng},
        fuel_merkle::sparse,
        fuel_tx::{
            field::{Inputs, Outputs, Script},
            Address,
            AssetId,
            Cacheable,
            ConsensusParameters,
            Output,
            Transaction,
            TxParameters,
            UniqueIdentifier,
            UtxoId,
        },
        fuel_types::{canonical::Serialize, ChainId, ContractId, Word},
        fuel_vm::{
            checked_transaction::IntoChecked,
            interpreter::{ExecutableTransaction, MemoryInstance},
            script_with_data_offset,
            util::test_helpers::TestBuilder,
            Call,
            ProgramState,
        },
        services::executor::{Error, TransactionValidityError},
    };
    use revm_primitives::{alloy_primitives, U256};

    fn test_builder() -> TestBuilder {
        TestBuilder::new(1234u64)
    }
    fn contract_context() -> ContractContext {
        ContractContext {
            address: alloy_primitives::Address::from_slice(&[01; 20]),
            bytecode_address: alloy_primitives::Address::from_slice(&[00; 20]),
            caller: alloy_primitives::Address::from_slice(&[00; 20]),
            is_static: false,
            value: U256::default(),
        }
    }

    fn journal_state() -> JournalState<TestingContext> {
        let mut journal_state_builder = JournalStateBuilder::default();
        journal_state_builder.add_contract_context(contract_context());
        JournalState::builder(TestingContext::empty(), journal_state_builder)
    }

    type TestingSDK = JournalState<TestingContext>;

    #[test]
    fn skipped_tx_not_changed_spent_status() {
        let mut sdk = journal_state();
        let wasm_storage = WasmStorage { sdk: &mut sdk };
        let mut storage = StructuredStorage::new(wasm_storage);
        // let mut tb = || TestBuilder::new(2322u64);
        // let mut db = GenericDatabase::from_storage(storage);
        // `tx2` has two inputs: one used by `tx1` and on random. So after the execution of `tx1`,
        // the `tx2` become invalid and should be skipped by the block producers. Skipped
        // transactions should not affect the state so the second input should be `Unspent`.
        // # Dev-note: `TxBuilder::new(2322u64)` is used to create transactions, it produces
        // the same first input.
        let tx1 = test_builder()
            .coin_input(AssetId::default(), 100)
            .change_output(AssetId::default())
            .build()
            .transaction()
            .clone();

        let tx2 = test_builder()
            // The same input as `tx1`
            .coin_input(AssetId::default(), 100)
            // Additional unique for `tx2` input
            .coin_input(AssetId::default(), 100)
            .change_output(AssetId::default())
            .build()
            .transaction()
            .clone();

        let first_input = tx2.inputs()[0].clone();
        let mut first_coin = CompressedCoin::default();
        first_coin.set_owner(*first_input.input_owner().unwrap());
        first_coin.set_amount(100);
        let second_input = tx2.inputs()[1].clone();
        let mut second_coin = CompressedCoin::default();
        second_coin.set_owner(*second_input.input_owner().unwrap());
        second_coin.set_amount(100);
        // Insert both inputs
        let utxo_id = first_input.utxo_id().unwrap().clone();
        <StructuredStorage<WasmStorage<'_, TestingSDK>> as StorageMutate<Coins>>::insert(
            &mut storage,
            &utxo_id,
            &first_coin,
        )
        .expect("insert first utxo success");
        <StructuredStorage<WasmStorage<'_, TestingSDK>> as StorageMutate<Coins>>::insert(
            &mut storage,
            &second_input.utxo_id().unwrap().clone(),
            &second_coin,
        )
        .expect("insert first utxo success");

        let block = PartialFuelBlock {
            header: Default::default(),
            transactions: vec![tx1.clone().into(), tx2.clone().into()],
        };

        // The first input should be `Unspent` before execution.
        <StructuredStorage<WasmStorage<'_, TestingSDK>> as StorageInspect<Coins>>::get(
            &storage,
            first_input.utxo_id().unwrap(),
        )
        .unwrap()
        .expect("coin should be unspent");
        // The second input should be `Unspent` before execution.
        <StructuredStorage<WasmStorage<'_, TestingSDK>> as StorageInspect<Coins>>::get(
            &storage,
            second_input.utxo_id().unwrap(),
        )
        .unwrap()
        .expect("coin should be unspent");

        let consensus_params = ConsensusParameters::default();
        let coinbase_contract_id = ContractId::default();
        let tx1 = tx1.as_script().unwrap().clone();
        let create_tx_checked = tx1
            .into_checked(*block.header.height(), &consensus_params)
            .expect("into_checked successful");
        let mut storage_transaction = storage.write_transaction();
        let execution_data = &mut ExecutionData::new();
        let exec_result1 = fvm_transact_commit(
            &mut storage_transaction,
            create_tx_checked,
            &block.header,
            coinbase_contract_id,
            0,
            consensus_params.clone(),
            true,
            execution_data,
        );
        assert_eq!(true, exec_result1.is_ok());
        let exec_result1 = exec_result1.unwrap();
        storage.commit_changes(exec_result1.changes).unwrap();

        let tx2 = tx2.as_script().unwrap().clone();
        let tx2_checked = tx2
            .into_checked(*block.header.height(), &consensus_params)
            .expect("into_checked successful");
        let mut storage_transaction = storage.write_transaction();
        let execution_data = &mut ExecutionData::new();
        let exec_result2 = fvm_transact_commit(
            &mut storage_transaction,
            tx2_checked,
            &block.header,
            coinbase_contract_id,
            0,
            consensus_params.clone(),
            true,
            execution_data,
        );
        assert_eq!(true, exec_result2.is_err());

        // The first input should be spent by `tx1` after execution.
        let coin = storage
            .storage::<Coins>()
            .get(first_input.utxo_id().unwrap())
            .unwrap();
        // verify coin is pruned from utxo set
        assert!(coin.is_none());
        // The second input should be `Unspent` after execution.
        storage
            .storage::<Coins>()
            .get(second_input.utxo_id().unwrap())
            .unwrap()
            .expect("coin should be unspent");
    }

    #[test]
    fn coin_input_fails_when_mismatches_database() {
        const AMOUNT: u64 = 100;

        let tx = test_builder()
            .coin_input(AssetId::default(), AMOUNT)
            .change_output(AssetId::default())
            .build()
            .transaction()
            .clone();

        let input = tx.inputs()[0].clone();
        let mut coin = CompressedCoin::default();
        coin.set_owner(*input.input_owner().unwrap());
        coin.set_amount(AMOUNT - 1);
        let mut sdk = journal_state();
        let wasm_storage = WasmStorage { sdk: &mut sdk };
        let mut storage = StructuredStorage::new(wasm_storage);

        // Inserting a coin with `AMOUNT - 1` should cause a mismatching error during production.
        <StructuredStorage<WasmStorage<'_, TestingSDK>> as StorageMutate<Coins>>::insert(
            &mut storage,
            &input.utxo_id().unwrap().clone(),
            &coin,
        )
        .unwrap();

        let block = PartialFuelBlock {
            header: Default::default(),
            transactions: vec![tx.clone().into()],
        };

        // fluent tests
        let consensus_params = ConsensusParameters::default();
        let coinbase_contract_id = ContractId::default();
        let checked_tx = tx
            .into_checked(*block.header.height(), &consensus_params)
            .expect("into_checked successful");
        let mut memory = MemoryInstance::new();
        let execution_data = &mut ExecutionData::new();
        let mut storage_transaction = storage.write_transaction();
        let fvm_exec_result = fvm_transact(
            &mut storage_transaction,
            checked_tx,
            &block.header,
            coinbase_contract_id,
            0,
            &mut memory,
            consensus_params,
            true,
            execution_data,
        );
        assert_eq!(true, fvm_exec_result.is_err());
        let err = fvm_exec_result.unwrap_err();
        assert!(matches!(
            &err,
            &Error::TransactionValidity(TransactionValidityError::CoinMismatch(_))
        ));
    }

    #[test]
    fn contract_input_fails_when_doesnt_exist_in_database() {
        let contract_id: ContractId = [1; 32].into();
        let tx = test_builder()
            .contract_input(contract_id)
            .coin_input(AssetId::default(), 100)
            .change_output(AssetId::default())
            .contract_output(&contract_id)
            .build()
            .transaction()
            .clone();

        let input = tx.inputs()[1].clone();
        let mut coin = CompressedCoin::default();
        coin.set_owner(*input.input_owner().unwrap());
        coin.set_amount(100);
        let mut sdk = journal_state();
        let wasm_storage = WasmStorage { sdk: &mut sdk };
        let mut storage = StructuredStorage::new(wasm_storage);

        let block = PartialFuelBlock {
            header: Default::default(),
            transactions: vec![tx.clone().into()],
        };

        // fluent tests
        let consensus_params = ConsensusParameters::default();
        let coinbase_contract_id = ContractId::default();
        let checked_tx = tx
            .into_checked(*block.header.height(), &consensus_params)
            .expect("into_checked successful");
        let mut memory = MemoryInstance::new();
        let execution_data = &mut ExecutionData::new();
        let mut storage_transaction = storage.write_transaction();
        let fvm_exec_result = fvm_transact(
            &mut storage_transaction,
            checked_tx,
            &block.header,
            coinbase_contract_id,
            0,
            &mut memory,
            consensus_params,
            true,
            execution_data,
        );
        assert_eq!(true, fvm_exec_result.is_err());
        assert!(matches!(
            &fvm_exec_result.unwrap_err(),
            &Error::TransactionValidity(TransactionValidityError::ContractDoesNotExist(_))
        ));
    }

    #[test]
    fn contracts_balance_and_state_roots_no_modifications_updated() {
        // Values in inputs and outputs are random. If the execution of the transaction successful,
        // it should actualize them to use a valid the balance and state roots. Because it is not
        // changes, the balance the root should be default - `[0; 32]`.
        let mut rng = StdRng::seed_from_u64(2322u64);

        let (create, contract_id) = create_contract(vec![], &mut rng);
        let non_modify_state_tx: Transaction = test_builder()
            .script_gas_limit(10000)
            .coin_input(AssetId::zeroed(), 10000)
            .start_script(vec![op::ret(1)], vec![])
            .contract_input(contract_id)
            .fee_input()
            .contract_output(&contract_id)
            .build()
            .transaction()
            .clone()
            .into();
        let mut sdk = journal_state();
        let wasm_storage = WasmStorage { sdk: &mut sdk };
        let mut storage = StructuredStorage::new(wasm_storage);

        let block = PartialFuelBlock {
            header: PartialBlockHeader {
                consensus: ConsensusHeader {
                    height: 1.into(),
                    ..Default::default()
                },
                ..Default::default()
            },
            transactions: vec![create.clone().into(), non_modify_state_tx.clone()],
        };

        // fluent tests
        let consensus_params = ConsensusParameters::default();
        let coinbase_contract_id = ContractId::default();
        let create_tx = create.as_create().unwrap().clone();
        let create_tx_checked = create_tx
            .into_checked(*block.header.height(), &consensus_params)
            .expect("into_checked successful");
        let mut storage_transaction = storage.write_transaction();
        let execution_data = &mut ExecutionData::new();
        let fvm_exec_result = fvm_transact_commit(
            &mut storage_transaction,
            create_tx_checked,
            &block.header,
            coinbase_contract_id,
            0,
            consensus_params.clone(),
            false,
            execution_data,
        );
        assert_eq!(true, fvm_exec_result.is_ok());

        let script_non_modify_state_tx = non_modify_state_tx.as_script().unwrap().clone();
        let script_non_modify_state_tx_checked = script_non_modify_state_tx
            .into_checked(*block.header.height(), &consensus_params)
            .expect("into_checked successful");
        let fvm_exec_result = fvm_transact_commit(
            &mut storage_transaction,
            script_non_modify_state_tx_checked,
            &block.header,
            coinbase_contract_id,
            0,
            consensus_params,
            false,
            execution_data,
        );
        assert_eq!(true, fvm_exec_result.is_ok());
        let fvm_exec_result = fvm_exec_result.unwrap();
        let empty_state = (*sparse::empty_sum()).into();
        let executed_tx = fvm_exec_result.tx;
        assert_eq!(executed_tx.inputs()[0].state_root(), Some(&empty_state));
        assert_eq!(executed_tx.inputs()[0].balance_root(), Some(&empty_state));
        assert_eq!(executed_tx.outputs()[0].state_root(), Some(&empty_state));
        assert_eq!(executed_tx.outputs()[0].balance_root(), Some(&empty_state));
    }

    #[test]
    fn contracts_balance_and_state_roots_updated_no_modifications_on_fail() {
        // Values in inputs and outputs are random. If the execution of the transaction fails,
        // it still should actualize them to use the balance and state roots before the execution.
        let mut rng = StdRng::seed_from_u64(2322u64);

        let (create, contract_id) = create_contract(vec![], &mut rng);
        // The transaction with invalid script.
        let non_modify_state_tx: Transaction = test_builder()
            .start_script(vec![op::add(RegId::PC, RegId::PC, RegId::PC)], vec![])
            .contract_input(contract_id)
            .fee_input()
            .contract_output(&contract_id)
            .build()
            .transaction()
            .clone()
            .into();
        let mut sdk = journal_state();
        let wasm_storage = WasmStorage { sdk: &mut sdk };
        let mut storage = StructuredStorage::new(wasm_storage);

        let block = PartialFuelBlock {
            header: PartialBlockHeader {
                consensus: ConsensusHeader {
                    height: 1.into(),
                    ..Default::default()
                },
                ..Default::default()
            },
            transactions: vec![create.clone().into(), non_modify_state_tx.clone()],
        };

        // fluent tests
        let consensus_params = ConsensusParameters::default();
        let coinbase_contract_id = ContractId::default();
        let create_tx = create.as_create().unwrap().clone();
        let create_tx_checked = create_tx
            .into_checked(*block.header.height(), &consensus_params)
            .expect("into_checked successful");
        let mut storage_transaction = storage.write_transaction();
        let execution_data = &mut ExecutionData::new();
        let fvm_exec_result = fvm_transact_commit(
            &mut storage_transaction,
            create_tx_checked,
            &block.header,
            coinbase_contract_id,
            0,
            consensus_params.clone(),
            false,
            execution_data,
        );
        assert_eq!(true, fvm_exec_result.is_ok());

        let script_non_modify_state_tx = non_modify_state_tx.as_script().unwrap().clone();
        let script_non_modify_state_tx_checked = script_non_modify_state_tx
            .into_checked(*block.header.height(), &consensus_params)
            .expect("into_checked successful");
        let fvm_exec_result = fvm_transact_commit(
            &mut storage_transaction,
            script_non_modify_state_tx_checked,
            &block.header,
            coinbase_contract_id,
            0,
            consensus_params,
            false,
            execution_data,
        );
        assert_eq!(true, fvm_exec_result.is_ok());
        let fvm_exec_result = fvm_exec_result.unwrap();
        let empty_state = (*sparse::empty_sum()).into();
        let executed_tx = fvm_exec_result.tx;
        assert!(matches!(
            fvm_exec_result.program_state,
            ProgramState::Revert { .. }
        ));
        assert_eq!(
            executed_tx.inputs()[0].state_root(),
            executed_tx.outputs()[0].state_root()
        );
        assert_eq!(
            executed_tx.inputs()[0].balance_root(),
            executed_tx.outputs()[0].balance_root()
        );
        assert_eq!(executed_tx.inputs()[0].state_root(), Some(&empty_state));
        assert_eq!(executed_tx.inputs()[0].balance_root(), Some(&empty_state));
    }

    #[test]
    fn contracts_balance_and_state_roots_updated_modifications_updated() {
        // Values in inputs and outputs are random. If the execution of the transaction that
        // modifies the state and the balance is successful, it should update roots.
        let mut rng = StdRng::seed_from_u64(2322u64);

        // Create a contract that modifies the state
        let (create, contract_id) = create_contract(
            vec![
                // Sets the state STATE[0x1; 32] = value of `RegId::PC`;
                op::sww(0x1, 0x29, RegId::PC),
                op::ret(1),
            ]
            .into_iter()
            .collect::<Vec<u8>>(),
            &mut rng,
        );

        let transfer_amount = 100 as Word;
        let asset_id = AssetId::from([2; 32]);
        let (script, data_offset) = script_with_data_offset!(
            data_offset,
            vec![
                // Set register `0x10` to `Call`
                op::movi(0x10, data_offset + AssetId::LEN as u32),
                // Set register `0x11` with offset to data that contains `asset_id`
                op::movi(0x11, data_offset),
                // Set register `0x12` with `transfer_amount`
                op::movi(0x12, transfer_amount as u32),
                op::call(0x10, 0x12, 0x11, RegId::CGAS),
                op::ret(RegId::ONE),
            ],
            TxParameters::DEFAULT.tx_offset()
        );

        let script_data: Vec<u8> = [
            asset_id.as_ref(),
            Call::new(contract_id, transfer_amount, data_offset as Word)
                .to_bytes()
                .as_ref(),
        ]
        .into_iter()
        .flatten()
        .copied()
        .collect();

        let modify_balance_and_state_tx = test_builder()
            .script_gas_limit(10000)
            .coin_input(AssetId::zeroed(), 10000)
            .start_script(script, script_data)
            .contract_input(contract_id)
            .coin_input(asset_id, transfer_amount)
            .fee_input()
            .contract_output(&contract_id)
            .build()
            .transaction()
            .clone();
        let mut sdk = journal_state();
        let wasm_storage = WasmStorage { sdk: &mut sdk };
        let mut storage = StructuredStorage::new(wasm_storage);

        let block = PartialFuelBlock {
            header: PartialBlockHeader {
                consensus: ConsensusHeader {
                    height: 1.into(),
                    ..Default::default()
                },
                ..Default::default()
            },
            transactions: vec![
                create.clone().into(),
                modify_balance_and_state_tx.clone().into(),
            ],
        };

        // fluent tests
        let consensus_params = ConsensusParameters::default();
        let coinbase_contract_id = ContractId::default();
        let create_tx = create.as_create().unwrap().clone();
        let create_tx_checked = create_tx
            .into_checked(*block.header.height(), &consensus_params)
            .expect("into_checked successful");
        let mut storage_transaction = storage.write_transaction();
        let execution_data = &mut ExecutionData::new();
        let fvm_exec_result = fvm_transact_commit(
            &mut storage_transaction,
            create_tx_checked,
            &block.header,
            coinbase_contract_id,
            0,
            consensus_params.clone(),
            false,
            execution_data,
        );
        assert_eq!(true, fvm_exec_result.is_ok());

        let script_modify_balance_and_state_tx =
            modify_balance_and_state_tx.as_script().unwrap().clone();
        let script_modify_balance_and_state_tx_checked = script_modify_balance_and_state_tx
            .into_checked(*block.header.height(), &consensus_params)
            .expect("into_checked successful");
        let fvm_exec_result = fvm_transact_commit(
            &mut storage_transaction,
            script_modify_balance_and_state_tx_checked,
            &block.header,
            coinbase_contract_id,
            0,
            consensus_params,
            false,
            execution_data,
        );
        assert_eq!(true, fvm_exec_result.is_ok());
        let fvm_exec_result = fvm_exec_result.unwrap();

        let empty_state = (*sparse::empty_sum()).into();
        let executed_tx = fvm_exec_result.tx;
        assert_eq!(executed_tx.inputs()[0].state_root(), Some(&empty_state));
        assert_eq!(executed_tx.inputs()[0].balance_root(), Some(&empty_state));
        // Roots should be different
        assert_ne!(
            executed_tx.inputs()[0].state_root(),
            executed_tx.outputs()[0].state_root()
        );
        assert_ne!(
            executed_tx.inputs()[0].balance_root(),
            executed_tx.outputs()[0].balance_root()
        );
    }

    #[test]
    fn contracts_balance_and_state_roots_in_inputs_updated() {
        // Values in inputs and outputs are random. If the execution of the transaction that
        // modifies the state and the balance is successful, it should update roots.
        // The first transaction updates the `balance_root` and `state_root`.
        // The second transaction is empty. The executor should update inputs of the second
        // transaction with the same value from `balance_root` and `state_root`.
        let mut rng = StdRng::seed_from_u64(2322u64);

        // Create a contract that modifies the state
        let (create, contract_id) = create_contract(
            vec![
                // Sets the state STATE[0x1; 32] = value of `RegId::PC`;
                op::sww(0x1, 0x29, RegId::PC),
                op::ret(1),
            ]
            .into_iter()
            .collect::<Vec<u8>>(),
            &mut rng,
        );

        let transfer_amount = 100 as Word;
        let asset_id = AssetId::from([2; 32]);
        let (script, data_offset) = script_with_data_offset!(
            data_offset,
            vec![
                // Set register `0x10` to `Call`
                op::movi(0x10, data_offset + AssetId::LEN as u32),
                // Set register `0x11` with offset to data that contains `asset_id`
                op::movi(0x11, data_offset),
                // Set register `0x12` with `transfer_amount`
                op::movi(0x12, transfer_amount as u32),
                op::call(0x10, 0x12, 0x11, RegId::CGAS),
                op::ret(RegId::ONE),
            ],
            TxParameters::DEFAULT.tx_offset()
        );

        let script_data: Vec<u8> = [
            asset_id.as_ref(),
            Call::new(contract_id, transfer_amount, data_offset as Word)
                .to_bytes()
                .as_ref(),
        ]
        .into_iter()
        .flatten()
        .copied()
        .collect();

        let modify_balance_and_state_tx = test_builder()
            .script_gas_limit(10000)
            .coin_input(AssetId::zeroed(), 10000)
            .start_script(script, script_data)
            .contract_input(contract_id)
            .coin_input(asset_id, transfer_amount)
            .fee_input()
            .contract_output(&contract_id)
            .build()
            .transaction()
            .clone();
        let mut db = Database::<OnChain, RegularStage<OnChain>>::default();

        let consensus_parameters = ConsensusParameters::default();

        let block = PartialFuelBlock {
            header: PartialBlockHeader {
                consensus: ConsensusHeader {
                    height: 1.into(),
                    ..Default::default()
                },
                ..Default::default()
            },
            transactions: vec![
                create.clone().into(),
                modify_balance_and_state_tx.clone().into(),
            ],
        };

        // fluent tests
        let consensus_params = ConsensusParameters::default();
        let coinbase_contract_id = ContractId::default();
        let tx1 = create.as_create().unwrap().clone();
        let tx1_checked = tx1
            .into_checked(*block.header.height(), &consensus_params)
            .expect("into_checked successful");
        let mut storage_transaction = db.write_transaction();
        let execution_data = &mut ExecutionData::new();
        let exec_result1 = fvm_transact_commit(
            &mut storage_transaction,
            tx1_checked,
            &block.header,
            coinbase_contract_id,
            0,
            consensus_params.clone(),
            false,
            execution_data,
        );
        assert_eq!(true, exec_result1.is_ok());
        let exec_result1 = exec_result1.unwrap();
        db.commit_changes(exec_result1.changes).unwrap();

        let tx2 = modify_balance_and_state_tx.as_script().unwrap().clone();
        let tx2_checked = tx2
            .into_checked(*block.header.height(), &consensus_params)
            .expect("into_checked successful");
        let mut storage_transaction = db.write_transaction();
        let exec_result2 = fvm_transact_commit(
            &mut storage_transaction,
            tx2_checked,
            &block.header,
            coinbase_contract_id,
            0,
            consensus_params.clone(),
            false,
            execution_data,
        );
        assert_eq!(true, exec_result2.is_ok());
        let exec_result2 = exec_result2.unwrap();
        db.commit_changes(exec_result2.changes).unwrap();
        let executed_tx2 = &exec_result2.tx;
        let state_root = executed_tx2.outputs()[0].state_root();
        let balance_root = executed_tx2.outputs()[0].balance_root();

        let mut new_tx = executed_tx2.clone();
        *new_tx.script_mut() = vec![];
        new_tx.precompute(&consensus_parameters.chain_id()).unwrap();

        let block = PartialFuelBlock {
            header: PartialBlockHeader {
                consensus: ConsensusHeader {
                    height: 2.into(),
                    ..Default::default()
                },
                ..Default::default()
            },
            transactions: vec![new_tx.clone().into()],
        };

        // fluent tests
        let tx1 = new_tx.as_script().unwrap().clone();
        let create_tx_checked = tx1
            .into_checked_basic(*block.header.height(), &consensus_params)
            .expect("into_checked successful");
        let mut storage_transaction = db.write_transaction();
        let execution_data = &mut ExecutionData::new();
        let exec_result1 = fvm_transact_commit(
            &mut storage_transaction,
            create_tx_checked,
            &block.header,
            coinbase_contract_id,
            0,
            consensus_params.clone(),
            false,
            execution_data,
        );
        assert_eq!(true, exec_result1.is_ok());
        let exec_result1 = exec_result1.unwrap();

        let tx = exec_result1.tx;
        assert_eq!(tx.inputs()[0].balance_root(), balance_root);
        assert_eq!(tx.inputs()[0].state_root(), state_root);
    }

    #[test]
    fn contracts_balance_and_state_roots_in_inputs_updated_v2() {
        // Values in inputs and outputs are random. If the execution of the transaction that
        // modifies the state and the balance is successful, it should update roots.
        // The first transaction updates the `balance_root` and `state_root`.
        // The second transaction is empty. The executor should update inputs of the second
        // transaction with the same value from `balance_root` and `state_root`.
        let mut rng = StdRng::seed_from_u64(2322u64);

        // Create a contract that modifies the state
        let (create, contract_id) = create_contract(
            vec![
                // Sets the state STATE[0x1; 32] = value of `RegId::PC`;
                op::sww(0x1, 0x29, RegId::PC),
                op::ret(1),
            ]
            .into_iter()
            .collect::<Vec<u8>>(),
            &mut rng,
        );

        let transfer_amount = 100 as Word;
        let asset_id = AssetId::from([2; 32]);
        let (script, data_offset) = script_with_data_offset!(
            data_offset,
            vec![
                // Set register `0x10` to `Call`
                op::movi(0x10, data_offset + AssetId::LEN as u32),
                // Set register `0x11` with offset to data that contains `asset_id`
                op::movi(0x11, data_offset),
                // Set register `0x12` with `transfer_amount`
                op::movi(0x12, transfer_amount as u32),
                op::call(0x10, 0x12, 0x11, RegId::CGAS),
                op::ret(RegId::ONE),
            ],
            TxParameters::DEFAULT.tx_offset()
        );

        let script_data: Vec<u8> = [
            asset_id.as_ref(),
            Call::new(contract_id, transfer_amount, data_offset as Word)
                .to_bytes()
                .as_ref(),
        ]
        .into_iter()
        .flatten()
        .copied()
        .collect();

        let modify_balance_and_state_tx = test_builder()
            .script_gas_limit(10000)
            .coin_input(AssetId::zeroed(), 10000)
            .start_script(script, script_data)
            .contract_input(contract_id)
            .coin_input(asset_id, transfer_amount)
            .fee_input()
            .contract_output(&contract_id)
            .build()
            .transaction()
            .clone();
        let mut sdk = journal_state();
        let wasm_storage = WasmStorage { sdk: &mut sdk };
        let mut storage = StructuredStorage::new(wasm_storage);

        let consensus_parameters = ConsensusParameters::default();

        let block = PartialFuelBlock {
            header: PartialBlockHeader {
                consensus: ConsensusHeader {
                    height: 1.into(),
                    ..Default::default()
                },
                ..Default::default()
            },
            transactions: vec![
                create.clone().into(),
                modify_balance_and_state_tx.clone().into(),
            ],
        };

        // fluent tests
        let consensus_params = ConsensusParameters::default();
        let coinbase_contract_id = ContractId::default();
        let tx1 = create.as_create().unwrap().clone();
        let tx1_checked = tx1
            .into_checked(*block.header.height(), &consensus_params)
            .expect("into_checked successful");
        let mut storage_transaction = storage.write_transaction();
        let execution_data = &mut ExecutionData::new();
        let exec_result1 = fvm_transact_commit(
            &mut storage_transaction,
            tx1_checked,
            &block.header,
            coinbase_contract_id,
            0,
            consensus_params.clone(),
            false,
            execution_data,
        );
        assert_eq!(true, exec_result1.is_ok());
        let exec_result1 = exec_result1.unwrap();
        storage.commit_changes(exec_result1.changes).unwrap();

        let tx2 = modify_balance_and_state_tx.as_script().unwrap().clone();
        let tx2_checked = tx2
            .into_checked(*block.header.height(), &consensus_params)
            .expect("into_checked successful");
        let mut storage_transaction = storage.write_transaction();
        let execution_data = &mut ExecutionData::new();
        let exec_result2 = fvm_transact_commit(
            &mut storage_transaction,
            tx2_checked,
            &block.header,
            coinbase_contract_id,
            0,
            consensus_params.clone(),
            false,
            execution_data,
        );
        assert_eq!(true, exec_result2.is_ok());
        let exec_result2 = exec_result2.unwrap();
        storage.commit_changes(exec_result2.changes).unwrap();
        let executed_tx2 = &exec_result2.tx;
        let state_root = executed_tx2.outputs()[0].state_root();
        let balance_root = executed_tx2.outputs()[0].balance_root();

        let mut new_tx = executed_tx2.clone();
        *new_tx.script_mut() = vec![];
        new_tx.precompute(&consensus_parameters.chain_id()).unwrap();

        let block = PartialFuelBlock {
            header: PartialBlockHeader {
                consensus: ConsensusHeader {
                    height: 2.into(),
                    ..Default::default()
                },
                ..Default::default()
            },
            transactions: vec![new_tx.clone().into()],
        };

        let tx1 = new_tx.as_script().unwrap().clone();
        let create_tx_checked = tx1
            .into_checked_basic(*block.header.height(), &consensus_params)
            .expect("into_checked successful");
        let mut storage_transaction = storage.write_transaction();
        let execution_data = &mut ExecutionData::new();
        let exec_result1 = fvm_transact_commit(
            &mut storage_transaction,
            create_tx_checked,
            &block.header,
            coinbase_contract_id,
            0,
            consensus_params.clone(),
            false,
            execution_data,
        );
        assert_eq!(true, exec_result1.is_ok());
        let exec_result1 = exec_result1.unwrap();

        let tx = exec_result1.tx;
        assert_eq!(tx.inputs()[0].state_root(), state_root);
        assert_eq!(tx.inputs()[0].balance_root(), balance_root);
    }

    #[test]
    fn foreign_transfer_should_not_affect_balance_root() {
        // The foreign transfer of tokens should not affect the balance root of the transaction.
        let mut rng = StdRng::seed_from_u64(2322u64);

        let (create, contract_id) = create_contract(vec![], &mut rng);

        let transfer_amount = 100 as Word;
        let asset_id = AssetId::from([2; 32]);
        let mut foreign_transfer = test_builder()
            .script_gas_limit(10000)
            .coin_input(AssetId::zeroed(), 10000)
            .start_script(vec![op::ret(1)], vec![])
            .coin_input(asset_id, transfer_amount)
            .coin_output(asset_id, transfer_amount)
            .build()
            .transaction()
            .clone();
        if let Some(Output::Coin { to, .. }) = foreign_transfer
            .as_script_mut()
            .unwrap()
            .outputs_mut()
            .last_mut()
        {
            *to = Address::try_from(contract_id.as_ref()).unwrap();
        } else {
            panic!("Last outputs should be a coin for the contract");
        }
        let mut db = Database::<OnChain, RegularStage<OnChain>>::default();

        let block = PartialFuelBlock {
            header: PartialBlockHeader {
                consensus: ConsensusHeader {
                    height: 1.into(),
                    ..Default::default()
                },
                ..Default::default()
            },
            transactions: vec![create.clone().into(), foreign_transfer.clone().into()],
        };

        // fluent tests
        let consensus_params = ConsensusParameters::default();
        let coinbase_contract_id = ContractId::default();
        let tx1 = create.as_create().unwrap().clone();
        let create_tx_checked = tx1
            .into_checked(*block.header.height(), &consensus_params)
            .expect("into_checked successful");
        let mut storage_transaction = db.write_transaction();
        let execution_data = &mut ExecutionData::new();
        let exec_result1 = fvm_transact_commit(
            &mut storage_transaction,
            create_tx_checked,
            &block.header,
            coinbase_contract_id,
            0,
            consensus_params.clone(),
            false,
            execution_data,
        );
        assert_eq!(true, exec_result1.is_ok());

        let tx2 = foreign_transfer.as_script().unwrap().clone();
        let tx2_checked = tx2
            .into_checked_basic(*block.header.height(), &consensus_params)
            .expect("into_checked successful");
        let exec_result2 = fvm_transact_commit(
            &mut storage_transaction,
            tx2_checked,
            &block.header,
            coinbase_contract_id,
            0,
            consensus_params.clone(),
            false,
            execution_data,
        );
        assert_eq!(true, exec_result2.is_ok());
        let exec_result2 = exec_result2.unwrap();

        db.commit_changes(exec_result2.changes).unwrap();
        let contract_ref = ContractRef::new(db.clone(), contract_id);
        // Assert the balance root should not be affected.
        let empty_state = (*sparse::empty_sum()).into();
        assert_eq!(contract_ref.balance_root().unwrap(), empty_state);
    }

    #[test]
    fn outputs_with_amount_are_included_utxo_set() {
        let (deploy, script) = setup_executable_script();
        let script_id = script.id(&ChainId::default());

        let mut sdk = journal_state();
        let wasm_storage = WasmStorage { sdk: &mut sdk };
        let mut storage = StructuredStorage::new(wasm_storage);

        let block = PartialFuelBlock {
            header: Default::default(),
            transactions: vec![deploy.clone().into(), script.clone().into()],
        };

        // fluent tests
        let consensus_params = ConsensusParameters::default();
        let coinbase_contract_id = ContractId::default();

        let tx1 = deploy.as_create().unwrap().clone();
        let create_tx_checked = tx1
            .into_checked(*block.header.height(), &consensus_params)
            .expect("into_checked successful");
        let mut storage_transaction = storage.write_transaction();
        let execution_data = &mut ExecutionData::new();
        let exec_result1 = fvm_transact_commit(
            &mut storage_transaction,
            create_tx_checked,
            &block.header,
            coinbase_contract_id,
            0,
            consensus_params.clone(),
            false,
            execution_data,
        );
        assert_eq!(true, exec_result1.is_ok());
        let exec_result1 = exec_result1.unwrap();
        storage.commit_changes(exec_result1.changes).unwrap();

        let tx2 = script.as_script().unwrap().clone();
        let tx2_checked = tx2
            .into_checked_basic(*block.header.height(), &consensus_params)
            .expect("into_checked successful");
        let mut storage_transaction = storage.write_transaction();
        let execution_data = &mut ExecutionData::new();
        let exec_result2 = fvm_transact_commit(
            &mut storage_transaction,
            tx2_checked,
            &block.header,
            coinbase_contract_id,
            0,
            consensus_params.clone(),
            false,
            execution_data,
        );
        assert_eq!(true, exec_result2.is_ok());
        let exec_result2 = exec_result2.unwrap();
        storage.commit_changes(exec_result2.changes).unwrap();

        for (idx, output) in exec_result2.tx.outputs().iter().enumerate() {
            let id = UtxoId::new(script_id, idx as u16);
            match output {
                Output::Change { .. } | Output::Variable { .. } | Output::Coin { .. } => {
                    let maybe_utxo = storage.storage::<Coins>().get(&id).unwrap();
                    assert!(maybe_utxo.is_some());
                    let utxo = maybe_utxo.unwrap();
                    assert!(*utxo.amount() > 0)
                }
                _ => (),
            }
        }
    }

    #[test]
    fn outputs_with_no_value_are_excluded_from_utxo_set() {
        let mut rng = StdRng::seed_from_u64(2322);
        let asset_id: AssetId = rng.gen();
        let input_amount = 0;
        let coin_output_amount = 0;

        let tx: Transaction = test_builder()
            .coin_input(asset_id, input_amount)
            .variable_output(Default::default())
            .coin_output(asset_id, coin_output_amount)
            .change_output(asset_id)
            .build()
            .transaction()
            .clone()
            .into();
        let tx_id = tx.id(&ChainId::default());

        let block = PartialFuelBlock {
            header: Default::default(),
            transactions: vec![tx.clone()],
        };

        let mut sdk = journal_state();
        let wasm_storage = WasmStorage { sdk: &mut sdk };
        let mut storage = StructuredStorage::new(wasm_storage);

        // fluent tests
        let consensus_params = ConsensusParameters::default();
        let coinbase_contract_id = ContractId::default();
        let tx1 = tx.as_script().unwrap().clone();
        let create_tx_checked = tx1
            .into_checked(*block.header.height(), &consensus_params)
            .expect("into_checked successful");
        let mut storage_transaction = storage.write_transaction();
        let execution_data = &mut ExecutionData::new();
        let exec_result1 = fvm_transact_commit(
            &mut storage_transaction,
            create_tx_checked,
            &block.header,
            coinbase_contract_id,
            0,
            consensus_params.clone(),
            false,
            execution_data,
        );
        assert_eq!(true, exec_result1.is_ok());
        let exec_result1 = exec_result1.unwrap();
        storage.commit_changes(exec_result1.changes).unwrap();

        for idx in 0..2 {
            let id = UtxoId::new(tx_id, idx);
            let maybe_utxo = storage.storage::<Coins>().get(&id).unwrap();
            assert!(maybe_utxo.is_none());
        }
    }

    // we dont support messages
    // #[test]
    // fn reverted_execution_consume_only_message_coins() {
    //     let mut rng = StdRng::seed_from_u64(2322);
    //     let to: Address = rng.gen();
    //     let amount = 500;
    //
    //     // Script that return `1` - failed script -> execution result will be reverted.
    //     let script = vec![op::ret(1)].into_iter().collect();
    //     let tx = TransactionBuilder::script(script, vec![])
    //         // Add `Input::MessageCoin`
    //         .add_unsigned_message_input(
    //             SecretKey::random(&mut rng),
    //             rng.gen(),
    //             rng.gen(),
    //             amount,
    //             vec![],
    //         )
    //         // Add `Input::MessageData`
    //         .add_unsigned_message_input(
    //             SecretKey::random(&mut rng),
    //             rng.gen(),
    //             rng.gen(),
    //             amount,
    //             vec![0xff; 10],
    //         )
    //         .add_output(Output::change(to, amount + amount, AssetId::BASE))
    //         .finalize();
    //     let tx_id = tx.id(&ChainId::default());
    //
    //     let message_coin = message_from_input(&tx.inputs()[0], 0);
    //     let message_data = message_from_input(&tx.inputs()[1], 0);
    //     let messages = vec![&message_coin, &message_data];
    //
    //     let block = PartialFuelBlock {
    //         header: Default::default(),
    //         transactions: vec![tx.clone().into()],
    //     };
    //
    //     // let mut exec = make_executor(&messages);
    //
    //     let mut db = Database::<OnChain, RegularStage<OnChain>>::default();
    //     // let wasm_storage = WasmStorage {
    //     //     cr: &GuestContextReader::DEFAULT,
    //     //     am: &GuestAccountManager::DEFAULT,
    //     // };
    //     // let mut storage = StructuredStorage::new(wasm_storage);
    //     for message in messages {
    //         db.storage::<Messages>()
    //             .insert(message.id(), message)
    //             .unwrap();
    //     }
    //
    //     let view = db.latest_view().unwrap();
    //     assert!(view.message_exists(message_coin.nonce()).unwrap());
    //     assert!(view.message_exists(message_data.nonce()).unwrap());
    //     let consensus_params = ConsensusParameters::default();
    //     let coinbase_contract_id = ContractId::default();
    //     let tx1 = tx.as_script().unwrap().clone();
    //     let create_tx_checked = tx1
    //         .into_checked(*block.header.height(), &consensus_params)
    //         .expect("into_checked successful");
    //     let mut storage_transaction = db.write_transaction();
    //     let execution_data = &mut ExecutionData::new();
    //     let exec_result1 = fvm_transact_commit(
    //         &mut storage_transaction,
    //         create_tx_checked,
    //         &block.header,
    //         coinbase_contract_id,
    //         0,
    //         consensus_params.clone(),
    //         false,
    //         execution_data,
    //     );
    //     assert_eq!(true, exec_result1.is_ok());
    //     let exec_result1 = exec_result1.unwrap();
    //     db.commit_changes(exec_result1.4).unwrap();
    //
    //     // We should spend only `message_coin`. The `message_data` should be unspent.
    //     let view = db.latest_view().unwrap();
    //     assert!(!view.message_exists(message_coin.nonce()).unwrap());
    //     assert!(view.message_exists(message_data.nonce()).unwrap());
    //     assert_eq!(*view.coin(&UtxoId::new(tx_id, 0)).unwrap().amount(), amount);
    // }

    // we dont support messages
    // #[test]
    // fn message_input_fails_when_mismatches_database() {
    //     let mut rng = StdRng::seed_from_u64(2322);
    //
    //     let (tx, mut message) = make_tx_and_message(&mut rng, 0);
    //
    //     // Modifying the message to make it mismatch
    //     message.set_amount(123);
    //
    //     let block = PartialFuelBlock {
    //         header: Default::default(),
    //         transactions: vec![tx.clone()],
    //     };
    //
    //     // let mut db = Database::<OnChain, RegularStage<OnChain>>::default();
    //     let wasm_storage = WasmStorage {
    //         cr: &GuestContextReader::DEFAULT,
    //         am: &GuestAccountManager::DEFAULT,
    //     };
    //     let mut storage = StructuredStorage::new(wasm_storage);
    //     storage
    //         .storage::<Messages>()
    //         .insert(message.id(), &message)
    //         .unwrap();
    //     let consensus_params = ConsensusParameters::default();
    //     let coinbase_contract_id = ContractId::default();
    //     let tx1 = tx.as_script().unwrap().clone();
    //     let create_tx_checked = tx1
    //         .into_checked(*block.header.height(), &consensus_params)
    //         .expect("into_checked successful");
    //     let mut storage_transaction = storage.write_transaction();
    //     let execution_data = &mut ExecutionData::new();
    //     let exec_result1 = fvm_transact_commit(
    //         &mut storage_transaction,
    //         create_tx_checked,
    //         &block.header,
    //         coinbase_contract_id,
    //         0,
    //         consensus_params.clone(),
    //         true,
    //         execution_data,
    //     );
    //     assert_eq!(true, exec_result1.is_err());
    //     let exec_result1 = exec_result1.err().unwrap();
    //
    //     assert!(matches!(
    //         &exec_result1,
    //         &Error::TransactionValidity(TransactionValidityError::MessageMismatch(_))
    //     ));
    // }
}
