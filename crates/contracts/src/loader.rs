use fluentbase_sdk::{
    alloc_slice,
    basic_entrypoint,
    contracts::{EvmAPI, EvmClient, PRECOMPILE_EVM},
    derive::Contract,
    types::EvmCallMethodInput,
    AccountManager,
    ContextReader,
    SharedAPI,
};
use revm_precompile::Bytes;

#[derive(Contract)]
pub struct LOADER<'a, CR: ContextReader, AM: AccountManager> {
    cr: &'a CR,
    am: &'a AM,
}

impl<'a, CR: ContextReader, AM: AccountManager> LOADER<'a, CR, AM> {
    pub fn deploy<SDK: SharedAPI>(&self) {
        unreachable!("deploy is not supported for loader")
    }
    pub fn main<SDK: SharedAPI>(&self) {
        let input_size = SDK::input_size();
        let input = alloc_slice(input_size as usize);
        SDK::read(input.as_mut_ptr(), input_size, 0);
        let evm_client = EvmClient::new(PRECOMPILE_EVM);
        let output = evm_client.call(EvmCallMethodInput {
            callee: self.cr.contract_address(),
            value: self.cr.contract_value(),
            input: Bytes::copy_from_slice(input),
            gas_limit: self.cr.contract_gas_limit(),
            depth: 0,
        });
        SDK::write(output.output.as_ptr(), output.output.len() as u32);
        SDK::exit(output.exit_code);
    }
}

basic_entrypoint!(
    LOADER<'static, fluentbase_sdk::GuestContextReader, fluentbase_sdk::GuestAccountManager>
);
