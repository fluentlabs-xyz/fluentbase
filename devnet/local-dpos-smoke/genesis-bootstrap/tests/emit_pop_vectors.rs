//! Generator for the BLS fixtures the staking contract's e2e suite runs on.
//!
//! Those fixtures live in the OTHER worktree —
//! `feat/flu-989-port-solidity-delta:e2e/src/bls_pop_vectors.bin` — because that
//! is where the contract sources are until they merge. The bytes are opaque
//! there and this is their only provenance, so the generator stays here rather
//! than becoming a recipe in a comment: the fixtures have to come from the NODE's
//! signer (`fluentbase-bls`, i.e. `blst`) or they prove nothing about the
//! contract's agreement with it.
//!
//! Both are `#[ignore]`d: they write files and take arguments, so they run when
//! somebody regenerates the fixtures, not on every `cargo test`.
//!
//! ```text
//! VEC_CHAIN_ID=1 VEC_COUNT=400 VEC_OUT=/path/bls_pop_vectors.bin \
//!   cargo test -p fluentbase-genesis-bootstrap --test emit_pop_vectors -- \
//!   --ignored emit_pop_vectors --nocapture
//! ```
//!
//! `VEC_CHAIN_ID` is load-bearing: a proof of possession signs
//! `"FLUENT_DPOS_V1_" ‖ chain_id` as a big-endian u64, so fixtures are only
//! valid on the chain id they were made for. The consumer pins it as
//! `bls_vectors::CHAIN_ID`.

use fluentbase_genesis_bootstrap::{keys, pop};

const MNEMONIC: &str = "test test test test test test test test test test test junk";

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} must be set; see the module docs"))
}

/// `pubkey_uncompressed(256) ‖ pop_uncompressed(128) ‖ pubkey_compressed(96)`
/// per validator, in derivation-index order.
#[test]
#[ignore]
fn emit_pop_vectors() {
    let chain_id: u64 = env("VEC_CHAIN_ID").parse().unwrap();
    let count: u32 = env("VEC_COUNT").parse().unwrap();
    let set = keys::derive(MNEMONIC, count, chain_id).unwrap();
    let mut out = Vec::new();
    for validator in &set.validators {
        let artefacts = pop::produce(&validator.bls, chain_id).unwrap();
        out.extend_from_slice(&artefacts.bls_pubkey_uncompressed);
        out.extend_from_slice(&artefacts.bls_pop_uncompressed);
        out.extend_from_slice(&validator.bls.public_bytes());
    }
    std::fs::write(env("VEC_OUT"), &out).unwrap();
    println!("wrote {} bytes", out.len());
}

/// Real MinSig signatures by validator 0 over the two proposals inside the
/// conformance evidence blob, under the NOTARIZE domain — what lets the e2e
/// slash path run to a real verdict instead of stopping at a hash gate.
///
/// Emits `sig_compressed(48) ‖ sig_uncompressed(128)` per half.
///
/// The secret is re-derived here rather than taken from the keypair, because
/// `ValidatorBlsKeypair::secret` is crate-private. The derivation must stay in
/// step with `keys::derive`; if it drifts, the signatures verify against nothing
/// and the consumer fails loudly rather than silently.
#[test]
#[ignore]
fn emit_evidence_signatures() {
    use commonware_codec::EncodeFixed;
    use commonware_cryptography::bls12381::primitives::ops;
    use fluentbase_bls::{encoding::signature_compressed_to_eip2537, fluent_namespace, Variant};
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng;
    use sha2::{Digest, Sha256};

    let chain_id: u64 = env("VEC_CHAIN_ID").parse().unwrap();
    let m = coins_bip39::Mnemonic::<coins_bip39::English>::new_from_phrase(MNEMONIC).unwrap();
    let seed = m.to_seed(None).unwrap();
    let mut h = Sha256::new();
    h.update(b"fluent-dpos-smoke-v1");
    h.update(seed.as_slice());
    h.update(b"|");
    h.update(b"bls");
    h.update(b"|");
    h.update(0u32.to_be_bytes());
    let bls_seed: [u8; 32] = h.finalize().into();
    let mut rng = StdRng::from_seed(bls_seed);
    let (secret, _public) = ops::keypair::<_, Variant>(&mut rng);

    // `contracts/staking/src/evidence.rs::tests::CONFLICTING_NOTARIZE`. Each
    // 84-byte half is `Proposal(35) ‖ signerIdx(1) ‖ sig(48)`, and the proposal
    // is exactly what was signed.
    let blob = hex::decode(concat!(
        "072a29aa000000000000000000000000000000000000000000000000000000000000aa",
        "038aa1d24f195fc333878b14744f62a363acf0051249c949c4cc473850991aa708",
        "41eea2171a333b13de2e61fed4936305",
        "072a29bb000000000000000000000000000000000000000000000000000000000000bb",
        "03923c9abd2f0abe63eed5a2d9ac175032b2b48685c61f9e6a7c8b7419d7807782",
        "1d82a3bfd41a5f10bcfcd8434444f820",
    ))
    .unwrap();
    let mut namespace = fluent_namespace(chain_id);
    namespace.extend_from_slice(b"_NOTARIZE");

    let mut out = Vec::new();
    for half in 0..2usize {
        let message = &blob[half * 84..half * 84 + 35];
        let signature = ops::sign_message::<Variant>(&secret, &namespace, message);
        let compressed = signature.encode_fixed::<48>();
        let uncompressed = signature_compressed_to_eip2537(&compressed).unwrap();
        out.extend_from_slice(&compressed);
        out.extend_from_slice(&uncompressed);
    }
    std::fs::write(env("VEC_OUT"), &out).unwrap();
    println!("wrote {} bytes", out.len());
}
