//! The agreement artifact: what the plane leaves behind, where it is kept, and
//! how a node that never ran the ceremony gets it.
//!
//! # Why this exists at all
//!
//! No block carries the epoch key any more, so every node
//! that never ran the ceremony — followers, observers, a cold-started node, the
//! STF verifier — holds no dealer logs, cannot recompute `PK_E`, and has no
//! other source for it. **This artifact is their only one.** So it must be
//! checkable by someone who has nothing but the staking contract:
//! [`verify_artifact`] takes the artifact and `committee[epoch]` and needs no
//! local ceremony state, no share, no journal and no block.
//!
//! # The three pieces
//!
//! - **The artifact** is `(DkgProposal, Finalization)` — the certified payload
//!   plus simplex's own finalization certificate, which is a quorum of
//!   `committee[target_epoch]` BLS signatures over the payload digest under the
//!   agreement namespace. Attributability of the signers comes from the multisig
//!   bitmap; there is no attestation layer on top of it.
//! - **The store** ([`ArtifactStore`]) is per-epoch, in memory and on disk, and
//!   is what a peer is served from. It is also what closes the residual the
//!   agreement supervisor names today: a node that restarts after its instance
//!   finalized loses the artifact entirely, because no live sender re-broadcasts
//!   a decided proposal (`dkg_agree_body_lost_total` counts it). The durable half
//!   is written on the same edge the artifact is produced, so recovering it after
//!   a restart is a read, not a re-agreement — [`restart_replay`] is the half of
//!   that which the `DkgActor` needs, since its own intake only ever hears from a
//!   live instance.
//! - **The pull seam** ([`ArtifactBridge`] serving, [`ArtifactPull`] fetching)
//!   rides the DKG-log resolver's key space
//!   ([`BeaconFetchKey`](crate::beacon::log_resolver::BeaconFetchKey)), i.e. the
//!   `BEACON_RESOLVER_CHANNEL` engine at 16/s — never a 128/s consensus quota,
//!   because an inbound over-quota sleeps the WHOLE connection to that peer and
//!   stalls its other channels.
//!
//! # A pulled artifact enters the write-back, it does not just land in the store
//!
//! Filing an artifact answers the `PK_epoch` rungs and lets this node serve peers,
//! and neither of those reaches
//! [`DkgActor::on_artifact`](crate::beacon::actor::DkgActor::on_artifact) — that is
//! fed by the agreement write-back alone. So on a FIRST insert the seam hands the
//! artifact to the same write-back channel a live instance and [`restart_replay`]
//! enter on, and the actor's existing adoption, first-wins and retention-hold rules
//! run over it unchanged. Without that hop the member whose own instance died
//! mid-agreement — its peers decided without it and their launchers hold the target
//! in `started`, so no re-agreement is coming — can verify the epoch key it pulled
//! and still be permanently shareless for the epoch it was elected to sign in.
//!
//! # The seam carries exactly two states, and the reason there is no third
//!
//! [`ArtifactResponse`] is `Have` or `NotYet{epoch}`. Both are honest
//! DELIVERIES, never a dropped responder — which is the whole point:
//! `Consumer::deliver` returning `false` inserts the peer into commonware's
//! resolver `excluded` set (`resolver/src/p2p/fetcher.rs:94`, `:189`, `:242`,
//! `:516`), and that set has **no removal path anywhere**, so a peer blocked once
//! is blocked for the life of the process. Here `false` is reserved for proven
//! misbehaviour — bytes that do not decode, or a certificate that fails against
//! the committee — and every other answer, including "I do not have it", is a
//! delivered value.
//!
//! There is deliberately **no `Never{epoch}`**. Nothing in this design can
//! produce it honestly: the store has no eviction policy, the plane never aborts
//! and the instance is never restarted, so for any epoch that is still a target
//! "no peer will ever hold it" is not a true answer — and a wrong "never" tells a
//! peer to stop asking for a key it genuinely needs. The state can only return
//! together with a retention policy, and not before.
//!
//! # The caller-facing shape is the RPC seam's, not the resolver's
//!
//! [`ArtifactPull::pull`] answers `Some(Have)`, `Some(NotYet)` or `None`, where
//! `None` means the walk was exhausted — nobody answered inside the window. That
//! is the cert-follow RPC seam's semantics (`node/src/cert_follow/upstream.rs:372`,
//! `:388-390`, `:415-464`: a bounded one-pass walk whose exhaustion reaches the
//! caller). It is explicitly NOT `Consumer::failed`, which fires solely on
//! Cancel/Retain/Clear and never on an error or a timeout
//! (`resolver/src/p2p/engine.rs:261`, `:278`, `:299`, `:444-457`), leaving the
//! caller in unbounded silent retry with no signal above the transport.

