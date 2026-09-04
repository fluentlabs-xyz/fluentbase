//! The F-type consensus unit: ordering only — txs + parent digest + result
//! commitment. The digest deliberately excludes every execution output of
//! THIS block; `result` commits the derived block hash K heights back, so
//! agreeing OrderBlock N+K is the committee's attestation of block N's
//! execution result.

use crate::digest::Digest;
use crate::slasher::evidence::MAX_EQUIVOCATION_SIZE;
use alloy_primitives::{keccak256, Address, Bytes, B256};
use bytes::{Buf, BufMut};
use commonware_codec::{Encode as _, EncodeSize, FixedSize, Read, Write};
use commonware_consensus::{types::Height, Heightable};
use commonware_cryptography::{Committable, Digestible};
use reth_ethereum_primitives::TransactionSigned;
use reth_primitives_traits::SealedBlock;

/// Result lag in blocks (D=3). Consensus-critical: MUST be
/// byte-identical across nodes (same class as MAX_MESSAGE_SIZE, G11).
/// Changing it is a chain-spec release, not a config knob.
pub const K: u64 = 3;

/// EIP-1559 hard floor for a header gas limit. Consensus-uniform (propose clamps
/// to it, verify's ±1/1024 rule rejects below it); homed here so the
/// byte-identical anchor constructor ([`anchor_order_block`]) owns its own floor.
pub const MIN_GAS_LIMIT: u64 = 5_000;

/// Per-artifact decode cap (defense-in-depth + channel-specific bound) —
/// same wire budget as the executed-block era: 50M gas / 16 B-per-calldata
/// ≈ 3.125 MB worst case + ~30% headroom. Coupled to but independent from
/// `fluentbase_p2p::constants::MAX_MESSAGE_SIZE`.
pub const MAX_ORDER_BLOCK_SIZE: usize = 4 * 1024 * 1024;

/// Tx-list byte budget for ordering assembly: [`MAX_ORDER_BLOCK_SIZE`] minus the
/// [`MAX_EXTRA_DATA_SIZE`] allowance for the non-tx fields (parent/height/result,
/// extra_data, codec framing), [`PROPOSAL_VIEW_FRAMING`] for the always-present
/// `proposal_view`, and [`EQUIVOCATION_FRAMING`] for the one optional field a
/// block may still carry — so an assembled artifact always fits its own decode
/// cap.
///
/// This grew by 68 bytes (`2 * MAX_U64_VARINT_SIZE + BlsSignature::SIZE`) when
/// `parent_seed` left the wire: the allowance existed only to keep a full-budget
/// block carrying that field under the cap, and there is no such field to carry.
/// The composition argument is unchanged — the sole consumer is the proposer's
/// `assemble` call, and `MAX_ORDER_BLOCK_SIZE` (the decode cap every verifier
/// enforces) did not move.
pub const TX_BYTE_BUDGET: usize =
    MAX_ORDER_BLOCK_SIZE - MAX_EXTRA_DATA_SIZE - PROPOSAL_VIEW_FRAMING - EQUIVOCATION_FRAMING;

/// Worst-case wire cost of the `equivocation` field: its decode cap plus the
/// `u32` length prefix. Reserved out of [`TX_BYTE_BUDGET`] for EVERY block — a
/// charge can ride any block whose proposer holds one — so a full-budget block
/// carrying one still fits `MAX_ORDER_BLOCK_SIZE` / the byte-identical p2p frame
/// cap (the composed-oversize class, bug 1).
pub const EQUIVOCATION_FRAMING: usize = MAX_EQUIVOCATION_SIZE + u32::SIZE;

/// Wire cost of `proposal_view` — a fixed `u64` on every block. Reserved out of
/// [`TX_BYTE_BUDGET`] so a full-budget block still fits `MAX_ORDER_BLOCK_SIZE` /
/// the byte-identical p2p frame cap — the same composed-oversize class (bug 1)
/// [`EQUIVOCATION_FRAMING`] documents. Not folded into the
/// [`MAX_EXTRA_DATA_SIZE`] lump because it was carved out separately when the
/// witness arrived and the field it shared that carve-out with has since left.
pub const PROPOSAL_VIEW_FRAMING: usize = u64::SIZE;

