mod context;

use crate::{
    alloc_slice,
    byteorder::{ByteOrder, LittleEndian},
    shared::context::ContextReaderImpl,
};
use alloc::vec;
use core::cell::RefCell;
use fluentbase_types::{
    native_api::NativeAPI,
    Address,
    Bytes,
    ContextReader,
    ExitCode,
    IsAccountEmpty,
    IsAccountOwnable,
    IsColdAccess,
    MetadataAPI,
    SharedAPI,
    SharedContextInputV1,
    StorageAPI,
    SyscallResult,
    B256,
    BN254_G1_POINT_COMPRESSED_SIZE,
    BN254_G1_POINT_DECOMPRESSED_SIZE,
    BN254_G2_POINT_COMPRESSED_SIZE,
    BN254_G2_POINT_DECOMPRESSED_SIZE,
    STATE_MAIN,
    SYSCALL_ID_BALANCE,
    SYSCALL_ID_BLOCK_HASH,
    SYSCALL_ID_CALL,
    SYSCALL_ID_CALL_CODE,
    SYSCALL_ID_CODE_COPY,
    SYSCALL_ID_CODE_HASH,
    SYSCALL_ID_CODE_SIZE,
    SYSCALL_ID_CREATE,
    SYSCALL_ID_CREATE2,
    SYSCALL_ID_DELEGATE_CALL,
    SYSCALL_ID_DESTROY_ACCOUNT,
    SYSCALL_ID_EMIT_LOG,
    SYSCALL_ID_METADATA_COPY,
    SYSCALL_ID_METADATA_CREATE,
    SYSCALL_ID_METADATA_SIZE,
    SYSCALL_ID_METADATA_WRITE,
    SYSCALL_ID_SELF_BALANCE,
    SYSCALL_ID_STATIC_CALL,
    SYSCALL_ID_STORAGE_READ,
    SYSCALL_ID_STORAGE_WRITE,
    SYSCALL_ID_TRANSIENT_READ,
    SYSCALL_ID_TRANSIENT_WRITE,
    U256,
};

pub struct SharedContextImpl<API: NativeAPI> {
    native_sdk: API,
    shared_context_input_v1: RefCell<Option<SharedContextInputV1>>,
}

impl<API: NativeAPI> SharedContextImpl<API> {
    pub fn new(native_sdk: API) -> Self {
        Self {
            native_sdk,
            shared_context_input_v1: RefCell::new(None),
        }
    }

    fn shared_context_ref(&self) -> &RefCell<Option<SharedContextInputV1>> {
        let mut shared_context_input_v1 = self.shared_context_input_v1.borrow_mut();
        shared_context_input_v1.get_or_insert_with(|| {
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
        });
        &self.shared_context_input_v1
    }

    pub fn commit_changes_and_exit(&mut self) -> ! {
        self.native_sdk.exit(ExitCode::Ok);
    }
}

/// SharedContextImpl always created from input
impl<API: NativeAPI> StorageAPI for SharedContextImpl<API> {
    fn write_storage(&mut self, slot: U256, value: U256) -> SyscallResult<()> {
        let mut input = [0u8; U256::BYTES + U256::BYTES];
        unsafe {
            core::ptr::copy(
                slot.as_limbs().as_ptr() as *mut u8,
                input.as_mut_ptr(),
                U256::BYTES,
            );
            core::ptr::copy(
                value.as_limbs().as_ptr() as *mut u8,
                input.as_mut_ptr().add(U256::BYTES),
                U256::BYTES,
            );
        }
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk
                .exec(SYSCALL_ID_STORAGE_WRITE, &input, None, STATE_MAIN);
        SyscallResult::new((), fuel_consumed, fuel_refunded, exit_code)
    }

    fn storage(&self, slot: &U256) -> SyscallResult<U256> {
        let (fuel_consumed, fuel_refunded, exit_code) = self.native_sdk.exec(
            SYSCALL_ID_STORAGE_READ,
            slot.as_le_slice(),
            None,
            STATE_MAIN,
        );
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
        let mut buffer = vec![0u8; 20 + 4 + metadata.len()];
        buffer[0..20].copy_from_slice(address.as_slice());
        LittleEndian::write_u32(&mut buffer[20..24], offset);
        buffer[24..].copy_from_slice(&metadata);
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk
                .exec(SYSCALL_ID_METADATA_WRITE, &buffer, None, STATE_MAIN);
        SyscallResult::new((), fuel_consumed, fuel_refunded, exit_code)
    }