use bytes::{Buf, BufMut, Bytes};
use commonware_codec::{
    Decode as _, DecodeExt as _, Encode as _, EncodeSize, Error as CodecError, FixedSize as _,
    Read, ReadExt as _, Write,
};
use commonware_consensus::simplex::types::Finalization;
use commonware_parallel::Sequential;
use commonware_runtime::{Clock, Handle, Metrics, Spawner, Storage};
use commonware_storage::metadata::{Config as MetadataConfig, Error as MetadataError, Metadata};
use commonware_utils::sequence::U64;
use fluentbase_bls::{
    beacon::dkg_namespace, fluent_namespace, scheme::build_verifier, EpochCommittee,
    Scheme as BlsScheme,
};
use fluentbase_p2p::constants::MAX_COMMITTEE_SIZE;
// Only [`verify_artifact_from_snapshot`] speaks the snapshot type, and that is
// test-only.
#[cfg(test)]
use fluentbase_staking_reader::reader::ValidatorSetSnapshot;
use rand_core::{CryptoRngCore, OsRng};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::Path,
    sync::{Arc, Mutex, RwLock},
    time::{Duration, SystemTime},
};
use tokio::sync::{
    mpsc::{error::TrySendError, UnboundedReceiver},
    oneshot,
};
use tracing::{debug, warn};

#[cfg(test)]
use crate::scheme::epoch_committee_from_snapshot;
use crate::{
    beacon::{dkg_agree::AgreedArtifact, metrics::BeaconMetrics, share_state},
    digest::Digest,
};

/// The finalization-certificate half of the artifact.
type Cert = Finalization<BlsScheme, Digest>;

/// Decode cap for one served artifact.
///
/// Worst case at `n = 51`: a 51-entry log set (~1.7 KiB), a 64 KiB outcome cap
/// ([`MAX_BEACON_OUTCOME_SIZE`](crate::beacon::outcome::MAX_BEACON_OUTCOME_SIZE)),
/// 51 confirmations each carrying a 51-entry recorded set and a 64-byte signature
/// (~88 KiB), and a certificate well under 1 KiB — about 154 KiB. The cap is
/// rounded up from that, and it is a NETWORK-WIDE constant rather than a bound
/// derived from the live committee, so every node accepts and refuses the same
/// bytes.
pub(crate) const MAX_ARTIFACT_SIZE: usize = 256 * 1024;

/// Shortest gap between two pulls for one target epoch.
///
/// The per-peer bound this seam owes the network. The resolver picks the peer and
/// the channel quota (16/s) bounds the far side, but nothing upstream stops a
/// caller from re-issuing a fetch in a tight loop — and an inbound over-quota
/// SLEEPS THE WHOLE CONNECTION to that peer, stalling its consensus channels
/// along with this one. `NotYet` is the normal answer for most of an epoch, so
/// this is the common path, not the exceptional one.
pub(crate) const PULL_MIN_INTERVAL: Duration = Duration::from_secs(5);

/// How long one pull waits for an answer before reporting the walk exhausted.
///
/// Bounds the caller the way the RPC seam's one-pass walk does: an isolated node
/// gets `None` and can act on it, instead of parking on a fetch the resolver will
/// silently retry forever.
pub(crate) const PULL_TIMEOUT: Duration = Duration::from_secs(8);

/// How `committee[epoch]` is read — the ONLY input artifact verification needs
/// beyond the artifact itself.
///
/// A closure rather than a trait because the production implementation is a
/// staking-contract read (`epoch_committee_snapshot` at a finalized hash) that
/// lives above this crate, and because that is already how the epoch-boundary
/// orchestrator takes its own chain reads. `None` means the committee cannot be
/// read yet (the executor has not reached the block that committed it) — a
/// transient state, never a verdict about the artifact.
pub type CommitteeSource = Arc<dyn Fn(u64) -> Option<EpochCommittee> + Send + Sync>;

