use crate::debug_log;
use fluentbase_sdk::{types::FvmCreateMethodOutput, AccountManager, ContextReader};
use fuel_core_executor::executor::TxStorageTransaction;
use fuel_core_storage::column::Column;
use fuel_core_types::{
    blockchain::header::PartialBlockHeader,
    fuel_tx::{Cacheable, ConsensusParameters, ContractId, Word},
    fuel_vm::{
        checked_transaction::{Checked, IntoChecked},
        interpreter::{CheckedMetadata, ExecutableTransaction, MemoryInstance},
    },
};

pub fn _fvm_transact<'a, Tx, T, CR: ContextReader, AM: AccountManager>(
    cr: &CR,
    am: &AM,
    checked_tx: Checked<Tx>,
    header: &'a PartialBlockHeader,
    coinbase_contract_id: ContractId,
    gas_price: Word,
    storage_tx: &'a mut TxStorageTransaction<'a, T>,
    memory: &'a mut MemoryInstance,
    consensus_params: ConsensusParameters,
) -> FvmCreateMethodOutput
where
    Tx: ExecutableTransaction + Cacheable,
    <Tx as IntoChecked>::Metadata: CheckedMetadata + Send + Sync,
    T: fuel_core_storage::kv_store::KeyValueInspect<Column = Column>,
{
    debug_log!("ecl(_fvm_create): start");

    // let (reverted, ps, tx, receipts, changes) = fvm_transact(
    //     cr,
    //     am,
    //     checked_tx,
    //     header,
    //     coinbase_contract_id,
    //     gas_price,
    //     storage_tx,
    //     memory,
    //     consensus_params,
    // );

    FvmCreateMethodOutput {}
}
