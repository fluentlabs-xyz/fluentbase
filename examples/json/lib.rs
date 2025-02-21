#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate core;
extern crate fluentbase_sdk;

use fluentbase_sdk::{alloc_slice, func_entrypoint, SharedAPI};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct JsonInput<'a> {
    message: &'a str,
}

pub fn main(mut sdk: impl SharedAPI) {
    // read input
    let input_size = sdk.input_size() as usize;
    let mut buffer = alloc_slice(input_size);
    sdk.read(&mut buffer, 0);
    // parse json and extract name
    let (json_input, _) = serde_json_core::from_slice::<JsonInput>(&buffer)
        .unwrap_or_else(|_| panic!("invalid JSON input"));
    // write name as output
    sdk.write(json_input.message.as_bytes());
}

func_entrypoint!(main);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::testing::TestingContext;

    #[test]
    fn test_contract_works() {
        let sdk = TestingContext::default().with_input("{\"message\": \"Hello, World\"}");
        main(sdk.clone());
        let output = sdk.take_output();
        assert_eq!(&output, "Hello, World".as_bytes());
    }
}
