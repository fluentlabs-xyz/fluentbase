use fluentbase_core::evm::EvmRuntime;
use fluentbase_sdk::{basic_entrypoint, derive::Contract, ExitCode, SovereignAPI};
use revm_interpreter::Gas;

#[derive(Contract)]
pub struct EvmLoaderImpl<SDK> {
    sdk: SDK,
}

impl<SDK: SovereignAPI> EvmLoaderImpl<SDK> {
    pub fn deploy(&mut self) -> ExitCode {
        let contract_context = self.sdk.contract_context().cloned().unwrap();

        // execute EVM constructor to produce final EVM bytecode
        // let mut evm_runtime = EvmRuntime::new(&mut self.sdk);
        // let mut gas = Gas::new(contract_context.gas_limit);
        // evm_runtime.deploy_evm_contract(contract_context, &mut gas)

        ExitCode::Ok
    }

    pub fn main(&mut self) -> ExitCode {
        ExitCode::Ok
    }
}

basic_entrypoint!(EvmLoaderImpl);
