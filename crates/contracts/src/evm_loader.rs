use fluentbase_core::evm2::exec_evm_bytecode;
use fluentbase_sdk::{basic_entrypoint, derive::Contract, SovereignAPI};

#[derive(Contract)]
pub struct EvmLoaderImpl<SDK> {
    sdk: SDK,
}

impl<SDK: SovereignAPI> EvmLoaderImpl<SDK> {
    pub fn deploy(&mut self) {
        unreachable!("deploy is not supported for loader")
    }

    pub fn main(&mut self) {
        let result = exec_evm_bytecode(&mut self.sdk);

        // if matches!(result.result, return_ok!()) {
        //     self.sdk.commit();
        // } else {
        //     self.sdk.rollback(checkpoint);
        // }
    }
}

basic_entrypoint!(EvmLoaderImpl);
