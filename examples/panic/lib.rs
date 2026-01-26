#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
#![allow(unused)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{entrypoint, SharedAPI};

pub fn main_entry(mut sdk: impl SharedAPI) {
    // panic with some message
    panic!("it's panic time");
}

entrypoint!(main_entry);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_testing::TestingContextImpl;

    #[should_panic(expected = "it's panic time")]
    #[test]
    fn tets_contract_works() {
        let sdk = TestingContextImpl::default();
        main_entry(sdk);
    }
}
