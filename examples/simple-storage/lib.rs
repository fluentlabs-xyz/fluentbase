#![cfg_attr(target_arch = "wasm32", no_std)]
#![allow(dead_code)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{
    alloc_slice,
    basic_entrypoint,
    derive::{solidity_storage, Contract},
    Address,
    U256,
    ContractContextReader,
    SharedAPI,
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
    mapping(Address => U256) Values;
}

impl<SDK: SharedAPI> SIMPLESTORAGE<SDK> {
    fn deploy(&mut self) {}

    fn main(&mut self) {
        let input_size = self.sdk.input_size();
        let caller = self.sdk.context().contract_caller();
        if input_size == 0 {
            let value = Values::get(&self.sdk, caller);
            let value = value + U256::from(1);
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
