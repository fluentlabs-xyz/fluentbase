use crate::fvm::types::WasmRelayer;
use alloc::vec::Vec;
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
    services::executor::Result,
};

pub fn fvm_transact<'a, Tx, T>(
    storage: &mut T,
    checked_tx: Checked<Tx>,
    header: &'a PartialBlockHeader,
    coinbase_contract_id: ContractId,
    gas_price: Word,
    memory: &'a mut MemoryInstance,
    consensus_params: ConsensusParameters,
) -> Result<(bool, ProgramState, Tx, Vec<Receipt>, Changes)>
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

    let structured_storage = StructuredStorage::new(storage);
    let mut structured_storage = structured_storage.into_transaction();
    let in_memory_transaction = InMemoryTransaction::new(
        Changes::new(),
        ConflictPolicy::Overwrite,
        &mut structured_storage,
    );
    let mut storage_transaction = TxStorageTransaction::new(in_memory_transaction);
    let exec_result = block_executor.attempt_tx_execution_with_vm(
        checked_tx,
        header,
        coinbase_contract_id,
        gas_price,
        &mut storage_transaction,
        memory,
    )?;
    Ok((
        exec_result.0,
        exec_result.1,
        exec_result.2,
        exec_result.3,
        storage_transaction.changes().clone(),
    ))
}
