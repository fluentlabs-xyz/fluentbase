use fluentbase_core::evm::{call::_evm_call, create::_evm_create};
use fluentbase_sdk::{
    basic_entrypoint,
    codec::Encoder,
    contracts::{EvmAPI, EvmSloadInput, EvmSloadOutput, EvmSstoreInput, EvmSstoreOutput},
    derive::{router, signature, Contract},
    types::{EvmCallMethodInput, EvmCallMethodOutput, EvmCreateMethodInput, EvmCreateMethodOutput},
    ContextReader,
    SharedAPI,
    SovereignAPI,
};

#[derive(Contract)]
pub struct EVM<CTX: ContextReader, SDK: SovereignAPI> {
    ctx: CTX,
    sdk: SDK,
}

#[router(mode = "codec")]
impl<CTX: ContextReader, SDK: SovereignAPI> EvmAPI for EVM<CTX, SDK> {
    #[signature("_evm_call(address,uint256,bytes,uint64)")]
    fn call(&self, input: EvmCallMethodInput) -> EvmCallMethodOutput {
        _evm_call(&self.ctx, &self.sdk, input)
    }

    #[signature("_evm_create(bytes,uint256,u64,bool,uint256)")]
    fn create(&self, input: EvmCreateMethodInput) -> EvmCreateMethodOutput {
        _evm_create(&self.ctx, &self.sdk, input)
    }

    #[signature("_evm_sload(uint256)")]
    fn sload(&self, input: EvmSloadInput) -> EvmSloadOutput {
        let contract_address = self.ctx.contract_address();
        let (value, _is_cold) = self.sdk.storage(&contract_address, &input.index, false);
        EvmSloadOutput { value }
    }

    #[signature("_evm_sstore(uint256,uint256)")]
    fn sstore(&self, input: EvmSstoreInput) -> EvmSstoreOutput {
        let contract_address = self.ctx.contract_address();
        _ = self
            .sdk
            .write_storage(&contract_address, &input.index, &input.value);
        EvmSstoreOutput {}
    }
}

basic_entrypoint!(EVM);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{
        codec::BufferDecoder,
        runtime::TestingContext,
        types::{
            CoreInput,
            EvmSloadMethodInput,
            EvmSloadMethodOutput,
            EvmSstoreMethodInput,
            EVM_SLOAD_METHOD_ID,
            EVM_SSTORE_METHOD_ID,
        },
        ContractInput,
    };
    use revm_precompile::primitives::U256;

    #[test]
    fn test_sstore_sload() {
        let ctx = ContractInput::default();
        let sdk = TestingContext::new()
            .with_context(ctx.encode_to_vec(0))
            .with_devnet_genesis();
        // call sstore
        let core_input = CoreInput {
            method_id: EVM_SSTORE_METHOD_ID,
            method_data: EvmSstoreMethodInput {
                index: U256::from(1),
                value: U256::from(2),
            },
        }
        .encode_to_vec(0);
        let evm = EVM::new(ctx.clone(), sdk.clone().with_input(core_input));
        evm.main();
        // call sload
        let core_input = CoreInput {
            method_id: EVM_SLOAD_METHOD_ID,
            method_data: EvmSloadMethodInput {
                index: U256::from(1),
            },
        }
        .encode_to_vec(0);
        let evm = EVM::new(ctx.clone(), sdk.clone().with_input(core_input));
        evm.main();
        let output = sdk.output();
        assert!(!output.is_empty());
        let mut decoder = BufferDecoder::new(&output);
        let mut result = EvmSloadMethodOutput::default();
        EvmSloadMethodOutput::decode_body(&mut decoder, 0, &mut result);
        assert_eq!(result.value, U256::from(2));
    }

    #[test]
    fn test_sstore_sload_api() {
        let ctx = ContractInput::default();
        let sdk = TestingContext::new()
            .with_context(ctx.encode_to_vec(0))
            .with_devnet_genesis();
        // call sstore
        let core_input = CoreInput {
            method_id: EVM_SSTORE_METHOD_ID,
            method_data: EvmSstoreMethodInput {
                index: U256::from(1),
                value: U256::from(2),
            },
        }
        .encode_to_vec(0);
        let evm = EVM::new(ctx.clone(), sdk.clone().with_input(core_input));
        evm.main();
        // call sload
        let core_input = CoreInput {
            method_id: EVM_SLOAD_METHOD_ID,
            method_data: EvmSloadMethodInput {
                index: U256::from(1),
            },
        }
        .encode_to_vec(0);
        let evm = EVM::new(ctx.clone(), sdk.clone().with_input(core_input));
        evm.main();
        let output = sdk.output();
        assert!(!output.is_empty());
        let mut decoder = BufferDecoder::new(&output);
        let mut result = EvmSloadMethodOutput::default();
        EvmSloadMethodOutput::decode_body(&mut decoder, 0, &mut result);
        assert_eq!(result.value, U256::from(2));
    }
}
