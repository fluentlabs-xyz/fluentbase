mod context;

use crate::{
    byteorder::{ByteOrder, LittleEndian},
    evm::{write_evm_exit_message, write_evm_panic_message},
    shared::context::SharedContextReaderImpl,
};
use alloc::vec;
use core::cell::RefCell;
use fluentbase_codec::{CompactABI, FluentEncoder};
use fluentbase_types::{
    alloc_slice,
    Address,
    Bytes,
    ExitCode,
    NativeAPI,
    SharedAPI,
    SharedContextInputV1,
    SharedContextReader,
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
    SYSCALL_ID_DELEGATE_CALL,
    SYSCALL_ID_DESTROY_ACCOUNT,
    SYSCALL_ID_EMIT_LOG,
    SYSCALL_ID_EXT_STORAGE_READ,
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
                input_size >= SharedContextInputV1::FLUENT_HEADER_SIZE,
                "malformed input header"
            );
            let mut header_input: [u8; SharedContextInputV1::FLUENT_HEADER_SIZE] =
                [0u8; SharedContextInputV1::FLUENT_HEADER_SIZE];
            self.native_sdk.read(&mut header_input, 0);
            let result = CompactABI::<SharedContextInputV1>::decode(&&header_input[..], 0)
                .unwrap_or_else(|_| unreachable!("fluentbase: malformed input header"));
            result
        });
        &self.shared_context_input_v1
    }

    pub fn commit_changes_and_exit(&mut self) -> ! {
        self.native_sdk.exit(0);
    }
}

/// SharedContextImpl always created from input
impl<API: NativeAPI> SharedAPI for SharedContextImpl<API> {
    fn context(&self) -> impl SharedContextReader {
        SharedContextReaderImpl(self.shared_context_ref())
    }

    fn keccak256(&self, data: &[u8]) -> B256 {
        API::keccak256(data)
    }

    fn read(&self, target: &mut [u8], offset: u32) {
        self.native_sdk.read(
            target,
            SharedContextInputV1::FLUENT_HEADER_SIZE as u32 + offset,
        )
    }

    fn input_size(&self) -> u32 {
        let input_size = self.native_sdk.input_size();
        assert!(
            input_size >= SharedContextInputV1::FLUENT_HEADER_SIZE as u32,
            "input less than context header"
        );
        input_size - SharedContextInputV1::FLUENT_HEADER_SIZE as u32
    }

    fn charge_fuel(&self, value: u64) {
        self.native_sdk.charge_fuel(value);
    }

    fn fuel(&self) -> u64 {
        self.native_sdk.fuel()
    }

    fn write(&mut self, output: &[u8]) {
        self.native_sdk.write(output);
    }

    fn evm_exit(&self, exit_code: i32) -> ! {
        // write an EVM-compatible exit message (only if exit code is not zero)
        if exit_code != 0 {
            write_evm_exit_message(&self.native_sdk, exit_code);
        }
        // exit with the exit code specified
        self.native_sdk.exit(if exit_code != 0 {
            ExitCode::Panic as i32
        } else {
            ExitCode::Ok as i32
        })
    }

    fn exit(&self, exit_code: i32) -> ! {
        debug_assert!(exit_code <= 0, "exit code must be non-positive");
        self.native_sdk.exit(exit_code)
    }

