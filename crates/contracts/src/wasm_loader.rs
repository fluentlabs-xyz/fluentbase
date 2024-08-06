use fluentbase_sdk::{basic_entrypoint, derive::Contract, SovereignAPI};

#[derive(Contract)]
pub struct WasmLoaderImpl<SDK> {
    sdk: SDK,
}

impl<SDK: SovereignAPI> WasmLoaderImpl<SDK> {
    pub fn deploy(&self) {
        unreachable!("deploy is not supported for loader")
    }
    pub fn main(&self) {
        // let input_size = self.sdk.input_size();
        // let input = alloc_slice(input_size as usize);
        // self.sdk.read(input, 0);
        // let input = WasmCallMethodInput {
        //     bytecode_address: self.ctx.contract_address(),
        //     value: self.ctx.contract_value(),
        //     input: Bytes::copy_from_slice(input),
        //     gas_limit: self.ctx.contract_gas_limit(),
        //     depth: 0,
        // };
        // let output = _wasm_call(&self.ctx, &self.sdk, input);
        // self.sdk.write(&output.output);
        // self.sdk.exit(output.exit_code);
    }
}

basic_entrypoint!(WasmLoaderImpl);
