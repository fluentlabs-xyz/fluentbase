use core::cell::RefCell;
use fluentbase_runtime::{RuntimeContext, RuntimeContextWrapper};
use fluentbase_sdk::{
    bytes::Buf,
    native_api::NativeAPI,
    Address,
    Bytes,
    ContextReader,
    ContractContextV1,
    ExitCode,
    IsAccountEmpty,
    IsColdAccess,
    SharedAPI,
    SharedContextInputV1,
    StorageAPI,
    SyscallResult,
    B256,
    FUEL_DENOM_RATE,
    U256,
};
use hashbrown::HashMap;
use std::rc::Rc;

#[derive(Clone)]
pub struct HostTestingContext {
    inner: Rc<RefCell<TestingContextInner>>,
}

pub type HostTestingContextNativeAPI = RuntimeContextWrapper;

impl HostTestingContext {
    pub fn with_shared_context_input(self, ctx: SharedContextInputV1) -> Self {
        self.inner.borrow_mut().shared_context_input_v1 = ctx;
        self
    }
    pub fn with_contract_context(self, contract_context: ContractContextV1) -> Self {
        self.inner.borrow_mut().shared_context_input_v1.contract = contract_context;
        self
    }
    pub fn with_devnet_genesis(self) -> Self {
        // TODO(dmitry123): "implement this"
        self
    }
    pub fn with_block_number(self, number: u64) -> Self {
        self.inner.borrow_mut().shared_context_input_v1.block.number = number;
        self
    }
    pub fn with_input<I: Into<Bytes>>(self, input: I) -> Self {
        self.inner
            .borrow_mut()
            .native_sdk
            .ctx
            .borrow_mut()
            .change_input(input.into());
        self
    }
    pub fn with_fuel_limit(self, fuel_limit: u64) -> Self {
        self.inner.borrow_mut().native_sdk.set_fuel(fuel_limit);
        self
    }
    pub fn with_gas_limit(self, gas_limit: u64) -> Self {
        self.inner
            .borrow_mut()
            .native_sdk
            .set_fuel(gas_limit * FUEL_DENOM_RATE);
        self
    }
    pub fn take_output(&self) -> Vec<u8> {
        self.inner.borrow_mut().native_sdk.take_output()
    }
    pub fn exit_code(&self) -> i32 {
        self.inner.borrow_mut().native_sdk.exit_code()
    }
    pub fn dump_storage(&self) -> HashMap<(Address, U256), U256> {
        self.inner.borrow().persistent_storage.clone()
    }
    pub fn visit_inner_storage_mut<F: FnMut(&mut HashMap<(Address, U256), U256>)>(&self, mut f: F) {
        f(&mut self.inner.borrow_mut().persistent_storage)
    }
    pub fn visit_inner_storage<F: Fn(&HashMap<(Address, U256), U256>)>(&self, f: F) {
        f(&self.inner.borrow_mut().persistent_storage)
    }
}

struct TestingContextInner {
    shared_context_input_v1: SharedContextInputV1,
    native_sdk: RuntimeContextWrapper,
    persistent_storage: HashMap<(Address, U256), U256>,
    transient_storage: HashMap<(Address, U256), U256>,
    logs: Vec<(Bytes, Vec<B256>)>,
    preimages: HashMap<B256, Bytes>,
}

impl Default for HostTestingContext {
    fn default() -> Self {
        Self {
            inner: Rc::new(RefCell::new(TestingContextInner {
                shared_context_input_v1: SharedContextInputV1::default(),
                native_sdk: RuntimeContextWrapper::new(RuntimeContext::root(0)),
                persistent_storage: Default::default(),
                transient_storage: Default::default(),
                logs: vec![],
                preimages: Default::default(),
            })),
        }
    }
}

impl StorageAPI for HostTestingContext {
    fn write_storage(&mut self, slot: U256, value: U256) -> SyscallResult<()> {
        let target_address = self.inner.borrow().shared_context_input_v1.contract.address;
        self.inner
            .borrow_mut()
            .persistent_storage
            .insert((target_address, slot), value);
        SyscallResult::new((), 0, 0, 0)
    }

    fn storage(&self, slot: &U256) -> SyscallResult<U256> {
        let target_address = self.inner.borrow().shared_context_input_v1.contract.address;
        let value = self
            .inner
            .borrow()
            .persistent_storage
            .get(&(target_address, *slot))
            .cloned()
            .unwrap_or_default();
        SyscallResult::new(value, 0, 0, 0)
    }
}

impl SharedAPI for HostTestingContext {
    fn context(&self) -> impl ContextReader {
        self.inner.borrow().shared_context_input_v1.clone()
    }

    fn keccak256(&self, data: &[u8]) -> B256 {
        RuntimeContextWrapper::keccak256(data)
    }

    fn read(&self, target: &mut [u8], offset: u32) {
        self.inner.borrow().native_sdk.read(target, offset);
    }

    fn input_size(&self) -> u32 {
        self.inner.borrow().native_sdk.input_size()
    }

    fn read_context(&self, target: &mut [u8], offset: u32) {
        let buffer = self
            .inner
            .borrow()
            .shared_context_input_v1
            .encode_to_vec()
            .unwrap();
        assert!(target.len() + offset as usize <= SharedContextInputV1::SIZE);
        buffer
            .slice(offset as usize..offset as usize + target.len())
            .copy_to_slice(target);
    }

