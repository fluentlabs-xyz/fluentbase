#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{alloc_slice, func_entrypoint, SharedAPI};

pub fn main(mut sdk: impl SharedAPI) {
    let input_size = sdk.input_size();
    let input = alloc_slice(input_size as usize);
    sdk.read(input, 0);
    sdk.write(input);
}

func_entrypoint!(main);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::testing::TestingContext;
    use hex_literal::hex;

    #[test]
    fn test_contract_works() {
        let sdk = TestingContext::default().with_input("Hello, World");
        main(sdk.clone());
        let output = sdk.take_output();
        assert_eq!(&output[0..12], hex!("48656c6c6f2c20576f726c64"));
    }
}
