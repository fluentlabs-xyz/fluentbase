use fluentbase_core::evm::call::_evm_call;
use fluentbase_sdk::{
    alloc_slice,
    basic_entrypoint,
    derive::Contract,
    types::EvmCallMethodInput,
    AccountManager,
    ContextReader,
    SovereignAPI,
};
use revm_precompile::Bytes;

#[derive(Contract)]
pub struct EvmLoaderImpl<'a, CR: ContextReader, AM: AccountManager> {
    cr: &'a CR,
    am: &'a AM,
}

impl<'a, CR: ContextReader, AM: AccountManager> EvmLoaderImpl<'a, CR, AM> {
    pub fn deploy<SDK: SovereignAPI>(&self) {
        unreachable!("deploy is not supported for loader")
    }
    pub fn main<SDK: SovereignAPI>(&self) {
        let input_size = SDK::input_size();
        let input = alloc_slice(input_size as usize);
        SDK::read(input.as_mut_ptr(), input_size, 0);
        let input = EvmCallMethodInput {
            callee: self.cr.contract_address(),
            value: self.cr.contract_value(),
            input: Bytes::copy_from_slice(input),
            gas_limit: self.cr.contract_gas_limit(),
            depth: 0,
        };
        let output = _evm_call(self.cr, self.am, input);
        SDK::write(output.output.as_ptr(), output.output.len() as u32);
        SDK::exit(output.exit_code);
    }
}

basic_entrypoint!(
    EvmLoaderImpl<'static, fluentbase_sdk::GuestContextReader, fluentbase_sdk::GuestAccountManager>
);
