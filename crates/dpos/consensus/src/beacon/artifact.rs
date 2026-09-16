//! The agreement artifact: what the plane leaves behind, where it is kept, and
//! how a node that never ran the ceremony gets it.
//!
//! No block carries the epoch key any more, so a node that never ran the
//! ceremony — a cold-started member, a validator that sat outside `committee[E]`,
//! the STF verifier — holds no dealer logs, cannot recompute `PK_E`, and this
//! artifact is the only form the key still reaches it in. It must be checkable by
//! someone who has nothing but the staking contract: [`verify_artifact`] takes the
//! artifact and `committee[epoch]` and needs no local ceremony state, share,
//! journal or block.
//!
//! Delivery has two seams. The pull seam here rides `BEACON_RESOLVER_CHANNEL` and
//! reaches only nodes on the consensus plane; a `--cert-follow` follower has no
//! peer set and no peer asks it, so it fetches the same artifact over its cert
//! upstream, checked against the same `committee[minted_at]`.
//!
//! [`ArtifactStore`] is per-epoch, in memory and on disk, and is what a peer is
//! served from. It is also what closes the residual the agreement supervisor
//! names: a node that restarts after its instance finalized loses the artifact
//! entirely, because no live sender re-broadcasts a decided proposal. The durable
//! half is written on the same edge the artifact is produced, so recovering it
//! after a restart is a read, not a re-agreement.
//!
//! [`ArtifactResponse`] is `Have` or `NotYet{epoch}`; both are delivered, never a
//! dropped responder. Commonware's resolver inserts a peer that answers `false`
//! into an `excluded` set with no removal path, so `false` is reserved here for
//! proven misbehaviour and every other answer, including "I do not have it", is a
//! delivered value. There is deliberately no `Never`: a wrong "never" tells a peer
//! to stop asking for a key it genuinely needs, and the state can only return
//! together with a retention policy.
//!
//! [`ArtifactPull::pull`] answers `Some(Have)`, `Some(NotYet)` or `None`, where
//! `None` means the walk was exhausted. That is the cert-follow RPC seam's
//! semantics; it is not `Consumer::failed`, which fires only on
//! Cancel/Retain/Clear and never on an error or a timeout.
//!
//! A pulled artifact enters the write-back rather than only the store: filing it
//! answers every `PK_epoch` read and lets this node serve peers, but neither
//! reaches `DkgActor::on_artifact`, which is fed by the agreement write-back
//! alone. Without that hop a member whose own instance died mid-agreement can
//! verify the key it pulled and still stay shareless for the epoch.

use bytes::{Buf, BufMut, Bytes};
use commonware_codec::{
    Decode as _, DecodeExt as _, Encode as _, EncodeSize, Error as CodecError, FixedSize as _,
    Read, ReadExt as _, Write,
};
use commonware_consensus::simplex::types::Finalization;
use commonware_cryptography::bls12381::primitives::{sharing::Sharing, variant::MinSig};
use commonware_parallel::Sequential;
use commonware_runtime::{Clock, Handle, Metrics, Spawner, Storage};
use commonware_storage::metadata::{Config as MetadataConfig, Error as MetadataError, Metadata};
use commonware_utils::{sequence::U64, vec::NonEmptyVec};
use fluentbase_bls::{
    beacon::dkg_namespace, beacon::GroupPublic, fluent_namespace, scheme::build_verifier,
    EpochCommittee, PeerPubkey, Scheme as BlsScheme,
};
use fluentbase_p2p::constants::MAX_COMMITTEE_SIZE;
// Spoken only by the test-only [`verify_artifact_from_snapshot`].
#[cfg(test)]
use fluentbase_staking_reader::reader::ValidatorSetSnapshot;
use rand_core::{CryptoRngCore, OsRng};
#[cfg(test)]
use std::collections::BTreeSet;
use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::{Arc, Mutex, RwLock},
    time::{Duration, SystemTime},
};
use tokio::sync::{
    mpsc::{error::TrySendError, UnboundedReceiver},
    oneshot,
};
use tracing::{debug, error, warn};

#[cfg(test)]
use crate::scheme::epoch_committee_from_snapshot;
use crate::{
    beacon::{
        dkg_agree::{AgreedArtifact, DkgProposal},
        metrics::BeaconMetrics,
        outcome::{encode_outcome, group_public_key},
        share_state,
    },
    digest::Digest,
};

/// The finalization-certificate half of the artifact.
type Cert = Finalization<BlsScheme, Digest>;

/// Decode cap for one served artifact.
///
/// Worst case at `n = 51` is about 154 KiB: a 51-entry log set (~1.7 KiB), a
/// 64 KiB outcome, 51 confirmations each carrying a 51-entry recorded set and a
/// 64-byte signature (~88 KiB), and a certificate under 1 KiB. The cap is rounded
/// up from that, and it is a network-wide constant rather than a bound derived
/// from the live committee, so every node accepts and refuses the same bytes.
///
/// It may only ever grow: the same value is the durable journal's decode bound
/// ([`ArtifactJournal::init`]), so lowering it invalidates records already on
/// disk and panics the node at startup.
pub(crate) const MAX_ARTIFACT_SIZE: usize = 256 * 1024;

/// Shortest gap between two pulls for one target epoch.
///
/// The per-peer bound this seam owes the network. The resolver picks the peer and
/// the channel quota bounds the far side, but nothing upstream stops a caller from
/// re-issuing a fetch in a tight loop, and an inbound over-quota sleeps the whole
/// connection to that peer. `NotYet` is the normal answer for most of an epoch.
pub(crate) const PULL_MIN_INTERVAL: Duration = Duration::from_secs(5);

/// How long one pull waits for an answer before reporting the walk exhausted.
///
/// Bounds the caller: an isolated node gets `None` and can act on it instead of
/// parking on a fetch the resolver will silently retry forever.
pub(crate) const PULL_TIMEOUT: Duration = Duration::from_secs(8);

/// How `committee[epoch]` is read — the only input artifact verification needs
/// beyond the artifact itself.
///
/// A closure rather than a trait because the production implementation is a
/// staking-contract read that lives above this crate. `None` means the committee
/// cannot be read yet (the executor has not reached the block that committed it) —
/// a transient state, never a verdict about the artifact.
pub type CommitteeSource = Arc<dyn Fn(u64) -> Option<EpochCommittee> + Send + Sync>;

/// Why an artifact does not verify against `committee[epoch]`.
///
/// Every arm except [`Self::CommitteeUnreadable`] is a property of the artifact,
/// which is what makes them safe to punish a peer for; that one arm is a property
/// of this node and must never reach a peer as a verdict.
#[derive(Debug, thiserror::Error)]
pub enum ArtifactError {
    /// The proposal, the certificate and the committee do not all name one epoch.
    #[error("artifact epoch mismatch: committee[{committee}], proposal {proposal}, certificate {certificate}")]
    EpochMismatch {
        committee: u64,
        proposal: u64,
        certificate: u64,
    },
    /// The certificate certifies a different payload than the body carried with
    /// it — a mispaired or substituted body.
    #[error("the certificate does not name this proposal's digest")]
    PayloadMismatch,
    /// The multisig quorum does not verify under `committee[epoch]`.
    #[error("the finalization certificate does not verify against committee[{0}]")]
    Certificate(u64),
    /// The committee could not be read at all. Not a fault of the artifact.
    #[error("committee[{0}] is not readable yet")]
    CommitteeUnreadable(u64),
    /// The snapshot does not form a committee (duplicate peer or BLS key).
    #[cfg(test)]
    #[error("committee[{epoch}] snapshot is not a valid participant set: {source}")]
    Committee {
        epoch: u64,
        #[source]
        source: commonware_utils::ordered::Error,
    },
    #[error("artifact does not decode: {0}")]
    Decode(#[from] CodecError),
    #[error("artifact is {0} bytes, over the {MAX_ARTIFACT_SIZE}-byte cap")]
    TooLarge(usize),
}

/// Verify an artifact against `committee[epoch]` and nothing else.
///
/// The inputs are the artifact, the committee read from the staking contract, and
/// the chain id — no ceremony state, no share, no local scheme registry, no block.
/// The quorum is checked under the agreement namespace ([`dkg_namespace`]), not the
/// chain namespace, which is a distinct and prefix-free tag so the two planes'
/// signatures can never be read as each other's.
pub(crate) fn verify_artifact<R: CryptoRngCore>(
    rng: &mut R,
    chain_id: u64,
    committee: &EpochCommittee,
    artifact: &AgreedArtifact,
) -> Result<(), ArtifactError> {
    let (proposal, certificate) = artifact;
    let cert_epoch = certificate.proposal.round.epoch().get();
    if committee.epoch != proposal.target_epoch || cert_epoch != proposal.target_epoch {
        return Err(ArtifactError::EpochMismatch {
            committee: committee.epoch,
            proposal: proposal.target_epoch,
            certificate: cert_epoch,
        });
    }
    if certificate.proposal.payload != proposal.digest() {
        return Err(ArtifactError::PayloadMismatch);
    }
    let namespace = dkg_namespace(&fluent_namespace(chain_id));
    // `oracle: None`: the agreement instance carries no beacon half at all, so its
    // certificate is a plain multisig quorum and `verify_certificate` returns on
    // the vote arm.
    let verifier = build_verifier(&namespace, committee.bimap.clone(), committee.epoch, None);
    if !certificate.verify(rng, &verifier, &Sequential) {
        return Err(ArtifactError::Certificate(committee.epoch));
    }
    Ok(())
}

/// [`verify_artifact`] straight off a staking read.
///
/// The snapshot is exactly what `RethStakingStateReader::epoch_committee_snapshot`
/// returns, so this is the whole path from a staking read to the verified key.
#[cfg(test)]
pub(crate) fn verify_artifact_from_snapshot<R: CryptoRngCore>(
    rng: &mut R,
    chain_id: u64,
    snapshot: &ValidatorSetSnapshot,
    artifact: &AgreedArtifact,
) -> Result<(), ArtifactError> {
    let committee =
        epoch_committee_from_snapshot(snapshot).map_err(|source| ArtifactError::Committee {
            epoch: snapshot.epoch,
            source,
        })?;
    verify_artifact(rng, chain_id, &committee, artifact)
}

/// [`verify_artifact`] against a committee this node may not be able to read yet.
///
/// The unreadable case gets its own error arm because the two demand opposite
/// responses: an artifact that cannot be checked is dropped while the peer keeps
/// its standing, whereas one that fails the check is a proven forgery and costs
/// the peer.
pub(crate) fn verify_artifact_for_epoch<R: CryptoRngCore>(
    rng: &mut R,
    chain_id: u64,
    committee: &CommitteeSource,
    epoch: u64,
    artifact: &AgreedArtifact,
) -> Result<(), ArtifactError> {
    let committee = committee(epoch).ok_or(ArtifactError::CommitteeUnreadable(epoch))?;
    verify_artifact(rng, chain_id, &committee, artifact)
}

/// Canonical bytes for one artifact — the durable record and the `Have` payload.
///
/// The value identity of a certified payload: `keccak256(target_epoch ‖ logs ‖
/// group_key)`. [`DkgProposal::digest`] also covers `confirms`, so two certificates
/// over one pinned set and one key with different confirmation metadata certify the
/// same value and are one artifact here. This is the identity `Conflict` is judged
/// by.
pub(crate) fn value_digest(proposal: &DkgProposal) -> alloy_primitives::B256 {
    let mut buf = Vec::with_capacity(8 + 4 + proposal.logs.len() * 33);
    buf.extend_from_slice(&proposal.target_epoch.to_be_bytes());
    buf.extend_from_slice(&(proposal.logs.len() as u32).to_be_bytes());
    for (idx, hash) in &proposal.logs {
        buf.push(*idx);
        buf.extend_from_slice(hash.as_slice());
    }
    buf.extend_from_slice(&encode_outcome(&proposal.group_key));
    alloy_primitives::keccak256(buf)
}

pub(crate) fn encode_artifact(artifact: &AgreedArtifact) -> Vec<u8> {
    artifact.encode().to_vec()
}

/// Decode an artifact under the network-wide caps.
pub fn decode_artifact(bytes: &[u8]) -> Result<AgreedArtifact, ArtifactError> {
    if bytes.len() > MAX_ARTIFACT_SIZE {
        return Err(ArtifactError::TooLarge(bytes.len()));
    }
    // The signer bitmap is decoded with a bounded cap: the bytes come from an
    // untrusted peer and the unbounded decoder allocates eagerly from a tiny length
    // prefix.
    let cap = MAX_COMMITTEE_SIZE as usize;
    Ok(<(crate::beacon::dkg_agree::DkgProposal, Cert)>::decode_cfg(
        bytes,
        &((), cap),
    )?)
}

/// What a peer answers when asked for `committee[epoch]`'s artifact.
///
/// Both states are delivered; a peer that lacks the artifact answers instead of
/// dropping the responder.
#[derive(Clone, Debug)]
pub(crate) enum ArtifactResponse {
    /// The peer holds the artifact for the requested epoch.
    Have(Box<AgreedArtifact>),
    /// The peer does not hold it yet — never a statement about whether it will
    /// ever exist.
    NotYet { epoch: u64 },
}

impl ArtifactResponse {
    const TAG_HAVE: u8 = 0;
    const TAG_NOT_YET: u8 = 1;
}

impl Write for ArtifactResponse {
    fn write(&self, buf: &mut impl BufMut) {
        match self {
            Self::Have(artifact) => {
                Self::TAG_HAVE.write(buf);
                artifact.0.write(buf);
                artifact.1.write(buf);
            }
            Self::NotYet { epoch } => {
                Self::TAG_NOT_YET.write(buf);
                epoch.write(buf);
            }
        }
    }
}

impl EncodeSize for ArtifactResponse {
    fn encode_size(&self) -> usize {
        u8::SIZE
            + match self {
                Self::Have(artifact) => artifact.0.encode_size() + artifact.1.encode_size(),
                Self::NotYet { epoch } => epoch.encode_size(),
            }
    }
}

impl Read for ArtifactResponse {
    type Cfg = ();