/// Why an artifact does not verify against `committee[epoch]`.
///
/// Every arm except [`Self::CommitteeUnreadable`] is a property of the ARTIFACT,
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
    /// The committee could not be read at all. NOT a fault of the artifact.
    #[error("committee[{0}] is not readable yet")]
    CommitteeUnreadable(u64),
    /// The snapshot does not form a committee (duplicate peer or BLS key).
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
/// This is the standalone check the whole design rests on. Its inputs are the
/// artifact, the committee read from the staking contract, and the chain id —
/// no ceremony state, no share, no local scheme registry, no block.
///
/// Three things are checked and all three are load-bearing:
/// 1. the committee, the proposal and the certificate name ONE epoch, so a
///    genuine artifact for `E` cannot be replayed as one for `E'`;
/// 2. the certificate names THIS proposal's digest, so a valid certificate
///    cannot be re-paired with a substituted body;
/// 3. the certificate carries a `committee[epoch]` quorum under the AGREEMENT
///    namespace ([`dkg_namespace`]) — not the chain namespace, which is a
///    distinct and prefix-free tag precisely so the two planes' signatures can
///    never be read as each other's.
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
    // `beacon: None` and `cert_seed_pin: None`: the agreement instance signs with
    // no beacon part at all (it exists to AGREE the key), so its certificate is a
    // plain multisig quorum and `verify_certificate` returns on the vote arm.
    let verifier = build_verifier(&namespace, committee.bimap.clone(), None, None);
    if !certificate.verify(rng, &verifier, &Sequential) {
        return Err(ArtifactError::Certificate(committee.epoch));
    }
    Ok(())
}

/// [`verify_artifact`] straight off a staking read.
///
/// The snapshot is exactly what `RethStakingStateReader::epoch_committee_snapshot`
/// returns, so this is the whole path from "I can call the contract" to "this key
/// is the one `committee[E]` agreed".
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
/// The unreadable case gets its OWN error arm rather than folding into a generic
/// failure, because the two demand opposite responses: an artifact that cannot be
/// checked is dropped while the peer keeps its standing, whereas one that fails
/// the check is a proven forgery and costs the peer.
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
pub(crate) fn encode_artifact(artifact: &AgreedArtifact) -> Vec<u8> {
    artifact.encode().to_vec()
}

/// Decode an artifact under the network-wide caps.
pub fn decode_artifact(bytes: &[u8]) -> Result<AgreedArtifact, ArtifactError> {
    if bytes.len() > MAX_ARTIFACT_SIZE {
        return Err(ArtifactError::TooLarge(bytes.len()));
    }
    // The signer bitmap is decoded with a BOUNDED cap: the bytes come from an
    // untrusted peer and the unbounded decoder allocates eagerly from a tiny
    // length prefix (the same guard `plane_upstream::decode_frontier` applies).
    let cap = MAX_COMMITTEE_SIZE as usize;
    Ok(<(crate::beacon::dkg_agree::DkgProposal, Cert)>::decode_cfg(
        bytes,
        &((), cap),
    )?)
}

/// What a peer answers when asked for `committee[epoch]`'s artifact.
///
/// Two states, and both are DELIVERED. See the module doc for why a producer that
/// lacks the artifact answers instead of dropping the responder, and why there is
/// no third state.
#[derive(Clone, Debug)]
pub(crate) enum ArtifactResponse {
    /// The peer holds the artifact for the requested epoch.
    Have(Box<AgreedArtifact>),
    /// The peer does not hold it yet. The normal answer for most of epoch `E`,
    /// and never a statement about whether it will ever exist.
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

/// The per-epoch artifact store: a RAM map every reader sees synchronously, and
/// an optional durable mirror behind it.
///
/// # Why the durable write may lag the RAM write
///
/// The same argument [`crate::beacon::key_journal`] makes: the RAM write is what
/// in-process readers (the producer serving a peer, the write-back adopting the
/// pinned set) see and must stay synchronous, while durability is only ever read
/// by a LATER process after a restart, so it is free to lag. Records go out over
/// a non-blocking unbounded channel and every disk touch happens on the writer.
///
/// # Retention: none
///
/// Deliberate, and it is load-bearing rather than laziness. An artifact is the
/// only source of `PK_epoch` for a node that never ran the ceremony, and on a
/// committee that has been stable for a long time the entry worth having is the
/// OLDEST one — so any window measured in epochs drops the valuable record first.
/// It is also what makes the seam's two states honest: with no eviction, "not
/// yet" is never a lie, and there is nothing that could produce a truthful
/// "never". The arithmetic that makes "just keep them" the right answer: one
/// record per COMMITTEE CHANGE, on the order of a few KiB, so a chain that
/// changed committee every epoch at day-long epochs writes single-digit MB a year.
#[derive(Clone, Default)]
pub struct ArtifactStore {
    ram: Arc<RwLock<BTreeMap<u64, Arc<AgreedArtifact>>>>,
    durable: Option<tokio::sync::mpsc::UnboundedSender<(u64, Vec<u8>)>>,
}

impl ArtifactStore {
    /// A RAM-only store — what tests and any in-process run get.
    pub fn new() -> Self {
        Self::default()
    }

