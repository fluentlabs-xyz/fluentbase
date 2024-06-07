use fluentbase_core::evm::{call::_evm_call, create::_evm_create};
use fluentbase_sdk::{
    codec::Encoder,
    AccountManager,
    Bytes,
    ContextReader,
    CoreInput,
    EvmCallMethodInput,
    EvmCreateMethodInput,
    EvmSloadMethodInput,
    EvmSloadMethodOutput,
    EvmSstoreMethodInput,
    EvmSstoreMethodOutput,
    ExecutionContext,
    ICoreInput,
    JzktAccountManager,
    SharedAPI,
    EVM_CALL_METHOD_ID,
    EVM_CREATE_METHOD_ID,
    EVM_SLOAD_METHOD_ID,
    EVM_SSTORE_METHOD_ID,
    U256,
};

pub trait EvmAPI {
    fn sload<SDK: SharedAPI>(&self, index: U256) -> U256;
    fn sstore<SDK: SharedAPI>(&self, index: U256, value: U256);
}

pub struct EVM<'a, CR: ContextReader, AM: AccountManager> {
    cr: &'a CR,
    am: &'a AM,
}

impl<'a, CR: ContextReader, AM: AccountManager> EvmAPI for EVM<'a, CR, AM> {
    fn sload<SDK: SharedAPI>(&self, index: U256) -> U256 {
        let contract_address = self.cr.contract_address();
        let (value, _is_cold) = self.am.storage(contract_address, index, false);
        value
    }

    fn sstore<SDK: SharedAPI>(&self, index: U256, value: U256) {
        let contract_address = self.cr.contract_address();
        _ = self.am.write_storage(contract_address, index, value);
    }
}

impl<'a, CR: ContextReader, AM: AccountManager> EVM<'a, CR, AM> {
    pub fn deploy<SDK: SharedAPI>(&self) {}

    fn decode_method_input<T: Encoder<T> + Default>(input: &[u8]) -> T {
        let mut core_input = T::default();
        <CoreInput<T> as ICoreInput>::MethodData::decode_field_body(input, &mut core_input);
        core_input
    }

    pub fn main<SDK: SharedAPI>(&self) {
        let input = ExecutionContext::contract_input();
        if input.len() < 4 {
            panic!("not well-formed input");
        }
        let mut method_id = 0u32;
        <CoreInput<Bytes> as ICoreInput>::MethodId::decode_field_header(
            &input[0..4],
            &mut method_id,
        );
        match method_id {
            EVM_CREATE_METHOD_ID => {
                let input = Self::decode_method_input::<EvmCreateMethodInput>(&input[4..]);
                let output = _evm_create(self.cr, self.am, input);
                SDK::write(&output.encode_to_vec(0));
            }
            EVM_CALL_METHOD_ID => {
                let input = Self::decode_method_input::<EvmCallMethodInput>(&input[4..]);
                let output = _evm_call(self.cr, self.am, input);
                SDK::write(&output.encode_to_vec(0));
            }
            EVM_SLOAD_METHOD_ID => {
                let input = Self::decode_method_input::<EvmSloadMethodInput>(&input[4..]);
                let value = self.sload::<SDK>(input.index);
                let output = EvmSloadMethodOutput { value }.encode_to_vec(0);
                SDK::write(&output);
            }
            EVM_SSTORE_METHOD_ID => {
                let input = Self::decode_method_input::<EvmSstoreMethodInput>(&input[4..]);
                self.sstore::<SDK>(input.index, input.value);
                let output = EvmSstoreMethodOutput {}.encode_to_vec(0);
                SDK::write(&output);
            }
            _ => panic!("unknown method: {}", method_id),
        }
    }
}

impl Default for EVM<'static, ExecutionContext, JzktAccountManager> {
    fn default() -> Self {
        EVM {
            cr: &ExecutionContext::DEFAULT,
            am: &JzktAccountManager::DEFAULT,
        }
    }
}
