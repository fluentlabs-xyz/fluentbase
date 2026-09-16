//! On-disk persistence of a live-DKG per-epoch `Share`.
//!
//! The live DKG memoizes `(PK_E, share)` for each committee-change epoch in the
//! in-memory [`crate::beacon::actor::CeremonyStore`]. A mid-epoch restart
//! otherwise loses it (the node then carries-forward the wrong key for E and
//! stalls its own seed votes). This module persists each memoized share to disk
//! (mode 0600) and reloads them at launch, so a restarted committee member
//! rejoins the seed quorum without re-running the ceremony.
//!
//! The record is the share and nothing else: the polynomial a reloaded share
//! pairs with is read from the artifact of the same minting epoch
//! (`KeyIndex::sharing_at`), so this file never becomes a second on-disk owner of
//! `PK_E`. A node that restarts without that artifact acquires the epoch key from
//! peers.
//!
//! Frames are dispatched on a leading 1-byte tag. The tagless inner frame is
//! `u32_be(len(share)) ‖ share`: `TAG_PLAINTEXT_V2` writes `tag ‖ inner`, and
//! `TAG_ENCRYPTED_V2` seals `inner` as the AEAD plaintext.
//!
//! The encrypted arm (keystore mode) writes
//! `tag(1) ‖ version(1) ‖ nonce(24) ‖ XChaCha20-Poly1305(key, nonce, aad, inner)`
//! with `aad = tag ‖ version ‖ u64_be(epoch)`, binding the ciphertext to its
//! epoch and frame version; the key is [`fluentbase_bls::ShareSealKey`],
//! HKDF-derived from the validator BLS secret. The plaintext-dev BLS path
//! (`--dpos.bls-key-path`) has no off-disk secret and stays plaintext.
//!
//! An older two- or three-field record, or one using the retired v1 tags, fails
//! to decode — the v1 tags are unknown to the reader and the extra fields are
//! trailing bytes — and takes `load_all`'s warn-and-skip path rather than
//! aborting startup or adopting a wrong share.

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

pub enum ShareState {
    /// Unencrypted framing (`--dpos.bls-key-path` plaintext-dev nodes).
    Plaintext,
    /// XChaCha20-Poly1305 envelope keyed by the HKDF-derived [`ShareSealKey`]
    /// (keystore-mode validators).
    Encrypted(ShareSealKey),
}

const TAG_PLAINTEXT: u8 = 0;
const TAG_ENCRYPTED: u8 = 1;
const TAG_PLAINTEXT_V2: u8 = 2;
const TAG_ENCRYPTED_V2: u8 = 3;
/// Envelope version inside an encrypted frame — lets the AEAD/KDF params evolve
/// without colliding with the tag byte.
const ENVELOPE_VERSION: u8 = 1;
const NONCE_BYTES: usize = 24;

const FILE_PREFIX: &str = "beacon-share-e";
const FILE_SUFFIX: &str = ".bin";
/// The durable `Conflict(E)` marker: the epoch's signing was stopped on this node.
const CONFLICT_PREFIX: &str = "beacon-conflict-e";

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

/// `what` names the field in the error so a truncated file says which field ran
/// out.
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

/// Trailing bytes are rejected: an older record that carried the group output or
/// artifact as extra fields reads as trailing bytes and is refused rather than
/// silently truncated.
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

/// `tag` and `aad` MUST agree (the AAD's first byte is the tag), so one frame
/// version's ciphertext cannot open as another's.
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

/// `rest` is the envelope body without its leading tag; errors on a bad version, a
/// short nonce, or AEAD failure (wrong key, tampered, or wrong-epoch AAD).
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

/// Always writes the v2 frame.
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

/// Dispatch is on the leading tag, not on `state`: a plaintext file always
/// decodes, while an encrypted file requires `state == Encrypted(key)`. The retired
/// v1 tags are unknown here, and `load_all` turns any error into a warn-and-skip.
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