    fn read_cfg(buf: &mut impl Buf, _: &Self::Cfg) -> Result<Self, CodecError> {
        match u8::read(buf)? {
            Self::TAG_HAVE => {
                let proposal = crate::beacon::dkg_agree::DkgProposal::read(buf)?;
                let certificate = Cert::read_cfg(buf, &(MAX_COMMITTEE_SIZE as usize))?;
                Ok(Self::Have(Box::new((proposal, certificate))))
            }
            Self::TAG_NOT_YET => Ok(Self::NotYet {
                epoch: u64::read(buf)?,
            }),
            tag => Err(CodecError::InvalidEnum(tag)),
        }
    }
}

/// The per-epoch artifact store: a RAM map every reader sees synchronously, and an
/// optional durable mirror behind it.
///
/// The RAM write is what in-process readers (the producer serving a peer, the
/// write-back adopting the pinned set) see and it must stay synchronous, while
/// durability is only ever read by a later process after a restart, so it is free
/// to lag. Records go out over a non-blocking unbounded channel and every disk
/// touch happens on the writer.
///
/// Retention is by mint, not by epoch age ([`Self::retain_mints_for_window`]): an
/// artifact is the only source of `PK_epoch` for a node that never ran the
/// ceremony, and on a committee that has been stable for a long time the entry
/// worth having is the oldest, so a window measured in epochs would drop the
/// valuable record first. What goes is a mint no epoch inside the retention window
/// resolves to any more.
#[derive(Clone, Default)]
pub struct ArtifactStore {
    ram: Arc<RwLock<BTreeMap<u64, Arc<AgreedArtifact>>>>,
    durable: Option<tokio::sync::mpsc::UnboundedSender<Durable>>,
    /// The value digest of a second quorum-certified artifact seen for an epoch
    /// this store already holds one for — the `Conflict` witness, kept beside the
    /// held value so the fact survives a lost hand-off to the `DkgActor`. First-wins
    /// like `ram`. [`Self::note_divergent`] writes the
    /// `beacon-conflict-e<E>.bin` marker under `conflict_dir` the instant it notes
    /// the value, and a store opened over that directory reloads every marker there.
    divergent: Arc<RwLock<BTreeMap<u64, alloy_primitives::B256>>>,
    /// Where the conflict markers live. `None` (RAM-only, the in-process/test
    /// default) keeps the witness in this process only.
    conflict_dir: Option<PathBuf>,
    /// One notifier per [`Self::subscribe`] caller, fired by every accepted
    /// [`Self::insert`].
    ///
    /// Per consumer, never a shared handle: `notify_one` wakes exactly one waiter,
    /// so two consumers sharing a handle silently swallow each other's wake-ups.
    listeners: Arc<Mutex<Vec<Arc<tokio::sync::Notify>>>>,
}

/// What the RAM store hands the durable writer.
pub(crate) enum Durable {
    /// One accepted artifact, encoded.
    Append(u64, Vec<u8>),
    /// Drop every journal record whose mint the predicate refuses.
    Retain(Arc<dyn Fn(u64) -> bool + Send + Sync>),
}

impl ArtifactStore {
    /// A RAM-only store: tests and any in-process run.
    pub fn new() -> Self {
        Self::default()
    }

    fn with_persistence(
        rehydrated: Vec<(u64, AgreedArtifact)>,
        durable: tokio::sync::mpsc::UnboundedSender<Durable>,
    ) -> Self {
        let ram = rehydrated
            .into_iter()
            .map(|(epoch, artifact)| (epoch, Arc::new(artifact)))
            .collect();
        Self {
            ram: Arc::new(RwLock::new(ram)),
            durable: Some(durable),
            divergent: Arc::default(),
            conflict_dir: None,
            listeners: Arc::default(),
        }
    }

    /// Make the divergence witness durable under `dir` and reload every marker
    /// already there, so a store that restarts between the note and the actor's
    /// verdict still answers `divergent` for the epoch. A malformed marker reloads
    /// as a witness with no known digest (fail-closed: only a verdict writes one).
    pub fn with_conflict_dir(self, dir: PathBuf) -> Self {
        {
            let mut divergent = self
                .divergent
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for (epoch, marker) in share_state::conflict_markers(&dir) {
                let second = match marker {
                    share_state::ConflictMarker::Pair(_, second) => second,
                    share_state::ConflictMarker::Malformed => alloy_primitives::B256::ZERO,
                };
                divergent.entry(epoch).or_insert(second);
            }
        }
        Self {
            conflict_dir: Some(dir),
            ..self
        }
    }

    /// A notifier of this consumer's own, fired by every accepted [`Self::insert`].
    pub fn subscribe(&self) -> Arc<tokio::sync::Notify> {
        let handle = Arc::new(tokio::sync::Notify::new());
        if let Ok(mut listeners) = self.listeners.lock() {
            listeners.push(handle.clone());
        }
        handle
    }

    /// Record `artifact` as the agreement's output for `epoch`.
    ///
    /// `Err(artifact)` returns the value handed back (boxed, so the `Ok` stays
    /// small) when an artifact for that epoch is already held; the store keeps the
    /// one it has. First-wins is correct rather than convenient: the agreement
    /// instance certifies exactly one value per target epoch, so a second artifact
    /// is either identical or evidence this store cannot adjudicate, and
    /// overwriting would let a fetched artifact displace the one this node itself
    /// agreed. The loser comes back so the caller can note it as divergent.
    pub fn insert(&self, epoch: u64, artifact: AgreedArtifact) -> Result<(), Box<AgreedArtifact>> {
        let mut ram = self.lock_mut();
        if ram.contains_key(&epoch) {
            return Err(Box::new(artifact));
        }
        let encoded = encode_artifact(&artifact);
        ram.insert(epoch, Arc::new(artifact));
        // Handed over under the RAM lock, as `retain` does: the journal then sees
        // appends and retains in the order RAM applied them, so the two cannot
        // disagree on a mint at the retention floor. Unbounded and non-blocking: a
        // closed writer costs a re-fetch after the next restart and nothing now.
        if let Some(durable) = &self.durable {
            if durable.send(Durable::Append(epoch, encoded)).is_err() {
                // Counted as well as logged: "accepted in RAM, never written" is a
                // state an operator must be able to see, and a `warn!` alone is not
                // a signal anything scrapes.
                metrics::counter!("dpos_artifact_store_handoff_failed_total").increment(1);
                warn!(
                    epoch,
                    "artifact store: the durable writer is gone; this epoch stays RAM-only \
                     and must be re-fetched from a peer after a restart"
                );
            }
        }
        drop(ram);
        // Unconditional and strictly after the durable hand-off: a waiter re-reads
        // on wake-up, so firing regardless is idempotent.
        if let Ok(listeners) = self.listeners.lock() {
            for handle in listeners.iter() {
                handle.notify_one();
            }
        }
        Ok(())
    }

    /// The artifact for `epoch`, if this node holds one.
    pub fn get(&self, epoch: u64) -> Option<Arc<AgreedArtifact>> {
        self.lock().get(&epoch).cloned()
    }

    /// Note a quorum-certified `artifact` for `epoch` whose value differs from the
    /// held one ([`value_digest`]). Returns whether it is the first such value, so
    /// the caller reports the conflict once. A note for an epoch nothing is held
    /// for, or for the held value itself, is refused: the witness is a pair.
    ///
    /// The first note is made durable here and now when the store has a
    /// `conflict_dir`, so the verdict survives a death before the actor's next tick.
    /// A marker that cannot be written is logged and counted; the RAM witness
    /// stands.
    pub(crate) fn note_divergent(&self, epoch: u64, artifact: &AgreedArtifact) -> bool {
        let Some(held) = self.get(epoch) else {
            return false;
        };
        let (held, second) = (value_digest(&held.0), value_digest(&artifact.0));
        if held == second {
            return false;
        }
        let mut divergent = self
            .divergent
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if divergent.contains_key(&epoch) {
            return false;
        }
        if let Some(dir) = &self.conflict_dir {
            if let Err(err) = share_state::persist_conflict(dir, epoch, &held, &second) {
                metrics::counter!("dpos_artifact_store_conflict_marker_failed_total").increment(1);
                error!(
                    epoch,
                    ?err,
                    "artifact store: could not write the conflict marker for a second \
                     certified value; the witness holds in this process only until the \
                     DKG actor's verdict re-attempts it"
                );
            }
        }
        divergent.insert(epoch, second);
        true
    }

    /// The held artifact for `epoch`, and the value digest of the divergent second
    /// one if it was ever noted.
    pub fn view(
        &self,
        epoch: u64,
    ) -> Option<(Arc<AgreedArtifact>, Option<alloy_primitives::B256>)> {
        let held = self.get(epoch)?;
        let divergent = self
            .divergent
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&epoch)
            .copied();
        Some((held, divergent))
    }

    /// Whether this node can answer `Have` for `epoch`.
    pub fn has(&self, epoch: u64) -> bool {
        self.lock().contains_key(&epoch)
    }

    /// Every mint this store can serve, ascending.
    pub fn epochs(&self) -> Vec<u64> {
        self.lock().keys().copied().collect()
    }

    /// Keep only the mints `keep` accepts, in RAM now and in the journal on the
    /// writer's next turn. The divergence witness is left alone: a conflict marker
    /// has its own lifetime owner in the DKG actor.
    pub fn retain(&self, keep: impl Fn(u64) -> bool + Send + Sync + 'static) {
        let keep: Arc<dyn Fn(u64) -> bool + Send + Sync> = Arc::new(keep);
        let mut ram = self.lock_mut();
        ram.retain(|mint, _| keep(*mint));
        if let Some(durable) = &self.durable {
            if durable.send(Durable::Retain(keep)).is_err() {
                metrics::counter!("dpos_artifact_store_handoff_failed_total").increment(1);
                warn!("artifact store: the durable writer is gone; the journal keeps its records");
            }
        }
    }

    /// The retention rule: keep every mint that some epoch in
    /// `[now − window, now + MAX_COMMITTEE_LOOKAHEAD_EPOCHS]` resolves to — the
    /// newest mint at or below `now − window` and everything above it
    /// ([`share_state::ceremony_retain_floor`], the same rule the shares follow).
    /// On a stable committee the live mint can be far older than the window and
    /// survives; only a mint superseded before the window opened goes.
    pub fn retain_mints_for_window(&self, now: u64, window: u64) {
        let floor = share_state::ceremony_retain_floor(self.epochs().into_iter(), now, window);
        if floor > 0 {
            self.retain(move |mint| mint >= floor);
        }
    }

    fn lock(&self) -> std::sync::RwLockReadGuard<'_, BTreeMap<u64, Arc<AgreedArtifact>>> {
        // A poisoned lock means a reader panicked mid-read; the map is a plain
        // BTreeMap and cannot be left half-written, and taking the node down over it
        // would forfeit the very key this store exists to serve.
        self.ram
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn lock_mut(&self) -> std::sync::RwLockWriteGuard<'_, BTreeMap<u64, Arc<AgreedArtifact>>> {
        self.ram
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Read the frozen on-chain `dkgQual[epoch]` bit: did the committee change at
/// `epoch`, i.e. did its DKG re-mint the key. `None` = could not read yet, which
/// the caller must treat as undecided and never as "no re-mint".
pub(crate) type ChangedAt = Arc<dyn Fn(u64) -> Option<bool> + Send + Sync>;

/// `epoch → the epoch that minted the key in force at it`, memoised on disk.
///
/// `minted_at(E) = last e in (BOOTSTRAP, E] with changed[e], else BOOTSTRAP`. The
/// bits are set deterministically by the contract at `commitEpochCommittee` and
/// never mutated, so a resolved answer is a function of frozen chain facts and
/// cannot change later.
///
/// The memo is durable because the walk reads the chain, and the committee module
/// answers a bit only inside
/// `[epoch(anchor) − SCHEME_RETENTION_EPOCHS, epoch(anchor) + lookahead]`, folding
/// everything below that window into `None`: a restarted node with an empty memo
/// would hold a durable artifact it cannot address. Two gaps remain: a fresh node
/// whose mint is older than the window, and an epoch whose memo entry lies below an
/// unreadable bit — the walk answers `None` at the first undecided bit, so the
/// answer on disk is unreachable until the bits above it become readable.
///
/// Two rules make this safe, and breaking either is unrecoverable: `None` is never
/// memoised (it would pin the bootstrap answer onto an epoch the chain has not
/// committed yet), and a resolved answer is write-once (a second, differing answer
/// for one epoch cannot be honest).
#[derive(Clone)]
pub(crate) struct MintIndex {
    changed: ChangedAt,
    /// `epoch → minted_at`. Shared by clone: every reader of the same plane shares
    /// one, which makes the walk cost one step instead of `E − BOOTSTRAP` per
    /// certificate.
    memo: Arc<Mutex<BTreeMap<u64, u64>>>,
    /// Decided bits, so the walk pays one chain read per epoch per process rather
    /// than per call. Not persisted: it is derivable, and the memo above is what
    /// has to survive.
    bits: Arc<Mutex<BTreeMap<u64, bool>>>,
    /// Durable sink for resolved answers. `None` ⇒ RAM-only (tests, and any config
    /// without a partition). Non-blocking: readers see the RAM map, and durability
    /// is only ever read by a later process.
    persist: Option<tokio::sync::mpsc::UnboundedSender<(u64, u64)>>,
}

impl MintIndex {
    /// A RAM-only index over `changed`.
    pub(crate) fn new(changed: ChangedAt) -> Self {
        Self {
            changed,
            memo: Arc::default(),
            bits: Arc::default(),
            persist: None,
        }
    }

