#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{alloc_slice, basic_entrypoint, ContextReader, SharedAPI};

#[derive(Default)]
struct HASHING;

impl HASHING {
    fn deploy<SDK: SharedAPI>(&self) {
        // any custom deployment logic here
    }
    fn main<SDK: SharedAPI>(&self) {
        // get size of the input and allocate memory for input
        let input_size = SDK::input_size();
        let input = alloc_slice(input_size as usize);
        // copy input to the allocated memory
        SDK::read(input, 0);
        // calculate keccak256 & poseidon hashes
        let mut keccak256_hash: [u8; 32] = [0u8; 32];
        SDK::keccak256(
            input.as_ptr(),
            input.len() as u32,
            keccak256_hash.as_mut_ptr(),
        );
        let mut poseidon_hash: [u8; 32] = [0u8; 32];
        SDK::poseidon(
            input.as_ptr(),
            input.len() as u32,
            poseidon_hash.as_mut_ptr(),
        );
        // write both hashes to output (multiple writes do append)
        SDK::write(&keccak256_hash);
        SDK::write(&poseidon_hash);
    }
}

basic_entrypoint!(HASHING);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::LowLevelSDK;
    use hex_literal::hex;

    #[test]
    fn test_contract_works() {
        LowLevelSDK::with_test_input("Hello, World".into());
        let hashing = HASHING::default();
        hashing.deploy::<LowLevelSDK>();
        hashing.main::<LowLevelSDK>();
        let test_output = LowLevelSDK::get_test_output();
        assert_eq!(
            &test_output[0..32],
            hex!("a04a451028d0f9284ce82243755e245238ab1e4ecf7b9dd8bf4734d9ecfd0529")
        );
        assert_eq!(
            &test_output[32..],
            hex!("9796a3ea6a12e2df13db77ead033b6c14c213726905fb03bd8fab41c72719902")
        );
    }
}
