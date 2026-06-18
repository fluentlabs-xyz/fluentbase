//! EIP-2335 BLS12-381 keystore (version 4).
//!
//! Wire format: serde-deserialized JSON conforming to
//! <https://eips.ethereum.org/EIPS/eip-2335>. Pinned by
//! `crates/bls/tests/eip2335_conformance_vectors.rs` against the EIP-2335
//! Appendix A reference vectors.
//!
//! Decryption flow:
//! 1. Normalize the password to NFKD UTF-8 and strip C0/C1 control codes
//!    (EIP-2335 §6.1), via `normalize_password`. Non-UTF-8 input is passed
//!    through unchanged.
//! 2. KDF (scrypt or PBKDF2-HMAC-SHA256) → 32-byte derivation key DK.
//! 3. Verify checksum: `SHA256(DK[16..32] || cipher.message) == checksum.message`.
//! 4. AES-128-CTR decrypt with key=DK[0..16], iv=cipher.params.iv → secret bytes.
//!
//! Only the decrypt path is implemented; encrypt (keygen-CLI) is intentionally
//! deferred. Adding it later must re-introduce serde::Serialize alongside an
//! `encrypt()` constructor that gates KDF parameter selection.

use aes::cipher::{KeyIvInit, StreamCipher};
use hmac::Hmac;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;
use zeroize::Zeroizing;

use crate::{error::Error, SECRET_BYTES};

type Aes128Ctr = ctr::Ctr64BE<aes::Aes128>;

/// EIP-2335 §6.1 password normalization: NFKD UTF-8 with C0/C1
/// control codes stripped. Callers in the integration path pass UTF-8
/// password bytes; this helper normalizes them per spec so a password
/// written with mathematical-fraktur (𝔱) decodes identically to its
/// ASCII NFKD form (t).
fn normalize_password(raw: &[u8]) -> Zeroizing<Vec<u8>> {
    let Ok(s) = std::str::from_utf8(raw) else {
        return Zeroizing::new(raw.to_vec());
    };
    let normalized: String = s
        .nfkd()
        .filter(|c| {
            let cp = *c as u32;
            // Strip C0 (U+0000..U+001F) + DEL (U+007F) + C1 (U+0080..U+009F).
            !(cp <= 0x1F || cp == 0x7F || (0x80..=0x9F).contains(&cp))
        })
        .collect();
    Zeroizing::new(normalized.into_bytes())
}

/// Top-level EIP-2335 keystore JSON. `version` MUST be 4.
#[derive(Deserialize)]
pub struct EthKeystoreV4 {
    pub version: u8,
    pub uuid: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub pubkey: String,
    pub crypto: CryptoSection,
}

#[derive(Deserialize)]
pub struct CryptoSection {
    pub kdf: ModuleSection<KdfParams>,
    pub checksum: ModuleSection<ChecksumParams>,
    pub cipher: ModuleSection<CipherParams>,
}

#[derive(Deserialize)]
pub struct ModuleSection<P> {
    pub function: String,
    pub params: P,
    #[serde(with = "hex_string")]
    pub message: Vec<u8>,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum KdfParams {
    Scrypt {
        dklen: u32,
        n: u32,
        p: u32,
        r: u32,
        #[serde(with = "hex_string")]
        salt: Vec<u8>,
    },
    Pbkdf2 {
        dklen: u32,
        c: u32,
        prf: String,
        #[serde(with = "hex_string")]
        salt: Vec<u8>,
    },
}

#[derive(Deserialize)]
pub struct ChecksumParams {}

#[derive(Deserialize)]
pub struct CipherParams {
    #[serde(with = "hex_string")]
    pub iv: Vec<u8>,
}

mod hex_string {
    use serde::{de, Deserialize, Deserializer};

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let s: &str = Deserialize::deserialize(deserializer)?;
        let s = s.strip_prefix("0x").unwrap_or(s);
        hex::decode(s).map_err(de::Error::custom)
    }
}

impl EthKeystoreV4 {
    /// Parse and validate the wire format. Rejects anything other than version 4.
    pub fn from_json(raw: &str) -> Result<Self, Error> {
        let ks: EthKeystoreV4 = serde_json::from_str(raw).map_err(|_| Error::InvalidKeystore)?;
        if ks.version != 4 {
            return Err(Error::InvalidKeystore);
        }
        Ok(ks)
    }

