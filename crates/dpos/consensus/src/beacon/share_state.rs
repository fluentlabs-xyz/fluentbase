//! On-disk persistence of a live-DKG per-epoch share `(CeremonyOutput, Share)`.
//!
//! The live DKG (§8.11.1) memoizes `(PK_E, share)` for each committee-change epoch
//! into the in-memory [`crate::beacon::actor::CeremonyStore`]. A mid-epoch restart
//! otherwise loses it (the node then carries-forward the wrong key for E and stalls
//! its own seed votes). This module persists each memoized share to disk (mode 0600)
//! and reloads them at launch, so a restarted committee member rejoins the seed
//! quorum without re-running the ceremony.
//!
//! The record is `(output, share, agreed artifact)` — the artifact rides along
//! because it is what lets a restarted member come back holding the value its own
//! agreement instance decided, instead of re-agreeing from nothing.
//!
//! Two FRAME versions, each in a plaintext and an encrypted arm, dispatched on the
//! leading 1-byte tag. The tagless `inner` frame is what both arms of a version
//! carry:
//! - v1 (`TAG_PLAINTEXT` / `TAG_ENCRYPTED`) — READ-ONLY, never written any more:
//!   `u32_be(len(output)) ‖ encode_outcome(output) ‖ share.encode()`, the share
//!   running unlengthed to the end of the frame.
//! - v2 (`TAG_PLAINTEXT_V2` / `TAG_ENCRYPTED_V2`) — what is written today:
//!   `u32_be(len(output)) ‖ output ‖ u32_be(len(share)) ‖ share ‖
//!   u32_be(len(artifact)) ‖ artifact`. A ZERO-length artifact means "this node
//!   holds no artifact for the epoch" and is a normal record, not a damaged one:
//!   the reload treats it as "re-agree", never as an error.
//!
//! A plaintext arm writes `tag(1) ‖ inner`; an encrypted arm (E2 — gated on
//! keystore mode) writes
//! `tag(1) ‖ version(1) ‖ nonce(24) ‖ XChaCha20-Poly1305(key, nonce, aad, inner)`
//! with `aad = tag ‖ version ‖ u64_be(epoch)`, which binds the ciphertext to its
//! epoch (a swapped `beacon-share-e7 ↔ e9` fails AEAD verification) AND — because
//! the tag is in the AAD — to its frame version. The key is
//! [`fluentbase_bls::ShareSealKey`], HKDF-derived from the validator BLS secret
//! (gated on `--dpos.bls-keystore-path`); the plaintext-dev BLS path
//! (`--dpos.bls-key-path`) has no off-disk secret, so it stays on a plaintext tag.
//!
//! Backward-compat is tag-as-version, and it runs both ways: a v1 file decodes on
//! its own arm forever, and a v2 file met by a binary that predates the tag takes
//! the same warn-and-skip path a malformed file does (never aborts startup) — it
//! re-runs the ceremony rather than crashing. The analogous at-rest VALIDATOR-key
//! secret uses EIP-2335 (`bls/keystore.rs`), a different secret shape with its own
//! codec.

use crate::beacon::{
    ceremony::CeremonyOutput,
    dkg_msg::{Ack, DealerReveal},
    outcome::{encode_outcome, parse_outcome},
    seed::parse_share,
};
use chacha20poly1305::{
    aead::{Aead, AeadCore as _, KeyInit, OsRng, Payload},
    XChaCha20Poly1305, XNonce,
};
use commonware_codec::{Encode as _, Read as _, ReadExt as _, Write as _};
use commonware_cryptography::bls12381::{
    dkg::{DealerPrivMsg, DealerPubMsg},
    primitives::{group::Share, variant::MinSig},
};
use fluentbase_bls::{PeerPubkey, ShareSealKey};
use std::num::NonZeroU32;
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
/// v2 plaintext: every field length-prefixed, third field = the agreed artifact.
const TAG_PLAINTEXT_V2: u8 = 2;
/// v2 encrypted. Its AAD leads with THIS byte, so a v1 ciphertext can never
/// AEAD-open as a v2 record even at the same epoch under the same key.
const TAG_ENCRYPTED_V2: u8 = 3;
/// Envelope version inside an encrypted frame — lets the AEAD/KDF params evolve
/// without colliding with the tag byte.
const ENVELOPE_VERSION: u8 = 1;
/// XChaCha20-Poly1305 nonce length.
const NONCE_BYTES: usize = 24;

const FILE_PREFIX: &str = "beacon-share-e";
const FILE_SUFFIX: &str = ".bin";

/// The v2 tagless inner frame: the byte-for-byte payload both v2 arms carry.
/// `TAG_PLAINTEXT_V2` writes `tag ‖ inner`; `TAG_ENCRYPTED_V2` seals `inner` as
/// the AEAD plaintext.
///
/// `artifact = None` is written as a zero-length field, so "no artifact" and "an
/// empty artifact" are the same on-disk record and both reload as "re-agree".
fn inner_frame(output: &CeremonyOutput, share: &Share, artifact: Option<&[u8]>) -> Vec<u8> {
    let out_bytes = encode_outcome(output);
    let share_bytes = share.encode();
    let art_bytes = artifact.unwrap_or(&[]);
    let mut buf = Vec::with_capacity(12 + out_bytes.len() + share_bytes.len() + art_bytes.len());
    push_field(&mut buf, &out_bytes);
    push_field(&mut buf, share_bytes.as_ref());
    push_field(&mut buf, art_bytes);
    buf
}

fn push_field(buf: &mut Vec<u8>, field: &[u8]) {
    buf.extend_from_slice(&(field.len() as u32).to_be_bytes());
    buf.extend_from_slice(field);
}

/// Split one `u32_be(len) ‖ bytes` field off the front of `rest`, returning it and
/// the remainder. `what` names the field in the error so a truncated file says
/// WHICH field ran out.
fn take_field<'a>(rest: &'a [u8], what: &str) -> eyre::Result<(&'a [u8], &'a [u8])> {
    if rest.len() < 4 {
        eyre::bail!("truncated share file (no {what}-length prefix)");
    }
    let (len_bytes, rest) = rest.split_at(4);
    let len = u32::from_be_bytes(len_bytes.try_into().expect("4 bytes")) as usize;
    if rest.len() < len {
        eyre::bail!(
            "truncated share file ({what} length {len} > remaining {})",
            rest.len()
        );
    }
    Ok(rest.split_at(len))
}

