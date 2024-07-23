use crate::fvm::types::{WasmRelayer, WasmStorage};
use alloc::vec::Vec;
use fluentbase_sdk::{AccountManager, ContextReader};
use fuel_core_executor::executor::{BlockExecutor, ExecutionOptions, TxStorageTransaction};
use fuel_core_storage::{
    column::Column,
    kv_store::KeyValueInspect,
    structured_storage::StructuredStorage,
    transactional::{Changes, ConflictPolicy, InMemoryTransaction, IntoTransaction},
};
use fuel_core_types::{
    blockchain::header::PartialBlockHeader,
    fuel_tx::{Cacheable, ConsensusParameters, ContractId, Receipt, Word},
    fuel_vm::{
        checked_transaction::{Checked, IntoChecked},
        interpreter::{CheckedMetadata, ExecutableTransaction, MemoryInstance},
        ProgramState,
    },
    services::executor::Result as ExecutorResult,
};

pub fn fvm_transact<'a, Tx, T, CR: ContextReader, AM: AccountManager>(
    cr: &CR,
    am: &AM,
    checked_tx: Checked<Tx>,
    header: &'a PartialBlockHeader,
    coinbase_contract_id: ContractId,
    gas_price: Word,
    memory: &'a mut MemoryInstance,
    consensus_params: ConsensusParameters,
) -> ExecutorResult<(bool, ProgramState, Tx, Vec<Receipt>)>
where
    Tx: ExecutableTransaction + Cacheable,
    <Tx as IntoChecked>::Metadata: CheckedMetadata + Send + Sync,
    T: KeyValueInspect<Column = Column>,
{
    let execution_options = ExecutionOptions {
        extra_tx_checks: true,
        ..Default::default()
    };

    let mut block_executor =
        BlockExecutor::new(WasmRelayer {}, execution_options.clone(), consensus_params)
            .expect("failed to create block executor");

    let storage = WasmStorage { cr, am };
    let structured_storage = StructuredStorage::new(storage);
    let mut structured_storage = structured_storage.into_transaction();
    let in_memory_transaction = InMemoryTransaction::new(
        Changes::new(),
        ConflictPolicy::Overwrite,
        &mut structured_storage,
    );
    // let mut b = BlockStorageTransaction::default();
    // let instance = ExecutionInstance {
    //     relayer: WasmRelayer {},
    //     database: TxStorageTransaction::new(InMemoryTransaction::new(
    //         Changes::new(),
    //         ConflictPolicy::Overwrite,
    //         &mut b,
    //     )),
    //     options: execution_options,
    // };
    // let (block_executor, mut storage_transaction) = instance
    //     .into_executor(consensus_params_version)
    //     .expect("failed to derive executor");
    // let mut storage_transaction = storage_transaction.into_transaction();
    let mut storage_transaction = TxStorageTransaction::new(in_memory_transaction);
    let exec_result = block_executor.attempt_tx_execution_with_vm(
        checked_tx,
        header,
        coinbase_contract_id,
        gas_price,
        &mut storage_transaction,
        memory,
    );
    exec_result
}

