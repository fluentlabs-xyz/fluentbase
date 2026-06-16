//! Wire types for the per-epoch threshold randomness beacon.
//!
//! Two artifacts ride in agreed [`crate::order_block::OrderBlock`] data: the
//! per-epoch DKG outcome (boundary blocks, carried as opaque bytes — the
//! encoded commonware `Output`, whose `Read::Cfg` needs the committee size so
//! it is parsed by the beacon module, not the block codec — mirroring how
//! `extra_data` carries the attestation bitmap) and the per-height [`Seed`].

use bytes::{Buf, BufMut};
use commonware_codec::{EncodeSize, Read, ReadExt as _, Write};
use fluentbase_bls::BlsSignature;

/// Decode cap for an embedded DKG outcome (the encoded commonware `Output`
/// for a committee ≤ `MAX_COMMITTEE_SIZE`: a MinSig public polynomial of
/// degree `quorum-1` in G2 plus the dealer/player/revealed sets). 64 KiB is
/// generous headroom over the ~5 KiB worst case at n=51.
pub const MAX_BEACON_OUTCOME_SIZE: usize = 64 * 1024;

/// The per-height threshold randomness seed: a recovered BLS threshold
/// signature over `(seed_namespace ‖ target_height)`, unique by construction
/// (any ≥t partials recover the same value). `prev_randao(target_height) =
/// H(signature)`. It carries `target_height` because the seed is signed only
/// AFTER that height finalizes and embedded by a later proposer within the
/// `(h, h+K]` window — it cannot live on its own height's block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seed {
    pub target_height: u64,
    pub signature: BlsSignature,
}

impl Write for Seed {
    fn write(&self, buf: &mut impl BufMut) {
        self.target_height.write(buf);
        self.signature.write(buf);
    }
}

impl EncodeSize for Seed {
    fn encode_size(&self) -> usize {
        self.target_height.encode_size() + self.signature.encode_size()
    }
}

impl Read for Seed {
    type Cfg = ();

    fn read_cfg(buf: &mut impl Buf, _: &Self::Cfg) -> Result<Self, commonware_codec::Error> {
        let target_height = u64::read(buf)?;
        let signature = BlsSignature::read(buf)?;
        Ok(Self {
            target_height,
            signature,
        })
    }
}
