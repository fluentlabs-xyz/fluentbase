use crate::{Address, Bytes, B256, RWASM_SIG, RWASM_SIG_LEN, WASM_SIG, WASM_SIG_LEN};
use core::fmt::Formatter;
use rwasm_core::RwasmModule;

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
    Bytecode {
        bytecode: RwasmModule,
        hash: B256,
        address: Address,
    },
    Hash(B256),
}

impl core::fmt::Display for BytecodeOrHash {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            BytecodeOrHash::Bytecode {
                address,
                hash: code_hash,
                ..
            } => {
                write!(f, "bytecode {}::{}", address, code_hash)
            }
            BytecodeOrHash::Hash(code_hash) => write!(f, "{}", code_hash),
        }
    }
}

impl From<B256> for BytecodeOrHash {
    #[inline(always)]
    fn from(value: B256) -> Self {
        Self::Hash(value)
    }
}

impl Default for BytecodeOrHash {
    #[inline(always)]
    fn default() -> Self {
        Self::Hash(B256::ZERO)
    }
}

impl BytecodeOrHash {
    pub fn code_hash(&self) -> B256 {
        match self {
            BytecodeOrHash::Bytecode {
                hash: code_hash, ..
            } => *code_hash,
            BytecodeOrHash::Hash(hash) => *hash,
        }
    }
}

#[derive(Clone, Debug)]
pub enum BytesOrRef<'a> {
    Bytes(Bytes),
    Ref(&'a [u8]),
}

impl<'a> BytesOrRef<'a> {
    pub fn into_bytes(self) -> Bytes {
        match self {
            BytesOrRef::Bytes(bytes) => bytes,
            BytesOrRef::Ref(slice) => Bytes::copy_from_slice(slice),
        }
    }
}
