//! On-disk persistence of a live-DKG per-epoch share `(CeremonyOutput, Share)`.
//!
//! The live DKG (§8.11.1) memoizes `(PK_E, share)` for each committee-change epoch
//! into the in-memory [`crate::beacon::actor::CeremonyStore`]. A mid-epoch restart
//! otherwise loses it (the node then carries-forward the wrong key for E and stalls
//! its own seed votes). This module persists each memoized share to disk (mode 0600)
//! and reloads them at launch, so a restarted committee member rejoins the seed
//! quorum without re-running the ceremony.
//!
//! Wire format (forward-compatible — a 1-byte tag reserves room for an encrypted
//! variant without a format break):
//! `tag(1) ‖ u32_be(len(output)) ‖ encode_outcome(output) ‖ share.encode()`.
//!
//! Only [`ShareState::Plaintext`] ships in v1; the genesis `--dpos.beacon-share-path`
//! is encrypted-at-rest via EIP-2335 (`bls/keystore.rs`), and the encrypted on-disk
//! variant for these per-epoch shares is the P2 follow-up (a length-generic
//! `keystore.rs` encrypt path) — `TAG_ENCRYPTED` is reserved for it.

use crate::beacon::{
    ceremony::CeremonyOutput,
    outcome::{encode_outcome, parse_outcome},
    seed::parse_share,
};
use commonware_codec::Encode as _;
use commonware_cryptography::bls12381::primitives::group::Share;
use std::{
    io,
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

/// On-disk encoding variant (the 1-byte frame tag).
pub enum ShareState {
    /// Unencrypted framing. v1 default.
    Plaintext,
    // Encrypted (keystore-style, P2 follow-up) — TAG_ENCRYPTED reserved below.
}

const TAG_PLAINTEXT: u8 = 0;
// const TAG_ENCRYPTED: u8 = 1; // reserved — P2 follow-up

const FILE_PREFIX: &str = "beacon-share-e";
const FILE_SUFFIX: &str = ".bin";

impl ShareState {
    fn tag(&self) -> u8 {
        match self {
            ShareState::Plaintext => TAG_PLAINTEXT,
        }
    }
}

/// Frame a `(output, share)` pair for on-disk storage (`ShareState::Plaintext`).
/// Takes references — `CeremonyOutput` is not `Clone` and the caller persists
/// before moving the pair into the in-memory store.
pub fn encode(output: &CeremonyOutput, share: &Share) -> Vec<u8> {
    let out_bytes = encode_outcome(output);
    let share_bytes = share.encode();
    let mut buf = Vec::with_capacity(1 + 4 + out_bytes.len() + share_bytes.len());
    buf.push(ShareState::Plaintext.tag());
    buf.extend_from_slice(&(out_bytes.len() as u32).to_be_bytes());
    buf.extend_from_slice(&out_bytes);
    buf.extend_from_slice(share_bytes.as_ref());
    buf
}

/// Decode a framed share. Errors on an unknown tag or a malformed body.
pub fn decode(bytes: &[u8]) -> eyre::Result<(CeremonyOutput, Share)> {
    let (&tag, rest) = bytes
        .split_first()
        .ok_or_else(|| eyre::eyre!("empty share file"))?;
    if tag != TAG_PLAINTEXT {
        eyre::bail!("unknown share-state tag {tag} (v1 supports only plaintext={TAG_PLAINTEXT})");
    }
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

fn file_for(dir: &Path, epoch: u64) -> PathBuf {
    dir.join(format!("{FILE_PREFIX}{epoch}{FILE_SUFFIX}"))
}

/// Persist a memoized `(output, share)` for `epoch` under `dir`, mode 0600. The
/// encoded bytes (which embed the secret share) are scrubbed on drop. Best-effort:
/// the caller logs + continues on error (the in-memory store is authoritative for
/// the running process).
pub fn persist(dir: &Path, epoch: u64, output: &CeremonyOutput, share: &Share) -> eyre::Result<()> {
    std::fs::create_dir_all(dir).map_err(|e| eyre::eyre!("create share dir {dir:?}: {e}"))?;
    let bytes = Zeroizing::new(encode(output, share));
    write_mode_0600(&file_for(dir, epoch), &bytes).map_err(|e| eyre::eyre!("write share file: {e}"))
}

/// Reload every persisted `beacon-share-e<E>.bin` under `dir` into
/// `(epoch, output, share)`. A missing dir → empty; a malformed file is skipped
/// with a warning (never aborts startup).
pub fn load_all(dir: &Path) -> Vec<(u64, CeremonyOutput, Share)> {
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
            .and_then(|b| decode(&b))
        {
            Ok((output, share)) => out.push((epoch, output, share)),
            Err(e) => tracing::warn!(epoch, ?e, "beacon: skipping malformed persisted share"),
        }
    }
    out
}

#[cfg(unix)]
fn write_mode_0600(path: &Path, data: &[u8]) -> io::Result<()> {
    use std::{io::Write as _, os::unix::fs::OpenOptionsExt as _};
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(data)
}

#[cfg(not(unix))]
fn write_mode_0600(path: &Path, data: &[u8]) -> io::Result<()> {
    std::fs::write(path, data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beacon::dkg::run_local_dkg;
    use commonware_cryptography::ed25519::PrivateKey as Ed25519PrivateKey;
    use commonware_math::algebra::Random as _;
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng as _;

    // A persisted (output, share) round-trips byte-for-byte through encode/decode
    // and through a 0600 file on disk.
    #[test]
    fn output_share_persist_load_round_trip() {
        let mut rng = StdRng::seed_from_u64(7);
        let keys: Vec<Ed25519PrivateKey> = (0..4)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        let (output, shares) =
            run_local_dkg(&mut rng, b"FLUENT_DPOS_V1_test", 0, &keys, &keys).expect("dkg");
        let share = shares.values().next().expect("a share").clone();

        let dir = std::env::temp_dir().join(format!("beacon-share-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        persist(&dir, 7, &output, &share).expect("persist");

        let loaded = load_all(&dir);
        assert_eq!(loaded.len(), 1, "exactly one persisted share reloaded");
        let (epoch, out2, share2) = &loaded[0];
        assert_eq!(*epoch, 7);
        assert_eq!(
            encode_outcome(out2),
            encode_outcome(&output),
            "output round-trips"
        );
        assert_eq!(
            share2.encode().as_ref(),
            share.encode().as_ref(),
            "share round-trips"
        );

        // An unknown tag is rejected, not silently mis-decoded.
        let mut bytes = encode(&output, &share);
        bytes[0] = 0xFF;
        assert!(decode(&bytes).is_err(), "unknown tag rejected");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
