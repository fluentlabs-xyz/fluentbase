//! `CertifiedBlock` — the wire unit a cert-follower pulls from an upstream and
//! verifies: a finalized block paired with its consensus finalization certificate.
//!
//! Mirrors tempo's `consensus_getFinalization` DTO. Both the certificate and the
//! block ride as hex: the certificate is the commonware `Finalization` codec, the
//! block is fluentbase's RLP/commonware-codec `Block` (which, unlike tempo's
//! serde-native block, is not serde — so it is hex-encoded too).
//!
//! `epoch`/`view`/`digest` are informational (debug + by-height indexing). The
//! follower's trust comes from decoding and verifying `certificate`, never from
//! these fields.

use alloy_primitives::B256;
use commonware_codec::{Decode as _, DecodeExt as _, Encode as _};
use commonware_consensus::{simplex::types::Finalization, Heightable as _};
use commonware_cryptography::certificate::Scheme as _;
use eyre::WrapErr as _;
use fluentbase_bls::Scheme as BlsScheme;
use fluentbase_consensus::{Block, Digest};
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
    /// Hex-encoded `Block` (RLP via commonware codec).
    pub block: String,
}

impl CertifiedBlock {
    /// Build from the archive's typed `(cert, block)` (server side).
    pub fn from_parts(cert: &Cert, block: &Block) -> Self {
        Self {
            height: block.height().get(),
            epoch: cert.proposal.round.epoch().get(),
            view: cert.proposal.round.view().get(),
            digest: cert.proposal.payload.0,
            certificate: hex::encode(cert.encode()),
            block: hex::encode(block.encode()),
        }
    }

    /// Decode back to the marshal-format typed pair (client side).
    ///
    /// The certificate is decoded with the UNBOUNDED cert codec config — the same
    /// the marshal's archive uses ([`outer.rs`] `certificate_codec_config_unbounded`).
    /// The per-epoch scheme is required only later, for verification, not for decode.
    pub fn into_parts(&self) -> eyre::Result<(Cert, Block)> {
        let cert_bytes = hex::decode(&self.certificate).wrap_err("decode certificate hex")?;
        let cert = Cert::decode_cfg(
            cert_bytes.as_slice(),
            &BlsScheme::certificate_codec_config_unbounded(),
        )
        .wrap_err("decode finalization certificate")?;
        let block_bytes = hex::decode(&self.block).wrap_err("decode block hex")?;
        let block = Block::decode(block_bytes.as_slice()).wrap_err("decode block")?;
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
