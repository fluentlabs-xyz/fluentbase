#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
extern crate fluentbase_sdk;

use alloc::vec;
use cairo_platinum_prover::air::CairoAIR;
use fluentbase_sdk::{basic_entrypoint, SharedAPI};
use stark_platinum_prover::{
    proof::options::{ProofOptions, SecurityLevel},
    transcript::StoneProverTranscript,
    verifier::{IsStarkVerifier, Verifier},
};

#[derive(Default)]
struct CAIRO;

impl CAIRO {
    fn deploy<SDK: SharedAPI>(&self) {
        // any custom deployment logic here
    }
    fn verify_cairo_proof_wasm(&self, proof_bytes: &[u8], proof_options: &ProofOptions) -> bool {
        let bytes = proof_bytes;
        // This logic is the same as main verify, with only error handling changing. In wasm, we
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
    fn main<SDK: SharedAPI>(&self) {
        let proof_options = ProofOptions::new_secure(SecurityLevel::Conjecturable100Bits, 3);
        let input_size = SDK::input_size();
        let mut input_buffer = vec![0u8; input_size as usize];
        SDK::read(input_buffer.as_mut_ptr(), input_buffer.len() as u32, 0);
        assert!(
            self.verify_cairo_proof_wasm(&input_buffer[..], &proof_options),
            "failed to verify cairo proof"
        );
    }
}

basic_entrypoint!(CAIRO);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::LowLevelSDK;

    #[test]
    fn test_contract_works() {
        let cairo_proof = include_bytes!("./fib100.proof");
        LowLevelSDK::with_test_input(cairo_proof.to_vec());
        let cairo = CAIRO::default();
        cairo.deploy::<LowLevelSDK>();
        cairo.main::<LowLevelSDK>();
    }
}
