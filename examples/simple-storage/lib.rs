#![cfg_attr(target_arch = "wasm32", no_std)]
#![allow(dead_code)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{
    debug_log,
    derive::solidity_storage,
    func_entrypoint,
    Address,
    SharedAPI,
    U256,
};

solidity_storage! {
    mapping(Address => U256) Values;
}

pub fn deploy(mut sdk: impl SharedAPI) {
    sdk.write_storage(U256::from(1), U256::from(2));
}

pub fn main(sdk: impl SharedAPI) {
    debug_log!("Message - 1");
    let value = sdk.storage(&U256::from(1));
    debug_log!("Message - 2"); // This is used to test wasmtime interrupts
    assert_eq!(value.data, U256::from(2));
}

func_entrypoint!(main, deploy);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::testing::TestingContext;

    #[test]
    fn test_simple_storage_set_and_get() {
        let sdk = TestingContext::default();
        deploy(sdk.clone());
        main(sdk.clone());
    }
}