/// Decode cap for `extra_data`. Deliberately far above the 3-byte production
/// record it actually carries: this cap only has to compose with the
/// TX_BYTE_BUDGET allowance. The BINDING bound is the vote-time exact-length
/// rule in `application::structural_checks`, without which an over-length field
/// could finalize here and then be unexecutable at the reth header cap
/// (`FLUENT_MAXIMUM_EXTRA_DATA_SIZE`).
const MAX_EXTRA_DATA_SIZE: usize = 4 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderBlock {
    /// Digest of OrderBlock N−1 (the ordering chain, NOT the EVM parent hash).
    pub parent: Digest,
    pub height: u64,
    /// The simplex VIEW this block was proposed in — self-attested by the
    /// proposer and (rule SA) checked at THIS block's own vote time against
    /// `ctx.round.view()`, so for every certified block it is the TRUE
    /// proposal view, sealed under the committee multisig via `digest()`.
    /// That makes the round in which the PARENT was certified a pure function
    /// of agreed data (`Round::new(parent_epoch, parent.proposal_view)`) —
    /// including at the epoch boundary, where `ctx.parent` is the
    /// GENESIS_VIEW sentinel and the certified round is otherwise
    /// per-node/first-wins. Non-optional by design: an `Option` would
    /// re-introduce a downgrade arm. The anchor block is never proposed (it
    /// has no round) and writes `0` — a value, not a sentinel; nothing may
    /// branch on it. No ingress path may recompute or re-validate this field
    /// (it is inside the digest; SA is a vote-time-only obligation).
    /// Written by `build_proposal` from `ctx.round.view()`; enforced by rule
    /// SA in `structural_checks` at the block's own vote; consumed by the
    /// verify gate's PIN and by the executor's speculative-round
    /// re-canonicalisation.
    pub proposal_view: u64,
    /// Proposer-chosen; becomes the derived header timestamp. `verify`
    /// (`structural_checks`) gates it on TWO bounds: strictly monotonic vs the
    /// parent AND `<= local_now + TIMESTAMP_FUTURE_TOLERANCE_SECS` — the latter
    /// a VOTE-time wall-clock upper bound (the verifier's own clock) that
    /// rejects future-dated blocks and defeats the time-ratchet attack. The
    /// wall-clock bound gates the VOTE only; the state transition copies this
    /// verbatim (`derive.rs`), so STF determinism is unaffected.
    pub timestamp: u64,
    /// Proposer's fee recipient — derived header beneficiary.
    pub fee_recipient: Address,
    /// Derived block gas limit, as AGREED in the artifact: derivation and
    /// `verify` read THIS field — never a node's local config — so every node
    /// derives an identical block. `verify` only bounds it within the EIP-1559
    /// ±1/1024 step vs the parent. The PROPOSER nudges it one such step per
    /// block toward its own `--builder.gaslimit` (the `target_gas_limit` fed to
    /// `step_gas_limit` on the propose path): that flag sets the TARGET the
    /// agreed value walks toward, NOT the per-block value itself — reading a
    /// local `--builder.gaslimit` at derive/verify time would diverge nodes.
    pub gas_limit: u64,
    /// The production record — `[version: u8][leader_index: u8][accused: u8]`,
    /// naming this block's producer and, optionally, the committee member it
    /// convicts of equivocation. The producer half is checked at VOTE time
    /// against the consensus-supplied round leader
    /// (`application::structural_checks`), the accusation half against
    /// [`Self::equivocation`] (`application::equivocation_gate_decision`). Copied
    /// verbatim into the derived EVM header, where the executor feeds the
    /// producer to `ProductionLiveness.recordProduction` and the accusation to
    /// the slash system call — which is why the VERDICT rides here and its
    /// EVIDENCE does not: a node syncing the EL from peers re-executes from the
    /// header alone and has no OrderBlock.
    pub extra_data: Bytes,
    /// EVM hash of the DERIVED block at `height − K`; `B256::ZERO` while
    /// `height < anchor + K` (see [`result_target`]); the anchor EVM hash in
    /// the genesis/anchor artifact (binds the ordering chain to the EVM
    /// chain).
    pub result: B256,
    /// Ordered raw transactions.
    pub txs: Vec<TransactionSigned>,
    /// Equivocation evidence backing this block's charge — the encoded
    /// commonware `Activity` for one of the three attributable Byzantine
    /// variants. Present IFF `extra_data` names an accused committee index, and
    /// the two are bound to each other at vote time
    /// (`application::equivocation_gate_decision`): every voter verifies the
    /// charge from the block in front of it, so block validity never depends on
    /// whether the evidence gossip reached that voter in time.
    ///
    /// Opaque at this codec layer (the decode needs the epoch committee, which
    /// this layer does not hold). Never reaches the EVM — it is a consensus-only
    /// artifact; the verdict the executor acts on rides in `extra_data`, which
    /// the derived header copies verbatim.
    pub equivocation: Option<Bytes>,
}

impl OrderBlock {
    /// keccak256 over the canonical codec encoding — the consensus identity.
    pub fn digest(&self) -> Digest {
        Digest(keccak256(self.encode()))
    }
}

/// Which executed hash an OrderBlock at `height` must commit in `result`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResultTarget {
    /// `height < anchor + K`: no DPoS-derived block exists K back (a fresh
    /// node may not even hold pre-anchor history) — `result` MUST be ZERO.
    PreActivation,
    /// `result` MUST equal the derived EVM hash at this height.
    Height(u64),
}

/// `anchor_height` = the ordering-chain genesis height ([`anchor_order_block`]).
/// The result-final cursor for an ordering-finalized tip: `tip - K`, clamped
/// to `floor` (the cold-start anchor, result-final by construction). The ONE
/// definition of the two-tier lag — every FCU-finalized computation (executor,
/// trust-follower mirror) must go through it so the tiers cannot drift.
pub fn result_final_height(ordering_tip: u64, floor: u64) -> u64 {
    ordering_tip.saturating_sub(K).max(floor)
}

