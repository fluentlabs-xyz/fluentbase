//! The F-type consensus unit: ordering only — txs + parent digest + result
//! commitment. The digest deliberately excludes every execution output of
//! THIS block; `result` commits the derived block hash K heights back, so
//! agreeing OrderBlock N+K is the committee's attestation of block N's
//! execution result.

use crate::digest::Digest;
use alloy_primitives::{keccak256, Address, Bytes, B256};
use bytes::{Buf, BufMut};
use commonware_codec::{Encode as _, EncodeSize, Read, Write};
use commonware_consensus::{types::Height, Heightable};
use commonware_cryptography::{Committable, Digestible};
use reth_ethereum_primitives::TransactionSigned;
use reth_primitives_traits::SealedBlock;

/// Result lag in blocks (Monad D=3). Consensus-critical: MUST be
/// byte-identical across nodes (same class as MAX_MESSAGE_SIZE, G11).
/// Changing it is a chain-spec release, not a config knob.
pub const K: u64 = 3;

/// Per-artifact decode cap (defense-in-depth + channel-specific bound) —
/// same wire budget as the executed-block era: 50M gas / 16 B-per-calldata
/// ≈ 3.125 MB worst case + ~30% headroom. Coupled to but independent from
/// `fluentbase_p2p::constants::MAX_MESSAGE_SIZE`.
pub const MAX_ORDER_BLOCK_SIZE: usize = 4 * 1024 * 1024;

/// Tx-list byte budget for ordering assembly: [`MAX_ORDER_BLOCK_SIZE`] minus
/// a fixed 4 KiB allowance for the non-tx fields (parent/height/result,
/// extra_data — committee-size-bounded bitmap, codec framing), so an
/// assembled artifact always fits its own decode cap.
pub const TX_BYTE_BUDGET: usize = MAX_ORDER_BLOCK_SIZE - 4 * 1024;

/// Decode cap for `extra_data` (the simplex liveness attestation: round +
/// u8-capped committee bitmap ≪ 1 KiB; cap matches the TX_BYTE_BUDGET
/// allowance so the two bounds compose).
const MAX_EXTRA_DATA_SIZE: usize = 4 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderBlock {
    /// Digest of OrderBlock N−1 (the ordering chain, NOT the EVM parent hash).
    pub parent: Digest,
    pub height: u64,
    /// Proposer-chosen; becomes the derived header timestamp. Verified
    /// strictly monotonic vs the parent (no wall-clock check — verify must be
    /// a pure function of agreed state).
    pub timestamp: u64,
    /// Proposer's fee recipient — derived header beneficiary.
    pub fee_recipient: Address,
    /// Derived block gas limit. Agreed data, NOT local config (a local
    /// `--builder.gaslimit` would diverge derived blocks across nodes).
    /// Verified within the EIP-1559 ±1/1024 bound vs the parent.
    pub gas_limit: u64,
    /// Simplex liveness attestation (same wire format as the executed-block
    /// era header extra_data); copied verbatim into the derived EVM header so
    /// the on-chain `processBitmap` path is unchanged.
    pub extra_data: Bytes,
    /// EVM hash of the DERIVED block at `height − K`; `B256::ZERO` while
    /// `height < anchor + K` (see [`result_target`]); the anchor EVM hash in
    /// the genesis/anchor artifact (binds the ordering chain to the EVM
    /// chain).
    pub result: B256,
    /// Ordered raw transactions.
    pub txs: Vec<TransactionSigned>,
}

impl OrderBlock {
    /// keccak256 over the canonical codec encoding — the consensus identity.
    pub fn digest(&self) -> Digest {
        Digest(keccak256(self.encode()))
    }

