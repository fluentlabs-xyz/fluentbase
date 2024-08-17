use fluentbase_core::{evm::EvmLoader, helpers::exit_code_from_evm_error};
use fluentbase_sdk::{basic_entrypoint, derive::Contract, SovereignAPI};

#[derive(Contract)]
pub struct EvmLoaderEntrypoint<SDK> {
    sdk: SDK,
}

impl<SDK: SovereignAPI> EvmLoaderEntrypoint<SDK> {
    pub fn deploy(&mut self) {
        unreachable!("deploy is not allowed for genesis contract")
    }

    pub fn main(&mut self) {
        let (caller, address, value, input) = self
            .sdk
            .contract_context()
            .map(|v| (v.caller, v.address, v.value, v.input.clone()))
            .unwrap();
        let gas_limit = self.sdk.native_sdk().fuel();
        let result = EvmLoader::new(&mut self.sdk).call(caller, address, value, input, gas_limit);
        self.sdk.native_sdk().write(result.output.as_ref());
        let exit_code = exit_code_from_evm_error(result.result);
        self.sdk.native_sdk().exit(exit_code.into_i32());
    }
}

basic_entrypoint!(EvmLoaderEntrypoint);
