mod context;

use crate::{
    byteorder::{ByteOrder, LittleEndian},
    syscall::*,
};
use core::cell::OnceCell;
use fluentbase_types::{
    Address, BytecodeOrHash, Bytes, ContextReader, ExitCode, IsAccountEmpty, IsAccountOwnable,
    IsColdAccess, MetadataAPI, MetadataStorageAPI, NativeAPI, SharedAPI, SharedContextInputV1,
    StorageAPI, SyscallResult, B256, BN254_G1_POINT_COMPRESSED_SIZE,
    BN254_G1_POINT_DECOMPRESSED_SIZE, BN254_G2_POINT_COMPRESSED_SIZE,
    BN254_G2_POINT_DECOMPRESSED_SIZE, U256,
};

pub struct SharedContextImpl<API: NativeAPI> {
    native_sdk: API,
    shared_context_input_v1: OnceCell<SharedContextInputV1>,
}

impl<API: NativeAPI> SharedContextImpl<API> {
    pub fn new(native_sdk: API) -> Self {
        Self {
            native_sdk,
            shared_context_input_v1: OnceCell::new(),
        }
    }

    pub fn into_native_sdk(self) -> API {
        self.native_sdk
    }

    fn shared_context_ref(&self) -> &SharedContextInputV1 {
        self.shared_context_input_v1.get_or_init(|| {
            let input_size = self.native_sdk.input_size() as usize;
            assert!(
                input_size >= SharedContextInputV1::SIZE,
                "malformed input header"
            );
            let mut header_input: [u8; SharedContextInputV1::SIZE] =
                [0u8; SharedContextInputV1::SIZE];
            self.native_sdk.read(&mut header_input, 0);
            let result = SharedContextInputV1::decode_from_slice(&header_input)
                .unwrap_or_else(|_| unreachable!("fluentbase: malformed input header"));
            result
        })
    }

    pub fn commit_changes_and_exit(&mut self) -> ! {
        self.native_sdk.exit(ExitCode::Ok);
    }
}

/// SharedContextImpl always created from input
impl<API: NativeAPI> StorageAPI for SharedContextImpl<API> {
    fn write_storage(&mut self, slot: U256, value: U256) -> SyscallResult<()> {
        let (fuel_consumed, fuel_refunded, exit_code) = self.native_sdk.write_storage(slot, value);
        SyscallResult::new((), fuel_consumed, fuel_refunded, exit_code)
    }

    fn storage(&self, slot: &U256) -> SyscallResult<U256> {
        let (fuel_consumed, fuel_refunded, exit_code) = self.native_sdk.storage(slot);
        let mut output = [0u8; U256::BYTES];
        if SyscallResult::is_ok(exit_code) {
            self.native_sdk.read_output(&mut output, 0);
        };
        let value = U256::from_le_slice(&output);
        SyscallResult::new(value, fuel_consumed, fuel_refunded, exit_code)
    }
}

impl<API: NativeAPI> MetadataAPI for SharedContextImpl<API> {
    fn metadata_write(
        &mut self,
        address: &Address,
        offset: u32,
        metadata: Bytes,
    ) -> SyscallResult<()> {
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk.metadata_write(address, offset, metadata);
        SyscallResult::new((), fuel_consumed, fuel_refunded, exit_code)
    }

    fn metadata_size(
        &self,
        address: &Address,
    ) -> SyscallResult<(u32, IsAccountOwnable, IsColdAccess, IsAccountEmpty)> {
        let (fuel_consumed, fuel_refunded, exit_code) = self.native_sdk.metadata_size(address);
        let mut output: [u8; 7] = [0u8; 7];
        if SyscallResult::<()>::is_ok(exit_code) {
            self.native_sdk.read_output(&mut output, 0);
        };
        let value = LittleEndian::read_u32(&output[0..4]);
        let is_account_ownable = output[4] != 0x00;
        let is_cold_access = output[5] != 0x00;
        let is_account_empty = output[6] != 0x00;
        SyscallResult::new(
            (value, is_account_ownable, is_cold_access, is_account_empty),
            fuel_consumed,
            fuel_refunded,
            exit_code,
        )
    }

    fn metadata_create(&mut self, salt: &U256, metadata: Bytes) -> SyscallResult<()> {
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk.metadata_create(salt, metadata);
        SyscallResult::new((), fuel_consumed, fuel_refunded, exit_code)
    }

