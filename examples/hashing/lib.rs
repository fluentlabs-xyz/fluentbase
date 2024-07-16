#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate fluentbase_sdk;

use fluentbase_sdk::{alloc_slice, basic_entrypoint, derive::Contract, ContextReader, SharedAPI};

#[derive(Contract)]
struct HASHING<CTX, SDK> {
    ctx: CTX,
    sdk: SDK,
}

impl<CTX: ContextReader, SDK: SharedAPI> HASHING<CTX, SDK> {
    fn deploy(&self) {
        // any custom deployment logic here
    }
    fn main(&self) {
        // get the size of the input and allocate memory for input
        let input_size = self.sdk.input_size();
        let input = alloc_slice(input_size as usize);
        // copy input to the allocated memory
        self.sdk.read(input, 0);
        // calculate keccak256 & poseidon hashes
        let keccak256_hash = SDK::keccak256(input);
        let poseidon_hash = SDK::poseidon(input);
        // write both hashes to output (multiple writes do append)
        self.sdk.write(keccak256_hash.as_slice());
        self.sdk.write(poseidon_hash.as_slice());
    }
}

basic_entrypoint!(HASHING);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{runtime::TestingContext, ContractInput};
    use hex_literal::hex;

    #[test]
    fn test_contract_works() {
        let ctx = ContractInput::default();
        let sdk = TestingContext::new().with_input("Hello, World");
        let hashing = HASHING::new(ctx, sdk.clone());
        hashing.deploy();
        hashing.main();
        let output = sdk.output();
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
