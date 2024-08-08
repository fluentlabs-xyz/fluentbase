use fluentbase_core::evm::EvmBytecodeExecutor;
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
        // let result = EvmBytecodeExecutor::new(&mut self.sdk).call();

        // if matches!(result.result, return_ok!()) {
        //     self.sdk.commit();
        // } else {
        //     self.sdk.rollback(checkpoint);
        // }
    }
}

basic_entrypoint!(EvmLoaderImpl);
