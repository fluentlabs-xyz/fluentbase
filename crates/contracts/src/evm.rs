use crate::utils::decode_method_input;
use fluentbase_core::evm::{call::_evm_call, create::_evm_create};
use fluentbase_sdk::{
    basic_entrypoint,
    codec::Encoder,
    contracts::EvmAPI,
    derive::Contract,
    types::{
        CoreInput,
        EvmCallMethodInput,
        EvmCreateMethodInput,
        EvmSloadMethodInput,
        EvmSloadMethodOutput,
        EvmSstoreMethodInput,
        EvmSstoreMethodOutput,
        ICoreInput,
        EVM_CALL_METHOD_ID,
        EVM_CREATE_METHOD_ID,
        EVM_SLOAD_METHOD_ID,
        EVM_SSTORE_METHOD_ID,
    },
    AccountManager,
    Address,
    Bytes,
    ContextReader,
    GuestContextReader,
    SharedAPI,
    U256,
};

#[derive(Contract)]
pub struct EVM<'a, CR: ContextReader, AM: AccountManager> {
    cr: &'a CR,
    am: &'a AM,
}

impl<'a, CR: ContextReader, AM: AccountManager> EvmAPI for EVM<'a, CR, AM> {
    fn address<SDK: SharedAPI>(&self) -> Address {
        self.cr.contract_address()
    }

    fn balance<SDK: SharedAPI>(&self, address: Address) -> U256 {
        let (account, _) = self.am.account(address);
        account.balance
    }

    fn calldatacopy<SDK: SharedAPI>(&self, mem_ptr: *mut u8, data_offset: u64, len: u64) {
        SDK::read(mem_ptr, len as u32, data_offset as u32)
    }

    fn calldataload<SDK: SharedAPI>(&self, mut offset: u64) -> U256 {
        let input_size = SDK::input_size() as u64;
        if offset > input_size {
            offset = input_size;
        }
        let mut buffer32: [u8; 32] = [0u8; 32];
        SDK::read(buffer32.as_mut_ptr(), 32, offset as u32);
        U256::ZERO
    }

    fn calldatasize<SDK: SharedAPI>(&self) -> u64 {
        SDK::input_size() as u64
    }

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
    pub fn deploy<SDK: SharedAPI>(&self) {
        unreachable!("precompiles can't be deployed, it exists since a genesis state")
    }

    pub fn main<SDK: SharedAPI>(&self) {
        let input = GuestContextReader::contract_input();
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
                let input = decode_method_input::<EvmCreateMethodInput>(&input[4..]);
                let output = _evm_create(self.cr, self.am, input);
                let output = output.encode_to_vec(0);
                SDK::write(output.as_ptr(), output.len() as u32);
            }
            EVM_CALL_METHOD_ID => {
                let input = decode_method_input::<EvmCallMethodInput>(&input[4..]);
                let output = _evm_call(self.cr, self.am, input);
                let output = output.encode_to_vec(0);
                SDK::write(output.as_ptr(), output.len() as u32);
            }
            EVM_SLOAD_METHOD_ID => {
                let input = decode_method_input::<EvmSloadMethodInput>(&input[4..]);
                let value = self.sload::<SDK>(input.index);
                let output = EvmSloadMethodOutput { value }.encode_to_vec(0);
                SDK::write(output.as_ptr(), output.len() as u32);
            }
            EVM_SSTORE_METHOD_ID => {
                let input = decode_method_input::<EvmSstoreMethodInput>(&input[4..]);
                self.sstore::<SDK>(input.index, input.value);
                let output = EvmSstoreMethodOutput {}.encode_to_vec(0);
                SDK::write(output.as_ptr(), output.len() as u32);
            }
            _ => panic!("unknown method: {}", method_id),
        }
    }
}

basic_entrypoint!(
    EVM<'static, fluentbase_sdk::GuestContextReader, fluentbase_sdk::GuestAccountManager>
);
