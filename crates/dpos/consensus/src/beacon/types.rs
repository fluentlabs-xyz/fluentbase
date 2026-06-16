//! Wire types for the per-epoch threshold randomness beacon.
//!
//! Two artifacts ride in agreed [`crate::order_block::OrderBlock`] data: the
//! per-epoch DKG outcome (boundary blocks, carried as opaque bytes — the
//! encoded commonware `Output`, whose `Read::Cfg` needs the committee size so
//! it is parsed by the beacon module, not the block codec — mirroring how
//! `extra_data` carries the attestation bitmap) and the per-height [`Seed`].

use bytes::{Buf, BufMut};
use commonware_codec::{EncodeSize, Read, ReadExt as _, Write};
use commonware_consensus::types::Round;
use fluentbase_bls::BlsSignature;

/// Decode cap for an embedded DKG outcome (the encoded commonware `Output`
/// for a committee ≤ `MAX_COMMITTEE_SIZE`: a MinSig public polynomial of
/// degree `quorum-1` in G2 plus the dealer/player/revealed sets). 64 KiB is
/// generous headroom over the ~5 KiB worst case at n=51.
pub const MAX_BEACON_OUTCOME_SIZE: usize = 64 * 1024;

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
