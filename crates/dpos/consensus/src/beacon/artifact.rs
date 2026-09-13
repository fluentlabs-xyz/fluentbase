//! The agreement artifact: what the plane leaves behind, where it is kept, and
//! how a node that never ran the ceremony gets it.
//!
//! # Why this exists at all
//!
//! No block carries the epoch key any more, so a node that never ran the
//! ceremony — a cold-started member, a validator that sat outside
//! `committee[E]`, the STF verifier — holds no dealer logs, cannot recompute
//! `PK_E`, and this artifact is the only form the key still reaches it in. So it
//! must be checkable by someone who has nothing but the staking contract:
//! [`verify_artifact`] takes the artifact and `committee[epoch]` and needs no
//! local ceremony state, no share, no journal and no block.
//!
//! # Who this seam does NOT reach — and who gets a second one instead
//!
//! The check is that cheap, but the DELIVERY here rides
//! `BEACON_RESOLVER_CHANNEL`, so THIS seam reaches exactly the nodes that are on
//! the consensus plane. A `--cert-follow` follower is not one: it mints an
//! ephemeral p2p identity, configures no bootstrappers, listens on an ephemeral
//! loopback port and never tracks a peer set (`node/src/cert_follow/mod.rs`), so
//! it has no peer to ask here and no peer asks it.
//!
//! That is still true, and it is no longer the end of the story. Nothing this
//! module lacks was ever what blocked a follower: [`verify_artifact`] needs only
//! a staking read it makes on every cert. What was missing was a delivery route
//! over the ONE relationship a follower has — its cert upstream — and that route
//! now exists as `CertUpstream::get_epoch_artifact` over the `consensus`
//! namespace's `getEpochArtifact` (FLU-1167). It is a SECOND seam, deliberately
//! separate from this one, built in [`crate::beacon::follower`]: same artifact,
//! same [`verify_artifact_for_epoch`] check against `committee[minted_at]`, a
//! different transport. So a follower's cert-inlet leaves vote-only admission
//! when the epoch's artifact arrives, not when the process exits. See
//! Both transports speak ONE acquisition ([`AcquireArtifact`]), and what a caller
//! may spend on one is stated there rather than in a ladder of rungs — the tiered
//! key store that ladder walked is gone (П-3).
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
use commonware_cryptography::bls12381::primitives::{sharing::Sharing, variant::MinSig};
use commonware_parallel::Sequential;
use commonware_runtime::{Clock, Handle, Metrics, Spawner, Storage};
use commonware_storage::metadata::{Config as MetadataConfig, Error as MetadataError, Metadata};
use commonware_utils::sequence::U64;
use fluentbase_bls::{
    beacon::dkg_namespace, beacon::GroupPublic, fluent_namespace, scheme::build_verifier,
    EpochCommittee, Scheme as BlsScheme,
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
    beacon::{
        dkg_agree::AgreedArtifact, metrics::BeaconMetrics, outcome::group_public_key, share_state,
    },
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
///
/// **It may only ever GROW.** The same value is the durable journal's decode
/// bound ([`ArtifactJournal::init`]), and that decode is infallible-by-`expect`:
/// a record already on disk that no longer fits panics the node at STARTUP —
/// a node that was healthy when it shut down, with a codec message that names
/// neither this constant nor the epoch whose artifact it refused. Raising it
/// only widens what a peer may send; lowering it retroactively invalidates
/// history nothing can re-fetch until the node is up.
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
    /// The snapshot does not form a committee (duplicate peer or BLS key). Only
    /// [`verify_artifact_from_snapshot`] reaches it, and that entry point is
    /// itself test-only.
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
    // `oracle: None`: the agreement instance carries no beacon half at all (it
    // exists to AGREE the key), so its certificate is a plain multisig quorum and
    // `verify_certificate` returns on the vote arm.
    let verifier = build_verifier(&namespace, committee.bimap.clone(), committee.epoch, None);
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
/// The same argument [`crate::beacon::seed_journal`] makes, and the one the deleted
/// key journal made: the RAM write is what in-process readers (the producer serving
/// a peer, the write-back adopting the pinned set) see and must stay synchronous,
/// while durability is only ever read by a LATER process after a restart, so it is
/// free to lag. Records go out over
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
    /// One notifier per [`Self::subscribe`] caller, fired by every accepted
    /// [`Self::insert`].
    ///
    /// THE EDGE THAT USED TO BE `BeaconKeys::subscribe`, moved here with the fact
    /// (П-3): "a key this node could not resolve became resolvable" now means
    /// "an artifact landed", because the artifact is the only thing a key comes
    /// from. Its two consumers are the quarantined-σ promoter and the
    /// `KeyAvailable` event bridge.
    ///
    /// PER CONSUMER, never a shared handle, and the reason is the one
    /// `beacon::keys`' module header gave for the store it replaces: `notify_one`
    /// wakes exactly ONE waiter, so two consumers sharing a handle silently
    /// swallow each other's wake-ups — and each loss here is a silent degrade (an
    /// epoch left vote-only, or a σ that never leaves quarantine).
    listeners: Arc<Mutex<Vec<Arc<tokio::sync::Notify>>>>,
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
            listeners: Arc::default(),
        }
    }

    /// A notifier of this consumer's OWN, fired by every accepted [`Self::insert`].
    /// See the field's doc for why it is never shared.
    pub fn subscribe(&self) -> Arc<tokio::sync::Notify> {
        let handle = Arc::new(tokio::sync::Notify::new());
        if let Ok(mut listeners) = self.listeners.lock() {
            listeners.push(handle.clone());
        }
        handle
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
                // Counted as well as logged: "accepted in RAM, never written" is
                // precisely the state §5.4 allows and an operator must be able to
                // see, and a `warn!` alone is not a signal anything scrapes. Same
                // arm the writer's own `sync` failure lands on
                // (`dpos_artifact_store_sync_failed_total`) — the value stands, the
                // durability does not.
                metrics::counter!("dpos_artifact_store_handoff_failed_total").increment(1);
                warn!(
                    epoch,
                    "artifact store: the durable writer is gone; this epoch stays RAM-only \
                     and must be re-fetched from a peer after a restart"
                );
            }
        }
        // Strictly AFTER the durable hand-off, and unconditionally for every
        // accepted insert: what a waiter does on the wake-up is re-read, which is
        // idempotent, so a conditional fire would have to reason about which
        // reader's answer this particular artifact can change.
        if let Ok(listeners) = self.listeners.lock() {
            for handle in listeners.iter() {
                handle.notify_one();
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

/// Read the FROZEN on-chain `dkgQual[epoch]` bit: did the committee CHANGE at
/// `epoch`, i.e. did its DKG re-mint the key. `None` = could not read yet, which
/// the caller must treat as undecided and never as "no re-mint".
///
/// The `committed` leg the raw staking read carries is NOT here, and that is Д-7
/// resolved rather than dropped: the module answers a record only for an epoch
/// whose committee it actually read, so "is it committed" is answered by having a
/// record at all (`committee::CommitteeReadsFacade::dkg_qual`, whose second leg is
/// unconditionally `true`). The `!(bit || committed) ⇒ None` guard this used to
/// carry had exactly one live meaning left — `false` — so keeping it would be a
/// second name for the `None` above.
pub(crate) type ChangedAt = Arc<dyn Fn(u64) -> Option<bool> + Send + Sync>;

/// `epoch → the epoch that MINTED the key in force at it`, memoised on disk.
///
/// # The rule
///
/// `minted_at(E) = last e in (BOOTSTRAP, E] with changed[e], else BOOTSTRAP`. The
/// bits are set deterministically by the contract at `commitEpochCommittee`
/// (`changed[e] = committee[e] != committee[e−1]`) and never mutated, so a
/// resolved answer is a function of frozen chain facts and cannot change later.
/// The deterministic bootstrap epoch mints unconditionally.
///
/// # Why it is DURABLE, which is the part the project did not say
///
/// The project keeps this memo ("правила memo сохраняются") and deletes the key
/// journal. Those two are only compatible if the memo survives a restart, and the
/// key journal's own doc says why: the walk reads the chain, and "the `dkgQual`
/// read is exactly what is NOT answerable while a restarted node's EL is still
/// catching up" — the gap that store existed to close. Worse than "slow": the
/// committee module answers a bit only inside
/// `[epoch(anchor) − SCHEME_RETENTION_EPOCHS, epoch(anchor) + lookahead]`
/// (`committee/mod.rs`), and folds everything below that window into the same
/// `None`, so a walk that must reach a mint older than the window never terminates
/// in an answer at all. A restarted node with an empty memo would therefore hold a
/// durable artifact it cannot ADDRESS.
///
/// Persisting the memo closes that for every epoch this node ever resolved AND can
/// still REACH, and the cost is a `(u64, u64)` per epoch against the whole key
/// journal. Two epochs it does not close, both stated rather than implied:
///
/// - a FRESH node (empty memo) whose mint is older than the window — see this
///   module's `open_mint_memo` doc;
/// - an epoch whose memo entry lies BELOW an unreadable bit: the walk answers
///   `None` at the first undecided bit (it must — recording a guess is
///   unrecoverable), so a node restarting more than `SCHEME_RETENTION_EPOCHS`
///   epochs behind cannot read PAST the gap to the entry underneath it. The answer
///   is on disk and unreachable until the bits above it become readable. Low
///   reachability rather than closed: the restarted node's committee anchor is
///   equally old, and `cold_start_jump` bounds the jump to two epochs.
///
/// # Two rules carried over verbatim, because breaking either is unrecoverable
///
/// - **`None` is never memoised.** An undecided bit means "retry", and recording it
///   would pin the bootstrap answer onto an epoch the chain has not committed yet
///   — a wrong mint is a wrong key for the epoch's whole life.
/// - **Write-once.** A resolved answer is frozen chain fact; a second, different
///   answer for one epoch cannot be honest, so the first stands and the
///   disagreement is loud.
#[derive(Clone)]
pub(crate) struct MintIndex {
    changed: ChangedAt,
    /// `epoch → minted_at`. Shared by clone: every reader of the same plane shares
    /// one, which is what makes the walk cost one step instead of `E − BOOTSTRAP`
    /// per certificate.
    memo: Arc<Mutex<BTreeMap<u64, u64>>>,
    /// Decided bits, so the walk pays one chain read per epoch per PROCESS rather
    /// than per call. Not persisted: it is derivable and the memo above is what has
    /// to survive.
    bits: Arc<Mutex<BTreeMap<u64, bool>>>,
    /// Durable sink for resolved answers. `None` ⇒ RAM-only (tests, and any config
    /// without a partition). Non-blocking by construction, like every other durable
    /// half in this module: readers see the RAM map, and durability is only ever
    /// read by a LATER process.
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
    /// cannot answer yet.
    ///
    /// FLAT `Option`, unlike the two-layer `Option<Option<u64>>` this replaces. The
    /// inner layer meant "`epoch` predates the beacon entirely", and it had one
    /// consumer beyond its own answer: `signer_scheme`'s `Signs` arm was safe only
    /// because that `Some(None)` made the material resolve to `None` below the
    /// bootstrap epoch. That arm STATES the gate itself now
    /// (`mandatory_at(epoch).then(|| material(..))`), so the layer has no second
    /// reader and collapsing it removes a distinction every caller was flattening
    /// anyway — "not now" and "never" both mean "no key here".
    pub(crate) fn minted_at(&self, epoch: u64) -> Option<u64> {
        if epoch < super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH {
            return None;
        }
        if let Some(hit) = self.memo.lock().ok().and_then(|m| m.get(&epoch).copied()) {
            return Some(hit);
        }
        let mut answer = None;
        for e in (super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH + 1..=epoch).rev() {
            // A memoised LOWER epoch answers this one too, by monotonicity: nothing
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
                // Undecided: abort WITHOUT recording anything.
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

    /// Write-once record plus the durable offer. A DIFFERING second answer is
    /// fork-grade — two mint epochs for one epoch cannot both be chain fact — so it
    /// is kept loud and the first answer stands.
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

/// `PK_epoch` and the public polynomial, read from their ONE owner (П-3).
///
/// Two facts and nothing else: the chain says WHICH epoch minted the key in force
/// at `epoch` ([`MintIndex`]), and that mint's quorum-certified artifact carries the
/// key ([`ArtifactStore`]). There is no store of keys any more — `BeaconKeys`, its
/// three provenance tiers, its journal and the ladder over them are gone, and with
/// them the class where "the key I rebuilt" and "the key the network attested" were
/// two possible values of one map entry.
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

    /// `PK_epoch` in force at `epoch`. SYNCHRONOUS and I/O-free: "not resolvable"
    /// is the answer a vote path acts on, never a reason to go fetch.
    pub(crate) fn key_at(&self, epoch: u64) -> Option<GroupPublic> {
        let minted_at = self.mints.minted_at(epoch)?;
        self.artifacts
            .get(minted_at)
            .map(|a| *group_public_key(&a.0.group_key))
    }

    /// `(minting epoch, public polynomial)` in force at `epoch` — what a partial is
    /// signed and verified against. The share that pairs with it is the
    /// `CeremonyStore`'s, keyed by the SAME minting epoch.
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

/// ONE bounded acquisition of a MINTING epoch's artifact — the single shape both
/// classes that need one speak (Д-11).
///
/// # Why this is a trait and was a closure
///
/// Journal 5.0's Д-11 kept `ArtifactFetch` a closure because it had exactly one
/// implementation and "a trait from one method with one implementor is another type
/// on the boundary with no second caller to justify it". The second caller appears
/// HERE: a validator that is not in `committee[E]` needs the same acquisition the
/// follower needed, over a different transport, and the CALLER must not be able to
/// tell them apart — that is what makes the non-member's route and the follower's
/// route one code path rather than two that agree today.
///
/// # The contract
///
/// One bounded attempt, throttled to at most one network round-trip per epoch per
/// [`PULL_MIN_INTERVAL`], answering whether the store now holds `minted_at`. An
/// implementation may only return `true` for an artifact it VERIFIED against
/// `committee[minted_at]` read from this node's own chain state — a lying transport
/// is caught by the implementation, never by the caller.
pub(crate) trait AcquireArtifact: Send + Sync {
    fn fetch(&self, minted_at: u64) -> futures::future::BoxFuture<'_, bool>;
}

/// The handle every consumer of [`AcquireArtifact`] holds.
pub(crate) type AcquireMint = Arc<dyn AcquireArtifact>;

/// The BYTES half of an acquisition: one transport's answer for a minting epoch.
///
/// `None` for every negative alike — no artifact, a peer/upstream too old to know
/// the method, a dead link. The three are one answer to the caller (stay unpinned,
/// ask again) and separating them would only invite someone to treat one as a
/// fault. Two implementations: the follower's cert-upstream RPC and, on a
/// validator, the `BEACON_RESOLVER_CHANNEL` pull.
pub(crate) type ArtifactBytes =
    Arc<dyn Fn(u64) -> futures::future::BoxFuture<'static, Option<Vec<u8>>> + Send + Sync>;

/// [`AcquireArtifact`] over a byte transport: throttle, fetch, decode, VERIFY
/// against `committee[minted_at]`, file, and hand the artifact to the write-back.
///
/// ONE body for every transport that delivers bytes, which is the half that must
/// not diverge: the verify is what makes a lying upstream and a lying peer the same
/// non-event, and a second copy of it is a second place for the check to be
/// weakened. The follower used to carry this inline; the non-member validator half
/// of PLAN row 5.1 is the second caller that makes it shared code rather than a
/// generalisation for its own sake.
pub(crate) struct TransportAcquire<E: Clock> {
    chain_id: u64,
    committees: CommitteeSource,
    bytes: ArtifactBytes,
    store: ArtifactStore,
    adopt: Option<tokio::sync::mpsc::Sender<AgreedArtifact>>,
    clock: E,
    /// Per-epoch budget, pruned as it expires — the same `PULL_MIN_INTERVAL` the
    /// plane's pull seam owes its peers. Without it a want that arrives on every
    /// certificate (~1/s) would be one round-trip a second per unresolved epoch.
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
            // Short-circuit on a local hit: an acquisition is never the way to read
            // what is already held.
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
                // Not the server's fault and not a verdict on the artifact: this
                // node has not reached the block that committed
                // `committee[minted_at]`. Drop it and re-ask later.
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
            let first = self.store.insert(minted_at, artifact.clone());
            self.metrics.follower_artifact_adopted.inc();
            if first {
                debug!(
                    epoch = minted_at,
                    group_public = %pk_prefix(&pk),
                    "beacon: PK_epoch obtained and verified against committee[epoch]"
                );
                // The write-back hop, where the caller wired one: a pulled artifact
                // must reach the actor's adoption rails exactly as an agreed one
                // does, or a member whose own instance died stays shareless for an
                // epoch it was elected to sign in (see this module's header).
                if let Some(adopt) = self.adopt.as_ref() {
                    let _ = adopt.try_send(artifact);
                }
            }
            true
        })
    }
}

/// First 8 serialized bytes of a group public key, hex — a stable, greppable value
/// fingerprint. Enough to byte-diff key VALUES across nodes from logs alone; the
/// full G2 hex is 192 chars of log noise.
///
/// It lived in `beacon::keys` until that module's store was deleted (П-3); it is a
/// formatter for the fact this module now owns.
pub(crate) fn pk_prefix(pk: &GroupPublic) -> String {
    let mut s = pk.to_string();
    s.truncate(16);
    s
}

/// ONE artifact builder for every test in this module tree that needs a
/// [`KeyIndex`] to answer, rather than one per test module.
///
/// The committee is a throwaway set whose only job is to make a real
/// `Finalization` constructible: nothing downstream of [`ArtifactStore::insert`]
/// re-verifies (verification happens BEFORE the insert on every production path —
/// `verify_artifact` at the bridge, the instance's own certificate on the plane),
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

/// The ONE test seam for "this node holds the epoch key", for every module that
/// needs a [`KeyIndex`] to answer.
///
/// A test states a MINT — an epoch plus the real `Output` its ceremony produced —
/// and the fixture does what production does with one: files the artifact that
/// carries it and records the chain's `changed` bit at that epoch. There is no way
/// to state a bare `GroupPublic`, and that is the point: the key is a projection of
/// the artifact now (П-3), so a fixture that could inject one without an artifact
/// would be testing a state the node cannot be in.
#[cfg(test)]
pub(crate) struct MintFixture {
    pub(crate) artifacts: ArtifactStore,
    bits: Arc<Mutex<BTreeSet<u64>>>,
    /// How many times the CHAIN was asked for an epoch's `changed` bit — the rung
    /// the ladder pays for, in production a staticcall. A test that ingests many
    /// certificates of one epoch asserts on this to pin that the walk is memoised
    /// and the chain is read ONCE, not once per certificate.
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

    /// Record that the chain says `epoch` re-minted, WITHOUT the artifact arriving.
    ///
    /// The two halves are separate because they happen at different times and the
    /// order is load-bearing: the contract writes the bit with the committee, an
    /// epoch before the epoch, while the artifact arrives when the agreement
    /// converges or a peer serves it. A fixture that set the bit LATE would be
    /// staging a chain that changed its own history — and [`MintIndex`]'s write-once
    /// memo correctly refuses to notice, so the test would fail for the wrong reason.
    pub(crate) fn changed(&self, epoch: u64) {
        self.bits
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(epoch);
    }

    /// The artifact for a mint the chain has already recorded ARRIVES.
    pub(crate) fn arrive(&self, minted_at: u64, group_key: crate::beacon::outcome::DkgOutcome) {
        self.artifacts
            .insert(minted_at, artifact_with_key(minted_at, group_key));
    }

    /// Both halves at once, for a fixture that only needs the end state.
    pub(crate) fn mint(&self, minted_at: u64, group_key: crate::beacon::outcome::DkgOutcome) {
        self.changed(minted_at);
        self.arrive(minted_at, group_key);
    }
}

/// A [`ChangedAt`] over a fixed set of CHANGE epochs — the frozen chain record a
/// test asserts against.
#[cfg(test)]
pub(crate) fn changed_at(bits: &[u64]) -> ChangedAt {
    let set: BTreeSet<u64> = bits.iter().copied().collect();
    Arc::new(move |e| Some(set.contains(&e)))
}

/// A [`KeyIndex`] over a store the CALLER holds, so a test can make the mint's
/// artifact ARRIVE mid-test — the one edge a key resolve turns on.
#[cfg(test)]
pub(crate) fn key_index_over(artifacts: ArtifactStore, bits: &[u64]) -> KeyIndex {
    KeyIndex::new(artifacts, MintIndex::new(changed_at(bits)))
}

/// Open the durable mint memo and join it to a [`MintIndex`] over `changed`, or
/// hand back a RAM-only index when no partition is configured.
///
/// # What this closes and what it does not
///
/// Rehydrating the memo means a restarted node needs NO chain read for any epoch it
/// already resolved, and one step for the next — which is the whole of what the
/// deleted key journal provided for this question, and strictly more: the journal
/// held `epoch → pk` for MINT epochs only, so it never answered a carry epoch at
/// all (W1 did, and W1 is gone).
///
/// It does NOT close a FRESH node whose datadir is empty and whose committee last
/// minted more than `SCHEME_RETENTION_EPOCHS` epochs ago: its walk must reach that
/// mint, every epoch below its own anchor's window folds to `None`, and no memo
/// entry exists to stop the walk. That node stays on vote-only admission for the
/// epoch — which is I4's residual and NOT a regression: the deleted ladder had the
/// identical walk and the deleted journal was empty for it too.
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
    // THE TORN OUTCOME, NAMED. A record of the wrong length is dropped, and a memo
    // that dropped everything behaves exactly like a memo that was empty — so
    // without this line the two are distinguishable only by a counter nobody was
    // told to look at. There is no state to enter: the walk re-resolves whatever it
    // can still read, which is the correct response to a partly lost memo, and
    // `None` was never memoised so nothing wrong can have been kept.
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
///
/// # What "the write is retried" means, exactly (§5.4)
///
/// `Metadata::put` is an in-memory insert and `sync` commits the WHOLE map, so a
/// failed `sync` loses nothing it was given: the record stays staged and the next
/// `sync` re-attempts it. That is the retry §5.4 asks for, and its trigger is the
/// next record on `rx` — nothing else calls `sync` here. Artifacts arrive once per
/// committee change, so the retry is real but can be far away, and the gap is
/// bounded by the loss being LOUD rather than by a timer (a timer is the thing this
/// plane does not have and does not want). PLAN row 5.4 is where the beacon's clock
/// becomes a `watch` every task can subscribe to; a clock-driven re-`sync` belongs
/// there and not in a second consumer of today's single-consumer height channel.
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
                    "artifact store: sync failed; the record stays staged and the next artifact's \
                     sync re-attempts it — until then a hard kill loses this epoch and it must be \
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
            // An epoch whose slot has already passed is indistinguishable from an
            // absent one, so the map keeps only the epochs it is still holding
            // back — otherwise it grows one entry per epoch ever pulled, forever.
            next.retain(|_, at| *at > now);
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

    /// PERSIST ERROR OF THE ARTIFACT (project §5.4; PLAN row 5.1 names this test).
    /// A failing durable half must NOT gate acceptance: the artifact stands in RAM,
    /// the epoch stays VERIFIABLE off it, and the failure is observable.
    ///
    /// Why RAM acceptance is the rule rather than a convenience: refusing an
    /// artifact this node cannot journal would turn a local disk fault into "no
    /// `PK_E` ⇒ every σ of the epoch `Pending` ⇒ execution parks" — a node that
    /// cannot WRITE would stop being able to VERIFY, which is the one failure mode
    /// §5.4 refuses to buy.
    ///
    /// The injected class is the durable writer being GONE (its task died, or the
    /// drain closed). It is the class this runtime can inject — commonware's
    /// deterministic `Storage` is in-memory and has no fault knob, so a real
    /// `Metadata::sync` error is not reachable here — and it is the SAME arm: value
    /// kept, durability lost, operator told. The `sync` half is
    /// `dpos_artifact_store_sync_failed_total` in this module's writer.
    ///
    /// Falsifier: `insert` answering `false`, or the key going unresolvable, when
    /// the durable half is dead; the counter staying at 0 (a silent loss of
    /// durability, which is the state an operator must never have to infer).
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
                store.insert(TARGET, mine.clone()),
                "the artifact is ACCEPTED — the durable half does not gate it"
            );
            assert_eq!(
                *store.get(TARGET).expect("held in RAM"),
                mine,
                "and it is the artifact that was offered, not a placeholder"
            );
            // And the node keeps verifying: the key of the epoch resolves off the
            // RAM half alone, which is the whole point of accepting it.
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

    /// (П-3, RECREATED — `resolve.rs::a_stable_committees_attested_mint_outlives_the_retention_window`
    /// went with its file) A STABLE committee's mint outlives the retention window,
    /// and now the restart with it.
    ///
    /// WHY THE CLAIM SURVIVED ITS OWNER. On a committee that never changes the mint
    /// is the deterministic bootstrap epoch FOREVER, while every pruner in the plane
    /// runs frontier-relative — so an epoch-measured window over the wrong store
    /// deletes the one object every later epoch depends on and nothing re-inserts
    /// it. That is what the old test guarded, and the shape of the danger did not
    /// change when the owner did.
    ///
    /// WHAT THE OLD ASSERT PROVED, ITEM BY ITEM, AND WHERE EACH ITEM IS NOW:
    ///
    /// 1. *A local publication (`KeySource::LocalDkg`) and a carry memo
    ///    (`KeySource::Carried`) far below the frontier ARE pruned* — the window
    ///    still bounds growth. GONE AS A CLASS, not moved: W1, W3, `Carried` and the
    ///    three-tier `BeaconKeys` are deleted, so there is no derived tier left to
    ///    prune. What is still prunable and still pruned is the σ half
    ///    (`FollowerRandomness::retain_from`) and this node's own shares
    ///    (`actor::ceremony_retain_floor`).
    /// 2. *The attested tier at the mint epoch is EXEMPT from the window, and
    ///    survives a frontier ten windows above it.* CARRIED HERE, and it is the
    ///    heart of the test: the owner of the key is the artifact store, which has
    ///    no prune surface at all, plus the mint memo that ADDRESSES it.
    /// 3. *Therefore the share gate keeps demoting instead of publishing a divergent
    ///    key at a far-frontier stable epoch.* GONE AS A CLASS: the divergence gate
    ///    compared "the key I rebuilt" against "the key the network attested", and
    ///    [`KeyIndex`] has one value — `WithheldReason::KeyDivergence` and its
    ///    counter are deleted rather than left unreachable.
    /// 4. *`ceremony_retain_floor` keeps the single old mint of a stable committee*
    ///    (the old test read this through the resolver). Already pinned directly, as
    ///    a unit, by `actor::retain_floor_tests::single_old_mint_on_stable_committee_is_retained`
    ///    — not duplicated here.
    ///
    /// WHAT THIS TEST ADDS that the old one could not: the RESTART. The old owner
    /// rebuilt its answer by walking the chain's `dkgQual` bits, and that walk cannot
    /// terminate for this fixture after a restart — the committee module folds every
    /// epoch below `epoch(anchor) − SCHEME_RETENTION_EPOCHS` into the same `None`
    /// (`committee/facade.rs`), which the second half models by a `changed` closure
    /// that answers `None` for every epoch. The durable memo is what still addresses
    /// the artifact, and the third half is the negative control that says so: the
    /// same blind chain with a FRESH memo answers nothing at all, which is I4's
    /// residual stated as an assertion rather than as prose.
    ///
    /// Falsifier: any epoch above the mint resolving to a different mint (the walk
    /// or the memo is wrong); the artifact store losing the entry over a frontier ten
    /// windows above it; `key_at` at the far frontier answering `None` after the
    /// restart (the memo is not durable, and a restarted node holds an artifact it
    /// cannot address); the FRESH index answering the far frontier off a blind chain
    /// (then the walk does not depend on the memo and the second half proves
    /// nothing).
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
            assert!(store.insert(BOOTSTRAP, mint.clone()));

            // THE CHAIN: one mint, at the bootstrap epoch, and a committee that never
            // changes after it. The asked-epoch log is what makes the restart half
            // non-vacuous — it shows the second process asking nothing.
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

            // (1) EVERY epoch from the mint to the far frontier is minted by the
            // bootstrap epoch — resolved one epoch at a time, as the live callers do
            // (one `observe_epoch` per epoch entered).
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

            // (2) AND THE KEY IS SERVED THERE. The window is an epoch count and the
            // frontier is ten of them above the mint; nothing evicted the object that
            // answers.
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

            // Let both writers drain, then stop them: the restart below reopens the
            // same two partitions.
            context.sleep(Duration::from_millis(50)).await;
            for writer in [
                store_writer.expect("a partitioned store has a writer"),
                memo_writer.expect("a partitioned memo has a writer"),
            ] {
                writer.abort();
                drop(writer.await);
            }

            // (3) THE RESTART, over a chain that can no longer answer the bit. Every
            // epoch of this fixture is more than one window below the anchor's
            // window floor, and the committee module folds that into `None` — so a
            // walk has nothing to bottom out on and the memo is the whole answer.
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

            // (4) NEGATIVE CONTROL, and it is I4's residual as an assertion: the same
            // blind chain with a FRESH memo answers nothing. So (3) is the memo's
            // durability and not something the walk would have managed anyway — and a
            // node whose datadir is empty and whose mint is older than the window
            // stays on vote-only admission for the epoch.
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
