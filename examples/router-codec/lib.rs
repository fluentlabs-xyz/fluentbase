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
    EvmCallMethodOutput,
    EvmCreateMethodInput,
    EvmCreateMethodOutput,
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
    fn evm_create<SDK: SharedAPI>(&self, input: EvmCreateMethodInput) -> EvmCreateMethodOutput {
        _evm_create(self.cr, self.am, input)
    }

    #[signature("_evm_call(address,uint256,bytes,uint64)")]
    fn evm_call<SDK: SharedAPI>(&self, input: EvmCallMethodInput) -> EvmCallMethodOutput {
        _evm_call(self.cr, self.am, input)
    }

    #[signature("_evm_sload(uint256)")]
    fn evm_sload<SDK: SharedAPI>(&self, input: EvmSloadMethodInput) -> EvmSloadMethodOutput {
        let value = self.sload::<SDK>(input.index);
        EvmSloadMethodOutput { value }
    }
    #[signature("_evm_sstore(uint256,uint256)")]
    fn evm_sstore<SDK: SharedAPI>(&self, input: EvmSstoreMethodInput) -> EvmSstoreMethodOutput {
        self.sstore::<SDK>(input.index, input.value);
        EvmSstoreMethodOutput {}
    }
}

impl Default for EVM<'static, GuestContextReader, GuestAccountManager> {
    fn default() -> Self {
        EVM {
            cr: &GuestContextReader::DEFAULT,
            am: &GuestAccountManager::DEFAULT,
        }
    }
}
