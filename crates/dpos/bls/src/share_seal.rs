//! The at-rest seal key for per-epoch DKG shares (E2 — derived from the
//! validator BLS secret).
//!
//! [`ShareSealKey`] is a 32-byte symmetric key derived ONCE at launch via
//! HKDF-SHA256 from the in-memory validator BLS signing scalar (the IKM). The
//! consumer (`consensus/beacon/share_state.rs`) uses it as an XChaCha20-Poly1305
//! AEAD key to seal/open the per-epoch share file.
//!
//! The IKM (raw scalar) never crosses the crate boundary: HKDF runs inside
//! [`crate::keys::ValidatorBlsKeypair::derive_share_seal_key`] over the exposed
//! secret, and only the derived 32-byte [`ShareSealKey`] leaves. Deriving an
//! encryption key from a (signing) scalar is sound ONLY with domain separation:
//! the mandatory [`SHARE_AT_REST_INFO`] HKDF `info` label provides it (RFC 5869),
//! making the AEAD key provably independent of the scalar's signing use.

use zeroize::Zeroizing;

/// HKDF `info` label — the MANDATORY domain-separation context that makes
/// reusing the validator BLS scalar as HKDF IKM cryptographically sound (RFC
/// 5869 §3.2). Never empty, never shared with any other HKDF use of the same
/// IKM. The `_v1` suffix lets the AEAD/KDF scheme rotate to `_v2` (a fresh
/// independent key) without changing the IKM.
pub const SHARE_AT_REST_INFO: &[u8] = b"FLUENT_DPOS_V1_SHARE_AT_REST_v1";

/// A 32-byte HKDF-derived symmetric key sealing per-epoch DKG shares at rest.
/// Zeroized on drop. Strictly downstream of the validator secret it is derived
/// from (which is already alive whole-session), so it adds no new long-lived
/// secret exposure beyond that scalar.
#[derive(Clone)]
pub struct ShareSealKey(Zeroizing<[u8; 32]>);

impl ShareSealKey {
    pub(crate) fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    /// The raw 32-byte AEAD key, for the share codec's seal/open. This is the
    /// DERIVED key (already past the crate boundary by design), NOT the validator
    /// scalar — that IKM never leaves the bls crate.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl core::fmt::Debug for ShareSealKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ShareSealKey(<redacted>)")
    }
}
