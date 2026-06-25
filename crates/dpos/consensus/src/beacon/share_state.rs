//! On-disk persistence of a live-DKG per-epoch share `(CeremonyOutput, Share)`.
//!
//! The live DKG (§8.11.1) memoizes `(PK_E, share)` for each committee-change epoch
//! into the in-memory [`crate::beacon::actor::CeremonyStore`]. A mid-epoch restart
//! otherwise loses it (the node then carries-forward the wrong key for E and stalls
//! its own seed votes). This module persists each memoized share to disk (mode 0600)
//! and reloads them at launch, so a restarted committee member rejoins the seed
//! quorum without re-running the ceremony.
//!
//! Two on-disk variants, dispatched on the leading 1-byte tag:
//! - [`ShareState::Plaintext`] (`TAG_PLAINTEXT`):
//!   `tag(1) ‖ u32_be(len(output)) ‖ encode_outcome(output) ‖ share.encode()`.
//! - [`ShareState::Encrypted`] (`TAG_ENCRYPTED`, E2 — gated on keystore mode):
//!   `tag(1) ‖ version(1) ‖ nonce(24) ‖ XChaCha20-Poly1305(key, nonce, aad, inner)`,
//!   where `inner` is the SAME plaintext frame minus its leading tag and
//!   `aad = TAG_ENCRYPTED ‖ version ‖ u64_be(epoch)` binds the ciphertext to its
//!   epoch (a swapped `beacon-share-e7 ↔ e9` then fails AEAD verification). The
//!   key is [`fluentbase_bls::ShareSealKey`], HKDF-derived from the validator BLS
//!   secret (gated on `--dpos.bls-keystore-path`); the plaintext-dev BLS path
//!   (`--dpos.bls-key-path`) has no off-disk secret, so it stays `TAG_PLAINTEXT`.
//!
//! Backward-compat is tag-as-version: old `TAG_PLAINTEXT` files decode unchanged
//! forever; `TAG_ENCRYPTED` is a NEW per-file variant a reader without the key
//! skips with a warning (never aborts startup), exactly like a malformed file.
//! The analogous at-rest VALIDATOR-key secret uses EIP-2335 (`bls/keystore.rs`),
//! a different secret shape with its own codec.

use crate::beacon::{
    ceremony::CeremonyOutput,
    outcome::{encode_outcome, parse_outcome},
    seed::parse_share,
};
use chacha20poly1305::{
    aead::{Aead, AeadCore as _, KeyInit, OsRng, Payload},
    XChaCha20Poly1305, XNonce,
};
use commonware_codec::Encode as _;
use commonware_cryptography::bls12381::primitives::group::Share;
use fluentbase_bls::ShareSealKey;
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

/// On-disk encoding variant. Carries the AEAD seal key for the encrypted arm.
pub enum ShareState {
    /// Unencrypted framing (`--dpos.bls-key-path` plaintext-dev nodes).
    Plaintext,
    /// XChaCha20-Poly1305 envelope keyed by the HKDF-derived [`ShareSealKey`]
    /// (E2 — keystore-mode validators).
    Encrypted(ShareSealKey),
}

const TAG_PLAINTEXT: u8 = 0;
const TAG_ENCRYPTED: u8 = 1;
/// Envelope version inside the `TAG_ENCRYPTED` frame — lets the AEAD/KDF params
/// evolve without colliding with the tag byte.
const ENVELOPE_VERSION: u8 = 1;
/// XChaCha20-Poly1305 nonce length.
const NONCE_BYTES: usize = 24;

const FILE_PREFIX: &str = "beacon-share-e";
const FILE_SUFFIX: &str = ".bin";

/// The plaintext inner frame (no leading tag): the byte-for-byte payload both
/// variants carry. `TAG_PLAINTEXT` writes `tag ‖ inner`; `TAG_ENCRYPTED` seals
/// `inner` as the AEAD plaintext.
fn inner_frame(output: &CeremonyOutput, share: &Share) -> Vec<u8> {
    let out_bytes = encode_outcome(output);
    let share_bytes = share.encode();
    let mut buf = Vec::with_capacity(4 + out_bytes.len() + share_bytes.len());
    buf.extend_from_slice(&(out_bytes.len() as u32).to_be_bytes());
    buf.extend_from_slice(&out_bytes);
    buf.extend_from_slice(share_bytes.as_ref());
    buf
}

