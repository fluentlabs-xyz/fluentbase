use fluentbase_sdk::{basic_entrypoint, derive::Contract, SovereignAPI};

#[derive(Contract)]
pub struct EvmDeployerImpl<SDK> {
    sdk: SDK,
}

impl<SDK: SovereignAPI> EvmDeployerImpl<SDK> {
    pub fn deploy(&self) {
        unreachable!("deploy is not supported for loader")
    }
    pub fn main(&self) {
        // let input_size = self.sdk.input_size();
        // let input = alloc_slice(input_size as usize);
        // self.sdk.read(input, 0);
        // let input = EvmCreateMethodInput {
        //     bytecode: Bytes::copy_from_slice(input),
        //     value: self.ctx.contract_value(),
        //     gas_limit: self.ctx.contract_gas_limit(),
        //     salt: None,
        //     depth: 0,
        // };
        // let output = _evm_create(&self.ctx, &self.sdk, input);
        // self.sdk.write(&output.output);
        // self.sdk.exit(output.exit_code);
    }
}

basic_entrypoint!(EvmDeployerImpl);