    fn with_persistence(
        rehydrated: Vec<(u64, AgreedArtifact)>,
        durable: tokio::sync::mpsc::UnboundedSender<(u64, Vec<u8>)>,
    ) -> Self {
        let ram = rehydrated
            .into_iter()
            .map(|(epoch, artifact)| (epoch, Arc::new(artifact)))
            .collect();
        Self {
            ram: Arc::new(RwLock::new(ram)),
            durable: Some(durable),
        }
    }

    /// Record `artifact` as the agreement's output for `epoch`.
    ///
    /// Returns `false` when an artifact for that epoch is already held, and keeps
    /// the one it has. FIRST-WINS is correct rather than convenient: the
    /// agreement instance certifies exactly one value per target epoch (a
    /// certified value bars every other one at `propose` and `verify`), so a
    /// second artifact for one epoch is either the identical bytes or evidence of
    /// something this store cannot adjudicate — and overwriting would let a
    /// fetched artifact displace the one this node itself agreed.
    pub fn insert(&self, epoch: u64, artifact: AgreedArtifact) -> bool {
        let encoded = {
            let mut ram = self.lock_mut();
            if ram.contains_key(&epoch) {
                return false;
            }
            let encoded = encode_artifact(&artifact);
            ram.insert(epoch, Arc::new(artifact));
            encoded
        };
        if let Some(durable) = &self.durable {
            // Unbounded and non-blocking: a closed writer means the durable half
            // is gone for the rest of the process, which costs a re-fetch after
            // the next restart and nothing now.
            if durable.send((epoch, encoded)).is_err() {
                warn!(
                    epoch,
                    "artifact store: the durable writer is gone; this epoch stays RAM-only"
                );
            }
        }
        true
    }

    /// The artifact for `epoch`, if this node holds one.
    pub fn get(&self, epoch: u64) -> Option<Arc<AgreedArtifact>> {
        self.lock().get(&epoch).cloned()
    }

    /// Whether this node can answer `Have` for `epoch`.
    pub fn has(&self, epoch: u64) -> bool {
        self.lock().contains_key(&epoch)
    }

    /// Every epoch this store can serve, ascending.
    pub fn epochs(&self) -> Vec<u64> {
        self.lock().keys().copied().collect()
    }

