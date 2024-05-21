use alloy_primitives::{address, Address};

/// A special account for storing EVM storage trie `keccak256("evm_storage_trie")[12..32]`
pub const EVM_STORAGE_ADDRESS: Address = address!("fabefeab43f96e51d7ace194b9abd33305bb6bfb");
