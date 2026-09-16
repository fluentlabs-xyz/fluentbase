//! The at-rest seal key for per-epoch DKG shares, derived from the validator BLS
//! secret.
//!
//! [`ShareSealKey`] is a 32-byte symmetric key derived once at launch via
//! HKDF-SHA256 from the in-memory validator BLS signing scalar (the IKM). The
//! consumer (`consensus/beacon/share_state.rs`) uses it as an XChaCha20-Poly1305
//! AEAD key to seal and open the per-epoch share file.
//!
//! The IKM never crosses the crate boundary: HKDF runs inside
//! [`crate::keys::ValidatorBlsKeypair::derive_share_seal_key`] over the exposed
//! secret and only the derived key leaves. Deriving an encryption key from a
//! signing scalar is sound only with domain separation, which the mandatory
//! [`SHARE_AT_REST_INFO`] HKDF `info` label provides (RFC 5869).

use zeroize::Zeroizing;

/// HKDF `info` label — the mandatory domain-separation context that makes reusing
/// the validator BLS scalar as HKDF IKM sound (RFC 5869 §3.2). Never empty, never
/// shared with any other HKDF use of the same IKM; the `_v1` suffix lets the
/// scheme rotate to `_v2` without changing the IKM.
pub const SHARE_AT_REST_INFO: &[u8] = b"FLUENT_DPOS_V1_SHARE_AT_REST_v1";

/// A 32-byte HKDF-derived symmetric key sealing per-epoch DKG shares at rest,
/// zeroized on drop. It is strictly downstream of the validator secret it is
/// derived from, so it adds no new long-lived secret exposure.
#[derive(Clone)]
pub struct ShareSealKey(Zeroizing<[u8; 32]>);

impl ShareSealKey {
    pub(crate) fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    /// The raw 32-byte AEAD key for the share codec's seal/open — the derived key,
    /// not the validator scalar, which never leaves this crate.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl core::fmt::Debug for ShareSealKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ShareSealKey(<redacted>)")
    }
}
