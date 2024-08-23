use crate::fvm::types::WasmRelayer;
use alloc::vec::Vec;
use fuel_core_executor::executor::{
    BlockExecutor,
    ExecutionData,
    ExecutionOptions,
    TxStorageTransaction,
};
use fuel_core_storage::{
    column::Column,
    kv_store::{KeyValueInspect, KeyValueMutate, WriteOperation},
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
    services::executor::Result,
};

#[derive(Debug, Clone)]
pub struct FvmTransactResult<Tx> {
    pub reverted: bool,
    pub program_state: ProgramState,
    pub tx: Tx,
    pub receipts: Vec<Receipt>,
    pub changes: Changes,
}

pub fn fvm_transact<'a, Tx, T>(
    storage: &mut T,
    checked_tx: Checked<Tx>,
    header: &'a PartialBlockHeader,
    coinbase_contract_id: ContractId,
    gas_price: Word,
    memory: &'a mut MemoryInstance,
    consensus_params: ConsensusParameters,
    extra_tx_checks: bool,
    execution_data: &mut ExecutionData,
) -> Result<FvmTransactResult<Tx>>
where
    Tx: ExecutableTransaction + Cacheable + Send + Sync + 'static,
    <Tx as IntoChecked>::Metadata: CheckedMetadata + Send + Sync,
    T: KeyValueInspect<Column = Column>,
{
    let execution_options = ExecutionOptions {
        extra_tx_checks,
        backtrace: false,
    };

    let block_executor =
        BlockExecutor::new(WasmRelayer {}, execution_options.clone(), consensus_params)
            .expect("failed to create block executor");

    let structured_storage = StructuredStorage::new(storage);
    let mut structured_storage = structured_storage.into_transaction();
    let in_memory_transaction = InMemoryTransaction::new(
        Changes::new(),
        ConflictPolicy::Overwrite,
        &mut structured_storage,
    );
    let tx_transaction = &mut TxStorageTransaction::new(in_memory_transaction);

    let tx_id = checked_tx.id();

    let mut checked_tx = checked_tx;
    if execution_options.extra_tx_checks {
        checked_tx = block_executor.extra_tx_checks(checked_tx, header, tx_transaction, memory)?;
    }

    let (reverted, program_state, tx, receipts) = block_executor.attempt_tx_execution_with_vm(
        checked_tx,
        header,
        coinbase_contract_id,
        gas_price,
        tx_transaction,
        memory,
    )?;

    block_executor.spend_input_utxos(tx.inputs(), tx_transaction, reverted, execution_data)?;

    block_executor.persist_output_utxos(
        *header.height(),
        execution_data,
        &tx_id,
        tx_transaction,
        tx.inputs(),
        tx.outputs(),
    )?;

    // tx_st_transaction
    //     .storage::<ProcessedTransactions>()
    //     .insert(&tx_id, &());

    block_executor.update_execution_data(
        &tx,
        execution_data,
        receipts.clone(),
        gas_price,
        reverted,
        program_state,
        tx_id,
    )?;

    Ok(FvmTransactResult {
        reverted,
        program_state,
        tx,
        receipts,
        changes: tx_transaction.changes().clone(),
    })
}

pub fn fvm_transact_commit<Tx, T>(
    storage: &mut T,
    checked_tx: Checked<Tx>,
    header: &PartialBlockHeader,
    coinbase_contract_id: ContractId,
    gas_price: Word,
    consensus_params: ConsensusParameters,
    extra_tx_checks: bool,
    execution_data: &mut ExecutionData,
) -> Result<FvmTransactResult<Tx>>
where
    Tx: ExecutableTransaction + Cacheable + Send + Sync + 'static,
    <Tx as IntoChecked>::Metadata: CheckedMetadata + Send + Sync,
    T: KeyValueMutate<Column = Column>,
{
    // debug_log!("ecl(fvm_transact_commit): start");

    // TODO warmup storage from state based on tx inputs?
    // let inputs = checked_tx.transaction().inputs();
    // for input in inputs {
    //     match input {
    //         Input::CoinSigned(v) => {}
    //         Input::CoinPredicate(v) => {}
    //         Input::Contract(v) => {}
    //         Input::MessageCoinSigned(v) => {}
    //         Input::MessageCoinPredicate(v) => {}
    //         Input::MessageDataSigned(v) => {}
    //         Input::MessageDataPredicate(v) => {}
    //     }
    // }

    let mut memory = MemoryInstance::new();

    let result = fvm_transact(
        storage,
        checked_tx,
        header,
        coinbase_contract_id,
        gas_price,
        &mut memory,
        consensus_params,
        extra_tx_checks,
        execution_data,
    )?;

    for (col_num, changes) in &result.changes {
        let column: Column = col_num.clone().try_into().expect("valid column number");
        match column {
            Column::Metadata
            | Column::ContractsRawCode
            | Column::ContractsState
            | Column::ContractsLatestUtxo
            | Column::ContractsAssets
            | Column::ContractsAssetsMerkleData
            | Column::ContractsAssetsMerkleMetadata
            | Column::ContractsStateMerkleData
            | Column::ContractsStateMerkleMetadata
            | Column::Coins => {
                for (key, op) in changes {
                    match op {
                        WriteOperation::Insert(v) => {
                            storage.write(key, column, v)?;
                        }
                        WriteOperation::Remove => {
                            storage.delete(key, column)?;
                        }
                    }
                }
            }

            Column::Transactions
            | Column::FuelBlocks
            | Column::FuelBlockMerkleData
            | Column::FuelBlockMerkleMetadata
            | Column::Messages
            | Column::ProcessedTransactions
            | Column::FuelBlockConsensus
            | Column::ConsensusParametersVersions
            | Column::StateTransitionBytecodeVersions
            | Column::UploadedBytecodes
            | Column::GenesisMetadata => {
                panic!("unsupported column {:?} operation", column)
            }
        }
    }

    Ok(result)
}
