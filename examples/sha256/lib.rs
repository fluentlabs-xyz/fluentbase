#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{crypto::crypto_sha256, entrypoint, SharedAPI};

pub fn main_entry<SDK: SharedAPI>(mut sdk: SDK) {
    let input = sdk.bytes_input();
    let hash = crypto_sha256(input); // calculate the hash via syscall to builtin keccak256
    sdk.write(&hash.as_slice());
}

entrypoint!(main_entry);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::hex;
    use fluentbase_testing::TestingContextImpl;

    #[test]
    fn test_contract_works() {
        let sdk = TestingContextImpl::default().with_input("Hello, World");
        main_entry(sdk.clone());
        let output = sdk.take_output();
        assert_eq!(
            &output[0..32],
            hex!("03675ac53ff9cd1535ccc7dfcdfa2c458c5218371f418dc136f2d19ac1fbe8a5")
        );
    }
}
