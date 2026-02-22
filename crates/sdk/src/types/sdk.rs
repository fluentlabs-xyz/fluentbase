use crate::{
    alloc_ptr_unaligned,
    evm::{write_evm_exit_message, write_evm_panic_message},
    system::RuntimeInterruptionOutcomeV1,
    Address, Bytes, ContextReader, ExitCode, SyscallResult, B256, FUEL_DENOM_RATE, U256,
};
use alloc::{borrow::Cow, vec::Vec};
use fluentbase_crypto::crypto_keccak256;

pub type IsAccountOwnable = bool;
pub type IsColdAccess = bool;
pub type IsAccountEmpty = bool;

pub trait StorageAPI {
    fn write_storage(&mut self, slot: U256, value: U256) -> SyscallResult<()>;
    fn storage(&self, slot: &U256) -> SyscallResult<U256>;
}

pub trait SharedAPI: StorageAPI {
    /// We keep it here only for backward compatibility, but we suggest using a crypto library instead.
    /// This function can be removed in the future.
    #[deprecated(note = "Use crypto_keccak256() instead", since = "0.5.2")]
    fn keccak256(data: &[u8]) -> B256 {
        crypto_keccak256(data)
    }

    fn context(&self) -> impl ContextReader;

    fn read(&self, target: &mut [u8], offset: u32);
    fn input_size(&self) -> u32;

    #[deprecated(note = "Use bytes_input() instead", since = "0.5.2")]
    fn input(&self) -> &[u8] {
        let input_size = self.input_size();
        let buffer = unsafe {
            &mut *core::ptr::slice_from_raw_parts_mut(
                alloc_ptr_unaligned(input_size as usize),
                input_size as usize,
            )
        };
        self.read(buffer, 0);
        buffer
    }

    fn bytes_input(&self) -> Bytes {
        let input_size = self.input_size() as usize;
        let mut buffer = Vec::with_capacity(input_size);
        unsafe {
            buffer.set_len(input_size);
        }
        self.read(buffer.as_mut_slice(), 0);
        buffer.into()
    }

    fn read_context(&self, target: &mut [u8], offset: u32);

    fn charge_fuel(&self, fuel_consumed: u64);

    fn fuel(&self) -> u64;

    fn write<T: AsRef<[u8]>>(&mut self, output: T);

    fn evm_exit(&mut self, exit_code: u32) -> ! {
        // write an EVM-compatible exit message (only if the exit code is not zero)
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
        input: Cow<'_, [u8]>,
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
        input: Cow<'_, [u8]>,
        fuel_limit: Option<u64>,
        state: u32,
    );

    fn unique_key(&self) -> u32;

    fn write_contract_metadata(&mut self, metadata: Bytes);

    fn contract_metadata(&self) -> Bytes;

    fn sync_evm_gas(&self, gas_consumed: u64) -> Result<(), ExitCode> {
        let fuel_consumed = gas_consumed
            .checked_mul(FUEL_DENOM_RATE)
            .unwrap_or(u64::MAX);
        let fuel_remaining = self.fuel();
        // Important: We assume fuel remaining is always set (no matter fuel enabled or disabled)
        if fuel_consumed > fuel_remaining {
            return Err(ExitCode::OutOfFuel);
        }
        self.charge_fuel(fuel_consumed);
        Ok(())
    }
}
