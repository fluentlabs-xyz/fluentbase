//! Deriver-side beacon helpers: decode the epoch threshold material, and turn a
//! recovered seed into the EVM `prev_randao` (gated against `PK_epoch`).
//!
//! The seed CRYPTO (round-keyed sign / recover / verify-partial) lives in
//! [`fluentbase_bls::beacon`] — the combined consensus scheme recovers the seed
//! from the notarization/finalization certificate. This module defines the
//! per-height [`Seed`] wire type, the EVM-facing pieces (it owns the alloy
//! types) and the key loaders.

use alloy_primitives::{keccak256, B256};
use bytes::{Buf, BufMut};
use commonware_codec::{Encode as _, EncodeSize, Read, ReadExt as _, Write};
use commonware_consensus::types::Round;
use commonware_cryptography::{bls12381::primitives::group::Share, Hasher as _, Sha256};
use fluentbase_bls::BlsSignature;
use fluentbase_staking_reader::reader::ValidatorSetSnapshot;

/// The per-round threshold randomness seed: the recovered BLS threshold
/// signature over `(seed_namespace ‖ round)`, unique by construction (any ≥t
/// partials recover the same value). `prev_randao = H(signature)`. It is
/// recovered from the notarization/finalization certificate of `target_round`
/// (the combined consensus scheme); the deriver pairs it with the round it read
/// from that certificate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seed {
    pub target_round: Round,
    pub signature: BlsSignature,
}

// Wire: target_round ‖ signature — both fixed-size, no length prefix.
impl Write for Seed {
    fn write(&self, buf: &mut impl BufMut) {
        self.target_round.write(buf);
        self.signature.write(buf);
    }
}

impl EncodeSize for Seed {
    fn encode_size(&self) -> usize {
        self.target_round.encode_size() + self.signature.encode_size()
    }
}

impl Read for Seed {
    type Cfg = ();

    fn read_cfg(buf: &mut impl Buf, _: &Self::Cfg) -> Result<Self, commonware_codec::Error> {
        let target_round = Round::read(buf)?;
        let signature = BlsSignature::read(buf)?;
        Ok(Self {
            target_round,
            signature,
        })
    }
}

/// Decode a single DKG `Share` from its encoded bytes (a node's loaded
/// `beacon-share.hex`). Rejects trailing bytes.
pub(crate) fn parse_share(bytes: &[u8]) -> Result<Share, commonware_codec::Error> {
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

// ── The seedless arm's base ───────────────────────────────────────────────────
//
// The leader elector falls back to these where a round's certificate carries no
// threshold seed (view 1 of an epoch, nullify-justified views). They live HERE,
// not beside the elector, because choosing what stands in for randomness is a
// randomness decision: an implementation that computes the seed from a hash
// substitutes its own base with it, and the consensus core reads both only as
// opaque 32-byte values.

/// Last-resort base for the seedless arm: `sha256(epoch_be ‖ sorted peer
/// pubkeys)`, derivable from constants and therefore predictable an epoch ahead.
/// Reached only where no witness seed can exist — epoch 0, a non-computable
/// terminal height, and pre-bootstrap links whose terminal block legitimately
/// carries no witness. Sorting the peers makes it invariant under any snapshot
/// iteration order, so honest nodes that observe the epoch's keys in any order
/// derive the identical base.
pub fn constant_fallback_seed(snap: &ValidatorSetSnapshot) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(&snap.epoch.to_be_bytes());
    let mut peers: Vec<&[u8]> = snap
        .validators
        .iter()
        .map(|v| v.keys.peer_pubkey.as_ref())
        .collect();
    peers.sort_unstable();
    for p in peers {
        h.update(p);
    }
    <[u8; 32]>::try_from(h.finalize().as_ref()).expect("sha256 is 32 bytes")
}

/// Compress the previous epoch's terminal-round seed into the seedless arm's base.
/// Deliberately NOT [`prev_randao_from_seed`]: that value is a header field, and
/// D6 requires the leader draw to stay disjoint from it.
///
/// The name still says "witness" for the σ it once read off the terminal block's
/// `parent_seed`; since FLU-1204 the same σ comes from the store pin at that
/// block's own round. Same value, same round — a different way of holding it.
pub fn witness_fallback_seed(seed: &Seed) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(seed.signature.encode().as_ref());
    <[u8; 32]>::try_from(h.finalize().as_ref()).expect("sha256 is 32 bytes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_consensus::types::{Epoch, View};
    use commonware_cryptography::bls12381::{dkg::deal_anonymous, primitives::variant::MinSig};
    use commonware_utils::{test_rng, N3f1, NZU32};
    use fluentbase_bls::beacon::{recover_seed, seed_namespace, sign_seed_partial};

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
}