/// Parse a v2 tagless inner frame back into `(output, share, artifact)`.
///
/// Trailing bytes are REJECTED. In v1 that check came for free from
/// [`parse_share`] consuming to the end of the frame; length-prefixing the share
/// took it away, so it is made explicit here — it is what catches a file whose
/// tail was corrupted or appended to.
fn parse_inner(rest: &[u8]) -> eyre::Result<(CeremonyOutput, Share, Option<Vec<u8>>)> {
    let (out_bytes, rest) = take_field(rest, "output")?;
    let (share_bytes, rest) = take_field(rest, "share")?;
    let (art_bytes, rest) = take_field(rest, "artifact")?;
    if !rest.is_empty() {
        eyre::bail!(
            "trailing bytes after the share record ({} bytes)",
            rest.len()
        );
    }
    let output =
        parse_outcome(out_bytes).map_err(|e| eyre::eyre!("parse persisted outcome: {e:?}"))?;
    let share =
        parse_share(share_bytes).map_err(|e| eyre::eyre!("parse persisted share: {e:?}"))?;
    let artifact = (!art_bytes.is_empty()).then(|| art_bytes.to_vec());
    Ok((output, share, artifact))
}

/// Parse a v1 tagless inner frame — `u32_be(len(output)) ‖ output ‖ share`, the
/// share unlengthed to the end. Read-only: nothing writes this frame any more.
fn parse_inner_v1(rest: &[u8]) -> eyre::Result<(CeremonyOutput, Share)> {
    let (out_bytes, share_bytes) = take_field(rest, "output")?;
    let output =
        parse_outcome(out_bytes).map_err(|e| eyre::eyre!("parse persisted outcome: {e:?}"))?;
    let share =
        parse_share(share_bytes).map_err(|e| eyre::eyre!("parse persisted share: {e:?}"))?;
    Ok((output, share))
}

/// Seal `inner` into an encrypted envelope under `key` for `aad`:
/// `tag(1) ‖ version(1) ‖ nonce(24) ‖ XChaCha20-Poly1305(key, nonce, aad, inner)`.
/// Shared by the share-file and the journal-record paths — they differ ONLY in the
/// tag, the AAD (domain-separated by its first byte) and the inner payload.
///
/// `tag` and `aad` MUST agree: the AAD leads with the same tag byte, which is what
/// stops one frame version's ciphertext from opening as another's.
fn seal_envelope(key: &ShareSealKey, tag: u8, aad: &[u8], inner: &[u8]) -> Vec<u8> {
    let cipher = XChaCha20Poly1305::new(key.as_bytes().into());
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    // AEAD-encrypt of a single in-memory buffer cannot fail in this cipher.
    let ct = cipher
        .encrypt(&nonce, Payload { msg: inner, aad })
        .expect("XChaCha20-Poly1305 encrypt of an in-memory buffer is infallible");
    let mut buf = Vec::with_capacity(1 + 1 + NONCE_BYTES + ct.len());
    buf.push(tag);
    buf.push(ENVELOPE_VERSION);
    buf.extend_from_slice(nonce.as_slice());
    buf.extend_from_slice(&ct);
    buf
}

/// Open a `TAG_ENCRYPTED` envelope body (`rest` = everything AFTER the leading
/// `TAG_ENCRYPTED` byte) under `key` for `aad`, returning the scrubbed plaintext
/// `inner`. Errors on a bad version / short nonce / AEAD failure (wrong key,
/// tampered, or wrong-epoch AAD). The inverse of [`seal_envelope`].
fn open_envelope(key: &ShareSealKey, aad: &[u8], rest: &[u8]) -> eyre::Result<Zeroizing<Vec<u8>>> {
    let (&version, rest) = rest
        .split_first()
        .ok_or_else(|| eyre::eyre!("truncated encrypted envelope (no version)"))?;
    if version != ENVELOPE_VERSION {
        eyre::bail!("unsupported encrypted envelope version {version}");
    }
    if rest.len() < NONCE_BYTES {
        eyre::bail!("truncated encrypted envelope (no nonce)");
    }
    let (nonce_bytes, ct) = rest.split_at(NONCE_BYTES);
    let cipher = XChaCha20Poly1305::new(key.as_bytes().into());
    Ok(Zeroizing::new(
        cipher
            .decrypt(XNonce::from_slice(nonce_bytes), Payload { msg: ct, aad })
            .map_err(|_| {
                eyre::eyre!(
                    "encrypted envelope AEAD-open failed (wrong key, tampered, or wrong epoch)"
                )
            })?,
    ))
}

/// AAD binding an encrypted ciphertext to its frame version (`tag`) and its epoch.
fn tagged_aad(tag: u8, epoch: u64) -> [u8; 10] {
    let mut aad = [0u8; 10];
    aad[0] = tag;
    aad[1] = ENVELOPE_VERSION;
    aad[2..].copy_from_slice(&epoch.to_be_bytes());
    aad
}

/// Frame an `(output, share, artifact)` record for on-disk storage under `state`
/// for `epoch`, always in the v2 frame. Takes references because the caller
/// persists before moving the pair into the in-memory store; `CeremonyOutput` is
/// `Clone` in the pinned commonware, so this is about ownership order, not about
/// the type. The `Encrypted` arm seals a `Zeroizing` inner buffer with a fresh
/// random 24-byte nonce + version-and-epoch-bound AAD.
pub fn encode(
    state: &ShareState,
    epoch: u64,
    output: &CeremonyOutput,
    share: &Share,
    artifact: Option<&[u8]>,
) -> Vec<u8> {
    let inner = Zeroizing::new(inner_frame(output, share, artifact));
    match state {
        ShareState::Plaintext => {
            let mut buf = Vec::with_capacity(1 + inner.len());
            buf.push(TAG_PLAINTEXT_V2);
            buf.extend_from_slice(&inner);
            buf
        }
        ShareState::Encrypted(key) => seal_envelope(
            key,
            TAG_ENCRYPTED_V2,
            &tagged_aad(TAG_ENCRYPTED_V2, epoch),
            &inner,
        ),
    }
}

/// Decode a framed share for `epoch`, dispatching on the LEADING TAG BYTE (not on
/// `state`): a plaintext file always decodes; an encrypted file requires
/// `state == Encrypted(key)` and AEAD-opens with the version-and-epoch-bound AAD.
/// A v1 file yields no artifact, which the caller reads as "re-agree".
/// Errors on an unknown tag, a malformed body, a missing key, or AEAD failure
/// (wrong key / tampered / wrong-epoch / wrong-version file) — `load_all` turns
/// those into a warn+skip.
pub fn decode(
    bytes: &[u8],
    epoch: u64,
    state: &ShareState,
) -> eyre::Result<(CeremonyOutput, Share, Option<Vec<u8>>)> {
    let (&tag, rest) = bytes
        .split_first()
        .ok_or_else(|| eyre::eyre!("empty share file"))?;
    match tag {
        TAG_PLAINTEXT => parse_inner_v1(rest).map(|(o, s)| (o, s, None)),
        TAG_ENCRYPTED => {
            let inner = open_share_envelope(state, &tagged_aad(TAG_ENCRYPTED, epoch), rest)?;
            parse_inner_v1(&inner).map(|(o, s)| (o, s, None))
        }
        TAG_PLAINTEXT_V2 => parse_inner(rest),
        TAG_ENCRYPTED_V2 => {
            let inner = open_share_envelope(state, &tagged_aad(TAG_ENCRYPTED_V2, epoch), rest)?;
            parse_inner(&inner)
        }
        other => {
            eyre::bail!("unknown share-state tag {other} (supported: plaintext v1={TAG_PLAINTEXT}, encrypted v1={TAG_ENCRYPTED}, plaintext v2={TAG_PLAINTEXT_V2}, encrypted v2={TAG_ENCRYPTED_V2})")
        }
    }
}

