use crate::{Bytes, ExitCode, B256};
use fluentbase_codec::Codec;

#[derive(Codec, Clone, Default, Debug, PartialEq, Eq, Hash)]
pub struct SyscallInvocationParams {
    pub code_hash: B256,
    pub input: Bytes,
    pub fuel_limit: u64,
    pub state: u32,
    pub fuel16_ptr: u32,
}

#[derive(Debug)]
pub struct SyscallResult<T> {
    pub data: T,
    pub fuel_consumed: u64,
    pub fuel_refunded: i64,
    pub status: ExitCode,
}

impl SyscallResult<()> {
    pub fn is_ok<I: Into<ExitCode>>(status: I) -> bool {
        Into::<ExitCode>::into(status) == ExitCode::Ok
    }
    pub fn is_panic<I: Into<ExitCode>>(status: I) -> bool {
        Into::<ExitCode>::into(status) == ExitCode::Panic
    }
    pub fn is_err<I: Into<ExitCode>>(status: I) -> bool {
        Into::<ExitCode>::into(status) == ExitCode::Err
    }
}

impl<T> SyscallResult<T> {
    pub fn new<I: Into<ExitCode>>(
        data: T,
        fuel_consumed: u64,
        fuel_refunded: i64,
        status: I,
    ) -> Self {
        Self {
            data,
            fuel_consumed,
            fuel_refunded,
            status: Into::<ExitCode>::into(status),
        }
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

pub const SYSCALL_ID_STORAGE_READ: B256 = B256::with_last_byte(0x01);
pub const SYSCALL_ID_STORAGE_WRITE: B256 = B256::with_last_byte(0x02);
pub const SYSCALL_ID_CALL: B256 = B256::with_last_byte(0x03);
pub const SYSCALL_ID_STATIC_CALL: B256 = B256::with_last_byte(0x04);
pub const SYSCALL_ID_CALL_CODE: B256 = B256::with_last_byte(0x05);
pub const SYSCALL_ID_DELEGATE_CALL: B256 = B256::with_last_byte(0x06);
pub const SYSCALL_ID_CREATE: B256 = B256::with_last_byte(0x07);
pub const SYSCALL_ID_CREATE2: B256 = B256::with_last_byte(0x08);
pub const SYSCALL_ID_EMIT_LOG: B256 = B256::with_last_byte(0x09);
pub const SYSCALL_ID_DESTROY_ACCOUNT: B256 = B256::with_last_byte(0x0a);
pub const SYSCALL_ID_BALANCE: B256 = B256::with_last_byte(0x0b);
pub const SYSCALL_ID_SELF_BALANCE: B256 = B256::with_last_byte(0x0c);
pub const SYSCALL_ID_CODE_SIZE: B256 = B256::with_last_byte(0x0d);
pub const SYSCALL_ID_CODE_HASH: B256 = B256::with_last_byte(0x0e);
pub const SYSCALL_ID_CODE_COPY: B256 = B256::with_last_byte(0x0f);

pub const SYSCALL_ID_TRANSIENT_READ: B256 = B256::with_last_byte(0x11);
pub const SYSCALL_ID_TRANSIENT_WRITE: B256 = B256::with_last_byte(0x12);

pub const SYSCALL_ID_WRITE_PREIMAGE: B256 = B256::with_last_byte(0x30);
pub const SYSCALL_ID_PREIMAGE_COPY: B256 = B256::with_last_byte(0x31);
pub const SYSCALL_ID_PREIMAGE_SIZE: B256 = B256::with_last_byte(0x32);
pub const SYSCALL_ID_DELEGATED_STORAGE: B256 = B256::with_last_byte(0x33);

// TODO(dmitry): "this syscall will be removed"
pub const SYSCALL_ID_SYNC_EVM_GAS: B256 = B256::with_last_byte(0xf0);
