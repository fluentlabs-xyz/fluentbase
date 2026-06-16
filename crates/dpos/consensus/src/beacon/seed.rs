//! Per-height threshold randomness seed: the cryptographic core of the beacon.
//!
//! Each committee member partial-signs `(seed_namespace ‖ height)` with its
//! DKG share ONLY AFTER that height is ordering-finalized (the sign-after-
//! finalize / "robust instant VRF" rule — the producing leader can no longer
//! bias or abort, since the height is already final). Any ≥t partials recover
//! the same unique threshold signature; `prev_randao(height) =
//! keccak256(signature)`. A verifier-only node (no share) checks a recovered
//! seed against the group public key `PK_epoch` published on L2.

use crate::beacon::types::Seed;
use alloy_primitives::{keccak256, B256};
use commonware_codec::{Encode as _, Read as _};
use commonware_cryptography::bls12381::primitives::{
    group::Share,
    ops,
    ops::threshold,
    sharing::{ModeVersion, Sharing},
    variant::{MinSig, PartialSignature, Variant},
    Error,
};
use commonware_parallel::Sequential;
use commonware_utils::N3f1;
use core::num::NonZeroU32;
use fluentbase_p2p::constants::MAX_COMMITTEE_SIZE;

/// Domain separator suffix for beacon seed signatures — distinct from the
/// consensus vote and proof-of-possession namespaces so a beacon seed can
/// never be replayed as a consensus signature (or vice versa).
const BEACON_SEED_SUFFIX: &[u8] = b"_BEACON_SEED";

/// The seed signing namespace for a chain: `chain_namespace ‖ "_BEACON_SEED"`.
/// Binding to the chain namespace prevents cross-chain seed replay.
pub fn seed_namespace(chain_namespace: &[u8]) -> Vec<u8> {
    let mut ns = Vec::with_capacity(chain_namespace.len() + BEACON_SEED_SUFFIX.len());
    ns.extend_from_slice(chain_namespace);
    ns.extend_from_slice(BEACON_SEED_SUFFIX);
    ns
}

/// The message signed for a given ordering height (big-endian, fixed width).
fn seed_message(height: u64) -> [u8; 8] {
    height.to_be_bytes()
}

/// The group public key `PK_epoch` for the beacon — what a verifier-only node
/// checks recovered seeds against (read from L2 / the local DKG output).
pub type GroupPublic = <MinSig as Variant>::Public;

/// Partial-sign the seed for `height` with this member's DKG share.
pub fn sign_seed_partial(share: &Share, namespace: &[u8], height: u64) -> PartialSignature<MinSig> {
    threshold::sign_message::<MinSig>(share, namespace, &seed_message(height))
}

/// Verify a single partial against the public polynomial (used while collecting
/// partials before recovery, to drop invalid contributions).
pub fn verify_seed_partial(
    sharing: &Sharing<MinSig>,
    namespace: &[u8],
    height: u64,
    partial: &PartialSignature<MinSig>,
) -> bool {
    threshold::verify_message::<MinSig>(sharing, namespace, &seed_message(height), partial).is_ok()
}

/// Recover the unique threshold seed for `height` from ≥t verified partials.
pub fn recover_seed(
    sharing: &Sharing<MinSig>,
    partials: &[PartialSignature<MinSig>],
    height: u64,
) -> Result<Seed, Error> {
    let signature = threshold::recover::<MinSig, _, N3f1>(sharing, partials, &Sequential)?;
    Ok(Seed {
        target_height: height,
        signature,
    })
}

/// Verify a recovered seed against the group public key `PK_epoch` — the only
/// check a verifier-only node (no share / no polynomial) can run.
pub fn verify_seed(group_public: &GroupPublic, namespace: &[u8], seed: &Seed) -> bool {
    ops::verify_message::<MinSig>(
        group_public,
        namespace,
        &seed_message(seed.target_height),
        &seed.signature,
    )
    .is_ok()
}

/// Decode the beacon public polynomial (`Sharing`) from its encoded bytes
/// (e.g. a node's loaded `beacon-sharing.hex`), bounding the committee to
/// `MAX_COMMITTEE_SIZE` — the config the codec requires. `.public()` of the
/// result is `PK_epoch`. Rejects trailing bytes (a well-formed encoding is
/// fully consumed).
pub fn parse_sharing(bytes: &[u8]) -> Result<Sharing<MinSig>, commonware_codec::Error> {
    let max = NonZeroU32::new(MAX_COMMITTEE_SIZE as u32).expect("MAX_COMMITTEE_SIZE > 0");
    let mut buf = bytes;
    let sharing = Sharing::<MinSig>::read_cfg(&mut buf, &(max, ModeVersion::v0()))?;
    if !buf.is_empty() {
        return Err(commonware_codec::Error::Invalid(
            "beacon_seed",
            "trailing bytes after Sharing",
        ));
    }
    Ok(sharing)
}

/// Decode a single DKG `Share` from its encoded bytes (a node's loaded
/// `beacon-share.hex`). Rejects trailing bytes.
pub fn parse_share(bytes: &[u8]) -> Result<Share, commonware_codec::Error> {
    let mut buf = bytes;
    let share = Share::read_cfg(&mut buf, &())?;
    if !buf.is_empty() {
        return Err(commonware_codec::Error::Invalid(
            "beacon_seed",
            "trailing bytes after Share",
        ));
    }
    Ok(share)
}

