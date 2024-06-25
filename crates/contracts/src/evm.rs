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

    #[signature("_evm_sload(uint256)")]
    fn sload(&self, input: EvmSloadInput) -> EvmSloadOutput {
        let contract_address = self.cr.contract_address();
        let (value, _is_cold) = self.am.storage(contract_address, input.index, false);
        EvmSloadOutput { value }
    }

    #[signature("_evm_sstore(uint256,uint256)")]
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

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{
        codec::BufferDecoder,
        types::{
            CoreInput,
            EvmSloadMethodInput,
            EvmSloadMethodOutput,
            EvmSstoreMethodInput,
            EVM_SLOAD_METHOD_ID,
            EVM_SSTORE_METHOD_ID,
        },
        ContractInput,
        LowLevelSDK,
    };
    use revm_precompile::primitives::U256;

    #[test]
    fn test_sstore_sload() {
        let context = ContractInput::default().encode_to_vec(0);
        LowLevelSDK::with_test_context(context);
        // call sstore
        LowLevelSDK::init_with_devnet_genesis();
        let core_input = CoreInput {
            method_id: EVM_SSTORE_METHOD_ID,
            method_data: EvmSstoreMethodInput {
                index: U256::from(1),
                value: U256::from(2),
            },
        }
        .encode_to_vec(0);
        LowLevelSDK::with_test_input(core_input);
        let evm = EVM::default();
        evm.main::<LowLevelSDK>();
        LowLevelSDK::get_test_output();
        // call sload
        let core_input = CoreInput {
            method_id: EVM_SLOAD_METHOD_ID,
            method_data: EvmSloadMethodInput {
                index: U256::from(1),
            },
        }
        .encode_to_vec(0);
        LowLevelSDK::with_test_input(core_input);
        let evm = EVM::default();
        evm.main::<LowLevelSDK>();
        let output = LowLevelSDK::get_test_output();
        assert!(!output.is_empty());
        let mut decoder = BufferDecoder::new(&output);
        let mut result = EvmSloadMethodOutput::default();
        EvmSloadMethodOutput::decode_body(&mut decoder, 0, &mut result);
        assert_eq!(result.value, U256::from(2));
    }

    #[test]
    fn test_sstore_sload_api() {
        let context = ContractInput::default().encode_to_vec(0);
        LowLevelSDK::with_test_context(context);
        // call sstore
        LowLevelSDK::init_with_devnet_genesis();
        let core_input = CoreInput {
            method_id: EVM_SSTORE_METHOD_ID,
            method_data: EvmSstoreMethodInput {
                index: U256::from(1),
                value: U256::from(2),
            },
        }
        .encode_to_vec(0);
        LowLevelSDK::with_test_input(core_input);
        let evm = EVM::default();
        evm.main::<LowLevelSDK>();
        LowLevelSDK::get_test_output();
        // call sload
        let core_input = CoreInput {
            method_id: EVM_SLOAD_METHOD_ID,
            method_data: EvmSloadMethodInput {
                index: U256::from(1),
            },
        }
        .encode_to_vec(0);
        LowLevelSDK::with_test_input(core_input);
        let evm = EVM::default();
        evm.main::<LowLevelSDK>();
        let output = LowLevelSDK::get_test_output();
        assert!(!output.is_empty());
        let mut decoder = BufferDecoder::new(&output);
        let mut result = EvmSloadMethodOutput::default();
        EvmSloadMethodOutput::decode_body(&mut decoder, 0, &mut result);
        assert_eq!(result.value, U256::from(2));
    }
}
