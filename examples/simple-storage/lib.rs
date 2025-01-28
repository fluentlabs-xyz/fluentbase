#![cfg_attr(target_arch = "wasm32", no_std)]
#![allow(dead_code)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{
    alloc_slice,
    basic_entrypoint,
    derive::{solidity_storage, Contract},
    Address,
    ContractContextReader,
    SharedAPI,
    U256,
};

#[derive(Contract)]
struct SIMPLESTORAGE<SDK> {
    sdk: SDK,
}

pub trait SIMPLESTORAGEAPI {
    fn get(&self) -> U256;
    fn set(&mut self, value: U256);
}

solidity_storage! {
    mapping(Address=>U256) Values;
}

impl<SDK: SharedAPI> SIMPLESTORAGE<SDK> {
    fn deploy(&mut self) {}

    fn main(&mut self) {
        let caller = self.sdk.context().contract_caller();
        let input_size = self.sdk.input_size();
        if input_size == 0 {
            let value = Values::get(&self.sdk, caller);
            self.sdk.write(&value.to_le_bytes::<32>());
        } else {
            let input = alloc_slice(input_size as usize);
            self.sdk.read(input, 0);
            let value = U256::from_le_slice(input);
            Values::set(&mut self.sdk, caller, value);
        }
    }
}

basic_entrypoint!(SIMPLESTORAGE);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{
        journal::{JournalState, JournalStateBuilder},
        runtime::TestingContext,
        Address,
        ContractContextV1,
        U256,
    };
    use hex_literal::hex;

    fn rewrite_input<T: Into<Vec<u8>>>(
        sdk: &mut JournalState<TestingContext>,
        input: T,
        caller: Option<Address>,
    ) {
        sdk.inner.borrow_mut().native_sdk.take_output();
        sdk.inner.borrow_mut().native_sdk.set_input(input);
        sdk.rewrite_contract_context(ContractContextV1 {
            caller: caller.unwrap_or_default(),
            ..Default::default()
        });
    }
    /// Helper function to rewrite input and contract context.
    fn with_test_input<T: Into<Vec<u8>>>(
        input: T,
        caller: Option<Address>,
    ) -> JournalState<TestingContext> {
        JournalStateBuilder::default()
            .with_contract_context(ContractContextV1 {
                caller: caller.unwrap_or_default(),
                ..Default::default()
            })
            .with_devnet_genesis()
            .build(TestingContext::empty().with_input(input))
    }
    #[test]
    fn test_simple_storage_set_and_get() {
        let owner_address = Address::from(hex!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));
        let test_value = U256::from(42);
        let sdk = with_test_input(
            Vec::from(test_value.to_le_bytes::<32>()),
            Some(owner_address),
        );
        let mut simple_storage = SIMPLESTORAGE::new(sdk);
        simple_storage.main(); // Set value
        rewrite_input(&mut simple_storage.sdk, vec![], Some(owner_address));
        simple_storage.main(); // Get value
        let output = simple_storage
            .sdk
            .inner
            .borrow_mut()
            .native_sdk
            .take_output();
        let retrieved_value = U256::from_le_slice(&output);
        assert_eq!(retrieved_value, test_value);
    }
}
