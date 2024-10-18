use fluentbase_sdk::{
    derive::derive_keccak256,
    types::FvmMethodOutput,
    Bytes,
    Bytes32,
    ExitCode,
    SharedAPI,
    B256,
};

pub fn _exec_fuel_tx<SDK: SharedAPI>(
    sdk: &mut SDK,
    gas_limit: u64,
    raw_tx_bytes: Bytes,
) -> FvmMethodOutput {
    FvmMethodOutput {
        output: Default::default(),
        exit_code: ExitCode::Ok.into_i32(),
        gas_remaining: gas_limit,
        gas_refund: 0,
    }
}

//--> let fvm_exec_result = fvm_transact ...
