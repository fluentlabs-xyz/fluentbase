#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{basic_entrypoint, derive::Contract, ContextReader, SharedAPI};

#[derive(Contract)]
struct GREETING<CTX, SDK> {
    ctx: CTX,
    sdk: SDK,
}

impl<CTX: ContextReader, SDK: SharedAPI> GREETING<CTX, SDK> {
    fn deploy(&self) {
        // any custom deployment logic here
    }
    fn main(&self) {
        // write "Hello, World" message into output
        self.sdk.write("Hello, World".as_bytes());
    }
}

basic_entrypoint!(GREETING);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{runtime::TestingContext, ContractInput};

    #[test]
    fn test_contract_works() {
        let ctx = ContractInput::default();
        let sdk = TestingContext::new();
        let greeting = GREETING::new(ctx, sdk.clone());
        greeting.deploy();
        greeting.main();
        let output = sdk.output();
        assert_eq!(&output, "Hello, World".as_bytes());
    }
}
