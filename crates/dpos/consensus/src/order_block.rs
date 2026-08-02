//! The F-type consensus unit: ordering only — txs + parent digest + result
//! commitment. The digest deliberately excludes every execution output of
//! THIS block; `result` commits the derived block hash K heights back, so
//! agreeing OrderBlock N+K is the committee's attestation of block N's
//! execution result.

use crate::beacon::{outcome::MAX_BEACON_OUTCOME_SIZE, seed::Seed};
use crate::digest::Digest;
use alloy_primitives::{keccak256, Address, Bytes, B256};
use bytes::{Buf, BufMut};
use commonware_codec::{
    varint::MAX_U64_VARINT_SIZE, Encode as _, EncodeSize, FixedSize, Read, Write,
};
use commonware_consensus::{types::Height, Heightable};
use commonware_cryptography::{Committable, Digestible};
use fluentbase_bls::BlsSignature;
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

/// Tx-list byte budget for ordering assembly: [`MAX_ORDER_BLOCK_SIZE`] minus
/// the [`MAX_EXTRA_DATA_SIZE`] allowance for the non-tx fields (parent/height/
/// result, extra_data, codec framing) and the
/// [`PARENT_SEED_FRAMING`] allowance for `proposal_view` + `parent_seed`, so an
/// assembled artifact always fits its own decode cap.
pub const TX_BYTE_BUDGET: usize =
    MAX_ORDER_BLOCK_SIZE - MAX_EXTRA_DATA_SIZE - PARENT_SEED_FRAMING - DKG_LOGS_FRAMING;

/// Decode cap for the `dkg_logs` field (AMENDMENT 5): up to `MAX_COMMITTEE_SIZE`
/// entries, each `idx(u8) ‖ hash([u8;32])`. A dealer-log hash index over the
/// committee, so it is committee-size-bounded like the leader index.
pub const MAX_DKG_LOGS_SIZE: usize =
    (fluentbase_p2p::constants::MAX_COMMITTEE_SIZE as usize) * (u8::SIZE + 32);

/// Worst-case wire cost of the `dkg_logs` field: its decode cap plus the `u32`
/// count prefix. Reserved out of [`TX_BYTE_BUDGET`] for EVERY block (any block in
/// the seal→settle window MAY carry dealer-log hashes), the same composed-oversize
/// carve-out (bug 1) [`BEACON_OUTCOME_FRAMING`] documents.
pub const DKG_LOGS_FRAMING: usize = MAX_DKG_LOGS_SIZE + u32::SIZE;

/// Worst-case wire cost of the `beacon_outcome` field: its decode cap plus the
/// `u32` length prefix. Reserved out of the tx budget on a change-epoch boundary
/// block (the ONE block that carries `beacon_outcome`) — `TX_BYTE_BUDGET`'s
/// carve-out list omits it, so a boundary block at the full tx budget could
/// exceed `MAX_ORDER_BLOCK_SIZE` / the byte-identical p2p frame cap (bug 1).
pub const BEACON_OUTCOME_FRAMING: usize = MAX_BEACON_OUTCOME_SIZE + u32::SIZE;

/// Worst-case wire cost of the fields the parent-seed witness added:
/// `proposal_view` (fixed `u64`, always present) plus a present `parent_seed`
/// ([`Seed`] = `Round ‖ BlsSignature`, no length prefix; `Round`'s epoch and
/// view are varint-encoded, so the ceiling is [`MAX_U64_VARINT_SIZE`] each —
/// the flag bit rides the pre-existing `beacon_flags` byte). Reserved out of
/// [`TX_BYTE_BUDGET`] so a full-budget block carrying the witness still fits
/// `MAX_ORDER_BLOCK_SIZE` / the byte-identical p2p frame cap — the same
/// composed-oversize class (bug 1) [`BEACON_OUTCOME_FRAMING`] documents.
pub const PARENT_SEED_FRAMING: usize = u64::SIZE + 2 * MAX_U64_VARINT_SIZE + BlsSignature::SIZE;

