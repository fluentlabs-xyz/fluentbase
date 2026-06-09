//! EIP-2537 ↔ compressed round-trip coverage for the MinSig point encodings.
//!
//! The forward direction ships in `fluentbase_bls::encoding`; the inverse is a
//! test-only helper in `tests/common`. These tests assert `reverse(forward(x))
//! == x` over many keys plus a randomized property, so a byte-order regression
//! in either direction is caught.

mod common;

use common::{pubkey_eip2537_to_compressed, signature_eip2537_to_compressed};
use fluentbase_bls::{
    encoding::{pubkey_compressed_to_eip2537, signature_compressed_to_eip2537},
    fluent_namespace,
    keys::ValidatorBlsKeypair,
    pop::sign_pop,
};
use proptest::prelude::*;
use rand_08::rngs::StdRng;
use rand_core::SeedableRng;

fn kp(seed: u64) -> ValidatorBlsKeypair {
    ValidatorBlsKeypair::generate(&mut StdRng::seed_from_u64(seed))
}

#[test]
fn pubkey_roundtrip() {
    for seed in 0..64u64 {
        let comp = kp(seed).public_bytes();
        let eip = pubkey_compressed_to_eip2537(&comp).unwrap();
        let back = pubkey_eip2537_to_compressed(&eip).unwrap();
        assert_eq!(comp, back, "seed {seed}");
    }
}

#[test]
fn signature_roundtrip() {
    for seed in 0..64u64 {
        let k = kp(seed);
        let sig = sign_pop(&k, &fluent_namespace(20994));
        let eip = signature_compressed_to_eip2537(&sig).unwrap();
        let back = signature_eip2537_to_compressed(&eip).unwrap();
        assert_eq!(sig, back, "seed {seed}");
    }
}

proptest! {
    #[test]
    fn pubkey_roundtrip_prop(seed in any::<u64>()) {
        let comp = ValidatorBlsKeypair::generate(&mut StdRng::seed_from_u64(seed)).public_bytes();
        let eip = pubkey_compressed_to_eip2537(&comp).unwrap();
        prop_assert_eq!(comp, pubkey_eip2537_to_compressed(&eip).unwrap());
    }
}
