#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{basic_entrypoint, derive::Contract, NativeAPI, SharedAPI};

#[derive(Contract)]
struct GREETING<SDK> {
    sdk: SDK,
}

impl<SDK: SharedAPI> GREETING<SDK> {
    fn deploy(&self) {
        // any custom deployment logic here
    }
    fn main(&self) {
        // write "Hello, World" message into output
        self.sdk.native_sdk().write("Hello, World".as_bytes());
    }
}

basic_entrypoint!(GREETING);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{journal::JournalState, runtime::TestingContext};

    #[test]
    fn test_contract_works() {
        let native_sdk = TestingContext::new().with_input("Hello, World");
        let sdk = JournalState::empty(native_sdk.clone());
        let greeting = GREETING::new(sdk);
        greeting.deploy();
        greeting.main();
        let output = native_sdk.take_output();
        assert_eq!(&output, "Hello, World".as_bytes());
    }
}
