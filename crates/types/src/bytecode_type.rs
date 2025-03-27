use crate::{F254, KECCAK_EMPTY};
use alloy_primitives::{keccak256, Bytes};

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

#[derive(Clone, Debug)]
pub enum BytecodeOrHash {
    Bytecode(Bytes, Option<F254>),
    Hash(F254),
}

impl From<Bytes> for BytecodeOrHash {
    #[inline(always)]
    fn from(value: Bytes) -> Self {
        Self::Bytecode(value, None)
    }
}
impl From<F254> for BytecodeOrHash {
    #[inline(always)]
    fn from(value: F254) -> Self {
        Self::Hash(value)
    }
}
impl From<(Bytes, F254)> for BytecodeOrHash {
    #[inline(always)]
    fn from(value: (Bytes, F254)) -> Self {
        Self::Bytecode(value.0, Some(value.1))
    }
}

impl Default for BytecodeOrHash {
    #[inline(always)]
    fn default() -> Self {
        Self::Bytecode(Bytes::new(), Some(KECCAK_EMPTY))
    }
}

impl BytecodeOrHash {
    pub fn with_resolved_hash(self) -> Self {
        match self {
            BytecodeOrHash::Bytecode(_, Some(_)) => self,
            BytecodeOrHash::Bytecode(bytecode, None) => {
                let hash = keccak256(bytecode.as_ref());
                BytecodeOrHash::Bytecode(bytecode, Some(hash))
            }
            BytecodeOrHash::Hash(_) => self,
        }
    }

    pub fn resolve_hash(&self) -> F254 {
        match self {
            BytecodeOrHash::Bytecode(_, hash) => hash.expect("hash must be resolved"),
            BytecodeOrHash::Hash(hash) => *hash,
        }
    }
}