    /// The epoch that minted the key in force at `epoch`, or `None` when the chain
    /// cannot answer yet or `epoch` predates the beacon.
    pub(crate) fn minted_at(&self, epoch: u64) -> Option<u64> {
        if epoch < super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH {
            return None;
        }
        if let Some(hit) = self.memo.lock().ok().and_then(|m| m.get(&epoch).copied()) {
            return Some(hit);
        }
        let mut answer = None;
        for e in (super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH + 1..=epoch).rev() {
            // A memoised lower epoch answers this one too, by monotonicity: nothing
            // between it and `epoch` had its bit set, or the scan would have stopped.
            if let Some(hit) = self.memo.lock().ok().and_then(|m| m.get(&e).copied()) {
                answer = Some(hit);
                break;
            }
            match self.bit(e) {
                Some(true) => {
                    answer = Some(e);
                    break;
                }
                Some(false) => continue,
                None => return None,
            }
        }
        let minted_at = answer.unwrap_or(super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH);
        self.record(epoch, minted_at);
        Some(minted_at)
    }

    /// One decided bit, cached for the process.
    fn bit(&self, epoch: u64) -> Option<bool> {
        if let Some(hit) = self.bits.lock().ok().and_then(|b| b.get(&epoch).copied()) {
            return Some(hit);
        }
        let bit = (self.changed)(epoch)?;
        if let Ok(mut b) = self.bits.lock() {
            b.insert(epoch, bit);
        }
        Some(bit)
    }

    /// Write-once record plus the durable offer. A differing second answer is
    /// fork-grade — two mint epochs for one epoch cannot both be chain fact — so the
    /// first stands and the disagreement is loud.
    fn record(&self, epoch: u64, minted_at: u64) {
        let fresh = match self.memo.lock() {
            Ok(mut memo) => match memo.get(&epoch).copied() {
                Some(existing) if existing != minted_at => {
                    warn!(
                        epoch,
                        existing,
                        offered = minted_at,
                        "mint index: a DIFFERING mint epoch for one epoch — keeping the \
                         first; two answers cannot both be chain fact"
                    );
                    metrics::counter!("dpos_mint_index_conflict_total").increment(1);
                    false
                }
                Some(_) => false,
                None => {
                    memo.insert(epoch, minted_at);
                    true
                }
            },
            Err(_) => false,
        };
        if !fresh {
            return;
        }
        if let Some(tx) = self.persist.as_ref() {
            if tx.send((epoch, minted_at)).is_err() {
                warn!(
                    epoch,
                    "mint index: the durable writer is gone; this answer is in memory only"
                );
            }
        }
    }
}

/// `PK_epoch` and the public polynomial, read from their one owner.
///
/// Two facts and nothing else: the chain says which epoch minted the key in force
/// at `epoch` ([`MintIndex`]), and that mint's quorum-certified artifact carries
/// the key ([`ArtifactStore`]). There is no separate store of keys.
#[derive(Clone)]
pub(crate) struct KeyIndex {
    artifacts: ArtifactStore,
    mints: MintIndex,
}

impl KeyIndex {
    pub(crate) fn new(artifacts: ArtifactStore, mints: MintIndex) -> Self {
        Self { artifacts, mints }
    }

    /// The epoch that minted the key in force at `epoch`.
    pub(crate) fn minted_at(&self, epoch: u64) -> Option<u64> {
        self.mints.minted_at(epoch)
    }

    /// `PK_epoch` in force at `epoch`. Synchronous and I/O-free: "not resolvable"
    /// is the answer a vote path acts on, never a reason to go fetch.
    pub(crate) fn key_at(&self, epoch: u64) -> Option<GroupPublic> {
        let minted_at = self.mints.minted_at(epoch)?;
        self.artifacts
            .get(minted_at)
            .map(|a| *group_public_key(&a.0.group_key))
    }

    /// `(minting epoch, public polynomial)` in force at `epoch` — what a partial is
    /// signed and verified against. The share that pairs with it is the
    /// `CeremonyStore`'s, keyed by the same minting epoch.
    pub(crate) fn sharing_at(&self, epoch: u64) -> Option<(u64, Sharing<MinSig>)> {
        let minted_at = self.mints.minted_at(epoch)?;
        let artifact = self.artifacts.get(minted_at)?;
        Some((minted_at, artifact.0.group_key.public().clone()))
    }

    /// Whether the mint's artifact is already local — the probe an acquisition
    /// short-circuits on.
    pub(crate) fn holds_mint_of(&self, epoch: u64) -> Option<bool> {
        Some(self.artifacts.has(self.mints.minted_at(epoch)?))
    }
}

/// One bounded acquisition of a minting epoch's artifact — the single shape both
/// classes that need one speak.
///
/// One bounded attempt, throttled to at most one network round-trip per epoch per
/// [`PULL_MIN_INTERVAL`], answering whether the store now holds `minted_at`. An
/// implementation may return `true` only for an artifact it verified against
/// `committee[minted_at]` read from this node's own chain state: a lying transport
/// is caught by the implementation, never by the caller.
pub(crate) trait AcquireArtifact: Send + Sync {
    fn fetch(&self, minted_at: u64) -> futures::future::BoxFuture<'_, bool>;
}

/// The handle every consumer of [`AcquireArtifact`] holds.
pub(crate) type AcquireMint = Arc<dyn AcquireArtifact>;

/// The bytes half of an acquisition: one transport's answer for a minting epoch.
///
/// `None` covers every negative alike — no artifact, an upstream too old to know
/// the method, a dead link. The three are one answer to the caller (stay unpinned,
/// ask again), and separating them would invite someone to treat one as a fault.
pub(crate) type ArtifactBytes =
    Arc<dyn Fn(u64) -> futures::future::BoxFuture<'static, Option<Vec<u8>>> + Send + Sync>;

/// [`AcquireArtifact`] over a byte transport: throttle, fetch, decode, verify
/// against `committee[minted_at]`, file, and hand the artifact to the write-back.
///
/// One body for every transport that delivers bytes: the verify is what makes a
/// lying upstream and a lying peer the same non-event, and a second copy of it is a
/// second place for the check to be weakened.
pub(crate) struct TransportAcquire<E: Clock> {
    chain_id: u64,
    committees: CommitteeSource,
    bytes: ArtifactBytes,
    store: ArtifactStore,
    adopt: Option<tokio::sync::mpsc::Sender<AgreedArtifact>>,
    clock: E,
    /// Per-epoch budget, pruned as it expires. Without it a want that arrives on
    /// every certificate would be one round-trip a second per unresolved epoch.
    next_allowed: Arc<Mutex<BTreeMap<u64, SystemTime>>>,
    metrics: BeaconMetrics,
}

impl<E: Clock> TransportAcquire<E> {
    pub fn new(
        chain_id: u64,
        committees: CommitteeSource,
        bytes: ArtifactBytes,
        store: ArtifactStore,
        adopt: Option<tokio::sync::mpsc::Sender<AgreedArtifact>>,
        clock: E,
        metrics: BeaconMetrics,
    ) -> Self {
        Self {
            chain_id,
            committees,
            bytes,
            store,
            adopt,
            clock,
            next_allowed: Arc::default(),
            metrics,
        }
    }

    /// `false` when this epoch's budget has not refilled yet.
    fn claim(&self, minted_at: u64) -> bool {
        let now = self.clock.current();
        let mut next = self
            .next_allowed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        next.retain(|_, at| *at > now);
        if next.contains_key(&minted_at) {
            return false;
        }
        next.insert(minted_at, now + PULL_MIN_INTERVAL);
        true
    }
}

impl<E: Clock + Send + Sync> AcquireArtifact for TransportAcquire<E> {
    fn fetch(&self, minted_at: u64) -> futures::future::BoxFuture<'_, bool> {
        Box::pin(async move {
            // Short-circuit on a local hit: an acquisition never reads what is
            // already held.
            if self.store.has(minted_at) {
                return true;
            }
            if !self.claim(minted_at) {
                return false;
            }
            let Some(bytes) = (self.bytes)(minted_at).await else {
                self.metrics.follower_artifact_miss.inc();
                return false;
            };
            let artifact = match decode_artifact(&bytes) {
                Ok(artifact) => artifact,
                Err(e) => {
                    warn!(
                        epoch = minted_at,
                        ?e,
                        "beacon: a served epoch artifact does not decode"
                    );
                    self.metrics.dkg_artifact_rejected.inc();
                    return false;
                }
            };
            match verify_artifact_for_epoch(
                &mut OsRng,
                self.chain_id,
                &self.committees,
                minted_at,
                &artifact,
            ) {
                Ok(()) => {}
                // Not the server's fault: this node has not reached the block that
                // committed `committee[minted_at]`. Drop it and re-ask later.
                Err(ArtifactError::CommitteeUnreadable(_)) => {
                    self.metrics.dkg_artifact_unverifiable.inc();
                    return false;
                }
                Err(e) => {
                    warn!(
                        epoch = minted_at,
                        ?e,
                        "beacon: REJECTING a served epoch artifact — it does not carry a \
                         committee[epoch] quorum; staying on vote-only admission"
                    );
                    self.metrics.dkg_artifact_rejected.inc();
                    return false;
                }
            }
            let pk = *group_public_key(&artifact.0.group_key);
            let first = match self.store.insert(minted_at, artifact.clone()) {
                Ok(()) => true,
                // A producer inserted between the `has` above and here: the held
                // value stands, and if this one differs it is the `Conflict` witness.
                Err(loser) => {
                    self.store.note_divergent(minted_at, &loser);
                    false
                }
            };
            self.metrics.follower_artifact_adopted.inc();
            if first {
                tracing::info!(
                    epoch = minted_at,
                    group_public = %pk_prefix(&pk),
                    "beacon: PK_epoch obtained and verified against committee[epoch] — \
                     certificates of the epochs it covers leave vote-only admission"
                );
                if let Some(adopt) = self.adopt.as_ref() {
                    hand_off(adopt, minted_at, &artifact, &self.metrics);
                }
            }
            true
        })
    }
}

/// Hand `artifact` to the agreement write-back — the only route from an
/// acquisition seam to the `DkgActor`. A refused send is logged and counted, never
/// fatal: the store owns the artifact and the actor reads it on its next height
/// tick, so what is lost is a tick of latency, not the fact.
fn hand_off(
    tx: &tokio::sync::mpsc::Sender<AgreedArtifact>,
    epoch: u64,
    artifact: &AgreedArtifact,
    metrics: &BeaconMetrics,
) {
    match tx.try_send(artifact.clone()) {
        Ok(()) => debug!(
            epoch,
            "artifact seam: handing an acquired artifact to the agreement write-back"
        ),
        Err(TrySendError::Full(_)) => {
            metrics.dkg_artifact_handoff_lost.inc();
            warn!(
                epoch,
                "artifact seam: the agreement write-back is backed up; the store holds the \
                 artifact and the DKG actor reads it on its next height tick"
            );
        }
        Err(TrySendError::Closed(_)) => {
            metrics.dkg_artifact_handoff_lost.inc();
            warn!(
                epoch,
                "artifact seam: the agreement write-back is gone; the store holds the artifact \
                 and nothing adopts it until the actor is back"
            );
        }
    }
}

/// First 8 serialized bytes of a group public key, hex — a stable, greppable
/// fingerprint. Enough to byte-diff key values across nodes from logs alone; the
/// full G2 hex is 192 chars of log noise.
pub(crate) fn pk_prefix(pk: &GroupPublic) -> String {
    let mut s = pk.to_string();
    s.truncate(16);
    s
}

/// One artifact builder for every test in this module tree that needs a
/// [`KeyIndex`] to answer.
///
/// The committee is a throwaway set whose only job is to make a real
/// `Finalization` constructible: nothing downstream of [`ArtifactStore::insert`]
/// re-verifies (verification happens before the insert on every production path),
/// so a caller that wants a store answering for `minted_at` wants exactly this.
#[cfg(test)]
pub(crate) fn artifact_with_key(
    minted_at: u64,
    group_key: crate::beacon::outcome::DkgOutcome,
) -> AgreedArtifact {
    use alloy_primitives::B256;
    use commonware_codec::DecodeExt as _;
    use commonware_consensus::{
        simplex::types::{Finalization, Finalize, Proposal},
        types::{Epoch, Round, View},
    };
    use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer as _};
    use commonware_math::algebra::Random as _;
    use commonware_utils::{ordered::BiMap, TryCollect as _};
    use fluentbase_bls::{keys::ValidatorBlsKeypair, scheme::build_signer, BlsPubkey};
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng as _;

    let mut rng = StdRng::seed_from_u64(0xA27 ^ minted_at);
    let peers: Vec<Ed25519PrivateKey> = (0..4)
        .map(|_| Ed25519PrivateKey::random(&mut rng))
        .collect();
    let bls: Vec<ValidatorBlsKeypair> = (0..4)
        .map(|_| ValidatorBlsKeypair::generate(&mut rng))
        .collect();
    let bimap: BiMap<fluentbase_bls::PeerPubkey, BlsPubkey> = peers
        .iter()
        .zip(bls.iter())
        .map(|(p, b)| {
            (
                p.public_key(),
                BlsPubkey::decode(b.public_bytes().as_slice()).expect("bls pubkey"),
            )
        })
        .try_collect()
        .expect("unique committee");
    let proposal = crate::beacon::dkg_agree::DkgProposal {
        target_epoch: minted_at,
        logs: (0..4u8).map(|i| (i, B256::repeat_byte(0x40 + i))).collect(),
        group_key,
        confirms: Vec::new(),
    };
    let ns = dkg_namespace(&fluent_namespace(1));
    let round = Round::new(Epoch::new(minted_at), View::new(1));
    let subject = Proposal::new(round, View::new(0), proposal.digest());
    let finalizes: Vec<_> = bls
        .iter()
        .take(3)
        .map(|kp| {
            let signer =
                build_signer(&ns, bimap.clone(), kp, minted_at, None).expect("committee member");
            Finalize::sign(&signer, subject.clone()).expect("sign")
        })
        .collect();
    let cert = Finalization::from_finalizes(
        &build_verifier(&ns, bimap, minted_at, None),
        finalizes.iter(),
        &Sequential,
    )
    .expect("quorum");
    (proposal, cert)
}

