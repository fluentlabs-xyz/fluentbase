pub mod encode;

use crate::B256;

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
pub const SYSCALL_ID_BLOCK_HASH: B256 = B256::with_last_byte(0x13);

pub const SYSCALL_ID_METADATA_WRITE: B256 = B256::with_last_byte(0x40);
pub const SYSCALL_ID_METADATA_SIZE: B256 = B256::with_last_byte(0x41);
pub const SYSCALL_ID_METADATA_CREATE: B256 = B256::with_last_byte(0x42);
pub const SYSCALL_ID_METADATA_COPY: B256 = B256::with_last_byte(0x43);

pub const SYSCALL_ID_METADATA_STORAGE_READ: B256 = B256::with_last_byte(0x44);
pub const SYSCALL_ID_METADATA_STORAGE_WRITE: B256 = B256::with_last_byte(0x45);
pub const SYSCALL_ID_METADATA_ACCOUNT_OWNER: B256 = B256::with_last_byte(0x46);