/// Persist the memoized share for `epoch` under `dir`, mode 0600. `DkgActor::adopt_share`
/// treats an error as a refusal to sign, not a warning.
pub fn persist(dir: &Path, epoch: u64, share: &Share, state: &ShareState) -> eyre::Result<()> {
    std::fs::create_dir_all(dir).map_err(|e| eyre::eyre!("create share dir {dir:?}: {e}"))?;
    let bytes = Zeroizing::new(encode(state, epoch, share));
    fluentbase_bls::secret_store::write_mode_0600(&file_for(dir, epoch), &bytes)
        .map_err(|e| eyre::eyre!("write share file: {e}"))
}

/// One reloaded share record: the minting epoch and this node's share at it.
pub(crate) type ReloadedShare = (u64, Share);

/// A missing dir yields empty; a malformed or undecryptable file is skipped with a
/// warning, never aborting startup — the in-memory store rebuilds via the next
/// ceremony or carry-forward.
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
        // Refuse a group/other-accessible share file (e.g. recreated at 0644 by a
        // backup) rather than load a leaked secret.
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
/// `Player::resume` instead of re-dealing a divergent contribution.
///
/// `ReceivedDealing` carries a per-dealer `DealerPrivMsg`, so on a keystore-mode
/// node the journal MUST seal with the same `ShareState::Encrypted` AEAD as the
/// share file.
pub(crate) enum JournalRecord {
    /// A dealing this node accepted; `Player::resume` re-feeds it to rebuild
    /// `Player.view` and re-emit acks.
    ReceivedDealing(PeerPubkey, Box<DealerPubMsg<MinSig>>, Box<DealerPrivMsg>),
    /// This node's own sealed log; on resume the node re-broadcasts it rather than
    /// re-dealing a fresh, divergent one.
    OwnSeal(Box<DealerReveal>),
    /// A peer's sealed log, fed into `Logs` for `select`/`finalize` and into
    /// `Player::resume`, which needs it to finalize when that peer revealed our
    /// share.
    PeerLog(Box<DealerReveal>),
    /// An ack this node's dealer received; journaled so a pre-seal resume rebuilds
    /// `unsent` and does not re-reveal an already-acked player.
    OwnDealerAck(PeerPubkey, Box<Ack>),
    /// Evidence that one dealer signed two distinct `check`-valid logs for this
    /// epoch, `(first, second)` in the order this node recorded them. Written in
    /// place of a second `PeerLog` so a restart restores both the ban and the second
    /// body. Replay re-`check`s both halves as the same dealer under different
    /// hashes and drops the record whole otherwise.
    DealerEquivocation(Box<DealerReveal>, Box<DealerReveal>),
}

const REC_RECEIVED_DEALING: u8 = 0;
const REC_OWN_SEAL: u8 = 1;
const REC_PEER_LOG: u8 = 2;
const REC_OWN_DEALER_ACK: u8 = 3;
const REC_DEALER_EQUIVOCATION: u8 = 4;

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

/// AAD binding a journal-record ciphertext to its epoch. Its first byte
/// (`TAG_ENCRYPTED ^ 0x80`) domain-separates it from every share-file AAD, so
/// neither can AEAD-open as the other; the high bit keeps that true as share tags
/// keep counting up.
fn journal_aad(epoch: u64) -> [u8; 10] {
    let mut aad = tagged_aad(TAG_ENCRYPTED, epoch);
    aad[0] ^= 0x80;
    aad
}

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

/// `committee_size` is an upper bound on the DKG decoders, as on the wire path.
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

/// Append one ceremony [`JournalRecord`] for `epoch`, mode 0600. Best-effort at the
/// call site: the in-memory ceremony is authoritative for the running process, and
/// only a crash loses an unwritten tail.
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

