use crate::{Bytes, B256};

pub fn evm_metadata_code_size(evm_metadata: &Bytes) -> usize {
    // TODO(dmitry123): We probably want to support metadata versions, make sure it covers cases with analyzed EVM bytecodes
    evm_metadata.len().checked_sub(32).unwrap_or(0)
}

pub fn evm_metadata_code_hash(evm_metadata: &Bytes) -> B256 {
    if evm_metadata.len() >= 32 {
        B256::from_slice(&evm_metadata.as_ref()[0..32])
    } else {
        // If metadata less than 32 bytes then such account doesn't exist
        B256::ZERO
    }
}

pub fn evm_metadata_code_copy(evm_metadata: &Bytes) -> Bytes {
    if evm_metadata.len() >= 32 {
        evm_metadata.slice(32..)
    } else {
        Bytes::default()
    }
}