/// Parse a tagless inner frame back into `(output, share)`.
fn parse_inner(rest: &[u8]) -> eyre::Result<(CeremonyOutput, Share)> {
    if rest.len() < 4 {
        eyre::bail!("truncated share file (no output-length prefix)");
    }
    let (len_bytes, rest) = rest.split_at(4);
    let out_len = u32::from_be_bytes(len_bytes.try_into().expect("4 bytes")) as usize;
    if rest.len() < out_len {
        eyre::bail!(
            "truncated share file (output length {out_len} > remaining {})",
            rest.len()
        );
    }
    let (out_bytes, share_bytes) = rest.split_at(out_len);
    let output =
        parse_outcome(out_bytes).map_err(|e| eyre::eyre!("parse persisted outcome: {e:?}"))?;
    let share =
        parse_share(share_bytes).map_err(|e| eyre::eyre!("parse persisted share: {e:?}"))?;
    Ok((output, share))
}

/// AAD binding a `TAG_ENCRYPTED` ciphertext to its epoch.
fn encrypted_aad(epoch: u64) -> [u8; 10] {
    let mut aad = [0u8; 10];
    aad[0] = TAG_ENCRYPTED;
    aad[1] = ENVELOPE_VERSION;
    aad[2..].copy_from_slice(&epoch.to_be_bytes());
    aad
}

/// Frame a `(output, share)` pair for on-disk storage under `state` for `epoch`.
/// Takes references — `CeremonyOutput` is not `Clone` and the caller persists
/// before moving the pair into the in-memory store. The `Encrypted` arm seals a
/// `Zeroizing` inner buffer with a fresh random 24-byte nonce + epoch-bound AAD.
pub fn encode(state: &ShareState, epoch: u64, output: &CeremonyOutput, share: &Share) -> Vec<u8> {
    let inner = Zeroizing::new(inner_frame(output, share));
    match state {
        ShareState::Plaintext => {
            let mut buf = Vec::with_capacity(1 + inner.len());
            buf.push(TAG_PLAINTEXT);
            buf.extend_from_slice(&inner);
            buf
        }
        ShareState::Encrypted(key) => {
            let cipher = XChaCha20Poly1305::new(key.as_bytes().into());
            let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
            let aad = encrypted_aad(epoch);
            // AEAD-encrypt of a single in-memory buffer cannot fail in this cipher.
            let ct = cipher
                .encrypt(
                    &nonce,
                    Payload {
                        msg: &inner,
                        aad: &aad,
                    },
                )
                .expect("XChaCha20-Poly1305 encrypt of an in-memory buffer is infallible");
            let mut buf = Vec::with_capacity(1 + 1 + NONCE_BYTES + ct.len());
            buf.push(TAG_ENCRYPTED);
            buf.push(ENVELOPE_VERSION);
            buf.extend_from_slice(nonce.as_slice());
            buf.extend_from_slice(&ct);
            buf
        }
    }
}

