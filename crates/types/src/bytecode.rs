#[allow(non_camel_case_types)]
pub enum BytecodeType {
    EVM,
    WASM,
}

const WASM_SIG: [u8; 4] = [0x00, 0x61, 0x73, 0x6d];
const RWASM_SIG: [u8; 2] = [0xef, 0x00];

impl BytecodeType {
    pub fn from_slice(input: &[u8]) -> Self {
        // default WebAssembly signature (\0ASM)
        if input.len() >= 4 && input[0..4] == WASM_SIG {
            return Self::WASM;
        }
        // case for rWASM contracts that are inside genesis
        if input.len() >= 2 && input[0..2] == RWASM_SIG {
            return Self::WASM;
        }
        // all the rest are EVM bytecode
        Self::EVM
    }
}
