use crate::{
    syscall::*, Address, BytecodeOrHash, Bytes, ContextReader, CryptoAPI, ExitCode, NativeAPI,
    SharedAPI, SharedContextInputV1, StorageAPI, SyscallResult, B256, U256,
};
use alloc::{borrow::Cow, vec, vec::Vec};
use core::cell::OnceCell;
use fluentbase_types::STATE_MAIN;

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

impl<API: NativeAPI + CryptoAPI> StorageAPI for SharedContextImpl<API> {
    fn write_storage(&mut self, slot: U256, value: U256) -> SyscallResult<()> {
        let mut buffer = [0u8; encode::storage_write_size_hint()];
        encode::storage_write_into(&mut &mut buffer[..], &slot, &value);
        let (fuel_consumed, fuel_refunded, exit_code) = self.native_sdk.exec(
            BytecodeOrHash::Hash(SYSCALL_ID_STORAGE_WRITE),
            Cow::Borrowed(&buffer[..]),
            None,
            STATE_MAIN,
        );
        SyscallResult::new((), fuel_consumed, fuel_refunded, exit_code)
    }

    fn storage(&self, slot: &U256) -> SyscallResult<U256> {
        let mut buffer = [0u8; encode::storage_read_size_hint()];
        encode::storage_read_into(&mut &mut buffer[..], slot);
        let (fuel_consumed, fuel_refunded, exit_code) = self.native_sdk.exec(
            BytecodeOrHash::Hash(SYSCALL_ID_STORAGE_READ),
            Cow::Borrowed(&buffer),
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
impl<API: NativeAPI + CryptoAPI> SharedAPI for SharedContextImpl<API> {
    fn context(&self) -> impl ContextReader {
        self.shared_context_ref()
    }

    fn read(&self, target: &mut [u8], offset: u32) {
        self.native_sdk
            .read(target, SharedContextInputV1::SIZE as u32 + offset)
    }

    fn input_size(&self) -> u32 {
        let input_size = self.native_sdk.input_size();
        if input_size < SharedContextInputV1::SIZE as u32 {
            self.native_sdk.exit(ExitCode::InputOutputOutOfBounds);
        }
        unsafe { input_size.unchecked_sub(SharedContextInputV1::SIZE as u32) }
    }

    fn bytes_input(&self) -> Bytes {
        let input_size = self.input_size();
        let mut buffer = vec![0u8; input_size as usize];
        self.read(&mut buffer, 0);
        buffer.into()
    }

    fn read_context(&self, target: &mut [u8], offset: u32) {
        self.native_sdk.read(target, offset)
    }

    fn charge_fuel(&self, fuel_consumed: u64) {
        self.native_sdk.charge_fuel(fuel_consumed);
    }

    fn fuel(&self) -> u64 {
        self.native_sdk.fuel()
    }

    fn write<T: AsRef<[u8]>>(&mut self, output: T) {
        self.native_sdk.write(output.as_ref());
    }

    fn native_exit(&self, exit_code: ExitCode) -> ! {
        self.native_sdk.exit(exit_code)
    }

    fn native_exec(
        &self,
        code_hash: B256,
        input: Cow<'_, [u8]>,
        fuel_limit: Option<u64>,
        state: u32,
    ) -> (u64, i64, i32) {
        self.native_sdk
            .exec(BytecodeOrHash::Hash(code_hash), input, fuel_limit, state)
    }

    fn return_data(&self) -> Bytes {
        let output_size = self.native_sdk.output_size();
        let mut result = vec![0u8; output_size as usize];
        self.native_sdk.read_output(&mut result, 0);
        result.into()
    }

    fn write_transient_storage(&mut self, slot: U256, value: U256) -> SyscallResult<()> {
        let mut buffer = [0u8; encode::transient_write_size_hint()];
        encode::transient_write_into(&mut &mut buffer[..], &slot, &value);
        let (fuel_consumed, fuel_refunded, exit_code) = self.native_sdk.exec(
            BytecodeOrHash::Hash(SYSCALL_ID_TRANSIENT_WRITE),
            Cow::Borrowed(&buffer),
            None,
            STATE_MAIN,
        );
        SyscallResult::new((), fuel_consumed, fuel_refunded, exit_code)
    }

    fn transient_storage(&self, slot: &U256) -> SyscallResult<U256> {
        let mut buffer = [0u8; encode::transient_read_size_hint()];
        encode::transient_read_into(&mut &mut buffer[..], slot);
        let (fuel_consumed, fuel_refunded, exit_code) = self.native_sdk.exec(
            BytecodeOrHash::Hash(SYSCALL_ID_TRANSIENT_READ),
            Cow::Borrowed(&buffer),
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

    fn emit_log<D: AsRef<[u8]>>(&mut self, topics: &[B256], data: D) -> SyscallResult<()> {
        let mut buffer = vec![0u8; encode::emit_log_size_hint(topics.len(), data.as_ref().len(),)];
        encode::emit_log_into(&mut &mut buffer[..], topics, data.as_ref());
        let (fuel_consumed, fuel_refunded, exit_code) = self.native_sdk.exec(
            BytecodeOrHash::Hash(SYSCALL_ID_EMIT_LOG),
            Cow::Owned(buffer),
            None,
            STATE_MAIN,
        );
        SyscallResult::new((), fuel_consumed, fuel_refunded, exit_code)
    }

    fn self_balance(&self) -> SyscallResult<U256> {
        let mut buffer = [0u8; encode::self_balance_size_hint()];
        encode::self_balance_into(&mut &mut buffer[..]);
        let (fuel_consumed, fuel_refunded, exit_code) = self.native_sdk.exec(
            BytecodeOrHash::Hash(SYSCALL_ID_SELF_BALANCE),
            Cow::Borrowed(&buffer),
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
        let mut buffer = [0u8; encode::balance_size_hint()];
        encode::balance_into(&mut &mut buffer[..], address);
        let (fuel_consumed, fuel_refunded, exit_code) = self.native_sdk.exec(
            BytecodeOrHash::Hash(SYSCALL_ID_BALANCE),
            Cow::Borrowed(&buffer),
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

    fn block_hash(&self, block_number: u64) -> SyscallResult<B256> {
        let mut buffer = [0u8; encode::block_hash_size_hint()];
        encode::block_hash_into(&mut &mut buffer[..], block_number);
        let (fuel_consumed, fuel_refunded, exit_code) = self.native_sdk.exec(
            BytecodeOrHash::Hash(SYSCALL_ID_BLOCK_HASH),
            Cow::Borrowed(&buffer),
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
        let mut buffer = [0u8; encode::code_size_size_hint()];
        encode::code_size_into(&mut &mut buffer[..], address);
        let (fuel_consumed, fuel_refunded, exit_code) = self.native_sdk.exec(
            BytecodeOrHash::Hash(SYSCALL_ID_CODE_SIZE),
            Cow::Borrowed(&buffer),
            None,
            STATE_MAIN,
        );
        let mut output: [u8; 4] = [0u8; 4];
        if SyscallResult::is_ok(exit_code) {
            self.native_sdk.read_output(&mut output, 0);
        }
        let value = u32::from_le_bytes(output);
        SyscallResult::new(value, fuel_consumed, fuel_refunded, exit_code)
    }

    fn code_hash(&self, address: &Address) -> SyscallResult<B256> {
        let mut buffer = [0u8; encode::code_hash_size_hint()];
        encode::code_hash_into(&mut &mut buffer[..], address);
        let (fuel_consumed, fuel_refunded, exit_code) = self.native_sdk.exec(
            BytecodeOrHash::Hash(SYSCALL_ID_CODE_HASH),
            Cow::Borrowed(&buffer),
            None,
            STATE_MAIN,
        );
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
        let mut buffer = [0u8; encode::code_copy_size_hint()];
        encode::code_copy_into(&mut &mut buffer[..], address, code_offset, code_length);
        let (fuel_consumed, fuel_refunded, exit_code) = self.native_sdk.exec(
            BytecodeOrHash::Hash(SYSCALL_ID_CODE_COPY),
            Cow::Borrowed(&buffer),
            None,
            STATE_MAIN,
        );
        let value = if SyscallResult::is_ok(exit_code) {
            self.return_data()
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
        let mut buffer =
            Vec::with_capacity(encode::create_size_hint(init_code.len(), salt.is_some()));
        encode::create_into(&mut &mut buffer[..], salt.as_ref(), value, init_code);
        let syscall_id = SYSCALL_ID_CREATE2;
        let (fuel_consumed, fuel_refunded, exit_code) = self.native_sdk.exec(
            BytecodeOrHash::Hash(syscall_id),
            Cow::Owned(buffer),
            None,
            STATE_MAIN,
        );
        let value = self.return_data();
        SyscallResult::new(value, fuel_consumed, fuel_refunded, exit_code)
    }

    fn call(
        &mut self,
        address: Address,
        value: U256,
        input: &[u8],
        fuel_limit: Option<u64>,
    ) -> SyscallResult<Bytes> {
        let mut buffer = vec![0u8; encode::call_size_hint(input.len(), true)];
        encode::call_into(&mut &mut buffer[..], address, Some(value), input);
        let (fuel_consumed, fuel_refunded, exit_code) = self.native_sdk.exec(
            BytecodeOrHash::Hash(SYSCALL_ID_CALL),
            Cow::Owned(buffer),
            fuel_limit,
            STATE_MAIN,
        );
        SyscallResult::new(self.return_data(), fuel_consumed, fuel_refunded, exit_code)
    }

    fn call_code(
        &mut self,
        address: Address,
        value: U256,
        input: &[u8],
        fuel_limit: Option<u64>,
    ) -> SyscallResult<Bytes> {
        let mut buffer = vec![0u8; encode::call_size_hint(input.len(), true)];
        encode::call_into(&mut &mut buffer[..], address, Some(value), input);
        let (fuel_consumed, fuel_refunded, exit_code) = self.native_sdk.exec(
            BytecodeOrHash::Hash(SYSCALL_ID_CALL_CODE),
            Cow::Owned(buffer),
            fuel_limit,
            STATE_MAIN,
        );
        SyscallResult::new(self.return_data(), fuel_consumed, fuel_refunded, exit_code)
    }

    fn delegate_call(
        &mut self,
        address: Address,
        input: &[u8],
        fuel_limit: Option<u64>,
    ) -> SyscallResult<Bytes> {
        let mut buffer = vec![0u8; encode::call_size_hint(input.len(), false)];
        encode::call_into(&mut &mut buffer[..], address, None, input);
        let (fuel_consumed, fuel_refunded, exit_code) = self.native_sdk.exec(
            BytecodeOrHash::Hash(SYSCALL_ID_DELEGATE_CALL),
            Cow::Owned(buffer),
            fuel_limit,
            STATE_MAIN,
        );
        SyscallResult::new(self.return_data(), fuel_consumed, fuel_refunded, exit_code)
    }

    fn static_call(
        &mut self,
        address: Address,
        input: &[u8],
        fuel_limit: Option<u64>,
    ) -> SyscallResult<Bytes> {
        let mut buffer = vec![0u8; encode::call_size_hint(input.len(), false)];
        encode::call_into(&mut &mut buffer[..], address, None, input);
        let (fuel_consumed, fuel_refunded, exit_code) = self.native_sdk.exec(
            BytecodeOrHash::Hash(SYSCALL_ID_STATIC_CALL),
            Cow::Owned(buffer),
            fuel_limit,
            STATE_MAIN,
        );
        SyscallResult::new(self.return_data(), fuel_consumed, fuel_refunded, exit_code)
    }

    fn destroy_account(&mut self, address: Address) -> SyscallResult<()> {
        let mut buffer = [0u8; encode::destroy_account_size_hint()];
        encode::destroy_account_into(&mut &mut buffer[..], &address);
        let (fuel_consumed, fuel_refunded, exit_code) = self.native_sdk.exec(
            BytecodeOrHash::Hash(SYSCALL_ID_DESTROY_ACCOUNT),
            Cow::Borrowed(&buffer),
            None,
            STATE_MAIN,
        );
        SyscallResult::new((), fuel_consumed, fuel_refunded, exit_code)
    }
}