    pub fn parent_digest(&self) -> Digest {
        self.parent
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

/// The ordering-chain anchor for an EVM anchor block: empty tx list,
/// `result` = the anchor's EVM hash, parent = EMPTY. Deterministic across
/// nodes given the same anchor (devnet genesis / migration weak-subjectivity
/// checkpoint).
pub fn anchor_order_block(anchor: &SealedBlock<reth_ethereum_primitives::Block>) -> OrderBlock {
    use alloy_consensus::BlockHeader as _;
    OrderBlock {
        parent: Digest(B256::ZERO),
        height: anchor.number(),
        timestamp: anchor.timestamp(),
        fee_recipient: Address::ZERO,
        // Seeds the EIP-1559 ±1/1024 gas-limit progression of the ordering chain.
        gas_limit: anchor.gas_limit(),
        extra_data: Bytes::new(),
        result: anchor.hash(),
        txs: Vec::new(),
    }
}

// Wire format (all integers big-endian via commonware primitives):
//   parent(32) ‖ height(8) ‖ timestamp(8) ‖ fee_recipient(20) ‖ gas_limit(8)
//   ‖ result(32) ‖ extra_data_len(4)+bytes ‖ txs as one RLP list.
// The RLP tx list reuses alloy's canonical encoding so tx bytes are identical
// to their EVM-block representation.

impl Write for OrderBlock {
    fn write(&self, buf: &mut impl BufMut) {
        use alloy_rlp::Encodable as _;
        self.parent.write(buf);
        self.height.write(buf);
        self.timestamp.write(buf);
        buf.put_slice(self.fee_recipient.as_slice());
        self.gas_limit.write(buf);
        buf.put_slice(self.result.as_slice());
        (self.extra_data.len() as u32).write(buf);
        buf.put_slice(&self.extra_data);
        self.txs.encode(buf);
    }
}

impl EncodeSize for OrderBlock {
    fn encode_size(&self) -> usize {
        use alloy_rlp::Encodable as _;
        32 + 8 + 8 + 20 + 8 + 32 + 4 + self.extra_data.len() + self.txs.length()
    }
}

impl Read for OrderBlock {
    type Cfg = ();

    fn read_cfg(buf: &mut impl Buf, _cfg: &Self::Cfg) -> Result<Self, commonware_codec::Error> {
        let parent = Digest::read_cfg(buf, &())?;
        let height = u64::read_cfg(buf, &())?;
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
        Ok(Self {
            parent,
            height,
            timestamp,
            fee_recipient,
            gas_limit,
            extra_data,
            result,
            txs,
        })
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
            timestamp: 1_700_000_000,
            fee_recipient: Address::repeat_byte(0x22),
            gas_limit: 50_000_000,
            extra_data: Bytes::from(vec![1u8, 2, 3]),
            result: B256::repeat_byte(0x33),
            txs: Vec::new(),
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
        ];
        for m in mutations {
            assert_ne!(m.digest(), d);
        }
    }

    #[test]
    fn read_rejects_oversize_extra_data() {
        let mut buf = Vec::new();
        let b = sample_order_block();
        b.parent.write(&mut buf);
        b.height.write(&mut buf);
        b.timestamp.write(&mut buf);
        buf.extend_from_slice(b.fee_recipient.as_slice());
        b.gas_limit.write(&mut buf);
        buf.extend_from_slice(b.result.as_slice());
        ((MAX_EXTRA_DATA_SIZE + 1) as u32).write(&mut buf);
        buf.resize(buf.len() + MAX_EXTRA_DATA_SIZE + 1, 0);

        let err = OrderBlock::read(&mut buf.as_slice()).expect_err("oversize extra_data");
        assert!(matches!(err, commonware_codec::Error::Invalid(_, _)));
    }

    #[test]
    fn read_rejects_oversize_tx_list() {
        let b = sample_order_block();
        let mut buf = Vec::new();
        b.parent.write(&mut buf);
        b.height.write(&mut buf);
        b.timestamp.write(&mut buf);
        buf.extend_from_slice(b.fee_recipient.as_slice());
        b.gas_limit.write(&mut buf);
        buf.extend_from_slice(b.result.as_slice());
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

        let anchor = anchor_order_block(&sealed);
        assert_eq!(anchor.result, sealed.hash());
        assert_eq!(anchor.height, 6_700_000);
        assert_eq!(anchor.gas_limit, 50_000_000);
        assert!(anchor.txs.is_empty());

        // Deterministic across construction sites: identity = digest equality.
        assert_eq!(anchor.digest(), anchor_order_block(&sealed).digest());
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
