// Test case: Nested structs (struct containing other structs)

#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{
    basic_entrypoint,
    derive::{router, Codec},
    Address,
    SharedAPI,
    U256,
};

// Level 1: Basic struct
#[derive(Codec, Debug, Clone)]
pub struct UserProfile {
    pub name: String,
    pub level: U256,
    pub verified: bool,
}

// Level 2: Struct containing another struct
#[derive(Codec, Debug, Clone)]
pub struct User {
    pub address: Address,
    pub profile: UserProfile, // nested struct
    pub balance: U256,
}

// Level 2: Another struct for testing
#[derive(Codec, Debug, Clone)]
pub struct Payment {
    pub amount: U256,
    pub token: Address,
    pub deadline: U256,
}

// Level 3: Struct with multiple nested structs
#[derive(Codec, Debug, Clone)]
pub struct Order {
    pub id: U256,
    pub buyer: User,      // nested struct
    pub seller: User,     // same nested struct type
    pub payment: Payment, // different nested struct
    pub status: u8,
}

// Level 4: Deeply nested for edge case testing
#[derive(Codec, Debug, Clone)]
pub struct OrderBatch {
    pub batch_id: U256,
    pub primary_order: Order, // 3 levels deep
    pub total_value: U256,
}

#[derive(Default)]
pub struct NestedContract<SDK> {
    sdk: SDK,
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> NestedContract<SDK> {
    /// Test: nested struct as parameter
    pub fn create_user(&mut self, user: User) -> Address {
        user.address
    }

    /// Test: deeply nested struct
    pub fn submit_order(&mut self, order: Order) -> U256 {
        order.id
    }

    /// Test: very deeply nested (4 levels)
    pub fn process_batch(&mut self, batch: OrderBatch) -> bool {
        true
    }

    /// Test: return nested struct
    pub fn get_user(&self, addr: Address) -> User {
        User {
            address: addr,
            profile: UserProfile {
                name: String::from("Alice"),
                level: U256::from(10),
                verified: true,
            },
            balance: U256::from(1000),
        }
    }

    /// Test: multiple nested struct parameters
    pub fn match_order(&mut self, buyer: User, seller: User, payment: Payment) -> Order {
        Order {
            id: U256::from(1),
            buyer,
            seller,
            payment,
            status: 1,
        }
    }
}

impl<SDK: SharedAPI> NestedContract<SDK> {
    pub fn new(sdk: SDK) -> Self {
        Self { sdk }
    }
}

basic_entrypoint!(NestedContract);
