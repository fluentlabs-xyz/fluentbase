#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{basic_entrypoint, SharedAPI};

#[derive(Default)]
struct GREETING;

impl GREETING {
    fn deploy<SDK: SharedAPI>(&self) {
        // any custom deployment logic here
    }
    fn main<SDK: SharedAPI>(&self) {
        // write "Hello, World" message into output
        SDK::write("Hello, World".as_ptr(), 12);
    }
}

basic_entrypoint!(GREETING);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::LowLevelSDK;

    #[test]
    fn test_contract_works() {
        let greeting = GREETING::default();
        greeting.deploy::<LowLevelSDK>();
        greeting.main::<LowLevelSDK>();
        let test_output = LowLevelSDK::get_test_output();
        assert_eq!(&test_output, "Hello, World".as_bytes());
    }
}