    fn metadata_copy(&self, address: &Address, offset: u32, length: u32) -> SyscallResult<Bytes> {
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk.metadata_copy(address, offset, length);
        let value = self.native_sdk.return_data();
        SyscallResult::new(value, fuel_consumed, fuel_refunded, exit_code)
    }
}

impl<API: NativeAPI> MetadataStorageAPI for SharedContextImpl<API> {
    fn metadata_storage_read(&self, slot: &U256) -> SyscallResult<U256> {
        let (fuel_consumed, fuel_refunded, exit_code) = self.native_sdk.metadata_storage_read(slot);
        let value = U256::from_le_slice(&self.native_sdk.return_data());
        SyscallResult::new(value, fuel_consumed, fuel_refunded, exit_code)
    }

    fn metadata_storage_write(&mut self, slot: &U256, value: U256) -> SyscallResult<()> {
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk.metadata_storage_write(slot, value);
        SyscallResult::new((), fuel_consumed, fuel_refunded, exit_code)
    }
}

/// SharedContextImpl always created from input
impl<API: NativeAPI> SharedAPI for SharedContextImpl<API> {
    fn context(&self) -> impl ContextReader {
        self.shared_context_ref()
    }

    fn keccak256(&self, data: &[u8]) -> B256 {
        API::keccak256(data)
    }

    fn sha256(data: &[u8]) -> B256 {
        API::sha256(data)
    }

    fn blake3(data: &[u8]) -> B256 {
        API::blake3(data)
    }

    fn poseidon(parameters: u32, endianness: u32, data: &[u8]) -> Result<B256, ExitCode> {
        API::poseidon(parameters, endianness, data)
    }

    fn secp256k1_recover(digest: &B256, sig: &[u8; 64], rec_id: u8) -> Option<[u8; 65]> {
        API::secp256k1_recover(digest, sig, rec_id)
    }

    fn curve25519_edwards_decompress_validate(p: &[u8; 32]) -> bool {
        API::curve25519_edwards_decompress_validate(p)
    }

    fn curve25519_edwards_add(p: &mut [u8; 32], q: &[u8; 32]) -> bool {
        API::curve25519_edwards_add(p, q)
    }

    fn curve25519_edwards_sub(p: &mut [u8; 32], q: &[u8; 32]) -> bool {
        API::curve25519_edwards_sub(p, q)
    }

    fn curve25519_edwards_mul(p: &mut [u8; 32], q: &[u8; 32]) -> bool {
        API::curve25519_edwards_mul(p, q)
    }

    fn curve25519_edwards_multiscalar_mul(
        pairs: &[([u8; 32], [u8; 32])],
        out: &mut [u8; 32],
    ) -> bool {
        API::curve25519_edwards_multiscalar_mul(pairs, out)
    }

    fn curve25519_ristretto_decompress_validate(p: &[u8; 32]) -> bool {
        API::curve25519_ristretto_decompress_validate(p)
    }

    fn curve25519_ristretto_add(p: &mut [u8; 32], q: &[u8; 32]) -> bool {
        API::curve25519_ristretto_add(p, q)
    }

    fn curve25519_ristretto_sub(p: &mut [u8; 32], q: &[u8; 32]) -> bool {
        API::curve25519_ristretto_sub(p, q)
    }

    fn curve25519_ristretto_mul(p: &mut [u8; 32], q: &[u8; 32]) -> bool {
        API::curve25519_ristretto_mul(p, q)
    }

    fn curve25519_ristretto_multiscalar_mul(
        pairs: &[([u8; 32], [u8; 32])],
        out: &mut [u8; 32],
    ) -> bool {
        API::curve25519_ristretto_multiscalar_mul(pairs, out)
    }

    fn bn254_add(p: &mut [u8; 64], q: &[u8; 64]) {
        API::bn254_add(p, q)
    }

    fn bn254_mul(p: &mut [u8; 64], q: &[u8; 32]) {
        API::bn254_mul(p, q)
    }

    fn bn254_multi_pairing(elements: &[([u8; 64], [u8; 128])]) -> [u8; 32] {
        API::bn254_multi_pairing(elements)
    }

    fn bn254_g1_compress(
        point: &[u8; BN254_G1_POINT_DECOMPRESSED_SIZE],
    ) -> Result<[u8; BN254_G1_POINT_COMPRESSED_SIZE], ExitCode> {
        API::bn254_g1_compress(point)
    }

