use crate::{fvm::types::WasmStorage, helpers_fvm::fvm_transact_commit};
use alloc::vec::Vec;
use fluentbase_sdk::SovereignAPI;
use fuel_core_executor::executor::ExecutionData;
use fuel_core_storage::transactional::Changes;
use fuel_core_types::{
    blockchain::header::PartialBlockHeader,
    fuel_tx::{Cacheable, ConsensusParameters, ContractId, Receipt, Word},
    fuel_vm::{
        checked_transaction::{Checked, IntoChecked},
        interpreter::{CheckedMetadata, ExecutableTransaction},
        ProgramState,
    },
    services::executor::Result,
};

pub fn _fvm_transact_commit_inner<Tx, SDK: SovereignAPI>(
    sdk: &mut SDK,
    checked_tx: Checked<Tx>,
    header: &PartialBlockHeader,
    coinbase_contract_id: ContractId,
    gas_price: Word,
    consensus_params: ConsensusParameters,
) -> Result<(bool, ProgramState, Tx, Vec<Receipt>, Changes)>
where
    Tx: ExecutableTransaction + Cacheable + Send + Sync + 'static,
    <Tx as IntoChecked>::Metadata: CheckedMetadata + Send + Sync,
{
    // debug_log!(sdk, "ecl(_fvm_transact_inner): start");

    let mut storage = WasmStorage { sdk };

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

    // let mut memory = MemoryInstance::new();
    // let res = fvm_transact(
    //     &mut storage,
    //     checked_tx,
    //     header,
    //     coinbase_contract_id,
    //     gas_price,
    //     &mut memory,
    //     consensus_params,
    //     true,
    // )?;

    let mut execution_data = ExecutionData::new();

    let res = fvm_transact_commit(
        &mut storage,
        checked_tx,
        header,
        coinbase_contract_id,
        gas_price,
        consensus_params,
        true,
        &mut execution_data,
    )?;

    Ok(res)
}
