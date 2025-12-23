use crate::{
    alloc_slice, debug_log, Address, Bytes, ContextReader, IsAccountEmpty, IsAccountOwnable,
    IsColdAccess, MetadataAPI, MetadataStorageAPI, SharedAPI, SharedContextInputV1, StorageAPI,
    B256, U256,
};
use alloc::{vec, vec::Vec};
use core::cell::OnceCell;
pub use fluentbase_types::system::*;
use fluentbase_types::{CryptoAPI, ExitCode, NativeAPI, SyscallResult};
use hashbrown::HashMap;

pub struct SystemContextImpl<API> {
    native_sdk: API,
    storage: JournalStorage,
    metadata: Bytes,
    input: Bytes,
    context: Bytes,
    context_ref: OnceCell<SharedContextInputV1>,
    balances: HashMap<Address, U256>,
    output: Vec<u8>,
    outcome: Option<RuntimeInterruptionOutcomeV1>,
}

impl<API: NativeAPI + CryptoAPI> SystemContextImpl<API> {
    pub fn new(native_sdk: API) -> Self {
        let input_size = native_sdk.input_size() as usize;
        let output_size = native_sdk.output_size() as usize;
        if input_size > 0 {
            let input = alloc_slice(input_size);
            native_sdk.read(input, 0);
            let (input, _): (RuntimeNewFrameInputV1, usize) =
                bincode::decode_from_slice(input, bincode::config::legacy()).unwrap();
            let RuntimeNewFrameInputV1 {
                metadata,
                input,
                context,
                storage,
                balances,
            } = input;
            Self {
                native_sdk,
                storage: JournalStorage::new(storage.unwrap_or_default()),
                metadata,
                input,
                context,
                context_ref: OnceCell::new(),
                balances: balances.unwrap_or_default(),
                output: vec![],
                outcome: None,
            }
        } else if output_size > 0 {
            // let output = alloc_slice(output_size);
            // native_sdk.read_output(output, 0);
            // let (outcome, _): (RuntimeInterruptionOutcomeV1, usize) =
            //     bincode::decode_from_slice(output, bincode::config::legacy()).unwrap();
            unimplemented!("not implemented yet")
        } else {
            unreachable!()
        }
    }

    pub fn finalize(self, result: Result<(), ExitCode>) {
        let exit_code = match result {
            Ok(_) => ExitCode::Ok,
            Err(exit_code) => exit_code,
        };
        let (storage, logs) = self.storage.into_diff();
        let outcome = RuntimeExecutionOutcomeV1 {
            exit_code,
            output: self.output.into(),
            storage: Some(storage),
            logs,
        };
        let result = bincode::encode_to_vec(outcome, bincode::config::legacy()).unwrap();
        let exit_code_be = exit_code.into_i32().to_le_bytes();
        self.native_sdk.write(&exit_code_be);
        self.native_sdk.write(&result);
    }
}

impl<API: NativeAPI + CryptoAPI> StorageAPI for SystemContextImpl<API> {
    fn write_storage(&mut self, slot: U256, value: U256) -> SyscallResult<()> {
        self.storage.write_storage(slot, value);
        // NOTE: Storage write here can't fail, that's why we always return `Ok`
        SyscallResult::default()
    }

    fn storage(&self, slot: &U256) -> SyscallResult<U256> {
        if let Some(value) = self.storage.storage(slot) {
            return SyscallResult::new(*value, 0, 0, ExitCode::Ok);
        };
        debug_log!("a missing storage slot detected at: {}", slot);
        // We return here a `MissingStorageSlot`, but this error should be at user-level, because
        // an attacker might intentionally pass incorrect params into the function
        SyscallResult::new(U256::ZERO, 0, 0, ExitCode::MissingStorageSlot)
    }
}

impl<API: NativeAPI + CryptoAPI> MetadataAPI for SystemContextImpl<API> {
    fn metadata_write(
        &mut self,
        _address: &Address,
        _offset: u32,
        _metadata: Bytes,
    ) -> SyscallResult<()> {
        unimplemented!()
    }

    fn metadata_size(
        &self,
        _address: &Address,
    ) -> SyscallResult<(u32, IsAccountOwnable, IsColdAccess, IsAccountEmpty)> {
        unimplemented!()
    }

