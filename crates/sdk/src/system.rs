mod state;

use crate::{
    debug_log, system::state::RecoverableState, Address, Bytes, ContextReader, SharedAPI,
    StorageAPI, SystemAPI, B256, U256,
};
use alloc::{borrow::Cow, vec, vec::Vec};
pub use fluentbase_types::system::*;
use fluentbase_types::{CryptoAPI, ExitCode, NativeAPI, SyscallInvocationParams, SyscallResult};

pub struct SystemContextImpl<API> {
    native_sdk: API,
    state: RecoverableState,
    interruption: Option<Bytes>,
    metadata: Option<Bytes>,
}

impl<API: NativeAPI + CryptoAPI> SystemContextImpl<API> {
    #[inline(never)]
    pub fn new(native_sdk: API) -> Self {
        let output_size = native_sdk.output_size() as usize;
        if output_size > 0 {
            // Output size greater than 0 indicates an interruption outcome
            let output_size = native_sdk.output_size();
            let mut return_data = Vec::with_capacity(output_size as usize);
            unsafe {
                return_data.set_len(output_size as usize);
            }
            native_sdk.read_output(&mut return_data, 0);
            let (outcome, _) = bincode::decode_from_slice::<RuntimeInterruptionOutcomeV1, _>(
                return_data.as_ref(),
                bincode::config::legacy(),
            )
            .unwrap();
            let state = RecoverableState::recover(outcome);
            return Self {
                native_sdk,
                state,
                interruption: None,
                metadata: None,
            };
        }

        let input_size = native_sdk.input_size() as usize;
        if input_size > 0 {
            // Input size greater than 0 indicates a new frame
            let input_size = native_sdk.input_size();
            let mut input = vec![0u8; input_size as usize];
            native_sdk.read(&mut input, 0);
            let (input, _): (RuntimeNewFrameInputV1, usize) =
                bincode::decode_from_slice(&input, bincode::config::legacy()).unwrap();
            let state = RecoverableState::new(input);
            return Self {
                native_sdk,
                state,
                interruption: None,
                metadata: None,
            };
        }

        // This should never happen
        native_sdk.exit(ExitCode::UnreachableCodeReached);
    }

    #[cfg(target_arch = "wasm32")]
    pub fn panic_handler(info: &core::panic::PanicInfo) -> ! {
        use crate::{ExitCode, NativeAPI, RwasmContext};
        crate::debug_log!("panic: {}", info.message());
        let native_sdk = RwasmContext {};
        // We can't forward any errors here into output because we already have corrupted
        // memory state (because of unwinding), so the best we can do is just to exit
        native_sdk.exit(ExitCode::Panic)
    }

    pub fn finalize(self, result: Result<(), ExitCode>) {
        let exit_code = result.err().unwrap_or(ExitCode::Ok);
        let SystemContextImpl {
            native_sdk,
            state,
            interruption,
            metadata,
        } = self;
        if exit_code == ExitCode::InterruptionCalled {
            let interruption = interruption.unwrap();
            state.remember();
            let exit_code_be = exit_code.into_i32().to_le_bytes();
            native_sdk.write(&exit_code_be);
            native_sdk.write(&interruption);
        } else {
            let (storage, logs) = state.storage.into_diff();
            let output = RuntimeExecutionOutcomeV1 {
                exit_code,
                output: state.output.into(),
                storage: Some(storage),
                logs,
                new_metadata: metadata,
            }
            .encode();
            let exit_code_be = exit_code.into_i32().to_le_bytes();
            native_sdk.write(&exit_code_be);
            native_sdk.write(&output);
        };
    }
}

impl<API: NativeAPI + CryptoAPI> SharedAPI for SystemContextImpl<API> {
    fn context(&self) -> impl ContextReader {
        &self.state.context
    }

    fn read(&self, target: &mut [u8], offset: u32) {
        // Safety: This code can panic, and it's intended behavior because it should never happen,
        //  inside the system runtime
        target.copy_from_slice(&self.state.input[offset as usize..offset as usize + target.len()]);
    }

    fn input_size(&self) -> u32 {
        self.state.input.len() as u32
    }

    fn bytes_input(&self) -> Bytes {
        self.state.input.clone()
    }

    fn read_context(&self, _target: &mut [u8], _offset: u32) {
        unimplemented!("read_context")
    }

    fn charge_fuel(&self, fuel_consumed: u64) {
        self.native_sdk.charge_fuel(fuel_consumed);
    }

    fn fuel(&self) -> u64 {
        self.native_sdk.fuel()
    }

    fn write<T: AsRef<[u8]>>(&mut self, output: T) {
        self.state.output.extend_from_slice(output.as_ref());
    }

    fn native_exit(&self, _exit_code: ExitCode) -> ! {
        unimplemented!("native_exit")
    }

    fn native_exec(
        &self,
        _code_hash: B256,
        _input: Cow<'_, [u8]>,
        _fuel_limit: Option<u64>,
        _state: u32,
    ) -> (u64, i64, i32) {
        unimplemented!("native_exec")
    }

