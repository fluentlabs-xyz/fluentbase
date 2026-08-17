//! Canonical byte format for the production record embedded in
//! `block.header.extra_data`.
//!
//! Wire format: `[version: u8][leader_index: u8][accused: u8]` — exactly 3 bytes.
//!
//! It records WHO PRODUCED the block and, optionally, WHO THIS BLOCK CONVICTS.
//! Every honest voter checks the carried producer index against the
//! consensus-supplied `Context.leader` (`application::structural_checks`) and the
//! accusation against the evidence in `OrderBlock::equivocation`
//! (`application::equivocation_gate_decision`), so forging either needs the full
//! `n − f` quorum rather than one Byzantine proposer.
//!
//! The accusation rides HERE, and not in the OrderBlock alongside its evidence,
//! because a node syncing the EL from peers re-executes every downloaded block
//! from the header alone — it has no OrderBlock. `extra_data` is copied verbatim
//! into the derived header (`derive.rs`), so the pre-execution system call sees
//! the same verdict on that path as on the consensus path.
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
use fluentbase_p2p::constants::MAX_COMMITTEE_SIZE;

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
const PR_ACCUSED_OFFSET: usize = 2;
/// Total encoded length. EXACT — not a minimum. See [`decode_production_record`].
pub const PRODUCTION_RECORD_LEN: usize = PR_ACCUSED_OFFSET + size_of::<u8>();

/// `accused` value meaning "this block convicts nobody". A sentinel rather than
/// a second record length, so the exact-length decode — this format's best
/// property — survives; safe because a committee index is always below
/// [`MAX_COMMITTEE_SIZE`] (51).
pub const NO_CHARGE: u8 = 0xFF;

/// Decoded production record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProductionRecord {
    /// Position of the block's producer in the epoch committee's BiMap.
    pub leader_index: u8,
    /// Position, in the SAME BiMap, of the member this block convicts of
    /// equivocation — `None` when it convicts nobody. `Some` iff the block
    /// carries the backing evidence in `OrderBlock::equivocation`.
    pub accused: Option<u8>,
}

/// Encode the production record for a block produced by committee member
/// `leader_index`, optionally convicting member `accused`.
pub fn encode_production_record(leader_index: u8, accused: Option<u8>) -> Vec<u8> {
    vec![
        PRODUCTION_RECORD_VERSION,
        leader_index,
        accused.unwrap_or(NO_CHARGE),
    ]
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
/// - Any length other than 0 or [`PRODUCTION_RECORD_LEN`] → `Err(WrongLength)`.
///   Exact, because the OrderBlock codec tolerates 4 KiB of `extra_data` while
///   the reth header caps it at `FLUENT_MAXIMUM_EXTRA_DATA_SIZE`: without an
///   exact-length vote rule an over-length field could finalize a block no
///   devp2p node can execute.
/// - Unknown version → `Err(UnknownVersion)`, fail-closed.
/// - An `accused` byte that is neither [`NO_CHARGE`] nor a legal committee
///   position → `Err(AccusedOutOfRange)`. The system call downstream resolves
///   the index against the epoch's committee array, so an out-of-range byte
///   could only ever revert there.
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
    let accused = match buf[PR_ACCUSED_OFFSET] {
        NO_CHARGE => None,
        idx if (idx as usize) < MAX_COMMITTEE_SIZE as usize => Some(idx),
        idx => return Err(ProductionRecordError::AccusedOutOfRange { got: idx }),
    };
    Ok(Some(ProductionRecord {
        leader_index: buf[PR_LEADER_OFFSET],
        accused,
    }))
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProductionRecordError {
    #[error("production record must be exactly {PRODUCTION_RECORD_LEN} bytes, got {got}")]
    WrongLength { got: usize },
    #[error("unknown production record version {got} (expected {PRODUCTION_RECORD_VERSION})")]
    UnknownVersion { got: u8 },
    #[error("accused index {got} is not a committee position (< {MAX_COMMITTEE_SIZE}) nor {NO_CHARGE:#04x}")]
    AccusedOutOfRange { got: u8 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_record_roundtrips() {
        for idx in [0u8, 1, 25, 50, 255] {
            for accused in [None, Some(0u8), Some(50)] {
                let buf = encode_production_record(idx, accused);
                assert_eq!(buf.len(), PRODUCTION_RECORD_LEN);
                let r = decode_production_record(&buf).unwrap().unwrap();
                assert_eq!(r.leader_index, idx);
                assert_eq!(r.accused, accused);
            }
        }
    }

    /// Hex-pinned: the on-chain `recordProduction` decoder reads these bytes and
    /// the equivocation system call reads the third, so a silent layout change
    /// here mis-credits production — or convicts the wrong member — with no
    /// other symptom.
    #[test]
    fn production_record_hex_pinned_fixture() {
        // version = 0x01, leader_index = 50 = 0x32, accused = none = 0xFF.
        assert_eq!(
            encode_production_record(50, None),
            hex::decode("0132ff").unwrap()
        );
        // Same block, convicting committee position 7.
        assert_eq!(
            encode_production_record(50, Some(7)),
            hex::decode("013207").unwrap()
        );
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
        // 2 and 4 bracket the exact length; 24 is the retired bitmap's encoded
        // width and 32 the reth header cap, i.e. the largest field that still
        // reaches an EVM header intact.
        for len in [1usize, 2, 4, 24, 32] {
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
            let buf = vec![version, 7, NO_CHARGE];
            assert_eq!(
                decode_production_record(&buf).unwrap_err(),
                ProductionRecordError::UnknownVersion { got: version },
                "version {version} must reject"
            );
        }
    }

    #[test]
    fn production_record_rejects_an_accused_outside_the_committee() {
        // The sentinel sits at 0xFF and every legal position below
        // MAX_COMMITTEE_SIZE; everything between is a byte the system call could
        // only ever fail to resolve against the epoch's committee array.
        for idx in [MAX_COMMITTEE_SIZE as u8, MAX_COMMITTEE_SIZE as u8 + 1, 254] {
            let buf = vec![PRODUCTION_RECORD_VERSION, 3, idx];
            assert_eq!(
                decode_production_record(&buf).unwrap_err(),
                ProductionRecordError::AccusedOutOfRange { got: idx },
                "accused {idx} must reject"
            );
        }
        // The bound is exclusive: the last legal position still decodes.
        let last = MAX_COMMITTEE_SIZE as u8 - 1;
        let buf = vec![PRODUCTION_RECORD_VERSION, 3, last];
        assert_eq!(
            decode_production_record(&buf).unwrap().unwrap().accused,
            Some(last)
        );
    }
}
