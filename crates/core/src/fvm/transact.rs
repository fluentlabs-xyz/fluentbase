use crate::{
    fvm::types::WasmStorage,
    helpers_fvm::{fvm_transact_commit, FvmTransactResult},
};
use fluentbase_sdk::SharedAPI;
use fuel_core_executor::executor::ExecutionData;
use fuel_core_types::{
    blockchain::header::PartialBlockHeader,
    fuel_tx::{Cacheable, ConsensusParameters, ContractId, Word},
    fuel_vm::{
        checked_transaction::{Checked, IntoChecked},
        interpreter::{CheckedMetadata, ExecutableTransaction},
    },
    services::executor::Result,
};

pub fn _fvm_transact_commit_inner<Tx, SDK: SharedAPI>(
    sdk: &mut SDK,
    checked_tx: Checked<Tx>,
    header: &PartialBlockHeader,
    coinbase_contract_id: ContractId,
    gas_price: Word,
    consensus_params: ConsensusParameters,
    execution_data: &mut ExecutionData,
) -> Result<FvmTransactResult<Tx>>
where
    Tx: ExecutableTransaction + Cacheable + Send + Sync + 'static,
    <Tx as IntoChecked>::Metadata: CheckedMetadata + Send + Sync,
{
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

    let result = fvm_transact_commit(
        &mut storage,
        checked_tx,
        header,
        coinbase_contract_id,
        gas_price,
        consensus_params,
        true,
        execution_data,
    )?;

    Ok(result)
}
