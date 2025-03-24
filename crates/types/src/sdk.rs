use crate::{
    alloc_slice,
    alloc_vec,
    context::SharedContextReader,
    Address,
    BytecodeOrHash,
    Bytes,
    ExitCode,
    B256,
    F254,
    U256,
};
use fluentbase_codec::Codec;

/// A trait for providing shared API functionality.
pub trait NativeAPI {
    fn keccak256(data: &[u8]) -> B256;
    fn sha256(data: &[u8]) -> B256;

    /// Computes a quick hash of the given data using the Keccak256 algorithm or another specified
    /// hashing method.
    ///
    /// The hashing result produced by this function is not standardized and can vary depending on
    /// the proving system used.
    ///
    /// # Parameters
    /// - `data`: A byte slice representing the input data to be hashed.
    ///
    /// # Returns
    /// - `B256`: A 256-bit hash of the input data.
    fn hash256(data: &[u8]) -> B256 {
        // TODO(dmitry123): "use the best hashing function here for our proving system"
        Self::keccak256(data)
    }

    fn poseidon(data: &[u8]) -> F254;
    fn poseidon_hash(fa: &F254, fb: &F254, fd: &F254) -> F254;
    fn ec_recover(digest: &B256, sig: &[u8; 64], rec_id: u8) -> [u8; 65];
    fn debug_log(message: &str);

    fn read(&self, target: &mut [u8], offset: u32);
    fn input_size(&self) -> u32;
    fn write(&self, value: &[u8]);
    fn forward_output(&self, offset: u32, len: u32);
    fn exit(&self, exit_code: i32) -> !;
    fn output_size(&self) -> u32;
    fn read_output(&self, target: &mut [u8], offset: u32);
    fn state(&self) -> u32;
    fn fuel(&self) -> u64;
    fn charge_fuel(&self, value: u64) -> u64;
    fn exec<I: Into<BytecodeOrHash>>(
        &self,
        code_hash: I,
        input: &[u8],
        fuel_limit: Option<u64>,
        state: u32,
    ) -> (u64, i64, i32);
    fn resume(
        &self,
        call_id: u32,
        return_data: &[u8],
        exit_code: i32,
        fuel_consumed: u64,
        fuel_refunded: i64,
    ) -> (u64, i64, i32);

    fn preimage_size(&self, hash: &B256) -> u32;
    fn preimage_copy(&self, hash: &B256, target: &mut [u8]);

    fn input(&self) -> Bytes {
        let input_size = self.input_size();
        let mut buffer = alloc_vec(input_size as usize);
        self.read(&mut buffer, 0);
        buffer.into()
    }

    fn return_data(&self) -> Bytes {
        let output_size = self.output_size();
        let mut buffer = alloc_vec(output_size as usize);
        self.read_output(&mut buffer, 0);
        buffer.into()
    }
}

pub type IsColdAccess = bool;

pub struct CallPrecompileResult {
    pub output: Bytes,
    pub exit_code: ExitCode,
    pub gas_remaining: u64,
    pub gas_refund: i64,
}

pub struct WriteStorageResult {
    pub original_value: U256,
    pub present_value: U256,
}

pub struct DestroyedAccountResult {
    pub had_value: bool,
    pub target_exists: bool,
    pub is_cold: bool,
    pub previously_destroyed: bool,
}

#[derive(Codec, Clone, Default, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SyscallInvocationParams {
    pub code_hash: B256,
    pub input: Bytes,
    pub fuel_limit: u64,
    pub state: u32,
    pub fuel16_ptr: u32,
}

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(i32)]
pub enum SyscallStatus {
    #[default]
    Ok = 0,
    Revert = -1,
    Err = -2,
    OutOfGas = -3,
}

impl From<i32> for SyscallStatus {
    fn from(value: i32) -> Self {
        match value {
            // TODO(dmitry123): "use enum for EVM error codes?"
            d if d >= 0 && d < 0x10 => Self::Ok,
            d if d >= 0x10 && d < 0x20 => Self::Revert,
            0x20 => unreachable!("sdk: action returned from revert syscall"),
            0x50 => Self::OutOfGas,
            _ => Self::Err,
        }
    }
}

#[derive(Debug)]
pub struct SyscallResult<T> {
    pub data: T,
    pub fuel_consumed: u64,
    pub fuel_refunded: i64,
    pub status: i32,
}

impl SyscallResult<()> {
    pub fn is_ok(status: i32) -> bool {
        // TODO(dmitry123): "use enum for EVM error codes?"
        status >= 0 && status < 0x10
    }
}

impl<T> SyscallResult<T> {
    pub fn new(data: T, fuel_consumed: u64, fuel_refunded: i64, status: i32) -> Self {
        Self {
            data,
            fuel_consumed,
            fuel_refunded,
            status,
        }
    }
    pub fn status(&self) -> SyscallStatus {
        SyscallStatus::from(self.status)
    }
}

impl<T> core::ops::Deref for SyscallResult<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}
impl<T> core::ops::DerefMut for SyscallResult<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.data
    }
}

pub trait SharedAPI {
    fn context(&self) -> impl SharedContextReader;

    fn keccak256(&self, data: &[u8]) -> B256;

    fn read(&self, target: &mut [u8], offset: u32);
    fn input_size(&self) -> u32;

    fn input<'a>(&self) -> &'a [u8] {
        let input_size = self.input_size();
        let mut buffer = alloc_slice(input_size as usize);
        self.read(&mut buffer, 0);
        buffer
    }

    fn charge_fuel(&self, value: u64);
    fn fuel(&self) -> u64;

    fn write(&mut self, output: &[u8]);
    fn evm_exit(&self, exit_code: i32) -> !;
    fn exit(&self, exit_code: i32) -> !;
    fn evm_panic(&self, panic_message: &str) -> !;

    fn write_storage(&mut self, slot: U256, value: U256) -> SyscallResult<()>;
    fn storage(&self, slot: &U256) -> SyscallResult<U256>;
    fn write_transient_storage(&mut self, slot: U256, value: U256) -> SyscallResult<()>;
    fn transient_storage(&self, slot: &U256) -> SyscallResult<U256>;
    fn ext_storage(&self, slot: &U256) -> SyscallResult<U256>;
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
