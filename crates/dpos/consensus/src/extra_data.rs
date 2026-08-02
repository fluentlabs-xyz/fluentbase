//! Canonical byte format for the production record embedded in
//! `block.header.extra_data`.
//!
//! Wire format: `[version: u8][leader_index: u8]` — exactly 2 bytes.
//!
//! It records WHO PRODUCED the block. Every honest voter checks the carried
//! index against the consensus-supplied `Context.leader`
//! (`application::structural_checks`), so forging it needs the full `n − f`
//! quorum rather than one Byzantine proposer.
//!
//! Cross-language parity:
//! - Solidity consumer: `ProductionLiveness.recordProduction(blockNumber,
//!   leaderIndex)`, called once per block by the executor.
//! - `leader_index` is `u8` here AND in Solidity. Both Rust
//!   `fluentbase_p2p::constants::MAX_COMMITTEE_SIZE` and Solidity
//!   `ChainConfig.MAX_ACTIVE_VALIDATORS` cap at 51. Bumping either past 255
//!   requires widening this wire format; the startup assert in
//!   `OuterBuilder::build` catches the config mistake before any block is
//!   proposed.
//! - Pinned by this module's hex-fixture unit tests.

use core::mem::size_of;

// The index→member binding is the epoch's committee BiMap, which is also the
// order the on-chain `committee[E]` array is verified against on every commit
// (strictly ascending by `peerPubkey`). That shared order is the ONE structural
// mirror this design keeps, and a divergence in it would not fail verify — it
// would silently mis-credit production. It is tested contract-side, not here.

/// Current production-record wire version. A bump is a consensus format change
/// (fresh genesis or a coordinated fork), never a rolling upgrade: honest voters
/// reject an unknown version, so a node emitting v2 into a v1 committee has its
/// proposals nullified.
pub const PRODUCTION_RECORD_VERSION: u8 = 1;

/// Byte offsets within the record.
const PR_VERSION_OFFSET: usize = 0;
const PR_LEADER_OFFSET: usize = 1;
/// Total encoded length. EXACT — not a minimum. See [`decode_production_record`].
pub const PRODUCTION_RECORD_LEN: usize = PR_LEADER_OFFSET + size_of::<u8>();

/// Decoded production record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProductionRecord {
    /// Position of the block's producer in the epoch committee's BiMap.
    pub leader_index: u8,
}

/// Encode the production record for a block produced by committee member
/// `leader_index`.
pub fn encode_production_record(leader_index: u8) -> Vec<u8> {
    vec![PRODUCTION_RECORD_VERSION, leader_index]
}

/// Decode the production record.
///
/// - Empty input → `Ok(None)`. This arm exists for the EXECUTOR, whose decode
///   gate keys on HEIGHT (`block_number >= dposActivationBlock`) rather than on
///   "was this produced by DPoS consensus", and the two are NOT the same
///   predicate for exactly one block. `launcher.rs`'s sequencer halts only once
///   its head has REACHED activation, so the block AT `block_number ==
///   activation` is built by the reth payload builder — which force-empties
///   `extra_data` under `dpos_active` (`payload.rs`) — and then decoded here
///   because the gate is `>=`. Fail-loud-decoding it would be a deterministic,
///   every-node, unrecoverable execution failure at the swap block of every
///   bring-up. It is NOT an arm the vote-time rule may take: an empty field must
///   REJECT at verify, which is what makes the empty case unreachable through
///   consensus in the first place. Callers on the vote path must require
///   `Some`, never merely `is_ok()`.
/// - Any length other than 0 or 2 → `Err(WrongLength)`. Exact, because the
///   OrderBlock codec tolerates 4 KiB of `extra_data` while the reth header caps
///   it at 32 bytes: without an exact-length vote rule an over-length field
///   could finalize a block no devp2p node can execute.
/// - Unknown version → `Err(UnknownVersion)`, fail-closed.
pub fn decode_production_record(
    buf: &[u8],
) -> Result<Option<ProductionRecord>, ProductionRecordError> {
    if buf.is_empty() {
        return Ok(None);
    }
    if buf.len() != PRODUCTION_RECORD_LEN {
        return Err(ProductionRecordError::WrongLength { got: buf.len() });
    }
    let version = buf[PR_VERSION_OFFSET];
    if version != PRODUCTION_RECORD_VERSION {
        return Err(ProductionRecordError::UnknownVersion { got: version });
    }
    Ok(Some(ProductionRecord {
        leader_index: buf[PR_LEADER_OFFSET],
    }))
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProductionRecordError {
    #[error("production record must be exactly {PRODUCTION_RECORD_LEN} bytes, got {got}")]
    WrongLength { got: usize },
    #[error("unknown production record version {got} (expected {PRODUCTION_RECORD_VERSION})")]
    UnknownVersion { got: u8 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_record_roundtrips() {
        for idx in [0u8, 1, 25, 50, 255] {
            let buf = encode_production_record(idx);
            assert_eq!(buf.len(), PRODUCTION_RECORD_LEN);
            let r = decode_production_record(&buf).unwrap().unwrap();
            assert_eq!(r.leader_index, idx);
        }
    }

    /// Hex-pinned: the on-chain `recordProduction` decoder reads these two
    /// bytes, so a silent layout change here mis-credits production with no
    /// other symptom.
    #[test]
    fn production_record_hex_pinned_fixture() {
        // version = 0x01, leader_index = 50 = 0x32.
        assert_eq!(encode_production_record(50), hex::decode("0132").unwrap());
    }

    /// The executor's `len == 0` ⇒ skip arm. Not reachable through consensus —
    /// honest voters reject an empty field — but reachable by HEIGHT for the one
    /// block the pre-DPoS sequencer produces at `block_number == activation`
    /// (`launcher.rs` halts only once its head has REACHED activation, and the
    /// executor decodes from `>= activation`).
    #[test]
    fn production_record_empty_decodes_to_none() {
        assert_eq!(decode_production_record(&[]).unwrap(), None);
    }

    #[test]
    fn production_record_rejects_wrong_length() {
        // 1 and 3 bracket the exact length; 24 is the retired bitmap's encoded
        // width and 32 the reth header cap, i.e. the largest field that still
        // reaches an EVM header intact.
        for len in [1usize, 3, 24, 32] {
            let buf = vec![PRODUCTION_RECORD_VERSION; len];
            assert_eq!(
                decode_production_record(&buf).unwrap_err(),
                ProductionRecordError::WrongLength { got: len },
                "length {len} must reject"
            );
        }
    }

    #[test]
    fn production_record_rejects_unknown_version() {
        // Length is correct; only the version byte is wrong. Fail-closed, so a
        // v2 record can never be silently read as v1 with a shifted field.
        for version in [0u8, 2, 255] {
            let buf = vec![version, 7];
            assert_eq!(
                decode_production_record(&buf).unwrap_err(),
                ProductionRecordError::UnknownVersion { got: version },
                "version {version} must reject"
            );
        }
    }
}