/// Open an encrypted share envelope body, refusing early when this node holds no
/// seal key at all.
fn open_share_envelope(
    state: &ShareState,
    aad: &[u8],
    rest: &[u8],
) -> eyre::Result<Zeroizing<Vec<u8>>> {
    let ShareState::Encrypted(key) = state else {
        eyre::bail!("encrypted share file but no seal key available (plaintext-dev node or rotated validator key)");
    };
    open_envelope(key, aad, rest)
}

fn file_for(dir: &Path, epoch: u64) -> PathBuf {
    dir.join(format!("{FILE_PREFIX}{epoch}{FILE_SUFFIX}"))
}

/// Persist a memoized `(output, share, artifact)` for `epoch` under `dir`, mode
/// 0600, framed per `state`. `artifact` is the encoded agreed artifact that
/// produced this share where one exists; `None` on a path that derived the share
/// without one (the reload then reads it as "re-agree"). The encoded bytes (which
/// embed the secret share) are scrubbed on drop. Best-effort: the caller logs +
/// continues on error (the in-memory store is authoritative for the running
/// process).
pub fn persist(
    dir: &Path,
    epoch: u64,
    output: &CeremonyOutput,
    share: &Share,
    artifact: Option<&[u8]>,
    state: &ShareState,
) -> eyre::Result<()> {
    std::fs::create_dir_all(dir).map_err(|e| eyre::eyre!("create share dir {dir:?}: {e}"))?;
    let bytes = Zeroizing::new(encode(state, epoch, output, share, artifact));
    fluentbase_bls::secret_store::write_mode_0600(&file_for(dir, epoch), &bytes)
        .map_err(|e| eyre::eyre!("write share file: {e}"))
}

/// One reloaded share record: the epoch, its memoized `(PK_E, share)`, and the
/// agreed artifact that produced it where the file carried one. `None` is the
/// normal shape of a v1 file and of a share derived without an artifact.
pub(crate) type ReloadedShare = (u64, CeremonyOutput, Share, Option<Vec<u8>>);

/// Reload every persisted `beacon-share-e<E>.bin` under `dir`, decoding each per
/// `state`. A missing dir → empty; a malformed OR undecryptable file is skipped
/// with a warning (never aborts startup — the in-memory store rebuilds via the
/// next ceremony / carry-forward).
pub fn load_all(dir: &Path, state: &ShareState) -> Vec<ReloadedShare> {
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
        // bug 13 read-side: refuse a group/other-accessible share file (a
        // backup/rsync that recreated it at 0644) — skip + warn, don't load a
        // leaked secret. Mirrors the p2p / validator-secret loaders.
        match fluentbase_bls::secret_store::reject_insecure_mode(&entry.path())
            .map_err(|e| eyre::eyre!(e))
            .and_then(|()| std::fs::read(entry.path()).map_err(|e| eyre::eyre!(e)))
            .and_then(|b| decode(&b, epoch, state))
        {
            Ok((output, share, artifact)) => out.push((epoch, output, share, artifact)),
            Err(e) => {
                tracing::warn!(
                    epoch,
                    ?e,
                    "beacon: skipping unreadable/insecure persisted share"
                )
            }
        }
    }
    out
}

const JOURNAL_PREFIX: &str = "beacon-dkgjournal-e";

/// One durable record of a mid-window DKG ceremony, journaled so a restart can
/// `Player::resume` instead of re-dealing a divergent contribution (§8.11.1).
///
/// The `ReceivedDealing` body carries a per-dealer `DealerPrivMsg` (secret-
/// equivalent) — so on a keystore-mode node the journal MUST seal with the same
/// `ShareState::Encrypted` AEAD as the share file (gated identically), never
/// plaintext.
pub(crate) enum JournalRecord {
    /// A `(dealer, DealerPubMsg, DealerPrivMsg)` dealing this node ACCEPTED — the
    /// input `Player::resume` re-feeds to rebuild `Player.view` (and re-emit acks).
    /// The (large, secret-bearing) bodies are boxed (clippy `large_enum_variant`).
    ReceivedDealing(PeerPubkey, Box<DealerPubMsg<MinSig>>, Box<DealerPrivMsg>),
    /// This node's OWN sealed log (also marks the epoch sealed: on resume the node
    /// re-broadcasts THIS log rather than re-dealing a fresh, divergent one).
    OwnSeal(Box<DealerReveal>),
    /// A peer's recorded sealed log — fed into both `Logs` (for `select`/`finalize`)
    /// and `Player::resume` (a peer log lets us finalize when that peer revealed our
    /// share).
    PeerLog(Box<DealerReveal>),
    /// An ack our OWN dealer received from `player` — journaled so a pre-seal reconstruct
    /// resume rebuilds `unsent` (who has acked) and does not re-reveal an already-acked
    /// player (§8.11.1). NOT secret (acks travel in the clear), but framed uniformly.
    OwnDealerAck(PeerPubkey, Box<Ack>),
}

const REC_RECEIVED_DEALING: u8 = 0;
const REC_OWN_SEAL: u8 = 1;
const REC_PEER_LOG: u8 = 2;
const REC_OWN_DEALER_ACK: u8 = 3;

/// The tagless inner frame of a journal record: `rec_tag(1) ‖ body`. Both
/// `ShareState` variants carry this — `TAG_PLAINTEXT` writes `tag ‖ inner`,
/// `TAG_ENCRYPTED` seals `inner` as the AEAD plaintext.
fn journal_inner(record: &JournalRecord) -> Vec<u8> {
    let mut buf = Vec::new();
    match record {
        JournalRecord::ReceivedDealing(dealer, pub_msg, priv_msg) => {
            buf.push(REC_RECEIVED_DEALING);
            dealer.write(&mut buf);
            pub_msg.write(&mut buf);
            priv_msg.write(&mut buf);
        }
        JournalRecord::OwnSeal(log) => {
            buf.push(REC_OWN_SEAL);
            log.write(&mut buf);
        }
        JournalRecord::PeerLog(log) => {
            buf.push(REC_PEER_LOG);
            log.write(&mut buf);
        }
        JournalRecord::OwnDealerAck(player, ack) => {
            buf.push(REC_OWN_DEALER_ACK);
            player.write(&mut buf);
            ack.write(&mut buf);
        }
    }
    buf
}

