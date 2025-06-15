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
    IsColdAccess,
    SharedAPI,
    SharedContextInputV1,
    StorageAPI,
    SyscallResult,
    B256,
    STATE_MAIN,
    SYSCALL_ID_BALANCE,
    SYSCALL_ID_CALL,
    SYSCALL_ID_CALL_CODE,
    SYSCALL_ID_CODE_COPY,
    SYSCALL_ID_CODE_HASH,
    SYSCALL_ID_CODE_SIZE,
    SYSCALL_ID_CREATE,
    SYSCALL_ID_CREATE2,
    SYSCALL_ID_DELEGATED_STORAGE,
    SYSCALL_ID_DELEGATE_CALL,
    SYSCALL_ID_DESTROY_ACCOUNT,
    SYSCALL_ID_EMIT_LOG,
    SYSCALL_ID_PREIMAGE_COPY,
    SYSCALL_ID_PREIMAGE_SIZE,
    SYSCALL_ID_SELF_BALANCE,
    SYSCALL_ID_STATIC_CALL,
    SYSCALL_ID_STORAGE_READ,
    SYSCALL_ID_STORAGE_WRITE,
    SYSCALL_ID_TRANSIENT_READ,
    SYSCALL_ID_TRANSIENT_WRITE,
    SYSCALL_ID_WRITE_PREIMAGE,
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
        self.native_sdk.exit(0);
    }
}

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

/// SharedContextImpl always created from input
impl<API: NativeAPI> SharedAPI for SharedContextImpl<API> {
    fn context(&self) -> impl ContextReader {
        ContextReaderImpl(self.shared_context_ref())
    }

    fn keccak256(&self, data: &[u8]) -> B256 {
        API::keccak256(data)
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

    fn exit(&self, exit_code: ExitCode) -> ! {
        self.native_sdk.exit(exit_code.into_i32())
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
        if SyscallResult::is_ok(exit_code) {
            self.native_sdk.read_output(&mut output, 0);
        };
        let value = U256::from_le_slice(&output);
        SyscallResult::new(value, fuel_consumed, fuel_refunded, exit_code)
    }

    fn delegated_storage(
        &self,
        address: &Address,
        slot: &U256,
    ) -> SyscallResult<(U256, IsColdAccess, IsAccountEmpty)> {
        let mut input = [0u8; 20 + 32];
        input[..20].copy_from_slice(address.as_slice());
        input[20..].copy_from_slice(slot.as_le_slice());
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk
                .exec(SYSCALL_ID_DELEGATED_STORAGE, &input, None, STATE_MAIN);
        let mut output = [0u8; U256::BYTES + 1 + 1];
        if !SyscallResult::is_err(exit_code) {
            self.native_sdk.read_output(&mut output, 0);
        };
        let value = U256::from_le_slice(&output[..32]);
        let is_cold_access = output[32] != 0x0;
        let is_empty = output[33] != 0x0;
        SyscallResult::new(
            (value, is_cold_access, is_empty),
            fuel_consumed,
            fuel_refunded,
            exit_code,
        )
    }

    fn preimage_copy(&self, hash: &B256) -> SyscallResult<Bytes> {
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk
                .exec(SYSCALL_ID_PREIMAGE_COPY, hash.as_ref(), None, STATE_MAIN);
        let value = if SyscallResult::is_ok(exit_code) {
            self.native_sdk.return_data()
        } else {
            Bytes::new()
        };
        SyscallResult::new(value, fuel_consumed, fuel_refunded, exit_code)
    }

    fn preimage_size(&self, hash: &B256) -> SyscallResult<u32> {
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk
                .exec(SYSCALL_ID_PREIMAGE_SIZE, hash.as_ref(), None, STATE_MAIN);
        let mut output: [u8; 4] = [0u8; 4];
        if SyscallResult::is_ok(exit_code) {
            self.native_sdk.read_output(&mut output, 0);
        }
        let value = LittleEndian::read_u32(&output);
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

    fn write_preimage(&mut self, preimage: Bytes) -> SyscallResult<B256> {
        let (fuel_consumed, fuel_refunded, exit_code) = self.native_sdk.exec(
            SYSCALL_ID_WRITE_PREIMAGE,
            preimage.as_ref(),
            None,
            STATE_MAIN,
        );
        let mut output = [0u8; B256::len_bytes()];
        if SyscallResult::is_ok(exit_code) {
            self.native_sdk.read_output(&mut output, 0);
        };
        let value = B256::from_slice(&output);
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
