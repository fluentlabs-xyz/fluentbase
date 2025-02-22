use crate::{B256, FUEL_DENOM_RATE};

#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(non_camel_case_types)]
pub enum BytecodeType {
    EVM,
    WASM,
}

pub const EIP7702_SIG_LEN: usize = 2;
/// rWASM binary format signature:
/// - 0xef 0x00 - EIP-3540 compatible prefix
/// - 0x52 - rWASM version number (equal to 'R')
pub const EIP7702_SIG: [u8; EIP7702_SIG_LEN] = [0xef, 0x01];

pub const WASM_SIG_LEN: usize = 4;
/// WebAssembly signature (\00ASM)
pub const WASM_SIG: [u8; WASM_SIG_LEN] = [0x00, 0x61, 0x73, 0x6d];

pub const RWASM_SIG_LEN: usize = 2;
/// rWASM binary format signature:
/// - 0xef 0x00 - EIP-3540 compatible prefix
/// - 0x52 - rWASM version number (equal to 'R')
pub const RWASM_SIG: [u8; RWASM_SIG_LEN] = [0xef, 0x52];

impl BytecodeType {
    pub fn from_slice(input: &[u8]) -> Self {
        // default WebAssembly signature
        if input.len() >= WASM_SIG_LEN && input[0..WASM_SIG_LEN] == WASM_SIG {
            return Self::WASM;
        }
        // case for rWASM contracts that are inside genesis
        if input.len() >= RWASM_SIG_LEN && input[0..RWASM_SIG_LEN] == RWASM_SIG {
            return Self::WASM;
        }
        // all the rest are EVM bytecode
        Self::EVM
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
pub const SYSCALL_ID_WRITE_PREIMAGE: B256 = B256::with_last_byte(0x0c);
pub const SYSCALL_ID_PREIMAGE_COPY: B256 = B256::with_last_byte(0x0d);
pub const SYSCALL_ID_PREIMAGE_SIZE: B256 = B256::with_last_byte(0x0e);
pub const SYSCALL_ID_EXT_STORAGE_READ: B256 = B256::with_last_byte(0x0f);
pub const SYSCALL_ID_TRANSIENT_READ: B256 = B256::with_last_byte(0x10);
pub const SYSCALL_ID_TRANSIENT_WRITE: B256 = B256::with_last_byte(0x11);

pub const FUEL_LIMIT_SYSCALL_STORAGE_READ: u64 = 2_100 * FUEL_DENOM_RATE;
pub const FUEL_LIMIT_SYSCALL_STORAGE_WRITE: u64 = 22_100 * FUEL_DENOM_RATE;
pub const FUEL_LIMIT_SYSCALL_EMIT_LOG: u64 = 10_000 * FUEL_DENOM_RATE;
pub const FUEL_LIMIT_SYSCALL_DESTROY_ACCOUNT: u64 = 32_600 * FUEL_DENOM_RATE;
pub const FUEL_LIMIT_SYSCALL_BALANCE: u64 = 2_600 * FUEL_DENOM_RATE;
pub const FUEL_LIMIT_SYSCALL_EXT_STORAGE_READ: u64 = 2_100 * FUEL_DENOM_RATE;
pub const FUEL_LIMIT_SYSCALL_PREIMAGE_SIZE: u64 = 2_600 * FUEL_DENOM_RATE;
pub const FUEL_LIMIT_SYSCALL_TRANSIENT_READ: u64 = 100 * FUEL_DENOM_RATE;
pub const FUEL_LIMIT_SYSCALL_TRANSIENT_WRITE: u64 = 100 * FUEL_DENOM_RATE;
