use core::cell::RefCell;
use fluentbase_runtime::RuntimeContextWrapper;
use fluentbase_sdk::{
    bytes::Buf, system::RuntimeInterruptionOutcomeV1, Address, Bytes, ContextReader,
    ContractContextV1, ExitCode, SharedAPI, SharedContextInputV1, StorageAPI, SyscallResult,
    SystemAPI, B256, FUEL_DENOM_RATE, U256,
};
use hashbrown::HashMap;
use std::{borrow::Cow, cell::RefMut, mem::take, rc::Rc};

#[derive(Clone)]
pub struct TestingContextImpl {
    inner: Rc<RefCell<TestingContextInner>>,
}

pub type HostTestingContextNativeAPI = RuntimeContextWrapper;

impl TestingContextImpl {
    pub fn with_shared_context_input(self, ctx: SharedContextInputV1) -> Self {
        self.inner.borrow_mut().shared_context_input_v1 = ctx;
        self
    }
    pub fn with_contract_context(self, contract_context: ContractContextV1) -> Self {
        self.inner.borrow_mut().shared_context_input_v1.contract = contract_context;
        self
    }
    pub fn context_mut(&self) -> RefMut<'_, ContractContextV1> {
        RefMut::map(self.inner.borrow_mut(), |v| {
            &mut v.shared_context_input_v1.contract
        })
    }
    pub fn with_caller(self, caller: Address) -> Self {
        self.inner
            .borrow_mut()
            .shared_context_input_v1
            .contract
            .caller = caller;
        self
    }
    pub fn with_block_number(self, number: u64) -> Self {
        self.inner.borrow_mut().shared_context_input_v1.block.number = number;
        self
    }
    pub fn with_input<I: Into<Bytes>>(self, input: I) -> Self {
        let mut ctx = self.inner.borrow_mut();
        ctx.input = input.into();
        drop(ctx);
        self
    }
    /// Sets the initial storage state
    pub fn with_storage(self, storage: HashMap<(Address, U256), U256>) -> Self {
        self.inner.borrow_mut().persistent_storage = storage;
        self
    }

    /// Merges storage entries
    pub fn with_storage_entries(
        self,
        entries: impl IntoIterator<Item = ((Address, U256), U256)>,
    ) -> Self {
        self.inner.borrow_mut().persistent_storage.extend(entries);
        self
    }

    /// Sets storage for a specific contract
    pub fn with_contract_storage(self, contract: Address, slots: HashMap<U256, U256>) -> Self {
        for (slot, value) in slots {
            self.inner
                .borrow_mut()
                .persistent_storage
                .insert((contract, slot), value);
        }
        self
    }

    pub fn set_ownable_account_address(&mut self, address: Address) {
        self.inner.borrow_mut().ownable_account_address = Some(address);
    }
    pub fn with_fuel_limit(self, fuel_limit: u64) -> Self {
        self.inner.borrow_mut().fuel_limit = Some(fuel_limit);
        self
    }
    pub fn with_gas_limit(self, gas_limit: u64) -> Self {
        self.inner.borrow_mut().fuel_limit = Some(gas_limit * FUEL_DENOM_RATE);
        self
    }
    pub fn consumed_fuel(&self) -> u64 {
        self.inner.borrow().consumed_fuel
    }
    pub fn take_output(&self) -> Vec<u8> {
        let mut ctx = self.inner.borrow_mut();
        take(&mut ctx.output)
    }
    pub fn exit_code(&self) -> i32 {
        let ctx = self.inner.borrow();
        ctx.exit_code
    }
    pub fn dump_storage(&self) -> HashMap<(Address, U256), U256> {
        self.inner.borrow().persistent_storage.clone()
    }
    pub fn restore_storage(&self, new_storage: HashMap<(Address, U256), U256>) {
        self.inner.borrow_mut().persistent_storage = new_storage;
    }
    pub fn dump_metadata_storage(&self) -> HashMap<(Address, U256), U256> {
        self.inner.borrow().metadata_storage.clone()
    }
    pub fn dump_metadata(&self) -> HashMap<(Address, Address), Vec<u8>> {
        self.inner.borrow().metadata.clone()
    }
    pub fn visit_inner_storage_mut<F: FnMut(&mut HashMap<(Address, U256), U256>)>(&self, mut f: F) {
        f(&mut self.inner.borrow_mut().persistent_storage)
    }
    pub fn visit_inner_metadata_storage_mut<F: FnMut(&mut HashMap<(Address, U256), U256>)>(
        &self,
        mut f: F,
    ) {
        f(&mut self.inner.borrow_mut().metadata_storage)
    }
    pub fn visit_inner_metadata_mut<F: FnMut(&mut HashMap<(Address, Address), Vec<u8>>)>(
        &self,
        mut f: F,
    ) {
        f(&mut self.inner.borrow_mut().metadata)
    }
    pub fn visit_inner_storage<F: Fn(&HashMap<(Address, U256), U256>)>(&self, f: F) {
        f(&self.inner.borrow_mut().persistent_storage)
    }

    /// Returns and clears all emitted logs.
    /// Each log is (data, topics).
    pub fn take_logs(&self) -> Vec<(Bytes, Vec<B256>)> {
        take(&mut self.inner.borrow_mut().logs)
    }

    /// Returns logs without clearing.
    pub fn logs(&self) -> Vec<(Bytes, Vec<B256>)> {
        self.inner.borrow().logs.clone()
    }
}

