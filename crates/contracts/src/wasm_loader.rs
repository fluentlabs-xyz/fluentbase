use fluentbase_core::wasm::call::_wasm_call;
use fluentbase_sdk::{
    alloc_slice,
    basic_entrypoint,
    derive::Contract,
    types::WasmCallMethodInput,
    AccountManager,
    ContextReader,
    SovereignAPI,
};
use revm_precompile::Bytes;

#[derive(Contract)]
pub struct WasmLoaderImpl<'a, CR: ContextReader, AM: AccountManager> {
    cr: &'a CR,
    am: &'a AM,
}

impl<'a, CR: ContextReader, AM: AccountManager> WasmLoaderImpl<'a, CR, AM> {
    pub fn deploy<SDK: SovereignAPI>(&self) {
        unreachable!("deploy is not supported for loader")
    }
    pub fn main<SDK: SovereignAPI>(&self) {
        let input_size = SDK::input_size();
        let input = alloc_slice(input_size as usize);
        SDK::read(input.as_mut_ptr(), input_size, 0);
        let input = WasmCallMethodInput {
            callee: self.cr.contract_address(),
            value: self.cr.contract_value(),
            input: Bytes::copy_from_slice(input),
            gas_limit: self.cr.contract_gas_limit(),
            depth: 0,
        };
        let output = _wasm_call(self.cr, self.am, input);
        SDK::write(output.output.as_ptr(), output.output.len() as u32);
        SDK::exit(output.exit_code);
    }
}

basic_entrypoint!(
    WasmLoaderImpl<
        'static,
        fluentbase_sdk::GuestContextReader,
        fluentbase_sdk::GuestAccountManager,
    >
);