/// Result of [`load_journal`]: distinguishes a genuine first run from a
/// present-but-damaged journal, so the caller never re-deals an already-sealed
/// epoch. A non-empty journal proves this node participated, and a second sealed
/// log of the same dealer is self-equivocation.
pub(crate) enum JournalLoad {
    /// No journal file exists: a genuine first run, so the caller may deal.
    NoFile,
    /// A journal file decoded; the records are the recoverable prefix, and the
    /// caller resumes player-only rather than re-dealing.
    Present(Vec<JournalRecord>),
    /// A journal file exists but its first record is undecodable (torn length
    /// prefix, truncated body, or undecryptable — e.g. wrong keystore mode). The
    /// caller evicts it: at or after the seal deadline it heals as a player, and
    /// before the deadline it starts fresh.
    Torn,
}

/// A malformed or undecryptable record truncates the read at that point — the tail
/// is the crash-lost part — with a warning; never an abort.
pub(crate) fn load_journal(
    dir: &Path,
    epoch: u64,
    state: &ShareState,
    committee_size: NonZeroU32,
) -> JournalLoad {
    let Ok(bytes) = std::fs::read(journal_file_for(dir, epoch)) else {
        return JournalLoad::NoFile;
    };
    // A 0-byte file is a genuine first run, not a torn record: the create succeeded
    // but no dealing was committed, so the caller may still deal. Only a non-empty
    // file whose first record fails to decode is `Torn` (we wrote it, so re-dealing
    // would self-equivocate).
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

/// Delete a superseded per-epoch share secret file; `reconcile_journals` prunes
/// older shares once carry-forward has moved on.
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

/// Record `Conflict(E)` durably as `held ‖ second`, the value digests of the two
/// quorum-certified artifacts (64 public bytes). `DkgActor::recover` reads it before
/// any share for the epoch, so a share file that outlived the terminal cannot re-key
/// the epoch on a restart.
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
    /// The file is present but not 64 bytes. Fail-closed: only a verdict writes the
    /// file, so the verdict stands even though its digests are lost, and the file is
    /// left in place for the operator.
    Malformed,
}

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

/// Every `Conflict(E)` marker under `dir`, ascending by epoch.
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

/// Reconcile the on-disk beacon directory on the first tick: prune boundary-passed
/// ceremony journals and superseded share secrets that no in-memory map holds.
/// This is the durable lifetime owner — a finalize-then-restart-before-boundary
/// leaves the epoch in no in-memory map, so without this its files leak.
///
/// A journal is deleted when `epoch + JOURNAL_RETENTION_EPOCHS < now`, the same
/// predicate the running sweep applies. Shares keep the active carry-forward (the
/// max share epoch `<= now`) and every future share (`> now`); only strictly older
/// shares are deleted, because keeping only the max would delete the active
/// carry-forward whenever a future share exists and demote the node for the rest of
/// the epoch. A missing dir is a no-op; malformed or foreign filenames are ignored,
/// never deleted.
pub(crate) fn reconcile_journals(dir: &Path, now: u64) {
    let (journals, shares, conflicts) = scan_beacon_dir(dir);
    for epoch in journals {
        if epoch + crate::beacon::JOURNAL_RETENTION_EPOCHS < now {
            evict_journal(dir, epoch);
        }
    }
    // A conflict marker rides the journal's window: an epoch past the window has no
    // slot to be terminal in.
    for epoch in conflicts {
        if epoch + crate::beacon::JOURNAL_RETENTION_EPOCHS < now {
            evict_conflict(dir, epoch);
        }
    }
    if let Some(floor) = shares.iter().copied().filter(|e| *e <= now).max() {
        for epoch in shares {
            if epoch < floor {
                evict_share(dir, epoch);
            }
        }
    }
}