/// Derive the EVM `prev_randao` from a seed: `keccak256(threshold signature)`.
/// Deterministic across nodes (the threshold signature is unique).
pub fn prev_randao_from_seed(seed: &Seed) -> B256 {
    keccak256(seed.signature.encode())
}

/// Decide `prev_randao` for the EVM block at `height`, with the beacon-failure
/// fallback GATED so it can never halt consensus (research Decisions Q4 / G1).
///
/// Returns `(prev_randao, assurance)`. Threshold randomness is used ONLY when a
/// seed for exactly this height is present, `PK_epoch` is known, and the seed
/// verifies against it — then `(keccak256(seed), true)`. In every other case
/// (no `PK_epoch` yet, seed not recovered in time, or a claimed-but-invalid
/// seed) it degrades to the deterministic weak `fallback` with `assurance =
/// false`, so the chain stays live and randomness-dependent apps can pause.
pub fn prev_randao_for_height(
    height: u64,
    seed: Option<&Seed>,
    pk_epoch: Option<&GroupPublic>,
    namespace: &[u8],
    fallback: B256,
) -> (B256, bool) {
    match (seed, pk_epoch) {
        (Some(s), Some(pk)) if s.target_height == height && verify_seed(pk, namespace, s) => {
            (prev_randao_from_seed(s), true)
        }
        _ => (fallback, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_cryptography::bls12381::dkg::deal_anonymous;
    use commonware_utils::{test_rng, NZU32};

    #[test]
    fn seed_sign_recover_verify_roundtrip() {
        let mut rng = test_rng();
        let (sharing, shares) =
            deal_anonymous::<MinSig, N3f1>(&mut rng, Default::default(), NZU32!(5));
        let ns = seed_namespace(b"fluent-devnet");
        let height = 1969u64;

        let partials: Vec<_> = shares
            .iter()
            .map(|s| sign_seed_partial(s, &ns, height))
            .collect();
        for p in &partials {
            assert!(verify_seed_partial(&sharing, &ns, height, p));
        }

        let seed = recover_seed(&sharing, &partials, height).expect("recover");
        assert_eq!(seed.target_height, height);
        assert!(verify_seed(sharing.public(), &ns, &seed));

        // A seed signed under a different chain namespace must not verify.
        let other = seed_namespace(b"other-chain");
        assert!(!verify_seed(sharing.public(), &other, &seed));
    }

    #[test]
    fn prev_randao_is_unique_per_height_and_deterministic() {
        let mut rng = test_rng();
        let (sharing, shares) =
            deal_anonymous::<MinSig, N3f1>(&mut rng, Default::default(), NZU32!(5));
        let ns = seed_namespace(b"fluent-devnet");

        let recover_at = |h: u64| {
            let partials: Vec<_> = shares
                .iter()
                .map(|s| sign_seed_partial(s, &ns, h))
                .collect();
            recover_seed(&sharing, &partials, h).expect("recover")
        };

        let r10 = prev_randao_from_seed(&recover_at(10));
        let r10_again = prev_randao_from_seed(&recover_at(10));
        let r11 = prev_randao_from_seed(&recover_at(11));

        assert_eq!(r10, r10_again, "same height → identical randomness");
        assert_ne!(r10, r11, "different heights → different randomness");
    }

    #[test]
    fn recover_below_threshold_fails() {
        let mut rng = test_rng();
        let (sharing, shares) =
            deal_anonymous::<MinSig, N3f1>(&mut rng, Default::default(), NZU32!(5));
        let ns = seed_namespace(b"fluent-devnet");
        let height = 7u64;
        // quorum for n=5 under 3f+1 is 4; two partials must be insufficient.
        let partials: Vec<_> = shares
            .iter()
            .take(2)
            .map(|s| sign_seed_partial(s, &ns, height))
            .collect();
        assert!(recover_seed(&sharing, &partials, height).is_err());
    }

    #[test]
    fn prev_randao_fallback_is_gated() {
        let mut rng = test_rng();
        let (sharing, shares) =
            deal_anonymous::<MinSig, N3f1>(&mut rng, Default::default(), NZU32!(5));
        let ns = seed_namespace(b"fluent-devnet");
        let height = 42u64;
        let fallback = B256::repeat_byte(0xAB);
        let partials: Vec<_> = shares
            .iter()
            .map(|s| sign_seed_partial(s, &ns, height))
            .collect();
        let seed = recover_seed(&sharing, &partials, height).expect("recover");
        let pk = sharing.public();

        // valid seed + PK for this height → threshold randomness, assurance true.
        let (r, a) = prev_randao_for_height(height, Some(&seed), Some(pk), &ns, fallback);
        assert!(a);
        assert_eq!(r, prev_randao_from_seed(&seed));
        assert_ne!(r, fallback);

        // every failure mode falls back deterministically, assurance false.
        for (h, s, p, n) in [
            (height, Some(&seed), None, &ns),         // no PK_epoch yet
            (height, None, Some(pk), &ns),            // seed not recovered
            (height + 1, Some(&seed), Some(pk), &ns), // seed is for another height
            (height, Some(&seed), Some(pk), &seed_namespace(b"other")), // invalid vs PK
        ] {
            let (r, a) = prev_randao_for_height(h, s, p, n, fallback);
            assert!(!a, "must not claim assurance on the fallback path");
            assert_eq!(r, fallback, "fallback must be the weak deterministic value");
        }
    }
}