/// The one test seam for "this node holds the epoch key", for every module that
/// needs a [`KeyIndex`] to answer.
///
/// A test states a mint — an epoch plus the real `Output` its ceremony produced —
/// and the fixture does what production does with one: files the artifact that
/// carries it and records the chain's `changed` bit at that epoch. There is no way
/// to state a bare `GroupPublic`: the key is a projection of the artifact, so a
/// fixture that could inject one would test a state the node cannot be in.
#[cfg(test)]
pub(crate) struct MintFixture {
    pub(crate) artifacts: ArtifactStore,
    bits: Arc<Mutex<BTreeSet<u64>>>,
    /// How many times the chain was asked for an epoch's `changed` bit. A test that
    /// ingests many certificates of one epoch asserts on this to pin that the walk is
    /// memoised and the chain is read once, not once per certificate.
    chain_reads: Arc<Mutex<BTreeMap<u64, u32>>>,
    pub(crate) keys: KeyIndex,
}

#[cfg(test)]
impl MintFixture {
    pub(crate) fn new() -> Self {
        let artifacts = ArtifactStore::new();
        let bits: Arc<Mutex<BTreeSet<u64>>> = Arc::default();
        let chain_reads: Arc<Mutex<BTreeMap<u64, u32>>> = Arc::default();
        let changed: ChangedAt = {
            let bits = bits.clone();
            let reads = chain_reads.clone();
            Arc::new(move |e| {
                *reads
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .entry(e)
                    .or_default() += 1;
                Some(
                    bits.lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .contains(&e),
                )
            })
        };
        let keys = KeyIndex::new(artifacts.clone(), MintIndex::new(changed));
        Self {
            artifacts,
            bits,
            chain_reads,
            keys,
        }
    }

    /// How many times the chain was asked for `epoch`'s bit since this fixture was
    /// built.
    pub(crate) fn chain_reads(&self, epoch: u64) -> u32 {
        self.chain_reads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&epoch)
            .copied()
            .unwrap_or_default()
    }

    /// Record that the chain says `epoch` re-minted, without the artifact arriving.
    ///
    /// The two halves are separate and the order is load-bearing: the contract writes
    /// the bit with the committee, an epoch before the artifact arrives, and a
    /// fixture that set the bit late would stage a chain that changed its own
    /// history — [`MintIndex`]'s write-once memo correctly refuses to notice.
    pub(crate) fn changed(&self, epoch: u64) {
        self.bits
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(epoch);
    }

    /// The artifact for a mint the chain has already recorded arrives.
    pub(crate) fn arrive(&self, minted_at: u64, group_key: crate::beacon::outcome::DkgOutcome) {
        // First-wins: a fixture that arrives twice keeps the first value.
        drop(
            self.artifacts
                .insert(minted_at, artifact_with_key(minted_at, group_key)),
        );
    }

    /// Both halves at once, for a fixture that only needs the end state.
    pub(crate) fn mint(&self, minted_at: u64, group_key: crate::beacon::outcome::DkgOutcome) {
        self.changed(minted_at);
        self.arrive(minted_at, group_key);
    }
}

/// A [`ChangedAt`] over a fixed set of change epochs — the frozen chain record a
/// test asserts against.
#[cfg(test)]
pub(crate) fn changed_at(bits: &[u64]) -> ChangedAt {
    let set: BTreeSet<u64> = bits.iter().copied().collect();
    Arc::new(move |e| Some(set.contains(&e)))
}

/// A [`KeyIndex`] over a store the caller holds, so a test can make the mint's
/// artifact arrive mid-test — the one edge a key resolve turns on.
#[cfg(test)]
pub(crate) fn key_index_over(artifacts: ArtifactStore, bits: &[u64]) -> KeyIndex {
    KeyIndex::new(artifacts, MintIndex::new(changed_at(bits)))
}

/// Open the durable mint memo and join it to a [`MintIndex`] over `changed`, or
/// hand back a RAM-only index when no partition is configured.
///
/// Rehydrating the memo means a restarted node needs no chain read for any epoch it
/// already resolved, and one step for the next.
///
/// It does not close a fresh node whose datadir is empty and whose committee last
/// minted more than `SCHEME_RETENTION_EPOCHS` epochs ago: its walk must reach that
/// mint, every epoch below its own anchor's window folds to `None`, and no memo
/// entry exists to stop the walk. That node stays on vote-only admission for the
/// epoch.
pub(crate) async fn open_mint_memo<E>(
    journal_context: E,
    writer_context: E,
    partition: &str,
    changed: ChangedAt,
) -> eyre::Result<(MintIndex, Option<Handle<()>>)>
where
    E: Storage + Clock + Metrics + Spawner + Clone + Send + 'static,
{
    if partition.is_empty() {
        return Ok((MintIndex::new(changed), None));
    }
    let mut store: Metadata<E, U64, Vec<u8>> = Metadata::init(
        journal_context,
        MetadataConfig {
            partition: partition.to_string(),
            codec_config: ((0..=u64::BITS as usize).into(), ()),
        },
    )
    .await
    .map_err(|e| eyre::eyre!("opening the durable mint memo: {e}"))?;
    let mut memo = BTreeMap::new();
    let mut rejected = 0u64;
    for key in store.keys() {
        match store
            .get(key)
            .and_then(|b| <[u8; 8]>::try_from(&b[..]).ok())
        {
            Some(bytes) => {
                memo.insert(u64::from(key), u64::from_be_bytes(bytes));
            }
            None => rejected += 1,
        }
    }
    metrics::counter!("dpos_mint_memo_replayed_total").increment(memo.len() as u64);
    metrics::counter!("dpos_mint_memo_replay_rejected_total").increment(rejected);
    tracing::info!(
        entries = memo.len(),
        rejected,
        "rehydrated the beacon mint memo from disk"
    );
    // A record of the wrong length is dropped, and a memo that dropped everything
    // behaves exactly like an empty memo — so without this warn the two are
    // distinguishable only by a counter. There is no state to enter: the walk
    // re-resolves whatever it can still read.
    if rejected > 0 {
        warn!(
            rejected,
            entries = memo.len(),
            "mint memo: PARTLY UNREADABLE — every rejected record is re-resolved from the \
             chain, and any epoch whose bits are no longer readable stays unaddressable \
             until they are"
        );
    }
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(u64, u64)>();
    let writer = writer_context.spawn(move |_| async move {
        while let Some((epoch, minted_at)) = rx.recv().await {
            store.put(U64::new(epoch), minted_at.to_be_bytes().to_vec());
            // One record per epoch entered, so there is no batch worth waiting for
            // and the answer is durable the moment it is known.
            if let Err(e) = store.sync().await {
                metrics::counter!("dpos_mint_memo_sync_failed_total").increment(1);
                warn!(
                    epoch,
                    ?e,
                    "mint memo: sync failed; this answer is lost on a hard kill and the walk \
                     re-runs for it after a restart"
                );
                continue;
            }
            metrics::counter!("dpos_mint_memo_appended_total").increment(1);
        }
    });
    Ok((
        MintIndex {
            changed,
            memo: Arc::new(Mutex::new(memo)),
            bits: Arc::default(),
            persist: Some(tx),
        },
        Some(writer),
    ))
}

/// Failures of the durable artifact store.
#[derive(Debug, thiserror::Error)]
pub(crate) enum StoreError {
    #[error("artifact store: {0}")]
    Store(#[from] MetadataError),
}

/// The durable half. Exactly one instance per process: a second handle over the
/// same partition is a dual-writer.
///
/// [`Metadata`] rather than an `Ordinal`: an artifact is variable-length, the
/// collection is small and sparse, and `Metadata` commits a batch atomically
/// through its two-blob discipline — which matters because a half-written artifact
/// is a key nobody can verify.
pub(crate) struct ArtifactJournal<E: Storage + Clock + Metrics> {
    store: Metadata<E, U64, Vec<u8>>,
}

impl<E: Storage + Clock + Metrics> ArtifactJournal<E> {
    pub async fn init(context: E, partition: String) -> Result<Self, StoreError> {
        let store = Metadata::init(
            context,
            MetadataConfig {
                partition,
                codec_config: ((0..=MAX_ARTIFACT_SIZE).into(), ()),
            },
        )
        .await?;
        Ok(Self { store })
    }

    /// Stage one record. Not durable until [`Self::sync`].
    pub fn append(&mut self, epoch: u64, bytes: Vec<u8>) {
        self.store.put(U64::new(epoch), bytes);
    }

    /// Drop every record whose mint `keep` refuses. Not durable until [`Self::sync`].
    pub fn retain(&mut self, keep: &dyn Fn(u64) -> bool) {
        self.store.retain(|key, _| keep(u64::from(key.clone())));
    }

    /// Commit every staged record atomically.
    pub async fn sync(&mut self) -> Result<(), StoreError> {
        self.store.sync().await?;
        Ok(())
    }

    /// Every retained artifact, for the startup refill.
    ///
    /// A record that no longer decodes is skipped with a warn rather than failing
    /// the load: one unreadable epoch costs a peer fetch, while refusing to start
    /// costs the node.
    pub fn replay(&self) -> Vec<(u64, AgreedArtifact)> {
        let mut loaded = Vec::new();
        let mut rejected = 0u64;
        for key in self.store.keys() {
            let Some(bytes) = self.store.get(key) else {
                continue;
            };
            match decode_artifact(bytes) {
                Ok(artifact) => loaded.push((u64::from(key), artifact)),
                Err(e) => {
                    rejected += 1;
                    warn!(?e, "artifact store: skipping an undecodable record");
                }
            }
        }
        metrics::counter!("dpos_artifact_store_replayed_total").increment(loaded.len() as u64);
        metrics::counter!("dpos_artifact_store_replay_rejected_total").increment(rejected);
        loaded
    }
}

/// Open the durable artifact store and join it to a RAM map, or hand back a
/// RAM-only store when no partition is configured.
///
/// `writer_context` must be a sibling of `journal_context`, never a clone: the
/// deterministic runtime panics on a duplicate metric registered under the same
/// label, and both halves register store metrics.
pub async fn open<E>(
    journal_context: E,
    writer_context: E,
    partition: &str,
) -> eyre::Result<(ArtifactStore, Option<Handle<()>>)>
where
    E: Storage + Clock + Metrics + Spawner + Clone + Send + 'static,
{
    if partition.is_empty() {
        return Ok((ArtifactStore::new(), None));
    }
    let journal = ArtifactJournal::init(journal_context, partition.to_string())
        .await
        .map_err(|e| eyre::eyre!("opening the durable artifact store: {e}"))?;
    let rehydrated = journal.replay();
    tracing::info!(
        entries = rehydrated.len(),
        "rehydrated the agreement-artifact store from disk"
    );
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let writer = spawn_writer(writer_context, journal, rx);
    Ok((
        ArtifactStore::with_persistence(rehydrated, tx),
        Some(writer),
    ))
}

/// Drain `rx` onto the journal. The RAM store hands records over and returns
/// immediately; every disk touch happens here.
///
/// A failed `sync` loses nothing it was given: `Metadata::put` is an in-memory
/// insert and `sync` commits the whole map, so the record stays staged and the next
/// `sync` re-attempts it. The trigger is the next record on `rx`; artifacts arrive
/// once per committee change, so the retry can be far away and the gap is bounded
/// by the loss being loud.
pub(crate) fn spawn_writer<E>(
    context: E,
    mut journal: ArtifactJournal<E>,
    mut rx: UnboundedReceiver<Durable>,
) -> Handle<()>
where
    E: Storage + Clock + Metrics + Spawner + Clone + Send + 'static,
{
    context.spawn(move |_| async move {
        while let Some(record) = rx.recv().await {
            let epoch = match record {
                Durable::Append(epoch, bytes) => {
                    journal.append(epoch, bytes);
                    Some(epoch)
                }
                Durable::Retain(keep) => {
                    journal.retain(keep.as_ref());
                    None
                }
            };
            // One record per committee change, so there is no batch worth
            // waiting for and every artifact is durable the moment it is known.
            if let Err(e) = journal.sync().await {
                metrics::counter!("dpos_artifact_store_sync_failed_total").increment(1);
                warn!(
                    epoch,
                    ?e,
                    "artifact store: sync failed; the record stays staged and the next artifact's \
                     sync re-attempts it — until then a hard kill loses this epoch and it must be \
                     re-fetched from a peer"
                );
                continue;
            }
            if epoch.is_some() {
                metrics::counter!("dpos_artifact_store_appended_total").increment(1);
            }
        }
    })
}

/// Per-epoch correlation map: a delivered answer is fanned to every waiting pull.
type Waiters = Arc<Mutex<HashMap<u64, Vec<oneshot::Sender<PullAnswer>>>>>;

/// What a peer answered one pull.
#[derive(Clone, Debug)]
pub enum PullAnswer {
    /// A peer served the artifact and it verified against `committee[epoch]`.
    Have(Arc<AgreedArtifact>),
    /// A peer answered honestly that its plane has not converged for this epoch.
    NotYet,
}

/// The serve-and-consume half of the seam, held by the resolver handler and cloned
/// into the resolver engine for both roles.
#[derive(Clone)]
pub struct ArtifactBridge {
    chain_id: u64,
    store: ArtifactStore,
    committee: CommitteeSource,
    waiters: Waiters,
    adopt_tx: tokio::sync::mpsc::Sender<AgreedArtifact>,
    metrics: BeaconMetrics,
}

impl ArtifactBridge {
    /// `adopt_tx` is the agreement write-back's own inbound channel — the one a live
    /// instance sends its artifact on. It is a required constructor argument rather
    /// than an optional one because a bridge wired without it is precisely the defect
    /// this seam had: a pulled artifact that answers every key question and never
    /// reaches [`crate::beacon::actor::DkgActor::on_artifact`].
    pub fn new(
        chain_id: u64,
        store: ArtifactStore,
        committee: CommitteeSource,
        adopt_tx: tokio::sync::mpsc::Sender<AgreedArtifact>,
        metrics: BeaconMetrics,
    ) -> Self {
        Self {
            chain_id,
            store,
            committee,
            waiters: Arc::new(Mutex::new(HashMap::new())),
            adopt_tx,
            metrics,
        }
    }

