#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
use fluentbase_sdk::{derive::solidity_storage, entrypoint, Address, SharedAPI, U256};

solidity_storage! {
    mapping(Address => U256) Values;
}

pub fn deploy(mut sdk: impl SharedAPI) {
    sdk.write_storage(U256::from(1), U256::from(2));
}

pub fn main_entry(sdk: impl SharedAPI) {
    let value = sdk.storage(&U256::from(1));
    assert_eq!(value.data, U256::from(2));
}

entrypoint!(main_entry, deploy);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk_testing::HostTestingContext;

    #[test]
    fn test_simple_storage_set_and_get() {
        let sdk = HostTestingContext::default();
        deploy(sdk.clone());
        main_entry(sdk.clone());
    }
}
