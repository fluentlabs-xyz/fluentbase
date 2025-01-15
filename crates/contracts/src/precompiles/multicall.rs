#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;
extern crate fluentbase_sdk;

use alloc::vec::Vec;
use fluentbase_sdk::{
    basic_entrypoint,
    bytes::Buf,
    derive::{function_id, router, Contract},
    Bytes,
    ContractContextReader,
    SharedAPI,
};

#[derive(Contract)]
struct Multicall<SDK> {
    sdk: SDK,
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> Multicall<SDK> {
    #[function_id("multicall(bytes[])")] // 0xac9650d8
    pub fn multicall(&mut self, data: Vec<Bytes>) -> Vec<Bytes> {
        // Use target address from context - this is the address of original contract
        let target_addr = self.sdk.context().contract_address();

        let mut results = Vec::with_capacity(data.len());

        for call_data in data {
            let chunk = call_data.chunk();
            // Always delegate call to the target contract address
            let (output, exit_code) = self.sdk.delegate_call(target_addr, chunk, 0);

            // If any of the delegate calls fail, panic
            if exit_code != 0 {
                panic!("Multicall: delegate call failed");
            }

            results.push(output);
        }

        results
    }
}

#[allow(dead_code)]
impl<SDK: SharedAPI> Multicall<SDK> {
    fn deploy(&self) {
        // any custom deployment logic here
    }
}

basic_entrypoint!(Multicall);
