#![cfg_attr(target_arch = "wasm32", no_std)]
#![allow(dead_code)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{
    alloc_slice,
    basic_entrypoint,
    derive::{solidity_storage, Contract},
    SharedAPI,
    U256,
};

#[derive(Contract)]
struct CONSTRUCTORPARAMS<SDK> {
    sdk: SDK,
}

solidity_storage! {
    U256 Value;
}

impl<SDK: SharedAPI> CONSTRUCTORPARAMS<SDK> {
    fn deploy(&mut self) {
        let input_size = self.sdk.input_size();
        let input = alloc_slice(input_size as usize);
        self.sdk.read(input, 0);
        let value = U256::from_le_slice(input);
        Value::set(&mut self.sdk, value);
    }

    fn main(&mut self) {
        let value = Value::get(&self.sdk);
        self.sdk.write(&value.to_le_bytes::<32>());
    }
}

basic_entrypoint!(CONSTRUCTORPARAMS);
