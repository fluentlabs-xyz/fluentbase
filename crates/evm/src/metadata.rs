use crate::bytecode::{AnalyzedBytecode, LegacyBytecode};
use alloc::vec;
use alloc::vec::Vec;
use fluentbase_sdk::{Bytes, B256};
use revm_helpers::reusable_pool::global::{vec_u8_try_reuse_and_copy_from, VecU8};

pub enum EthereumMetadata {
    Legacy(LegacyBytecode),
    Analyzed(AnalyzedBytecode),
}

pub const ETHEREUM_METADATA_VERSION_ANALYZED: B256 = B256::with_last_byte(0x01);

impl EthereumMetadata {
    pub fn read_from_bytes(metadata: &[u8]) -> Option<Self> {
        if metadata.len() < 32 {
            return None;
        }
        Some(match B256::from_slice(&metadata[0..32]) {
            ETHEREUM_METADATA_VERSION_ANALYZED => Self::Analyzed(
                AnalyzedBytecode::deserialize(&metadata[32..])
                    .unwrap_or_else(|_| unreachable!("failed to deserialize analyzed bytecode")),
            ),
            hash => {
                let bytecode = VecU8::try_from_slice(&metadata[32..]).expect("enough cap");
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
                let mut result = vec![];
                result.extend_from_slice(&ETHEREUM_METADATA_VERSION_ANALYZED[..]);
                let raw_analyzed_bytecode = analyzed_bytecode
                    .serialize()
                    .unwrap_or_else(|_| unreachable!("failed to serialize analyzed bytecode"));
                result.extend_from_slice(&raw_analyzed_bytecode);
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

    pub fn code_copy(&self) -> VecU8 {
        match self {
            EthereumMetadata::Legacy(bytecode) => {
                VecU8::try_from_slice(&bytecode.bytecode).expect("enough cap")
            }
            EthereumMetadata::Analyzed(bytecode) => {
                VecU8::try_from_slice(&bytecode.bytecode[0..bytecode.len]).expect("enough cap")
            }
        }
    }
}
