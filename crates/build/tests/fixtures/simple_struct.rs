// tests/fixtures/simple_struct.rs
// Test case: Simple struct as parameter and return value

#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{
    basic_entrypoint,
    derive::{router, Codec},
    Address, SharedAPI, B256, U256,
};

// Simple struct with basic types
#[derive(Codec, Debug, Clone)]
pub struct TransferParams {
    pub recipient: Address,
    pub amount: U256,
    pub deadline: U256,
    pub nonce: u64,
}

// Return struct
#[derive(Codec, Debug, Clone)]
pub struct TransferReceipt {
    pub success: bool,
    pub tx_hash: B256,
    pub gas_used: U256,
    pub block_number: U256,
}

#[derive(Default)]
pub struct SimpleContract<SDK> {
    sdk: SDK,
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> SimpleContract<SDK> {
    /// Transfer with struct parameter
    pub fn transfer(&mut self, params: TransferParams) -> bool {
        // Simple implementation
        println!("Transfer {} to {}", params.amount, params.recipient);
        true
    }

    /// Get transfer receipt - returns struct
    pub fn get_receipt(&self, tx_id: U256) -> TransferReceipt {
        TransferReceipt {
            success: true,
            tx_hash: B256::from([0u8; 32]),
            gas_used: U256::from(21000),
            block_number: U256::from(12345),
        }
    }

    /// Mixed: struct param and struct return
    pub fn execute_transfer(&mut self, params: TransferParams) -> TransferReceipt {
        TransferReceipt {
            success: true,
            tx_hash: B256::from([1u8; 32]),
            gas_used: U256::from(25000),
            block_number: U256::from(12346),
        }
    }

    /// Control: method without structs
    pub fn simple_transfer(&mut self, to: Address, amount: U256) -> bool {
        println!("Simple transfer {} to {}", amount, to);
        true
    }
}

impl<SDK: SharedAPI> SimpleContract<SDK> {
    pub fn new(sdk: SDK) -> Self {
        Self { sdk }
    }
}

basic_entrypoint!(SimpleContract);