/// AAD binding a journal-record ciphertext to its epoch. A distinct first byte
/// (`TAG_ENCRYPTED ^ 0x80` = 0x81) domain-separates it from every share-file AAD
/// (0x00..0x03) so a share ciphertext can never AEAD-open as a journal record (or
/// vice-versa) — the high bit keeps that true as share tags keep counting up.
fn journal_aad(epoch: u64) -> [u8; 10] {
    let mut aad = tagged_aad(TAG_ENCRYPTED, epoch);
    aad[0] ^= 0x80;
    aad
}

/// Frame one record for the on-disk journal: `u32_be(len) ‖ framed`, where
/// `framed` is the SAME `tag ‖ inner` / `tag ‖ version ‖ nonce ‖ ct` framing the
/// share file uses (encryption MANDATORY on keystore nodes, plaintext only on dev).
fn encode_record(state: &ShareState, epoch: u64, record: &JournalRecord) -> Vec<u8> {
    let inner = Zeroizing::new(journal_inner(record));
    let framed = match state {
        ShareState::Plaintext => {
            let mut buf = Vec::with_capacity(1 + inner.len());
            buf.push(TAG_PLAINTEXT);
            buf.extend_from_slice(&inner);
            buf
        }
        ShareState::Encrypted(key) => {
            seal_envelope(key, TAG_ENCRYPTED, &journal_aad(epoch), &inner)
        }
    };
    let mut out = Vec::with_capacity(4 + framed.len());
    out.extend_from_slice(&(framed.len() as u32).to_be_bytes());
    out.extend_from_slice(&framed);
    out
}

/// Decode one tagless inner frame back into a [`JournalRecord`], bounding the DKG
/// decoders by `committee_size` (an upper bound, like the wire path).
fn parse_journal_inner(inner: &[u8], committee_size: NonZeroU32) -> eyre::Result<JournalRecord> {
    let (&rec_tag, mut body) = inner
        .split_first()
        .ok_or_else(|| eyre::eyre!("empty journal record"))?;
    let record = match rec_tag {
        REC_RECEIVED_DEALING => {
            let dealer = PeerPubkey::read(&mut body)
                .map_err(|e| eyre::eyre!("parse journal dealer key: {e:?}"))?;
            let pub_msg = DealerPubMsg::<MinSig>::read_cfg(&mut body, &committee_size)
                .map_err(|e| eyre::eyre!("parse journal DealerPubMsg: {e:?}"))?;
            let priv_msg = DealerPrivMsg::read_cfg(&mut body, &())
                .map_err(|e| eyre::eyre!("parse journal DealerPrivMsg: {e:?}"))?;
            JournalRecord::ReceivedDealing(dealer, Box::new(pub_msg), Box::new(priv_msg))
        }
        REC_OWN_SEAL | REC_PEER_LOG => {
            let log = DealerReveal::read_cfg(&mut body, &committee_size)
                .map_err(|e| eyre::eyre!("parse journal SignedDealerLog: {e:?}"))?;
            if rec_tag == REC_OWN_SEAL {
                JournalRecord::OwnSeal(Box::new(log))
            } else {
                JournalRecord::PeerLog(Box::new(log))
            }
        }
        REC_OWN_DEALER_ACK => {
            let player = PeerPubkey::read(&mut body)
                .map_err(|e| eyre::eyre!("parse journal dealer-ack player key: {e:?}"))?;
            let ack = Ack::read_cfg(&mut body, &())
                .map_err(|e| eyre::eyre!("parse journal ack: {e:?}"))?;
            JournalRecord::OwnDealerAck(player, Box::new(ack))
        }
        other => eyre::bail!("unknown journal record tag {other}"),
    };
    if !body.is_empty() {
        eyre::bail!("trailing bytes after journal record body");
    }
    Ok(record)
}

/// Decode one `framed` record (leading `tag ‖ …`) under `state` for `epoch`.
fn decode_record(
    framed: &[u8],
    epoch: u64,
    state: &ShareState,
    committee_size: NonZeroU32,
) -> eyre::Result<JournalRecord> {
    let (&tag, rest) = framed
        .split_first()
        .ok_or_else(|| eyre::eyre!("empty journal record frame"))?;
    match tag {
        TAG_PLAINTEXT => parse_journal_inner(rest, committee_size),
        TAG_ENCRYPTED => {
            let ShareState::Encrypted(key) = state else {
                eyre::bail!("encrypted journal record but no seal key available");
            };
            let inner = open_envelope(key, &journal_aad(epoch), rest)?;
            parse_journal_inner(&inner, committee_size)
        }
        other => eyre::bail!("unknown journal record state tag {other}"),
    }
}

fn journal_file_for(dir: &Path, epoch: u64) -> PathBuf {
    dir.join(format!("{JOURNAL_PREFIX}{epoch}{FILE_SUFFIX}"))
}

/// Append one ceremony [`JournalRecord`] for `epoch` to `beacon-dkgjournal-e<E>.bin`
/// under `dir`, mode 0600, framed per `state`. Best-effort: the caller logs +
/// continues on error (the in-memory ceremony is authoritative for the running
/// process; only a crash loses an unwritten tail, which the DKG-log recovery
/// resolver re-fetches).
pub(crate) fn append_journal(
    dir: &Path,
    epoch: u64,
    record: &JournalRecord,
    state: &ShareState,
) -> eyre::Result<()> {
    std::fs::create_dir_all(dir).map_err(|e| eyre::eyre!("create beacon dir {dir:?}: {e}"))?;
    let bytes = Zeroizing::new(encode_record(state, epoch, record));
    fluentbase_bls::secret_store::append_mode_0600(&journal_file_for(dir, epoch), &bytes)
        .map_err(|e| eyre::eyre!("append journal record: {e}"))
}

/// Result of [`load_journal`] — distinguishes a GENUINE first run (no journal file)
/// from a PRESENT-but-damaged journal, so the caller (`actor::maybe_start`) never
/// re-deals an already-sealed epoch. A torn/undecryptable journal means
/// THIS node already participated in `epoch`'s ceremony (it wrote at least one
/// record); re-dealing fresh would draw new `OsRng` randomness → a divergent
/// commitment → self-equivocation. So a damaged journal must SIT OUT, never start
/// fresh.
pub(crate) enum JournalLoad {
    /// No journal file exists for this epoch — a genuine first run; the caller may
    /// `start_fresh` (deal).
    NoFile,
    /// A journal file exists and decoded (possibly to an empty/short prefix if the
    /// tail was torn). The records are the recoverable prefix; the caller RESUMES
    /// player-only (never re-deals).
    Present(Vec<JournalRecord>),
    /// A journal file exists but its VERY FIRST record is undecodable (torn length
    /// prefix, truncated/garbled body, or undecryptable — e.g. wrong keystore mode).
    /// We wrote it, so we already participated; the caller SITS OUT (never re-deals).
    Torn,
}