/// Decode a framed share for `epoch`, dispatching on the LEADING TAG BYTE (not on
/// `state`): a `TAG_PLAINTEXT` file always decodes; a `TAG_ENCRYPTED` file
/// requires `state == Encrypted(key)` and AEAD-opens with the epoch-bound AAD.
/// Errors on an unknown tag, a malformed body, a missing key, or AEAD failure
/// (wrong key / tampered / wrong-epoch file) — `load_all` turns those into a
/// warn+skip.
pub fn decode(bytes: &[u8], epoch: u64, state: &ShareState) -> eyre::Result<(CeremonyOutput, Share)> {
    let (&tag, rest) = bytes
        .split_first()
        .ok_or_else(|| eyre::eyre!("empty share file"))?;
    match tag {
        TAG_PLAINTEXT => parse_inner(rest),
        TAG_ENCRYPTED => {
            let ShareState::Encrypted(key) = state else {
                eyre::bail!("encrypted share file but no seal key available (plaintext-dev node or rotated validator key)");
            };
            let (&version, rest) = rest
                .split_first()
                .ok_or_else(|| eyre::eyre!("truncated encrypted share file (no version)"))?;
            if version != ENVELOPE_VERSION {
                eyre::bail!("unsupported encrypted-share envelope version {version}");
            }
            if rest.len() < NONCE_BYTES {
                eyre::bail!("truncated encrypted share file (no nonce)");
            }
            let (nonce_bytes, ct) = rest.split_at(NONCE_BYTES);
            let cipher = XChaCha20Poly1305::new(key.as_bytes().into());
            let aad = encrypted_aad(epoch);
            let inner = Zeroizing::new(
                cipher
                    .decrypt(
                        XNonce::from_slice(nonce_bytes),
                        Payload { msg: ct, aad: &aad },
                    )
                    .map_err(|_| {
                        eyre::eyre!("encrypted share AEAD-open failed (wrong key, tampered, or wrong epoch)")
                    })?,
            );
            parse_inner(&inner)
        }
        other => {
            eyre::bail!("unknown share-state tag {other} (supported: plaintext={TAG_PLAINTEXT}, encrypted={TAG_ENCRYPTED})")
        }
    }
}

fn file_for(dir: &Path, epoch: u64) -> PathBuf {
    dir.join(format!("{FILE_PREFIX}{epoch}{FILE_SUFFIX}"))
}

/// Persist a memoized `(output, share)` for `epoch` under `dir`, mode 0600, framed
/// per `state`. The encoded bytes (which embed the secret share) are scrubbed on
/// drop. Best-effort: the caller logs + continues on error (the in-memory store is
/// authoritative for the running process).
pub fn persist(
    dir: &Path,
    epoch: u64,
    output: &CeremonyOutput,
    share: &Share,
    state: &ShareState,
) -> eyre::Result<()> {
    std::fs::create_dir_all(dir).map_err(|e| eyre::eyre!("create share dir {dir:?}: {e}"))?;
    let bytes = Zeroizing::new(encode(state, epoch, output, share));
    fluentbase_bls::secret_store::write_mode_0600(&file_for(dir, epoch), &bytes)
        .map_err(|e| eyre::eyre!("write share file: {e}"))
}