    /// Serve `epoch`. Always an answer, never a dropped responder.
    ///
    /// Dropping it would make the requester's resolver read "no data" and retry
    /// another peer forever, with nothing surfaced above the transport.
    pub fn produce(&self, epoch: u64) -> Bytes {
        let response = match self.store.get(epoch) {
            Some(artifact) => {
                self.metrics.dkg_artifact_served.inc();
                ArtifactResponse::Have(Box::new((*artifact).clone()))
            }
            None => {
                self.metrics.dkg_artifact_not_yet.inc();
                ArtifactResponse::NotYet { epoch }
            }
        };
        response.encode()
    }

    /// Take a peer's answer for `epoch`.
    ///
    /// Returns `false` — which permanently excludes the peer, since commonware's
    /// resolver `excluded` set has no removal path — only for proven misbehaviour:
    /// bytes that do not decode, an answer about a different epoch, or an artifact
    /// whose certificate fails against `committee[epoch]`. Every other outcome,
    /// `NotYet` and an unreadable local committee included, is an honest delivery
    /// and returns `true`.
    pub fn deliver(&self, epoch: u64, value: &[u8]) -> bool {
        let response = match ArtifactResponse::decode(value) {
            Ok(response) => response,
            Err(e) => {
                debug!(epoch, ?e, "artifact seam: undecodable response");
                self.metrics.dkg_artifact_rejected.inc();
                return false;
            }
        };
        match response {
            ArtifactResponse::NotYet { epoch: answered } => {
                if answered != epoch {
                    debug!(
                        epoch,
                        answered, "artifact seam: a NotYet naming a different epoch"
                    );
                    self.metrics.dkg_artifact_rejected.inc();
                    return false;
                }
                self.wake(epoch, PullAnswer::NotYet);
                true
            }
            ArtifactResponse::Have(artifact) => {
                match verify_artifact_for_epoch(
                    &mut OsRng,
                    self.chain_id,
                    &self.committee,
                    epoch,
                    artifact.as_ref(),
                ) {
                    Ok(()) => {}
                    // This node cannot check the artifact yet, so it neither stores
                    // it nor punishes the peer that sent it. The pull sees no answer
                    // and exhausts, which is the honest outcome.
                    Err(ArtifactError::CommitteeUnreadable(_)) => {
                        self.metrics.dkg_artifact_unverifiable.inc();
                        warn!(
                            epoch,
                            "artifact seam: committee[epoch] is not readable, so a served \
                             artifact could not be checked and was dropped"
                        );
                        return true;
                    }
                    Err(e) => {
                        warn!(epoch, ?e, "artifact seam: rejecting a served artifact");
                        self.metrics.dkg_artifact_rejected.inc();
                        return false;
                    }
                }
                let artifact = *artifact;
                // One decision, under the store's write lock: the value is held
                // (first insert) or comes back as the loser. There is no
                // check-then-insert window in which a producer that raced this pull
                // in could make the served value vanish un-noted.
                match self.store.insert(epoch, artifact) {
                    Ok(()) => {
                        // Read back rather than re-wrapping: every waiter is
                        // woken with the value the store will actually serve.
                        if let Some(held) = self.store.get(epoch) {
                            self.adopt(epoch, &held);
                            self.wake(epoch, PullAnswer::Have(held));
                        }
                    }
                    // A second quorum-certified value for an epoch this node already
                    // holds one for. The same value is nothing new; a different one
                    // is not the peer's fault (its certificate verified) and is not
                    // adjudicable here — the instance bars a second value within
                    // itself, so this is two instances for one target. The store
                    // keeps the held one and notes the second durably, which is what
                    // makes the epoch `Conflict` at the DKG actor. Identity is by
                    // value, so a certificate over the held set and key with other
                    // confirmation metadata is the held value again.
                    Err(served) => {
                        if self.store.note_divergent(epoch, &served) {
                            warn!(
                                epoch,
                                held = %self.store.get(epoch).map(|h| value_digest(&h.0)).unwrap_or_default(),
                                served = %value_digest(&served.0),
                                "artifact seam: a peer served a DIFFERENT quorum-certified artifact \
                                 for an epoch this node already holds one for; keeping the held one \
                                 and reporting the conflict"
                            );
                            self.adopt(epoch, &served);
                        }
                        if let Some(held) = self.store.get(epoch) {
                            self.wake(epoch, PullAnswer::Have(held));
                        }
                    }
                }
                true
            }
        }
    }

    /// Hand a newly-pulled artifact to the agreement write-back, which is the only
    /// route from this seam to the `DkgActor`.
    ///
    /// Sent on a first insert and on a divergent second value (a different
    /// quorum-certified payload for an epoch already held — the actor's `Conflict`
    /// input). A repeat delivery of the held value has nothing new to adopt, and
    /// re-adopting a settled epoch takes a ceremony-retention hold that nothing
    /// releases until the next height tick.
    ///
    /// A refused send is logged and counted, never a `false` from `deliver`: the peer
    /// served an artifact that verified, and this node's own write-back being gone or
    /// backed up is not its misbehaviour.
    fn adopt(&self, epoch: u64, artifact: &AgreedArtifact) {
        hand_off(&self.adopt_tx, epoch, artifact, &self.metrics);
    }

    fn wake(&self, epoch: u64, answer: PullAnswer) {
        let waiters = self
            .waiters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&epoch);
        for tx in waiters.unwrap_or_default() {
            drop(tx.send(answer.clone()));
        }
    }
}

/// The fetching half of the seam.
///
/// Holds the same waiter map the [`ArtifactBridge`] resolves, plus the per-epoch
/// throttle that bounds how often this node asks. Cloneable and `Send + Sync`.
#[derive(Clone)]
pub struct ArtifactPull<E: Clock> {
    context: E,
    bridge: ArtifactBridge,
    /// Per-epoch pull state — one map, one retention rule ([`Self::throttle`] ages
    /// it, a held artifact removes it), so the rotation cursor can never outlive the
    /// throttle slot it rides with.
    slots: Arc<Mutex<HashMap<u64, PullSlot>>>,
    /// This node's own peer key, skipped by the rotation: a member pulling its own
    /// committee's artifact must not spend an attempt asking itself. `None` where the
    /// puller has no identity (tests).
    me: Option<PeerPubkey>,
}

/// What one epoch's pulls carry between attempts.
#[derive(Clone, Copy, Debug)]
struct PullSlot {
    /// Earliest time a pull for the epoch may touch the network again.
    next_allowed: SystemTime,
    /// Pulls issued so far — the round-robin cursor over `committee[epoch]` that
    /// [`ArtifactPull::minter_to_ask`] advances.
    cursor: usize,
}

impl<E: Clock> ArtifactPull<E> {
    pub fn new(context: E, bridge: ArtifactBridge, me: Option<PeerPubkey>) -> Self {
        Self {
            context,
            bridge,
            slots: Arc::new(Mutex::new(HashMap::new())),
            me,
        }
    }