/// Tx-list byte budget for a change-epoch BOUNDARY block: [`TX_BYTE_BUDGET`]
/// minus [`BEACON_OUTCOME_FRAMING`], so a boundary block carrying `beacon_outcome`
/// still fits `MAX_ORDER_BLOCK_SIZE` and the p2p frame cap.
pub const TX_BYTE_BUDGET_AT_BOUNDARY: usize = TX_BYTE_BUDGET - BEACON_OUTCOME_FRAMING;

/// Decode cap for `extra_data`. Deliberately far above the 2-byte production
/// record it actually carries: this cap only has to compose with the
/// TX_BYTE_BUDGET allowance. The BINDING bound is the vote-time exact-length
/// rule in `application::structural_checks`, without which an over-length field
/// could finalize here and then be unexecutable at the 32-byte reth header cap.
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
    /// The production record — `[version: u8][leader_index: u8]`, naming this
    /// block's producer. Checked at VOTE time against the consensus-supplied
    /// round leader (`application::structural_checks`) and copied verbatim into
    /// the derived EVM header, where the executor feeds it to
    /// `ProductionLiveness.recordProduction`.
    pub extra_data: Bytes,
    /// EVM hash of the DERIVED block at `height − K`; `B256::ZERO` while
    /// `height < anchor + K` (see [`result_target`]); the anchor EVM hash in
    /// the genesis/anchor artifact (binds the ordering chain to the EVM
    /// chain).
    pub result: B256,
    /// Ordered raw transactions.
    pub txs: Vec<TransactionSigned>,
    /// Per-epoch DKG outcome (the encoded commonware `Output` = group key
    /// `PK_{E+1}` + public polynomial), present ONLY on the epoch-boundary
    /// block that agrees the next committee's beacon key. Opaque at this codec
    /// layer (parsed by `beacon` with the committee-size config — `Output`'s
    /// decode needs it); the system call then publishes `PK_{E+1}` to L2.
    pub beacon_outcome: Option<Bytes>,
    /// AMENDMENT 5 determinism core: the content hashes of the E+1 DKG dealer logs
    /// this block's proposer has recorded (a `check`-valid body it holds), as
    /// `(idx, hash)` where `idx` = the dealer's position in the agreed
    /// `committee[epoch_of(height)+1]` and `hash = keccak256(SignedDealerLog)`.
    /// CANONICAL: strictly ascending by `idx`, no duplicate `idx` (enforced at
    /// decode so the digest is unambiguous). Accumulated across finalized blocks
    /// into the FINALIZED dealer-log set that the DKG finalize `select` runs over —
    /// making `PK_{E+1}` a pure function of agreed consensus data (honest divergence
    /// impossible). Empty on every block outside the seal→settle window. `verify` is
    /// FORMAT-ONLY (`idx < n`, sorted, no dups, cap) — a verifier need NOT hold the
    /// bodies (off-chain-first; they ride the dealer `Reveal` + the recovery resolver).
    pub dkg_logs: Vec<(u8, B256)>,
    /// Threshold randomness seed of the round in which the PARENT block was
    /// certified, carried by the child (Design B′): `prev_randao(parent) =
    /// H(seed.signature)`. Delivers the parent's seed via the one object that
    /// always exists when the parent has a finalized descendant — the child —
    /// instead of a per-height finalization cert that commonware builds only
    /// best-effort (an ancestry-finalized height may have NO standalone cert
    /// anywhere, ever). `seed.target_round` must equal
    /// `Round::new(parent_epoch, parent.proposal_view)` (rule PIN — agreed
    /// data, no local cert state). Mandatory on every beacon-active link,
    /// boundary included; `None` only on pre-bootstrap links — enforced by
    /// `FluentApp::verify`'s witness gate at vote time. Embedded by
    /// `build_proposal` from `SeedStore`; consumed by the executor's
    /// one-block-lookahead pipeline (the child's `parent_seed` IS the seed the
    /// parent derives with) and by the blocks-only crash-recovery replay.
    pub parent_seed: Option<Seed>,
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
        // is a plain value, not a sentinel — nothing may branch on it. Its
        // child sits below the beacon-bootstrap epoch, so the witness pin
        // never reads it either.
        proposal_view: 0,
        timestamp: anchor.timestamp(),
        fee_recipient: Address::ZERO,
        // Seeds the EIP-1559 ±1/1024 gas-limit progression of the ordering chain.
        gas_limit: anchor.gas_limit(),
        extra_data: Bytes::new(),
        result: anchor.hash(),
        txs: Vec::new(),
        beacon_outcome: None,
        dkg_logs: Vec::new(),
        parent_seed: None,
    })
}

