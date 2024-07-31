use fluentbase_core::evm::call::_evm_call;
use fluentbase_sdk::{
    alloc_slice,
    basic_entrypoint,
    derive::Contract,
    types::EvmCallMethodInput,
    ContextReader,
    SovereignAPI,
};
use revm_precompile::Bytes;

#[derive(Contract)]
pub struct EvmLoaderImpl<CTX: ContextReader, SDK: SovereignAPI> {
    ctx: CTX,
    sdk: SDK,
}

impl<CTX: ContextReader, SDK: SovereignAPI> EvmLoaderImpl<CTX, SDK> {
    pub fn deploy(&self) {
        unreachable!("deploy is not supported for loader")
    }
    pub fn main(&self) {
        let input_size = self.sdk.input_size();
        let input = alloc_slice(input_size as usize);
        self.sdk.read(input, 0);
        let input = EvmCallMethodInput {
            bytecode_address: self.ctx.contract_address(),
            value: self.ctx.contract_value(),
            input: Bytes::copy_from_slice(input),
            gas_limit: self.ctx.contract_gas_limit(),
            depth: 0,
        };
        let output = _evm_call(&self.ctx, &self.sdk, input);
        self.sdk.write(&output.output);
        self.sdk.exit(output.exit_code);
    }
}

basic_entrypoint!(EvmLoaderImpl);