pub fn result_target(height: u64, anchor_height: u64) -> ResultTarget {
    if height < anchor_height.saturating_add(K) {
        ResultTarget::PreActivation
    } else {
        ResultTarget::Height(height - K)
    }
}

/// True iff `result` matches what the locally-derived chain commits at the
/// K-lagged result-final height. `executed_hash(h)` returns `None` while the
/// derive at `h` is not yet locally resolved — the caller decides whether
/// absence (`None`) is tolerable (executor: keep cursor) or a vote-false
/// (verify). The single definition of the trustless result cross-check, shared
/// by `FluentApp::verify` and the executor's finalized derive.
pub fn result_matches(
    result: B256,
    height: u64,
    anchor_height: u64,
    executed_hash: impl Fn(u64) -> Option<B256>,
) -> Option<bool> {
    match result_target(height, anchor_height) {
        ResultTarget::PreActivation => Some(result == B256::ZERO),
        ResultTarget::Height(h) => executed_hash(h).map(|local| result == local),
    }
}

/// The ordering-chain anchor for an EVM anchor block: empty tx list,
/// `result` = the anchor's EVM hash, parent = EMPTY. Deterministic across
/// nodes given the same anchor (devnet genesis / migration weak-subjectivity
/// checkpoint).
///
/// Fails loud (bug 14) if the anchor's gas limit is below [`MIN_GAS_LIMIT`]: the
/// anchor seeds the EIP-1559 ±1/1024 progression, and `step_gas_limit`'s
/// `.max(MIN_GAS_LIMIT)` at anchor+1 would jump further than `gas_limit_within_1_1024`
/// accepts, so a sub-floor anchor (a malformed genesis/checkpoint) bricks the chain
/// at anchor+1 with only a per-node vote-false. The check lives in this
/// byte-identical constructor so every node rejects identically.
pub fn anchor_order_block(
    anchor: &SealedBlock<reth_ethereum_primitives::Block>,
) -> eyre::Result<OrderBlock> {
    use alloy_consensus::BlockHeader as _;
    eyre::ensure!(
        anchor.gas_limit() >= MIN_GAS_LIMIT,
        "anchor block {} carries gas_limit {} < MIN_GAS_LIMIT {} — a malformed \
         genesis/checkpoint would brick the chain at anchor+1 (propose clamps to the \
         floor, verify rejects the jump); refusing to anchor",
        anchor.number(),
        anchor.gas_limit(),
        MIN_GAS_LIMIT
    );
    Ok(OrderBlock {
        parent: Digest(B256::ZERO),
        height: anchor.number(),
        // The anchor is never PROPOSED (it has no simplex round), so `0` here
        // is a plain value, not a sentinel — nothing may branch on it.
        proposal_view: 0,
        timestamp: anchor.timestamp(),
        fee_recipient: Address::ZERO,
        // Seeds the EIP-1559 ±1/1024 gas-limit progression of the ordering chain.
        gas_limit: anchor.gas_limit(),
        extra_data: Bytes::new(),
        result: anchor.hash(),
        txs: Vec::new(),
        equivocation: None,
    })
}

// Wire format (all integers big-endian via commonware primitives):
//   parent(32) ‖ height(8) ‖ proposal_view(8) ‖ timestamp(8) ‖ fee_recipient(20)
//   ‖ gas_limit(8) ‖ result(32) ‖ extra_data_len(4)+bytes ‖ txs as one RLP list
//   ‖ beacon_flags(1)
//   ‖ [equivocation_len(4)+bytes].
// `beacon_flags` bit2 = equivocation present, its body written iff the bit is
// set. It is the only live bit.
//
// BITS 0, 1 AND 3 ARE RESERVED AND MUST STAY CLEAR. They carried the retired
// `beacon_outcome`, `parent_seed` and `dkg_logs` bodies; the epoch key is now
// agreed on the p2p agreement plane and delivered as a quorum-signed artifact,
// and the round seed is resolved from the local `SeedStore` at the block's own
// round, so no block carries beacon material at all. None of them were
// renumbered — bit2 and its body stay byte-identical to the pre-shrink encoding
// — and decode REJECTS any of the three being set, because a set-but-unread bit
// would be a second spelling of the same block and the flags byte is inside
// `digest()`.
//
// RELEASE DISCIPLINE FOR THOSE REMOVALS: COORDINATED, NOT ROLLING. The flags
// byte and every optional body sit inside `digest()`, so an old binary and a new
// one disagree on the digest of any block that carried an outcome, a witness or
// dealer logs — they cannot be run against each other on one chain, at any
// overlap. There is no migration to perform and none is provided: this is
// acceptable ONLY because Fluent networks are relaunched from block 0, so no
// live chain has to cross the change. A future deployment that must survive a
// rolling upgrade cannot reuse this pattern.
//
// `equivocation` is CANONICAL: its flag-set-but-empty encoding is rejected
// because it is a second spelling of an absent charge. The RLP tx list reuses
// alloy's canonical encoding so tx bytes are identical to their EVM-block
// representation. `proposal_view` and `equivocation` are both part of the
// encoding (hence the digest): a `proposal_view` outside the digest would be
// forgeable, and evidence outside it could be swapped after the committee
// attested it.