// pub struct BlockExecutorCtx {
//     pub options: ExecutionOptions,
// }
//
// impl BlockExecutorCtx {
//     #[allow(clippy::too_many_arguments)]
//     pub fn get_coin_or_default<T>(
//         &self,
//         db: &TxStorageTransaction<T>,
//         utxo_id: UtxoId,
//         owner: fuel_core_types::fuel_types::Address,
//         amount: u64,
//         asset_id: AssetId,
//     ) -> ExecutorResult<CompressedCoin>
//     where
//         T: KeyValueInspect<Column = Column>,
//     {
//         if self.options.extra_tx_checks {
//             db.storage::<Coins>()
//                 .get(&utxo_id)?
//                 .ok_or(ExecutorError::TransactionValidity(
//                     TransactionValidityError::CoinDoesNotExist(utxo_id),
//                 ))
//                 .map(Cow::into_owned)
//         } else {
//             // if utxo validation is disabled, just assign this new input to the original block
//             let coin = CompressedCoinV1 {
//                 owner,
//                 amount,
//                 asset_id,
//                 tx_pointer: Default::default(),
//             }
//             .into();
//             Ok(coin)
//         }
//     }
//
//     pub fn compute_inputs<T>(
//         &self,
//         inputs: &mut [Input],
//         db: &TxStorageTransaction<T>,
//     ) -> ExecutorResult<()>
//     where
//         T: KeyValueInspect<Column = Column>,
//     {
//         for input in inputs {
//             match input {
//                 Input::CoinSigned(CoinSigned {
//                     tx_pointer,
//                     utxo_id,
//                     owner,
//                     amount,
//                     asset_id,
//                     ..
//                 })
//                 | Input::CoinPredicate(fuel_tx::input::coin::CoinPredicate {
//                     tx_pointer,
//                     utxo_id,
//                     owner,
//                     amount,
//                     asset_id,
//                     ..
//                 }) => {
//                     let coin =
//                         self.get_coin_or_default(db, *utxo_id, *owner, *amount, *asset_id)?;
//                     *tx_pointer = *coin.tx_pointer();
//                 }
//                 Input::Contract(fuel_tx::input::contract::Contract {
//                     ref mut utxo_id,
//                     ref mut balance_root,
//                     ref mut state_root,
//                     ref mut tx_pointer,
//                     ref contract_id,
//                     ..
//                 }) => {
//                     let contract = ContractRef::new(StructuredStorage::new(db), *contract_id);
//                     let utxo_info = contract.validated_utxo(self.options.extra_tx_checks)?;
//                     *utxo_id = *utxo_info.utxo_id();
//                     *tx_pointer = utxo_info.tx_pointer();
//                     *balance_root = contract.balance_root()?;
//                     *state_root = contract.state_root()?;
//                 }
//                 _ => {}
//             }
//         }
//         Ok(())
//     }
//
//     pub fn update_tx_outputs<Tx, T>(
//         &self,
//         storage_tx: &TxStorageTransaction<T>,
//         tx_id: TxId,
//         tx: &mut Tx,
//     ) -> ExecutorResult<()>
//     where
//         Tx: ExecutableTransaction,
//         T: KeyValueInspect<Column = Column>,
//     {
//         let mut outputs = core::mem::take(tx.outputs_mut());
//         self.compute_state_of_not_utxo_outputs(&mut outputs, tx.inputs(), tx_id, storage_tx)?;
//         *tx.outputs_mut() = outputs;
//         Ok(())
//     }
//     #[allow(clippy::type_complexity)]
//     // TODO: Maybe we need move it to `fuel-vm`? O_o Because other `Outputs` are processed there
//     /// Computes all zeroed or variable outputs.
//     /// In production mode, updates the outputs with computed values.
//     /// In validation mode, compares the outputs with computed inputs.
//     fn compute_state_of_not_utxo_outputs<T>(
//         &self,
//         outputs: &mut [Output],
//         inputs: &[Input],
//         tx_id: TxId,
//         db: &TxStorageTransaction<T>,
//     ) -> ExecutorResult<()>
//     where
//         T: KeyValueInspect<Column = Column>,
//     {
//         for output in outputs {
//             if let Output::Contract(contract_output) = output {
//                 let contract_id = if let Some(Input::Contract(input::contract::Contract {
//                     contract_id,
//                     ..
//                 })) = inputs.get(contract_output.input_index as usize)
//                 {
//                     contract_id
//                 } else {
//                     return Err(ExecutorError::InvalidTransactionOutcome {
//                         transaction_id: tx_id,
//                     });
//                 };
//
//                 let contract = ContractRef::new(StructuredStorage::new(db), *contract_id);
//                 contract_output.balance_root = contract.balance_root()?;
//                 contract_output.state_root = contract.state_root()?;
//             }
//         }
//         Ok(())
//     }
// }
