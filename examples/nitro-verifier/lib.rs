#![cfg_attr(target_arch = "wasm32", no_std)]
#![allow(dead_code)]

extern crate alloc;
extern crate fluentbase_sdk;
mod attestation;

use fluentbase_sdk::{alloc_slice, basic_entrypoint, derive::Contract, SharedAPI};

#[derive(Contract)]
struct NITROVERIFIER<SDK> {
    sdk: SDK,
}

impl<SDK: SharedAPI> NITROVERIFIER<SDK> {
    fn deploy(&mut self) {}

    fn main(&mut self) {
        let input_size = self.sdk.input_size();
        let input = alloc_slice(input_size as usize);
        self.sdk.read(input, 0);
        attestation::parse_and_verify(&input);
    }
}

basic_entrypoint!(NITROVERIFIER);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{
        journal::{JournalState, JournalStateBuilder},
        runtime::TestingContext,
        Address,
        ContractContext,
    };
    use hex_literal::hex;

    /// Helper function to rewrite input and contract context.
    fn with_test_input<T: Into<Vec<u8>>>(
        input: T,
        caller: Option<Address>,
    ) -> JournalState<TestingContext> {
        JournalStateBuilder::default()
            .with_contract_context(ContractContext {
                caller: caller.unwrap_or_default(),
                ..Default::default()
            })
            .with_devnet_genesis()
            .build(TestingContext::empty().with_input(input))
    }

    #[test]
    fn test_nitro_attestation_verification() {
        // Example of valid attestation document
        // https://github.com/evervault/attestation-doc-validation/blob/main/test-data/valid-attestation-doc-base64
        let data: Vec<u8> = hex::decode(include_bytes!("attestation-example.hex"))
            .unwrap()
            .into();
        let doc = attestation::parse_and_verify(&data);
        assert_eq!(doc.digest, "SHA384");

        let sdk = with_test_input(
            Vec::from(data),
            Some(Address::from(hex!(
                "f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
            ))),
        );
        let mut nitro_verifier = NITROVERIFIER::new(sdk);
        nitro_verifier.main();
    }
}
