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

#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(non_camel_case_types)]
pub enum BytecodeType {
    EVM,
    WASM,
}

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