    /// Drop the epoch's slot: the artifact is held, nothing will be pulled again.
    fn forget(&self, epoch: u64) {
        self.slots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&epoch);
    }

    /// The one member of `committee[epoch]` this attempt asks, rotating through the
    /// committee one member per pull; `None` when the committee is unreadable (the
    /// fetch then goes untargeted).
    ///
    /// The minters hold the artifact first, so they are who to ask. The resolver
    /// ranks peers by response time, and a fast `NotYet` is a fast response: an
    /// untargeted fetch re-asks the same non-holder for as long as it keeps
    /// answering, while a peer it never asked stays at the initial estimate and never
    /// comes first. Rotating the target makes a member that keeps answering `NotYet`
    /// cost one attempt, never the pull. A target the resolver does not track costs
    /// one [`PULL_TIMEOUT`] and moves on; this node itself is skipped.
    fn minter_to_ask(&self, epoch: u64) -> Option<NonEmptyVec<PeerPubkey>> {
        let committee = (self.bridge.committee)(epoch)?;
        let others: Vec<&PeerPubkey> = committee
            .bimap
            .iter()
            .filter(|pk| self.me.as_ref() != Some(*pk))
            .collect();
        if others.is_empty() {
            return None;
        }
        let target = {
            let mut slots = self
                .slots
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // `throttle` claimed the slot just before; an absent one (a test that
            // never throttled) starts a fresh walk.
            let slot = slots.entry(epoch).or_insert(PullSlot {
                next_allowed: self.context.current(),
                cursor: 0,
            });
            let target = others[slot.cursor % others.len()].clone();
            slot.cursor = slot.cursor.wrapping_add(1);
            target
        };
        NonEmptyVec::try_from(vec![target]).ok()
    }

    /// One bounded pull for `epoch`.
    ///
    /// - `Some(PullAnswer::Have)` — a peer served it and it verified;
    /// - `Some(PullAnswer::NotYet)` — a peer answered honestly that it has none;
    /// - `None` — the walk was exhausted: nobody answered inside [`PULL_TIMEOUT`].
    ///
    /// A locally-held artifact short-circuits without touching the network. The fetch
    /// is issued no sooner than [`PULL_MIN_INTERVAL`] after the previous one for this
    /// epoch. On the way out an unanswered fetch is cancelled, so the resolver stops
    /// probing peers for a key nobody is waiting on any more.
    pub async fn pull<R>(&self, resolver: &mut R, epoch: u64) -> Option<PullAnswer>
    where
        R: commonware_resolver::Resolver<
            Key = crate::beacon::log_resolver::BeaconFetchKey,
            PublicKey = PeerPubkey,
        >,
    {
        if let Some(held) = self.bridge.store.get(epoch) {
            self.forget(epoch);
            return Some(PullAnswer::Have(held));
        }
        self.throttle(epoch).await;
        // A delivery during the throttle sleep woke only the waiters of that moment;
        // re-read the store before registering, or this attempt would sit out the
        // whole `PULL_TIMEOUT` for an artifact already held.
        if let Some(held) = self.bridge.store.get(epoch) {
            self.forget(epoch);
            return Some(PullAnswer::Have(held));
        }

        let (tx, rx) = oneshot::channel();
        self.bridge
            .waiters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(epoch)
            .or_default()
            .push(tx);
        let key = crate::beacon::log_resolver::BeaconFetchKey::Artifact { epoch };
        match self.minter_to_ask(epoch) {
            Some(minter) => resolver.fetch_targeted(key.clone(), minter).await,
            None => resolver.fetch(key.clone()).await,
        }

        let answer = tokio::select! {
            answer = rx => answer.ok(),
            () = self.context.sleep(PULL_TIMEOUT) => None,
        };
        if matches!(answer, Some(PullAnswer::Have(_))) {
            self.forget(epoch);
        }
        if answer.is_none() {
            self.bridge.metrics.dkg_artifact_pull_exhausted.inc();
            debug!(
                epoch,
                "artifact seam: pull exhausted — no peer answered inside the window"
            );
        }
        // The waiter is spent either way; the cancel is only for the unanswered
        // case, because a delivered answer already completed the fetch.
        let empty = {
            let mut waiters = self
                .bridge
                .waiters
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let empty = match waiters.get_mut(&epoch) {
                Some(v) => {
                    v.retain(|s| !s.is_closed());
                    v.is_empty()
                }
                None => true,
            };
            if empty {
                waiters.remove(&epoch);
            }
            empty
        };
        if empty && answer.is_none() {
            resolver.cancel(key).await;
        }
        answer
    }

    /// Sleep until this epoch's next request slot, then claim it.
    async fn throttle(&self, epoch: u64) {
        let now = self.context.current();
        let wait = {
            let mut slots = self
                .slots
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // Retention: an epoch nobody has pulled for a whole
            // `PULL_MIN_INTERVAL + PULL_TIMEOUT` is done with, so its slot — throttle
            // and cursor — goes; otherwise the map grows one entry per epoch ever
            // pulled. The `PULL_TIMEOUT` term keeps a walk alive across attempts,
            // whose cursor must not restart at the same member.
            slots.retain(|_, slot| slot.next_allowed + PULL_TIMEOUT > now);
            let slot = slots.entry(epoch).or_insert(PullSlot {
                next_allowed: now,
                cursor: 0,
            });
            let wait = slot.next_allowed.duration_since(now).unwrap_or_default();
            slot.next_allowed = slot.next_allowed.max(now) + PULL_MIN_INTERVAL;
            wait
        };
        if !wait.is_zero() {
            self.context.sleep(wait).await;
        }
    }

    /// Epochs with a live pull slot — the bound the retention rule keeps. Test-only.
    #[cfg(test)]
    fn live_slots(&self) -> usize {
        self.slots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beacon::{dkg_agree::DkgProposal, outcome::DkgOutcome};
    use alloy_primitives::{Address, B256};
    use commonware_consensus::{
        simplex::types::{Finalize, Proposal},
        types::{Epoch, Round, View},
    };
    use commonware_cryptography::{
        bls12381::{
            dkg::deal,
            primitives::{sharing::Mode, variant::MinSig},
        },
        ed25519::PrivateKey as Ed25519PrivateKey,
        Signer as _,
    };
    use commonware_math::algebra::Random as _;
    use commonware_resolver::Resolver;
    use commonware_runtime::{deterministic, Runner as _};
    use commonware_utils::{ordered::BiMap, ordered::Set, N3f1, TryCollect as _};
    use fluentbase_bls::{keys::ValidatorBlsKeypair, scheme::build_signer, BlsPubkey, PeerPubkey};
    use fluentbase_staking_reader::reader::{ConsensusKeys, ValidatorWithKeys};
    use rand_08::{rngs::StdRng, SeedableRng as _};

    const CHAIN_ID: u64 = 20_994;
    const TARGET: u64 = 9;
    const N: usize = 4;

    /// A committee's private material, plus everything needed to speak for it.
    struct Committee {
        peers: Vec<Ed25519PrivateKey>,
        bls: Vec<ValidatorBlsKeypair>,
    }

    fn committee(seed: u64) -> Committee {
        let mut rng = StdRng::seed_from_u64(seed);
        Committee {
            peers: (0..N)
                .map(|_| Ed25519PrivateKey::random(&mut rng))
                .collect(),
            bls: (0..N)
                .map(|_| ValidatorBlsKeypair::generate(&mut rng))
                .collect(),
        }
    }

    impl Committee {
        fn bimap(&self) -> BiMap<PeerPubkey, BlsPubkey> {
            self.peers
                .iter()
                .zip(self.bls.iter())
                .map(|(p, b)| {
                    (
                        p.public_key(),
                        BlsPubkey::decode(b.public_bytes().as_slice()).expect("bls pubkey"),
                    )
                })
                .try_collect()
                .expect("unique committee")
        }

        /// Exactly what `RethStakingStateReader::epoch_committee_snapshot` returns.
        fn snapshot(&self, epoch: u64) -> ValidatorSetSnapshot {
            ValidatorSetSnapshot {
                block_hash: B256::repeat_byte(0x11),
                block_number: 4_096,
                epoch,
                validators: self
                    .peers
                    .iter()
                    .zip(self.bls.iter())
                    .enumerate()
                    .map(|(i, (p, b))| ValidatorWithKeys {
                        address: Address::repeat_byte(i as u8 + 1),
                        keys: ConsensusKeys {
                            bls_pubkey: BlsPubkey::decode(b.public_bytes().as_slice())
                                .expect("bls pubkey"),
                            peer_pubkey: p.public_key(),
                            activation_epoch: 0,
                        },
                        tombstoned: false,
                    })
                    .collect(),
                weights: None,
            }
        }

        /// A quorum finalization over `payload` at `(epoch, view)`.
        fn certify(&self, epoch: u64, view: u64, payload: Digest) -> Cert {
            let bimap = self.bimap();
            let ns = dkg_namespace(&fluent_namespace(CHAIN_ID));
            let round = Round::new(Epoch::new(epoch), View::new(view));
            let proposal = Proposal::new(round, View::new(0), payload);
            let finalizes: Vec<_> = self
                .bls
                .iter()
                .take(3)
                .map(|kp| {
                    let signer = build_signer(&ns, bimap.clone(), kp, epoch, None).expect("member");
                    Finalize::sign(&signer, proposal.clone()).expect("sign")
                })
                .collect();
            Finalization::from_finalizes(
                &build_verifier(&ns, bimap, epoch, None),
                finalizes.iter(),
                &Sequential,
            )
            .expect("quorum")
        }
    }

    fn group_key(seed: u64) -> DkgOutcome {
        let mut rng = StdRng::seed_from_u64(seed);
        let players: Set<PeerPubkey> =
            Set::from_iter_dedup((0..N).map(|_| Ed25519PrivateKey::random(&mut rng).public_key()));
        deal::<MinSig, PeerPubkey, N3f1>(&mut rng, Mode::NonZeroCounter, players)
            .expect("deal")
            .0
    }

    fn proposal(epoch: u64) -> DkgProposal {
        DkgProposal {
            target_epoch: epoch,
            logs: (0..N as u8)
                .map(|i| (i, B256::repeat_byte(0x40 + i)))
                .collect(),
            group_key: group_key(7),
            confirms: Vec::new(),
        }
    }

    fn artifact(c: &Committee, epoch: u64) -> AgreedArtifact {
        let p = proposal(epoch);
        let cert = c.certify(epoch, 1, p.digest());
        (p, cert)
    }

    /// The artifact verifies against `committee[E+1]` read from the staking contract
    /// and nothing else: no ceremony, no share, no journal, no block, no
    /// locally-registered scheme. The snapshot below is byte-for-byte the value
    /// `epoch_committee_snapshot` hands back.
    #[test]
    fn an_artifact_verifies_from_the_staking_committee_alone() {
        let c = committee(1);
        let artifact = artifact(&c, TARGET);
        let mut rng = StdRng::seed_from_u64(99);
        verify_artifact_from_snapshot(&mut rng, CHAIN_ID, &c.snapshot(TARGET), &artifact)
            .expect("a quorum-certified artifact verifies against the committee alone");

        // A different committee must not certify it — the check is the quorum, not
        // the shape.
        let other = committee(2);
        let err =
            verify_artifact_from_snapshot(&mut rng, CHAIN_ID, &other.snapshot(TARGET), &artifact)
                .expect_err("a foreign committee must not verify it");
        assert!(matches!(err, ArtifactError::Certificate(TARGET)), "{err:?}");

        // The chain namespace must not verify it either: the agreement plane signs
        // under its own, prefix-free tag.
        let mut chain_ns_verifier_failed = false;
        let bimap = c.bimap();
        let verifier = build_verifier(&fluent_namespace(CHAIN_ID), bimap, TARGET, None);
        if !artifact.1.verify(&mut rng, &verifier, &Sequential) {
            chain_ns_verifier_failed = true;
        }
        assert!(
            chain_ns_verifier_failed,
            "an agreement certificate must not verify under the CHAIN namespace"
        );
    }

    /// A certificate cannot be re-paired with a substituted body, and an artifact
    /// for one epoch cannot be presented as another's.
    #[test]
    fn verification_binds_the_body_and_the_epoch() {
        let c = committee(3);
        let mut rng = StdRng::seed_from_u64(5);
        let (_, cert) = artifact(&c, TARGET);

        let mut swapped = proposal(TARGET);
        swapped.logs[0].1 = B256::repeat_byte(0xEE);
        let err = verify_artifact_from_snapshot(
            &mut rng,
            CHAIN_ID,
            &c.snapshot(TARGET),
            &(swapped, cert.clone()),
        )
        .expect_err("a substituted body must not verify");
        assert!(matches!(err, ArtifactError::PayloadMismatch), "{err:?}");

        // The proposal names TARGET, the certificate names TARGET, the committee
        // read names another epoch.
        let err = verify_artifact_from_snapshot(
            &mut rng,
            CHAIN_ID,
            &c.snapshot(TARGET + 1),
            &(proposal(TARGET), cert.clone()),
        )
        .expect_err("a committee for another epoch must not verify it");
        assert!(
            matches!(err, ArtifactError::EpochMismatch { .. }),
            "{err:?}"
        );

        // A certificate lifted from another epoch's round onto this proposal.
        let lifted = c.certify(TARGET + 1, 1, proposal(TARGET).digest());
        let err = verify_artifact_from_snapshot(
            &mut rng,
            CHAIN_ID,
            &c.snapshot(TARGET),
            &(proposal(TARGET), lifted),
        )
        .expect_err("a certificate from another epoch's round must not verify");
        assert!(
            matches!(err, ArtifactError::EpochMismatch { .. }),
            "{err:?}"
        );
    }

    #[test]
    fn the_wire_round_trips_both_states_and_refuses_junk() {
        let c = committee(4);
        let a = artifact(&c, TARGET);
        let have = ArtifactResponse::Have(Box::new(a.clone()));
        let decoded = ArtifactResponse::decode(have.encode()).expect("Have round-trips");
        match decoded {
            ArtifactResponse::Have(back) => assert_eq!(*back, a),
            other => panic!("wrong arm: {other:?}"),
        }
        let not_yet = ArtifactResponse::NotYet { epoch: TARGET };
        match ArtifactResponse::decode(not_yet.encode()).expect("NotYet round-trips") {
            ArtifactResponse::NotYet { epoch } => assert_eq!(epoch, TARGET),
            other => panic!("wrong arm: {other:?}"),
        }
        assert!(
            ArtifactResponse::decode([2u8].as_slice()).is_err(),
            "unknown tag accepted"
        );

        // The artifact's own record codec, which is what the disk holds.
        assert_eq!(
            decode_artifact(&encode_artifact(&a)).expect("round-trip"),
            a
        );
        let mut trailing = encode_artifact(&a);
        trailing.push(0);
        assert!(
            decode_artifact(&trailing).is_err(),
            "trailing bytes accepted"
        );
        assert!(
            matches!(
                decode_artifact(&vec![0u8; MAX_ARTIFACT_SIZE + 1]),
                Err(ArtifactError::TooLarge(_))
            ),
            "the size cap did not fire"
        );
    }

    /// The store keeps the artifact per epoch, first-wins, and a restart finds it
    /// again — which is the residual the agreement supervisor names: nothing
    /// re-broadcasts a decided proposal, so a restart without this store loses the
    /// artifact outright.
    #[test]
    fn the_store_is_per_epoch_first_wins_and_survives_a_restart() {
        let runner = deterministic::Runner::default();
        runner.start(|context| async move {
            let c = committee(5);
            let mine = artifact(&c, TARGET);
            let (store, writer) = open(
                context.with_label("journal"),
                context.with_label("writer"),
                "artifacts",
            )
            .await
            .expect("open");
            assert!(store.insert(TARGET, mine.clone()).is_ok());
            assert!(
                store.insert(TARGET, artifact(&c, TARGET)).is_err(),
                "a second artifact for one epoch must not displace the first"
            );
            assert!(store.insert(TARGET + 2, artifact(&c, TARGET + 2)).is_ok());
            assert_eq!(store.epochs(), vec![TARGET, TARGET + 2]);
            assert_eq!(*store.get(TARGET).expect("held"), mine);
            assert!(store.get(TARGET + 1).is_none());

            context.sleep(Duration::from_millis(50)).await;
            let writer = writer.expect("a partitioned store has a writer");
            writer.abort();
            drop(writer.await);

            let (restarted, _) = open(
                context.with_label("journal2"),
                context.with_label("writer2"),
                "artifacts",
            )
            .await
            .expect("reopen");
            assert_eq!(
                restarted.epochs(),
                vec![TARGET, TARGET + 2],
                "the artifact did not survive the restart"
            );
            assert_eq!(*restarted.get(TARGET).expect("rehydrated"), mine);
        });
    }

    /// A failing durable half must not gate acceptance: the artifact stands in RAM,
    /// the epoch stays verifiable off it, and the failure is observable.
    ///
    /// Refusing an artifact this node cannot journal would turn a local disk fault
    /// into "no `PK_E` ⇒ every σ of the epoch Pending ⇒ execution parks" — a node
    /// that cannot write would stop being able to verify.
    ///
    /// The injected class is the durable writer being gone (its task died, or the
    /// drain closed), the same arm as a real `Metadata::sync` error: value kept,
    /// durability lost, operator told.
    ///
    /// Falsifier: `insert` answering `false`, or the key going unresolvable, when the
    /// durable half is dead; the counter staying at 0.
    #[test]
    fn an_artifact_whose_durable_write_is_lost_is_kept_in_ram_and_counted() {
        use metrics_util::debugging::{DebugValue, DebuggingRecorder};
        let recorder = DebuggingRecorder::new();
        let snap = recorder.snapshotter();
        metrics::with_local_recorder(&recorder, || {
            let c = committee(5);
            let mine = artifact(&c, TARGET);
            // The writer end dropped: what a dead writer task leaves behind.
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            drop(rx);
            let store = ArtifactStore::with_persistence(Vec::new(), tx);
            assert!(
                store.insert(TARGET, mine.clone()).is_ok(),
                "the artifact is ACCEPTED — the durable half does not gate it"
            );
            assert_eq!(
                *store.get(TARGET).expect("held in RAM"),
                mine,
                "and it is the artifact that was offered, not a placeholder"
            );
            // And the node keeps verifying: the key of the epoch resolves off the RAM
            // half alone, which is the whole point of accepting it.
            let keys = key_index_over(store.clone(), &[TARGET]);
            assert!(
                keys.key_at(TARGET).is_some(),
                "`PK_epoch` resolves, so certificates of the epoch are still checked"
            );
            assert_eq!(
                keys.holds_mint_of(TARGET),
                Some(true),
                "and nothing asks the network for an artifact this node holds"
            );
        });
        let lost: u64 = snap
            .snapshot()
            .into_vec()
            .into_iter()
            .filter(|(k, ..)| k.key().name() == "dpos_artifact_store_handoff_failed_total")
            .map(|(.., v)| match v {
                DebugValue::Counter(c) => c,
                _ => 0,
            })
            .sum();
        assert_eq!(
            lost, 1,
            "the lost durable write is OBSERVABLE — one counter increment, named"
        );
    }

    /// Retention keeps every mint the window still resolves to: with mints at
    /// {2, 5, 55} and the window `[52, 62]` at `now = 60`, epoch 52 resolves to the
    /// mint at 5 and epochs 55.. to the one at 55, so both stay and 2 goes; with
    /// mints at {2, 5, 40} the window resolves to 40 alone, so 2 and 5 go.
    #[test]
    fn retention_keeps_the_mints_the_window_resolves_to() {
        const WINDOW: u64 = 8;
        let c = committee(21);

        let store = ArtifactStore::new();
        for mint in [2u64, 5, 55] {
            assert!(store.insert(mint, artifact(&c, mint)).is_ok());
        }
        store.retain_mints_for_window(60, WINDOW);
        assert_eq!(
            store.epochs(),
            vec![5, 55],
            "5 is minted_at(52), the oldest epoch in the window, so it stays; 2 is \
             superseded before the window opens"
        );

        let store = ArtifactStore::new();
        for mint in [2u64, 5, 40] {
            assert!(store.insert(mint, artifact(&c, mint)).is_ok());
        }
        store.retain_mints_for_window(60, WINDOW);
        assert_eq!(
            store.epochs(),
            vec![40],
            "40 is minted_at(52) and of every epoch above; 2 and 5 are superseded"
        );
    }

    /// On a stable committee the only mint can sit far below the window floor and
    /// must survive: it is the key every epoch in the window resolves to.
    #[test]
    fn retention_keeps_a_stable_committees_only_mint_far_below_the_floor() {
        let c = committee(22);
        let store = ArtifactStore::new();
        assert!(store.insert(2, artifact(&c, 2)).is_ok());
        store.retain_mints_for_window(60, 8);
        assert_eq!(store.epochs(), vec![2], "the live key was evicted");
        store.retain_mints_for_window(10_000, 8);
        assert_eq!(store.epochs(), vec![2], "the live key was evicted");
    }

    /// The journal follows the RAM store: a mint retained out of RAM is gone from
    /// disk after the writer's next turn, and a reopened store does not bring it back.
    #[test]
    fn retention_reaches_the_journal() {
        let runner = deterministic::Runner::default();
        runner.start(|context| async move {
            let c = committee(23);
            let (store, writer) = open(
                context.with_label("journal"),
                context.with_label("writer"),
                "retained_artifacts",
            )
            .await
            .expect("open the artifact store");
            for mint in [2u64, 5, 55] {
                assert!(store.insert(mint, artifact(&c, mint)).is_ok());
            }
            store.retain_mints_for_window(60, 8);
            assert_eq!(store.epochs(), vec![5, 55]);
            context.sleep(Duration::from_millis(50)).await;
            let writer = writer.expect("a partitioned store has a writer");
            writer.abort();
            drop(writer.await);

            let (reopened, _) = open(
                context.with_label("journal2"),
                context.with_label("writer2"),
                "retained_artifacts",
            )
            .await
            .expect("reopen the artifact store");
            assert_eq!(
                reopened.epochs(),
                vec![5, 55],
                "the journal kept a mint the RAM store retained out"
            );
        });
    }

    /// A stable committee's mint outlives the retention window, and so does a
    /// restart: the durable memo addresses the artifact that a blind chain can no
    /// longer reach.
    #[test]
    fn a_stable_committees_mint_outlives_the_retention_window() {
        let runner = deterministic::Runner::default();
        runner.start(|context| async move {
            const BOOTSTRAP: u64 = super::super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH;
            /// Ten windows above the mint: whatever the window is, the frontier is
            /// far outside it.
            const WINDOWS: u64 = 10;
            let frontier = BOOTSTRAP + WINDOWS * crate::SCHEME_RETENTION_EPOCHS as u64;

            let c = committee(11);
            let mint = artifact(&c, BOOTSTRAP);
            let minted_pk = *group_public_key(&mint.0.group_key);

            let (store, store_writer) = open(
                context.with_label("journal"),
                context.with_label("writer"),
                "stable_artifacts",
            )
            .await
            .expect("open the artifact store");
            assert!(store.insert(BOOTSTRAP, mint.clone()).is_ok());

            // The chain: one mint, at the bootstrap epoch, and a committee that never
            // changes after it. The asked-epoch log shows the second process asking
            // nothing.
            let asked: Arc<Mutex<Vec<u64>>> = Arc::default();
            let stable: ChangedAt = {
                let asked = asked.clone();
                Arc::new(move |epoch: u64| {
                    asked.lock().expect("asked").push(epoch);
                    Some(false)
                })
            };
            let (mints, memo_writer) = open_mint_memo(
                context.with_label("memo_journal"),
                context.with_label("memo_writer"),
                "stable_mints",
                stable,
            )
            .await
            .expect("open the mint memo");
            let keys = KeyIndex::new(store.clone(), mints);

            // Every epoch from the mint to the far frontier is minted by the
            // bootstrap epoch, resolved one epoch at a time as the live callers do
            // (one resolve per epoch entered).
            for epoch in BOOTSTRAP..=frontier {
                assert_eq!(
                    keys.minted_at(epoch),
                    Some(BOOTSTRAP),
                    "epoch {epoch} resolved to a mint other than the only one there is"
                );
            }
            assert!(
                !asked.lock().expect("asked").is_empty(),
                "the walk never read the chain, so the memo below is memoising nothing"
            );

            // And the key is served there: the window is an epoch count and the
            // frontier is ten of them above the mint, and nothing evicted the object
            // that answers.
            assert_eq!(
                keys.key_at(frontier),
                Some(minted_pk),
                "the mint's key is gone at a frontier {WINDOWS} windows above it"
            );
            assert_eq!(
                keys.sharing_at(frontier).map(|(at, _)| at),
                Some(BOOTSTRAP),
                "the polynomial is served from an epoch other than the mint"
            );
            assert_eq!(
                store.epochs(),
                vec![BOOTSTRAP],
                "the artifact store gained or lost an entry over the walk"
            );

            context.sleep(Duration::from_millis(50)).await;
            for writer in [
                store_writer.expect("a partitioned store has a writer"),
                memo_writer.expect("a partitioned memo has a writer"),
            ] {
                writer.abort();
                drop(writer.await);
            }

            // The restart, over a chain that can no longer answer the bit: the walk
            // has nothing to bottom out on and the memo is the whole answer.
            let blind_asks: Arc<Mutex<Vec<u64>>> = Arc::default();
            let blind: ChangedAt = {
                let asks = blind_asks.clone();
                Arc::new(move |epoch: u64| {
                    asks.lock().expect("asks").push(epoch);
                    None
                })
            };
            let (restarted_store, _) = open(
                context.with_label("journal2"),
                context.with_label("writer2"),
                "stable_artifacts",
            )
            .await
            .expect("reopen the artifact store");
            let (restarted_mints, _) = open_mint_memo(
                context.with_label("memo_journal2"),
                context.with_label("memo_writer2"),
                "stable_mints",
                blind.clone(),
            )
            .await
            .expect("reopen the mint memo");
            let restarted = KeyIndex::new(restarted_store, restarted_mints);
            assert_eq!(
                restarted.key_at(frontier),
                Some(minted_pk),
                "a restarted node holds the artifact and cannot address it — the memo                  did not survive"
            );
            assert!(
                blind_asks.lock().expect("asks").is_empty(),
                "the restarted walk asked the chain after all, so the answer above is                  not the memo's: {:?}",
                blind_asks.lock().expect("asks")
            );

            // Negative control: the same blind chain with a fresh memo answers
            // nothing, so the restart above is the memo's durability and not
            // something the walk would have managed anyway.
            let fresh = KeyIndex::new(store.clone(), MintIndex::new(blind));
            assert_eq!(
                fresh.key_at(frontier),
                None,
                "an empty memo resolved a mint below the chain's own read window"
            );
        });
    }

    /// A bridge plus the write-back end it adopts into. The receiver is returned
    /// rather than dropped because dropping it closes the channel, which would turn
    /// every adopt into a refusal.
    fn bridge_and_adopt(
        c: &Committee,
        epoch: u64,
    ) -> (ArtifactBridge, tokio::sync::mpsc::Receiver<AgreedArtifact>) {
        bridge_over(c, epoch, ArtifactStore::new())
    }

    /// [`bridge_and_adopt`] over an explicit `store`.
    fn bridge_over(
        c: &Committee,
        epoch: u64,
        store: ArtifactStore,
    ) -> (ArtifactBridge, tokio::sync::mpsc::Receiver<AgreedArtifact>) {
        let snapshot = c.snapshot(epoch);
        let source: CommitteeSource = Arc::new(move |e| {
            (e == snapshot.epoch)
                .then(|| epoch_committee_from_snapshot(&snapshot).expect("committee"))
        });
        let (adopt_tx, adopt_rx) = tokio::sync::mpsc::channel(16);
        (
            ArtifactBridge::new(CHAIN_ID, store, source, adopt_tx, BeaconMetrics::default()),
            adopt_rx,
        )
    }

    /// A served value that loses the insert is not lost: a different value is noted
    /// as the divergence witness, written to disk by the store itself and read back
    /// by a store reopened over the directory, and handed to the write-back; the same
    /// value again is nothing new. The peer served a certified artifact either way,
    /// so the delivery is honest.
    #[test]
    fn a_served_value_that_loses_the_insert_is_noted_durably_not_lost() {
        let dir = std::env::temp_dir().join(format!(
            "beacon-artifact-divergent-{}-{}",
            std::process::id(),
            TARGET
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let c = committee(8);
        let store = ArtifactStore::new().with_conflict_dir(dir.clone());
        let (fetching, mut adopt_rx) = bridge_over(&c, TARGET, store.clone());
        // The local instance's value lands first (not through the bridge).
        let mine = artifact(&c, TARGET);
        assert!(store.insert(TARGET, mine.clone()).is_ok());
        assert!(adopt_rx.try_recv().is_err(), "nothing handed off yet");

        // A peer serves a different certified value for the same epoch.
        let other = {
            let mut p = proposal(TARGET);
            p.logs.pop();
            let cert = c.certify(TARGET, 1, p.digest());
            (p, cert)
        };
        assert_ne!(value_digest(&other.0), value_digest(&mine.0));
        let served = ArtifactResponse::Have(Box::new(other.clone())).encode();
        assert!(
            fetching.deliver(TARGET, served.as_ref()),
            "a certified value that lost the insert is an honest delivery"
        );
        assert_eq!(*store.get(TARGET).expect("held"), mine, "first-wins");
        assert_eq!(
            store.view(TARGET).expect("held").1,
            Some(value_digest(&other.0)),
            "the loser is the divergence witness"
        );
        assert_eq!(
            share_state::load_conflict(&dir, TARGET),
            Some(share_state::ConflictMarker::Pair(
                value_digest(&mine.0),
                value_digest(&other.0)
            )),
            "the STORE wrote the marker the instant it noted the value"
        );
        assert_eq!(
            adopt_rx
                .try_recv()
                .expect("the loser is handed to the write-back")
                .0,
            other.0,
        );

        // The held value served again: the same value, nothing new.
        let again = ArtifactResponse::Have(Box::new(artifact(&c, TARGET))).encode();
        assert!(fetching.deliver(TARGET, again.as_ref()));
        assert!(
            adopt_rx.try_recv().is_err(),
            "the held value is not re-adopted"
        );
        assert_eq!(
            store.view(TARGET).expect("held").1,
            Some(value_digest(&other.0)),
            "the witness is first-wins too"
        );

        // A store reopened over the directory knows the witness again — without
        // the actor's verdict ever having run in between.
        let reopened = ArtifactStore::new().with_conflict_dir(dir.clone());
        assert!(reopened.insert(TARGET, mine).is_ok());
        assert_eq!(
            reopened.view(TARGET).expect("held").1,
            Some(value_digest(&other.0)),
            "the reopened store reloaded its marker"
        );
        assert!(
            !reopened.note_divergent(TARGET, &other),
            "a reloaded witness is not noted twice"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A producer that lacks the artifact answers `NotYet` instead of dropping the
    /// responder, and the consumer takes both states as honest deliveries — so
    /// `deliver` never returns false for an honest peer and the un-removable
    /// `excluded`-peer defect cannot fire. `false` is reserved for proven
    /// misbehaviour.
    #[test]
    fn both_states_are_honest_deliveries_and_only_forgery_returns_false() {
        let c = committee(6);
        let (serving, _serving_adopt) = bridge_and_adopt(&c, TARGET);
        let (fetching, _fetching_adopt) = bridge_and_adopt(&c, TARGET);

        // A producer with nothing still answers, and the answer is NotYet.
        let answer = serving.produce(TARGET);
        assert!(matches!(
            ArtifactResponse::decode(answer.as_ref()).expect("decodes"),
            ArtifactResponse::NotYet { epoch: TARGET }
        ));
        assert!(
            fetching.deliver(TARGET, answer.as_ref()),
            "an honest NotYet must be a delivery, not a refusal"
        );
        assert!(!fetching.store.has(TARGET));

        // Once it holds one, it serves it, and the far side takes it.
        let mine = artifact(&c, TARGET);
        assert!(serving.store.insert(TARGET, mine.clone()).is_ok());
        let answer = serving.produce(TARGET);
        assert!(
            fetching.deliver(TARGET, answer.as_ref()),
            "a valid artifact must be delivered"
        );
        assert_eq!(*fetching.store.get(TARGET).expect("stored"), mine);

        // A forged artifact — certified by a committee that is not this epoch's
        // — is proven misbehaviour, and only that returns false.
        let (liar, _liar_adopt) = bridge_and_adopt(&c, TARGET);
        let forged = artifact(&committee(7), TARGET);
        let bytes = ArtifactResponse::Have(Box::new(forged)).encode();
        assert!(
            !liar.deliver(TARGET, bytes.as_ref()),
            "a forged artifact must be refused"
        );
        assert!(
            !liar.deliver(TARGET, b"not an artifact"),
            "junk must be refused"
        );
        assert!(
            !liar.deliver(
                TARGET,
                ArtifactResponse::NotYet { epoch: TARGET + 3 }
                    .encode()
                    .as_ref()
            ),
            "a NotYet about another epoch must be refused"
        );

        // An unreadable committee is a fact about this node, never a verdict on
        // the peer: the artifact is dropped and the peer keeps its standing.
        let (blind_adopt_tx, _blind_adopt) = tokio::sync::mpsc::channel(16);
        let blind = ArtifactBridge::new(
            CHAIN_ID,
            ArtifactStore::new(),
            Arc::new(|_| None),
            blind_adopt_tx,
            BeaconMetrics::default(),
        );
        assert!(
            blind.deliver(TARGET, serving.produce(TARGET).as_ref()),
            "an unverifiable artifact must not cost an honest peer its standing"
        );
        assert!(
            !blind.store.has(TARGET),
            "an unchecked artifact must not be stored"
        );
    }

    /// A pulled artifact must reach the agreement write-back, and reach it exactly
    /// once.
    ///
    /// The store answers every `PK_epoch` read and serves peers, but nothing that
    /// reads it reaches [`DkgActor::on_artifact`] — so without this push a node whose
    /// own instance died mid-agreement can verify the epoch key it pulled and still
    /// never derive its share for that epoch.
    #[test]
    fn a_pulled_artifact_is_handed_to_the_write_back_once() {
        let c = committee(10);
        let (serving, _serving_adopt) = bridge_and_adopt(&c, TARGET);
        let (fetching, mut adopt) = bridge_and_adopt(&c, TARGET);

        assert!(fetching.deliver(TARGET, serving.produce(TARGET).as_ref()));
        assert!(
            adopt.try_recv().is_err(),
            "a NotYet carries no set to adopt"
        );

        let mine = artifact(&c, TARGET);
        assert!(serving.store.insert(TARGET, mine.clone()).is_ok());
        assert!(fetching.deliver(TARGET, serving.produce(TARGET).as_ref()));
        assert_eq!(
            adopt
                .try_recv()
                .expect("the write-back must be handed the pulled artifact"),
            mine
        );

        // Re-delivering an epoch the store already holds has no new set to adopt,
        // and re-adopting a settled one would take a retention hold nothing
        // releases until the next height tick.
        assert!(fetching.deliver(TARGET, serving.produce(TARGET).as_ref()));
        assert!(
            adopt.try_recv().is_err(),
            "a repeat delivery must not re-enter the write-back"
        );

        let (liar, mut liar_adopt) = bridge_and_adopt(&c, TARGET);
        let forged = ArtifactResponse::Have(Box::new(artifact(&committee(11), TARGET))).encode();
        assert!(!liar.deliver(TARGET, forged.as_ref()));
        assert!(
            liar_adopt.try_recv().is_err(),
            "a forgery must never reach the write-back"
        );
    }

    /// A resolver that answers every fetch from a serving bridge, counting what
    /// it was asked. `silent` models a network where nobody answers at all.
    #[derive(Clone)]
    struct FakeResolver {
        serving: ArtifactBridge,
        fetching: ArtifactBridge,
        silent: bool,
        fetches: Arc<Mutex<Vec<u64>>>,
        cancels: Arc<Mutex<Vec<u64>>>,
        /// The peer each targeted fetch named, in order.
        targets: Arc<Mutex<Vec<PeerPubkey>>>,
    }

    impl Resolver for FakeResolver {
        type Key = crate::beacon::log_resolver::BeaconFetchKey;
        type PublicKey = PeerPubkey;

        async fn fetch(&mut self, key: Self::Key) {
            let crate::beacon::log_resolver::BeaconFetchKey::Artifact { epoch } = key else {
                return;
            };
            self.fetches
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(epoch);
            if self.silent {
                return;
            }
            let answer = self.serving.produce(epoch);
            assert!(
                self.fetching.deliver(epoch, answer.as_ref()),
                "the seam refused an honest answer"
            );
        }
        async fn fetch_all(&mut self, _: Vec<Self::Key>) {}
        // Targeting narrows who is asked; the fake network's one peer answers
        // regardless, so a targeted fetch is the untargeted one plus a record of
        // the target.
        async fn fetch_targeted(&mut self, key: Self::Key, targets: NonEmptyVec<Self::PublicKey>) {
            self.targets
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .extend(targets.iter().cloned());
            self.fetch(key).await;
        }
        async fn fetch_all_targeted(&mut self, _: Vec<(Self::Key, NonEmptyVec<Self::PublicKey>)>) {}
        async fn cancel(&mut self, key: Self::Key) {
            if let crate::beacon::log_resolver::BeaconFetchKey::Artifact { epoch } = key {
                self.cancels
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(epoch);
            }
        }
        async fn clear(&mut self) {}
        async fn retain(&mut self, _: impl Fn(&Self::Key) -> bool + Send + 'static) {}
    }

    /// The caller sees all three outcomes and is never left in silent retry: `Have`,
    /// `NotYet`, and — when nobody answers — an exhausted walk as `None`, which is
    /// the cert-follow RPC seam's semantics rather than `Consumer::failed`'s silence.
    #[test]
    fn the_pull_surfaces_have_not_yet_and_an_exhausted_walk() {
        let runner = deterministic::Runner::timed(Duration::from_secs(600));
        runner.start(|context| async move {
            let c = committee(8);
            let (serving, _serving_adopt) = bridge_and_adopt(&c, TARGET);
            let (fetching, _fetching_adopt) = bridge_and_adopt(&c, TARGET);
            let mut resolver = FakeResolver {
                serving: serving.clone(),
                fetching: fetching.clone(),
                silent: false,
                fetches: Arc::new(Mutex::new(Vec::new())),
                cancels: Arc::new(Mutex::new(Vec::new())),
                targets: Arc::new(Mutex::new(Vec::new())),
            };
            let pull = ArtifactPull::new(context.clone(), fetching.clone(), None);

            // The peer has not converged yet — the normal state for most of `E`.
            assert!(
                matches!(
                    pull.pull(&mut resolver, TARGET).await,
                    Some(PullAnswer::NotYet)
                ),
                "a peer without the artifact must answer, not go silent"
            );

            // It converges; the very next pull carries the artifact.
            let mine = artifact(&c, TARGET);
            assert!(serving.store.insert(TARGET, mine.clone()).is_ok());
            match pull.pull(&mut resolver, TARGET).await {
                Some(PullAnswer::Have(got)) => assert_eq!(*got, mine),
                other => panic!("expected the artifact, got {other:?}"),
            }
            // And now it is local: no further fetch is issued for it.
            let issued = resolver
                .fetches
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len();
            assert!(matches!(
                pull.pull(&mut resolver, TARGET).await,
                Some(PullAnswer::Have(_))
            ));
            assert_eq!(
                resolver
                    .fetches
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .len(),
                issued,
                "a locally-held artifact must not touch the network"
            );

            // Nobody answers at all: the walk exhausts and the caller is told,
            // instead of parking on a fetch the resolver retries forever.
            resolver.silent = true;
            assert!(
                pull.pull(&mut resolver, TARGET + 1).await.is_none(),
                "an unanswered pull must surface as None"
            );
            assert_eq!(
                *resolver
                    .cancels
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
                vec![TARGET + 1],
                "an exhausted pull must cancel its fetch"
            );
        });
    }

    /// Repeated pulls for one epoch are rate-bounded at the caller, which keeps this
    /// seam from tripping a peer's inbound quota — an over-quota sleeps the whole
    /// connection to that peer, not just this channel.
    #[test]
    fn repeated_pulls_are_rate_bounded_per_epoch() {
        let runner = deterministic::Runner::timed(Duration::from_secs(600));
        runner.start(|context| async move {
            let c = committee(9);
            let (fetching, _fetching_adopt) = bridge_and_adopt(&c, TARGET);
            let (serving, _serving_adopt) = bridge_and_adopt(&c, TARGET);
            let mut resolver = FakeResolver {
                serving,
                fetching: fetching.clone(),
                silent: false,
                fetches: Arc::new(Mutex::new(Vec::new())),
                cancels: Arc::new(Mutex::new(Vec::new())),
                targets: Arc::new(Mutex::new(Vec::new())),
            };
            let pull = ArtifactPull::new(context.clone(), fetching, None);

            let start = context.current();
            for _ in 0..4 {
                assert!(matches!(
                    pull.pull(&mut resolver, TARGET).await,
                    Some(PullAnswer::NotYet)
                ));
            }
            assert_eq!(
                resolver
                    .fetches
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .len(),
                4
            );
            // Each attempt is addressed to one minter, and successive attempts walk
            // the committee in its Commonware order: a member that keeps answering
            // `NotYet` costs one attempt, never the pull. (The resolver ranks peers by
            // response time; an untargeted fetch re-asks a fast non-holder forever.)
            let minters: Vec<PeerPubkey> = epoch_committee_from_snapshot(&c.snapshot(TARGET))
                .expect("committee")
                .bimap
                .iter()
                .cloned()
                .collect();
            assert_eq!(
                *resolver
                    .targets
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
                minters[..4].to_vec(),
                "four attempts, four different minters, in committee order"
            );
            let elapsed = context.current().duration_since(start).expect("monotonic");
            assert!(
                elapsed >= PULL_MIN_INTERVAL * 3,
                "four pulls took {elapsed:?}, which is faster than the per-epoch bound allows"
            );

            // A different epoch has its own slot and is not held behind this one.
            let before = context.current();
            assert!(matches!(
                pull.pull(&mut resolver, TARGET + 5).await,
                Some(PullAnswer::NotYet)
            ));
            assert!(
                context.current().duration_since(before).expect("monotonic") < PULL_MIN_INTERVAL,
                "the throttle is per epoch, not global"
            );

            // The slot map is bounded by one rule: an epoch nobody pulled for a whole
            // interval + timeout is forgotten (throttle and rotation cursor alike),
            // and a held artifact forgets its epoch at once — no entry per epoch ever
            // pulled, forever.
            assert_eq!(pull.live_slots(), 2, "two epochs in flight, two slots");
            context.sleep(PULL_MIN_INTERVAL + PULL_TIMEOUT).await;
            assert_eq!(
                pull.live_slots(),
                2,
                "nothing is dropped before its slot is stale"
            );
            context.sleep(Duration::from_millis(1)).await;
            assert!(matches!(
                pull.pull(&mut resolver, TARGET + 6).await,
                Some(PullAnswer::NotYet)
            ));
            assert_eq!(
                pull.live_slots(),
                1,
                "a pull ages the stale slots out and keeps only its own"
            );
            // Held locally (the store hit that short-circuits the network): forgotten.
            let mine = artifact(&c, TARGET + 6);
            assert!(resolver.fetching.store.insert(TARGET + 6, mine).is_ok());
            assert!(matches!(
                pull.pull(&mut resolver, TARGET + 6).await,
                Some(PullAnswer::Have(_))
            ));
            assert_eq!(pull.live_slots(), 0, "a held artifact forgets its slot");

            // The production shape skips the puller itself: a member pulling its own
            // committee's artifact walks the other members only, in order, wrapping.
            let me = minters[1].clone();
            let member =
                ArtifactPull::new(context.clone(), resolver.fetching.clone(), Some(me.clone()));
            let asked_before = resolver
                .targets
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len();
            for _ in 0..4 {
                assert!(matches!(
                    member.pull(&mut resolver, TARGET).await,
                    Some(PullAnswer::NotYet)
                ));
            }
            let asked: Vec<PeerPubkey> = resolver
                .targets
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)[asked_before..]
                .to_vec();
            assert_eq!(
                asked,
                vec![
                    minters[0].clone(),
                    minters[2].clone(),
                    minters[3].clone(),
                    minters[0].clone()
                ],
                "a member walks the other members, wrapping, and never asks itself"
            );
            assert!(!asked.contains(&me));

            // An artifact that lands during the throttle sleep is found before the
            // network is asked: the sleep answers only the waiters of its moment, so
            // the store is re-read after it — otherwise this attempt would wait out the
            // whole `PULL_TIMEOUT` for a body already held.
            assert!(matches!(
                pull.pull(&mut resolver, TARGET + 8).await,
                Some(PullAnswer::NotYet)
            ));
            let issued = resolver
                .fetches
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len();
            let landing = artifact(&c, TARGET + 8);
            let store = resolver.fetching.store.clone();
            drop(context.with_label("landing").spawn(move |ctx| async move {
                ctx.sleep(Duration::from_secs(1)).await;
                assert!(store.insert(TARGET + 8, landing).is_ok());
            }));
            assert!(
                matches!(
                    pull.pull(&mut resolver, TARGET + 8).await,
                    Some(PullAnswer::Have(_))
                ),
                "an artifact held by the end of the throttle is answered from the store"
            );
            assert_eq!(
                resolver
                    .fetches
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .len(),
                issued,
                "and the network is not asked for it"
            );
            assert_eq!(
                pull.live_slots(),
                0,
                "the found artifact forgets its slot too"
            );
        });
    }
}
