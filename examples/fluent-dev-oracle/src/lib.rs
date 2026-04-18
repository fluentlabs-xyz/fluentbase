//! # Fluent Dev Oracle
//!
//! This crate implements a developer identity registry on the Fluent Network.
//! It maps repository hashes to developer wallet addresses using rWasm execution
//! with strict namespace separation for security.

#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{entrypoint, SharedAPI, ContextReader};
use alloy_primitives::{U256, keccak256};

/// Unique Namespace for this Oracle to prevent Storage Collisions.
const STORAGE_NAMESPACE_PREFIX: &[u8] = b"fluent.oracle.dev_identity.v1";

/// The main entry point for the Fluent Dev Oracle smart contract.
pub fn main_entry(mut sdk: impl SharedAPI) {
    let input = sdk.bytes_input();
    
    // Make sure the input is sufficient to convert it to U256 (32 bytes)
    if input.len() >= 32 {
        let caller_address = sdk.context().contract_caller();
        
        // --- SECURITY: NAMESPACE SEPARATION ---
        let mut extended_input = [0u8; 61]; 
        extended_input[0..29].copy_from_slice(STORAGE_NAMESPACE_PREFIX);
        extended_input[29..61].copy_from_slice(&input[0..32]);
        
        let secure_slot_hash = keccak256(extended_input);
        let storage_key = U256::from_be_bytes(secure_slot_hash.0);
        
        // Convert the developer address to the storage value
        let mut val_bytes = [0u8; 32];
        val_bytes[12..32].copy_from_slice(caller_address.as_slice());
        let storage_value = U256::from_be_bytes(val_bytes);
        
        // Commit the key-value pair to blockchain storage
        sdk.write_storage(storage_key, storage_value);
        
        // Emit a permanent trace on the blockchain logs
        sdk.write(b"Secure Dev Registration via FreeDropOracle");
    }
}

entrypoint!(main_entry);

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, U256, keccak256};
    use fluentbase_testing::TestingContextImpl;

    #[test]
    fn test_dev_registration() {
        // Simulate a repo hash (32 bytes)
        let repo_hash = [42u8; 32];
        // Simulate a caller address
        let caller = Address::from([1u8; 20]);

        let sdk = TestingContextImpl::default()
            .with_caller(caller)
            .with_input(repo_hash);

        main_entry(sdk.clone());

        // Re-calculate the expected storage key with namespace
        let mut extended_input = [0u8; 61];
        extended_input[0..29].copy_from_slice(STORAGE_NAMESPACE_PREFIX);
        extended_input[29..61].copy_from_slice(&repo_hash);
        let secure_slot_hash = keccak256(extended_input);
        let storage_key = U256::from_be_bytes(secure_slot_hash.0);

        // Compute the expected storage value
        let mut val_bytes = [0u8; 32];
        val_bytes[12..32].copy_from_slice(caller.as_slice());
        let expected_value = U256::from_be_bytes(val_bytes);

        // Check storage
        let contract_addr = sdk.context().contract_address();
        let storage = sdk.dump_storage();
        assert_eq!(storage.get(&(contract_addr, storage_key)), Some(&expected_value));

        // Check output
        let output = sdk.take_output();
        assert_eq!(output, b"Secure Dev Registration via FreeDropOracle");
    }
}