    fn evm_panic(&self, panic_message: &str) -> ! {
        // write an EVM-compatible panic message
        write_evm_panic_message(&self.native_sdk, panic_message);
        // exit with panic exit code (-71 is a WASMI constant, we use the same)
        self.native_sdk.exit(ExitCode::Panic as i32)
    }

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
                .exec(SYSCALL_ID_STORAGE_WRITE, &input, 0, STATE_MAIN);
        SyscallResult::new((), fuel_consumed, fuel_refunded, exit_code)
    }

    fn storage(&self, slot: &U256) -> SyscallResult<U256> {
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk
                .exec(SYSCALL_ID_STORAGE_READ, slot.as_le_slice(), 0, STATE_MAIN);
        let mut output = [0u8; U256::BYTES];
        if exit_code == 0 {
            self.native_sdk.read_output(&mut output, 0);
        };
        let value = U256::from_le_slice(&output);
        SyscallResult::new(value, fuel_consumed, fuel_refunded, exit_code)
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
                .exec(SYSCALL_ID_TRANSIENT_WRITE, &input, 0, STATE_MAIN);
        SyscallResult::new((), fuel_consumed, fuel_refunded, exit_code)
    }

    fn transient_storage(&self, slot: &U256) -> SyscallResult<U256> {
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk
                .exec(SYSCALL_ID_TRANSIENT_READ, slot.as_le_slice(), 0, STATE_MAIN);
        let mut output = [0u8; U256::BYTES];
        if exit_code == 0 {
            self.native_sdk.read_output(&mut output, 0);
        };
        let value = U256::from_le_slice(&output);
        SyscallResult::new(value, fuel_consumed, fuel_refunded, exit_code)
    }

    fn ext_storage(&self, address: &Address, slot: &U256) -> SyscallResult<U256> {
        let mut input: [u8; 20 + 32] = [0u8; 20 + 32];
        input[0..20].copy_from_slice(address.as_slice());
        input[20..52].copy_from_slice(slot.as_le_slice());
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk
                .exec(SYSCALL_ID_EXT_STORAGE_READ, &input, 0, STATE_MAIN);
        let mut output = [0u8; U256::BYTES];
        if exit_code == 0 {
            self.native_sdk.read_output(&mut output, 0);
        };
        let value = U256::from_le_slice(&output);
        SyscallResult::new(value, fuel_consumed, fuel_refunded, exit_code)
    }

    fn preimage_copy(&self, hash: &B256, target: &mut [u8]) -> SyscallResult<()> {
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk
                .exec(SYSCALL_ID_PREIMAGE_COPY, hash.as_ref(), 0, STATE_MAIN);
        if exit_code == 0 {
            let preimage = self.native_sdk.return_data();
            target.copy_from_slice(preimage.as_ref());
        }
        SyscallResult::new((), fuel_consumed, fuel_refunded, exit_code)
    }

    fn preimage_size(&self, hash: &B256) -> SyscallResult<u32> {
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk
                .exec(SYSCALL_ID_PREIMAGE_SIZE, hash.as_ref(), 0, STATE_MAIN);
        let mut output: [u8; 4] = [0u8; 4];
        if exit_code == 0 {
            self.native_sdk.read_output(&mut output, 0);
        }
        let value = LittleEndian::read_u32(&output);
        SyscallResult::new(value, fuel_consumed, fuel_refunded, exit_code)
    }

    fn emit_log(&mut self, data: Bytes, topics: &[B256]) -> SyscallResult<()> {
        let mut buffer = vec![0u8; 1 + topics.len() * B256::len_bytes()];
        assert!(topics.len() <= 4);
        buffer[0] = topics.len() as u8;
        for (i, topic) in topics.iter().enumerate() {
            buffer[(1 + i * B256::len_bytes())..(1 + i * B256::len_bytes() + B256::len_bytes())]
                .copy_from_slice(topic.as_slice());
        }
        buffer.extend_from_slice(data.as_ref());
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk
                .exec(SYSCALL_ID_EMIT_LOG, &buffer, 0, STATE_MAIN);
        SyscallResult::new((), fuel_consumed, fuel_refunded, exit_code)
    }

    fn self_balance(&self) -> SyscallResult<U256> {
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk
                .exec(SYSCALL_ID_SELF_BALANCE, Default::default(), 0, STATE_MAIN);
        let mut output = [0u8; U256::BYTES];
        if exit_code == 0 {
            self.native_sdk.read_output(&mut output, 0);
        };
        let value = U256::from_le_slice(&output);
        SyscallResult::new(value, fuel_consumed, fuel_refunded, exit_code)
    }

    fn balance(&self, address: &Address) -> SyscallResult<U256> {
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk
                .exec(SYSCALL_ID_BALANCE, address.as_slice(), 0, STATE_MAIN);
        let mut output = [0u8; U256::BYTES];
        if exit_code == 0 {
            self.native_sdk.read_output(&mut output, 0);
        };
        let value = U256::from_le_slice(&output);
        SyscallResult::new(value, fuel_consumed, fuel_refunded, exit_code)
    }

    fn code_size(&self, address: &Address) -> SyscallResult<u32> {
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk
                .exec(SYSCALL_ID_CODE_SIZE, address.as_slice(), 0, STATE_MAIN);
        let mut output: [u8; 4] = [0u8; 4];
        if exit_code == 0 {
            self.native_sdk.read_output(&mut output, 0);
        }
        let value = u32::from_le_bytes(output);
        SyscallResult::new(value, fuel_consumed, fuel_refunded, exit_code)
    }

    fn code_hash(&self, address: &Address) -> SyscallResult<B256> {
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk
                .exec(SYSCALL_ID_CODE_HASH, address.as_slice(), 0, STATE_MAIN);
        let mut output = [0u8; B256::len_bytes()];
        if exit_code == 0 {
            self.native_sdk.read_output(&mut output, 0);
        }
        let value = B256::from(output);
        SyscallResult::new(value, fuel_consumed, fuel_refunded, exit_code)
    }

    fn code_copy(&self, address: &Address, offset: u32, target: &mut [u8]) -> SyscallResult<()> {
        let mut input = [0u8; 20 + 4];
        input[0..20].copy_from_slice(address.as_slice());
        LittleEndian::write_u32(&mut input[20..24], offset);
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk
                .exec(SYSCALL_ID_CODE_COPY, &input, 0, STATE_MAIN);
        if exit_code == 0 {
            self.native_sdk.read_output(target, 0);
        }
        SyscallResult::new((), fuel_consumed, fuel_refunded, exit_code)
    }

    fn write_preimage(&mut self, preimage: Bytes) -> SyscallResult<B256> {
        let (fuel_consumed, fuel_refunded, exit_code) =
            self.native_sdk
                .exec(SYSCALL_ID_WRITE_PREIMAGE, preimage.as_ref(), 0, STATE_MAIN);
        let mut output = [0u8; B256::len_bytes()];
        if exit_code == 0 {
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
            self.native_sdk.exec(code_hash, &buffer, 0, STATE_MAIN);
        let value = self.native_sdk.return_data();
        SyscallResult::new(value, fuel_consumed, fuel_refunded, exit_code)
    }

    fn call(
        &mut self,
        address: Address,
        value: U256,
        input: &[u8],
        fuel_limit: u64,
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
        fuel_limit: u64,
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
        fuel_limit: u64,
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
        fuel_limit: u64,
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
            0,
            STATE_MAIN,
        );
        SyscallResult::new((), fuel_consumed, fuel_refunded, exit_code)
    }
}