    fn lock(&self) -> std::sync::RwLockReadGuard<'_, BTreeMap<u64, Arc<AgreedArtifact>>> {
        // A poisoned lock means a reader panicked mid-read; the map itself is a
        // plain BTreeMap and cannot be left half-written, and taking the node down
        // over it would forfeit the very key this store exists to serve.
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

/// The stored artifacts a restarting node has to push back through the write-back,
/// and the reason that push has to exist at all.
///
/// The `DkgActor` adopts a pinned dealer-log set from exactly ONE place: the
/// artifacts channel a LIVE agreement instance feeds. Every other reader of this
/// store — the two `PK_epoch` key rungs, the pull seam's serve path — reads it to
/// answer a key question, and none of them reaches
/// [`crate::beacon::actor::DkgActor::on_artifact`]. So a member of
/// `committee[E+1]` that restarts after adopting an artifact but before
/// `finalize_over_pinned` completes comes back with an empty pinned set and waits
/// for a re-agreement no peer will run: every peer's launcher already holds `E+1`
/// in its `started` set. The artifact is on this node's own disk the whole time.
/// Replaying it is what makes the module's opening claim — recovering an artifact
/// after a restart is a read, not a re-agreement — true for the actor too.
///
/// SELECTED, never replayed wholesale: the store has no eviction policy, so most
/// of what it holds is history, and re-adopting a settled epoch would take a
/// ceremony-retention hold nothing releases until the next height tick. An
/// artifact is worth re-adopting only where BOTH hold:
///
/// - this node has no share for the epoch — a held share makes the adoption an
///   immediate no-op (`on_artifact` returns on exactly that check), and the
///   common case after a clean finalize is share-and-journal-both-present;
/// - the epoch's ceremony journal is still on disk — without it `maybe_start`
///   has nothing to resume, so there is no ceremony for the pinned set to be
///   finalized over.
///
/// The journal is window-scoped scratch that `reconcile_journals` prunes, so the
/// selection is a handful of records however old the store gets.
pub fn restart_replay(
    store: &ArtifactStore,
    share_dir: &Path,
    held_shares: &BTreeSet<u64>,
) -> Vec<AgreedArtifact> {
    let journaled: BTreeSet<u64> = share_state::journal_epochs(share_dir).into_iter().collect();
    store
        .epochs()
        .into_iter()
        .filter(|epoch| !held_shares.contains(epoch) && journaled.contains(epoch))
        .filter_map(|epoch| store.get(epoch).map(|a| (*a).clone()))
        .collect()
}

/// Failures of the durable artifact store.
#[derive(Debug, thiserror::Error)]
pub(crate) enum StoreError {
    #[error("artifact store: {0}")]
    Store(#[from] MetadataError),
}

/// The durable half. Exactly ONE instance per process: a second handle over the
/// same partition is a dual-writer.
///
/// [`Metadata`] rather than an `Ordinal`: an artifact is variable-length (a
/// `Ordinal` record has to be fixed-size), the collection is small and sparse,
/// and `Metadata` commits a batch atomically through its two-blob discipline —
/// which matters here because a half-written artifact is a key nobody can verify.
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

    /// Commit every staged record atomically.
    pub async fn sync(&mut self) -> Result<(), StoreError> {
        self.store.sync().await?;
        Ok(())
    }

    /// Every retained artifact, for the startup refill.
    ///
    /// A record that no longer decodes is SKIPPED with a warn rather than
    /// failing the load: one unreadable epoch costs a peer fetch, while refusing
    /// to start costs the node.
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
/// An EMPTY partition means "no journal", which is what tests and any in-process
/// run get. `writer_context` MUST be a SIBLING of `journal_context`, never a
/// clone: the deterministic runtime panics on a duplicate metric registered under
/// the same label, and both halves register store metrics.
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
pub(crate) fn spawn_writer<E>(
    context: E,
    mut journal: ArtifactJournal<E>,
    mut rx: UnboundedReceiver<(u64, Vec<u8>)>,
) -> Handle<()>
where
    E: Storage + Clock + Metrics + Spawner + Clone + Send + 'static,
{
    context.spawn(move |_| async move {
        while let Some((epoch, bytes)) = rx.recv().await {
            journal.append(epoch, bytes);
            // One record per committee change, so there is no batch worth
            // waiting for and every artifact is durable the moment it is known.
            if let Err(e) = journal.sync().await {
                metrics::counter!("dpos_artifact_store_sync_failed_total").increment(1);
                warn!(
                    epoch,
                    ?e,
                    "artifact store: sync failed; this epoch is lost on a hard kill and must be \
                     re-fetched from a peer"
                );
                continue;
            }
            metrics::counter!("dpos_artifact_store_appended_total").increment(1);
        }
    })
}

/// Per-epoch correlation map: a delivered answer is fanned to every waiting pull.
type Waiters = Arc<Mutex<HashMap<u64, Vec<oneshot::Sender<PullAnswer>>>>>;

/// What a peer answered one pull.
///
/// The seam's two states, and nothing else. Exhaustion is not an arm here — it is
/// the `None` [`ArtifactPull::pull`] returns, exactly as the cert-follow RPC seam
/// surfaces an exhausted upstream walk.
#[derive(Clone, Debug)]
pub enum PullAnswer {
    /// A peer served the artifact and it verified against `committee[epoch]`.
    Have(Arc<AgreedArtifact>),
    /// A peer answered honestly that its plane has not converged for this epoch.
    NotYet,
}

/// The serve-and-consume half of the seam, held by the resolver handler.
///
/// Cloned into the resolver engine for both roles, like
/// [`LogHandler`](crate::beacon::log_resolver::LogHandler).
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
    /// instance sends its artifact on, and the one [`restart_replay`] is pushed
    /// back through. It is a REQUIRED constructor argument rather than an optional
    /// one because a bridge wired without it is precisely the defect this seam had:
    /// a pulled artifact that answers every key question and never reaches
    /// [`crate::beacon::actor::DkgActor::on_artifact`].
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

