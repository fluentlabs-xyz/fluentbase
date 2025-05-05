#![cfg_attr(target_arch = "wasm32", no_std)]
#![allow(unused)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{func_entrypoint, SharedAPI};

pub fn main(mut sdk: impl SharedAPI) {
    // panic with some message
    panic!("it's panic time");
}

func_entrypoint!(main);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::testing::TestingContext;

    #[should_panic(expected = "it's panic time")]
    #[test]
    fn tets_contract_works() {
        let sdk = TestingContext::default();
        main(sdk);
    }
}
