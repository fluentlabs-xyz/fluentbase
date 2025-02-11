#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate core;
extern crate fluentbase_sdk;

use fluentbase_sdk::{alloc_slice, basic_entrypoint, derive::Contract, SharedAPI};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct JsonInput<'a> {
    message: &'a str,
}

#[derive(Contract)]
struct GREETING<SDK> {
    sdk: SDK,
}

impl<SDK: SharedAPI> GREETING<SDK> {
    fn deploy(&mut self) {
        // any custom deployment logic here
    }
    fn main(&mut self) {
        // read input
        let input_size = self.sdk.input_size() as usize;
        let mut buffer = alloc_slice(input_size);
        self.sdk.read(&mut buffer, 0);
        // parse json and extract name
        let (json_input, _) = serde_json_core::from_slice::<JsonInput>(&buffer)
            .unwrap_or_else(|_| panic!("invalid JSON input"));
        // write name as output
        self.sdk.write(json_input.message.as_bytes());
    }
}

basic_entrypoint!(GREETING);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{journal::JournalState, runtime::TestingContext};

    #[test]
    fn test_contract_works() {
        let native_sdk = TestingContext::empty().with_input("{\"message\": \"Hello, World\"}");
        let sdk = JournalState::empty(native_sdk.clone());
        let mut greeting = GREETING::new(sdk);
        greeting.deploy();
        greeting.main();
        let output = native_sdk.take_output();
        assert_eq!(&output, "Hello, World".as_bytes());
    }
}