    fn bn254_g1_decompress(
        point: &[u8; BN254_G1_POINT_COMPRESSED_SIZE],
    ) -> Result<[u8; BN254_G1_POINT_DECOMPRESSED_SIZE], ExitCode> {
        API::bn254_g1_decompress(point)
    }

    fn bn254_g2_compress(
        point: &[u8; BN254_G2_POINT_DECOMPRESSED_SIZE],
    ) -> Result<[u8; BN254_G2_POINT_COMPRESSED_SIZE], ExitCode> {
        API::bn254_g2_compress(point)
    }

    fn bn254_g2_decompress(
        point: &[u8; BN254_G2_POINT_COMPRESSED_SIZE],
    ) -> Result<[u8; BN254_G2_POINT_DECOMPRESSED_SIZE], ExitCode> {
        API::bn254_g2_decompress(point)
    }

    fn bn254_double(p: &mut [u8; 64]) {
        API::bn254_double(p)
    }

    fn bn254_fp_mul(p: &mut [u8; 64], q: &[u8; 32]) {
        API::bn254_fp_mul(p, q)
    }

    fn bn254_fp2_mul(p: &mut [u8; 64], q: &[u8; 32]) {
        API::bn254_fp2_mul(p, q)
    }

    fn big_mod_exp(base: &[u8], exponent: &[u8], modulus: &mut [u8]) -> Result<(), ExitCode> {
        API::big_mod_exp(base, exponent, modulus)
    }

    fn read(&self, target: &mut [u8], offset: u32) {
        self.native_sdk
            .read(target, SharedContextInputV1::SIZE as u32 + offset)
    }

    fn input_size(&self) -> u32 {
        let input_size = self.native_sdk.input_size();
        if input_size < SharedContextInputV1::SIZE as u32 {
            unsafe {
                core::hint::unreachable_unchecked();
            }
        }
        unsafe { input_size.unchecked_sub(SharedContextInputV1::SIZE as u32) }
    }

    fn bytes_input(&self) -> Bytes {
        self.native_sdk.input().slice(SharedContextInputV1::SIZE..)
    }

    fn read_context(&self, target: &mut [u8], offset: u32) {
        self.native_sdk.read(target, offset)
    }

    fn charge_fuel_manually(&self, fuel_consumed: u64, fuel_refunded: i64) {
        self.native_sdk
            .charge_fuel_manually(fuel_consumed, fuel_refunded);
    }

    fn fuel(&self) -> u64 {
        self.native_sdk.fuel()
    }

    fn write(&mut self, output: &[u8]) {
        self.native_sdk.write(output);
    }

    fn native_exit(&self, exit_code: ExitCode) -> ! {
        self.native_sdk.exit(exit_code)
    }

    fn native_exec(
        &self,
        code_hash: B256,
        input: &[u8],
        fuel_limit: Option<u64>,
        state: u32,
    ) -> (u64, i64, i32) {
        self.native_sdk
            .exec(BytecodeOrHash::Hash(code_hash), input, fuel_limit, state)
    }

    fn return_data(&self) -> Bytes {
        self.native_sdk.return_data()
    }

    fn write_transient_storage(&mut self, slot: U256, value: U256) -> SyscallResult<()> {
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk.write_transient_storage(slot, value);
        SyscallResult::new((), fuel_consumed, fuel_refunded, exit_code)
    }

    fn transient_storage(&self, slot: &U256) -> SyscallResult<U256> {
        let (fuel_consumed, fuel_refunded, exit_code) = self.native_sdk.transient_storage(slot);
        let mut output = [0u8; U256::BYTES];
        if SyscallResult::<()>::is_ok(exit_code) {
            self.native_sdk.read_output(&mut output, 0);
        };
        let value = U256::from_le_slice(&output);
        SyscallResult::new(value, fuel_consumed, fuel_refunded, exit_code)
    }

    fn emit_log(&mut self, topics: &[B256], data: &[u8]) -> SyscallResult<()> {
        let (fuel_consumed, fuel_refunded, exit_code) = self.native_sdk.emit_log(topics, data);
        SyscallResult::new((), fuel_consumed, fuel_refunded, exit_code)
    }

    fn self_balance(&self) -> SyscallResult<U256> {
        let (fuel_consumed, fuel_refunded, exit_code) = self.native_sdk.self_balance();
        let mut output = [0u8; U256::BYTES];
        if SyscallResult::is_ok(exit_code) {
            self.native_sdk.read_output(&mut output, 0);
        };
        let value = U256::from_le_slice(&output);
        SyscallResult::new(value, fuel_consumed, fuel_refunded, exit_code)
    }