impl Write for OrderBlock {
    fn write(&self, buf: &mut impl BufMut) {
        use alloy_rlp::Encodable as _;
        self.parent.write(buf);
        self.height.write(buf);
        self.proposal_view.write(buf);
        self.timestamp.write(buf);
        buf.put_slice(self.fee_recipient.as_slice());
        self.gas_limit.write(buf);
        buf.put_slice(self.result.as_slice());
        (self.extra_data.len() as u32).write(buf);
        buf.put_slice(&self.extra_data);
        self.txs.encode(buf);
        // bit2 = equivocation present. Bits 0, 1 and 3 are RESERVED (retired
        // `beacon_outcome` / `parent_seed` / `dkg_logs`) and stay clear — no
        // renumbering, so bit2 keeps its pre-shrink position.
        let flags = (self.equivocation.is_some() as u8) << 2;
        flags.write(buf);
        if let Some(evidence) = &self.equivocation {
            (evidence.len() as u32).write(buf);
            buf.put_slice(evidence);
        }
    }
}

impl EncodeSize for OrderBlock {
    fn encode_size(&self) -> usize {
        use alloy_rlp::Encodable as _;
        // Mirrors `write` field-for-field, drawing each term from the SAME size
        // source the matching `write` line emits — so the two cannot drift:
        // fixed-codec fields report their own `encode_size()`, raw `put_slice`
        // fields their slice length, and each length-prefixed field a `u32`
        // header. `LEN_PREFIX` is that header (the `(len as u32).write` in
        // `write`); `FLAGS` is the single beacon-presence byte. The
        // `codec_round_trip` test pins `encode_size() == encode().len()`.
        const LEN_PREFIX: usize = u32::SIZE;
        const FLAGS: usize = u8::SIZE;
        self.parent.encode_size()
            + self.height.encode_size()
            + self.proposal_view.encode_size()
            + self.timestamp.encode_size()
            + self.fee_recipient.as_slice().len()
            + self.gas_limit.encode_size()
            + self.result.as_slice().len()
            + LEN_PREFIX
            + self.extra_data.len()
            + self.txs.length()
            + FLAGS
            + self
                .equivocation
                .as_ref()
                .map_or(0, |e| LEN_PREFIX + e.len())
    }
}

impl Read for OrderBlock {
    type Cfg = ();

    fn read_cfg(buf: &mut impl Buf, _cfg: &Self::Cfg) -> Result<Self, commonware_codec::Error> {
        let parent = Digest::read_cfg(buf, &())?;
        let height = u64::read_cfg(buf, &())?;
        let proposal_view = u64::read_cfg(buf, &())?;
        let timestamp = u64::read_cfg(buf, &())?;
        let fee_recipient = Address::from(<[u8; 20]>::read_cfg(buf, &())?);
        let gas_limit = u64::read_cfg(buf, &())?;
        let result = B256::from(<[u8; 32]>::read_cfg(buf, &())?);
        let extra_len = u32::read_cfg(buf, &())? as usize;
        if extra_len > MAX_EXTRA_DATA_SIZE {
            return Err(commonware_codec::Error::Invalid(
                "order_block",
                "extra_data exceeds MAX_EXTRA_DATA_SIZE",
            ));
        }
        if extra_len > buf.remaining() {
            return Err(commonware_codec::Error::EndOfBuffer);
        }
        let extra_data = Bytes::from(buf.copy_to_bytes(extra_len));
        // NOTE: `buf.chunk()` is only guaranteed to return *a* contiguous
        // slice. Safe under the current p2p transport (delivers contiguous
        // `Bytes`); documented so a future segmented `Buf` source is caught
        // here — same caveat as the executed-block codec it replaces.
        let header = alloy_rlp::Header::decode(&mut buf.chunk()).map_err(|e| {
            commonware_codec::Error::Wrapped("reading tx list RLP header", e.into())
        })?;
        if header.length_with_payload() > MAX_ORDER_BLOCK_SIZE {
            return Err(commonware_codec::Error::Invalid(
                "order_block",
                "tx list exceeds MAX_ORDER_BLOCK_SIZE",
            ));
        }
        if header.length_with_payload() > buf.remaining() {
            return Err(commonware_codec::Error::EndOfBuffer);
        }
        let bytes = buf.copy_to_bytes(header.length_with_payload());
        let txs: Vec<TransactionSigned> = alloy_rlp::Decodable::decode(&mut bytes.as_ref())
            .map_err(|e| commonware_codec::Error::Wrapped("reading tx list", e.into()))?;
        let flags = u8::read_cfg(buf, &())?;
        // Bits 0, 1 and 3 carried the retired `beacon_outcome` / `parent_seed` /
        // `dkg_logs` bodies. They were not renumbered, and a set-but-unread bit
        // would be a second spelling of the same block under a different digest
        // (the flags byte is inside `digest()`), so reject rather than ignore.
        if flags & 0b0000_1011 != 0 {
            return Err(commonware_codec::Error::Invalid(
                "order_block",
                "beacon_flags: a reserved bit is set",
            ));
        }
        let equivocation = if flags & 4 != 0 {
            let len = u32::read_cfg(buf, &())? as usize;
            if len > MAX_EQUIVOCATION_SIZE {
                return Err(commonware_codec::Error::Invalid(
                    "order_block",
                    "equivocation exceeds MAX_EQUIVOCATION_SIZE",
                ));
            }
            if len == 0 {
                // bit2 set with an empty body is a second spelling of "no charge"
                // (the canonical one is a cleared bit), and two encodings of the
                // same block would carry two digests.
                return Err(commonware_codec::Error::Invalid(
                    "order_block",
                    "equivocation flag set with an empty body",
                ));
            }
            if len > buf.remaining() {
                return Err(commonware_codec::Error::EndOfBuffer);
            }
            Some(Bytes::from(buf.copy_to_bytes(len)))
        } else {
            None
        };
        let block = Self {
            parent,
            height,
            proposal_view,
            timestamp,
            fee_recipient,
            gas_limit,
            extra_data,
            result,
            txs,
            equivocation,
        };
        // Combined-size gate (bug 1): each variable-length field is bounded
        // against its OWN cap above (extra_data, tx list, equivocation), but the
        // TOTAL is not — a block at the full tx budget plus a charge could exceed
        // MAX_ORDER_BLOCK_SIZE and rely only on the p2p frame drop.
        // Reject the composed over-cap artifact here so every honest verifier
        // rejects deterministically. `encode_size` is the single drift-proof size
        // source (pinned == `encode().len()`), so this rejects strictly what would
        // not round-trip within the cap — honest artifacts (well under it) pass.
        if block.encode_size() > MAX_ORDER_BLOCK_SIZE {
            return Err(commonware_codec::Error::Invalid(
                "order_block",
                "composed OrderBlock exceeds MAX_ORDER_BLOCK_SIZE",
            ));
        }
        Ok(block)
    }
}