    /// The store this bridge serves from.
    pub fn store(&self) -> &ArtifactStore {
        &self.store
    }

    /// Serve `epoch`. ALWAYS an answer, never a dropped responder.
    ///
    /// Dropping it would make the requester's resolver read "no data" and retry
    /// another peer forever with nothing surfaced above the transport — the shape
    /// this seam exists to avoid.
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
    /// resolver `excluded` set has no removal path — ONLY for proven
    /// misbehaviour: bytes that do not decode, an answer about a different epoch,
    /// or an artifact whose certificate fails against `committee[epoch]`. Every
    /// other outcome, `NotYet` included and an unreadable local committee
    /// included, is an honest delivery and returns `true`.
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
                    // This node cannot check the artifact yet, so it neither
                    // stores it nor punishes the peer that sent it. The pull sees
                    // no answer and exhausts, which is the honest outcome.
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
                let digest = artifact.0.digest();
                let stored = self.store.insert(epoch, artifact);
                // Read back rather than re-wrapping: a first-wins store may
                // already hold this node's own agreed artifact, and every waiter
                // must be woken with the value the store will actually serve.
                if let Some(held) = self.store.get(epoch) {
                    if held.0.digest() != digest {
                        // Two artifacts for one epoch that BOTH carry a
                        // committee quorum. Not the peer's fault and not
                        // adjudicable here — the instance bars a second value
                        // within itself, so this can only be two instances for
                        // one target — but it is a divergence worth saying out
                        // loud, because the two halves of the network would
                        // otherwise pin different keys in silence.
                        warn!(
                            epoch,
                            held = %held.0.digest(),
                            served = %digest,
                            "artifact seam: a peer served a DIFFERENT quorum-certified artifact \
                             for an epoch this node already holds one for; keeping the held one"
                        );
                    }
                    if stored {
                        self.adopt(epoch, &held);
                    }
                    self.wake(epoch, PullAnswer::Have(held));
                }
                true
            }
        }
    }

    /// Hand a newly-pulled artifact to the agreement write-back, which is the only
    /// route from this seam to the `DkgActor`.
    ///
    /// Without it a pulled artifact answers the `PK_epoch` rungs and serves peers
    /// while the node stays permanently SHARELESS for the epoch: `on_artifact` is
    /// reached from the write-back alone, so nothing adopts the pinned set and
    /// nothing finalizes over it until `drive_recompute` heals — after the chain
    /// has already entered the epoch this node was meant to sign in. The reachable
    /// shape is a member of `committee[E+1]` whose instance died mid-agreement:
    /// its peers decided without it and their launchers hold `E+1` in `started`,
    /// so no re-agreement is coming and the pull is its only source.
    ///
    /// Sent ONLY on a first insert. A repeat delivery for an epoch the store
    /// already holds has nothing new to adopt, and re-adopting a settled epoch
    /// takes a ceremony-retention hold that nothing releases until the next height
    /// tick — the same reason [`restart_replay`] selects rather than replays
    /// wholesale. That bound also makes the channel's depth a non-issue: at most
    /// one send per target epoch, into a loop that only forwards.
    ///
    /// A refused send is logged, never a `false` from `deliver`: the peer served an
    /// artifact that verified, and this node's own write-back being gone or backed
    /// up is not its misbehaviour.
    fn adopt(&self, epoch: u64, artifact: &AgreedArtifact) {
        match self.adopt_tx.try_send(artifact.clone()) {
            Ok(()) => debug!(
                epoch,
                "artifact seam: handing a pulled artifact to the agreement write-back"
            ),
            Err(TrySendError::Full(_)) => warn!(
                epoch,
                "artifact seam: the agreement write-back is backed up; this epoch's pinned set \
                 was not adopted and waits for the recompute heal"
            ),
            Err(TrySendError::Closed(_)) => warn!(
                epoch,
                "artifact seam: the agreement write-back is gone; this epoch's pinned set was \
                 not adopted"
            ),
        }
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
    /// Earliest time a pull for each epoch may touch the network again.
    next_allowed: Arc<Mutex<HashMap<u64, SystemTime>>>,
}