    fn balance(&self, address: &Address) -> SyscallResult<U256> {
        let (fuel_consumed, fuel_refunded, exit_code) = self.native_sdk.balance(address);
        let mut output = [0u8; U256::BYTES];
        if SyscallResult::is_ok(exit_code) {
            self.native_sdk.read_output(&mut output, 0);
        };
        let value = U256::from_le_slice(&output);
        SyscallResult::new(value, fuel_consumed, fuel_refunded, exit_code)
    }

    fn block_hash(&self, block_number: u64) -> SyscallResult<B256> {
        let (fuel_consumed, fuel_refunded, exit_code) = self.native_sdk.block_hash(block_number);
        let mut output = [0u8; B256::len_bytes()];
        if SyscallResult::is_ok(exit_code) {
            self.native_sdk.read_output(&mut output, 0);
        }
        let value = B256::from_slice(&output);
        SyscallResult::new(value, fuel_consumed, fuel_refunded, exit_code)
    }

    fn code_size(&self, address: &Address) -> SyscallResult<u32> {
        let (fuel_consumed, fuel_refunded, exit_code) = self.native_sdk.code_size(address);
        let mut output: [u8; 4] = [0u8; 4];
        if SyscallResult::is_ok(exit_code) {
            self.native_sdk.read_output(&mut output, 0);
        }
        let value = u32::from_le_bytes(output);
        SyscallResult::new(value, fuel_consumed, fuel_refunded, exit_code)
    }

    fn code_hash(&self, address: &Address) -> SyscallResult<B256> {
        let (fuel_consumed, fuel_refunded, exit_code) = self.native_sdk.code_hash(address);
        let mut output = [0u8; B256::len_bytes()];
        if SyscallResult::is_ok(exit_code) {
            self.native_sdk.read_output(&mut output, 0);
        }
        let value = B256::from(output);
        SyscallResult::new(value, fuel_consumed, fuel_refunded, exit_code)
    }

    fn code_copy(
        &self,
        address: &Address,
        code_offset: u64,
        code_length: u64,
    ) -> SyscallResult<Bytes> {
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk.code_copy(address, code_offset, code_length);
        let value = if SyscallResult::is_ok(exit_code) {
            self.native_sdk.return_data()
        } else {
            Bytes::new()
        };
        SyscallResult::new(value, fuel_consumed, fuel_refunded, exit_code)
    }

    fn create(
        &mut self,
        salt: Option<U256>,
        value: &U256,
        init_code: &[u8],
    ) -> SyscallResult<Bytes> {
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk.create(salt, value, init_code);
        let value = self.native_sdk.return_data();
        SyscallResult::new(value, fuel_consumed, fuel_refunded, exit_code)
    }

    fn call(
        &mut self,
        address: Address,
        value: U256,
        input: &[u8],
        fuel_limit: Option<u64>,
    ) -> SyscallResult<Bytes> {
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk.call(address, value, input, fuel_limit);
        let value = self.native_sdk.return_data();
        SyscallResult::new(value, fuel_consumed, fuel_refunded, exit_code)
    }

    fn call_code(
        &mut self,
        address: Address,
        value: U256,
        input: &[u8],
        fuel_limit: Option<u64>,
    ) -> SyscallResult<Bytes> {
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk.call_code(address, value, input, fuel_limit);
        let value = self.native_sdk.return_data();
        SyscallResult::new(value, fuel_consumed, fuel_refunded, exit_code)
    }

    fn delegate_call(
        &mut self,
        address: Address,
        input: &[u8],
        fuel_limit: Option<u64>,
    ) -> SyscallResult<Bytes> {
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk.delegate_call(address, input, fuel_limit);
        let value = self.native_sdk.return_data();
        SyscallResult::new(value, fuel_consumed, fuel_refunded, exit_code)
    }

    fn static_call(
        &mut self,
        address: Address,
        input: &[u8],
        fuel_limit: Option<u64>,
    ) -> SyscallResult<Bytes> {
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk.static_call(address, input, fuel_limit);
        let value = self.native_sdk.return_data();
        SyscallResult::new(value, fuel_consumed, fuel_refunded, exit_code)
    }

    fn destroy_account(&mut self, address: Address) -> SyscallResult<()> {
        let (fuel_consumed, fuel_refunded, exit_code) = self.native_sdk.destroy_account(address);
        SyscallResult::new((), fuel_consumed, fuel_refunded, exit_code)
    }
}
