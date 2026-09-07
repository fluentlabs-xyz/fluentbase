use crate::bytecode::{AnalyzedBytecode, LegacyBytecode};
use alloc::vec;
use fluentbase_sdk::{crypto::crypto_keccak256, Bytes, B256};

pub enum EthereumMetadata {
    Legacy(LegacyBytecode),
    Analyzed(AnalyzedBytecode),
}

pub const ETHEREUM_METADATA_VERSION_ANALYZED: B256 = B256::with_last_byte(0x01);

impl EthereumMetadata {
    pub fn new_analyzed(bytecode: Bytes) -> Self {
        let code_hash = crypto_keccak256(bytecode.as_ref());
        Self::Analyzed(AnalyzedBytecode::new(bytecode, code_hash))
    }

    pub fn new_legacy(bytecode: Bytes) -> Self {
        let hash = crypto_keccak256(bytecode.as_ref());
        Self::Legacy(LegacyBytecode { hash, bytecode })
    }

    pub fn read_from_bytes(metadata: &Bytes) -> Option<Self> {
        if metadata.len() < 32 {
            return None;
        }
        Some(match B256::from_slice(&metadata[0..32]) {
            ETHEREUM_METADATA_VERSION_ANALYZED => {
                Self::Analyzed(AnalyzedBytecode::deserialize(&metadata[32..]).ok()?)
            }
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
                let mut result = vec![0u8; B256::len_bytes() + hint_size];
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
            EthereumMetadata::Analyzed(bytecode) => bytecode.len(),
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
            EthereumMetadata::Analyzed(bytecode) => bytecode.bytecode.slice(0..bytecode.len()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_truncated_analyzed_metadata() {
        let metadata = EthereumMetadata::new_analyzed(Bytes::from_static(&[0x60, 0x00, 0x5b]))
            .write_to_bytes();
        for len in 0..metadata.len() {
            assert!(
                EthereumMetadata::read_from_bytes(&metadata.slice(..len)).is_none(),
                "accepted metadata truncated to {len} bytes"
            );
        }
    }

    #[test]
    fn rejects_analyzed_metadata_with_invalid_code_length() {
        let mut metadata = EthereumMetadata::new_analyzed(Bytes::from_static(&[0x00]))
            .write_to_bytes()
            .to_vec();
        // The original code length follows the version and code hash.
        metadata[64..72].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(EthereumMetadata::read_from_bytes(&metadata.into()).is_none());
    }

    #[test]
    fn valid_metadata_preserves_code_queries() {
        for code in [Bytes::new(), Bytes::from_static(&[0x60, 0x00, 0x5b])] {
            for metadata in [
                EthereumMetadata::new_legacy(code.clone()),
                EthereumMetadata::new_analyzed(code.clone()),
            ] {
                let decoded =
                    EthereumMetadata::read_from_bytes(&metadata.write_to_bytes()).unwrap();
                assert_eq!(decoded.code_size(), code.len());
                assert_eq!(decoded.code_copy(), code);
                assert_eq!(decoded.code_hash(), crypto_keccak256(&code));
            }
        }
    }
}
