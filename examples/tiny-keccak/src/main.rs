#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{alloc_slice, entrypoint, SharedAPI};
use tiny_keccak::{Hasher, Keccak};

pub fn main_entry(mut sdk: impl SharedAPI) {
    // get the size of the input and allocate memory for input
    let input_size = sdk.input_size();
    let input = alloc_slice(input_size as usize);
    // copy input to the allocated memory
    sdk.read(input, 0);
    // calculate keccak256 hash
    let mut keccak256 = Keccak::v256();
    let mut output = [0u8; 32];
    keccak256.update(input);
    keccak256.finalize(&mut output);
    // write both hashes to output (multiple writes do append)
    sdk.write(&output);
}

entrypoint!(main_entry);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::hex;
    use fluentbase_sdk_testing::HostTestingContext;

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