impl Committable for OrderBlock {
    type Commitment = Digest;

    fn commitment(&self) -> Self::Commitment {
        self.digest()
    }
}

impl Digestible for OrderBlock {
    type Digest = Digest;

    fn digest(&self) -> Self::Digest {
        self.digest()
    }
}

impl Heightable for OrderBlock {
    fn height(&self) -> Height {
        Height::new(self.height)
    }
}

impl commonware_consensus::Block for OrderBlock {
    fn parent(&self) -> Digest {
        self.parent
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::{Block as AlloyBlock, BlockBody, Header};
    use alloy_primitives::U256;
    use commonware_codec::ReadExt as _;
    use reth_primitives_traits::SealedBlock;

    fn sample_order_block() -> OrderBlock {
        OrderBlock {
            parent: Digest(B256::repeat_byte(0x11)),
            height: 42,
            proposal_view: 7,
            timestamp: 1_700_000_000,
            fee_recipient: Address::repeat_byte(0x22),
            gas_limit: 50_000_000,
            extra_data: Bytes::from(vec![1u8, 2, 3]),
            result: B256::repeat_byte(0x33),
            txs: Vec::new(),
            equivocation: None,
        }
    }

    #[test]
    fn codec_round_trip() {
        let original = sample_order_block();
        let encoded = original.encode();
        assert_eq!(original.encode_size(), encoded.len());
        let decoded = OrderBlock::read(&mut encoded.as_ref()).expect("decode");
        assert_eq!(original, decoded);
    }

    #[test]
    fn codec_round_trip_with_the_equivocation_trailer() {
        let mut original = sample_order_block();
        original.equivocation = Some(Bytes::from(vec![0x5Au8; 291]));
        let encoded = original.encode();
        assert_eq!(original.encode_size(), encoded.len());
        let decoded = OrderBlock::read(&mut encoded.as_ref()).expect("decode");
        assert_eq!(original, decoded);
    }

    /// Hand-encode everything up to and including the beacon_flags byte with
    /// ONLY bit2 (equivocation present) set — the trailer is the test's variable.
    fn write_bit2_frame_prefix(buf: &mut Vec<u8>, b: &OrderBlock) {
        write_header_prefix(buf, b);
        (b.extra_data.len() as u32).write(buf);
        buf.extend_from_slice(&b.extra_data);
        {
            use alloy_rlp::Encodable as _;
            b.txs.encode(buf);
        }
        4u8.write(buf); // beacon_flags: equivocation present, nothing else
    }

    #[test]
    fn read_rejects_oversize_equivocation() {
        let mut buf = Vec::new();
        let b = sample_order_block();
        write_bit2_frame_prefix(&mut buf, &b);
        ((MAX_EQUIVOCATION_SIZE + 1) as u32).write(&mut buf);
        buf.resize(buf.len() + MAX_EQUIVOCATION_SIZE + 1, 0);

        let err = OrderBlock::read(&mut buf.as_slice()).expect_err("oversize equivocation");
        assert!(
            matches!(err, commonware_codec::Error::Invalid(_, m) if m.contains("MAX_EQUIVOCATION_SIZE")),
            "expected the per-field cap rejection, got {err:?}"
        );
    }

    #[test]
    fn read_rejects_equivocation_flag_set_with_an_empty_body() {
        // A set bit with a zero-length body is a second spelling of "no charge",
        // and two encodings of one block would carry two digests.
        let mut buf = Vec::new();
        let b = sample_order_block();
        write_bit2_frame_prefix(&mut buf, &b);
        0u32.write(&mut buf);

        let err = OrderBlock::read(&mut buf.as_slice()).expect_err("empty equivocation body");
        assert!(
            matches!(err, commonware_codec::Error::Invalid(_, m) if m.contains("empty body")),
            "expected the canonicality rejection, got {err:?}"
        );
    }

    #[test]
    fn read_bit2_set_but_truncated_equivocation_is_end_of_buffer() {
        // The flag promises `len` trailing bytes; a frame that ends early must
        // fail as a short buffer rather than decode a truncated charge.
        let b = sample_order_block();
        for keep in [0usize, 1, 290] {
            let mut buf = Vec::new();
            write_bit2_frame_prefix(&mut buf, &b);
            291u32.write(&mut buf);
            buf.resize(buf.len() + keep, 0);
            let err = OrderBlock::read(&mut buf.as_slice()).expect_err("truncated equivocation");
            assert!(
                matches!(err, commonware_codec::Error::EndOfBuffer),
                "expected EndOfBuffer with {keep} of 291 bytes, got {err:?}"
            );
        }
    }

    /// The wire-shrink's byte-identity claim, pinned against a golden capture
    /// taken from the pre-shrink codec: bits 0, 1 and 3 lost their bodies WITHOUT
    /// renumbering, so a block carrying `equivocation` encodes to the exact same
    /// bytes — flags `0b0000_0100`, charge trailing — as it did while
    /// `beacon_outcome` / `parent_seed` / `dkg_logs` still existed. Renumbering
    /// bit2→bit0 would flip the flags byte to `0b0000_0001` and silently move the
    /// digest of every block that carries a charge.
    ///
    /// The constant is NOT a capture of the current encoder. It is the same
    /// pre-shrink capture the previous revision pinned, edited by hand in exactly
    /// two places — flags `06` → `04`, and the 50-byte seed body deleted — so it
    /// still testifies against an independently-produced encoding rather than
    /// against the code under test.
    #[test]
    fn the_shrink_leaves_bit_2_byte_identical() {
        const PRE_SHRINK_GOLDEN: &str = "\
1111111111111111111111111111111111111111111111111111111111111111\
000000000000002a0000000000000007000000006553f100\
22222222222222222222222222222222222222220000000002faf080\
3333333333333333333333333333333333333333333333333333333333333333\
00000003010203c004\
000000085a5a5a5a5a5a5a5a";

        let mut block = sample_order_block();
        block.equivocation = Some(Bytes::from(vec![0x5Au8; 8]));
        let encoded = block.encode();
        assert_eq!(
            alloy_primitives::hex::encode(&encoded),
            PRE_SHRINK_GOLDEN,
            "the shrink moved bytes that must not move"
        );

        // Spelt out separately from the golden so a future reader sees WHICH
        // byte carries the claim: 0b0000_0100, not the renumbered 0b0000_0001.
        let flags_at = encoded.len() - u32::SIZE - 8 - 1;
        assert_eq!(encoded[flags_at], 0b0000_0100);
    }

    #[test]
    fn read_rejects_a_set_reserved_flag_bit() {
        // Bits 0, 1 and 3 are retired, not renumbered. Ignoring a set one would
        // give a block two spellings under two digests (the flags byte is inside
        // `digest()`), so decode must refuse it. Bit1 is the newest arrival
        // (`parent_seed`): the frames the seed-trailer tests used to hand-encode
        // are now exactly this rejection, not a decode.
        for bit in [0b0000_0001u8, 0b0000_0010, 0b0000_1000] {
            let mut buf = Vec::new();
            let b = sample_order_block();
            write_header_prefix(&mut buf, &b);
            (b.extra_data.len() as u32).write(&mut buf);
            buf.extend_from_slice(&b.extra_data);
            {
                use alloy_rlp::Encodable as _;
                b.txs.encode(&mut buf);
            }
            bit.write(&mut buf);
            let err = OrderBlock::read(&mut buf.as_slice()).expect_err("reserved flag bit");
            assert!(
                matches!(err, commonware_codec::Error::Invalid(_, m) if m.contains("reserved bit")),
                "bit {bit:#010b} must be rejected, got {err:?}"
            );
        }
    }

    #[test]
    fn digest_excludes_nothing_and_is_stable_per_field() {
        // The digest is the consensus identity: any field change MUST change
        // it (a field outside the digest would be unagreed data).
        let base = sample_order_block();
        let d = base.digest();
        let mutations: Vec<OrderBlock> = vec![
            OrderBlock {
                parent: Digest(B256::repeat_byte(0xAA)),
                ..base.clone()
            },
            OrderBlock {
                height: base.height + 1,
                ..base.clone()
            },
            OrderBlock {
                timestamp: base.timestamp + 1,
                ..base.clone()
            },
            OrderBlock {
                fee_recipient: Address::repeat_byte(0xBB),
                ..base.clone()
            },
            OrderBlock {
                gas_limit: base.gas_limit + 1,
                ..base.clone()
            },
            OrderBlock {
                extra_data: Bytes::from(vec![9u8]),
                ..base.clone()
            },
            OrderBlock {
                result: B256::repeat_byte(0xCC),
                ..base.clone()
            },
            OrderBlock {
                proposal_view: base.proposal_view + 1,
                ..base.clone()
            },
            OrderBlock {
                equivocation: Some(Bytes::from(vec![0x5Au8; 291])),
                ..base.clone()
            },
        ];
        for m in mutations {
            assert_ne!(m.digest(), d);
        }
    }

    /// Hand-encode the fixed OrderBlock header prefix (everything before
    /// extra_data) — mirrors the `Write` impl so the oversize-decode tests
    /// below share one copy of the byte layout instead of three.
    fn write_header_prefix(buf: &mut Vec<u8>, b: &OrderBlock) {
        b.parent.write(buf);
        b.height.write(buf);
        b.proposal_view.write(buf);
        b.timestamp.write(buf);
        buf.extend_from_slice(b.fee_recipient.as_slice());
        b.gas_limit.write(buf);
        buf.extend_from_slice(b.result.as_slice());
    }

    #[test]
    fn read_rejects_oversize_extra_data() {
        let mut buf = Vec::new();
        let b = sample_order_block();
        write_header_prefix(&mut buf, &b);
        ((MAX_EXTRA_DATA_SIZE + 1) as u32).write(&mut buf);
        buf.resize(buf.len() + MAX_EXTRA_DATA_SIZE + 1, 0);

        let err = OrderBlock::read(&mut buf.as_slice()).expect_err("oversize extra_data");
        assert!(matches!(err, commonware_codec::Error::Invalid(_, _)));
    }

    #[test]
    fn read_rejects_oversize_tx_list() {
        let b = sample_order_block();
        let mut buf = Vec::new();
        write_header_prefix(&mut buf, &b);
        0u32.write(&mut buf);
        let oversize = MAX_ORDER_BLOCK_SIZE + 1;
        alloy_rlp::Header {
            list: true,
            payload_length: oversize,
        }
        .encode(&mut buf);
        buf.resize(buf.len() + oversize, 0);

        let err = OrderBlock::read(&mut buf.as_slice()).expect_err("oversize tx list");
        assert!(matches!(err, commonware_codec::Error::Invalid(_, _)));
    }

    #[test]
    fn read_consumes_exactly_the_charge_bytes_of_a_bit2_frame() {
        // Hand-encoded (NOT via `Write`) so this pins the wire layout itself: a
        // bit2 frame's trailer is a `u32` length and exactly that many bytes —
        // no padding — and the decoder must stop precisely at its end. Inherited
        // from the bit1 frame this replaces: with the seed gone, the charge is
        // the only trailer whose exact consumption can still be pinned.
        let b = sample_order_block();
        let charge = vec![0x5Au8; 291];
        let mut buf = Vec::new();
        write_bit2_frame_prefix(&mut buf, &b);
        let before_charge = buf.len();
        (charge.len() as u32).write(&mut buf);
        buf.extend_from_slice(&charge);
        assert_eq!(buf.len() - before_charge, u32::SIZE + charge.len());

        let mut slice = buf.as_slice();
        let decoded = OrderBlock::read(&mut slice).expect("decode bit2 frame");
        assert!(
            slice.is_empty(),
            "decoder must consume the exact charge bytes"
        );
        assert_eq!(decoded.equivocation, Some(Bytes::from(charge)));
    }

    /// The composed-size gate (bug 1), and that it counts the equivocation field.
    ///
    /// It used to have a twin that composed an oversize block out of a max-size
    /// `beacon_outcome`; with that field gone the two state the same property, and
    /// this is the stronger one — it MEASURES the headroom instead of assuming a
    /// field big enough to blow past it.
    #[test]
    fn the_composed_size_gate_counts_the_equivocation_field() {
        use alloy_primitives::{Signature, TxKind};
        use reth_ethereum_primitives::Transaction;
        let block_with_tx = |input_len: usize| {
            let tx = alloy_consensus::TxEip1559 {
                chain_id: 1,
                nonce: 0,
                gas_limit: 21_000,
                max_fee_per_gas: 0,
                max_priority_fee_per_gas: 0,
                to: TxKind::Call(Address::ZERO),
                value: U256::ZERO,
                input: Bytes::from(vec![0u8; input_len]),
                ..Default::default()
            };
            let sig = Signature::new(U256::from(1u64), U256::from(1u64), false);
            let mut block = sample_order_block();
            block.txs = vec![TransactionSigned::new_unhashed(
                Transaction::Eip1559(tx),
                sig,
            )];
            block
        };

        // Grow the tx list until the charge-free block sits within one max-size
        // charge of the cap — only then does the charge alone decide whether the
        // composed artifact fits. Each step is smaller than the headroom it
        // measures, so the block stays under the cap throughout.
        let mut input_len = 4_100_000usize;
        let mut block = block_with_tx(input_len);
        let headroom = loop {
            let headroom = MAX_ORDER_BLOCK_SIZE
                .checked_sub(block.encode_size())
                .expect("the charge-free block stays under the cap");
            if headroom <= MAX_EQUIVOCATION_SIZE {
                break headroom;
            }
            input_len += headroom - MAX_EQUIVOCATION_SIZE / 2;
            block = block_with_tx(input_len);
        };
        OrderBlock::read(&mut block.encode().as_ref())
            .expect("the charge-free block is within the cap and decodes");

        // A charge that exactly fills the headroom is over it once its own 4-byte
        // length prefix is counted. It passes its OWN cap; only the combined gate
        // (bug 1) rejects — which is what EQUIVOCATION_FRAMING's carve-out out of
        // TX_BYTE_BUDGET keeps an honest proposer clear of.
        block.equivocation = Some(Bytes::from(vec![0u8; headroom]));
        assert!(
            block.encode_size() > MAX_ORDER_BLOCK_SIZE,
            "the composed block must be over the cap"
        );
        let err = OrderBlock::read(&mut block.encode().as_ref())
            .expect_err("combined-oversize block must be rejected at decode");
        assert!(
            matches!(err, commonware_codec::Error::Invalid(_, m) if m.contains("composed")),
            "expected the composed-size rejection, got {err:?}"
        );
    }

    #[test]
    fn anchor_binds_evm_hash_and_seeds_gas_limit() {
        let header = Header {
            parent_hash: B256::repeat_byte(0x44),
            number: 6_700_000,
            gas_limit: 50_000_000,
            timestamp: 1_700_000_000,
            difficulty: U256::ZERO,
            ..Default::default()
        };
        let body: BlockBody<TransactionSigned> = BlockBody::default();
        let sealed = SealedBlock::seal_slow(reth_ethereum_primitives::Block::from(
            AlloyBlock::new(header, body),
        ));

        let anchor = anchor_order_block(&sealed).expect("well-formed anchor");
        assert_eq!(anchor.result, sealed.hash());
        assert_eq!(anchor.height, 6_700_000);
        assert_eq!(anchor.gas_limit, 50_000_000);
        assert!(anchor.txs.is_empty());

        // Deterministic across construction sites: identity = digest equality.
        assert_eq!(
            anchor.digest(),
            anchor_order_block(&sealed).unwrap().digest()
        );
    }

    #[test]
    fn anchor_below_min_gas_limit_is_rejected_loud() {
        // A malformed genesis/checkpoint (< MIN_GAS_LIMIT) would brick the chain at
        // anchor+1; the byte-identical constructor refuses it (bug 14).
        let header = Header {
            number: 6_700_000,
            gas_limit: 4_000, // < MIN_GAS_LIMIT (5000)
            timestamp: 1_700_000_000,
            difficulty: U256::ZERO,
            ..Default::default()
        };
        let sealed = SealedBlock::seal_slow(reth_ethereum_primitives::Block::from(
            AlloyBlock::new(header, BlockBody::<TransactionSigned>::default()),
        ));
        let err = anchor_order_block(&sealed).expect_err("sub-floor anchor must be rejected");
        assert!(
            err.to_string().contains("MIN_GAS_LIMIT"),
            "the error must name the floor: {err}"
        );
    }

    #[test]
    fn result_matches_distinguishes_absence_match_and_mismatch() {
        let anchor = 100;
        let local = B256::repeat_byte(0x77);
        // Pre-activation: must commit ZERO, resolvable without an executed hash.
        assert_eq!(
            result_matches(B256::ZERO, anchor + 1, anchor, |_| None),
            Some(true)
        );
        assert_eq!(
            result_matches(local, anchor + 1, anchor, |_| None),
            Some(false)
        );
        // Post-activation, hash not yet locally derived → None (tolerable).
        assert_eq!(result_matches(local, anchor + K, anchor, |_| None), None);
        // Post-activation, present hash → exact match / mismatch.
        assert_eq!(
            result_matches(local, anchor + K, anchor, |_| Some(local)),
            Some(true)
        );
        assert_eq!(
            result_matches(local, anchor + K, anchor, |_| Some(B256::ZERO)),
            Some(false)
        );
    }

    #[test]
    fn result_target_pre_activation_window_is_k_blocks() {
        let anchor = 100;
        assert_eq!(
            result_target(anchor + 1, anchor),
            ResultTarget::PreActivation
        );
        assert_eq!(
            result_target(anchor + K - 1, anchor),
            ResultTarget::PreActivation
        );
        assert_eq!(
            result_target(anchor + K, anchor),
            ResultTarget::Height(anchor)
        );
        assert_eq!(
            result_target(anchor + K + 5, anchor),
            ResultTarget::Height(anchor + 5)
        );
    }
}