/// Load the per-epoch ceremony journal for `epoch` under `dir`, decoding each
/// length-prefixed record per `state`, bounded by `committee_size`.
///
/// Returns a tri-state ([`JournalLoad`]): a MISSING file is `NoFile` (genuine first
/// run → the caller may deal); a present file whose first record is undecodable is
/// `Torn` (we already participated → the caller must SIT OUT, never re-deal); a
/// present file is `Present(records)` where a malformed/undecryptable record
/// TRUNCATES the read (the tail is the crash-lost part) with a warning — never
/// aborts, mirroring `load_all`'s warn+skip fail-soft.
pub(crate) fn load_journal(
    dir: &Path,
    epoch: u64,
    state: &ShareState,
    committee_size: NonZeroU32,
) -> JournalLoad {
    let Ok(bytes) = std::fs::read(journal_file_for(dir, epoch)) else {
        return JournalLoad::NoFile;
    };
    // A 0-byte file (create succeeded, the first write/fsync crashed before any
    // bytes) is a genuine first run, NOT a torn record — we committed NO dealing, so
    // the caller may deal. Only a NON-empty file whose first record fails to decode is
    // `Torn` (we wrote it ⇒ we participated ⇒ re-dealing would self-equivocate).
    if bytes.is_empty() {
        return JournalLoad::NoFile;
    }
    let mut out = Vec::new();
    let mut rest = bytes.as_slice();
    while !rest.is_empty() {
        if rest.len() < 4 {
            tracing::warn!(epoch, "beacon: truncated DKG journal record length prefix");
            break;
        }
        let (len_bytes, after_len) = rest.split_at(4);
        let len = u32::from_be_bytes(len_bytes.try_into().expect("4 bytes")) as usize;
        if after_len.len() < len {
            tracing::warn!(epoch, "beacon: truncated DKG journal record body");
            break;
        }
        let (framed, tail) = after_len.split_at(len);
        match decode_record(framed, epoch, state, committee_size) {
            Ok(record) => out.push(record),
            Err(e) => {
                tracing::warn!(epoch, ?e, "beacon: skipping unreadable DKG journal record");
                break;
            }
        }
        rest = tail;
    }
    // A present-but-damaged journal whose FIRST record never decoded is `Torn` — the
    // caller must sit out (we participated, so re-dealing self-equivocates).
    if out.is_empty() {
        JournalLoad::Torn
    } else {
        JournalLoad::Present(out)
    }
}

/// Delete the per-epoch ceremony journal once the share is finalized (or the
/// stalled ceremony is swept past its boundary) — it is within-window scratch.
pub(crate) fn evict_journal(dir: &Path, epoch: u64) {
    let path = journal_file_for(dir, epoch);
    if let Err(e) = std::fs::remove_file(&path) {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(epoch, ?e, "beacon: failed to evict DKG journal");
        }
    }
}

/// Delete a superseded per-epoch share secret file (`reconcile_journals` prunes the
/// older shares once carry-forward has moved on — review [334]).
pub(crate) fn evict_share(dir: &Path, epoch: u64) {
    let path = file_for(dir, epoch);
    if let Err(e) = std::fs::remove_file(&path) {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(epoch, ?e, "beacon: failed to evict stale DKG share");
        }
    }
}

/// Reconcile the on-disk beacon directory on the first tick, in one scan: prune both
/// boundary-passed ceremony JOURNALS and superseded SHARE secrets that no in-memory map
/// holds. The durable lifetime owner — a finalize-then-restart-before-boundary holds the
/// epoch in NO in-memory map (the running sweep never sees it), so without this its files
/// leak forever (review [334]). A missing dir is a no-op; malformed/foreign filenames are
/// IGNORED, never deleted.
///
/// JOURNALS (recompute-heal scratch, §8.11.1): delete every journal that has aged out
/// of the retention window — `epoch + JOURNAL_RETENTION_EPOCHS < now` — the SAME
/// predicate the running sweep applies (`actor.rs`, `*e + JOURNAL_RETENTION_EPOCHS <
/// now`), so a startup reconcile and the running sweep can never disagree. A
/// finalized-but-pre-boundary epoch E (its ceremony ran during E-1, so at finalize
/// `now ∈ E-1 < E`) is KEPT (resume/serve/recompute still need it), and a boundary-
/// passed epoch is retained one more window for the demote-heal recompute before it is
/// reclaimed.
///
/// SHARES (the durable carry-forward key): KEEP the ACTIVE carry-forward — the max share
/// epoch `<= now`, exactly what `CeremonyStore.range(..=now).next_back()` resolves to — AND
/// every FUTURE share (`> now`, a just-finalized next-epoch key not yet active, normal
/// near a change boundary). DELETE only shares STRICTLY OLDER than that active floor: they
/// are superseded and never re-used (a node verifies/signs only the live epoch's key).
/// Keeping only `max` would be WRONG — it would delete the active `<= now` carry-forward
/// whenever a future share already exists, demoting the node for the rest of the epoch.
pub(crate) fn reconcile_journals(dir: &Path, now: u64) {
    let (journals, shares) = scan_beacon_dir(dir);
    for epoch in journals {
        if epoch + crate::beacon::JOURNAL_RETENTION_EPOCHS < now {
            evict_journal(dir, epoch);
        }
    }
    // Active carry-forward = the newest share at or before `now`. Older shares are
    // superseded; the floor itself and every future (`> now`) share are retained.
    if let Some(floor) = shares.iter().copied().filter(|e| *e <= now).max() {
        for epoch in shares {
            if epoch < floor {
                evict_share(dir, epoch);
            }
        }
    }
}

/// Every epoch whose ceremony journal is still on disk under `dir`.
///
/// A journal is the only thing `maybe_start` can resume a ceremony from, so this
/// is also the set of epochs an agreed artifact can still be finalized over —
/// which is what makes it the bound on the startup artifact replay
/// ([`crate::beacon::artifact::restart_replay`]).
pub(crate) fn journal_epochs(dir: &Path) -> Vec<u64> {
    scan_beacon_dir(dir).0
}

