//! Cross-language ed25519 committee-ordering conformance pin.
//!
//! Simplex assigns a `Participant` index = position of a validator's
//! ed25519 peer public key in its ordered committee container (the byte
//! ordering is the Commonware codec, `commonware_utils::ordered`). On-chain
//! signer-index resolution
//! (`Staking.sol` `commitEpochCommittee` / `resolveSigner`) reproduces that
//! order with a Solidity `bytes32` ascending sort. The two MUST agree
//! byte-for-byte or `signerIdx` resolves to the WRONG validator (slash an
//! innocent).
//!
//! Today they agree only because `ed25519::PublicKey`'s `Ord` is a
//! hand-written `cmp` over the 32-byte compressed key (`A_bytes`), which is
//! lexicographic / unsigned-big-endian — identical to Solidity `bytes32 <`.
//! This is one upstream refactor (e.g. `#[derive(Ord)]`, field reorder) away
//! from silent divergence. This test pins it: it builds the REAL Commonware
//! ordered container and asserts the order equals a plain ascending byte
//! sort. A Commonware bump that changes that `Ord` fails this test LOUDLY.
//!
//! This file is the SINGLE SOURCE and is self-documenting (no companion
//! markdown — same convention as `eip2537_conformance_vectors.rs`). The
//! `PEER_PUBKEYS_HEX` corpus + `EXPECTED_SORTED_INDICES` order below are
//! mirrored by-hand into `solidity-contracts`
//! `test/staking/StakingEpochCommittee.t.sol`
//! (`test_resolveSigner_matchesSimplexConformanceVectors`) — update both
//! in the SAME PR. Recipe: 10 ed25519 keys from
//! `PrivateKey::random(StdRng::seed_from_u64(seed))` for the fixed `SEEDS`
//! below (deliberately unsorted; corpus spans a `0x00…` and a `0xff…`
//! prefix to prove unsigned compare).
//!
//! Regenerate after a deliberate Commonware bump:
//!   cargo test -p fluentbase-bls --test ed25519_ordering_conformance \
//!       -- --ignored print_corpus --nocapture

use commonware_cryptography::ed25519::{PrivateKey, PublicKey};
use commonware_cryptography::Signer;
use commonware_math::algebra::Random;
use commonware_utils::ordered::Set;
use rand_08::rngs::StdRng;
use rand_08::SeedableRng;

/// Fixed seeds → deterministic, valid, distinct ed25519 keys. Order here is
/// deliberately NOT the sorted order, so the test proves sorting happened.
const SEEDS: &[u64] = &[7, 3, 9, 1, 5, 8, 2, 6, 4, 0];

fn peer_pubkey(seed: u64) -> [u8; 32] {
    let sk = PrivateKey::random(&mut StdRng::seed_from_u64(seed));
    let pk = sk.public_key();
    <[u8; 32]>::try_from(pk.as_ref()).expect("ed25519 pubkey is 32 bytes")
}

fn public_key(seed: u64) -> PublicKey {
    PrivateKey::random(&mut StdRng::seed_from_u64(seed)).public_key()
}

/// The committee fed to Simplex consensus, in SEED order (unsorted), lowercase hex.
/// Mirrored verbatim into the Solidity test.
const PEER_PUBKEYS_HEX: &[&str] = &[
    "478243aed376da313d7cf3a60637c264cb36acc936efb341ff8d3d712092d244",
    "c5bbbb60e412879bbec7bb769804fa8e36e68af10d5477280b63deeaca931bed",
    "00d21610e478bc59b0c1e70505874e191bf94ab73cb1f9246f963f9bc0a1b253",
    "ff87a0b0a3c7c0ce827e9cada5ff79e75a44a0633bfcb5b50f99307ddb26b337",
    "e2e8aa145e1ec5cb01ebfaa40e10e12f0230c832fd8135470c001cb86d77de00",
    "9ab068880fcc795c1ac317b9b5acff698a04b2f9fba6eea41f013dc9942fd8e2",
    "191fc38f134aaf1b7fdb1f86330b9d03e94bd4ba884f490389de964448e89b3f",
    "17888c2ca502371245e5e35d5bcf35246c3bc36878e859938c9ead3c54db174f",
    "4f44e6c7bdfed3d9f48d86149ee3d29382cae8c83ca253e06a70be54a301828b",
    "ee1aa49a4459dfe813a3cf6eb882041230c7b2558469de81f87c9bf23bf10a03",
];

