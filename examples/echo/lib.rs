#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{alloc_slice, basic_entrypoint, derive::Contract, SharedAPI};

#[derive(Contract)]
struct ECHO<SDK> {
    sdk: SDK,
}

impl<SDK: SharedAPI> ECHO<SDK> {
    fn deploy(&mut self) {
        // any custom deployment logic here
    }
    fn main(&mut self) {
        let input_size = self.sdk.input_size();
        let input = alloc_slice(input_size as usize);
        self.sdk.read(input, 0);
        self.sdk.write(input);
    }
}

basic_entrypoint!(ECHO);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{journal::JournalState, runtime::TestingContext};
    use hex_literal::hex;

    #[test]
    fn test_contract_works() {
        let native_sdk = TestingContext::empty().with_input("Hello, World");
        let sdk = JournalState::empty(native_sdk.clone());
        let mut echo = ECHO::new(sdk);
        echo.deploy();
        echo.main();
        let output = native_sdk.take_output();
        assert_eq!(&output[0..12], hex!("48656c6c6f2c20576f726c64"));
    }
}
