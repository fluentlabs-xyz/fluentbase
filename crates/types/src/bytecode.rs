#[allow(non_camel_case_types)]
pub enum BytecodeType {
    EVM,
    WASM,
}

/// WebAssembly signature (\0ASM)
const WASM_SIG: [u8; 4] = [0x00, 0x61, 0x73, 0x6d];

/// rWASM binary format signature:
/// - 0xef 0x00 - EIP-3540 compatible prefix
/// - 0x52 - rWASM version number (equal to 'R')
const RWASM_SIG: [u8; 3] = [0xef, 0x00, 0x52];

impl BytecodeType {
    pub fn from_slice(input: &[u8]) -> Self {
        // default WebAssembly signature (\0ASM)
        if input.len() >= WASM_SIG.len() && input[0..4] == WASM_SIG {
            return Self::WASM;
        }
        // case for rWASM contracts that are inside genesis
        if input.len() >= RWASM_SIG.len() && input[0..3] == RWASM_SIG {
            return Self::WASM;
        }
        // all the rest are EVM bytecode
        Self::EVM
    }
}