/// Reload every persisted `beacon-share-e<E>.bin` under `dir` into
/// `(epoch, output, share)`, decoding each per `state`. A missing dir → empty; a
/// malformed OR undecryptable file is skipped with a warning (never aborts
/// startup — the in-memory store rebuilds via the next ceremony / carry-forward).
pub fn load_all(dir: &Path, state: &ShareState) -> Vec<(u64, CeremonyOutput, Share)> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(epoch_str) = name
            .strip_prefix(FILE_PREFIX)
            .and_then(|s| s.strip_suffix(FILE_SUFFIX))
        else {
            continue;
        };
        let Ok(epoch) = epoch_str.parse::<u64>() else {
            continue;
        };
        match std::fs::read(entry.path())
            .map_err(|e| eyre::eyre!(e))
            .and_then(|b| decode(&b, epoch, state))
        {
            Ok((output, share)) => out.push((epoch, output, share)),
            Err(e) => tracing::warn!(epoch, ?e, "beacon: skipping unreadable persisted share"),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beacon::dkg_oracle::run_local_dkg;
    use commonware_cryptography::ed25519::PrivateKey as Ed25519PrivateKey;
    use commonware_math::algebra::Random as _;
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng as _;

    fn sample_output_share() -> (CeremonyOutput, Share) {
        let mut rng = StdRng::seed_from_u64(7);
        let keys: Vec<Ed25519PrivateKey> = (0..4)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        let (output, shares) =
            run_local_dkg(&mut rng, b"FLUENT_DPOS_V1_test", 0, &keys, &keys).expect("dkg");
        let share = shares.values().next().expect("a share").clone();
        (output, share)
    }

    fn seal_key(seed: u64) -> ShareSealKey {
        fluentbase_bls::keys::ValidatorBlsKeypair::generate(&mut StdRng::seed_from_u64(seed))
            .derive_share_seal_key(20994)
    }

    fn fresh_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("beacon-share-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// Plaintext bytes are byte-identical to the pre-encryption frame (regression
    /// pin: old `TAG_PLAINTEXT` files must reload forever).
    #[test]
    fn plaintext_frame_is_unchanged() {
        let (output, share) = sample_output_share();
        let bytes = encode(&ShareState::Plaintext, 7, &output, &share);
        assert_eq!(bytes[0], TAG_PLAINTEXT);
        let out_bytes = encode_outcome(&output);
        let mut expected = vec![TAG_PLAINTEXT];
        expected.extend_from_slice(&(out_bytes.len() as u32).to_be_bytes());
        expected.extend_from_slice(&out_bytes);
        expected.extend_from_slice(share.encode().as_ref());
        assert_eq!(bytes, expected, "plaintext frame is byte-identical to today");
    }

    #[test]
    fn plaintext_persist_load_round_trip() {
        let (output, share) = sample_output_share();
        let dir = fresh_dir("plain");
        persist(&dir, 7, &output, &share, &ShareState::Plaintext).expect("persist");

        let loaded = load_all(&dir, &ShareState::Plaintext);
        assert_eq!(loaded.len(), 1);
        let (epoch, out2, share2) = &loaded[0];
        assert_eq!(*epoch, 7);
        assert_eq!(encode_outcome(out2), encode_outcome(&output));
        assert_eq!(share2.encode().as_ref(), share.encode().as_ref());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn encrypted_persist_load_round_trip() {
        let (output, share) = sample_output_share();
        let key = seal_key(101);
        let dir = fresh_dir("enc");
        persist(&dir, 7, &output, &share, &ShareState::Encrypted(key.clone())).expect("persist");

        let on_disk = std::fs::read(file_for(&dir, 7)).unwrap();
        assert_eq!(on_disk[0], TAG_ENCRYPTED, "encrypted file leads with TAG_ENCRYPTED");

        let loaded = load_all(&dir, &ShareState::Encrypted(key));
        assert_eq!(loaded.len(), 1, "encrypted share round-trips");
        let (epoch, out2, share2) = &loaded[0];
        assert_eq!(*epoch, 7);
        assert_eq!(encode_outcome(out2), encode_outcome(&output));
        assert_eq!(share2.encode().as_ref(), share.encode().as_ref());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A `TAG_ENCRYPTED` file loaded with the WRONG key OR with `Plaintext` state
    /// is SKIPPED (warn), not panicked.
    #[test]
    fn encrypted_file_with_wrong_or_absent_key_is_skipped() {
        let (output, share) = sample_output_share();
        let dir = fresh_dir("enc-wrongkey");
        persist(&dir, 7, &output, &share, &ShareState::Encrypted(seal_key(1))).expect("persist");

        assert!(
            load_all(&dir, &ShareState::Encrypted(seal_key(2))).is_empty(),
            "wrong key → skipped, not decoded"
        );
        assert!(
            load_all(&dir, &ShareState::Plaintext).is_empty(),
            "no key (plaintext-dev state) → skipped, not decoded"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The epoch-bound AAD rejects a ciphertext file renamed to a different epoch.
    #[test]
    fn epoch_bound_aad_rejects_renamed_ciphertext() {
        let (output, share) = sample_output_share();
        let key = seal_key(55);
        let dir = fresh_dir("enc-rename");
        persist(&dir, 7, &output, &share, &ShareState::Encrypted(key.clone())).expect("persist");
        std::fs::rename(file_for(&dir, 7), file_for(&dir, 9)).unwrap();

        // load_all reads the filename epoch (9) for the AAD → AEAD-open fails.
        assert!(
            load_all(&dir, &ShareState::Encrypted(key.clone())).is_empty(),
            "a ciphertext renamed e7 → e9 fails AEAD (epoch-bound AAD)"
        );
        // Decoding the same bytes at the ORIGINAL epoch still succeeds.
        let bytes = std::fs::read(file_for(&dir, 9)).unwrap();
        assert!(decode(&bytes, 7, &ShareState::Encrypted(key)).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_tag_is_rejected() {
        let (output, share) = sample_output_share();
        let mut bytes = encode(&ShareState::Plaintext, 7, &output, &share);
        bytes[0] = 0xFF;
        assert!(decode(&bytes, 7, &ShareState::Plaintext).is_err());
    }
}
