#![cfg_attr(target_arch = "wasm32", no_std)]
#![allow(dead_code)]
extern crate alloc;
extern crate fluentbase_sdk;

use alloc::vec::Vec;
use fluentbase_sdk::{
    alloc_slice,
    basic_entrypoint,
    derive::{solidity_storage, Contract},
    Address,
    Bytes,
    ContractContextReader,
    SharedAPI,
};

#[derive(Contract)]
struct SIMPLESTORAGE<SDK> {
    sdk: SDK,
}

pub trait SIMPLESTORAGEAPI {
    fn get(&self) -> Bytes;
    fn set(&mut self, value: Bytes);
}

solidity_storage! {
    mapping(Address => Bytes) Values;
}

impl<SDK: SharedAPI> SIMPLESTORAGE<SDK> {
    fn deploy(&mut self) {}

    fn main(&mut self) {
        let input_size = self.sdk.input_size();
        let caller = self.sdk.context().contract_caller();
        if input_size == 0 {
            let value = Values::get(&self.sdk, caller);
            self.sdk.write(&value[..]);
        } else {
            let input = alloc_slice(input_size as usize);
            self.sdk.read(input, 0);
            let value = Bytes::from(Vec::from(input));
            Values::set(&mut self.sdk, caller, value);
        }
    }
}

basic_entrypoint!(SIMPLESTORAGE);