    fn metadata_size(
        &self,
        address: &Address,
    ) -> SyscallResult<(u32, IsAccountOwnable, IsColdAccess, IsAccountEmpty)> {
        let (fuel_consumed, fuel_refunded, exit_code) = self.native_sdk.exec(
            SYSCALL_ID_METADATA_SIZE,
            address.as_slice(),
            None,
            STATE_MAIN,
        );
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
        let mut buffer = vec![0u8; U256::BYTES + metadata.len()];
        buffer[0..32].copy_from_slice(salt.to_be_bytes::<32>().as_slice());
        buffer[32..].copy_from_slice(&metadata);
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk
                .exec(SYSCALL_ID_METADATA_CREATE, &buffer, None, STATE_MAIN);
        SyscallResult::new((), fuel_consumed, fuel_refunded, exit_code)
    }

    fn metadata_copy(&self, address: &Address, offset: u32, length: u32) -> SyscallResult<Bytes> {
        let mut buffer = [0u8; 20 + 4 + 4];
        buffer[0..20].copy_from_slice(address.as_slice());
        LittleEndian::write_u32(&mut buffer[20..24], offset);
        LittleEndian::write_u32(&mut buffer[24..28], length);
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk
                .exec(SYSCALL_ID_METADATA_COPY, &buffer, None, STATE_MAIN);
        let value = self.native_sdk.return_data();
        SyscallResult::new(value, fuel_consumed, fuel_refunded, exit_code)
    }
}

/// SharedContextImpl always created from input
impl<API: NativeAPI> SharedAPI for SharedContextImpl<API> {
    fn context(&self) -> impl ContextReader {
        ContextReaderImpl(self.shared_context_ref())
    }

    fn keccak256(&self, data: &[u8]) -> B256 {
        API::keccak256(data)
    }

    fn sha256(data: &[u8]) -> B256 {
        API::sha256(data)
    }

    fn secp256k1_recover(digest: &B256, sig: &[u8; 64], rec_id: u8) -> Option<[u8; 65]> {
        API::secp256k1_recover(digest, sig, rec_id)
    }

    fn ed25519_edwards_decompress_validate(p: &[u8; 32]) -> bool {
        API::ed25519_edwards_decompress_validate(p)
    }

    fn ed25519_edwards_add(p: &mut [u8; 32], q: &[u8; 32]) -> bool {
        API::ed25519_edwards_add(p, q)
    }

    fn ed25519_edwards_sub(p: &mut [u8; 32], q: &[u8; 32]) -> bool {
        API::ed25519_edwards_sub(p, q)
    }

    fn ed25519_edwards_mul(p: &mut [u8; 32], q: &[u8; 32]) -> bool {
        API::ed25519_edwards_mul(p, q)
    }

    fn ed25519_edwards_multiscalar_mul(pairs: &[([u8; 32], [u8; 32])], out: &mut [u8; 32]) -> bool {
        API::ed25519_edwards_multiscalar_mul(pairs, out)
    }

    fn ed25519_ristretto_decompress_validate(p: &[u8; 32]) -> bool {
        API::ed25519_ristretto_decompress_validate(p)
    }

    fn ed25519_ristretto_add(p: &mut [u8; 32], q: &[u8; 32]) -> bool {
        API::ed25519_ristretto_add(p, q)
    }

    fn ed25519_ristretto_sub(p: &mut [u8; 32], q: &[u8; 32]) -> bool {
        API::ed25519_ristretto_sub(p, q)
    }

    fn ed25519_ristretto_mul(p: &mut [u8; 32], q: &[u8; 32]) -> bool {
        API::ed25519_ristretto_mul(p, q)
    }

