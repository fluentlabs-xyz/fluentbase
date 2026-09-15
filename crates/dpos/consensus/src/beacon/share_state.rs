//! On-disk persistence of a live-DKG per-epoch `Share`.
//!
//! The live DKG (§8.11.1) memoizes `(PK_E, share)` for each committee-change epoch
//! into the in-memory [`crate::beacon::actor::CeremonyStore`]. A mid-epoch restart
//! otherwise loses it (the node then carries-forward the wrong key for E and stalls
//! its own seed votes). This module persists each memoized share to disk (mode 0600)
//! and reloads them at launch, so a restarted committee member rejoins the seed
//! quorum without re-running the ceremony.
//!
//! The record is the SHARE, and nothing else.
//!
//! # Neither the artifact nor the group output rides along any more (П-3)
//!
//! The v2 record used to be `(output, share, artifact)`. Both of the other two
//! fields were copies of `PK_E` and of the public polynomial: the artifact
//! literally, the `output` by carrying the same `Sharing`. That made this file a
//! SECOND on-disk owner of the fact beside
//! [`ArtifactStore`](crate::beacon::artifact::ArtifactStore), and the ratified rule
//! (`.dpos-study/DECISIONS.md`, П-3) is that the artifact store is the only one. The
//! polynomial a reloaded share pairs with is read from the artifact of the SAME
//! minting epoch (`KeyIndex::sharing_at`), so nothing is lost and the file holds the
//! one thing only it can hold: this node's secret.
//!
//! **The liveness this costs is named, not discovered.** A node that restarts
//! holding a share whose artifact never reached its durable store (the artifact
//! journal is write-behind) no longer has the epoch key locally at all: σ of the
//! epoch is held pending, execution parks, and the key arrives from peers —
//! `DkgActor::drive_acquisition` asks for it on the next height tick from the
//! `Acquiring(ArtifactForShare)` phase (`Acquiring(ArtifactForKey)` is the non-member's leg and
//! excludes members by design), and at `≤ f` faults the peers hold it. That is the
//! trade §5.4 of
//! `.dpos-study/history/E5-BEACON-DESIGN.md` accepts: one quorum-attested owner of
//! `PK_E` against a locally-sourced key on a disk fault.
//!
//! ONE frame version, in a plaintext and an encrypted arm, dispatched on the
//! leading 1-byte tag. The tagless `inner` frame is what both arms carry:
//! - v2 (`TAG_PLAINTEXT_V2` / `TAG_ENCRYPTED_V2`) — what is written today:
//!   `u32_be(len(share)) ‖ share`.
//!
//! # What happens to an OLDER file at startup — stated, because it is silent
//!
//! Both retired shapes take `load_all`'s warn-and-skip path, which is the same one
//! a corrupt file takes, and the node then treats the epoch as one it holds no
//! share for: `DkgActor::recover` resumes from the ceremony journal where that is still
//! on disk, and sits the epoch out as a verifier where it is not. Nothing aborts
//! startup and no wrong share is ever adopted.
//! - a **v1** file (leading tag `TAG_PLAINTEXT` / `TAG_ENCRYPTED`) — the arm is
//!   deleted, so its tag is now simply unknown to the share-file reader. (Those two
//!   tag BYTES are still live: the ceremony-journal frame below uses them, under its
//!   own domain-separated AAD.)
//! - a **two- or three-field v2** file written before this change — the extra fields
//!   are read as trailing bytes, which `parse_inner` rejects outright rather than
//!   ignoring.
//!
//! Both are acceptable rather than merely tolerable here because a DPoS network is
//! relaunched from a fresh genesis rather than migrated in place, so the upgrade
//! path an old share file represents is a test datadir, never a live validator's.
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

#[cfg(test)]
use crate::beacon::{ceremony::CeremonyOutput, outcome::encode_outcome};
use crate::beacon::{
    dkg_msg::{Ack, DealerReveal},
    seed::parse_share,
};
use alloy_primitives::B256;
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
/// v2 plaintext: the share, length-prefixed.
const TAG_PLAINTEXT_V2: u8 = 2;
/// v2 encrypted. Its AAD leads with THIS byte, so a retired-v1 ciphertext can
/// never AEAD-open as a v2 record even at the same epoch under the same key.
const TAG_ENCRYPTED_V2: u8 = 3;
/// Envelope version inside an encrypted frame — lets the AEAD/KDF params evolve
/// without colliding with the tag byte.
const ENVELOPE_VERSION: u8 = 1;
/// XChaCha20-Poly1305 nonce length.
const NONCE_BYTES: usize = 24;

