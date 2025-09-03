#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{entrypoint, SharedAPI};

pub fn main_entry(mut sdk: impl SharedAPI) {
    // write "Hello, World" message into output
    sdk.write("Hello, World".as_bytes());
}

entrypoint!(main_entry);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk_testing::HostTestingContext;

    #[test]
    fn test_contract_works() {
        let sdk = HostTestingContext::default();
        main_entry(sdk.clone());
        let output = sdk.take_output();
        assert_eq!(&output, "Hello, World".as_bytes());
    }
}