    fn metadata_create(&mut self, _salt: &U256, _metadata: Bytes) -> SyscallResult<()> {
        unimplemented!()
    }

    fn metadata_copy(
        &self,
        _address: &Address,
        _offset: u32,
        _length: u32,
    ) -> SyscallResult<Bytes> {
        unimplemented!()
    }

    fn metadata_account_owner(&self, _address: &Address) -> SyscallResult<Address> {
        unimplemented!()
    }
}

impl<API: NativeAPI + CryptoAPI> MetadataStorageAPI for SystemContextImpl<API> {
    fn metadata_storage_read(&self, _slot: &U256) -> SyscallResult<U256> {
        unimplemented!()
    }

    fn metadata_storage_write(&mut self, _slot: &U256, _value: U256) -> SyscallResult<()> {
        unimplemented!()
    }
}

impl<API: NativeAPI + CryptoAPI> SharedAPI for SystemContextImpl<API> {
    fn context(&self) -> impl ContextReader {
        self.context_ref
            .get_or_init(|| SharedContextInputV1::decode_from_slice(self.context.as_ref()).unwrap())
    }

    fn read(&self, target: &mut [u8], offset: u32) {
        // SAFETY: This code can panic, and it's intended behavior because it should never happen,
        //  inside the system runtime
        target.copy_from_slice(&self.input[offset as usize..offset as usize + target.len()]);
    }

    fn input_size(&self) -> u32 {
        self.input.len() as u32
    }

    fn read_context(&self, _target: &mut [u8], _offset: u32) {
        unimplemented!()
    }

    fn charge_fuel(&self, _fuel_consumed: u64) {
        unimplemented!()
    }

    fn fuel(&self) -> u64 {
        unimplemented!()
    }

    fn write(&mut self, output: &[u8]) {
        self.output.extend_from_slice(output);
    }

    fn native_exit(&self, _exit_code: ExitCode) -> ! {
        unimplemented!()
    }

    fn native_exec(
        &self,
        _code_hash: B256,
        _input: &[u8],
        _fuel_limit: Option<u64>,
        _state: u32,
    ) -> (u64, i64, i32) {
        unimplemented!()
    }

    fn return_data(&self) -> Bytes {
        unimplemented!()
    }

    fn write_transient_storage(&mut self, _slot: U256, _value: U256) -> SyscallResult<()> {
        unimplemented!()
    }

    fn transient_storage(&self, _slot: &U256) -> SyscallResult<U256> {
        unimplemented!()
    }

    fn emit_log<D: AsRef<[u8]>>(&mut self, topics: &[B256], data: D) -> SyscallResult<()> {
        self.storage
            .emit_log(topics.to_vec(), Bytes::copy_from_slice(data.as_ref()));
        SyscallResult::default()
    }

    fn self_balance(&self) -> SyscallResult<U256> {
        unimplemented!()
    }

    fn balance(&self, _address: &Address) -> SyscallResult<U256> {
        unimplemented!()
    }

    fn block_hash(&self, _block_number: u64) -> SyscallResult<B256> {
        unimplemented!()
    }

    fn code_size(&self, _address: &Address) -> SyscallResult<u32> {
        unimplemented!()
    }

    fn code_hash(&self, _address: &Address) -> SyscallResult<B256> {
        unimplemented!()
    }

    fn code_copy(
        &self,
        _address: &Address,
        _code_offset: u64,
        _code_length: u64,
    ) -> SyscallResult<Bytes> {
        unimplemented!()
    }

    fn create(
        &mut self,
        _salt: Option<U256>,
        _value: &U256,
        _init_code: &[u8],
    ) -> SyscallResult<Bytes> {
        unimplemented!()
    }

    fn call(
        &mut self,
        _address: Address,
        _value: U256,
        _input: &[u8],
        _fuel_limit: Option<u64>,
    ) -> SyscallResult<Bytes> {
        unimplemented!()
    }

    fn call_code(
        &mut self,
        _address: Address,
        _value: U256,
        _input: &[u8],
        _fuel_limit: Option<u64>,
    ) -> SyscallResult<Bytes> {
        unimplemented!()
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
        unimplemented!()
    }

    fn destroy_account(&mut self, _address: Address) -> SyscallResult<()> {
        unimplemented!()
    }
}