const FILE_PREFIX: &str = "beacon-share-e";
const FILE_SUFFIX: &str = ".bin";
/// The durable `Conflict(E)` marker (`persist_conflict`): the epoch's signing was
/// stopped on this node. Lives beside the share file on the journal's window.
const CONFLICT_PREFIX: &str = "beacon-conflict-e";

/// The v2 tagless inner frame: the byte-for-byte payload both v2 arms carry.
/// `TAG_PLAINTEXT_V2` writes `tag ‖ inner`; `TAG_ENCRYPTED_V2` seals `inner` as
/// the AEAD plaintext.
///
fn inner_frame(share: &Share) -> Vec<u8> {
    let share_bytes = share.encode();
    let mut buf = Vec::with_capacity(4 + share_bytes.len());
    push_field(&mut buf, share_bytes.as_ref());
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

/// Parse a v2 tagless inner frame back into the share.
///
/// Trailing bytes are REJECTED, and that check carries a second job: a pre-П-3
/// record leads with the OUTPUT's length prefix, so it reaches here with fields left
/// over, and this is what refuses it instead of silently reading a record whose shape
/// it does not know.
fn parse_inner(rest: &[u8]) -> eyre::Result<Share> {
    let (share_bytes, rest) = take_field(rest, "share")?;
    if !rest.is_empty() {
        eyre::bail!(
            "trailing bytes after the share record ({} bytes) — a pre-П-3 record \
             carrying the group output and/or the artifact reads exactly like this",
            rest.len()
        );
    }
    parse_share(share_bytes).map_err(|e| eyre::eyre!("parse persisted share: {e:?}"))
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

/// Frame a share record for on-disk storage under `state` for `epoch`, always in the
/// v2 frame. Takes a reference because the caller persists before moving the share
/// into the in-memory store. The `Encrypted` arm seals a `Zeroizing` inner buffer
/// with a fresh random 24-byte nonce + version-and-epoch-bound AAD.
pub fn encode(state: &ShareState, epoch: u64, share: &Share) -> Vec<u8> {
    let inner = Zeroizing::new(inner_frame(share));
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
/// Errors on an unknown tag (which now includes the retired v1 share tags), a
/// malformed body, a missing key, or AEAD failure (wrong key / tampered /
/// wrong-epoch / wrong-version file) — `load_all` turns those into a warn+skip.
pub fn decode(bytes: &[u8], epoch: u64, state: &ShareState) -> eyre::Result<Share> {
    let (&tag, rest) = bytes
        .split_first()
        .ok_or_else(|| eyre::eyre!("empty share file"))?;
    match tag {
        TAG_PLAINTEXT_V2 => parse_inner(rest),
        TAG_ENCRYPTED_V2 => {
            let inner = open_share_envelope(state, &tagged_aad(TAG_ENCRYPTED_V2, epoch), rest)?;
            parse_inner(&inner)
        }
        other => {
            eyre::bail!("unknown share-state tag {other} (supported: plaintext v2={TAG_PLAINTEXT_V2}, encrypted v2={TAG_ENCRYPTED_V2}; v1 tags {TAG_PLAINTEXT}/{TAG_ENCRYPTED} are RETIRED for the share file and re-run the ceremony)")
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

/// Persist the memoized share for `epoch` under `dir`, mode 0600, framed per
/// `state`. The encoded bytes (which embed the secret share) are scrubbed on drop.
///
/// NOT best-effort to its caller any more: the error is returned as it always was,
/// but `DkgActor::adopt_share` now treats it as a REFUSAL rather than a warning —
/// see §5.4 of `.dpos-study/history/E5-BEACON-DESIGN.md` and that function's doc
/// for why "signing now, mute after a restart" is the outcome that rule exists to
/// forbid.
pub fn persist(dir: &Path, epoch: u64, share: &Share, state: &ShareState) -> eyre::Result<()> {
    std::fs::create_dir_all(dir).map_err(|e| eyre::eyre!("create share dir {dir:?}: {e}"))?;
    let bytes = Zeroizing::new(encode(state, epoch, share));
    fluentbase_bls::secret_store::write_mode_0600(&file_for(dir, epoch), &bytes)
        .map_err(|e| eyre::eyre!("write share file: {e}"))
}

/// One reloaded share record: the minting epoch and this node's share at it.
pub(crate) type ReloadedShare = (u64, Share);

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
            Ok(share) => out.push((epoch, share)),
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
    /// EVIDENCE that one dealer signed two distinct `check`-valid logs for this epoch:
    /// `(first, second)` in the order this node recorded them. Written INSTEAD of a
    /// `PeerLog` for the second log on the live paths, so the one record that
    /// restores the local ban after a restart is the record that restores the second
    /// body. Replay records through the same rule as the live path, so a journal
    /// that holds BOTH bodies as plain `PeerLog`s (the past-boundary heal writes
    /// those, `actor.rs::ingest_recompute_log`) proves the pair again without this
    /// record — this record is what makes the pair durable when only ONE body is
    /// journaled on its own. Self-contained on replay (`DkgCeremony::resume`): both
    /// halves must re-`check` as the SAME dealer under DIFFERENT hashes, else the
    /// record is dropped whole and neither body is recorded from it. Public data
    /// (signed logs), framed uniformly with the rest; the codec (`parse_journal_inner`)
    /// decodes the two bodies and checks nothing — the replay does.
    DealerEquivocation(Box<DealerReveal>, Box<DealerReveal>),
}

const REC_RECEIVED_DEALING: u8 = 0;
const REC_OWN_SEAL: u8 = 1;
const REC_PEER_LOG: u8 = 2;
const REC_OWN_DEALER_ACK: u8 = 3;
const REC_DEALER_EQUIVOCATION: u8 = 4;

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
        JournalRecord::DealerEquivocation(first, second) => {
            buf.push(REC_DEALER_EQUIVOCATION);
            first.write(&mut buf);
            second.write(&mut buf);
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
        REC_DEALER_EQUIVOCATION => {
            let first = DealerReveal::read_cfg(&mut body, &committee_size)
                .map_err(|e| eyre::eyre!("parse journal equivocation first log: {e:?}"))?;
            let second = DealerReveal::read_cfg(&mut body, &committee_size)
                .map_err(|e| eyre::eyre!("parse journal equivocation second log: {e:?}"))?;
            JournalRecord::DealerEquivocation(Box::new(first), Box::new(second))
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
/// from a PRESENT-but-damaged journal, so the caller (`actor::recover`) never
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

fn conflict_file_for(dir: &Path, epoch: u64) -> PathBuf {
    dir.join(format!("{CONFLICT_PREFIX}{epoch}{FILE_SUFFIX}"))
}

/// Record `Conflict(E)` durably: `held ‖ second`, the value digests of the two
/// quorum-certified artifacts, 64 plaintext bytes (both are public). Written by
/// the artifact STORE the instant it notes a second value
/// (`ArtifactStore::note_divergent` — the store owns the witness, so no restart
/// window between the note and the actor's next tick can lose it), and by the
/// actor as it enters the terminal when the store has not (`stop_signing`,
/// BEFORE the share file is evicted). Read back by `DkgActor::recover` BEFORE
/// anything else it holds for the epoch — a share file that outlived the
/// terminal (an `evict_share` that failed, a death between the marker and the
/// eviction) must not re-key the epoch on a restart. Reclaimed with the epoch's
/// journal (`reconcile_journals` on the first tick, `sweep_epoch_state` after).
pub(crate) fn persist_conflict(
    dir: &Path,
    epoch: u64,
    held: &B256,
    second: &B256,
) -> eyre::Result<()> {
    std::fs::create_dir_all(dir).map_err(|e| eyre::eyre!("create share dir {dir:?}: {e}"))?;
    let mut bytes = Vec::with_capacity(64);
    bytes.extend_from_slice(held.as_slice());
    bytes.extend_from_slice(second.as_slice());
    let path = conflict_file_for(dir, epoch);
    std::fs::write(&path, bytes).map_err(|e| eyre::eyre!("write conflict marker {path:?}: {e}"))?;
    std::fs::File::open(&path)
        .and_then(|f| f.sync_all())
        .map_err(|e| eyre::eyre!("sync conflict marker {path:?}: {e}"))
}

/// What a `Conflict(E)` marker on disk says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConflictMarker {
    /// `(held, second)` — the value digests of the two certified artifacts.
    Pair(B256, B256),
    /// The file is there but is not 64 bytes. FAIL-CLOSED: only a verdict ever
    /// writes the file, so the verdict stands even though its digests are lost;
    /// the epoch is `Conflict` with no known pair, said as an ERROR, and the
    /// file is left in place for the operator.
    Malformed,
}

/// The `Conflict(E)` marker, if one is on disk.
pub(crate) fn load_conflict(dir: &Path, epoch: u64) -> Option<ConflictMarker> {
    let bytes = std::fs::read(conflict_file_for(dir, epoch)).ok()?;
    if bytes.len() != 64 {
        tracing::error!(
            epoch,
            len = bytes.len(),
            "beacon: MALFORMED conflict marker — the epoch's signing stays stopped \
             (fail-closed); the file is left in place"
        );
        return Some(ConflictMarker::Malformed);
    }
    Some(ConflictMarker::Pair(
        B256::from_slice(&bytes[..32]),
        B256::from_slice(&bytes[32..]),
    ))
}

/// Every `Conflict(E)` marker under `dir`, ascending by epoch — what the artifact
/// store reloads its divergence witnesses from at launch.
pub(crate) fn conflict_markers(dir: &Path) -> Vec<(u64, ConflictMarker)> {
    let mut epochs = scan_beacon_dir(dir).2;
    epochs.sort_unstable();
    epochs
        .into_iter()
        .filter_map(|epoch| Some((epoch, load_conflict(dir, epoch)?)))
        .collect()
}

/// Delete the `Conflict(E)` marker once the epoch has aged out of the window.
pub(crate) fn evict_conflict(dir: &Path, epoch: u64) {
    let path = conflict_file_for(dir, epoch);
    if let Err(e) = std::fs::remove_file(&path) {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(epoch, ?e, "beacon: failed to evict conflict marker");
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
    let (journals, shares, conflicts) = scan_beacon_dir(dir);
    for epoch in journals {
        if epoch + crate::beacon::JOURNAL_RETENTION_EPOCHS < now {
            evict_journal(dir, epoch);
        }
    }
    // A conflict marker rides the journal's window: the terminal it records is
    // an epoch's, and an epoch past the window has no slot to be terminal in.
    for epoch in conflicts {
        if epoch + crate::beacon::JOURNAL_RETENTION_EPOCHS < now {
            evict_conflict(dir, epoch);
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

/// `(journal epochs, share epochs, conflict-marker epochs)` parsed out of ONE
/// directory scan. A missing dir yields three empty sets; malformed / foreign
/// filenames are ignored.
fn scan_beacon_dir(dir: &Path) -> (Vec<u64>, Vec<u64>, Vec<u64>) {
    let mut journals: Vec<u64> = Vec::new();
    let mut shares: Vec<u64> = Vec::new();
    let mut conflicts: Vec<u64> = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (journals, shares, conflicts);
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
        } else if let Some(epoch) = name
            .strip_prefix(CONFLICT_PREFIX)
            .and_then(|s| s.strip_suffix(FILE_SUFFIX))
            .and_then(|s| s.parse::<u64>().ok())
        {
            conflicts.push(epoch);
        }
    }
    (journals, shares, conflicts)
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

    /// The v1 inner frame this module no longer writes, rebuilt by hand so the
    /// retired-format tests exercise real legacy bytes rather than a v1 encoder
    /// kept alive only for them.
    fn v1_inner(output: &CeremonyOutput, share: &Share) -> Vec<u8> {
        let out_bytes = encode_outcome(output);
        let mut buf = (out_bytes.len() as u32).to_be_bytes().to_vec();
        buf.extend_from_slice(&out_bytes);
        buf.extend_from_slice(share.encode().as_ref());
        buf
    }

    /// The v2 plaintext frame, pinned byte-for-byte: ONE length-prefixed field — the
    /// share — and nothing after it.
    ///
    /// Reds if ANY second field comes back. That is the assertion П-3 needs from this
    /// module: both of the copies of `PK_E` this record used to carry (the group
    /// output, and the artifact after it) were length-prefixed tails, so their
    /// absence is checkable as a byte count rather than only as an API shape.
    #[test]
    fn plaintext_v2_frame_is_pinned() {
        let (_output, share) = sample_output_share();
        let bytes = encode(&ShareState::Plaintext, 7, &share);
        let share_bytes = share.encode();
        let mut expected = vec![TAG_PLAINTEXT_V2];
        expected.extend_from_slice(&(share_bytes.len() as u32).to_be_bytes());
        expected.extend_from_slice(share_bytes.as_ref());
        assert_eq!(bytes, expected);
    }

    /// A v1 file is no longer readable, and the outcome is the warn-and-skip one —
    /// the node re-runs the ceremony rather than crashing or adopting a share it
    /// cannot frame. Both arms, because the encrypted one used to open on its own
    /// AAD and must now fail on the tag alone.
    ///
    /// Reds if the v1 share arms come back: a reader for them would resurrect a
    /// frame the write path cannot produce.
    #[test]
    fn a_v1_share_file_is_retired_and_skipped_not_read() {
        let (output, share) = sample_output_share();
        let mut plain = vec![TAG_PLAINTEXT];
        plain.extend_from_slice(&v1_inner(&output, &share));
        assert!(
            decode(&plain, 7, &ShareState::Plaintext).is_err(),
            "the v1 plaintext arm is deleted, so its tag is unknown"
        );

        let key = seal_key(303);
        let sealed = seal_envelope(
            &key,
            TAG_ENCRYPTED,
            &tagged_aad(TAG_ENCRYPTED, 7),
            &v1_inner(&output, &share),
        );
        assert!(
            decode(&sealed, 7, &ShareState::Encrypted(key.clone())).is_err(),
            "and so is the v1 encrypted arm, holder of the key or not"
        );
        // Non-vacuity: the SAME reader accepts a current record, so the two
        // refusals above are about the retired frame and not about this fixture.
        let current = encode(&ShareState::Encrypted(key.clone()), 7, &share);
        assert!(decode(&current, 7, &ShareState::Encrypted(key)).is_ok());
    }

    /// A PRE-П-3 v2 record — `(output, share)` or `(output, share, artifact)` — must be
    /// REFUSED rather than read with its tail ignored.
    ///
    /// It is the one legacy shape whose leading tag still matches, so the
    /// trailing-byte check is the only thing standing between it and a record whose
    /// third field would be read as nothing at all. The outcome is `load_all`'s
    /// warn-and-skip, stated in this module's doc.
    #[test]
    fn a_pre_p3_record_is_refused_not_silently_truncated() {
        let (output, share) = sample_output_share();
        // Exactly the old `inner_frame`: output, then share, then the artifact.
        let mut legacy = vec![TAG_PLAINTEXT_V2];
        let out_bytes = encode_outcome(&output);
        legacy.extend_from_slice(&(out_bytes.len() as u32).to_be_bytes());
        legacy.extend_from_slice(&out_bytes);
        let share_bytes = share.encode();
        legacy.extend_from_slice(&(share_bytes.len() as u32).to_be_bytes());
        legacy.extend_from_slice(share_bytes.as_ref());
        let artifact = b"an encoded agreed artifact";
        legacy.extend_from_slice(&(artifact.len() as u32).to_be_bytes());
        legacy.extend_from_slice(artifact);
        assert!(
            decode(&legacy, 7, &ShareState::Plaintext).is_err(),
            "a pre-П-3 record leads with the OUTPUT's length prefix, and reading it as \
             a current one would accept a frame this binary cannot write"
        );

        let dir = fresh_dir("legacy-3field");
        std::fs::create_dir_all(&dir).unwrap();
        fluentbase_bls::secret_store::write_mode_0600(&file_for(&dir, 7), &legacy).unwrap();
        assert!(
            load_all(&dir, &ShareState::Plaintext).is_empty(),
            "and the startup outcome is a skip, never an abort"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The other direction of tag-as-version: a v2 file met by a binary that knows
    /// only the v1 tags falls into the unknown-tag arm, which `load_all` turns into
    /// a warn-and-skip — the node re-runs the ceremony instead of crashing.
    #[test]
    fn v2_file_is_unreadable_to_a_v1_only_reader_and_skipped_not_fatal() {
        let (_output, share) = sample_output_share();
        let dir = fresh_dir("v1-reader");
        persist(&dir, 7, &share, &ShareState::Plaintext).expect("persist");

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
        let (_output, share) = sample_output_share();
        let mut bytes = encode(&ShareState::Plaintext, 7, &share);
        bytes.push(0);
        assert!(decode(&bytes, 7, &ShareState::Plaintext).is_err());
    }

    #[test]
    fn plaintext_persist_load_round_trip() {
        let (_output, share) = sample_output_share();
        let dir = fresh_dir("plain");
        persist(&dir, 7, &share, &ShareState::Plaintext).expect("persist");

        let loaded = load_all(&dir, &ShareState::Plaintext);
        assert_eq!(loaded.len(), 1);
        let (epoch, share2) = &loaded[0];
        assert_eq!(*epoch, 7);
        assert_eq!(share2.encode().as_ref(), share.encode().as_ref());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn encrypted_persist_load_round_trip() {
        let (_output, share) = sample_output_share();
        let key = seal_key(101);
        let dir = fresh_dir("enc");
        persist(&dir, 7, &share, &ShareState::Encrypted(key.clone())).expect("persist");

        let on_disk = std::fs::read(file_for(&dir, 7)).unwrap();
        assert_eq!(
            on_disk[0], TAG_ENCRYPTED_V2,
            "encrypted file leads with TAG_ENCRYPTED_V2"
        );

        let loaded = load_all(&dir, &ShareState::Encrypted(key));
        assert_eq!(loaded.len(), 1, "encrypted share round-trips");
        let (epoch, share2) = &loaded[0];
        assert_eq!(*epoch, 7);
        assert_eq!(share2.encode().as_ref(), share.encode().as_ref());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An encrypted file loaded with the WRONG key OR with `Plaintext` state is
    /// SKIPPED (warn), not panicked.
    #[test]
    fn encrypted_file_with_wrong_or_absent_key_is_skipped() {
        let (_output, share) = sample_output_share();
        let dir = fresh_dir("enc-wrongkey");
        persist(&dir, 7, &share, &ShareState::Encrypted(seal_key(1))).expect("persist");

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
        let (_output, share) = sample_output_share();
        let key = seal_key(55);
        let dir = fresh_dir("enc-rename");
        persist(&dir, 7, &share, &ShareState::Encrypted(key.clone())).expect("persist");
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
        let (_output, share) = sample_output_share();
        let mut bytes = encode(&ShareState::Plaintext, 7, &share);
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
        let mut player = Player::new(info.clone(), player_key.clone()).expect("player");
        let ack = player
            .dealer_message::<N3f1>(dealer_key.public_key(), pub_msg.clone(), priv_msg.clone())
            .expect("ack");
        dealer
            .receive_player_ack(player_pk.clone(), ack.clone())
            .expect("receive ack");
        let log = dealer.finalize::<N3f1>();
        // A second, independently dealt log of the SAME dealer (no acks ⇒ all
        // reveals) — the other half of an equivocation pair.
        let (second_dealer, _, _) =
            Dealer::start::<N3f1>(&mut rng, info.clone(), dealer_key.clone(), None)
                .expect("second start");
        let second = second_dealer.finalize::<N3f1>();
        assert_ne!(second.encode(), log.encode(), "the two logs differ");

        let records = vec![
            JournalRecord::ReceivedDealing(
                dealer_key.public_key(),
                Box::new(pub_msg),
                Box::new(priv_msg),
            ),
            JournalRecord::OwnDealerAck(player_pk, Box::new(ack)),
            JournalRecord::OwnSeal(Box::new(log.clone())),
            JournalRecord::PeerLog(Box::new(log.clone())),
            JournalRecord::DealerEquivocation(Box::new(log), Box::new(second)),
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
                (
                    JournalRecord::DealerEquivocation(f0, s0),
                    JournalRecord::DealerEquivocation(f1, s1),
                ) => {
                    assert_eq!(f0.encode(), f1.encode(), "{state}: equivocation first log");
                    assert_eq!(s0.encode(), s1.encode(), "{state}: equivocation second log");
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
    /// boundary), and never touches a foreign file.
    ///
    /// The heights are DERIVED from the window rather than written down, because the
    /// window is now the crate's one retention constant (`beacon/mod.rs`) and a test
    /// that hard-codes `1` pins a number instead of the predicate. `now` sits one
    /// epoch above the aged-out one's edge, so exactly one of the three is past it.
    #[test]
    fn reconcile_journals_deletes_past_window_keeps_in_window_ignores_foreign() {
        let (records, _n) = sample_journal_records();
        let dir = fresh_dir("journal-reconcile");
        let window = crate::beacon::JOURNAL_RETENTION_EPOCHS;
        let aged_out = 3u64;
        // `aged_out + window < now` by exactly one, so `in_window + window >= now`.
        let now = aged_out + window + 1;
        let in_window = now;
        let future = now + 2;
        for e in [aged_out, in_window, future] {
            append_journal(&dir, e, &records[0], &ShareState::Plaintext).expect("append");
        }
        let foreign = dir.join("some-other-file.bin");
        std::fs::write(&foreign, b"keep me").expect("write foreign");

        reconcile_journals(&dir, now);

        assert!(
            !journal_file_for(&dir, aged_out).exists(),
            "e{aged_out} + RET({window}) < now={now} → aged out of the recompute window, deleted"
        );
        assert!(
            journal_file_for(&dir, in_window).exists(),
            "e{in_window} + RET({window}) >= now={now} → still in the recompute window, kept"
        );
        assert!(
            journal_file_for(&dir, future).exists(),
            "e{future} > now={now} → future, kept"
        );
        assert!(foreign.exists(), "a foreign filename is never deleted");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The `Conflict(E)` marker round-trips, a malformed one is a verdict with no
    /// pair (fail-closed, never deleted), and a marker ages out with the journal
    /// window.
    #[test]
    fn a_conflict_marker_round_trips_and_ages_out_with_the_journal() {
        let dir = fresh_dir("conflict-marker");
        let (held, second) = (B256::repeat_byte(0xA1), B256::repeat_byte(0xB2));
        let window = crate::beacon::JOURNAL_RETENTION_EPOCHS;
        let aged_out = 3u64;
        let now = aged_out + window + 1;
        persist_conflict(&dir, aged_out, &held, &second).expect("persist");
        persist_conflict(&dir, now, &held, &second).expect("persist");
        assert_eq!(
            load_conflict(&dir, now),
            Some(ConflictMarker::Pair(held, second))
        );
        assert_eq!(load_conflict(&dir, now + 1), None, "no marker, no verdict");
        std::fs::write(conflict_file_for(&dir, now + 2), b"short").expect("write");
        assert_eq!(
            load_conflict(&dir, now + 2),
            Some(ConflictMarker::Malformed),
            "malformed: still a verdict (fail-closed), never a pair"
        );
        assert_eq!(
            conflict_markers(&dir),
            vec![
                (aged_out, ConflictMarker::Pair(held, second)),
                (now, ConflictMarker::Pair(held, second)),
                (now + 2, ConflictMarker::Malformed),
            ],
            "the scan reports every marker, malformed ones included"
        );

        reconcile_journals(&dir, now);
        assert_eq!(
            load_conflict(&dir, aged_out),
            None,
            "aged out with the journal"
        );
        assert_eq!(
            load_conflict(&dir, now),
            Some(ConflictMarker::Pair(held, second)),
            "in window: kept"
        );
        assert!(
            conflict_file_for(&dir, now + 2).exists(),
            "a malformed marker is left in place"
        );
        evict_conflict(&dir, now);
        assert_eq!(load_conflict(&dir, now), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `reconcile_journals` prunes SUPERSEDED share secrets ([334]): it keeps the ACTIVE
    /// carry-forward (`max{share epoch <= now}`) AND every FUTURE share (`> now`, a
    /// just-finalized next-epoch key), deleting only strictly-older shares. Keeping only the
    /// max share would WRONGLY delete the active `<= now` carry-forward whenever a future
    /// share exists (normal at a committee-change boundary) — this pins the floor rule.
    #[test]
    fn reconcile_prunes_superseded_shares_keeps_active_and_future() {
        let (_output, share) = sample_output_share();
        let dir = fresh_dir("share-reconcile");
        // Shares {3, 5, 7}, now=6 → floor = max{e<=6} = 5 (active), 7 is future.
        for e in [3u64, 5, 7] {
            persist(&dir, e, &share, &ShareState::Plaintext).expect("persist");
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
        let (_output, share) = sample_output_share();
        let dir = fresh_dir("share-all-future");
        for e in [8u64, 9] {
            persist(&dir, e, &share, &ShareState::Plaintext).expect("persist");
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
