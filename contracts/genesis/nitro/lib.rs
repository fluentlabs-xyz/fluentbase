#![cfg_attr(target_arch = "wasm32", no_std)]
#![allow(dead_code)]

extern crate alloc;
extern crate fluentbase_sdk;

mod attestation;

use fluentbase_sdk::{alloc_slice, func_entrypoint, SharedAPI};

pub fn main(sdk: impl SharedAPI) {
    let input_size = sdk.input_size();
    let input = alloc_slice(input_size as usize);
    sdk.read(input, 0);
    attestation::parse_and_verify(&input);
}

func_entrypoint!(main);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::testing::TestingContext;

    #[test]
    fn test_nitro_attestation_verification() {
        // Example of valid attestation document
        // https://github.com/evervault/attestation-doc-validation/blob/main/test-data/valid-attestation-doc-base64
        let data: Vec<u8> = hex::decode(include_bytes!("attestation-example.hex"))
            .unwrap()
            .into();
        let doc = attestation::parse_and_verify(&data);
        assert_eq!(doc.digest, "SHA384");
        let sdk = TestingContext::default().with_input(data);
        main(sdk);
    }
}
