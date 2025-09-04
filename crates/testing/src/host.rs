use core::cell::RefCell;
use fluentbase_runtime::{RuntimeContext, RuntimeContextWrapper};
use fluentbase_sdk::syscall::SyscallResult;
use fluentbase_sdk::{
    bytes::Buf, calc_create4_address, native_api::NativeAPI, Address, Bytes, ContextReader,
    ContractContextV1, ExitCode, IsAccountEmpty, IsAccountOwnable, IsColdAccess, MetadataAPI,
    MetadataStorageAPI, SharedAPI, SharedContextInputV1, StorageAPI, B256,
    BN254_G1_POINT_COMPRESSED_SIZE, BN254_G1_POINT_DECOMPRESSED_SIZE,
    BN254_G2_POINT_COMPRESSED_SIZE, BN254_G2_POINT_DECOMPRESSED_SIZE, FUEL_DENOM_RATE, U256,
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
    pub fn set_ownable_account_address(&mut self, address: Address) {
        self.inner.borrow_mut().ownable_account_address = Some(address);
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
}

struct TestingContextInner {
    shared_context_input_v1: SharedContextInputV1,
    native_sdk: RuntimeContextWrapper,
    persistent_storage: HashMap<(Address, U256), U256>,
    metadata: HashMap<(Address, Address), Vec<u8>>,
    metadata_storage: HashMap<(Address, U256), U256>,
    transient_storage: HashMap<(Address, U256), U256>,
    logs: Vec<(Bytes, Vec<B256>)>,
    ownable_account_address: Option<Address>,
}

impl Default for HostTestingContext {
    fn default() -> Self {
        Self {
            inner: Rc::new(RefCell::new(TestingContextInner {
                shared_context_input_v1: SharedContextInputV1::default(),
                native_sdk: RuntimeContextWrapper::new(RuntimeContext::root(0)),
                persistent_storage: Default::default(),
                metadata: Default::default(),
                metadata_storage: Default::default(),
                transient_storage: Default::default(),
                logs: vec![],
                ownable_account_address: None,
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

impl MetadataAPI for HostTestingContext {
    fn metadata_write(
        &mut self,
        address: &Address,
        _offset: u32,
        metadata: Bytes,
    ) -> SyscallResult<()> {
        let mut ctx = self.inner.borrow_mut();
        let account_owner = ctx
            .ownable_account_address
            .expect("expected ownable account address");
        let value = ctx.metadata.get_mut(&(account_owner, *address));
        if let Some(value) = value {
            value.resize(metadata.len(), 0);
            value.copy_from_slice(metadata.as_ref());
        } else {
            ctx.metadata
                .insert((account_owner, address.clone()), metadata.to_vec());
        }
        SyscallResult::new((), 0, 0, ExitCode::Err)
    }

    fn metadata_size(
        &self,
        address: &Address,
    ) -> SyscallResult<(u32, IsAccountOwnable, IsColdAccess, IsAccountEmpty)> {
        let ctx = self.inner.borrow();
        let account_owner = ctx
            .ownable_account_address
            .expect("expected ownable account address");
        let value = ctx.metadata.get(&(account_owner, *address));
        if let Some(value) = value {
            let len = value.len();
            return SyscallResult::new((len as u32, false, false, false), 0, 0, ExitCode::Ok);
        }
        SyscallResult::new(Default::default(), 0, 0, ExitCode::Err)
    }

    fn metadata_create(&mut self, salt: &U256, metadata: Bytes) -> SyscallResult<()> {
        let mut ctx = self.inner.borrow_mut();
        let account_owner = ctx
            .ownable_account_address
            .expect("ownable account address should exist");
        let derived_metadata_address =
            calc_create4_address(&account_owner, salt, HostTestingContextNativeAPI::keccak256);
        let target_address = ctx.shared_context_input_v1.contract.address;
        ctx.metadata
            .insert(
                (target_address, derived_metadata_address),
                metadata.to_vec(),
            )
            .expect("metadata account collision");
        SyscallResult::new(Default::default(), 0, 0, ExitCode::Ok)
    }

    fn metadata_copy(&self, address: &Address, _offset: u32, length: u32) -> SyscallResult<Bytes> {
        let ctx = self.inner.borrow();
        let account_owner = ctx
            .ownable_account_address
            .expect("expected ownable account address");
        let value = ctx.metadata.get(&(account_owner, *address));
        if let Some(value) = value {
            let length = length.min(value.len() as u32);
            return SyscallResult::new(
                Bytes::copy_from_slice(&value[0..length as usize]),
                0,
                0,
                ExitCode::Ok,
            );
        }
        SyscallResult::new(Default::default(), 0, 0, ExitCode::Err)
    }
}

impl MetadataStorageAPI for HostTestingContext {
    fn metadata_storage_read(&self, slot: &U256) -> SyscallResult<U256> {
        let ctx = self.inner.borrow();
        let account_owner = ctx
            .ownable_account_address
            .expect("expected ownable account address");
        let value = ctx
            .metadata_storage
            .get(&(account_owner, *slot))
            .unwrap_or(&U256::ZERO)
            .clone();
        SyscallResult::new(value, 0, 0, ExitCode::Ok)
    }

    fn metadata_storage_write(&mut self, slot: &U256, value: U256) -> SyscallResult<()> {
        let mut ctx = self.inner.borrow_mut();
        let account_owner = ctx
            .ownable_account_address
            .expect("expected ownable account address");
        ctx.metadata_storage.insert((account_owner, *slot), value);
        SyscallResult::new((), 0, 0, ExitCode::Ok)
    }
}

impl SharedAPI for HostTestingContext {
    fn context(&self) -> impl ContextReader {
        self.inner.borrow().shared_context_input_v1.clone()
    }

    fn keccak256(&self, data: &[u8]) -> B256 {
        RuntimeContextWrapper::keccak256(data)
    }

    fn sha256(data: &[u8]) -> B256 {
        RuntimeContextWrapper::sha256(data)
    }

    fn blake3(data: &[u8]) -> B256 {
        RuntimeContextWrapper::blake3(data)
    }

    fn poseidon(parameters: u32, endianness: u32, data: &[u8]) -> Result<B256, ExitCode> {
        RuntimeContextWrapper::poseidon(parameters, endianness, data)
    }

    fn secp256k1_recover(digest: &B256, sig: &[u8; 64], rec_id: u8) -> Option<[u8; 65]> {
        RuntimeContextWrapper::secp256k1_recover(digest, sig, rec_id)
    }
    fn curve25519_edwards_decompress_validate(p: &[u8; 32]) -> bool {
        RuntimeContextWrapper::curve25519_edwards_decompress_validate(p)
    }
    fn curve25519_edwards_add(p: &mut [u8; 32], q: &[u8; 32]) -> bool {
        RuntimeContextWrapper::curve25519_edwards_add(p, q)
    }
    fn curve25519_edwards_sub(p: &mut [u8; 32], q: &[u8; 32]) -> bool {
        RuntimeContextWrapper::curve25519_edwards_sub(p, q)
    }
    fn curve25519_edwards_mul(p: &mut [u8; 32], q: &[u8; 32]) -> bool {
        RuntimeContextWrapper::curve25519_edwards_mul(p, q)
    }
    fn curve25519_edwards_multiscalar_mul(
        pairs: &[([u8; 32], [u8; 32])],
        out: &mut [u8; 32],
    ) -> bool {
        RuntimeContextWrapper::curve25519_edwards_multiscalar_mul(pairs, out)
    }
    fn curve25519_ristretto_decompress_validate(p: &[u8; 32]) -> bool {
        RuntimeContextWrapper::curve25519_ristretto_decompress_validate(p)
    }
    fn curve25519_ristretto_add(p: &mut [u8; 32], q: &[u8; 32]) -> bool {
        RuntimeContextWrapper::curve25519_ristretto_add(p, q)
    }
    fn curve25519_ristretto_sub(p: &mut [u8; 32], q: &[u8; 32]) -> bool {
        RuntimeContextWrapper::curve25519_ristretto_sub(p, q)
    }
    fn curve25519_ristretto_mul(p: &mut [u8; 32], q: &[u8; 32]) -> bool {
        RuntimeContextWrapper::curve25519_ristretto_mul(p, q)
    }
    fn curve25519_ristretto_multiscalar_mul(
        pairs: &[([u8; 32], [u8; 32])],
        out: &mut [u8; 32],
    ) -> bool {
        RuntimeContextWrapper::curve25519_ristretto_multiscalar_mul(pairs, out)
    }
    fn bls12_381_g1_add(p: &mut [u8; 96], q: &[u8; 96]) {
        RuntimeContextWrapper::bls12_381_g1_add(p, q);
    }
    fn bls12_381_g1_msm(pairs: &[([u8; 64], [u8; 64])], out: &mut [u8; 64]) {
        RuntimeContextWrapper::bls12_381_g1_msm(pairs, out);
    }
    fn bls12_381_g2_add(p: &mut [u8; 192], q: &[u8; 192]) {
        RuntimeContextWrapper::bls12_381_g2_add(p, q);
    }
    fn bls12_381_g2_msm(pairs: &[([u8; 64], [u8; 64])], out: &mut [u8; 64]) {
        RuntimeContextWrapper::bls12_381_g2_msm(pairs, out);
    }
    fn bls12_381_pairing(pairs: &[([u8; 64], [u8; 64])], out: &mut [u8; 64]) {
        RuntimeContextWrapper::bls12_381_pairing(pairs, out);
    }
    fn bls12_381_map_fp_to_g1(p: &[u8; 64], out: &mut [u8; 64]) {
        RuntimeContextWrapper::bls12_381_map_fp_to_g1(p, out);
    }
    fn bls12_381_map_fp2_to_g2(p: &[u8; 64], out: &mut [u8; 64]) {
        RuntimeContextWrapper::bls12_381_map_fp2_to_g2(p, out);
    }
    fn bn254_add(p: &mut [u8; 64], q: &[u8; 64]) {
        RuntimeContextWrapper::bn254_add(p, q);
    }
    fn bn254_double(p: &mut [u8; 64]) {
        RuntimeContextWrapper::bn254_double(p);
    }
    fn bn254_mul(p: &mut [u8; 64], q: &[u8; 32]) {
        RuntimeContextWrapper::bn254_mul(p, q);
    }
    fn bn254_multi_pairing(elements: &[([u8; 64], [u8; 128])]) -> [u8; 32] {
        RuntimeContextWrapper::bn254_multi_pairing(elements)
    }
    fn bn254_g1_compress(
        point: &[u8; BN254_G1_POINT_DECOMPRESSED_SIZE],
    ) -> Result<[u8; BN254_G1_POINT_COMPRESSED_SIZE], ExitCode> {
        RuntimeContextWrapper::bn254_g1_compress(point)
    }
    fn bn254_g1_decompress(
        point: &[u8; BN254_G1_POINT_COMPRESSED_SIZE],
    ) -> Result<[u8; BN254_G1_POINT_DECOMPRESSED_SIZE], ExitCode> {
        RuntimeContextWrapper::bn254_g1_decompress(point)
    }
    fn bn254_g2_compress(
        point: &[u8; BN254_G2_POINT_DECOMPRESSED_SIZE],
    ) -> Result<[u8; BN254_G2_POINT_COMPRESSED_SIZE], ExitCode> {
        RuntimeContextWrapper::bn254_g2_compress(point)
    }
    fn bn254_g2_decompress(
        point: &[u8; BN254_G2_POINT_COMPRESSED_SIZE],
    ) -> Result<[u8; BN254_G2_POINT_DECOMPRESSED_SIZE], ExitCode> {
        RuntimeContextWrapper::bn254_g2_decompress(point)
    }
    fn bn254_fp_mul(p: &mut [u8; 64], q: &[u8; 32]) {
        RuntimeContextWrapper::bn254_fp_mul(p, q);
    }
    fn bn254_fp2_mul(p: &mut [u8; 64], q: &[u8; 32]) {
        RuntimeContextWrapper::bn254_fp2_mul(p, q);
    }
    fn big_mod_exp(base: &[u8], exponent: &[u8], modulus: &mut [u8]) -> Result<(), ExitCode> {
        RuntimeContextWrapper::big_mod_exp(base, exponent, modulus)
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

    fn native_exit(&self, exit_code: ExitCode) -> ! {
        self.inner.borrow().native_sdk.exit(exit_code);
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
