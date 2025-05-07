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
    use fluentbase_sdk_testing::HostTestingContext;

    #[should_panic(expected = "it's panic time")]
    #[test]
    fn tets_contract_works() {
        let sdk = HostTestingContext::default();
        main(sdk);
    }
}