    fn return_data(&self) -> Bytes {
        unimplemented!("return_data")
    }

    fn write_transient_storage(&mut self, _slot: U256, _value: U256) -> SyscallResult<()> {
        unimplemented!()
    }

    fn transient_storage(&self, _slot: &U256) -> SyscallResult<U256> {
        unimplemented!("transient_storage")
    }

    fn emit_log<D: AsRef<[u8]>>(&mut self, topics: &[B256], data: D) -> SyscallResult<()> {
        self.state
            .storage
            .emit_log(topics.to_vec(), Bytes::copy_from_slice(data.as_ref()));
        SyscallResult::default()
    }

    fn self_balance(&self) -> SyscallResult<U256> {
        unimplemented!("self_balance")
    }

    fn balance(&self, _address: &Address) -> SyscallResult<U256> {
        unimplemented!("balance")
    }

    fn block_hash(&self, _block_number: u64) -> SyscallResult<B256> {
        unimplemented!("block_hash")
    }

    fn code_size(&self, _address: &Address) -> SyscallResult<u32> {
        unimplemented!("code_size")
    }

    fn code_hash(&self, _address: &Address) -> SyscallResult<B256> {
        unimplemented!("code_hash")
    }

    fn code_copy(
        &self,
        _address: &Address,
        _code_offset: u64,
        _code_length: u64,
    ) -> SyscallResult<Bytes> {
        unimplemented!("code_copy")
    }

    fn create(
        &mut self,
        _salt: Option<U256>,
        _value: &U256,
        _init_code: &[u8],
    ) -> SyscallResult<Bytes> {
        unimplemented!("create")
    }

    fn call(
        &mut self,
        _address: Address,
        _value: U256,
        _input: &[u8],
        _fuel_limit: Option<u64>,
    ) -> SyscallResult<Bytes> {
        unimplemented!("call")
    }

    fn call_code(
        &mut self,
        _address: Address,
        _value: U256,
        _input: &[u8],
        _fuel_limit: Option<u64>,
    ) -> SyscallResult<Bytes> {
        unimplemented!("call_code")
    }

    fn delegate_call(
        &mut self,
        _address: Address,
        _input: &[u8],
        _fuel_limit: Option<u64>,
    ) -> SyscallResult<Bytes> {
        unimplemented!()
    }

    fn static_call(
        &mut self,
        _address: Address,
        _input: &[u8],
        _fuel_limit: Option<u64>,
    ) -> SyscallResult<Bytes> {
        unimplemented!("static_call")
    }

    fn destroy_account(&mut self, _address: Address) -> SyscallResult<()> {
        unimplemented!("destroy_account")
    }
}

impl<API: NativeAPI + CryptoAPI> StorageAPI for SystemContextImpl<API> {
    fn write_storage(&mut self, slot: U256, value: U256) -> SyscallResult<()> {
        self.state.storage.write_storage(slot, value);
        // Note: Storage write here can't fail, that's why we always return `Ok`
        SyscallResult::default()
    }

    fn storage(&self, slot: &U256) -> SyscallResult<U256> {
        if let Some(value) = self.state.storage.storage(slot) {
            return SyscallResult::new(*value, 0, 0, ExitCode::Ok);
        };
        debug_log!("a missing storage slot detected at: {}", slot);
        // We return here a `MissingStorageSlot`, but this error should be at user-level, because
        // an attacker might intentionally pass incorrect params into the function
        SyscallResult::new(U256::ZERO, 0, 0, ExitCode::MissingStorageSlot)
    }
}

impl<API: NativeAPI + CryptoAPI> SystemAPI for SystemContextImpl<API> {
    fn take_interruption_outcome(&mut self) -> Option<RuntimeInterruptionOutcomeV1> {
        self.state.interruption_outcome.take()
    }

    fn insert_interruption_income(
        &mut self,
        code_hash: B256,
        input: Cow<'_, [u8]>,
        fuel_limit: Option<u64>,
        state: u32,
    ) {
        let input = match input {
            Cow::Borrowed(input) => Bytes::copy_from_slice(input),
            Cow::Owned(input) => Bytes::from(input),
        };
        let input_offset = input.as_ptr() as usize;
        let syscall_params = SyscallInvocationParams {
            code_hash,
            input: input_offset..(input_offset + input.len()),
            fuel_limit: fuel_limit.unwrap_or(u64::MAX),
            state,
            fuel16_ptr: 0,
        };
        _ = self.interruption.insert(syscall_params.encode().into());
        // We must save input, otherwise it will be destructed (we store it inside
        //  the recoverable state, the one we don't free)
        _ = self.state.intermediary_input.insert(input);
    }

    fn unique_key(&self) -> u32 {
        self.state.unique_key
    }

    fn write_contract_metadata(&mut self, metadata: Bytes) {
        _ = self.metadata.insert(metadata);
    }

    fn contract_metadata(&self) -> Bytes {
        self.state.metadata.clone()
    }
}
