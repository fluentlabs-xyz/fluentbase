use alloc::vec;
use cairo_platinum_prover::air::CairoAIR;
use fluentbase_sdk::{LowLevelAPI, LowLevelSDK};
use stark_platinum_prover::{
    proof::options::ProofOptions,
    transcript::StoneProverTranscript,
    verifier::{IsStarkVerifier, Verifier},
};

pub fn verify_cairo_proof_wasm(proof_bytes: &[u8], proof_options: &ProofOptions) -> bool {
    let bytes = proof_bytes;

    // This logic is the same as main verify, with only error handling changing. In wasm, we simply
    // return a false if the proof is invalid, instead of rising an error.

    // Proof len was stored as an u32, 4u8 needs to be read
    let proof_len = u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;

    let bytes = &bytes[4..];
    if bytes.len() < proof_len {
        return false;
    }

    let (proof, _) =
        bincode::serde::decode_from_slice(&bytes[0..proof_len], bincode::config::standard())
            .unwrap();

    // let Ok((proof, _)) =
    //     bincode::serde::decode_from_slice(&bytes[0..proof_len], bincode::config::standard())
    // else {
    //     return false;
    // };
    // let bytes = &bytes[proof_len..];

    let Ok((pub_inputs, _)) = bincode::serde::decode_from_slice(bytes, bincode::config::standard())
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

use crate::deploy_internal;

pub fn deploy() {
    deploy_internal(include_bytes!("../bin/cairo.wasm"))
}

pub fn main() {
    let proof_options = ProofOptions {
        blowup_factor: 4,
        fri_number_of_queries: 3,
        coset_offset: 3,
        grinding_factor: 1,
    };
    let input_size = LowLevelSDK::sys_input_size();
    let mut input_buffer = vec![0u8; input_size as usize];
    LowLevelSDK::sys_read(&mut input_buffer, 0);
    assert!(
        verify_cairo_proof_wasm(&input_buffer[..], &proof_options),
        "failed to verify cairo proof"
    );
}

#[cfg(test)]
#[test]
fn test_verify_cairo() {
    let cairo_proof = include_bytes!("../../e2e/assets/fib_proof.proof");
    LowLevelSDK::with_test_input(cairo_proof.to_vec());
    main();
}
