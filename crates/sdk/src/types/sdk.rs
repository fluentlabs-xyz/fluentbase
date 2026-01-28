use crate::{
    evm::{write_evm_exit_message, write_evm_panic_message},
    system::RuntimeInterruptionOutcomeV1,
    Address, Bytes, ContextReader, ExitCode, SyscallResult, B256, FUEL_DENOM_RATE, U256,
};
use fluentbase_crypto::crypto_keccak256;

pub type IsAccountOwnable = bool;
pub type IsColdAccess = bool;
pub type IsAccountEmpty = bool;

pub trait StorageAPI {
    fn write_storage(&mut self, slot: U256, value: U256) -> SyscallResult<()>;
    fn storage(&self, slot: &U256) -> SyscallResult<U256>;
}

pub trait MetadataAPI {
    fn metadata_write(
        &mut self,
        address: &Address,
        offset: u32,
        metadata: Bytes,
    ) -> SyscallResult<()>;
    fn metadata_size(
        &self,
        address: &Address,
    ) -> SyscallResult<(u32, IsAccountOwnable, IsColdAccess, IsAccountEmpty)>;
    fn metadata_create(&mut self, salt: &U256, metadata: Bytes) -> SyscallResult<()>;
    fn metadata_copy(&self, address: &Address, offset: u32, length: u32) -> SyscallResult<Bytes>;
    fn metadata_account_owner(&self, address: &Address) -> SyscallResult<Address>;
}

pub trait MetadataStorageAPI {
    fn metadata_storage_read(&self, slot: &U256) -> SyscallResult<U256>;
    fn metadata_storage_write(&mut self, slot: &U256, value: U256) -> SyscallResult<()>;
}

pub trait SharedAPI: StorageAPI + MetadataAPI + MetadataStorageAPI {
    /// We keep it here only for backward compatibility, but we suggest using crypto library instead.
    /// This function can be removed in the future.
    fn keccak256(data: &[u8]) -> B256 {
        crypto_keccak256(data)
    }

    fn context(&self) -> impl ContextReader;

    fn read(&self, target: &mut [u8], offset: u32);
    fn input_size(&self) -> u32;

    fn input<'a>(&self) -> &'a [u8] {
        let input_size = self.input_size();
        let pointer = unsafe {
            alloc::alloc::alloc(core::alloc::Layout::from_size_align_unchecked(
                input_size as usize,
                8,
            ))
        };
        let mut buffer =
            unsafe { &mut *core::ptr::slice_from_raw_parts_mut(pointer, input_size as usize) };
        self.read(&mut buffer, 0);
        buffer
    }

    fn bytes_input(&self) -> Bytes {
        Bytes::from(self.input())
    }

    fn read_context(&self, target: &mut [u8], offset: u32);

    fn charge_fuel(&self, fuel_consumed: u64);

    fn sync_evm_gas(&self, gas_consumed: u64) -> Result<(), ExitCode> {
        let fuel_consumed = gas_consumed
            .checked_mul(FUEL_DENOM_RATE)
            .unwrap_or(u64::MAX);
        let fuel_remaining = self.fuel();
        if fuel_consumed > fuel_remaining {
            return Err(ExitCode::OutOfFuel);
        }
        self.charge_fuel(fuel_consumed);
        Ok(())
    }

    fn fuel(&self) -> u64;

    fn write<T: AsRef<[u8]>>(&mut self, output: T);

    fn evm_exit(&mut self, exit_code: u32) -> ! {
        // write an EVM-compatible exit message (only if exit code is not zero)
        if exit_code != 0 {
            write_evm_exit_message(exit_code, |slice| {
                self.write(slice);
            });
            self.native_exit(ExitCode::Panic);
        } else {
            self.native_exit(ExitCode::Ok)
        }
    }

    fn native_exit(&self, exit_code: ExitCode) -> !;

    fn native_exec(
        &self,
        code_hash: B256,
        input: &[u8],
        fuel_limit: Option<u64>,
        state: u32,
    ) -> (u64, i64, i32);

    fn return_data(&self) -> Bytes;

    fn exit(&self) -> ! {
        self.native_exit(ExitCode::Ok)
    }

    fn panic(&self) -> ! {
        self.native_exit(ExitCode::Panic)
    }

    fn evm_panic(&mut self, panic_message: &str) -> ! {
        // write an EVM-compatible panic message
        write_evm_panic_message(panic_message, |slice| {
            self.write(slice);
        });
        // exit with panic exit code
        self.native_exit(ExitCode::Panic)
    }
    fn write_transient_storage(&mut self, slot: U256, value: U256) -> SyscallResult<()>;
    fn transient_storage(&self, slot: &U256) -> SyscallResult<U256>;

    fn emit_log<D: AsRef<[u8]>>(&mut self, topics: &[B256], data: D) -> SyscallResult<()>;

    fn self_balance(&self) -> SyscallResult<U256>;
    fn balance(&self, address: &Address) -> SyscallResult<U256>;

    fn block_hash(&self, block_number: u64) -> SyscallResult<B256>;
    fn code_size(&self, address: &Address) -> SyscallResult<u32>;
    fn code_hash(&self, address: &Address) -> SyscallResult<B256>;
    fn code_copy(
        &self,
        address: &Address,
        code_offset: u64,
        code_length: u64,
    ) -> SyscallResult<Bytes>;
    fn create(
        &mut self,
        salt: Option<U256>,
        value: &U256,
        init_code: &[u8],
    ) -> SyscallResult<Bytes>;
    fn call(
        &mut self,
        address: Address,
        value: U256,
        input: &[u8],
        fuel_limit: Option<u64>,
    ) -> SyscallResult<Bytes>;
    fn call_code(
        &mut self,
        address: Address,
        value: U256,
        input: &[u8],
        fuel_limit: Option<u64>,
    ) -> SyscallResult<Bytes>;
    fn delegate_call(
        &mut self,
        address: Address,
        input: &[u8],
        fuel_limit: Option<u64>,
    ) -> SyscallResult<Bytes>;
    fn static_call(
        &mut self,
        address: Address,
        input: &[u8],
        fuel_limit: Option<u64>,
    ) -> SyscallResult<Bytes>;
    fn destroy_account(&mut self, address: Address) -> SyscallResult<()>;
}

pub trait SystemAPI: SharedAPI {
    fn take_interruption_outcome(&mut self) -> Option<RuntimeInterruptionOutcomeV1>;

    fn insert_interruption_income(
        &mut self,
        code_hash: B256,
        input: Bytes,
        fuel_limit: Option<u64>,
        state: u32,
    );

    fn unique_key(&self) -> u32;

    fn write_contract_metadata(&mut self, metadata: Bytes);

    fn contract_metadata(&self) -> Bytes;
}