/// `(journal epochs, share epochs)` parsed out of ONE directory scan. A missing
/// dir yields two empty sets; malformed / foreign filenames are ignored.
fn scan_beacon_dir(dir: &Path) -> (Vec<u64>, Vec<u64>) {
    let mut journals: Vec<u64> = Vec::new();
    let mut shares: Vec<u64> = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (journals, shares);
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if let Some(epoch) = name
            .strip_prefix(JOURNAL_PREFIX)
            .and_then(|s| s.strip_suffix(FILE_SUFFIX))
            .and_then(|s| s.parse::<u64>().ok())
        {
            journals.push(epoch);
        } else if let Some(epoch) = name
            .strip_prefix(FILE_PREFIX)
            .and_then(|s| s.strip_suffix(FILE_SUFFIX))
            .and_then(|s| s.parse::<u64>().ok())
        {
            shares.push(epoch);
        }
    }
    (journals, shares)
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

    /// Opaque stand-in for the encoded agreed artifact: this module frames the
    /// artifact as bytes and never parses it, so a real one would only make the
    /// test slower.
    const ARTIFACT: &[u8] = b"an encoded agreed artifact";

    /// The v1 inner frame this module no longer writes, rebuilt by hand so the
    /// backward-compat tests exercise real legacy bytes rather than a v1 encoder
    /// kept alive only for them.
    fn v1_inner(output: &CeremonyOutput, share: &Share) -> Vec<u8> {
        let out_bytes = encode_outcome(output);
        let mut buf = (out_bytes.len() as u32).to_be_bytes().to_vec();
        buf.extend_from_slice(&out_bytes);
        buf.extend_from_slice(share.encode().as_ref());
        buf
    }

    /// The v2 plaintext frame, pinned byte-for-byte: every field length-prefixed,
    /// the artifact last.
    #[test]
    fn plaintext_v2_frame_is_pinned() {
        let (output, share) = sample_output_share();
        let bytes = encode(&ShareState::Plaintext, 7, &output, &share, Some(ARTIFACT));
        let out_bytes = encode_outcome(&output);
        let share_bytes = share.encode();
        let mut expected = vec![TAG_PLAINTEXT_V2];
        expected.extend_from_slice(&(out_bytes.len() as u32).to_be_bytes());
        expected.extend_from_slice(&out_bytes);
        expected.extend_from_slice(&(share_bytes.len() as u32).to_be_bytes());
        expected.extend_from_slice(share_bytes.as_ref());
        expected.extend_from_slice(&(ARTIFACT.len() as u32).to_be_bytes());
        expected.extend_from_slice(ARTIFACT);
        assert_eq!(bytes, expected);
    }

    /// An absent artifact is a zero-length field, not a shorter frame.
    #[test]
    fn plaintext_v2_frame_without_an_artifact_is_a_zero_length_field() {
        let (output, share) = sample_output_share();
        let with = encode(&ShareState::Plaintext, 7, &output, &share, Some(&[]));
        let without = encode(&ShareState::Plaintext, 7, &output, &share, None);
        assert_eq!(with, without);
        assert_eq!(without[without.len() - 4..], [0, 0, 0, 0]);
    }

    /// A v1 plaintext file written by a binary that predates the artifact must keep
    /// decoding, yielding no artifact — which the reload reads as "re-agree".
    #[test]
    fn v1_plaintext_file_decodes_with_no_artifact() {
        let (output, share) = sample_output_share();
        let mut bytes = vec![TAG_PLAINTEXT];
        bytes.extend_from_slice(&v1_inner(&output, &share));

        let (out2, share2, artifact) =
            decode(&bytes, 7, &ShareState::Plaintext).expect("a v1 file still decodes");
        assert_eq!(encode_outcome(&out2), encode_outcome(&output));
        assert_eq!(share2.encode().as_ref(), share.encode().as_ref());
        assert!(artifact.is_none());
    }

    /// Same for a v1 ciphertext: its AAD leads with the v1 tag, so it opens on the
    /// v1 arm and only there.
    #[test]
    fn v1_encrypted_file_decodes_with_no_artifact() {
        let (output, share) = sample_output_share();
        let key = seal_key(303);
        let bytes = seal_envelope(
            &key,
            TAG_ENCRYPTED,
            &tagged_aad(TAG_ENCRYPTED, 7),
            &v1_inner(&output, &share),
        );

        let (out2, share2, artifact) = decode(&bytes, 7, &ShareState::Encrypted(key.clone()))
            .expect("a v1 ciphertext still decodes");
        assert_eq!(encode_outcome(&out2), encode_outcome(&output));
        assert_eq!(share2.encode().as_ref(), share.encode().as_ref());
        assert!(artifact.is_none());

        // Re-tagged as v2, the SAME ciphertext must fail: the frame version is in
        // the AAD, so a v1 body can never be opened as a v2 record.
        let mut retagged = bytes.clone();
        retagged[0] = TAG_ENCRYPTED_V2;
        assert!(decode(&retagged, 7, &ShareState::Encrypted(key)).is_err());
    }

    /// The other direction of tag-as-version: a v2 file met by a binary that knows
    /// only the v1 tags falls into the unknown-tag arm, which `load_all` turns into
    /// a warn-and-skip — the node re-runs the ceremony instead of crashing.
    #[test]
    fn v2_file_is_unreadable_to_a_v1_only_reader_and_skipped_not_fatal() {
        let (output, share) = sample_output_share();
        let dir = fresh_dir("v1-reader");
        persist(
            &dir,
            7,
            &output,
            &share,
            Some(ARTIFACT),
            &ShareState::Plaintext,
        )
        .expect("persist");

        let on_disk = std::fs::read(file_for(&dir, 7)).unwrap();
        assert!(
            !matches!(on_disk[0], TAG_PLAINTEXT | TAG_ENCRYPTED),
            "a v2 file must not lead with a tag a v1-only reader would MISPARSE"
        );

        // What that reader does with it, reproduced through the tag it cannot match.
        std::fs::write(file_for(&dir, 7), [0xFFu8; 8]).unwrap();
        assert!(
            load_all(&dir, &ShareState::Plaintext).is_empty(),
            "an unknown tag is skipped with a warning, never an abort"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Length-prefixing the share took away the trailing-byte check `parse_share`
    /// used to provide for free; v2 has to keep catching a corrupted tail.
    #[test]
    fn v2_frame_rejects_trailing_bytes() {
        let (output, share) = sample_output_share();
        let mut bytes = encode(&ShareState::Plaintext, 7, &output, &share, Some(ARTIFACT));
        bytes.push(0);
        assert!(decode(&bytes, 7, &ShareState::Plaintext).is_err());
    }

    #[test]
    fn plaintext_persist_load_round_trip() {
        let (output, share) = sample_output_share();
        let dir = fresh_dir("plain");
        persist(
            &dir,
            7,
            &output,
            &share,
            Some(ARTIFACT),
            &ShareState::Plaintext,
        )
        .expect("persist");

        let loaded = load_all(&dir, &ShareState::Plaintext);
        assert_eq!(loaded.len(), 1);
        let (epoch, out2, share2, artifact) = &loaded[0];
        assert_eq!(*epoch, 7);
        assert_eq!(encode_outcome(out2), encode_outcome(&output));
        assert_eq!(share2.encode().as_ref(), share.encode().as_ref());
        assert_eq!(artifact.as_deref(), Some(ARTIFACT));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A share persisted without an artifact (the demote-heal recompute) reloads as
    /// `None` — a well-formed record the caller reads as "re-agree", not a failure.
    #[test]
    fn persist_without_an_artifact_reloads_as_none() {
        let (output, share) = sample_output_share();
        let dir = fresh_dir("plain-noartifact");
        persist(&dir, 7, &output, &share, None, &ShareState::Plaintext).expect("persist");

        let loaded = load_all(&dir, &ShareState::Plaintext);
        assert_eq!(loaded.len(), 1);
        assert!(loaded[0].3.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn encrypted_persist_load_round_trip() {
        let (output, share) = sample_output_share();
        let key = seal_key(101);
        let dir = fresh_dir("enc");
        persist(
            &dir,
            7,
            &output,
            &share,
            Some(ARTIFACT),
            &ShareState::Encrypted(key.clone()),
        )
        .expect("persist");

        let on_disk = std::fs::read(file_for(&dir, 7)).unwrap();
        assert_eq!(
            on_disk[0], TAG_ENCRYPTED_V2,
            "encrypted file leads with TAG_ENCRYPTED_V2"
        );

        let loaded = load_all(&dir, &ShareState::Encrypted(key));
        assert_eq!(loaded.len(), 1, "encrypted share round-trips");
        let (epoch, out2, share2, artifact) = &loaded[0];
        assert_eq!(*epoch, 7);
        assert_eq!(encode_outcome(out2), encode_outcome(&output));
        assert_eq!(share2.encode().as_ref(), share.encode().as_ref());
        assert_eq!(artifact.as_deref(), Some(ARTIFACT));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An encrypted file loaded with the WRONG key OR with `Plaintext` state is
    /// SKIPPED (warn), not panicked.
    #[test]
    fn encrypted_file_with_wrong_or_absent_key_is_skipped() {
        let (output, share) = sample_output_share();
        let dir = fresh_dir("enc-wrongkey");
        persist(
            &dir,
            7,
            &output,
            &share,
            Some(ARTIFACT),
            &ShareState::Encrypted(seal_key(1)),
        )
        .expect("persist");

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
        persist(
            &dir,
            7,
            &output,
            &share,
            Some(ARTIFACT),
            &ShareState::Encrypted(key.clone()),
        )
        .expect("persist");
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
        let mut bytes = encode(&ShareState::Plaintext, 7, &output, &share, Some(ARTIFACT));
        bytes[0] = 0xFF;
        assert!(decode(&bytes, 7, &ShareState::Plaintext).is_err());
    }

    fn sample_journal_records() -> (Vec<JournalRecord>, NonZeroU32) {
        use commonware_cryptography::bls12381::dkg::{Dealer, Info, Player};
        use commonware_cryptography::bls12381::primitives::sharing::Mode;
        use commonware_cryptography::Signer as _;
        use commonware_utils::{ordered::Set, N3f1, NZU32};

        let mut rng = StdRng::seed_from_u64(7);
        let keys: Vec<Ed25519PrivateKey> = (0..3)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        let set = Set::from_iter_dedup(keys.iter().map(|k| k.public_key()));
        let info = Info::<MinSig, PeerPubkey>::new::<N3f1>(
            b"ns",
            0,
            None,
            Mode::NonZeroCounter,
            set.clone(),
            set,
        )
        .expect("info");
        let n = NZU32!(3);
        let dealer_key = &keys[0];

        let (mut dealer, pub_msg, priv_msgs) =
            Dealer::start::<N3f1>(&mut rng, info.clone(), dealer_key.clone(), None).expect("start");
        let (player_pk, priv_msg) = priv_msgs
            .into_iter()
            .find(|(pk, _)| *pk != dealer_key.public_key())
            .expect("a non-dealer player");
        let player_key = keys
            .iter()
            .find(|k| k.public_key() == player_pk)
            .expect("player key");
        let mut player = Player::new(info, player_key.clone()).expect("player");
        let ack = player
            .dealer_message::<N3f1>(dealer_key.public_key(), pub_msg.clone(), priv_msg.clone())
            .expect("ack");
        dealer
            .receive_player_ack(player_pk.clone(), ack.clone())
            .expect("receive ack");
        let log = dealer.finalize::<N3f1>();

        let records = vec![
            JournalRecord::ReceivedDealing(
                dealer_key.public_key(),
                Box::new(pub_msg),
                Box::new(priv_msg),
            ),
            JournalRecord::OwnDealerAck(player_pk, Box::new(ack)),
            JournalRecord::OwnSeal(Box::new(log.clone())),
            JournalRecord::PeerLog(Box::new(log)),
        ];
        (records, n)
    }

    fn assert_records_match(loaded: &[JournalRecord], expected: &[JournalRecord], state: &str) {
        assert_eq!(loaded.len(), expected.len(), "{state}: record count");
        for (got, want) in loaded.iter().zip(expected) {
            match (got, want) {
                (
                    JournalRecord::ReceivedDealing(d0, p0, s0),
                    JournalRecord::ReceivedDealing(d1, p1, s1),
                ) => {
                    assert_eq!(d0, d1, "{state}: dealer key");
                    assert_eq!(p0.encode(), p1.encode(), "{state}: DealerPubMsg");
                    assert_eq!(s0.encode(), s1.encode(), "{state}: DealerPrivMsg");
                }
                (JournalRecord::OwnSeal(l0), JournalRecord::OwnSeal(l1))
                | (JournalRecord::PeerLog(l0), JournalRecord::PeerLog(l1)) => {
                    assert_eq!(l0.encode(), l1.encode(), "{state}: SignedDealerLog");
                }
                (JournalRecord::OwnDealerAck(p0, a0), JournalRecord::OwnDealerAck(p1, a1)) => {
                    assert_eq!(p0, p1, "{state}: dealer-ack player key");
                    assert_eq!(a0.encode(), a1.encode(), "{state}: PlayerAck");
                }
                _ => panic!("{state}: record-kind mismatch"),
            }
        }
    }

    /// Records of a present-and-decodable journal (panics on `NoFile`/`Torn`).
    fn present(load: JournalLoad) -> Vec<JournalRecord> {
        match load {
            JournalLoad::Present(r) => r,
            JournalLoad::NoFile => panic!("expected a present journal, got NoFile"),
            JournalLoad::Torn => panic!("expected a present journal, got Torn"),
        }
    }

    #[test]
    fn journal_round_trip_plaintext_and_encrypted() {
        let (records, n) = sample_journal_records();
        for (tag, state) in [
            ("plain", ShareState::Plaintext),
            ("enc", ShareState::Encrypted(seal_key(101))),
        ] {
            let dir = fresh_dir(&format!("journal-{tag}"));
            for record in &records {
                append_journal(&dir, 5, record, &state).expect("append");
            }
            let loaded = present(load_journal(&dir, 5, &state, n));
            assert_records_match(&loaded, &records, tag);
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn load_journal_missing_file_is_no_file_not_torn() {
        let dir = fresh_dir("journal-missing");
        let n = NonZeroU32::new(4).unwrap();
        assert!(
            matches!(
                load_journal(&dir, 5, &ShareState::Plaintext, n),
                JournalLoad::NoFile
            ),
            "a genuine first run (no file) must be NoFile, not Torn — the caller may deal"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_encrypted_with_wrong_or_absent_key_is_torn_not_no_file() {
        let (records, n) = sample_journal_records();
        let dir = fresh_dir("journal-wrongkey");
        let state = ShareState::Encrypted(seal_key(1));
        for record in &records {
            append_journal(&dir, 5, record, &state).expect("append");
        }
        // A PRESENT file whose first record never decodes is `Torn` (we wrote it →
        // we participated → must sit out, NEVER `NoFile`/re-deal).
        assert!(
            matches!(
                load_journal(&dir, 5, &ShareState::Encrypted(seal_key(2)), n),
                JournalLoad::Torn
            ),
            "wrong key → first record AEAD-fails → Torn (present but undecodable)"
        );
        assert!(
            matches!(
                load_journal(&dir, 5, &ShareState::Plaintext, n),
                JournalLoad::Torn
            ),
            "no key (plaintext-dev state) → encrypted record skipped → Torn"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_epoch_bound_aad_rejects_wrong_epoch() {
        let (records, n) = sample_journal_records();
        let key = seal_key(55);
        let state = ShareState::Encrypted(key.clone());
        let dir = fresh_dir("journal-epoch");
        for record in &records {
            append_journal(&dir, 7, record, &state).expect("append");
        }
        // Read the e7 journal bytes back under epoch 9 → the epoch-bound AAD rejects.
        let loaded = present(load_journal(&dir, 7, &state, n)); // correct epoch decodes
        assert_eq!(loaded.len(), records.len(), "correct epoch decodes");
        std::fs::rename(journal_file_for(&dir, 7), journal_file_for(&dir, 9)).unwrap();
        assert!(
            matches!(load_journal(&dir, 9, &state, n), JournalLoad::Torn),
            "a journal renamed e7 → e9 fails AEAD (epoch-bound AAD) → Torn"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn evict_journal_removes_file() {
        let (records, _n) = sample_journal_records();
        let dir = fresh_dir("journal-evict");
        append_journal(&dir, 5, &records[0], &ShareState::Plaintext).expect("append");
        assert!(journal_file_for(&dir, 5).exists());
        evict_journal(&dir, 5);
        assert!(
            !journal_file_for(&dir, 5).exists(),
            "evicted journal is gone"
        );
        evict_journal(&dir, 5); // idempotent — no panic on a missing file
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `reconcile_journals(now)` deletes only journals that have aged OUT of the
    /// recompute-heal window (`epoch + JOURNAL_RETENTION_EPOCHS < now`), keeps the
    /// in-window ones (so a demoted member can still recompute from them past the
    /// boundary), and never touches a foreign file. With `JOURNAL_RETENTION_EPOCHS = 1`
    /// at now=5: e3 aged out (3+1<5), e5 in-window (5+1≥5), e7 future (kept).
    #[test]
    fn reconcile_journals_deletes_past_window_keeps_in_window_ignores_foreign() {
        let (records, _n) = sample_journal_records();
        let dir = fresh_dir("journal-reconcile");
        for e in [3u64, 5, 7] {
            append_journal(&dir, e, &records[0], &ShareState::Plaintext).expect("append");
        }
        let foreign = dir.join("some-other-file.bin");
        std::fs::write(&foreign, b"keep me").expect("write foreign");

        reconcile_journals(&dir, 5);

        assert!(
            !journal_file_for(&dir, 3).exists(),
            "e3 + RET(1) < now=5 → aged out of the recompute window, deleted"
        );
        assert!(
            journal_file_for(&dir, 5).exists(),
            "e5 + RET(1) >= now=5 → still in the recompute window, kept"
        );
        assert!(
            journal_file_for(&dir, 7).exists(),
            "e7 > now=5 → future, kept"
        );
        assert!(foreign.exists(), "a foreign filename is never deleted");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `reconcile_journals` prunes SUPERSEDED share secrets ([334]): it keeps the ACTIVE
    /// carry-forward (`max{share epoch <= now}`) AND every FUTURE share (`> now`, a
    /// just-finalized next-epoch key), deleting only strictly-older shares. Keeping only the
    /// max share would WRONGLY delete the active `<= now` carry-forward whenever a future
    /// share exists (normal at a committee-change boundary) — this pins the floor rule.
    #[test]
    fn reconcile_prunes_superseded_shares_keeps_active_and_future() {
        let (output, share) = sample_output_share();
        let dir = fresh_dir("share-reconcile");
        // Shares {3, 5, 7}, now=6 → floor = max{e<=6} = 5 (active), 7 is future.
        for e in [3u64, 5, 7] {
            persist(&dir, e, &output, &share, None, &ShareState::Plaintext).expect("persist");
        }
        reconcile_journals(&dir, 6);
        assert!(
            !file_for(&dir, 3).exists(),
            "e3 < floor(5) — superseded, deleted"
        );
        assert!(
            file_for(&dir, 5).exists(),
            "e5 = active carry-forward (max <= now=6) — kept"
        );
        assert!(
            file_for(&dir, 7).exists(),
            "e7 > now=6 — a just-finalized future share, kept (keep-only-max would wrongly drop e5)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Floor-rule edge: when EVERY share is in the future (`> now`) there is no `<= now`
    /// floor, so reconcile deletes NOTHING (a node with only future shares keeps them all).
    #[test]
    fn reconcile_keeps_all_when_every_share_is_future() {
        let (output, share) = sample_output_share();
        let dir = fresh_dir("share-all-future");
        for e in [8u64, 9] {
            persist(&dir, e, &output, &share, None, &ShareState::Plaintext).expect("persist");
        }
        reconcile_journals(&dir, 5); // now=5, both shares > now
        assert!(
            file_for(&dir, 8).exists(),
            "no `<= now` floor → nothing deleted"
        );
        assert!(
            file_for(&dir, 9).exists(),
            "no `<= now` floor → nothing deleted"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reconcile_journals_missing_dir_is_no_op() {
        let dir =
            std::env::temp_dir().join(format!("beacon-reconcile-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        reconcile_journals(&dir, 99); // must not panic on a missing dir
    }

    /// A 0-byte journal (create succeeded, the first write crashed before any bytes)
    /// is `NoFile` (genuine first run → deal), NOT `Torn`.
    #[test]
    fn zero_byte_journal_is_no_file_not_torn() {
        let dir = fresh_dir("journal-zerobyte");
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(journal_file_for(&dir, 5), b"").expect("write empty");
        let n = NonZeroU32::new(4).unwrap();
        assert!(
            matches!(
                load_journal(&dir, 5, &ShareState::Plaintext, n),
                JournalLoad::NoFile
            ),
            "a 0-byte journal = create-without-write = first run → NoFile, not Torn"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
