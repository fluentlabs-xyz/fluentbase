//! Deriver-side beacon helpers: decode the epoch threshold material, and turn a
//! recovered seed into the EVM `prev_randao` (gated against `PK_epoch`).
//!
//! The seed CRYPTO (round-keyed sign / recover / verify-partial) lives in
//! [`fluentbase_bls::beacon`] — the combined consensus scheme recovers the seed
//! from the notarization/finalization certificate. This module only adds the
//! EVM-facing pieces (it owns the alloy types) and the key loaders, and
//! re-exports the verifier the deriver needs.

use crate::beacon::types::Seed;
use alloy_primitives::{keccak256, B256};
use commonware_codec::{Encode as _, Read as _};
use commonware_consensus::types::Round;
use commonware_cryptography::bls12381::primitives::{
    group::Share,
    sharing::{ModeVersion, Sharing},
    variant::MinSig,
};
use core::num::NonZeroU32;
use fluentbase_p2p::constants::MAX_COMMITTEE_SIZE;

pub use fluentbase_bls::beacon::{seed_namespace, verify_seed, GroupPublic};

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

/// Decide `prev_randao` for the EVM block whose ordering round is `round`, with
/// the beacon-failure fallback GATED so it can never halt the chain (G1).
///
/// TODO(beacon-k-lag): this consumes `seed(round)` for block at
/// `round`, which is grindable via abort-to-next-view (non-slashable Nullify
/// re-rolls the round → a fresh seed). The fix is to feed `seed(round − K)`
/// here instead; see the `crate::beacon` module doc for the full rationale and
/// the multi-repo (node + STF guest) scope. Deferred — must be symmetric.
///
/// Returns `(prev_randao, assurance)`. Threshold randomness is used ONLY when a
/// seed for exactly this round is present, `PK_epoch` is known, and the seed
/// verifies against it — then `(keccak256(seed), true)`. Every other case
/// (no `PK_epoch`, no seed, or a claimed-but-invalid seed) degrades to the
/// deterministic weak `fallback` with `assurance = false`.
pub fn prev_randao_for_round(
    round: Round,
    seed: Option<&Seed>,
    pk_epoch: Option<&GroupPublic>,
    namespace: &[u8],
    fallback: B256,
) -> (B256, bool) {
    match (seed, pk_epoch) {
        (Some(s), Some(pk))
            if s.target_round == round && verify_seed(pk, namespace, round, &s.signature) =>
        {
            (prev_randao_from_seed(s), true)
        }
        _ => (fallback, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_consensus::types::{Epoch, View};
    use commonware_cryptography::bls12381::{dkg::deal_anonymous, primitives::variant::MinSig};
    use commonware_utils::{test_rng, N3f1, NZU32};
    use fluentbase_bls::beacon::{recover_seed, sign_seed_partial};

    fn recover_at(round: Round) -> Seed {
        let mut rng = test_rng();
        let (sharing, shares) =
            deal_anonymous::<MinSig, N3f1>(&mut rng, Default::default(), NZU32!(5));
        let ns = seed_namespace(b"fluent-devnet");
        let partials: Vec<_> = shares
            .iter()
            .map(|s| sign_seed_partial(s, &ns, round))
            .collect();
        Seed {
            target_round: round,
            signature: recover_seed::<N3f1>(&sharing, &partials).expect("recover"),
        }
    }

    #[test]
    fn prev_randao_is_round_unique_and_deterministic() {
        let r10 = Round::new(Epoch::new(1), View::new(10));
        let r11 = Round::new(Epoch::new(1), View::new(11));
        assert_eq!(
            prev_randao_from_seed(&recover_at(r10)),
            prev_randao_from_seed(&recover_at(r10)),
            "same round → identical randomness"
        );
        assert_ne!(
            prev_randao_from_seed(&recover_at(r10)),
            prev_randao_from_seed(&recover_at(r11)),
            "different rounds → different randomness"
        );
    }

    #[test]
    fn prev_randao_fallback_is_gated() {
        let mut rng = test_rng();
        let (sharing, shares) =
            deal_anonymous::<MinSig, N3f1>(&mut rng, Default::default(), NZU32!(5));
        let ns = seed_namespace(b"fluent-devnet");
        let round = Round::new(Epoch::new(2), View::new(42));
        let partials: Vec<_> = shares
            .iter()
            .map(|s| sign_seed_partial(s, &ns, round))
            .collect();
        let seed = Seed {
            target_round: round,
            signature: recover_seed::<N3f1>(&sharing, &partials).expect("recover"),
        };
        let pk = sharing.public();
        let fallback = B256::repeat_byte(0xAB);

        let (r, a) = prev_randao_for_round(round, Some(&seed), Some(pk), &ns, fallback);
        assert!(a);
        assert_eq!(r, prev_randao_from_seed(&seed));

        let other_round = Round::new(Epoch::new(2), View::new(43));
        for (rd, s, p) in [
            (round, Some(&seed), None),           // no PK_epoch
            (round, None, Some(pk)),              // no seed
            (other_round, Some(&seed), Some(pk)), // seed is for another round
        ] {
            let (r, a) = prev_randao_for_round(rd, s, p, &ns, fallback);
            assert!(!a, "must not claim assurance on the fallback path");
            assert_eq!(r, fallback);
        }
    }
}
