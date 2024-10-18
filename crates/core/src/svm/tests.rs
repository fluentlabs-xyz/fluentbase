#[cfg(test)]
mod solana_tests {
    use super::*;
    use crate::helpers_svm::{svm_transact_commit, SvmTransactResult};
    use fuel_core_storage::{
        transactional::{Modifiable, WriteTransaction},
        StorageAsRef,
    };
    use fuel_core_types::{
        fuel_tx::field::Inputs,
        fuel_vm::{checked_transaction::IntoChecked, interpreter::ExecutableTransaction},
    };
    use std::error::Error;
    // use solana_program_test::{BanksClient, ProgramTest};
    // use solana_sdk::{
    //     account::Account,
    //     pubkey::Pubkey,
    //     signature::{Keypair, Signer},
    //     transaction::Transaction,
    // };
    // use fluentbase_sdk::{BlockchainState, execute_solana_program};

    // The function that prepares the test environment and client
    // to interact with the Solana program
    // fn setup() -> (BanksClient, Keypair, Pubkey) {
    //     // let program_test = ProgramTest::new(
    //     //     "test_solana_program",
    //     //     test_solana_program::id(),
    //     //     processor!(processor_function));
    //     // let (banks_client, payer, recent_blockhash) = program_test.start();
    //     // let program_id = test_solana_program::id();
    //     //
    //     // (banks_client, payer, program_id)
    // }

    #[test]
    fn test_solana_program_execution_changes_state() {
        // let (mut banks_client, payer, program_id) = setup();
        // // build Transaction with Instructions
        // let mut tx = Transaction::new_with_payer(
        //     &[test_solana_program::instruction::instruction_method(
        //         &program_id,
        //         &payer.pubkey(),
        //         // ...
        //     )],
        //     Some(&payer.pubkey()),
        // );
        //
        // // Sign & send
        // tx.sign(&[&payer], recent_blockhash);
        // banks_client.process_transaction(transaction).unwrap();
        //
        // // Check blockchain state
        // let blockchain_state = BlockchainState::new();
        // let state_changed = execute_solana_program(&blockchain_state, &program_id);
        // assert!(state_changed, "State should be changed after executing the Solana program");

        let mut sdk = crate::fvm::tests::tests::journal_state();
        let wasm_storage = WasmStorage { sdk: &mut sdk };
        let mut storage = StructuredStorage::new(wasm_storage);
        // let mut tb = || TestBuilder::new(2322u64);
        // let mut db = GenericDatabase::from_storage(storage);
        // `tx2` has two inputs: one used by `tx1` and on random. So after the execution of `tx1`,
        // the `tx2` become invalid and should be skipped by the block producers. Skipped
        // transactions should not affect the state so the second input should be `Unspent`.
        // # Dev-note: `TxBuilder::new(2322u64)` is used to create transactions, it produces
        // the same first input.
        let tx1 = crate::fvm::tests::tests::test_builder()
            .coin_input(AssetId::default(), 100)
            .change_output(AssetId::default())
            .build()
            .transaction()
            .clone();

        let tx2 = crate::fvm::tests::tests::test_builder()
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
        <StructuredStorage<WasmStorage<'_,
            crate::fvm::tests::tests::TestingSDK>> as StorageMutate<Coins>>::insert(
            &mut storage,
            &first_input.utxo_id().unwrap().clone(),
            &first_coin,
        )
            .expect("insert first utxo success");
        <StructuredStorage<WasmStorage<'_,
            crate::fvm::tests::tests::TestingSDK>> as StorageMutate<Coins>>::insert(
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
        <StructuredStorage<WasmStorage<'_,
            crate::fvm::tests::tests::TestingSDK>> as StorageInspect<Coins>>::get(
            &storage,
            first_input.utxo_id().unwrap(),
        )
            .unwrap()
            .expect("coin should be unspent");
        // The second input should be `Unspent` before execution.
        <StructuredStorage<WasmStorage<'_,
            crate::fvm::tests::tests::TestingSDK>> as StorageInspect<Coins>>::get(
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
        let exec_result1 = svm_transact_commit(
            &mut storage_transaction,
            // create_tx_checked,
            // &block.header,
            // coinbase_contract_id,
            // 0,
            // consensus_params.clone(),
            // true,
            // execution_data,
        );
        assert_eq!(true, exec_result1.is_ok());
        // let exec_result1 = exec_result1.unwrap();
        // storage.commit_changes(exec_result1.changes).unwrap();

        let tx2 = tx2.as_script().unwrap().clone();
        let tx2_checked = tx2
            .into_checked(*block.header.height(), &consensus_params)
            .expect("into_checked successful");
        let mut storage_transaction = storage.write_transaction();
        let execution_data = &mut ExecutionData::new();
        let exec_result2 = svm_transact_commit(
            &mut storage_transaction,
            // tx2_checked,
            // &block.header,
            // coinbase_contract_id,
            // 0,
            // consensus_params.clone(),
            // true,
            // execution_data,
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
}