impl<E: Clock> ArtifactPull<E> {
    pub fn new(context: E, bridge: ArtifactBridge) -> Self {
        Self {
            context,
            bridge,
            next_allowed: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// One bounded pull for `epoch`.
    ///
    /// - `Some(PullAnswer::Have)` — a peer served it and it verified;
    /// - `Some(PullAnswer::NotYet)` — a peer answered honestly that it has none;
    /// - `None` — the walk was exhausted: nobody answered inside [`PULL_TIMEOUT`].
    ///
    /// A locally-held artifact short-circuits without touching the network. The
    /// fetch is issued no sooner than [`PULL_MIN_INTERVAL`] after the previous one
    /// for this epoch — the caller-side half of the per-peer rate bound, which
    /// matters because an inbound over-quota sleeps a peer's whole connection.
    /// On the way out an unanswered fetch is CANCELLED, so the resolver stops
    /// probing peers for a key nobody is waiting on any more.
    pub async fn pull<R>(&self, resolver: &mut R, epoch: u64) -> Option<PullAnswer>
    where
        R: commonware_resolver::Resolver<Key = crate::beacon::log_resolver::BeaconFetchKey>,
    {
        if let Some(held) = self.bridge.store.get(epoch) {
            return Some(PullAnswer::Have(held));
        }
        self.throttle(epoch).await;

        let (tx, rx) = oneshot::channel();
        self.bridge
            .waiters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(epoch)
            .or_default()
            .push(tx);
        let key = crate::beacon::log_resolver::BeaconFetchKey::Artifact { epoch };
        resolver.fetch(key.clone()).await;

        let answer = tokio::select! {
            answer = rx => answer.ok(),
            () = self.context.sleep(PULL_TIMEOUT) => None,
        };
        if answer.is_none() {
            self.bridge.metrics.dkg_artifact_pull_exhausted.inc();
            debug!(
                epoch,
                "artifact seam: pull exhausted — no peer answered inside the window"
            );
        }
        // The waiter is spent either way; the CANCEL is only for the unanswered
        // case, because a delivered answer already completed the fetch and the
        // resolver has nothing left to probe for.
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
            let mut next = self
                .next_allowed
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let at = next.get(&epoch).copied().unwrap_or(now);
            let wait = at.duration_since(now).unwrap_or_default();
            next.insert(epoch, at.max(now) + PULL_MIN_INTERVAL);
            wait
        };
        if !wait.is_zero() {
            self.context.sleep(wait).await;
        }
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
    use commonware_utils::{ordered::BiMap, ordered::Set, vec::NonEmptyVec, N3f1, TryCollect as _};
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
                    let signer = build_signer(&ns, bimap.clone(), kp, None).expect("member");
                    Finalize::sign(&signer, proposal.clone()).expect("sign")
                })
                .collect();
            Finalization::from_finalizes(
                &build_verifier(&ns, bimap, None, None),
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

    /// ITEM 1. The artifact verifies against `committee[E+1]` read from the
    /// staking contract and NOTHING else: no ceremony, no share, no journal, no
    /// block, no locally-registered scheme. The snapshot below is byte-for-byte
    /// the value `epoch_committee_snapshot` hands back.
    #[test]
    fn an_artifact_verifies_from_the_staking_committee_alone() {
        let c = committee(1);
        let artifact = artifact(&c, TARGET);
        let mut rng = StdRng::seed_from_u64(99);
        verify_artifact_from_snapshot(&mut rng, CHAIN_ID, &c.snapshot(TARGET), &artifact)
            .expect("a quorum-certified artifact verifies against the committee alone");

        // A DIFFERENT committee must not certify it — the check is the quorum,
        // not the shape.
        let other = committee(2);
        let err =
            verify_artifact_from_snapshot(&mut rng, CHAIN_ID, &other.snapshot(TARGET), &artifact)
                .expect_err("a foreign committee must not verify it");
        assert!(matches!(err, ArtifactError::Certificate(TARGET)), "{err:?}");

        // The chain namespace must not verify it either: the agreement plane
        // signs under its own, prefix-free tag.
        let mut chain_ns_verifier_failed = false;
        let bimap = c.bimap();
        let verifier = build_verifier(&fluent_namespace(CHAIN_ID), bimap, None, None);
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

    /// ITEM 2. The store keeps the artifact per epoch, first-wins, and a restart
    /// finds it again — which is the residual the agreement supervisor names
    /// today (`dkg_agree_body_lost_total`: nothing re-broadcasts a decided
    /// proposal, so a restart without this store loses the artifact outright).
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
            assert!(store.insert(TARGET, mine.clone()));
            assert!(
                !store.insert(TARGET, artifact(&c, TARGET)),
                "a second artifact for one epoch must not displace the first"
            );
            assert!(store.insert(TARGET + 2, artifact(&c, TARGET + 2)));
            assert_eq!(store.epochs(), vec![TARGET, TARGET + 2]);
            assert_eq!(*store.get(TARGET).expect("held"), mine);
            assert!(store.get(TARGET + 1).is_none());

            // Let the writer drain, then stop it — the restart below opens the
            // same partition with a fresh handle.
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

    /// A bridge plus the write-back end it adopts into. The receiver is returned
    /// rather than dropped because dropping it closes the channel, which would turn
    /// every adopt into a refusal.
    fn bridge_and_adopt(
        c: &Committee,
        epoch: u64,
    ) -> (ArtifactBridge, tokio::sync::mpsc::Receiver<AgreedArtifact>) {
        let snapshot = c.snapshot(epoch);
        let source: CommitteeSource = Arc::new(move |e| {
            (e == snapshot.epoch)
                .then(|| epoch_committee_from_snapshot(&snapshot).expect("committee"))
        });
        let (adopt_tx, adopt_rx) = tokio::sync::mpsc::channel(16);
        (
            ArtifactBridge::new(
                CHAIN_ID,
                ArtifactStore::new(),
                source,
                adopt_tx,
                BeaconMetrics::default(),
            ),
            adopt_rx,
        )
    }

    /// ITEMS 3 + 4. A producer that lacks the artifact ANSWERS `NotYet` instead
    /// of dropping the responder, and the consumer takes both states as honest
    /// deliveries — so `deliver` never returns false for an honest peer and the
    /// un-removable `excluded`-peer defect cannot fire on this instance. `false`
    /// is reserved for proven misbehaviour.
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
        assert!(serving.store.insert(TARGET, mine.clone()));
        let answer = serving.produce(TARGET);
        assert!(
            fetching.deliver(TARGET, answer.as_ref()),
            "a valid artifact must be delivered"
        );
        assert_eq!(*fetching.store.get(TARGET).expect("stored"), mine);

        // A forged artifact — certified by a committee that is not this epoch's
        // — is proven misbehaviour, and only THAT returns false.
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

        // An unreadable committee is a fact about THIS node, never a verdict on
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
    /// The store answers the `PK_epoch` rungs and serves peers, but nothing that
    /// reads it reaches `DkgActor::on_artifact` — so without this push a node whose
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
        assert!(serving.store.insert(TARGET, mine.clone()));
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
        async fn fetch_targeted(&mut self, _: Self::Key, _: NonEmptyVec<Self::PublicKey>) {}
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

    /// ITEM 3. The caller sees all three outcomes and is never left in silent
    /// retry: `Have`, `NotYet`, and — when nobody answers — an exhausted walk as
    /// `None`, which is the cert-follow RPC seam's semantics rather than
    /// `Consumer::failed`'s silence.
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
            };
            let pull = ArtifactPull::new(context.clone(), fetching.clone());

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
            assert!(serving.store.insert(TARGET, mine.clone()));
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

            // Nobody answers at all: the walk exhausts and the caller is TOLD,
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

    /// ITEM 5. Repeated pulls for one epoch are rate-bounded at the caller, which
    /// is what keeps this seam from tripping a peer's inbound quota — an
    /// over-quota sleeps the WHOLE connection to that peer, not just this channel.
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
            };
            let pull = ArtifactPull::new(context.clone(), fetching);

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
        });
    }
}
