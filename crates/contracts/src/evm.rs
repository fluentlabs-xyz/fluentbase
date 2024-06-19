use fluentbase_core::evm::{call::_evm_call, create::_evm_create};
use fluentbase_sdk::{
    basic_entrypoint,
    codec::Encoder,
    contracts::{EvmAPI, EvmSloadInput, EvmSloadOutput, EvmSstoreInput, EvmSstoreOutput},
    derive::{router, signature, Contract},
    types::{EvmCallMethodInput, EvmCallMethodOutput, EvmCreateMethodInput, EvmCreateMethodOutput},
    AccountManager,
    ContextReader,
    SharedAPI,
};

#[derive(Contract)]
pub struct EVM<'a, CR: ContextReader, AM: AccountManager> {
    cr: &'a CR,
    am: &'a AM,
}

#[router(mode = "codec")]
impl<'a, CR: ContextReader, AM: AccountManager> EvmAPI for EVM<'a, CR, AM> {
    #[signature("_evm_call(address,uint256,bytes,uint64)")]
    fn call(&self, input: EvmCallMethodInput) -> EvmCallMethodOutput {
        _evm_call(self.cr, self.am, input)
    }

    #[signature("_evm_create(bytes,uint256,u64,bool,uint256)")]
    fn create(&self, input: EvmCreateMethodInput) -> EvmCreateMethodOutput {
        _evm_create(self.cr, self.am, input)
    }

    #[signature("sload(u256)")]
    fn sload(&self, input: EvmSloadInput) -> EvmSloadOutput {
        let contract_address = self.cr.contract_address();
        let (value, _is_cold) = self.am.storage(contract_address, input.index, false);
        EvmSloadOutput { value }
    }

    #[signature("sstore(u256,u256)")]
    fn sstore(&self, input: EvmSstoreInput) -> EvmSstoreOutput {
        let contract_address = self.cr.contract_address();
        _ = self
            .am
            .write_storage(contract_address, input.index, input.value);
        EvmSstoreOutput {}
    }
}

basic_entrypoint!(
    EVM<'static, fluentbase_sdk::GuestContextReader, fluentbase_sdk::GuestAccountManager>
);
