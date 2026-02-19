use crate::bytecode::{AnalyzedBytecode, LegacyBytecode};
use alloc::{vec, vec::Vec};
use fluentbase_sdk::{Bytes, B256};

pub enum EthereumMetadata {
    Legacy(LegacyBytecode),
    Analyzed(AnalyzedBytecode),
}

pub const ETHEREUM_METADATA_VERSION_ANALYZED: B256 = B256::with_last_byte(0x01);

impl EthereumMetadata {
    pub fn read_from_bytes(metadata: &Bytes) -> Option<Self> {
        if metadata.len() < 32 {
            return None;
        }
        Some(match B256::from_slice(&metadata[0..32]) {
            ETHEREUM_METADATA_VERSION_ANALYZED => Self::Analyzed(
                AnalyzedBytecode::deserialize(&metadata[32..])
                    .unwrap_or_else(|_| unreachable!("failed to deserialize analyzed bytecode")),
            ),
            hash => {
                let bytecode = metadata.slice(32..);
                Self::Legacy(LegacyBytecode { hash, bytecode })
            }
        })
    }

    pub fn write_to_bytes(&self) -> Bytes {
        match self {
            EthereumMetadata::Legacy(legacy_bytecode) => {
                let mut result = vec![];
                result.extend_from_slice(&legacy_bytecode.hash[..]);
                result.extend_from_slice(&legacy_bytecode.bytecode[..]);
                result.into()
            }
            EthereumMetadata::Analyzed(analyzed_bytecode) => {
                let hint_size = analyzed_bytecode.hint_size();
                let mut result = Vec::with_capacity(B256::len_bytes() + hint_size);
                unsafe {
                    result.set_len(B256::len_bytes() + hint_size);
                }
                result[0..B256::len_bytes()]
                    .copy_from_slice(&ETHEREUM_METADATA_VERSION_ANALYZED[..]);
                analyzed_bytecode
                    .serialize(&mut result[B256::len_bytes()..])
                    .unwrap_or_else(|_| unreachable!("evm: failed to serialize analyzed bytecode"));
                result.into()
            }
        }
    }

    pub fn code_size(&self) -> usize {
        match self {
            EthereumMetadata::Legacy(bytecode) => bytecode.bytecode.len(),
            EthereumMetadata::Analyzed(bytecode) => bytecode.len,
        }
    }

    pub fn code_hash(&self) -> B256 {
        match self {
            EthereumMetadata::Legacy(bytecode) => bytecode.hash,
            EthereumMetadata::Analyzed(bytecode) => bytecode.hash,
        }
    }

    pub fn code_copy(&self) -> Bytes {
        match self {
            EthereumMetadata::Legacy(bytecode) => bytecode.bytecode.clone(),
            EthereumMetadata::Analyzed(bytecode) => bytecode.bytecode.slice(0..bytecode.len),
        }
    }
}
