#![cfg_attr(target_arch = "wasm32", no_std)]
#![allow(dead_code)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{
    basic_entrypoint,
    derive::{solidity_storage, Contract},
    SharedAPI,
    U256,
};

#[derive(Contract)]
struct App<SDK> {
    sdk: SDK,
}

solidity_storage! {
    U256 Value;
}

impl<SDK: SharedAPI> App<SDK> {
    fn deploy(&mut self) {
        let mut input = [0u8; 32];
        self.sdk.read(&mut input, 0);
        let value = U256::from_le_bytes(input);
        self.sdk.write_storage(Value::SLOT, value);
    }

    fn main(&mut self) {
        let value = self.sdk.storage(&Value::SLOT);
        self.sdk.write(&value.to_le_bytes::<32>());
    }
}

basic_entrypoint!(App);