/// Expected Simplex committee order AFTER sorting (= ascending byte lex,
/// Commonware codec).
/// Each entry is the index into `PEER_PUBKEYS_HEX` / `SEEDS`. Mirrored into
/// the Solidity test as the expected `getEpochCommittee` order.
const EXPECTED_SORTED_INDICES: &[usize] = &[2, 7, 6, 0, 8, 5, 1, 4, 9, 3];

#[test]
fn commonware_orders_ed25519_committee_by_raw_byte_lex() {
    // Build the REAL Commonware ordered container from the committee.
    let keys: Vec<PublicKey> = SEEDS.iter().map(|&s| public_key(s)).collect();
    let set: Set<PublicKey> = Set::try_from(keys).expect("distinct keys");
    let commonware_order: Vec<[u8; 32]> = set
        .iter()
        .map(|k| <[u8; 32]>::try_from(k.as_ref()).unwrap())
        .collect();

    // Independent reference: plain ascending byte sort (== Solidity bytes32 <).
    let mut byte_lex: Vec<[u8; 32]> = SEEDS.iter().map(|&s| peer_pubkey(s)).collect();
    byte_lex.sort();

    assert_eq!(
        commonware_order, byte_lex,
        "Simplex ed25519 committee order (Commonware codec) drifted from raw-byte lexicographic \
         order — Solidity bytes32 resolution would now slash the wrong validator"
    );

    // Sanity: the corpus is actually permuted by sorting (test would be vacuous
    // if the seed order already equaled the sorted order).
    let seed_order: Vec<[u8; 32]> = SEEDS.iter().map(|&s| peer_pubkey(s)).collect();
    assert_ne!(
        seed_order, commonware_order,
        "corpus seeds happen to be pre-sorted — pick different SEEDS so the \
         test exercises a real reordering"
    );

    // Pin against the committed cross-language corpus (mirrored into Solidity).
    if !PEER_PUBKEYS_HEX.is_empty() {
        let seed_hex: Vec<String> = SEEDS.iter().map(|&s| hex::encode(peer_pubkey(s))).collect();
        let committed: Vec<String> = PEER_PUBKEYS_HEX.iter().map(|h| h.to_string()).collect();
        assert_eq!(
            seed_hex, committed,
            "PEER_PUBKEYS_HEX no longer matches the recipe — regenerate via \
             `--ignored print_corpus` and re-mirror into the Solidity test"
        );
        let expected_sorted: Vec<[u8; 32]> = EXPECTED_SORTED_INDICES
            .iter()
            .map(|&i| peer_pubkey(SEEDS[i]))
            .collect();
        assert_eq!(
            commonware_order, expected_sorted,
            "EXPECTED_SORTED_INDICES drifted — regenerate and re-mirror"
        );
    }
}

#[test]
#[ignore = "regenerator: prints the pinned corpus for hand-mirroring"]
fn print_corpus() {
    let keys: Vec<PublicKey> = SEEDS.iter().map(|&s| public_key(s)).collect();
    let set: Set<PublicKey> = Set::try_from(keys).expect("distinct keys");
    let order: Vec<[u8; 32]> = set
        .iter()
        .map(|k| <[u8; 32]>::try_from(k.as_ref()).unwrap())
        .collect();

    println!("\n// --- PEER_PUBKEYS_HEX (SEED order) ---");
    for &s in SEEDS {
        println!("\"{}\",", hex::encode(peer_pubkey(s)));
    }
    println!("// --- EXPECTED_SORTED_INDICES (into SEEDS / PEER_PUBKEYS_HEX) ---");
    let idx: Vec<usize> = order
        .iter()
        .map(|sorted| {
            SEEDS
                .iter()
                .position(|&s| &peer_pubkey(s) == sorted)
                .unwrap()
        })
        .collect();
    println!("{idx:?}");
    println!("// --- Solidity bytes32 literals (committee in SEED order) ---");
    for &s in SEEDS {
        println!("0x{},", hex::encode(peer_pubkey(s)));
    }
}
