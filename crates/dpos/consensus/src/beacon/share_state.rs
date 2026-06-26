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
    dkg_msg::DealerReveal,
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

/// Seal `inner` into a `TAG_ENCRYPTED` envelope under `key` for `aad`:
/// `tag(1) ‖ version(1) ‖ nonce(24) ‖ XChaCha20-Poly1305(key, nonce, aad, inner)`.
/// Shared by the share-file and the journal-record paths — they differ ONLY in the
/// AAD (domain-separated by its first byte) and the inner payload.
fn seal_envelope(key: &ShareSealKey, aad: &[u8], inner: &[u8]) -> Vec<u8> {
    let cipher = XChaCha20Poly1305::new(key.as_bytes().into());
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    // AEAD-encrypt of a single in-memory buffer cannot fail in this cipher.
    let ct = cipher
        .encrypt(&nonce, Payload { msg: inner, aad })
        .expect("XChaCha20-Poly1305 encrypt of an in-memory buffer is infallible");
    let mut buf = Vec::with_capacity(1 + 1 + NONCE_BYTES + ct.len());
    buf.push(TAG_ENCRYPTED);
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
                eyre::eyre!("encrypted envelope AEAD-open failed (wrong key, tampered, or wrong epoch)")
            })?,
    ))
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
        ShareState::Encrypted(key) => seal_envelope(key, &encrypted_aad(epoch), &inner),
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
            let inner = open_envelope(key, &encrypted_aad(epoch), rest)?;
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

const JOURNAL_PREFIX: &str = "beacon-dkgjournal-e";

/// One durable record of a mid-window DKG ceremony, journaled so a restart can
/// `Player::resume` instead of re-dealing a divergent contribution (§8.11.1).
///
/// The `ReceivedDealing` body carries a per-dealer `DealerPrivMsg` (secret-
/// equivalent) — so on a keystore-mode node the journal MUST seal with the same
/// `ShareState::Encrypted` AEAD as the share file (gated identically), never
/// plaintext.
pub enum JournalRecord {
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
}

const REC_RECEIVED_DEALING: u8 = 0;
const REC_OWN_SEAL: u8 = 1;
const REC_PEER_LOG: u8 = 2;

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
    }
    buf
}

/// AAD binding a journal-record ciphertext to its epoch. A distinct first byte
/// (`TAG_ENCRYPTED ^ 0x80`) domain-separates it from the share-file AAD so a
/// share ciphertext can never AEAD-open as a journal record (or vice-versa).
fn journal_aad(epoch: u64) -> [u8; 10] {
    let mut aad = encrypted_aad(epoch);
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
        ShareState::Encrypted(key) => seal_envelope(key, &journal_aad(epoch), &inner),
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
pub fn append_journal(
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
pub enum JournalLoad {
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
pub fn load_journal(
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
pub fn evict_journal(dir: &Path, epoch: u64) {
    let path = journal_file_for(dir, epoch);
    if let Err(e) = std::fs::remove_file(&path) {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(epoch, ?e, "beacon: failed to evict DKG journal");
        }
    }
}

/// Delete a superseded per-epoch share secret file (`reconcile_journals` prunes the
/// older shares once carry-forward has moved on — review [334]).
pub fn evict_share(dir: &Path, epoch: u64) {
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
/// JOURNALS (within-window scratch): delete every `epoch <= now` — the SAME lifetime
/// predicate the running past-boundary sweep applies (`actor.rs`, `*e <= now`). A
/// finalized-but-pre-boundary epoch E (its ceremony ran during E-1, so at finalize
/// `now ∈ E-1 < E`) has `E > now` → KEPT (resume/serve still need it).
///
/// SHARES (the durable carry-forward key): KEEP the ACTIVE carry-forward — the max share
/// epoch `<= now`, exactly what `CeremonyStore.range(..=now).next_back()` resolves to — AND
/// every FUTURE share (`> now`, a just-finalized next-epoch key not yet active, normal
/// near a change boundary). DELETE only shares STRICTLY OLDER than that active floor: they
/// are superseded and never re-used (a node verifies/signs only the live epoch's key).
/// Keeping only `max` would be WRONG — it would delete the active `<= now` carry-forward
/// whenever a future share already exists, demoting the node for the rest of the epoch.
pub fn reconcile_journals(dir: &Path, now: u64) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut journals: Vec<u64> = Vec::new();
    let mut shares: Vec<u64> = Vec::new();
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
    for epoch in journals {
        if epoch <= now {
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

    fn sample_journal_records() -> (Vec<JournalRecord>, NonZeroU32) {
        use commonware_cryptography::bls12381::dkg::{Dealer, Info, Player};
        use commonware_cryptography::bls12381::primitives::sharing::Mode;
        use commonware_cryptography::Signer as _;
        use commonware_utils::{ordered::Set, N3f1, NZU32};

        let mut rng = StdRng::seed_from_u64(7);
        let keys: Vec<Ed25519PrivateKey> =
            (0..3).map(|_| Ed25519PrivateKey::random(&mut rng)).collect();
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
            .receive_player_ack(player_pk.clone(), ack)
            .expect("receive ack");
        let log = dealer.finalize::<N3f1>();

        let records = vec![
            JournalRecord::ReceivedDealing(
                dealer_key.public_key(),
                Box::new(pub_msg),
                Box::new(priv_msg),
            ),
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
        assert!(!journal_file_for(&dir, 5).exists(), "evicted journal is gone");
        evict_journal(&dir, 5); // idempotent — no panic on a missing file
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `reconcile_journals(now)` deletes every journal at-or-before `now` (boundary-
    /// passed scratch), keeps the still-needed `e > now` ones, and never touches a
    /// foreign file.
    #[test]
    fn reconcile_journals_deletes_past_boundary_keeps_future_ignores_foreign() {
        let (records, _n) = sample_journal_records();
        let dir = fresh_dir("journal-reconcile");
        for e in [3u64, 5, 7] {
            append_journal(&dir, e, &records[0], &ShareState::Plaintext).expect("append");
        }
        let foreign = dir.join("some-other-file.bin");
        std::fs::write(&foreign, b"keep me").expect("write foreign");

        reconcile_journals(&dir, 5);

        assert!(!journal_file_for(&dir, 3).exists(), "e3 <= now=5 deleted");
        assert!(!journal_file_for(&dir, 5).exists(), "e5 <= now=5 deleted");
        assert!(journal_file_for(&dir, 7).exists(), "e7 > now=5 kept");
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
            persist(&dir, e, &output, &share, &ShareState::Plaintext).expect("persist");
        }
        reconcile_journals(&dir, 6);
        assert!(!file_for(&dir, 3).exists(), "e3 < floor(5) — superseded, deleted");
        assert!(file_for(&dir, 5).exists(), "e5 = active carry-forward (max <= now=6) — kept");
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
            persist(&dir, e, &output, &share, &ShareState::Plaintext).expect("persist");
        }
        reconcile_journals(&dir, 5); // now=5, both shares > now
        assert!(file_for(&dir, 8).exists(), "no `<= now` floor → nothing deleted");
        assert!(file_for(&dir, 9).exists(), "no `<= now` floor → nothing deleted");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reconcile_journals_missing_dir_is_no_op() {
        let dir = std::env::temp_dir().join(format!(
            "beacon-reconcile-missing-{}",
            std::process::id()
        ));
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
