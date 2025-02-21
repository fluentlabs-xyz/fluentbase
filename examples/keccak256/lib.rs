#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{alloc_slice, func_entrypoint, SharedAPI};

pub fn main(mut sdk: impl SharedAPI) {
    // get the size of the input and allocate memory for input
    let input_size = sdk.input_size();
    let input = alloc_slice(input_size as usize);
    // copy input to the allocated memory
    sdk.read(input, 0);
    // calculate keccak256 & poseidon hashes
    let keccak256_hash = sdk.keccak256(input);
    // write both hashes to output (multiple writes do append)
    sdk.write(keccak256_hash.as_slice());
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
        assert_eq!(
            &output[0..32],
            hex!("a04a451028d0f9284ce82243755e245238ab1e4ecf7b9dd8bf4734d9ecfd0529")
        );
    }
}
