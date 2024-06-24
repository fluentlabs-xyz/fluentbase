use fluentbase_core::evm::create::_evm_create;
use fluentbase_sdk::{
    alloc_slice,
    basic_entrypoint,
    derive::Contract,
    types::EvmCreateMethodInput,
    AccountManager,
    Bytes,
    ContextReader,
    SovereignAPI,
};

#[derive(Contract)]
pub struct EvmDeployerImpl<'a, CR: ContextReader, AM: AccountManager> {
    cr: &'a CR,
    am: &'a AM,
}

impl<'a, CR: ContextReader, AM: AccountManager> EvmDeployerImpl<'a, CR, AM> {
    pub fn deploy<SDK: SovereignAPI>(&self) {
        unreachable!("deploy is not supported for loader")
    }
    pub fn main<SDK: SovereignAPI>(&self) {
        let input_size = SDK::input_size();
        let input = alloc_slice(input_size as usize);
        SDK::read(input.as_mut_ptr(), input_size, 0);
        let input = EvmCreateMethodInput {
            bytecode: Bytes::copy_from_slice(input),
            value: self.cr.contract_value(),
            gas_limit: self.cr.contract_gas_limit(),
            salt: None,
            depth: 0,
        };
        let output = _evm_create(self.cr, self.am, input);
        SDK::write(output.output.as_ptr(), output.output.len() as u32);
        SDK::exit(output.exit_code);
    }
}

basic_entrypoint!(
    EvmDeployerImpl<
        'static,
        fluentbase_sdk::GuestContextReader,
        fluentbase_sdk::GuestAccountManager,
    >
);