    fn ed25519_ristretto_multiscalar_mul(
        pairs: &[([u8; 32], [u8; 32])],
        out: &mut [u8; 32],
    ) -> bool {
        API::ed25519_ristretto_multiscalar_mul(pairs, out)
    }

    fn bn254_add(p: &mut [u8; 64], q: &[u8; 64]) {
        API::bn254_add(p, q)
    }

    fn bn254_double(p: &mut [u8; 64]) {
        API::bn254_double(p)
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

    fn write_transient_storage(&mut self, slot: U256, value: U256) -> SyscallResult<()> {
        let mut input: [u8; 64] = [0u8; 64];
        if !slot.is_zero() {
            input[0..32].copy_from_slice(slot.as_le_slice());
        }
        if !value.is_zero() {
            input[32..64].copy_from_slice(value.as_le_slice());
        }
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk
                .exec(SYSCALL_ID_TRANSIENT_WRITE, &input, None, STATE_MAIN);
        SyscallResult::new((), fuel_consumed, fuel_refunded, exit_code)
    }

    fn transient_storage(&self, slot: &U256) -> SyscallResult<U256> {
        let (fuel_consumed, fuel_refunded, exit_code) = self.native_sdk.exec(
            SYSCALL_ID_TRANSIENT_READ,
            slot.as_le_slice(),
            None,
            STATE_MAIN,
        );
        let mut output = [0u8; U256::BYTES];
        if SyscallResult::<()>::is_ok(exit_code) {
            self.native_sdk.read_output(&mut output, 0);
        };
        let value = U256::from_le_slice(&output);
        SyscallResult::new(value, fuel_consumed, fuel_refunded, exit_code)
    }

    fn emit_log(&mut self, topics: &[B256], data: &[u8]) -> SyscallResult<()> {
        let mut buffer = vec![0u8; 1 + topics.len() * B256::len_bytes()];
        assert!(topics.len() <= 4);
        buffer[0] = topics.len() as u8;
        for (i, topic) in topics.iter().enumerate() {
            buffer[(1 + i * B256::len_bytes())..(1 + i * B256::len_bytes() + B256::len_bytes())]
                .copy_from_slice(topic.as_slice());
        }
        buffer.extend_from_slice(data);
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk
                .exec(SYSCALL_ID_EMIT_LOG, &buffer, None, STATE_MAIN);
        SyscallResult::new((), fuel_consumed, fuel_refunded, exit_code)
    }

    fn self_balance(&self) -> SyscallResult<U256> {
        let (fuel_consumed, fuel_refunded, exit_code) = self.native_sdk.exec(
            SYSCALL_ID_SELF_BALANCE,
            Default::default(),
            None,
            STATE_MAIN,
        );
        let mut output = [0u8; U256::BYTES];
        if SyscallResult::is_ok(exit_code) {
            self.native_sdk.read_output(&mut output, 0);
        };
        let value = U256::from_le_slice(&output);
        SyscallResult::new(value, fuel_consumed, fuel_refunded, exit_code)
    }

    fn balance(&self, address: &Address) -> SyscallResult<U256> {
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk
                .exec(SYSCALL_ID_BALANCE, address.as_slice(), None, STATE_MAIN);
        let mut output = [0u8; U256::BYTES];
        if SyscallResult::is_ok(exit_code) {
            self.native_sdk.read_output(&mut output, 0);
        };
        let value = U256::from_le_slice(&output);
        SyscallResult::new(value, fuel_consumed, fuel_refunded, exit_code)
    }

    fn block_hash(&self, block_number: u64) -> SyscallResult<B256> {
        let (fuel_consumed, fuel_refunded, exit_code) = self.native_sdk.exec(
            SYSCALL_ID_BLOCK_HASH,
            &block_number.to_le_bytes(),
            None,
            STATE_MAIN,
        );
        let mut output = [0u8; B256::len_bytes()];
        if SyscallResult::is_ok(exit_code) {
            self.native_sdk.read_output(&mut output, 0);
        }
        let value = B256::from_slice(&output);
        SyscallResult::new(value, fuel_consumed, fuel_refunded, exit_code)
    }

    fn code_size(&self, address: &Address) -> SyscallResult<u32> {
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk
                .exec(SYSCALL_ID_CODE_SIZE, address.as_slice(), None, STATE_MAIN);
        let mut output: [u8; 4] = [0u8; 4];
        if SyscallResult::is_ok(exit_code) {
            self.native_sdk.read_output(&mut output, 0);
        }
        let value = u32::from_le_bytes(output);
        SyscallResult::new(value, fuel_consumed, fuel_refunded, exit_code)
    }

    fn code_hash(&self, address: &Address) -> SyscallResult<B256> {
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk
                .exec(SYSCALL_ID_CODE_HASH, address.as_slice(), None, STATE_MAIN);
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
        let mut input = [0u8; 20 + 8 * 2];
        input[0..20].copy_from_slice(address.as_slice());
        LittleEndian::write_u64(&mut input[20..28], code_offset);
        LittleEndian::write_u64(&mut input[28..36], code_length);
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk
                .exec(SYSCALL_ID_CODE_COPY, &input, None, STATE_MAIN);
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
        let (buffer, code_hash) = if let Some(salt) = salt {
            let buffer = alloc_slice(32 + 32 + init_code.len());
            buffer[0..32].copy_from_slice(value.as_le_slice());
            buffer[32..64].copy_from_slice(salt.as_le_slice());
            buffer[64..].copy_from_slice(init_code);
            (buffer, SYSCALL_ID_CREATE2)
        } else {
            let buffer = alloc_slice(32 + init_code.len());
            buffer[0..32].copy_from_slice(value.as_le_slice());
            buffer[32..].copy_from_slice(init_code);
            (buffer, SYSCALL_ID_CREATE)
        };
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk.exec(code_hash, &buffer, None, STATE_MAIN);
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
        let mut buffer = vec![0u8; 20 + 32];
        buffer[0..20].copy_from_slice(address.as_slice());
        if !value.is_zero() {
            buffer[20..52].copy_from_slice(value.as_le_slice());
        }
        buffer.extend_from_slice(input);
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk
                .exec(SYSCALL_ID_CALL, &buffer, fuel_limit, STATE_MAIN);
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
        let mut buffer = vec![0u8; 20 + 32];
        buffer[0..20].copy_from_slice(address.as_slice());
        if !value.is_zero() {
            buffer[20..52].copy_from_slice(value.as_le_slice());
        }
        buffer.extend_from_slice(input);
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk
                .exec(SYSCALL_ID_CALL_CODE, &buffer, fuel_limit, STATE_MAIN);
        let value = self.native_sdk.return_data();
        SyscallResult::new(value, fuel_consumed, fuel_refunded, exit_code)
    }

    fn delegate_call(
        &mut self,
        address: Address,
        input: &[u8],
        fuel_limit: Option<u64>,
    ) -> SyscallResult<Bytes> {
        let mut buffer = vec![0u8; 20];
        buffer[0..20].copy_from_slice(address.as_slice());
        buffer.extend_from_slice(input);
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk
                .exec(SYSCALL_ID_DELEGATE_CALL, &buffer, fuel_limit, STATE_MAIN);
        let value = self.native_sdk.return_data();
        SyscallResult::new(value, fuel_consumed, fuel_refunded, exit_code)
    }

    fn static_call(
        &mut self,
        address: Address,
        input: &[u8],
        fuel_limit: Option<u64>,
    ) -> SyscallResult<Bytes> {
        let mut buffer = vec![0u8; 20];
        buffer[0..20].copy_from_slice(address.as_slice());
        buffer.extend_from_slice(input);
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk
                .exec(SYSCALL_ID_STATIC_CALL, &buffer, fuel_limit, STATE_MAIN);
        let value = self.native_sdk.return_data();
        SyscallResult::new(value, fuel_consumed, fuel_refunded, exit_code)
    }

    fn destroy_account(&mut self, address: Address) -> SyscallResult<()> {
        let (fuel_consumed, fuel_refunded, exit_code) = self.native_sdk.exec(
            SYSCALL_ID_DESTROY_ACCOUNT,
            address.as_slice(),
            None,
            STATE_MAIN,
        );
        SyscallResult::new((), fuel_consumed, fuel_refunded, exit_code)
    }
}
