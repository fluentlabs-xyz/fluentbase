use fluentbase_core::fvm::exec::_exec_fuel_tx;
use fluentbase_sdk::{basic_entrypoint, derive::Contract, ExitCode, SharedAPI};

// [TODO:gmm] here is loadable contract
#[derive(Contract)]
pub struct FvmLoaderEntrypoint<SDK> {
    sdk: SDK,
}

impl<SDK: SharedAPI> FvmLoaderEntrypoint<SDK> {
    pub fn deploy(&mut self) {
        self.sdk.exit(ExitCode::Ok.into_i32());
    }

    // [TODO:gmm] entrypoint
    pub fn main(&mut self) {
        let exit_code = self.main_inner();
        self.sdk.exit(exit_code.into_i32());
    }

    // [TODO:gmm] here get transaction and exec...
    pub fn main_inner(&mut self) -> ExitCode {
        let raw_fuel_tx_bytes = self.sdk.input();
        // let fuel_start = self.sdk.fuel();
        let result = _exec_fuel_tx(&mut self.sdk, u64::MAX, raw_fuel_tx_bytes);
        // self.sdk.charge_fuel(fuel_start - result.gas_remaining);
        result.exit_code.into()
    }
}

basic_entrypoint!(FvmLoaderEntrypoint);
