#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
extern crate fluentbase_sdk;

use cairo_platinum_prover::air::CairoAIR;
use fluentbase_sdk::{alloc_slice, basic_entrypoint, derive::Contract, NativeAPI, SharedAPI};
use stark_platinum_prover::{
    proof::options::{ProofOptions, SecurityLevel},
    transcript::StoneProverTranscript,
    verifier::{IsStarkVerifier, Verifier},
};

#[derive(Contract)]
struct CAIRO<SDK> {
    sdk: SDK,
}

impl<SDK: SharedAPI> CAIRO<SDK> {
    fn deploy(&self) {
        // any custom deployment logic here
    }
    fn verify_cairo_proof_wasm(&self, proof_bytes: &[u8], proof_options: &ProofOptions) -> bool {
        let bytes = proof_bytes;
        // This logic is the same as main verifying, with only error handling changing. In wasm, we
        // simply return a false if the proof is invalid, instead of rising an error.
        // Proof len was stored as an u32, 4u8 needs to be read
        let proof_len = u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;
        let bytes = &bytes[4..];
        if bytes.len() < proof_len {
            return false;
        }
        let Ok((proof, _)) =
            bincode::serde::decode_from_slice(&bytes[0..proof_len], bincode::config::standard())
        else {
            return false;
        };
        let bytes = &bytes[proof_len..];
        let Ok((pub_inputs, _)) =
            bincode::serde::decode_from_slice(bytes, bincode::config::standard())
        else {
            return false;
        };
        Verifier::<CairoAIR>::verify(
            &proof,
            &pub_inputs,
            proof_options,
            StoneProverTranscript::new(&[]),
        )
    }
    fn main(&self) {
        let proof_options = ProofOptions::new_secure(SecurityLevel::Conjecturable100Bits, 3);
        let input_size = self.sdk.native_sdk().input_size();
        let input = alloc_slice(input_size as usize);
        self.sdk.native_sdk().read(input, 0);
        assert!(
            self.verify_cairo_proof_wasm(input, &proof_options),
            "failed to verify cairo proof"
        );
    }
}

basic_entrypoint!(CAIRO);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{journal::JournalState, runtime::TestingContext};

    #[test]
    fn test_contract_works() {
        let cairo_proof = include_bytes!("./fib100.proof");
        let sdk = TestingContext::new().with_input(cairo_proof);
        let cairo = CAIRO::new(JournalState::empty(sdk));
        cairo.deploy();
        cairo.main();
    }
}