    /// Decrypt the 32-byte BLS scalar. Caller wraps in
    /// [`crate::keys::ValidatorBlsKeypair::from_secret_bytes`].
    pub fn decrypt(&self, password: &[u8]) -> Result<Zeroizing<[u8; SECRET_BYTES]>, Error> {
        let pw = normalize_password(password);
        let dk = Zeroizing::new(self.derive_kdf_key(&pw)?);

        // EIP-2335 §5 fixes the derived key at 32 bytes: DK[0..16] is the AES
        // key, DK[16..32] feeds the checksum. A malformed/corrupt keystore can
        // declare a shorter `dklen`; reject it cleanly instead of panicking on
        // the slices below.
        if dk.len() < 32 {
            return Err(Error::InvalidKeystore);
        }

        // Checksum: SHA256(DK[16..32] || cipher.message).
        let mut hasher = Sha256::new();
        hasher.update(&dk[16..32]);
        hasher.update(&self.crypto.cipher.message);
        let computed = hasher.finalize();
        if computed.as_slice() != self.crypto.checksum.message.as_slice() {
            return Err(Error::KeystoreChecksum);
        }

        // AES-128-CTR decrypt with DK[0..16] as key and cipher.params.iv as IV.
        match self.crypto.cipher.function.as_str() {
            "aes-128-ctr" => {}
            _ => return Err(Error::InvalidKeystore),
        }
        let iv = &self.crypto.cipher.params.iv;
        if iv.len() != 16 {
            return Err(Error::InvalidKeystore);
        }
        // CTR keystream length == ciphertext length; reject wrong-sized payloads
        // before doing any AES work.
        if self.crypto.cipher.message.len() != SECRET_BYTES {
            return Err(Error::InvalidLength);
        }
        let mut cipher =
            Aes128Ctr::new_from_slices(&dk[0..16], iv).map_err(|_| Error::InvalidKeystore)?;
        let mut secret = Zeroizing::new(self.crypto.cipher.message.clone());
        cipher.apply_keystream(secret.as_mut_slice());

        let mut out = Zeroizing::new([0u8; SECRET_BYTES]);
        out.copy_from_slice(&secret);
        Ok(out)
    }

    fn derive_kdf_key(&self, password: &[u8]) -> Result<Vec<u8>, Error> {
        match (&self.crypto.kdf.function[..], &self.crypto.kdf.params) {
            (
                "scrypt",
                KdfParams::Scrypt {
                    dklen,
                    n,
                    p,
                    r,
                    salt,
                },
            ) => {
                let log_n = ilog2_u32(*n).ok_or(Error::InvalidKeystore)?;
                let params = scrypt::Params::new(log_n, *r, *p, *dklen as usize)
                    .map_err(|_| Error::InvalidKeystore)?;
                let mut out = vec![0u8; *dklen as usize];
                scrypt::scrypt(password, salt, &params, &mut out)
                    .map_err(|_| Error::KeystoreKdf)?;
                Ok(out)
            }
            (
                "pbkdf2",
                KdfParams::Pbkdf2 {
                    dklen,
                    c,
                    prf,
                    salt,
                },
            ) => {
                if prf != "hmac-sha256" {
                    return Err(Error::InvalidKeystore);
                }
                // Bound `dklen` before allocating: it is an unvalidated `u32`
                // from the keystore JSON, and unlike the scrypt branch (capped
                // by `scrypt::Params::new` at 64) PBKDF2 has no built-in limit,
                // so a hostile/corrupt file could request a multi-GiB buffer.
                if *dklen > 64 {
                    return Err(Error::InvalidKeystore);
                }
                let mut out = vec![0u8; *dklen as usize];
                pbkdf2::pbkdf2::<Hmac<Sha256>>(password, salt, *c, &mut out)
                    .map_err(|_| Error::KeystoreKdf)?;
                Ok(out)
            }
            _ => Err(Error::InvalidKeystore),
        }
    }
}

fn ilog2_u32(n: u32) -> Option<u8> {
    // scrypt's `log_n` requires N to be an exact power of two.
    n.is_power_of_two().then(|| n.trailing_zeros() as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_v4_version() {
        let json = r#"{"version":3,"uuid":"x","path":"","pubkey":"","crypto":{"kdf":{"function":"scrypt","params":{"dklen":32,"n":2,"p":1,"r":8,"salt":"00"},"message":""},"checksum":{"function":"sha256","params":{},"message":"00"},"cipher":{"function":"aes-128-ctr","params":{"iv":"00000000000000000000000000000000"},"message":"00"}}}"#;
        assert!(matches!(
            EthKeystoreV4::from_json(json),
            Err(Error::InvalidKeystore)
        ));
    }

    #[test]
    fn rejects_short_dklen_without_panicking() {
        // dklen=16 → derived key shorter than the DK[16..32] checksum slice.
        // Must surface InvalidKeystore, not panic.
        let json = r#"{"version":4,"uuid":"x","path":"","pubkey":"","crypto":{"kdf":{"function":"pbkdf2","params":{"dklen":16,"c":2,"prf":"hmac-sha256","salt":"00"},"message":""},"checksum":{"function":"sha256","params":{},"message":"00"},"cipher":{"function":"aes-128-ctr","params":{"iv":"00000000000000000000000000000000"},"message":"00"}}}"#;
        let ks = EthKeystoreV4::from_json(json).expect("v4 parses");
        assert!(matches!(ks.decrypt(b"pw"), Err(Error::InvalidKeystore)));
    }

    #[test]
    fn ilog2_round_trips_powers_of_two() {
        assert_eq!(ilog2_u32(1), Some(0));
        assert_eq!(ilog2_u32(2), Some(1));
        assert_eq!(ilog2_u32(262144), Some(18));
        assert_eq!(ilog2_u32(0), None);
        assert_eq!(ilog2_u32(3), None);
    }
}
