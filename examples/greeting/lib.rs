#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{func_entrypoint, SharedAPI};

pub fn main(mut sdk: impl SharedAPI) {
    // write "Hello, World" message into output
    sdk.write("Hello, World".as_bytes());
}

func_entrypoint!(main);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::testing::TestingContext;

    #[test]
    fn test_contract_works() {
        let sdk = TestingContext::default();
        main(sdk.clone());
        let output = sdk.take_output();
        assert_eq!(&output, "Hello, World".as_bytes());
    }
}