// Wire format (all integers big-endian via commonware primitives):
//   parent(32) ‖ height(8) ‖ proposal_view(8) ‖ timestamp(8) ‖ fee_recipient(20)
//   ‖ gas_limit(8) ‖ result(32) ‖ extra_data_len(4)+bytes ‖ txs as one RLP list
//   ‖ beacon_flags(1) ‖ [beacon_outcome_len(4)+bytes]
//   ‖ [dkg_logs_count(4) + count*(idx(1) ‖ hash(32))]
//   ‖ [parent_seed: Round(varint epoch ‖ varint view) ‖ signature(48)].
// `beacon_flags` bit0 = outcome present; bit1 = parent_seed present; bit3 =
// dkg_logs present (bit2 is retired — the removed `dkg_qual` field; each
// optional-body written iff its bit is set — the fixed-layout Seed needs no
// length prefix). Body write order follows the STRUCT field order (outcome,
// dkg_logs, parent_seed), independent of the flag-bit numbering. `dkg_logs` is
// CANONICAL — strictly ascending `idx`, no dups (enforced at decode). The RLP tx
// list reuses alloy's canonical encoding so tx bytes are identical to their
// EVM-block representation. `beacon_outcome`, `dkg_logs`, `proposal_view` and
// `parent_seed` are all part of the encoding (hence the digest): an unagreed
// next-epoch key, dealer-log set or randomness input under one digest would
// diverge derive/STF, and a `proposal_view` outside the digest would be forgeable.

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
        // bit0 = outcome present; bit1 = parent_seed present; bit3 = dkg_logs present
        // (bit2 is retired — was the removed `dkg_qual` field).
        let flags = self.beacon_outcome.is_some() as u8
            | (self.parent_seed.is_some() as u8) << 1
            | ((!self.dkg_logs.is_empty()) as u8) << 3;
        flags.write(buf);
        if let Some(outcome) = &self.beacon_outcome {
            (outcome.len() as u32).write(buf);
            buf.put_slice(outcome);
        }
        if !self.dkg_logs.is_empty() {
            (self.dkg_logs.len() as u32).write(buf);
            for (idx, hash) in &self.dkg_logs {
                idx.write(buf);
                buf.put_slice(hash.as_slice());
            }
        }
        if let Some(seed) = &self.parent_seed {
            seed.write(buf);
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
                .beacon_outcome
                .as_ref()
                .map_or(0, |o| LEN_PREFIX + o.len())
            + if self.dkg_logs.is_empty() {
                0
            } else {
                LEN_PREFIX + self.dkg_logs.len() * (u8::SIZE + 32)
            }
            + self.parent_seed.as_ref().map_or(0, |s| s.encode_size())
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
        let beacon_outcome = if flags & 1 != 0 {
            let len = u32::read_cfg(buf, &())? as usize;
            if len > MAX_BEACON_OUTCOME_SIZE {
                return Err(commonware_codec::Error::Invalid(
                    "order_block",
                    "beacon_outcome exceeds MAX_BEACON_OUTCOME_SIZE",
                ));
            }
            if len > buf.remaining() {
                return Err(commonware_codec::Error::EndOfBuffer);
            }
            Some(Bytes::from(buf.copy_to_bytes(len)))
        } else {
            None
        };
        let dkg_logs: Vec<(u8, B256)> = if flags & 8 != 0 {
            let count = u32::read_cfg(buf, &())? as usize;
            if count > fluentbase_p2p::constants::MAX_COMMITTEE_SIZE as usize {
                return Err(commonware_codec::Error::Invalid(
                    "order_block",
                    "dkg_logs exceeds MAX_COMMITTEE_SIZE entries",
                ));
            }
            if count == 0 {
                // bit3 set but empty is non-canonical (would encode as a cleared bit).
                return Err(commonware_codec::Error::Invalid(
                    "order_block",
                    "dkg_logs flag set with zero entries",
                ));
            }
            if count * (u8::SIZE + 32) > buf.remaining() {
                return Err(commonware_codec::Error::EndOfBuffer);
            }
            let mut logs = Vec::with_capacity(count);
            let mut prev: Option<u8> = None;
            for _ in 0..count {
                let idx = u8::read_cfg(buf, &())?;
                // The COUNT bound above does not bound the VALUE: idx is a u8, so without
                // this an entry could name participant 200 of a 51-seat committee. Such an
                // entry survives the vote gate whenever `committee_for` is unreadable
                // (accept-biased, `application.rs::beacon_gate_decision`) and then wedges
                // the ceremony that consumes it. MAX_COMMITTEE_SIZE is a network-wide
                // constant, so this rejects identically on every node — unlike a bound
                // against the live committee length, which would make decoding depend on
                // node-local state.
                if idx as usize >= fluentbase_p2p::constants::MAX_COMMITTEE_SIZE as usize {
                    return Err(commonware_codec::Error::Invalid(
                        "order_block",
                        "dkg_logs idx exceeds MAX_COMMITTEE_SIZE",
                    ));
                }
                // CANONICAL: strictly ascending idx (no dups, sorted) so the digest is
                // unambiguous — a permuted/duplicated encoding is rejected.
                if prev.is_some_and(|p| idx <= p) {
                    return Err(commonware_codec::Error::Invalid(
                        "order_block",
                        "dkg_logs idx not strictly ascending",
                    ));
                }
                prev = Some(idx);
                let hash = B256::from(<[u8; 32]>::read_cfg(buf, &())?);
                logs.push((idx, hash));
            }
            logs
        } else {
            Vec::new()
        };
        let parent_seed = if flags & 2 != 0 {
            Some(Seed::read_cfg(buf, &())?)
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
            beacon_outcome,
            dkg_logs,
            parent_seed,
        };
        // Combined-size gate (bug 1): each variable-length field is bounded
        // against its OWN cap above (extra_data, tx list, beacon_outcome), but the
        // TOTAL is not — a boundary block at the full tx budget plus beacon_outcome
        // could exceed MAX_ORDER_BLOCK_SIZE and rely only on the p2p frame drop.
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
            beacon_outcome: None,
            dkg_logs: Vec::new(),
            parent_seed: None,
        }
    }

    /// A real recovered threshold seed for `round` — the codec must carry a
    /// genuinely valid `Seed` (a `BlsSignature` decode enforces a valid curve
    /// point, so arbitrary bytes cannot stand in for one).
    fn real_seed(round: commonware_consensus::types::Round) -> Seed {
        use commonware_cryptography::bls12381::{dkg::deal_anonymous, primitives::variant::MinSig};
        use commonware_utils::{test_rng, N3f1, NZU32};
        use fluentbase_bls::beacon::{recover_seed, seed_namespace, sign_seed_partial};
        let mut rng = test_rng();
        let (sharing, shares) =
            deal_anonymous::<MinSig, N3f1>(&mut rng, Default::default(), NZU32!(5));
        let ns = seed_namespace(b"fluent-test");
        let partials: Vec<_> = shares
            .iter()
            .map(|s| sign_seed_partial(s, &ns, round))
            .collect();
        Seed {
            target_round: round,
            signature: recover_seed::<N3f1>(&sharing, &partials).expect("recover seed"),
        }
    }

    fn sample_seed() -> Seed {
        use commonware_consensus::types::{Epoch, Round, View};
        real_seed(Round::new(Epoch::new(3), View::new(41)))
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
    fn codec_round_trip_with_beacon_outcome() {
        let mut original = sample_order_block();
        original.beacon_outcome = Some(Bytes::from(vec![7u8; 96]));
        let encoded = original.encode();
        assert_eq!(original.encode_size(), encoded.len());
        let decoded = OrderBlock::read(&mut encoded.as_ref()).expect("decode");
        assert_eq!(original, decoded);
    }

    #[test]
    fn codec_round_trip_with_parent_seed() {
        let mut original = sample_order_block();
        original.parent_seed = Some(sample_seed());
        let encoded = original.encode();
        assert_eq!(original.encode_size(), encoded.len());
        let decoded = OrderBlock::read(&mut encoded.as_ref()).expect("decode");
        assert_eq!(original, decoded);
    }

    #[test]
    fn codec_round_trip_with_beacon_outcome_and_parent_seed() {
        // Both optional trailers present: outcome first, seed after (the
        // decoder must consume them in that order off one flags byte).
        let mut original = sample_order_block();
        original.beacon_outcome = Some(Bytes::from(vec![7u8; 96]));
        original.parent_seed = Some(sample_seed());
        let encoded = original.encode();
        assert_eq!(original.encode_size(), encoded.len());
        let decoded = OrderBlock::read(&mut encoded.as_ref()).expect("decode");
        assert_eq!(original, decoded);
    }

    #[test]
    fn codec_round_trip_with_all_optional_trailers() {
        // bit0 + bit1 both set: outcome, then parent_seed — the full
        // optional-trailer ordering off one flags byte.
        let mut original = sample_order_block();
        original.beacon_outcome = Some(Bytes::from(vec![7u8; 96]));
        original.parent_seed = Some(sample_seed());
        let encoded = original.encode();
        assert_eq!(original.encode_size(), encoded.len());
        let decoded = OrderBlock::read(&mut encoded.as_ref()).expect("decode");
        assert_eq!(original, decoded);
    }

    #[test]
    fn codec_round_trip_with_dkg_logs() {
        // bit3 (dkg_logs) present alongside outcome + parent_seed — the decoder must
        // consume outcome, then dkg_logs, then parent_seed off the one flags byte.
        let mut original = sample_order_block();
        original.beacon_outcome = Some(Bytes::from(vec![7u8; 96]));
        original.dkg_logs = vec![
            (0, B256::repeat_byte(0xA1)),
            (3, B256::repeat_byte(0xB2)),
            (9, B256::repeat_byte(0xC3)),
        ];
        original.parent_seed = Some(sample_seed());
        let encoded = original.encode();
        assert_eq!(original.encode_size(), encoded.len());
        let decoded = OrderBlock::read(&mut encoded.as_ref()).expect("decode");
        assert_eq!(original, decoded);
    }

    #[test]
    fn read_rejects_non_ascending_dkg_logs() {
        // A non-canonical dkg_logs (idx not strictly ascending, or a dup) must be
        // rejected at decode so the digest is unambiguous.
        let mut buf = Vec::new();
        let b = sample_order_block();
        write_header_prefix(&mut buf, &b);
        (b.extra_data.len() as u32).write(&mut buf);
        buf.extend_from_slice(&b.extra_data);
        {
            use alloy_rlp::Encodable as _;
            b.txs.encode(&mut buf);
        }
        8u8.write(&mut buf); // beacon_flags: dkg_logs present (bit3)
        2u32.write(&mut buf); // count = 2
        1u8.write(&mut buf); // idx 1
        buf.extend_from_slice(B256::repeat_byte(0x11).as_slice());
        1u8.write(&mut buf); // idx 1 again — NOT strictly ascending
        buf.extend_from_slice(B256::repeat_byte(0x22).as_slice());
        let err = OrderBlock::read(&mut buf.as_slice()).expect_err("non-ascending dkg_logs");
        assert!(matches!(err, commonware_codec::Error::Invalid(_, m) if m.contains("ascending")));
    }

    #[test]
    fn read_rejects_dkg_logs_idx_beyond_max_committee_size() {
        // The COUNT bound does not bound the VALUE: idx is a u8, so a single entry can
        // name participant 200 of a 51-seat committee. Such an entry survives the vote
        // gate whenever the committee is unreadable (accept-biased) and then reaches the
        // ceremony, which can never satisfy it. Rejecting at decode against the
        // network-wide constant is node-local-state-free and therefore identical on
        // every node.
        let max = fluentbase_p2p::constants::MAX_COMMITTEE_SIZE as u8;
        for bad_idx in [max, 255u8] {
            let mut buf = Vec::new();
            let b = sample_order_block();
            write_header_prefix(&mut buf, &b);
            (b.extra_data.len() as u32).write(&mut buf);
            buf.extend_from_slice(&b.extra_data);
            {
                use alloy_rlp::Encodable as _;
                b.txs.encode(&mut buf);
            }
            8u8.write(&mut buf); // bit3
            1u32.write(&mut buf); // count = 1
            bad_idx.write(&mut buf);
            buf.extend_from_slice(B256::repeat_byte(0x33).as_slice());
            let err = OrderBlock::read(&mut buf.as_slice()).expect_err("out-of-range dkg_logs idx");
            assert!(
                matches!(err, commonware_codec::Error::Invalid(_, m) if m.contains("idx exceeds")),
                "idx {bad_idx} must be rejected, got {err:?}"
            );
        }
    }

    #[test]
    fn read_accepts_dkg_logs_idx_at_the_last_committee_position() {
        // The bound is exclusive: the last legal position must still round-trip.
        let mut original = sample_order_block();
        let last = (fluentbase_p2p::constants::MAX_COMMITTEE_SIZE - 1) as u8;
        original.dkg_logs = vec![(last, B256::repeat_byte(0x44))];
        let encoded = original.encode();
        let decoded = OrderBlock::read(&mut encoded.as_ref()).expect("round-trip");
        assert_eq!(decoded.dkg_logs, original.dkg_logs);
    }

    #[test]
    fn read_rejects_oversize_dkg_logs() {
        // A dkg_logs count above MAX_COMMITTEE_SIZE is rejected before allocation.
        let mut buf = Vec::new();
        let b = sample_order_block();
        write_header_prefix(&mut buf, &b);
        (b.extra_data.len() as u32).write(&mut buf);
        buf.extend_from_slice(&b.extra_data);
        {
            use alloy_rlp::Encodable as _;
            b.txs.encode(&mut buf);
        }
        8u8.write(&mut buf); // bit3
        ((fluentbase_p2p::constants::MAX_COMMITTEE_SIZE as u32) + 1).write(&mut buf);
        let err = OrderBlock::read(&mut buf.as_slice()).expect_err("oversize dkg_logs");
        assert!(
            matches!(err, commonware_codec::Error::Invalid(_, m) if m.contains("MAX_COMMITTEE_SIZE"))
        );
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
                beacon_outcome: Some(Bytes::from(vec![9u8; 48])),
                ..base.clone()
            },
            OrderBlock {
                dkg_logs: vec![(0, B256::repeat_byte(0x5A))],
                ..base.clone()
            },
            OrderBlock {
                proposal_view: base.proposal_view + 1,
                ..base.clone()
            },
            OrderBlock {
                parent_seed: Some(sample_seed()),
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
    fn read_rejects_oversize_beacon_outcome() {
        // Hand-encode a block with the outcome flag set and an oversize length prefix.
        let mut buf = Vec::new();
        let b = sample_order_block();
        write_header_prefix(&mut buf, &b);
        (b.extra_data.len() as u32).write(&mut buf);
        buf.extend_from_slice(&b.extra_data);
        {
            use alloy_rlp::Encodable as _;
            b.txs.encode(&mut buf);
        }
        1u8.write(&mut buf); // beacon_flags: outcome present
        ((MAX_BEACON_OUTCOME_SIZE + 1) as u32).write(&mut buf);
        buf.resize(buf.len() + MAX_BEACON_OUTCOME_SIZE + 1, 0);

        let err = OrderBlock::read(&mut buf.as_slice()).expect_err("oversize beacon_outcome");
        assert!(matches!(err, commonware_codec::Error::Invalid(_, _)));
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

    /// Hand-encode everything up to and including the beacon_flags byte, with
    /// ONLY bit1 (parent_seed present) set — the seed bytes themselves are the
    /// test's variable.
    fn write_bit1_frame_prefix(buf: &mut Vec<u8>, b: &OrderBlock) {
        write_header_prefix(buf, b);
        (b.extra_data.len() as u32).write(buf);
        buf.extend_from_slice(&b.extra_data);
        {
            use alloy_rlp::Encodable as _;
            b.txs.encode(buf);
        }
        2u8.write(buf); // beacon_flags: parent_seed present, no outcome
    }

    #[test]
    fn read_bit1_set_but_truncated_seed_is_end_of_buffer() {
        // The seed flag promises a trailing Seed; a frame that ends before the
        // seed completes must fail as a short buffer, not decode a garbage seed.
        let b = sample_order_block();
        let seed_bytes = sample_seed().encode();
        for keep in [0, 1, seed_bytes.len() - 1] {
            let mut buf = Vec::new();
            write_bit1_frame_prefix(&mut buf, &b);
            buf.extend_from_slice(&seed_bytes[..keep]);
            let err = OrderBlock::read(&mut buf.as_slice()).expect_err("truncated seed");
            assert!(
                matches!(err, commonware_codec::Error::EndOfBuffer),
                "expected EndOfBuffer with {keep} seed bytes, got {err:?}"
            );
        }
    }

    #[test]
    fn read_consumes_exactly_the_seed_bytes_of_a_bit1_frame() {
        // Hand-encoded (NOT via `Write`) so this pins the wire layout itself:
        // a bit1 frame's trailer is exactly the Seed encoding — no length
        // prefix, no padding — and the decoder must stop precisely at its end.
        let b = sample_order_block();
        let seed = sample_seed();
        let mut buf = Vec::new();
        write_bit1_frame_prefix(&mut buf, &b);
        let before_seed = buf.len();
        seed.write(&mut buf);
        assert_eq!(buf.len() - before_seed, seed.encode_size());

        let mut slice = buf.as_slice();
        let decoded = OrderBlock::read(&mut slice).expect("decode bit1 frame");
        assert!(
            slice.is_empty(),
            "decoder must consume the exact seed bytes"
        );
        assert_eq!(decoded.parent_seed, Some(seed));
    }

    #[test]
    fn read_rejects_combined_oversize_even_when_each_field_is_within_its_cap() {
        use alloy_primitives::{Signature, TxKind};
        use reth_ethereum_primitives::Transaction;
        // One EIP-1559 tx whose calldata makes the tx list a bit under the 4 MiB
        // per-field cap — fine on its own — but combined with a max-size (64 KiB)
        // beacon_outcome the composed artifact exceeds MAX_ORDER_BLOCK_SIZE. Each
        // field passes its OWN decode cap; only the combined gate (bug 1) rejects.
        let tx = alloy_consensus::TxEip1559 {
            chain_id: 1,
            nonce: 0,
            gas_limit: 21_000,
            max_fee_per_gas: 0,
            max_priority_fee_per_gas: 0,
            to: TxKind::Call(Address::ZERO),
            value: U256::ZERO,
            input: Bytes::from(vec![0u8; 4_140_000]),
            ..Default::default()
        };
        let sig = Signature::new(U256::from(1u64), U256::from(1u64), false);
        let signed = TransactionSigned::new_unhashed(Transaction::Eip1559(tx), sig);

        let mut block = sample_order_block();
        block.txs = vec![signed];
        block.beacon_outcome = Some(Bytes::from(vec![0u8; MAX_BEACON_OUTCOME_SIZE]));
        block.parent_seed = Some(sample_seed());
        assert!(
            block.encode_size() > MAX_ORDER_BLOCK_SIZE,
            "the composed block must be over the cap"
        );

        let encoded = block.encode();
        let err = OrderBlock::read(&mut encoded.as_ref())
            .expect_err("combined-oversize block must be rejected at decode");
        // The COMBINED gate must fire (not the per-field tx cap) — else the test
        // would pass on the pre-fix code too.
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
