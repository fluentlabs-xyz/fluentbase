use core::ptr;
use fluentbase_core::evm::{call::_evm_call, create::_evm_create};
// use fluentbase_core::evm::{call::_evm_call, create::_evm_create};
use fluentbase_sdk::{
    codec::Encoder,
    router,
    signature,
    AccountManager,
    Address,
    Bytes,
    ContextReader,
    CoreInput,
    EvmCallMethodInput,
    EvmCreateMethodInput,
    EvmSloadMethodInput,
    EvmSloadMethodOutput,
    EvmSstoreMethodInput,
    EvmSstoreMethodOutput,
    GuestAccountManager,
    GuestContextReader,
    ICoreInput,
    SharedAPI,
    EVM_CALL_METHOD_ID,
    EVM_CREATE_METHOD_ID,
    EVM_SLOAD_METHOD_ID,
    EVM_SSTORE_METHOD_ID,
    U256,
};

pub trait EvmAPI {
    fn address<SDK: SharedAPI>(&self) -> Address;
    fn balance<SDK: SharedAPI>(&self, address: Address) -> U256;
    fn calldatacopy<SDK: SharedAPI>(&self, mem_ptr: *mut u8, data_offset: u64, len: u64);
    fn calldataload<SDK: SharedAPI>(&self, offset: u64) -> U256;
    fn calldatasize<SDK: SharedAPI>(&self) -> u64;
    fn sload<SDK: SharedAPI>(&self, index: U256) -> U256;
    fn sstore<SDK: SharedAPI>(&self, index: U256, value: U256);
}

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
        SDK::read(
            unsafe { &mut *ptr::slice_from_raw_parts_mut(mem_ptr, len as usize) },
            data_offset as u32,
        )
    }

    fn calldataload<SDK: SharedAPI>(&self, mut offset: u64) -> U256 {
        let input_size = SDK::input_size() as u64;
        if offset > input_size {
            offset = input_size;
        }
        let mut buffer32: [u8; 32] = [0u8; 32];
        SDK::read(
            &mut buffer32[..(input_size as usize - offset as usize)],
            offset as u32,
        );
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

#[router(mode = "codec")]
impl<'a, CR: ContextReader, AM: AccountManager> EVM<'a, CR, AM> {
    #[signature("_evm_create(bytes,uint256,u64,bool,uint256)")]
    fn evm_create<SDK: SharedAPI>(&self, input: EvmCreateMethodInput) {
        let input = Self::decode_method_input::<EvmCreateMethodInput>(&input[4..]);
        let output = _evm_create(self.cr, self.am, input);
        SDK::write(&output.encode_to_vec(0));
    }

    #[signature("_evm_call(address,uint256,bytes,uint64)")]
    fn evm_call<SDK: SharedAPI>(&self, input: EvmCallMethodInput) {
        let input = Self::decode_method_input::<EvmCallMethodInput>(&input[4..]);
        let output = _evm_call(self.cr, self.am, input);
        SDK::write(&output.encode_to_vec(0));
    }

    #[signature("_evm_sload(uint256)")]
    fn evm_sload<SDK: SharedAPI>(&self, input: EvmSloadMethodInput) {
        let input = Self::decode_method_input::<EvmSloadMethodInput>(&input[4..]);
        let value = self.sload::<SDK>(input.index);
        let output = EvmSloadMethodOutput { value }.encode_to_vec(0);
        SDK::write(&output);
    }
    #[signature("_evm_sstore(uint256,uint256)")]
    fn evm_sstore<SDK: SharedAPI>(&self, input: EvmSstoreMethodInput) {
        let input = Self::decode_method_input::<EvmSstoreMethodInput>(&input[4..]);
        self.sstore::<SDK>(input.index, input.value);
        let output = EvmSstoreMethodOutput {}.encode_to_vec(0);
        SDK::write(&output);
    }

    // fn decode_method_input<T: Encoder<T> + Default>(input: &[u8]) -> T {
    //     let mut core_input = T::default();
    //     <CoreInput<T> as ICoreInput>::MethodData::decode_field_body(input, &mut core_input);
    //     core_input
    // }

    // pub fn main<SDK: SharedAPI>(&self) {
    //     let input = GuestContextReader::contract_input();
    //     if input.len() < 4 {
    //         panic!("not well-formed input");
    //     }
    //     let mut method_id = 0u32;
    //     <CoreInput<Bytes> as ICoreInput>::MethodId::decode_field_header(
    //         &input[0..4],
    //         &mut method_id,
    //     );
    //     match method_id {
    //         EVM_CREATE_METHOD_ID => {
    //             let input = Self::decode_method_input::<EvmCreateMethodInput>(&input[4..]);
    //             let output = _evm_create(self.cr, self.am, input);
    //             SDK::write(&output.encode_to_vec(0));
    //         }
    //         EVM_CALL_METHOD_ID => {
    //             let input = Self::decode_method_input::<EvmCallMethodInput>(&input[4..]);
    //             let output = _evm_call(self.cr, self.am, input);
    //             SDK::write(&output.encode_to_vec(0));
    //         }
    //         EVM_SLOAD_METHOD_ID => {
    //             let input = Self::decode_method_input::<EvmSloadMethodInput>(&input[4..]);
    //             let value = self.sload::<SDK>(input.index);
    //             let output = EvmSloadMethodOutput { value }.encode_to_vec(0);
    //             SDK::write(&output);
    //         }
    //         EVM_SSTORE_METHOD_ID => {
    //             let input = Self::decode_method_input::<EvmSstoreMethodInput>(&input[4..]);
    //             self.sstore::<SDK>(input.index, input.value);
    //             let output = EvmSstoreMethodOutput {}.encode_to_vec(0);
    //             SDK::write(&output);
    //         }
    //         _ => panic!("unknown method: {}", method_id),
    //     }
    // }
}

impl Default for EVM<'static, GuestContextReader, GuestAccountManager> {
    fn default() -> Self {
        EVM {
            cr: &GuestContextReader::DEFAULT,
            am: &GuestAccountManager::DEFAULT,
        }
    }
}
