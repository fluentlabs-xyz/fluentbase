//! `CertifiedBlock` — the wire unit a cert-follower pulls from an upstream and
//! verifies: a finalized ORDERING artifact paired with its consensus
//! finalization certificate.
//!
//! Mirrors tempo's `consensus_getFinalization` DTO. Both the certificate and the
//! block ride as hex: the certificate is the commonware `Finalization` codec, the
//! block is fluentbase's commonware-codec `OrderBlock`.
//!
//! `epoch`/`view`/`digest` are informational (debug + by-height indexing). The
//! follower's trust comes from decoding and verifying `certificate`, never from
//! these fields.

use alloy_primitives::B256;
use commonware_codec::{Decode as _, DecodeExt as _, Encode as _};
use commonware_consensus::simplex::types::Finalization;
use eyre::WrapErr as _;
use fluentbase_bls::Scheme as BlsScheme;
use fluentbase_consensus::{Digest, OrderBlock};
use serde::{Deserialize, Serialize};

/// The finalization certificate paired with the block it finalizes.
type Cert = Finalization<BlsScheme, Digest>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CertifiedBlock {
    pub height: u64,
    pub epoch: u64,
    pub view: u64,
    pub digest: B256,
    /// Hex-encoded `Finalization<BlsScheme, Digest>` (commonware codec).
    pub certificate: String,
    /// Hex-encoded `OrderBlock` (commonware codec).
    pub block: String,
}

impl CertifiedBlock {
    /// Build from the archive's typed `(cert, block)` (server side).
    pub fn from_parts(cert: &Cert, block: &OrderBlock) -> Self {
        Self {
            height: block.height,
            epoch: cert.proposal.round.epoch().get(),
            view: cert.proposal.round.view().get(),
            digest: cert.proposal.payload.0,
            certificate: hex::encode(cert.encode()),
            block: hex::encode(block.encode()),
        }
    }

    /// Decode back to the marshal-format typed pair (client side).
    ///
    /// The certificate signer-bitmap is decoded with a BOUNDED cap
    /// (`MAX_COMMITTEE_SIZE`) — NOT the unbounded `u32::MAX` config the marshal's
    /// trusted archive uses. This data comes from an untrusted upstream over WS;
    /// the unbounded decoder eagerly allocates `VecDeque::with_capacity(num_chunks)`
    /// from a ~9-byte length prefix, so a tiny malicious certificate could force a
    /// ~512 MB allocation and OOM the follower (audit R4-5). A real finalization
    /// has ≤ the committee size (≤ `MAX_COMMITTEE_SIZE`) signers; exact participant
    /// validation still happens at cert-verify time against the per-epoch scheme.
    pub fn into_parts(&self) -> eyre::Result<(Cert, OrderBlock)> {
        let cert_bytes = hex::decode(&self.certificate).wrap_err("decode certificate hex")?;
        let cert = Cert::decode_cfg(
            cert_bytes.as_slice(),
            &(fluentbase_p2p::constants::MAX_COMMITTEE_SIZE as usize),
        )
        .wrap_err("decode finalization certificate")?;
        let block_bytes = hex::decode(&self.block).wrap_err("decode block hex")?;
        let block = OrderBlock::decode(block_bytes.as_slice()).wrap_err("decode block")?;
        Ok((cert, block))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> CertifiedBlock {
        CertifiedBlock {
            height: 42,
            epoch: 1,
            view: 7,
            digest: B256::repeat_byte(0xab),
            certificate: "00ff".into(),
            block: "deadbeef".into(),
        }
    }

    /// The follower and server agree on the JSON wire contract: camelCase keys and
    /// a lossless round-trip. (Full cert decode+verify is exercised end-to-end in the
    /// `case-cert-follow.sh` smoke against a running chain — a valid multisig
    /// `Finalization` fixture has no fluentbase-side constructor.)
    #[test]
    fn dto_serde_round_trip_is_camel_case() {
        let original = sample();
        let json = serde_json::to_string(&original).expect("serialize");
        assert!(json.contains("\"height\""));
        assert!(json.contains("\"certificate\""));
        let decoded: CertifiedBlock = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(original, decoded);
    }

    #[test]
    fn into_parts_rejects_malformed_certificate_hex() {
        let bad = CertifiedBlock {
            certificate: "nothex".into(),
            ..sample()
        };
        assert!(bad.into_parts().is_err());
    }
}
