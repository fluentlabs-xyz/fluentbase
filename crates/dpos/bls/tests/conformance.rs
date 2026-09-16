//! End-to-end registration → PoP → signer/verifier through the public API only;
//! a compiling, passing test shows downstream callers need no crate internals.

use commonware_codec::DecodeExt;
use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer};
use commonware_math::algebra::Random;
use commonware_utils::{ordered::BiMap, TryCollect};
use fluentbase_bls::{
    fluent_namespace, keys::ValidatorBlsKeypair, pop, scheme, BlsPubkey, PeerPubkey, Scheme,
};
use rand_08::rngs::StdRng;
use rand_core::SeedableRng;

const CHAIN_ID: u64 = 20994;

fn committee(n: usize) -> (BiMap<PeerPubkey, BlsPubkey>, Vec<ValidatorBlsKeypair>) {
    let mut rng = StdRng::seed_from_u64(0xC0FFEE);
    let peers: Vec<_> = (0..n)
        .map(|_| Ed25519PrivateKey::random(&mut rng))
        .collect();
    let blses: Vec<_> = (0..n)
        .map(|_| ValidatorBlsKeypair::generate(&mut rng))
        .collect();
    let bimap: BiMap<_, _> = peers
        .iter()
        .zip(blses.iter())
        .map(|(p, b)| {
            let bytes = b.public_bytes();
            (p.public_key(), BlsPubkey::decode(bytes.as_slice()).unwrap())
        })
        .try_collect()
        .unwrap();
    (bimap, blses)
}

#[test]
fn registration_to_signing_happy_path() {
    let ns = fluent_namespace(CHAIN_ID);
    let (bimap, blses) = committee(4);

    for kp in &blses {
        let sig = pop::sign_pop(kp, &ns);
        pop::verify_pop(&kp.public_bytes(), &ns, &sig).expect("PoP must verify at registration");
    }

    // Signer must wrap the same key material that produced the PoP above.
    let _signer: Scheme = scheme::build_signer(&ns, bimap.clone(), &blses[0], 1, None)
        .expect("validator 0 must be able to sign in committee");

    let _verifier: Scheme = scheme::build_verifier(&ns, bimap, 1, None);
}
