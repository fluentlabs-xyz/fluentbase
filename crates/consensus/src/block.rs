//! Fluent consensus block: thin wrapper around a reth `SealedBlock`.
//!
//! The block hash IS the consensus digest — no separate hashing.
//!
//! Intentionally omits a `CertifiableBlock` impl: that impl reads
//! `block.consensus_context`, a reth-fork header extension Fluent does
//! not have. The marshal `Inline` wrapper only needs the `Block` trait
//! (inline.rs:128-130).

use alloy_consensus::BlockHeader as _;
use alloy_primitives::B256;
use bytes::{Buf, BufMut};
use commonware_codec::{EncodeSize, Read, Write};
use commonware_consensus::{types::Height, Heightable};
use commonware_cryptography::{Committable, Digestible};
use reth_ethereum_primitives::Block as RethBlock;
use reth_primitives_traits::SealedBlock;

use crate::digest::Digest;

/// Per-block RLP decode cap (defense-in-depth + channel-specific bound).
///
/// Coupled to but independent from `fluentbase_p2p::constants::MAX_MESSAGE_SIZE`:
/// the p2p layer caps wire frames at 4 MiB globally for VOTE/CERT/RESOLVER/
/// BROADCAST/MARSHAL; blocks only flow over BROADCAST. A future bump of
/// `MAX_MESSAGE_SIZE` (e.g. to support larger control messages) must NOT
/// silently re-cap block ingress.
///
/// At 50M gas / 16 B-per-calldata-byte ≈ 3.125 MB worst-case calldata-heavy
/// block + ~30% headroom = 4 MiB.
const MAX_BLOCK_SIZE: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct Block(SealedBlock<RethBlock>);

impl Block {
    pub fn from_execution_block(block: SealedBlock<RethBlock>) -> Self {
        Self(block)
    }

    pub fn into_inner(self) -> SealedBlock<RethBlock> {
        self.0
    }

    pub fn block_hash(&self) -> B256 {
        self.0.hash()
    }

    pub fn digest(&self) -> Digest {
        Digest(self.0.hash())
    }

    pub fn parent_digest(&self) -> Digest {
        Digest(self.0.parent_hash())
    }

    pub fn timestamp(&self) -> u64 {
        self.0.timestamp()
    }
}

impl std::ops::Deref for Block {
    type Target = SealedBlock<RethBlock>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Write for Block {
    fn write(&self, buf: &mut impl BufMut) {
        use alloy_rlp::Encodable as _;
        self.0.encode(buf);
    }
}

impl Read for Block {
    type Cfg = ();

    fn read_cfg(buf: &mut impl Buf, _cfg: &Self::Cfg) -> Result<Self, commonware_codec::Error> {
        // NOTE: `buf.chunk()` is only guaranteed to return *a* contiguous slice,
        // not all of `remaining()`. This relies on the caller delivering a
        // contiguous `Bytes` (commonware p2p does); for a segmented `Buf` whose
        // RLP length prefix straddles a chunk boundary, `Header::decode` could
        // under-read. Safe under the current p2p transport; documented so a
        // future non-contiguous `Buf` source is caught here.
        let header = alloy_rlp::Header::decode(&mut buf.chunk())
            .map_err(|e| commonware_codec::Error::Wrapped("reading RLP header", e.into()))?;
        if header.length_with_payload() > MAX_BLOCK_SIZE {
            return Err(commonware_codec::Error::Invalid(
                "block",
                "exceeds MAX_BLOCK_SIZE",
            ));
        }
        if header.length_with_payload() > buf.remaining() {
            return Err(commonware_codec::Error::EndOfBuffer);
        }
        let bytes = buf.copy_to_bytes(header.length_with_payload());
        let inner = alloy_rlp::Decodable::decode(&mut bytes.as_ref())
            .map_err(|e| commonware_codec::Error::Wrapped("reading RLP encoded block", e.into()))?;
        Ok(Self::from_execution_block(inner))
    }
}

impl EncodeSize for Block {
    fn encode_size(&self) -> usize {
        use alloy_rlp::Encodable as _;
        self.0.length()
    }
}

impl Committable for Block {
    type Commitment = Digest;

    fn commitment(&self) -> Self::Commitment {
        self.digest()
    }
}

impl Digestible for Block {
    type Digest = Digest;

    fn digest(&self) -> Self::Digest {
        self.digest()
    }
}

impl Heightable for Block {
    fn height(&self) -> Height {
        Height::new(self.0.number())
    }
}

impl commonware_consensus::Block for Block {
    fn parent(&self) -> Digest {
        self.parent_digest()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::{Block as AlloyBlock, BlockBody, Header};
    use alloy_primitives::U256;
    use commonware_codec::{Encode as _, ReadExt as _};
    use reth_ethereum_primitives::TransactionSigned;

    fn sample_block() -> Block {
        let header = Header {
            parent_hash: B256::repeat_byte(0x11),
            number: 42,
            gas_limit: 30_000_000,
            gas_used: 0,
            timestamp: 1_700_000_000,
            difficulty: U256::ZERO,
            ..Default::default()
        };
        let body: BlockBody<TransactionSigned> = BlockBody::default();
        let alloy_block: AlloyBlock<TransactionSigned> = AlloyBlock::new(header, body);
        let reth_block = RethBlock::from(alloy_block);
        Block::from_execution_block(SealedBlock::seal_slow(reth_block))
    }

    #[test]
    fn codec_round_trip() {
        let original = sample_block();
        let encoded = original.encode();
        let decoded = Block::read(&mut encoded.as_ref()).expect("decode");
        assert_eq!(original, decoded);
    }

    #[test]
    fn encode_size_matches_write() {
        let block = sample_block();
        let encoded = block.encode();
        assert_eq!(block.encode_size(), encoded.len());
    }

    #[test]
    fn digest_matches_block_hash() {
        let block = sample_block();
        assert_eq!(block.digest().0, block.block_hash());
    }

    #[test]
    fn height_matches_inner_number() {
        let block = sample_block();
        assert_eq!(<Block as Heightable>::height(&block), Height::new(42));
    }

    #[test]
    fn read_rejects_block_exceeding_max_size() {
        // Forge an RLP header claiming payload_length > MAX_BLOCK_SIZE.
        // Per alloy_rlp::Header::decode → length_with_payload, we compute the
        // claimed total = header_encoding_bytes + payload_length, which must
        // exceed MAX_BLOCK_SIZE for the cap to fire.
        let oversize_payload = MAX_BLOCK_SIZE + 1;
        let header = alloy_rlp::Header {
            list: true,
            payload_length: oversize_payload,
        };
        let mut buf = Vec::new();
        header.encode(&mut buf);
        // Pad the buffer enough that buf.remaining() ≥ length_with_payload, so
        // the MAX_BLOCK_SIZE check is reached BEFORE the EndOfBuffer check.
        buf.resize(buf.len() + oversize_payload, 0u8);

        let err = Block::read(&mut buf.as_slice()).expect_err("should reject oversize block");
        match err {
            commonware_codec::Error::Invalid(what, why) => {
                assert_eq!(what, "block");
                assert_eq!(why, "exceeds MAX_BLOCK_SIZE");
            }
            other => panic!("expected Error::Invalid, got {other:?}"),
        }
    }
}
