//! Decode + inspect the per-epoch DKG outcome embedded in a boundary
//! `OrderBlock` (`beacon_outcome`). The aggregated commonware [`Output`]
//! (group key `PK_epoch` + public polynomial + dealer/player sets) is stored as
//! opaque bytes at the block-codec layer because `Output`'s decode needs the
//! committee-size config; this module supplies that config and extracts the
//! group public key the seed sub-protocol verifies against and the system call
//! publishes to L2.

use commonware_codec::{Encode as _, Read as _};
use commonware_cryptography::bls12381::{dkg::Output, primitives::sharing::ModeVersion};
use core::num::NonZeroU32;
use fluentbase_bls::PeerPubkey;
use fluentbase_p2p::constants::MAX_COMMITTEE_SIZE;

use crate::beacon::seed::GroupPublic;

/// The DKG outcome for our committee: MinSig keys, participants identified by
/// their ed25519 peer pubkey (the commonware participant-ordering key).
pub type DkgOutcome =
    Output<commonware_cryptography::bls12381::primitives::variant::MinSig, PeerPubkey>;

/// Errors decoding an embedded outcome — any of these means the boundary block
/// does not carry a well-formed agreed beacon key.
#[derive(Debug)]
pub enum OutcomeError {
    /// Bytes are not a valid encoded `Output` for a committee ≤ MAX_COMMITTEE_SIZE.
    Decode(commonware_codec::Error),
    /// Trailing bytes after the outcome (a well-formed `Output` consumes all).
    TrailingBytes,
}

/// Decode the embedded `beacon_outcome` bytes into the typed DKG [`DkgOutcome`],
/// bounding the committee to `MAX_COMMITTEE_SIZE` (NonZeroCounter mode, v0).
pub fn parse_outcome(bytes: &[u8]) -> Result<DkgOutcome, OutcomeError> {
    let max = NonZeroU32::new(MAX_COMMITTEE_SIZE as u32).expect("MAX_COMMITTEE_SIZE > 0");
    let mut buf = bytes;
    let outcome =
        DkgOutcome::read_cfg(&mut buf, &(max, ModeVersion::v0())).map_err(OutcomeError::Decode)?;
    if !buf.is_empty() {
        return Err(OutcomeError::TrailingBytes);
    }
    Ok(outcome)
}

/// Encode a DKG outcome to the opaque bytes carried in `OrderBlock.beacon_outcome`.
pub fn encode_outcome(outcome: &DkgOutcome) -> Vec<u8> {
    outcome.encode().to_vec()
}

/// The group public key `PK_epoch` — what seeds verify against and what the
/// system call commits to L2.
pub fn group_public_key(outcome: &DkgOutcome) -> &GroupPublic {
    outcome.public().public()
}

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_cryptography::{
        bls12381::{
            dkg::deal,
            primitives::{sharing::Mode, variant::MinSig},
        },
        ed25519::PrivateKey as Ed25519PrivateKey,
        Signer as _,
    };
    use commonware_math::algebra::Random as _;
    use commonware_utils::{ordered::Set, N3f1};
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng as _;

    fn deal_outcome(n: u32) -> DkgOutcome {
        let mut rng = StdRng::seed_from_u64(7);
        let players: Set<PeerPubkey> =
            Set::from_iter_dedup((0..n).map(|_| Ed25519PrivateKey::random(&mut rng).public_key()));
        let (outcome, _shares) =
            deal::<MinSig, PeerPubkey, N3f1>(&mut rng, Mode::NonZeroCounter, players)
                .expect("deal");
        outcome
    }

    #[test]
    fn outcome_encode_parse_roundtrip_and_group_key() {
        let outcome = deal_outcome(5);
        let bytes = encode_outcome(&outcome);
        let parsed = parse_outcome(&bytes).expect("parse");
        assert_eq!(parsed, outcome, "embedded outcome must round-trip exactly");
        assert_eq!(
            group_public_key(&parsed),
            group_public_key(&outcome),
            "every node derives the same PK_epoch from the embedded outcome"
        );
    }

    #[test]
    fn truncated_outcome_is_rejected() {
        let outcome = deal_outcome(5);
        let bytes = encode_outcome(&outcome);
        assert!(parse_outcome(&bytes[..bytes.len() - 1]).is_err());
    }

    #[test]
    fn trailing_bytes_are_rejected() {
        let outcome = deal_outcome(5);
        let mut bytes = encode_outcome(&outcome);
        bytes.push(0xFF);
        assert!(matches!(
            parse_outcome(&bytes),
            Err(OutcomeError::TrailingBytes)
        ));
    }
}
