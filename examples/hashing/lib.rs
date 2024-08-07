#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{alloc_slice, basic_entrypoint, derive::Contract, NativeAPI, SharedAPI};

#[derive(Contract)]
struct HASHING<SDK> {
    sdk: SDK,
}

impl<SDK: SharedAPI> HASHING<SDK> {
    fn deploy(&self) {
        // any custom deployment logic here
    }
    fn main(&self) {
        // get the size of the input and allocate memory for input
        let input_size = self.sdk.native_sdk().input_size();
        let input = alloc_slice(input_size as usize);
        // copy input to the allocated memory
        self.sdk.native_sdk().read(input, 0);
        // calculate keccak256 & poseidon hashes
        let keccak256_hash = self.sdk.native_sdk().keccak256(input);
        let poseidon_hash = self.sdk.native_sdk().poseidon(input);
        // write both hashes to output (multiple writes do append)
        self.sdk.native_sdk().write(keccak256_hash.as_slice());
        self.sdk.native_sdk().write(poseidon_hash.as_slice());
    }
}

basic_entrypoint!(HASHING);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{journal::JournalState, runtime::TestingContext};
    use hex_literal::hex;

    #[test]
    fn test_contract_works() {
        let native_sdk = TestingContext::empty().with_input("Hello, World");
        let sdk = JournalState::empty(native_sdk.clone());
        let hashing = HASHING::new(sdk);
        hashing.deploy();
        hashing.main();
        let output = native_sdk.take_output();
        assert_eq!(
            &output[0..32],
            hex!("a04a451028d0f9284ce82243755e245238ab1e4ecf7b9dd8bf4734d9ecfd0529")
        );
        assert_eq!(
            &output[32..],
            hex!("9796a3ea6a12e2df13db77ead033b6c14c213726905fb03bd8fab41c72719902")
        );
    }
}