struct TestingContextInner {
    shared_context_input_v1: SharedContextInputV1,
    persistent_storage: HashMap<(Address, U256), U256>,
    metadata: HashMap<(Address, Address), Vec<u8>>,
    metadata_storage: HashMap<(Address, U256), U256>,
    transient_storage: HashMap<(Address, U256), U256>,
    logs: Vec<(Bytes, Vec<B256>)>,
    ownable_account_address: Option<Address>,
    input: Bytes,
    output: Vec<u8>,
    exit_code: i32,
    consumed_fuel: u64,
    fuel_limit: Option<u64>,
}

impl Default for TestingContextImpl {
    fn default() -> Self {
        Self {
            inner: Rc::new(RefCell::new(TestingContextInner {
                shared_context_input_v1: SharedContextInputV1::default(),
                persistent_storage: Default::default(),
                metadata: Default::default(),
                metadata_storage: Default::default(),
                transient_storage: Default::default(),
                logs: vec![],
                ownable_account_address: None,
                input: Default::default(),
                output: vec![],
                exit_code: 0,
                consumed_fuel: 0,
                fuel_limit: None,
            })),
        }
    }
}

impl StorageAPI for TestingContextImpl {
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

impl SharedAPI for TestingContextImpl {
    fn context(&self) -> impl ContextReader {
        self.inner.borrow().shared_context_input_v1.clone()
    }

    fn read(&self, target: &mut [u8], offset: u32) {
        let ctx = self.inner.borrow();
        if offset + target.len() as u32 <= ctx.input.len() as u32 {
            target.copy_from_slice(&ctx.input[(offset as usize)..(offset as usize + target.len())]);
        } else {
            panic!("can't read input: InputOutputOutOfBounds");
        }
    }

    fn input_size(&self) -> u32 {
        let ctx = self.inner.borrow();
        ctx.input.len() as u32
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

    fn charge_fuel(&self, fuel_consumed: u64) {
        let mut ctx = self.inner.borrow_mut();
        ctx.consumed_fuel += fuel_consumed;
    }

    fn fuel(&self) -> u64 {
        let ctx = self.inner.borrow();
        let fuel_limit = ctx.fuel_limit.expect("fuel is disabled");
        fuel_limit - ctx.consumed_fuel
    }

    fn write<T: AsRef<[u8]>>(&mut self, output: T) {
        let mut ctx = self.inner.borrow_mut();
        ctx.output.extend_from_slice(output.as_ref());
    }

    fn native_exit(&self, exit_code: ExitCode) -> ! {
        unimplemented!(
            "not allowed to do native exit: {} ({})",
            exit_code,
            exit_code as i32
        )
    }

    fn native_exec(
        &self,
        _code_hash: B256,
        _input: Cow<'_, [u8]>,
        _fuel_limit: Option<u64>,
        _state: u32,
    ) -> (u64, i64, i32) {
        unimplemented!("native exec is not supported");
    }

    fn return_data(&self) -> Bytes {
        unimplemented!("return data is not supported");
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

    fn emit_log<D: AsRef<[u8]>>(&mut self, topics: &[B256], data: D) -> SyscallResult<()> {
        self.inner
            .borrow_mut()
            .logs
            .push((Bytes::copy_from_slice(data.as_ref()), topics.to_vec()));
        SyscallResult::new((), 0, 0, 0)
    }

    fn self_balance(&self) -> SyscallResult<U256> {
        unimplemented!("not supported for testing context")
    }

    fn balance(&self, _address: &Address) -> SyscallResult<U256> {
        unimplemented!("not supported for testing context")
    }

    fn block_hash(&self, _number: u64) -> SyscallResult<B256> {
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
}

impl SystemAPI for TestingContextImpl {
    fn take_interruption_outcome(&mut self) -> Option<RuntimeInterruptionOutcomeV1> {
        unimplemented!("take_interruption_outcome")
    }

    fn insert_interruption_income(
        &mut self,
        _code_hash: B256,
        _input: Cow<'_, [u8]>,
        _fuel_limit: Option<u64>,
        _state: u32,
    ) {
        unimplemented!("insert_interruption_income")
    }

    fn unique_key(&self) -> u32 {
        unimplemented!("unique_key")
    }

    fn write_contract_metadata(&mut self, _metadata: Bytes) {
        unimplemented!("write_contract_metadata")
    }

    fn contract_metadata(&self) -> Bytes {
        unimplemented!("contract_metadata")
    }
}