    fn charge_fuel_manually(&self, fuel_consumed: u64, fuel_refunded: i64) {
        self.inner
            .borrow()
            .native_sdk
            .charge_fuel_manually(fuel_consumed, fuel_refunded);
    }

    fn fuel(&self) -> u64 {
        self.inner.borrow().native_sdk.fuel()
    }

    fn write(&mut self, output: &[u8]) {
        self.inner.borrow().native_sdk.write(output);
    }

    fn exit(&self, exit_code: ExitCode) -> ! {
        self.inner.borrow().native_sdk.exit(exit_code.into_i32());
    }

    fn write_transient_storage(&mut self, slot: U256, value: U256) -> SyscallResult<()> {
        let target_address = self.inner.borrow().shared_context_input_v1.contract.address;
        self.inner
            .borrow_mut()
            .transient_storage
            .insert((target_address, slot), value);
        SyscallResult::new((), 0, 0, 0)
    }

    fn transient_storage(&self, slot: &U256) -> SyscallResult<U256> {
        let target_address = self.inner.borrow().shared_context_input_v1.contract.address;
        let value = self
            .inner
            .borrow()
            .transient_storage
            .get(&(target_address, *slot))
            .cloned()
            .unwrap_or_default();
        SyscallResult::new(value, 0, 0, 0)
    }

    fn delegated_storage(
        &self,
        address: &Address,
        slot: &U256,
    ) -> SyscallResult<(U256, IsColdAccess, IsAccountEmpty)> {
        let value = self
            .inner
            .borrow()
            .persistent_storage
            .get(&(*address, *slot))
            .cloned()
            .unwrap_or_default();
        SyscallResult::new((value, false, false), 0, 0, 0)
    }

    fn preimage_copy(&self, hash: &B256) -> SyscallResult<Bytes> {
        let value = self
            .inner
            .borrow()
            .preimages
            .get(hash)
            .cloned()
            .unwrap_or_default();
        SyscallResult::new(value, 0, 0, 0)
    }

    fn preimage_size(&self, hash: &B256) -> SyscallResult<u32> {
        let preimage_size = self
            .inner
            .borrow()
            .preimages
            .get(hash)
            .map(|v| v.len() as u32)
            .unwrap_or_default();
        SyscallResult::new(preimage_size, 0, 0, 0)
    }

    fn emit_log(&mut self, topics: &[B256], data: &[u8]) -> SyscallResult<()> {
        self.inner
            .borrow_mut()
            .logs
            .push((Bytes::copy_from_slice(data), topics.to_vec()));
        SyscallResult::new((), 0, 0, 0)
    }

    fn self_balance(&self) -> SyscallResult<U256> {
        unimplemented!("not supported for testing context")
    }

    fn balance(&self, _address: &Address) -> SyscallResult<U256> {
        unimplemented!("not supported for testing context")
    }

    fn code_size(&self, _address: &Address) -> SyscallResult<u32> {
        unimplemented!("not supported for testing context")
    }

    fn code_hash(&self, _address: &Address) -> SyscallResult<B256> {
        unimplemented!("not supported for testing context")
    }

    fn code_copy(
        &self,
        _address: &Address,
        _code_offset: u64,
        _code_length: u64,
    ) -> SyscallResult<Bytes> {
        unimplemented!("not supported for testing context")
    }

    fn write_preimage(&mut self, preimage: Bytes) -> SyscallResult<B256> {
        let hash = self.keccak256(preimage.as_ref());
        self.inner.borrow_mut().preimages.insert(hash, preimage);
        SyscallResult::new(hash, 0, 0, 0)
    }

    fn create(
        &mut self,
        _salt: Option<U256>,
        _value: &U256,
        _init_code: &[u8],
    ) -> SyscallResult<Bytes> {
        unimplemented!("not supported for testing context")
    }

    fn call(
        &mut self,
        _address: Address,
        _value: U256,
        _input: &[u8],
        _fuel_limit: Option<u64>,
    ) -> SyscallResult<Bytes> {
        unimplemented!("not supported for testing context")
    }

    fn call_code(
        &mut self,
        _address: Address,
        _value: U256,
        _input: &[u8],
        _fuel_limit: Option<u64>,
    ) -> SyscallResult<Bytes> {
        unimplemented!("not supported for testing context")
    }

    fn delegate_call(
        &mut self,
        _address: Address,
        _input: &[u8],
        _fuel_limit: Option<u64>,
    ) -> SyscallResult<Bytes> {
        unimplemented!("not supported for testing context")
    }

    fn static_call(
        &mut self,
        _address: Address,
        _input: &[u8],
        _fuel_limit: Option<u64>,
    ) -> SyscallResult<Bytes> {
        unimplemented!("not supported for testing context")
    }

    fn destroy_account(&mut self, _address: Address) -> SyscallResult<()> {
        unimplemented!("not supported for testing context")
    }

    fn metadata_write(
        &mut self,
        _address: &Address,
        _offset: u32,
        _metadata: Bytes,
    ) -> SyscallResult<()> {
        unimplemented!("not supported for testing context")
    }

    fn metadata_size(&self, _address: &Address) -> SyscallResult<u32> {
        unimplemented!("not supported for testing context")
    }

    fn metadata_copy(
        &self,
        _address: &Address,
        _offset: u32,
        _length: u32,
    ) -> SyscallResult<Bytes> {
        unimplemented!("not supported for testing context")
    }
}
