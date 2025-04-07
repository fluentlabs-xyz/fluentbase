use crate::{
    context::SharedContextReader,
    evm::{write_evm_exit_message, write_evm_panic_message},
    Address,
    Bytes,
    ExitCode,
    SyscallResult,
    B256,
    U256,
};

pub type IsColdAccess = bool;
pub type IsAccountEmpty = bool;

pub trait StorageAPI {
    fn write_storage(&mut self, slot: U256, value: U256) -> SyscallResult<()>;
    fn storage(&self, slot: &U256) -> SyscallResult<U256>;
}

pub trait SharedAPI: StorageAPI {
    fn context(&self) -> impl SharedContextReader;

    fn keccak256(&self, data: &[u8]) -> B256;

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

    fn charge_fuel(&self, value: u64);
    fn fuel(&self) -> u64;

    fn write(&mut self, output: &[u8]);

    fn evm_exit(&mut self, exit_code: u32) -> ! {
        // write an EVM-compatible exit message (only if exit code is not zero)
        write_evm_exit_message(exit_code, |slice| {
            self.write(slice);
        });
        // exit with the exit code specified
        self.exit(if exit_code != 0 {
            ExitCode::Panic
        } else {
            ExitCode::Ok
        })
    }

    fn exit(&self, exit_code: ExitCode) -> !;

    fn evm_panic(&mut self, panic_message: &str) -> ! {
        // write an EVM-compatible panic message
        write_evm_panic_message(panic_message, |slice| {
            self.write(slice);
        });
        // exit with panic exit code
        self.exit(ExitCode::Panic)
    }

    fn write_transient_storage(&mut self, slot: U256, value: U256) -> SyscallResult<()>;
    fn transient_storage(&self, slot: &U256) -> SyscallResult<U256>;
    fn delegated_storage(
        &self,
        address: &Address,
        slot: &U256,
    ) -> SyscallResult<(U256, IsColdAccess, IsAccountEmpty)>;

    fn sync_evm_gas(&self, gas_remaining: u64, gas_refunded: i64) -> SyscallResult<()>;

    fn preimage_copy(&self, hash: &B256) -> SyscallResult<Bytes>;
    fn preimage_size(&self, hash: &B256) -> SyscallResult<u32>;

    fn preimage(&self, hash: &B256) -> Bytes {
        let result = self.preimage_copy(hash);
        assert!(
            SyscallResult::is_ok(result.status),
            "sdk: failed reading preimage"
        );
        result.data
    }

    fn emit_log(&mut self, data: Bytes, topics: &[B256]) -> SyscallResult<()>;

    fn self_balance(&self) -> SyscallResult<U256>;
    fn balance(&self, address: &Address) -> SyscallResult<U256>;
    fn code_size(&self, address: &Address) -> SyscallResult<u32>;
    fn code_hash(&self, address: &Address) -> SyscallResult<B256>;
    fn code_copy(
        &self,
        address: &Address,
        code_offset: u64,
        code_length: u64,
    ) -> SyscallResult<Bytes>;
    fn write_preimage(&mut self, preimage: Bytes) -> SyscallResult<B256>;
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
