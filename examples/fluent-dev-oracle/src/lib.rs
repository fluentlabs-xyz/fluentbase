//! # Fluent Dev Oracle
//!
//! This crate implements a developer identity registry on the Fluent Network.
//! It maps repository hashes to developer wallet addresses using rWasm execution.

#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{entrypoint, SharedAPI, ContextReader};
use alloy_primitives::U256;

/// The main entry point for the Fluent Dev Oracle smart contract.
///
/// If called with at least 32 bytes of input, this contract uses the first
/// 32 bytes as the repository hash storage key, stores the caller address as
/// the value, and emits a registration log.
///
/// # Behavior
///
/// - Reads input bytes from the execution context.
/// - Uses the first 32 bytes of input as a repository hash key.
/// - Retrieves the caller's address from the runtime context.
/// - Stores a `repo_hash => developer_address` mapping on-chain.
/// - Emits a log message confirming registration.
pub fn main_entry(mut sdk: impl SharedAPI) {
    let input = sdk.bytes_input();
    
    // تأكد من أن المدخلات كافية لتحويلها لـ U256 (32 بايت)
    if input.len() >= 32 {
        let caller_address = sdk.context().contract_caller();
        
        // Convert the first 32 bytes of input to the storage key
        let mut key_bytes = [0u8; 32];
        key_bytes.copy_from_slice(&input[0..32]);
        let storage_key = U256::from_be_bytes(key_bytes);
        
        // Convert the developer address to the storage value
        let mut val_bytes = [0u8; 32];
        val_bytes[12..32].copy_from_slice(caller_address.as_slice());
        let storage_value = U256::from_be_bytes(val_bytes);
        
        // Commit the key-value pair to blockchain storage
        sdk.write_storage(storage_key, storage_value);
        
        // Emit a permanent trace on the blockchain logs
        sdk.write(b"Dev Registered via FreeDropOracle");
    }
}

entrypoint!(main_entry);