/// Returns `(journal epochs, share epochs, conflict-marker epochs)` parsed out of
/// one directory scan.
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

    /// The v1 inner frame, rebuilt by hand so the retired-format tests exercise real
    /// legacy bytes rather than a v1 encoder kept alive only for them.
    fn v1_inner(output: &CeremonyOutput, share: &Share) -> Vec<u8> {
        let out_bytes = encode_outcome(output);
        let mut buf = (out_bytes.len() as u32).to_be_bytes().to_vec();
        buf.extend_from_slice(&out_bytes);
        buf.extend_from_slice(share.encode().as_ref());
        buf
    }

    /// The v2 plaintext frame is pinned byte-for-byte: one length-prefixed field —
    /// the share — and nothing after it.
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

    /// A v1 file is unreadable on both arms; the node re-runs the ceremony rather
    /// than crashing or adopting a share it cannot frame.
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
        // Non-vacuity: the same reader accepts a current record, so the refusals
        // above are about the retired frame.
        let current = encode(&ShareState::Encrypted(key.clone()), 7, &share);
        assert!(decode(&current, 7, &ShareState::Encrypted(key)).is_ok());
    }

    /// An older v2 record carrying the output and/or artifact past the share must be
    /// refused, not read with its tail ignored — it is the one legacy shape whose
    /// leading tag still matches.
    #[test]
    fn a_pre_p3_record_is_refused_not_silently_truncated() {
        let (output, share) = sample_output_share();
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

    /// A v2 file read by a v1-only reader hits the unknown-tag arm and is skipped
    /// with a warning, so the node re-runs the ceremony instead of crashing.
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

        std::fs::write(file_for(&dir, 7), [0xFFu8; 8]).unwrap();
        assert!(
            load_all(&dir, &ShareState::Plaintext).is_empty(),
            "an unknown tag is skipped with a warning, never an abort"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// v2 must keep rejecting a corrupted tail even though the share is
    /// length-prefixed.
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

    /// An encrypted file loaded with the wrong key or with `Plaintext` state is
    /// skipped with a warning, not a panic.
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

        assert!(
            load_all(&dir, &ShareState::Encrypted(key.clone())).is_empty(),
            "a ciphertext renamed e7 → e9 fails AEAD (epoch-bound AAD)"
        );
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
        // A second independently dealt log of the same dealer — the other half of an
        // equivocation pair.
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
        // A present file whose first record never decodes is `Torn`: we wrote it, so
        // it must never be `NoFile`.
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
        let loaded = present(load_journal(&dir, 7, &state, n));
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

    /// `reconcile_journals(now)` deletes only journals that have aged out of the
    /// recompute-heal window (`epoch + JOURNAL_RETENTION_EPOCHS < now`), keeps the
    /// in-window ones so a demoted member can still recompute from them past the
    /// boundary, and never touches a foreign file. The heights are derived from the
    /// window rather than written down, so the test pins the predicate and not a
    /// number.
    #[test]
    fn reconcile_journals_deletes_past_window_keeps_in_window_ignores_foreign() {
        let (records, _n) = sample_journal_records();
        let dir = fresh_dir("journal-reconcile");
        let window = crate::beacon::JOURNAL_RETENTION_EPOCHS;
        let aged_out = 3u64;
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

    /// `reconcile_journals` keeps the active carry-forward (`max{share epoch <= now}`)
    /// and every future share (`> now`), deleting only strictly older shares; keeping
    /// only the max would drop the active carry-forward whenever a future share exists.
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

    /// Floor-rule edge: when every share is in the future (`> now`) there is no
    /// `<= now` floor, so reconcile deletes nothing.
    #[test]
    fn reconcile_keeps_all_when_every_share_is_future() {
        let (_output, share) = sample_output_share();
        let dir = fresh_dir("share-all-future");
        for e in [8u64, 9] {
            persist(&dir, e, &share, &ShareState::Plaintext).expect("persist");
        }
        reconcile_journals(&dir, 5); // now=5, both shares are future
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

    /// A 0-byte journal (create succeeded, no bytes written) is `NoFile` for a
    /// genuine first run, not `Torn`.
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
