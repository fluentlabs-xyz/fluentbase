#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{alloc_slice, crypto::crypto_keccak256, entrypoint, SharedAPI};

pub fn main_entry<SDK: SharedAPI>(mut sdk: SDK) {
    let input_size = sdk.input_size();
    let input = alloc_slice(input_size as usize);
    sdk.read(input, 0);
    let hash = crypto_keccak256(input);
    sdk.write(&hash.as_slice());
}

entrypoint!(main_entry);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::hex;
    use fluentbase_testing::HostTestingContext;

    #[test]
    fn test_contract_works() {
        let sdk = HostTestingContext::default().with_input("Hello, World");
        main_entry(sdk.clone());
        let output = sdk.take_output();
        assert_eq!(
            &output[0..32],
            hex!("a04a451028d0f9284ce82243755e245238ab1e4ecf7b9dd8bf4734d9ecfd0529")
        );
    }
}
