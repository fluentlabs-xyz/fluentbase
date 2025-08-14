// Test case: Arrays of structs (Vec<Struct>)

#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{
    basic_entrypoint,
    derive::{router, Codec},
    Address,
    SharedAPI,
    U256,
};

// Basic struct that will be used in arrays
#[derive(Codec, Debug, Clone)]
pub struct Pool {
    pub token0: Address,
    pub token1: Address,
    pub reserve0: U256,
    pub reserve1: U256,
    pub fee: U256,
}

// Struct containing array of structs
#[derive(Codec, Debug, Clone)]
pub struct Route {
    pub pools: Vec<Pool>,     // array of structs
    pub tokens: Vec<Address>, // array of primitives for comparison
    pub input_amount: U256,
}

// Struct with multiple arrays
#[derive(Codec, Debug, Clone)]
pub struct LiquidityPosition {
    pub pools: Vec<Pool>,
    pub balances: Vec<U256>,
    pub owner: Address,
}

// Nested arrays test
#[derive(Codec, Debug, Clone)]
pub struct PoolUpdate {
    pub pool: Pool,
    pub timestamp: U256,
}

#[derive(Codec, Debug, Clone)]
pub struct BatchUpdate {
    pub updates: Vec<PoolUpdate>, // array of structs containing structs
    pub batch_id: U256,
}

#[derive(Default)]
pub struct ArrayContract<SDK> {
    sdk: SDK,
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> ArrayContract<SDK> {
    /// Test: array of structs as parameter
    pub fn add_pools(&mut self, pools: Vec<Pool>) -> bool {
        !pools.is_empty()
    }

    /// Test: struct containing array of structs
    pub fn execute_route(&mut self, route: Route) -> U256 {
        route.input_amount
    }

    /// Test: return array of structs
    pub fn get_all_pools(&self) -> Vec<Pool> {
        vec![
            Pool {
                token0: Address::from([1u8; 20]),
                token1: Address::from([2u8; 20]),
                reserve0: U256::from(1000000),
                reserve1: U256::from(2000000),
                fee: U256::from(300),
            },
            Pool {
                token0: Address::from([3u8; 20]),
                token1: Address::from([4u8; 20]),
                reserve0: U256::from(3000000),
                reserve1: U256::from(4000000),
                fee: U256::from(500),
            },
        ]
    }

    /// Test: multiple array parameters
    pub fn update_reserves(&mut self, pools: Vec<Pool>, new_reserves: Vec<U256>) -> bool {
        pools.len() == new_reserves.len()
    }

    /// Test: nested arrays (array of structs that contain structs)
    pub fn apply_batch_update(&mut self, batch: BatchUpdate) -> U256 {
        batch.batch_id
    }

    /// Test: return struct containing arrays
    pub fn get_position(&self, owner: Address) -> LiquidityPosition {
        LiquidityPosition {
            pools: vec![Pool {
                token0: Address::from([5u8; 20]),
                token1: Address::from([6u8; 20]),
                reserve0: U256::from(5000000),
                reserve1: U256::from(6000000),
                fee: U256::from(100),
            }],
            balances: vec![U256::from(1000), U256::from(2000)],
            owner,
        }
    }
}

impl<SDK: SharedAPI> ArrayContract<SDK> {
    pub fn new(sdk: SDK) -> Self {
        Self { sdk }
    }
}

basic_entrypoint!(ArrayContract);
