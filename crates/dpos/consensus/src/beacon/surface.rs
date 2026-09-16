//! The consensus-facing surface of the randomness subsystem.
//!
//! The core knows two things: the seed for a round, and whether it may sign at
//! an epoch. It does not know that a DKG exists, that a key is agreed on a
//! second consensus plane, or that an artifact is a thing that can be pulled
//! from a peer. Substituting an implementation that computes the seed from a
//! hash must not require a single change above this line.

use super::{
    actor::CeremonyStore,
    artifact::{ArtifactStore, KeyIndex},
    metrics::BeaconMetrics,
    oracle::BeaconOracle,
    seed::Seed,
    verified_seed::VerifiedSeed,
};
use crate::digest::Digest;
use commonware_consensus::{
    simplex::types::{Finalization, Notarization},
    types::{Epoch, Round, View},
};
use commonware_cryptography::bls12381::primitives::{
    group::Share, sharing::Sharing, variant::MinSig,
};
#[cfg(test)]
use commonware_runtime::Metrics;
use commonware_utils::{ordered::Error as OrderedError, Participant};
use fluentbase_bls::oracle::SeedCheck;
use fluentbase_bls::{
    beacon as beacon_bls, keys::ValidatorBlsKeypair, oracle::SeedOracle, BlsSignature,
    Scheme as BlsScheme,
};
use fluentbase_staking_reader::reader::ValidatorSetSnapshot;
use futures::future::BoxFuture;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, OnceLock,
};
use tokio::sync::{broadcast, mpsc, watch};
use tracing::{error, warn};

/// Per-epoch threshold material this node holds: the public polynomial, its
/// share (`None` for a verifier-only node) and the seed namespace.
///
/// `pub(crate)` and no wider: it is the promote gates' input and never crosses
/// the crate edge.
pub(crate) type BeaconKey = (Sharing<MinSig>, Option<Share>, Vec<u8>);

/// Why participation was withheld. Named rather than a bool because each arm
/// owns its own metric family, and those names are scraped by the devnet smoke.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum WithheldReason {
    NoUsableShare,
    BadShare,
    /// The plane has not frozen `(dpos_activation, epoch_interval)` yet, so no
    /// ceremony has been able to run. Distinct from [`Self::NoUsableShare`], which
    /// says the ceremony ran and this node holds nothing from it.
    GeometryUnfrozen,
}

/// The cheap half of the participation question, answered before the
/// boundary-block lookup, so a shareless member returns without paying a marshal
/// read.
///
/// Deliberately not [`SignerVerdict`]: at the probe's position there is no scheme
/// to hand back, the committee has not been checked and the keypair is not one of
/// the probe's inputs.
#[derive(Debug, PartialEq, Eq)]
pub enum ShareProbe {
    Ready,
    Withheld(WithheldReason),
}

/// May this node sign at an epoch, and with what scheme?
pub enum SignerVerdict {
    /// Sign with this scheme. Also the answer where [`Randomness::mandatory_at`]
    /// is false: a pre-beacon epoch has a legitimate pure-multisig signer, so this
    /// arm means "may sign", not "beacon-active".
    Signs(BlsScheme),
    /// This node's BLS key is not in the committee's map — a misconfiguration
    /// safety net, not a wedge path. Verify-only.
    RotatedKey(BlsScheme),
    /// The committee snapshot is unusable (duplicate participant keys). The
    /// caller skips the epoch, as it does today when the engine's own decode
    /// fails.
    InvalidCommittee(OrderedError),
    /// Do not sign at this epoch.
    Withheld(WithheldReason),
}

/// How much a key acquisition may spend in latency. Nothing else: the provenance
/// floor is the same at both efforts, because it follows from what the key is used
/// for rather than from how hard the caller was willing to look.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum PinEffort {
    /// Store plus locally held material. No network. The variant a vote path or
    /// a per-certificate path may call.
    Local,
    /// May additionally spend one bounded peer pull. Off-path callers only.
    Thorough,
}

/// The one door the consensus core and the node reach the beacon through.
///
/// A trait rather than a struct because the substitution seam is the point: the
/// testbed swaps a node's beacon for a static or withholding one, and production
/// has exactly one implementation. What is not on it is as load-bearing as what
/// is — how the epoch key is agreed, where the artifact is stored, how a share is
/// derived and how a peer is served all stay behind [`super::build`].
///
/// [`certificate_verdict`] is the one rule every `observe_certificate` routes
/// through.
pub trait Beacon: Send + Sync {
    /// The verified seed in force at `round`, if this node has it.
    ///
    /// Sync, and every caller depends on it: the executor's derive and the
    /// crash-survivor replay both read it without an await. Answers only for the
    /// exact round asked.
    fn seed(&self, round: Round) -> Option<Seed>;

    /// σ of `round` for the caller that asks across an epoch boundary: the
    /// boundary base of the next epoch is σ of `E-1`'s terminal round.
    ///
    /// The same read as [`Self::seed`], kept as its own operation only because the
    /// epoch manager names it. Safety rests on the only caller taking the round off
    /// the agreed terminal block rather than guessing.
    fn terminal_seed(&self, round: Round) -> Option<Seed> {
        self.seed(round)
    }

    /// Is randomness mandatory at `epoch`? Consensus-agreed data, not a local
    /// capability: every node of one network must answer identically or the derive
    /// splits the network.
    fn mandatory_at(&self, epoch: u64) -> bool;

    /// Can this node participate at `epoch` at all?
    ///
    /// The cheap probe, kept separate from [`Self::signer`]: it is asked on every
    /// reconcile edge, ahead of the boundary-block read, and folding it in would
    /// make a shareless member pay a full resolve plus a marshal read per edge.
    fn can_participate(&self, epoch: Epoch) -> ShareProbe;

    /// May this node sign at `epoch`, and with what scheme?
    fn signer(
        &self,
        epoch: Epoch,
        snap: &ValidatorSetSnapshot,
        keypair: &ValidatorBlsKeypair,
    ) -> SignerVerdict;

    /// The beacon's threshold face for `epoch`, for a scheme that only verifies.
    /// Sync: consulted inline from the simplex batcher.
    fn oracle_for(&self, epoch: u64) -> Option<Arc<dyn SeedOracle>>;

    /// Try to make `epoch`'s group key locally resolvable, and report whether it
    /// now is. Acquisition, not verification — the key never leaves the beacon.
    fn ensure_key(&self, epoch: u64, effort: PinEffort) -> BoxFuture<'_, bool>;

    /// Take the σ a verified certificate carries and return the verdict here.
    ///
    /// The verdict rule is the beacon's, so two ingresses cannot judge the same σ
    /// differently.
    fn observe_certificate(&self, cert: ObservedCertificate<'_>) -> Observed;

    /// One held artifact's wire bytes, for `consensus_getEpochArtifact`.
    ///
    /// Safe to serve unauthenticated: the artifact is self-authenticating against
    /// `committee[epoch]`.
    fn artifact_bytes(&self, epoch: u64) -> Option<Vec<u8>>;

    /// The core reconciled `reconciled` while its highest registered epoch is
    /// `entered_frontier`.
    ///
    /// A default no-op; [`LiveBeacon`] runs the artifact store's retention on it,
    /// because this edge is the one epoch clock both node classes drive.
    fn observe_epoch(&self, _reconciled: Epoch, _entered_frontier: Epoch) {}

    /// The cert-inlet ingested a verified certificate for `epoch`.
    ///
    /// A default no-op; no production caller is left. It is kept on the trait only
    /// by a delegating wrapper in the testbed.
    fn observe_cert(&self, _epoch: u64) {}

    /// Wake-ups, not facts: on every one the consumer re-reads what it needs
    /// through the queries above.
    ///
    /// Subscribe before the first read, not merely before the loop: a `broadcast`
    /// buffers from the subscription onward and drops a send with no receiver, so
    /// the ordering has to be arranged and every consumer must read before it
    /// waits. `RecvError::Lagged` is a wake-up like any other.
    fn subscribe(&self) -> broadcast::Receiver<BeaconEvent>;

    /// The late `Refused` verdict — a σ admitted with no key, refused once the key
    /// landed. A fact rather than a wake-up, so it may not be lost.
    ///
    /// At most one consumer: the first call takes the receiver, later calls get
    /// `None`. Nothing sends until a receiver has been taken, so an unread channel
    /// cannot grow.
    fn faults(&self) -> Option<mpsc::UnboundedReceiver<DataFault>>;
}

/// The certificate an ingress hands to [`Beacon::observe_certificate`].
///
/// The two arms are not interchangeable, but what separates them is the door, not
/// the σ's provenance:
///
/// - [`Self::Finalization`] is the door that must not drop a value it could
///   re-check later, so an unresolvable key there leaves the σ `Pending` for the
///   settle to judge when the key lands.
/// - [`Self::Notarization`] is the speculation door. Its σ never reaches the
///   served state on a failure, so a failure is logged and refused outright.
///
/// Commonware reports `Activity::Notarization` for a certificate received from
/// the wire exactly as for one this node assembled, and the reported activity
/// carries no provenance field, which is why the arms are named after the doors.
pub enum ObservedCertificate<'a> {
    /// The round's notarization, as the voter reported it (`spec_exec`).
    Notarization(Round, &'a Notarization<BlsScheme, Digest>),
    /// Ingested from a verified finalization (the cert inlet and the plane arm's
    /// by-height pulls).
    Finalization(Round, &'a Finalization<BlsScheme, Digest>),
}

/// What [`Beacon::observe_certificate`] did with the σ it was handed.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[must_use]
pub enum Observed {
    /// Verified under the epoch key and filed.
    Recorded,
    /// Held: the epoch key is not resolvable here yet. The same admission the
    /// certificate itself just got.
    Pending,
    /// Verified-invalid under a key whose provenance makes that an accusation.
    Refused,
    /// Nothing to do: the certificate carries no σ, or the epoch is not
    /// beacon-active.
    Inactive,
}

/// A σ that was admitted with no key and refused once the key landed.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct DataFault {
    pub epoch: u64,
    /// How many held rounds of `epoch` failed the re-check once the key landed.
    pub refused: usize,
}

/// The late-`Refused` channel, one implementation for every provider that files
/// one.
///
/// Both node classes need it: the verdict on a held σ is reached long after
/// `observe_certificate` returned, so the only way it can cost the upstream
/// anything is a channel the inlet drains. A keyless window is the ordinary state
/// for the follower, so most of its σ lands `Pending`.
///
/// `armed` gates the send rather than the receive, so an unread channel cannot
/// grow, and [`Self::take`] hands the receiver out at most once.
pub(super) struct LateFaults {
    tx: mpsc::UnboundedSender<DataFault>,
    rx: Mutex<Option<mpsc::UnboundedReceiver<DataFault>>>,
    armed: AtomicBool,
}

impl LateFaults {
    pub(super) fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            tx,
            rx: Mutex::new(Some(rx)),
            armed: AtomicBool::new(false),
        }
    }

    /// File a late `Refused` verdict. Dropped on the floor until a consumer has
    /// taken the receiver — see [`Beacon::faults`].
    pub(super) fn report(&self, fault: DataFault) {
        if !self.armed.load(Ordering::Relaxed) {
            return;
        }
        let _ = self.tx.send(fault);
    }

    /// The receiver, at most once: a second caller gets `None`, which is what
    /// makes "at most one consumer" a property of the type rather than of the
    /// current call sites.
    pub(super) fn take(&self) -> Option<mpsc::UnboundedReceiver<DataFault>> {
        let taken = self.rx.lock().ok()?.take()?;
        // Arm only once a consumer exists, so the queue can never grow behind a
        // receiver nobody reads.
        self.armed.store(true, Ordering::Relaxed);
        Some(taken)
    }
}

impl Default for LateFaults {
    fn default() -> Self {
        Self::new()
    }
}

/// Which beacon state may have changed. A wake-up: the variant names the class,
/// never the value, and the consumer re-reads through the queries.
///
/// No payload: two of the three producers have none to give, and a consumer that
/// acted on a payload instead of re-reading would be wrong the moment a `Lagged`
/// collapsed two of them.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum BeaconEvent {
    /// A σ was filed. Consumer: the executor's held-height release.
    SeedRecorded,
    /// A key landed that was not resolvable before. Consumer: the epoch manager's
    /// repair sweep.
    KeyAvailable,
    /// This node's ability to participate may have changed, in either direction.
    /// Consumer: the epoch manager's reconcile arm.
    ParticipationChanged,
}

/// A publisher nothing ever fires — for the implementations that produce no
/// beacon state (`Absent`) or produce it without an edge (`StaticRandomness`
/// deals on demand, so there is nothing to wake for). A consumer subscribing to
/// it parks forever, exactly as the idle `Notify` it replaces did.
pub(crate) fn idle_events() -> broadcast::Sender<BeaconEvent> {
    broadcast::channel(1).0
}

/// Depth of the wake-up broadcast.
///
/// Generous next to the traffic — the busiest producer is one `SeedRecorded` per
/// round, so ~1/s at the target block rate — because the cost of overflow is a
/// `Lagged` and the cost of the buffer is three bytes a slot.
pub(crate) const EVENT_BUFFER: usize = 64;

/// The verdict rule of [`Beacon::observe_certificate`], in one body.
///
/// Every implementation routes here. The sinks are closures because that is all
/// the rule needs, and the rule itself must not be per-implementation: spreading
/// it over the callers is what let two ingresses judge the same σ differently.
pub(crate) fn certificate_verdict(
    cert: ObservedCertificate<'_>,
    oracle_for: impl FnOnce(u64) -> Option<Arc<dyn SeedOracle>>,
    record: impl FnOnce(VerifiedSeed),
    hold: impl FnOnce(Round, BlsSignature),
    first_refusal: impl FnOnce(u64) -> bool,
) -> Observed {
    // `speculation` names the door, never the σ's provenance — a notarization off
    // the wire arrives here indistinguishable from one assembled locally.
    let (round, seed, speculation) = match cert {
        ObservedCertificate::Notarization(round, n) => (round, n.certificate.seed(), true),
        ObservedCertificate::Finalization(round, f) => (round, f.certificate.seed(), false),
    };
    let Some(seed) = seed else {
        return Observed::Inactive;
    };
    // No oracle below `DETERMINISTIC_BOOTSTRAP_EPOCH`: those epochs have no
    // threshold seed to hold an opinion about.
    let Some(oracle) = oracle_for(round.epoch().get()) else {
        return Observed::Inactive;
    };
    match VerifiedSeed::check(oracle.as_ref(), round, seed) {
        Ok(verified) => {
            record(verified);
            Observed::Recorded
        }
        // Not a fault: the epoch key is not resolvable here yet. Hold the value
        // rather than drop it.
        Err(SeedCheck::NoKey) => {
            hold(round, seed);
            Observed::Pending
        }
        Err(check @ SeedCheck::Invalid) if speculation => {
            // The speculation door. Refused outright rather than held: nothing
            // later re-checks a notarization's σ at this round, so a held value
            // would only accumulate.
            error!(
                ?round,
                ?check,
                "locally recovered seed did not verify under its own epoch key"
            );
            Observed::Refused
        }
        // Off the wire, a failure is always a witness: the only key this rule can
        // fail against is one a `committee[minted_at]` quorum certified, so the
        // failure says something about the sender by construction. Loud once per
        // epoch: this runs per certificate.
        Err(SeedCheck::Invalid) => {
            if first_refusal(round.epoch().get()) {
                error!(
                    ?round,
                    "certificate seed does not verify under its epoch key"
                );
            }
            Observed::Refused
        }
        // Unreachable by construction: `VerifiedSeed::check` maps
        // `SeedCheck::Valid` onto `Ok` and only the other variants onto `Err`, so
        // this arm can be reached only by changing that function.
        Err(SeedCheck::Valid) => unreachable!("Valid is the Ok arm"),
    }
}

impl<T> Beacon for T
where
    T: Randomness + ?Sized,
{
    fn seed(&self, round: Round) -> Option<Seed> {
        Randomness::seed_for(self, round)
    }

    fn mandatory_at(&self, epoch: u64) -> bool {
        Randomness::mandatory_at(self, epoch)
    }

    fn can_participate(&self, epoch: Epoch) -> ShareProbe {
        Randomness::share_probe(self, epoch)
    }

    fn signer(
        &self,
        epoch: Epoch,
        snap: &ValidatorSetSnapshot,
        keypair: &ValidatorBlsKeypair,
    ) -> SignerVerdict {
        Randomness::signer_scheme(self, epoch, snap, keypair)
    }

    fn oracle_for(&self, epoch: u64) -> Option<Arc<dyn SeedOracle>> {
        Randomness::oracle_for(self, epoch)
    }

    fn ensure_key(&self, epoch: u64, effort: PinEffort) -> BoxFuture<'_, bool> {
        Randomness::ensure_key(self, epoch, effort)
    }

    fn observe_certificate(&self, cert: ObservedCertificate<'_>) -> Observed {
        certificate_verdict(
            cert,
            |epoch| Randomness::oracle_for(self, epoch),
            |verified| self.record_seed(verified),
            |round, seed| self.hold_seed(round, seed),
            |epoch| self.first_seed_refusal(epoch),
        )
    }

    fn artifact_bytes(&self, epoch: u64) -> Option<Vec<u8>> {
        Randomness::artifact_bytes(self, epoch)
    }

    fn observe_epoch(&self, reconciled: Epoch, entered_frontier: Epoch) {
        Randomness::observe_epoch(self, reconciled, entered_frontier)
    }

    fn subscribe(&self) -> broadcast::Receiver<BeaconEvent> {
        Randomness::events(self).subscribe()
    }

    fn faults(&self) -> Option<mpsc::UnboundedReceiver<DataFault>> {
        Randomness::faults(self)
    }
}

/// The module-internal face the two production implementations still speak.
///
/// Not the boundary — [`Beacon`] is, and it is blanket-implemented over this.
/// `pub(super)`: the test implementations moved onto [`Beacon`], so nothing
/// outside `beacon/` names it.
pub(super) trait Randomness: Send + Sync {
    /// Hand over a seed that verified against its epoch key.
    ///
    /// The witness carries its own round, so no implementation can be handed a σ
    /// nobody checked.
    ///
    /// Ordering-critical: the caller invokes this synchronously, from inside the
    /// simplex reporter. Sync by signature so no implementation can move the record
    /// behind an await — the voter awaits that reporter before advancing the view,
    /// and the executor derives each height from this memo at the height's own
    /// round.
    fn record_seed(&self, verified: VerifiedSeed);

    /// Hold a σ that arrived from the network for an epoch whose key is not
    /// resolvable here yet — the `Pending` state of the index.
    ///
    /// Separate from [`record_seed`](Self::record_seed) by design: the two mean
    /// different things, and no caller can reach the served state with a value it
    /// did not check.
    fn hold_seed(&self, round: Round, seed: BlsSignature);

    /// Is this the first σ refusal reported for `epoch`? The latch behind the
    /// error line — see [`certificate_verdict`]'s `Invalid` arm.
    fn first_seed_refusal(&self, epoch: u64) -> bool;

    /// The seed in force at `round`, if this node has it. Sync: every caller reads
    /// it without an await.
    fn seed_for(&self, round: Round) -> Option<Seed>;

    /// The wake-up publisher this implementation fires, and the one fan-out for
    /// all three classes — see [`Beacon::subscribe`], which consumers hold.
    ///
    /// A `broadcast` rather than independent `Notify` handles: `notify_one` wakes
    /// exactly one waiter, so every consumer would need its own handle from every
    /// producer or silently swallow another's wake-up.
    fn events(&self) -> &broadcast::Sender<BeaconEvent>;

    /// Is randomness mandatory at `epoch`?
    ///
    /// Consensus-agreed data, not a local capability: every node of one network
    /// must answer identically, or the derive splits. The default body is the
    /// single deterministic bootstrap edge every node of this network compiles in;
    /// override it only if your module owns a different bootstrap edge, never to
    /// report a local capability.
    fn mandatory_at(&self, epoch: u64) -> bool {
        epoch >= super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH
    }

    /// Can this node participate at `epoch` at all? See [`ShareProbe`].
    fn share_probe(&self, epoch: Epoch) -> ShareProbe;

    /// May this node sign at `epoch`, and with what scheme?
    ///
    /// Sync: the implementation publishes this node's key internally before
    /// returning, and the returned scheme is a required input of the engine spawn,
    /// so publish-happens-before-spawn is a data dependency.
    fn signer_scheme(
        &self,
        epoch: Epoch,
        snap: &ValidatorSetSnapshot,
        keypair: &ValidatorBlsKeypair,
    ) -> SignerVerdict;

    /// The beacon's threshold face for `epoch`, for a scheme that only verifies.
    ///
    /// Sync: the returned oracle is consulted inline from the simplex batcher, so
    /// anything it needs has to be resolvable without an await; acquiring what it
    /// reads is [`Self::ensure_key`]'s job, off-path.
    ///
    /// Contract, binding on every implementation: this must answer `None` where
    /// [`Self::mandatory_at`] is false. An oracle's presence is what tells
    /// `verify_certificate` "this epoch is beacon-active", so one attached to a
    /// pre-beacon epoch rejects every legal seedless certificate there.
    fn oracle_for(&self, epoch: u64) -> Option<Arc<dyn SeedOracle>>;

    /// Try to make `epoch`'s group key locally resolvable, and report whether it
    /// now is. Acquisition, not verification: the value never leaves the beacon —
    /// what reads it is the oracle above.
    ///
    /// Async because [`PinEffort::Thorough`] may spend one bounded peer round-trip.
    /// [`PinEffort::Local`] is contractually network-free.
    fn ensure_key(&self, epoch: u64, effort: PinEffort) -> BoxFuture<'_, bool>;

    /// One held artifact's wire bytes. Defaults to "this provider holds no
    /// artifact store" — the plane and the follower override it.
    fn artifact_bytes(&self, _epoch: u64) -> Option<Vec<u8>> {
        None
    }

    /// The core's epoch edge, see [`Beacon::observe_epoch`]. Defaults to a no-op;
    /// only a provider that holds an artifact store has anything to age out.
    fn observe_epoch(&self, _reconciled: Epoch, _entered_frontier: Epoch) {}

    /// The late-`Refused` channel's receiver, at most once. Defaults to "this
    /// provider never files a late verdict".
    fn faults(&self) -> Option<mpsc::UnboundedReceiver<DataFault>> {
        None
    }
}

/// The total, permanently-negative provider.
///
/// "Not yet" is the wrong answer for a node class whose rungs are absent for the
/// life of the process rather than until something resolves. This turns that
/// contract from prose into a type.
///
/// It does not describe `--cert-follow` any more: that class carries keys and its
/// own seed index, so it runs the same [`LiveBeacon`] a validator does. What is
/// left here is a struct-literal default that no production consumer observes,
/// plus the executor's test module.
///
/// Takes a metrics context because registration is context-scoped and commonware
/// prefixes each family with the context's label path.
#[cfg(test)]
pub(crate) fn absent(ctx: &impl Metrics) -> Arc<dyn Beacon> {
    let metrics = BeaconMetrics::default();
    metrics.register(ctx);
    absent_unregistered()
}

/// The same negative provider without the registration.
///
/// For instances that publish nothing by design: every test, and a
/// [`crate::application::FluentApp`] before its provider is attached.
pub(crate) fn absent_unregistered() -> Arc<dyn Beacon> {
    Arc::new(Absent {
        events: idle_events(),
    })
}

/// The acceptance test's implementation: randomness with no DKG, no agreement
/// plane, no network and no store.
///
/// The bar is that an implementation computing the seed from a hash be
/// substitutable here without a single change above this file; this is that
/// implementation.
///
/// It hashes into a sharing, not into each σ: a seed is a BLS threshold signature
/// real members recover from real partials, so a mock returning `H(round)` as a
/// signature would be rejected by `verify_seed` on the first vote. What can be
/// made static is the DKG's output: seed a deterministic RNG from
/// `H(chain_id ‖ epoch ‖ committee)`, deal once, and every member derives the
/// identical `PK_epoch` and share set. Everything downstream is the shipped
/// crypto.
#[cfg(test)]
pub(crate) struct StaticRandomness {
    chain_id: u64,
    namespace: Vec<u8>,
    /// The committee this implementation is randomness for: that is the whole
    /// input its `PK` is derived from.
    snap: ValidatorSetSnapshot,
    events: broadcast::Sender<BeaconEvent>,
}

#[cfg(test)]
impl StaticRandomness {
    pub(crate) fn build(chain_id: u64, snap: ValidatorSetSnapshot) -> Arc<dyn Beacon> {
        Arc::new(Self {
            chain_id,
            namespace: fluentbase_bls::beacon::seed_namespace(&fluentbase_bls::fluent_namespace(
                chain_id,
            )),
            snap,
            events: idle_events(),
        })
    }

    /// The whole of this implementation's "DKG": one deterministic deal, keyed on
    /// data every node of the epoch already agrees on.
    fn deal(
        &self,
        epoch: u64,
    ) -> (
        commonware_cryptography::bls12381::primitives::sharing::Sharing<
            commonware_cryptography::bls12381::primitives::variant::MinSig,
        >,
        Vec<commonware_cryptography::bls12381::primitives::group::Share>,
    ) {
        use commonware_cryptography::{
            bls12381::{dkg::deal_anonymous, primitives::variant::MinSig},
            Hasher as _, Sha256,
        };
        use rand_core::SeedableRng as _;

        let mut h = Sha256::new();
        h.update(&self.chain_id.to_be_bytes());
        h.update(&epoch.to_be_bytes());
        // Sorted, so the digest does not depend on snapshot iteration order.
        let mut peers: Vec<&[u8]> = self
            .snap
            .validators
            .iter()
            .map(|v| v.keys.peer_pubkey.as_ref())
            .collect();
        peers.sort_unstable();
        for p in &peers {
            h.update(p);
        }
        let digest = h.finalize();
        let mut seed = [0u8; 32];
        seed.copy_from_slice(digest.as_ref());
        let mut rng = rand_08::rngs::StdRng::from_seed(seed);
        let n = commonware_utils::NZU32!(u32::try_from(peers.len()).expect("committee fits u32"));
        deal_anonymous::<MinSig, commonware_utils::N3f1>(&mut rng, Default::default(), n)
    }

    /// A [`SeedOracle`] over the epoch's derived sharing; deriving instead of
    /// storing is what makes this provider static.
    fn dealt_oracle(&self, epoch: u64, share: Option<Share>) -> Arc<dyn SeedOracle> {
        Arc::new(DealtOracle {
            sharing: self.deal(epoch).0,
            share,
            namespace: self.namespace.clone(),
        })
    }

    /// σ for a round, produced the way the network produces it — `t` real
    /// partials, really recovered — just without anyone sending them.
    fn sigma(&self, round: Round) -> BlsSignature {
        let (sharing, shares) = self.deal(round.epoch().get());
        let t = sharing.required::<commonware_utils::N3f1>() as usize;
        let partials: Vec<_> = shares[..t]
            .iter()
            .map(|sh| fluentbase_bls::beacon::sign_seed_partial(sh, &self.namespace, round))
            .collect();
        fluentbase_bls::beacon::recover_seed::<commonware_utils::N3f1>(&sharing, &partials)
            .expect("t honest partials recover")
    }
}

#[cfg(test)]
/// A [`SeedOracle`] over one fixed sharing and optional share. Test-only:
/// production reads the live ceremony store through [`BeaconOracle`]. Shared with
/// the cert-inlet's beacon fixtures.
#[derive(Debug)]
pub(crate) struct DealtOracle {
    pub(crate) sharing: Sharing<MinSig>,
    pub(crate) share: Option<Share>,
    pub(crate) namespace: Vec<u8>,
}

#[cfg(test)]
impl SeedOracle for DealtOracle {
    fn sign_partial(&self, round: Round) -> Option<BlsSignature> {
        let share = self.share.as_ref()?;
        Some(beacon_bls::sign_seed_partial(share, &self.namespace, round).value)
    }

    fn verify_partial(&self, round: Round, index: Participant, value: &BlsSignature) -> bool {
        use commonware_cryptography::bls12381::primitives::variant::PartialSignature;
        beacon_bls::verify_seed_partial(
            &self.sharing,
            &self.namespace,
            round,
            &PartialSignature::<MinSig> {
                index,
                value: *value,
            },
        )
    }

    fn recover(
        &self,
        partials: &[(Participant, BlsSignature)],
        threshold: u32,
    ) -> Option<BlsSignature> {
        use commonware_cryptography::bls12381::primitives::variant::PartialSignature;
        let partials: Vec<PartialSignature<MinSig>> = partials
            .iter()
            .map(|&(index, value)| PartialSignature::<MinSig> { index, value })
            .collect();
        beacon_bls::recover_seed_with_threshold(&self.sharing, &partials, threshold).ok()
    }

    fn verify_seed(&self, round: Round, seed: &BlsSignature) -> SeedCheck {
        if beacon_bls::verify_seed(self.sharing.public(), &self.namespace, round, seed) {
            SeedCheck::Valid
        } else {
            SeedCheck::Invalid
        }
    }
}

#[cfg(test)]
impl Beacon for StaticRandomness {
    fn subscribe(&self) -> broadcast::Receiver<BeaconEvent> {
        self.events.subscribe()
    }

    /// The network-wide agreed rule, compiled in exactly as the live beacon's is.
    /// Answering a local capability here would split the derive.
    fn mandatory_at(&self, epoch: u64) -> bool {
        epoch >= super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH
    }

    fn seed(&self, round: Round) -> Option<Seed> {
        Some(Seed {
            target_round: round,
            signature: self.sigma(round),
        })
    }

    /// Never withheld: this implementation cannot fail to hold a share.
    fn can_participate(&self, _epoch: Epoch) -> ShareProbe {
        ShareProbe::Ready
    }

    /// The shared verdict rule over an empty sink: σ is recomputable for any
    /// round, so there is no memo to feed and nothing to hold; no key store to
    /// judge provenance with, so a failure proves nothing.
    fn observe_certificate(&self, cert: ObservedCertificate<'_>) -> Observed {
        certificate_verdict(
            cert,
            |epoch| self.oracle_for(epoch),
            |_verified| {},
            |_round, _seed| {},
            |_epoch| true,
        )
    }

    /// No artifact store: this implementation derives its key and stores nothing,
    /// so there is nothing to serve over `consensus_getEpochArtifact`.
    fn artifact_bytes(&self, _epoch: u64) -> Option<Vec<u8>> {
        None
    }

    /// Never files a late verdict: with no key store, no σ is ever admitted
    /// pending a key that could later refuse it.
    fn faults(&self) -> Option<mpsc::UnboundedReceiver<DataFault>> {
        None
    }

    fn signer(
        &self,
        epoch: Epoch,
        snap: &ValidatorSetSnapshot,
        keypair: &ValidatorBlsKeypair,
    ) -> SignerVerdict {
        let committee = match crate::scheme::epoch_committee_from_snapshot(snap) {
            Ok(c) => c,
            Err(e) => return SignerVerdict::InvalidCommittee(e),
        };
        let ns = fluentbase_bls::fluent_namespace(self.chain_id);
        let shares = self.deal(epoch.get()).1;
        // The share index must equal this node's consensus participant index —
        // the oracle refuses to sign otherwise. Ask the vote scheme where this
        // member sits rather than assuming the fixture's ordering.
        let seat = {
            use commonware_cryptography::certificate::Scheme as _;
            fluentbase_bls::scheme::build_signer(
                &ns,
                committee.bimap.clone(),
                keypair,
                epoch.get(),
                None,
            )
            .and_then(|s| s.me())
        };
        let Some(seat) = seat else {
            // Mirrors `LiveBeacon`: a rotated-out node still verifies the epoch's
            // seed slot.
            return SignerVerdict::RotatedKey(fluentbase_bls::scheme::build_verifier(
                &ns,
                committee.bimap,
                epoch.get(),
                self.oracle_for(epoch.get()),
            ));
        };
        // Unconditional, unlike this type's own `oracle_for` and unlike
        // `LiveBeacon`'s `Signs` arm, because this fixture can derive material for
        // any epoch. Every driver of this arm signs at a beacon-active epoch; a
        // future pre-beacon driver should gate this on `mandatory_at`.
        let oracle = self.dealt_oracle(epoch.get(), Some(shares[usize::from(seat)].clone()));
        match fluentbase_bls::scheme::build_signer(
            &ns,
            committee.bimap,
            keypair,
            epoch.get(),
            Some(oracle),
        ) {
            Some(scheme) => SignerVerdict::Signs(scheme),
            None => unreachable!("the seat lookup above already proved membership"),
        }
    }

    fn oracle_for(&self, epoch: u64) -> Option<Arc<dyn SeedOracle>> {
        // The contract's pre-beacon refusal binds every implementation: being able
        // to derive a key for any epoch is exactly what would make this one violate
        // it silently.
        self.mandatory_at(epoch)
            .then(|| self.dealt_oracle(epoch, None))
    }

    fn ensure_key(&self, epoch: u64, _effort: PinEffort) -> BoxFuture<'_, bool> {
        // Both efforts answer identically: there is no network rung to spend, and
        // a derivable key is always already resolvable.
        Box::pin(async move { self.mandatory_at(epoch) })
    }
}

/// An empty share store: for every test provider, and for the `--cert-follow`
/// follower, none of which carries local DKG material.
///
/// With no share, every material-bound answer of [`super::oracle::BeaconOracle`]
/// goes through `with_material`, which returns `None` on an empty store.
pub(super) fn keyless_ceremony() -> CeremonyStore {
    Arc::new(std::sync::RwLock::new(std::collections::BTreeMap::new()))
}

/// A [`KeyIndex`] over an empty artifact store and a bit history with nothing set:
/// it resolves the bootstrap mint and finds no artifact for it, which is the
/// "keyless" state every provider below wants.
#[cfg(test)]
pub(crate) fn keyless_index() -> KeyIndex {
    super::artifact::key_index_over(ArtifactStore::new(), &[])
}

/// Holds no store, and that is the honest shape: `ensure_key` never reads one and
/// no writer can reach one.
struct Absent {
    events: broadcast::Sender<BeaconEvent>,
}

impl Beacon for Absent {
    fn subscribe(&self) -> broadcast::Receiver<BeaconEvent> {
        self.events.subscribe()
    }

    /// The one operation this type answers truthfully rather than negatively:
    /// beacon-activity is network-wide agreed data, so answering it negatively
    /// would split the network rather than degrade this node.
    fn mandatory_at(&self, epoch: u64) -> bool {
        epoch >= super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH
    }

    fn seed(&self, _round: Round) -> Option<Seed> {
        None
    }

    fn can_participate(&self, _epoch: Epoch) -> ShareProbe {
        ShareProbe::Withheld(WithheldReason::NoUsableShare)
    }

    fn signer(
        &self,
        _epoch: Epoch,
        _snap: &ValidatorSetSnapshot,
        _keypair: &ValidatorBlsKeypair,
    ) -> SignerVerdict {
        SignerVerdict::Withheld(WithheldReason::NoUsableShare)
    }

    /// No oracle at any epoch, permanently: with nothing to read, one would only
    /// turn every legal seedless certificate into a rejection.
    fn oracle_for(&self, _epoch: u64) -> Option<Arc<dyn SeedOracle>> {
        None
    }

    fn ensure_key(&self, _epoch: u64, _effort: PinEffort) -> BoxFuture<'_, bool> {
        Box::pin(async { false })
    }

    /// `Inactive` at every epoch, through the shared rule rather than beside it:
    /// with no oracle anywhere, the rule's own second gate answers, and the sinks
    /// below are unreachable.
    fn observe_certificate(&self, cert: ObservedCertificate<'_>) -> Observed {
        certificate_verdict(
            cert,
            |_epoch| None,
            |_verified| {},
            |_round, _seed| {},
            |_epoch| true,
        )
    }

    fn artifact_bytes(&self, _epoch: u64) -> Option<Vec<u8>> {
        None
    }

    fn faults(&self) -> Option<mpsc::UnboundedReceiver<DataFault>> {
        None
    }
}

/// Test provider. One place for every migrated test to get a [`Beacon`], and a
/// spy: it records the `(epoch, effort)` of every [`Beacon::ensure_key`] call.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use crate::beacon::artifact::MintFixture;
    use std::{collections::BTreeMap, sync::Mutex};

    pub(crate) struct Canned {
        /// Canned mints, reachable through [`Beacon::oracle_for`] exactly as a real
        /// one is: a real [`KeyIndex`] over a real artifact store.
        mints: MintFixture,
        seeds: BTreeMap<Round, Seed>,
        bootstrap: u64,
        efforts: Mutex<Vec<(u64, PinEffort)>>,
        events: broadcast::Sender<BeaconEvent>,
        /// A real store, so a test can assert where a captured σ landed —
        /// served or held — instead of only that the call happened.
        store: crate::beacon::seed_index::SeedIndex,
        /// Must match the namespace the fixture signed under, or every σ this
        /// provider is asked about is `Invalid` rather than `Valid`.
        seed_namespace: Vec<u8>,
    }

    impl Canned {
        pub(crate) fn new() -> Self {
            Self {
                mints: MintFixture::new(),
                seeds: BTreeMap::new(),
                bootstrap: super::super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH,
                efforts: Mutex::new(Vec::new()),
                events: idle_events(),
                store: crate::beacon::seed_index::SeedIndex::new(),
                seed_namespace: Vec::new(),
            }
        }

        /// State a mint rather than a bare key: the key is a projection of the
        /// epoch's artifact, so a fixture may not offer one directly.
        pub(crate) fn with_mint(
            self,
            minted_at: u64,
            group_key: crate::beacon::outcome::DkgOutcome,
        ) -> Self {
            self.mints.mint(minted_at, group_key);
            self
        }

        pub(crate) fn with_seed_namespace(mut self, namespace: Vec<u8>) -> Self {
            self.seed_namespace = namespace;
            self
        }

        pub(crate) fn store(&self) -> &crate::beacon::seed_index::SeedIndex {
            &self.store
        }

        /// Every `ensure_key` this provider answered, in call order.
        pub(crate) fn efforts(&self) -> Vec<(u64, PinEffort)> {
            self.efforts.lock().expect("efforts lock").clone()
        }
    }

    impl Beacon for Canned {
        fn subscribe(&self) -> broadcast::Receiver<BeaconEvent> {
            self.events.subscribe()
        }

        fn seed(&self, round: Round) -> Option<Seed> {
            self.seeds.get(&round).cloned()
        }

        /// Reads the real `store` — the index [`Beacon::observe_certificate`]
        /// writes — not the default body, because this fixture's `seed` answers
        /// from its own canned map.
        fn terminal_seed(&self, round: Round) -> Option<Seed> {
            self.store.seed(round).map(|signature| Seed {
                target_round: round,
                signature,
            })
        }

        fn mandatory_at(&self, epoch: u64) -> bool {
            epoch >= self.bootstrap
        }

        fn can_participate(&self, _epoch: Epoch) -> ShareProbe {
            ShareProbe::Withheld(WithheldReason::NoUsableShare)
        }

        fn signer(
            &self,
            _epoch: Epoch,
            _snap: &ValidatorSetSnapshot,
            _keypair: &ValidatorBlsKeypair,
        ) -> SignerVerdict {
            SignerVerdict::Withheld(WithheldReason::NoUsableShare)
        }

        /// The production oracle over an empty ceremony store — the follower's
        /// shape: `verify_seed` answers off [`KeyIndex`], and every material-bound
        /// answer is `None` because this fixture holds no share.
        fn oracle_for(&self, epoch: u64) -> Option<Arc<dyn SeedOracle>> {
            self.mandatory_at(epoch).then(|| {
                Arc::new(super::super::oracle::BeaconOracle {
                    epoch,
                    ceremony: keyless_ceremony(),
                    keys: self.mints.keys.clone(),
                    namespace: self.seed_namespace.clone(),
                    me: None,
                    warned_threshold_mismatch: Arc::new(AtomicBool::new(false)),
                    warned_seat_mismatch: Arc::new(AtomicBool::new(false)),
                    metrics: BeaconMetrics::default(),
                }) as Arc<dyn SeedOracle>
            })
        }

        fn ensure_key(&self, epoch: u64, effort: PinEffort) -> BoxFuture<'_, bool> {
            self.efforts
                .lock()
                .expect("efforts lock")
                .push((epoch, effort));
            let known = self.mints.keys.key_at(epoch).is_some();
            Box::pin(async move { known })
        }

        /// The shared verdict rule over the real stores this fixture carries: a
        /// recorded σ lands `Verified` in `store`, a held one `Pending`.
        fn observe_certificate(&self, cert: ObservedCertificate<'_>) -> Observed {
            certificate_verdict(
                cert,
                |epoch| self.oracle_for(epoch),
                |verified| self.store.record(verified),
                |round, seed| self.store.hold(round, seed),
                |_epoch| true,
            )
        }

        fn artifact_bytes(&self, _epoch: u64) -> Option<Vec<u8>> {
            None
        }

        fn faults(&self) -> Option<mpsc::UnboundedReceiver<DataFault>> {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_runtime::{deterministic::Runner, Runner as _};
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng;

    #[test]
    fn the_absent_provider_answers_negatively_except_on_the_network_wide_rule() {
        let runner = Runner::default();
        runner.start(|ctx| async move {
            let r = absent(&ctx);

            assert_eq!(
                r.can_participate(Epoch::new(9)),
                ShareProbe::Withheld(WithheldReason::NoUsableShare)
            );
            assert!(r
                .seed(Round::new(
                    Epoch::new(9),
                    commonware_consensus::types::View::new(1)
                ))
                .is_none());
            assert!(r.oracle_for(9).is_none());
            assert!(!r.ensure_key(9, PinEffort::Local).await);
            assert!(!r.ensure_key(9, PinEffort::Thorough).await);

            // The one truthful answer: beacon-activity is network-wide agreed
            // data — it decides whether a height derives from σ or from `None` — so
            // answering it negatively would split the network.
            assert!(!r.mandatory_at(super::super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH - 1));
            assert!(r.mandatory_at(super::super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH));
        });
    }

    /// A committee snapshot with real BLS keypairs, plus a real DKG outcome dealt
    /// over its own peer set. Returns the keypair at index 0 (a member) and one
    /// that is not in the committee.
    ///
    /// It deals an `Output` rather than an anonymous `Sharing`: the polynomial a
    /// node signs with is now a projection of the epoch's artifact, and an artifact
    /// carries an `Output`.
    fn signer_fixture(
        epoch: Epoch,
    ) -> (
        ValidatorSetSnapshot,
        ValidatorBlsKeypair,
        ValidatorBlsKeypair,
        super::super::outcome::DkgOutcome,
        Share,
    ) {
        use alloy_primitives::{Address, B256};
        use commonware_codec::DecodeExt as _;
        use commonware_cryptography::{
            bls12381::{
                dkg::deal,
                primitives::{sharing::Mode, variant::MinSig},
            },
            ed25519::PrivateKey as Ed25519PrivateKey,
            Signer as _,
        };
        use commonware_math::algebra::Random as _;
        use commonware_utils::{ordered::Set, N3f1};
        use fluentbase_bls::{BlsPubkey, PeerPubkey};
        use fluentbase_staking_reader::reader::{ConsensusKeys, ValidatorWithKeys};

        let mut keypairs = Vec::new();
        let mut peers: Vec<PeerPubkey> = Vec::new();
        let validators = (0..4u8)
            .map(|i| {
                let mut rng = StdRng::seed_from_u64(0x5EED + i as u64);
                let kp = ValidatorBlsKeypair::generate(&mut rng);
                let bls_pubkey = BlsPubkey::decode(kp.public_bytes().as_slice()).unwrap();
                keypairs.push(kp);
                let peer = Ed25519PrivateKey::random(&mut rng).public_key();
                peers.push(peer.clone());
                ValidatorWithKeys {
                    address: Address::repeat_byte(i),
                    keys: ConsensusKeys {
                        bls_pubkey,
                        peer_pubkey: peer,
                        activation_epoch: 1,
                    },
                    tombstoned: false,
                }
            })
            .collect();
        let snap = ValidatorSetSnapshot {
            block_hash: B256::repeat_byte(0x22),
            block_number: 7,
            epoch: epoch.get(),
            validators,
            weights: None,
        };

        // Dealt over the committee's own peer set, so each player's share index is
        // the consensus participant index the oracle checks it against — by
        // construction rather than by a seat lookup.
        let players: Set<PeerPubkey> = Set::from_iter_dedup(peers.iter().cloned());
        let mut rng = StdRng::seed_from_u64(0xDEA1);
        let (outcome, shares) =
            deal::<MinSig, PeerPubkey, N3f1>(&mut rng, Mode::NonZeroCounter, players)
                .expect("deal");
        let member = keypairs.remove(0);
        let share = shares
            .get_value(&peers[0])
            .expect("the fixture's member has a share")
            .clone();

        let outsider = ValidatorBlsKeypair::generate(&mut StdRng::seed_from_u64(0xDEAD));
        (snap, member, outsider, outcome, share)
    }

    /// A live beacon whose key index answers for `epoch` out of a real artifact, and
    /// whose share store holds `share` at the same minting epoch — the shape a
    /// qualified member is in.
    fn provider_over(
        epoch: Epoch,
        outcome: super::super::outcome::DkgOutcome,
        share: Option<Share>,
    ) -> Arc<dyn Beacon> {
        let mints = super::super::artifact::MintFixture::new();
        mints.mint(epoch.get(), outcome);
        let ceremony = keyless_ceremony();
        if let Some(share) = share {
            ceremony
                .write()
                .expect("share store")
                .insert(epoch.get(), share);
        }
        LiveBeacon::build(LiveBeaconConfig {
            seeds: super::super::seed_index::SeedIndex::new(),
            keys: mints.keys.clone(),
            ceremony,
            acquire: None,
            metrics: BeaconMetrics::default(),
            chain_id: 1,
            artifacts: mints.artifacts.clone(),
            geometry: watch::channel(Some((0, 1))).1,
        })
    }

    /// A live beacon that resolves no key: the state of a node whose mint artifact
    /// has not reached it.
    fn keyless_provider() -> Arc<dyn Beacon> {
        LiveBeacon::build(LiveBeaconConfig {
            seeds: super::super::seed_index::SeedIndex::new(),
            keys: keyless_index(),
            ceremony: keyless_ceremony(),
            acquire: None,
            metrics: BeaconMetrics::default(),
            chain_id: 1,
            artifacts: ArtifactStore::new(),
            geometry: watch::channel(Some((0, 1))).1,
        })
    }

    /// Handing out a signing scheme writes nothing anywhere: with no
    /// locally-derived writer left, the signer path is read-only.
    ///
    /// Reds if the artifact store grows an entry the fixture did not put there.
    #[test]
    fn a_signing_scheme_publishes_nothing_of_its_own() {
        let epoch = Epoch::new(9);
        let (snap, member, _outsider, outcome, share) = signer_fixture(epoch);
        let mints = super::super::artifact::MintFixture::new();
        mints.mint(epoch.get(), outcome);
        let ceremony = keyless_ceremony();
        ceremony
            .write()
            .expect("share store")
            .insert(epoch.get(), share);
        let before = mints.artifacts.epochs();
        let randomness = LiveBeacon::build(LiveBeaconConfig {
            seeds: super::super::seed_index::SeedIndex::new(),
            keys: mints.keys.clone(),
            ceremony,
            acquire: None,
            metrics: BeaconMetrics::default(),
            chain_id: 1,
            artifacts: mints.artifacts.clone(),
            geometry: watch::channel(Some((0, 1))).1,
        });

        assert!(
            matches!(
                randomness.signer(epoch, &snap, &member),
                SignerVerdict::Signs(_)
            ),
            "a committee member holding verifying DKG material still signs — the scheme \
             is built from the material, only the PUBLISH is gone"
        );
        assert_eq!(
            mints.artifacts.epochs(),
            before,
            "the signer path must leave the fact's owner untouched: after П-3 the only \
             writers are the agreement write-back and a verified acquisition"
        );
    }

    /// The core's epoch edge is where the artifact store ages out, on both node
    /// classes: mints at {2, 5, 55} with the frontier at 60 keep {5, 55} (52 resolves
    /// to 5), and the live mint of a stable committee survives any frontier.
    #[test]
    fn the_epoch_edge_retains_the_artifacts_the_window_resolves_to() {
        let mints = super::super::artifact::MintFixture::new();
        for (mint, seed) in [(2u64, 0xA1u64), (5, 0xA2), (55, 0xA3)] {
            let (_, _, _, outcome, _) = signer_fixture(Epoch::new(seed));
            mints.mint(mint, outcome);
        }
        let randomness = LiveBeacon::build(LiveBeaconConfig {
            seeds: super::super::seed_index::SeedIndex::new(),
            keys: mints.keys.clone(),
            ceremony: keyless_ceremony(),
            acquire: None,
            metrics: BeaconMetrics::default(),
            chain_id: 1,
            artifacts: mints.artifacts.clone(),
            geometry: watch::channel(Some((0, 1))).1,
        });
        assert_eq!(mints.artifacts.epochs(), vec![2, 5, 55]);

        Beacon::observe_epoch(&*randomness, Epoch::new(60), Epoch::new(60));
        assert_eq!(
            mints.artifacts.epochs(),
            vec![5, 55],
            "the mint no epoch in the window resolves to must go, the others stay"
        );

        Beacon::observe_epoch(&*randomness, Epoch::new(10_000), Epoch::new(10_000));
        assert_eq!(
            mints.artifacts.epochs(),
            vec![55],
            "on a committee stable since 55 the live mint outlives every window"
        );
    }

    /// The misconfiguration safety net: a keypair the committee does not contain
    /// gets a verify-only scheme, not a withhold. The caller still spawns on it
    /// (and aborts on its next reconcile), which is the behaviour the engine used
    /// to implement for itself.
    #[test]
    fn a_keypair_outside_the_committee_gets_a_verify_only_scheme() {
        let epoch = Epoch::new(9);
        let (snap, _member, outsider, outcome, share) = signer_fixture(epoch);
        let randomness = provider_over(epoch, outcome, Some(share));

        assert!(matches!(
            randomness.signer(epoch, &snap, &outsider),
            SignerVerdict::RotatedKey(_)
        ));
    }

    /// A rotated-out node still judges the epoch's seed slot, a producer
    /// obligation no downstream guard covers: `Committee::upgrade_scheme` only
    /// checks an occupied scheme slot, and an empty one is filled unconditionally.
    /// The `RotatedKey` arm must therefore carry the epoch's oracle.
    #[test]
    fn a_rotated_out_node_still_verifies_the_epochs_seed_slot() {
        use crate::committee::Committee as _;
        use crate::outer::EpochSchemeProvider;
        use commonware_cryptography::certificate::Provider as _;

        let epoch = Epoch::new(9);
        let (snap, _member, outsider, outcome, share) = signer_fixture(epoch);
        let randomness = provider_over(epoch, outcome, Some(share));
        assert!(
            randomness.mandatory_at(epoch.get()),
            "fixture precondition: the epoch is beacon-active, so an oracle is \
             owed here"
        );

        let SignerVerdict::RotatedKey(scheme) = randomness.signer(epoch, &snap, &outsider) else {
            panic!("a keypair outside the committee gets the RotatedKey verdict");
        };
        assert!(
            scheme.is_beacon_active(),
            "the misconfiguration safety net must still carry the epoch's oracle"
        );

        // The vacant-slot path, spelled out because it is the reason the
        // assertion above cannot be moved into the map: nothing there checks
        // anything, so whatever the producer emitted is what the epoch gets.
        let module = crate::committee::testing::SchemeCommittee::new(|_| None);
        assert!(module.upgrade_scheme(epoch.get(), scheme));
        let provider = EpochSchemeProvider::new(module);
        assert!(
            provider
                .scoped(epoch)
                .expect("registered")
                .is_beacon_active(),
            "an empty scheme slot is filled unconditionally — the map cannot \
             tell that this epoch is beacon-active, so the producer must be \
             right on first insert"
        );
    }

    /// Each withhold reason owns a different metric family, so collapsing two of
    /// them loses a signal the devnet smoke scrapes. A share that does not verify
    /// against its own sharing reports `BadShare`, never the value gate's
    /// `KeyDivergence`.
    #[test]
    fn a_share_that_does_not_verify_reports_bad_share() {
        let epoch = Epoch::new(9);
        let (snap, member, _outsider, outcome, _share) = signer_fixture(epoch);
        // The epoch's real outcome, paired with a share from an unrelated deal at
        // the same index: the polynomial is right, the share's value is not on it.
        // That is the one shape `adopt_share`'s gate cannot have caught — a share
        // file written by an older binary, or edited under a running node.
        let mut rng = StdRng::seed_from_u64(0xBAD5);
        let (_, foreign) = commonware_cryptography::bls12381::dkg::deal_anonymous::<
            commonware_cryptography::bls12381::primitives::variant::MinSig,
            commonware_utils::N3f1,
        >(&mut rng, Default::default(), commonware_utils::NZU32!(4));
        let randomness = provider_over(epoch, outcome, Some(foreign[0].clone()));

        assert!(matches!(
            randomness.signer(epoch, &snap, &member),
            SignerVerdict::Withheld(WithheldReason::BadShare)
        ));
    }

    /// The acceptance test for this ticket: drives the whole randomness contract —
    /// the σ a certificate carries in, the store lookup a derive reads out, the
    /// signing scheme, and the certificate pin — against an implementation with no
    /// DKG, no agreement plane, no network and no store.
    ///
    /// Not a mock returning canned answers: every value is real BLS. What is static
    /// is only the deal (see [`StaticRandomness`]).
    #[test]
    fn the_whole_surface_is_satisfied_by_an_implementation_with_no_dkg_and_no_store() {
        let epoch = Epoch::new(9);
        let round = Round::new(epoch, commonware_consensus::types::View::new(4));
        let (snap, member, outsider, _outcome, _share) = signer_fixture(epoch);
        let r = StaticRandomness::build(1, snap.clone());

        // Propose: the leader has a seed for the round, with no one to receive it
        // from and nothing recorded.
        let seed = r.seed(round).expect("a static implementation always has σ");
        assert_eq!(seed.target_round, round);

        // Verify: another node checks that seed under the key in force, and agrees
        // — the two halves derived the same PK from the same committee without
        // exchanging anything. Asked through `oracle_for`, the seam the certificate
        // door checks a carried σ with.
        let oracle = r
            .oracle_for(epoch.get())
            .expect("a beacon-active epoch carries an oracle");
        assert!(crate::beacon::verified_seed::VerifiedSeed::check(
            oracle.as_ref(),
            round,
            seed.signature
        )
        .is_ok());

        // A different round's σ must not verify as this one's, or the Ok above
        // would also pass for an implementation that ignored the round entirely.
        let other = Round::new(epoch, commonware_consensus::types::View::new(5));
        assert!(matches!(
            crate::beacon::verified_seed::VerifiedSeed::check(
                oracle.as_ref(),
                round,
                r.seed(other).expect("σ").signature
            ),
            Err(fluentbase_bls::oracle::SeedCheck::Invalid)
        ));

        // Participation and scheme construction.
        assert_eq!(r.can_participate(epoch), ShareProbe::Ready);
        assert!(matches!(
            r.signer(epoch, &snap, &member),
            SignerVerdict::Signs(_)
        ));
        assert!(
            matches!(
                r.signer(epoch, &snap, &outsider),
                SignerVerdict::RotatedKey(_)
            ),
            "the rotated-out safety net is part of the contract, not of the beacon"
        );

        // The certificate oracle, and the network-wide rule.
        assert!(
            r.oracle_for(epoch.get()).is_some(),
            "a beacon-active epoch must carry an oracle, or its certificates are \
             admitted on the multisig half alone"
        );
        let runner = Runner::default();
        runner.start(move |_| async move {
            assert!(
                r.ensure_key(epoch.get(), PinEffort::Local).await,
                "a static implementation can always derive the epoch key"
            );
            assert!(
                r.ensure_key(epoch.get(), PinEffort::Thorough).await,
                "both efforts answer from the same derivation"
            );
            assert!(r.mandatory_at(super::super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH));
        });
    }

    /// The divergence class is gone by construction: the polynomial and the
    /// attested key are read from one object, the artifact of the epoch the chain
    /// says minted the key, so there is no second value to disagree with.
    ///
    /// What this asserts is the identity: the key the probe and the signer judge by
    /// is byte-for-byte the artifact's, on both the exact-mint and the carried path.
    #[test]
    fn the_key_and_the_polynomial_are_one_object_on_the_mint_and_on_a_carry() {
        let mint = Epoch::new(super::super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH + 1);
        let (snap, member, _outsider, outcome, share) = signer_fixture(mint);
        let expected = *outcome.public().public();
        let mints = super::super::artifact::MintFixture::new();
        mints.mint(mint.get(), outcome);
        let ceremony = keyless_ceremony();
        ceremony
            .write()
            .expect("share store")
            .insert(mint.get(), share);
        let keys = mints.keys.clone();
        let r = LiveBeacon::build(LiveBeaconConfig {
            seeds: super::super::seed_index::SeedIndex::new(),
            keys: keys.clone(),
            ceremony,
            acquire: None,
            metrics: BeaconMetrics::default(),
            chain_id: 1,
            artifacts: mints.artifacts.clone(),
            geometry: watch::channel(Some((0, 1))).1,
        });

        // The member participates and signs — no gate has anything to compare.
        assert_eq!(r.can_participate(mint), ShareProbe::Ready);
        assert!(matches!(
            r.signer(mint, &snap, &member),
            SignerVerdict::Signs(_)
        ));

        // The key at the mint is the artifact's.
        assert_eq!(
            keys.key_at(mint.get()),
            Some(expected),
            "the key at the minting epoch must be the artifact's own"
        );
        // And at a carry epoch above it — where no artifact exists at all — it is
        // still the same one object's, resolved through the chain's mint record.
        let carried = mint.get() + 3;
        assert_eq!(
            keys.minted_at(carried),
            Some(mint.get()),
            "a carry epoch's key epoch is the last epoch that changed"
        );
        assert_eq!(
            keys.key_at(carried),
            Some(expected),
            "and its key is that mint's, byte for byte — one owner, both paths"
        );
    }

    /// The share gate must hold at both samples, not just at the probe:
    /// `share_probe` runs before the caller's boundary-block lookup and
    /// `signer_scheme` after it, so the material can disappear in between.
    ///
    /// Every guard inside `signer_scheme` is written `if let Some(material)`, so on
    /// `None` they all pass vacuously and `build_signer` returns a working
    /// pure-multisig scheme — the node would spawn a participating engine and
    /// subtract itself from the quorum while believing it signs.
    #[test]
    fn a_beacon_active_epoch_with_no_material_is_withheld_and_never_signs_seedless() {
        let epoch = Epoch::new(super::super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH + 1);
        let (snap, member, ..) = signer_fixture(epoch);
        // A provider that resolves no key at all — the state of a member whose mint
        // artifact has not reached it, which is what "no material" now means.
        let r = keyless_provider();

        assert!(
            matches!(
                r.signer(epoch, &snap, &member),
                SignerVerdict::Withheld(WithheldReason::NoUsableShare)
            ),
            "a committee member with no DKG material must be withheld at a beacon-active epoch"
        );

        // The premise the assertion rests on: below the bootstrap epoch the same
        // provider, same absent material, legitimately signs pure-multisig.
        let pre = Epoch::new(super::super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH - 1);
        let (pre_snap, pre_member, ..) = signer_fixture(pre);
        assert!(
            matches!(
                r.signer(pre, &pre_snap, &pre_member),
                SignerVerdict::Signs(_)
            ),
            "a pre-beacon epoch has a legitimate seedless signer — the refusal is about \
             beacon-active epochs only"
        );
    }

    /// A missing key degrades the epoch to vote-only admission, and `ensure_key`
    /// says so honestly: `false` while the mint's artifact is absent, `true` once
    /// it is there, with nothing local able to produce the `true`.
    #[test]
    fn ensure_key_answers_only_for_a_mint_whose_artifact_is_held() {
        let runner = Runner::default();
        runner.start(|_| async move {
            let epoch = Epoch::new(super::super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH + 1);
            let (_snap, _member, _outsider, outcome, _share) = signer_fixture(epoch);

            // Nothing held: both efforts refuse, and `Thorough` has no route wired.
            let keyless = keyless_provider();
            assert!(!keyless.ensure_key(epoch.get(), PinEffort::Local).await);
            assert!(
                !keyless.ensure_key(epoch.get(), PinEffort::Thorough).await,
                "with no acquisition route wired there is no rung left to spend"
            );

            // The mint's artifact held: both efforts answer, and the cheap one does
            // it without a network rung — which is the contract a vote path relies on.
            let held = provider_over(epoch, outcome, None);
            assert!(held.ensure_key(epoch.get(), PinEffort::Local).await);
            assert!(held.ensure_key(epoch.get(), PinEffort::Thorough).await);
        });
    }

    /// The beacon-active rule, at the source. An oracle tells `verify_certificate`
    /// "this epoch is beacon-active", so one on a pre-beacon epoch rejects every
    /// legal seedless certificate there. The soft-enter path and the cert-inlet
    /// both take their oracle from `oracle_for`, so the refusal is asserted here
    /// for all of them.
    #[test]
    fn no_oracle_and_no_key_for_an_epoch_where_randomness_is_not_mandatory() {
        let runner = Runner::default();
        runner.start(|_| async move {
            let epoch = Epoch::new(9);
            let (snap, ..) = signer_fixture(epoch);
            let r = StaticRandomness::build(1, snap);
            let pre = super::super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH - 1;

            // Premise: this provider can answer — otherwise the assertions below
            // hold for an implementation that answers nothing at all.
            let bootstrap = super::super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH;
            assert!(
                r.oracle_for(bootstrap).is_some(),
                "premise: the provider answers at a beacon-active epoch"
            );
            assert!(r.ensure_key(bootstrap, PinEffort::Local).await);

            assert!(r.oracle_for(pre).is_none());
            assert!(!r.ensure_key(pre, PinEffort::Local).await);
            assert!(
                !r.ensure_key(pre, PinEffort::Thorough).await,
                "the rule is about the EPOCH, not about how hard the caller looked"
            );
        });
    }

    /// The other half of the acceptance bar: dropping a no-DKG-no-store
    /// implementation in needs no core file to know it exists. Asserted by where
    /// the type is named, not by a diff: a core file naming the type still fails.
    ///
    /// `testbed/` is exempt as a `#[cfg(test)]` module and the consumer this
    /// substitute exists for.
    #[test]
    fn no_file_outside_the_beacon_names_the_static_implementation() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut inside = 0usize;
        let mut outside: Vec<String> = Vec::new();

        let mut stack = vec![src.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("readable source dir") {
                let path = entry.expect("readable entry").path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).expect("readable source file");
                if !text.contains("StaticRandomness") {
                    continue;
                }
                let rel = path.strip_prefix(&src).expect("under src").to_owned();
                if rel.starts_with("beacon") {
                    inside += 1;
                } else if rel.starts_with("testbed") {
                    // The in-crate deterministic stand (see the doc comment).
                } else {
                    outside.push(rel.display().to_string());
                }
            }
        }

        // Non-vacuity: a typo in the needle, or a walk that visited nothing, would
        // make the real assertion below trivially true.
        assert!(
            inside > 0,
            "premise: the static implementation lives under beacon/ and must be found there"
        );
        assert!(
            outside.is_empty(),
            "a core file names the substitute implementation, so the surface still leaks: {outside:?}"
        );
    }

    /// The spy records here, before anything depends on it, so the repair sweep's
    /// tests can assert which effort was spent rather than only that a key
    /// appeared.
    #[test]
    fn the_test_provider_records_every_key_effort_it_was_asked_for() {
        let runner = Runner::default();
        runner.start(|_| async move {
            let (.., outcome, _share) = signer_fixture(Epoch::new(4));
            let canned = testing::Canned::new().with_mint(4, outcome);

            // 4 is the mint and its artifact is held. 3 is below it, so its own key
            // epoch is the bootstrap mint — which this fixture does not hold.
            assert!(Beacon::ensure_key(&canned, 4, PinEffort::Local).await);
            assert!(!Beacon::ensure_key(&canned, 3, PinEffort::Thorough).await);

            assert_eq!(
                canned.efforts(),
                vec![(4, PinEffort::Local), (3, PinEffort::Thorough)],
                "both calls recorded, in order, with the effort each was given"
            );
        });
    }
}

/// The promote share self-probe — one gate over one resolved sample of the beacon
/// material.
///
/// It publishes nothing and has no value gate: the polynomial this node signs with
/// is the artifact's, so there is no second value to compare against.
///
/// Free fn over its input so the gate cannot drift apart from the sample the
/// scheme is then built from.
fn promote_gates(
    metrics: &BeaconMetrics,
    beacon: Option<&BeaconKey>,
    epoch: Epoch,
) -> Result<(), WithheldReason> {
    // `CombinedScheme::new` asserts only that the share's index equals this node's
    // participant index, never that its value lies on the sharing. While blocks
    // flow a bad share is exposed on the notarize path; in a sustained stall there
    // are no proposals, and since every nullify carries a seed partial and
    // `t == quorum`, one such member makes the nullify quorum unreachable exactly
    // when nullification is the escape hatch.
    //
    // Not redundant with `adopt_share`'s gate: that one runs once, at adoption,
    // over the artifact's polynomial; this one runs at every promote over whatever
    // the store holds now, so a share written by an older binary or edited under a
    // running node reaches here without passing the other.
    if let Some((sharing, Some(share), namespace)) = beacon {
        let probe = Round::new(epoch, View::new(1));
        let partial = beacon_bls::sign_seed_partial(share, namespace, probe);
        if !beacon_bls::verify_seed_partial(sharing, namespace, probe, &partial) {
            metrics.engine_demoted_bad_share.inc();
            warn!(
                ?epoch,
                "resolved DKG share does not verify against its own sharing — \
                 verify-only (promote share-gate)"
            );
            return Err(WithheldReason::BadShare);
        }
    }
    Ok(())
}

/// The live provider: today's handles behind the trait.
///
/// A thin adapter: every method delegates to the function or store that already
/// implements it, so this phase cannot change behaviour.
pub(crate) struct LiveBeacon {
    seeds: super::seed_index::SeedIndex,
    /// The owner of `PK_epoch` and the public polynomial: the chain's mint record
    /// plus the artifact store.
    keys: KeyIndex,
    /// This node's own share per minting epoch, held here and nowhere above: a
    /// scheme reads the live store through the oracle rather than being handed a
    /// copy, so a ceremony that finishes after the scheme was built is picked up on
    /// the next vote instead of at the next epoch.
    ceremony: CeremonyStore,
    /// One bounded peer fetch of a minting epoch's artifact, for
    /// [`Randomness::ensure_key`]'s `Thorough` effort. `None` ⇒ no route (tests).
    acquire: Option<super::artifact::AcquireMint>,
    /// Epochs whose σ refusal has already been reported. Per epoch, because this
    /// runs per certificate.
    reported_refusal: Arc<Mutex<std::collections::BTreeSet<u64>>>,
    metrics: BeaconMetrics,
    chain_id: u64,
    /// The per-epoch artifact store, for [`Beacon::artifact_bytes`] and the
    /// retention on [`Beacon::observe_epoch`]. Held as the store and re-encoded per
    /// call: serving is a rare off-path request, and a second copy of every
    /// artifact in RAM would be paid on every node forever.
    artifacts: ArtifactStore,
    /// The plane's frozen `(dpos_activation, epoch_interval)`, read only to name
    /// the [`WithheldReason::GeometryUnfrozen`] state. `None` means no ceremony has
    /// been able to start yet.
    geometry: watch::Receiver<Option<(u64, u64)>>,
    /// The late-verdict channel, the same type the follower files through.
    faults: LateFaults,
    /// The key want, and only the `--cert-follow` class has one.
    ///
    /// A σ this node cannot check is exactly "I need `PK_epoch` for this epoch", so
    /// [`Randomness::hold_seed`] raises the want. On a validator the epoch manager
    /// already calls `ensure_key(Thorough)` and the acquisition runs off the
    /// certificate path, so the field is `None` and the push is skipped. On a
    /// follower nothing else asks, and this is the only trigger its artifact fetch
    /// task has.
    ///
    /// A full channel drops rather than blocks: the next certificate of the epoch
    /// re-asks a second later. Write-once because [`LiveBeaconConfig`] is built
    /// outside this module — see [`Self::wire_want`].
    want: OnceLock<mpsc::Sender<u64>>,
}

/// Everything [`LiveBeacon::build`] needs, in one value.
///
/// A parameter object rather than nine positions: the four `Option`/`Arc` slots in
/// the middle are type-compatible with each other, so a transposed pair compiles
/// and only shows up as a provider answering from the wrong rung.
pub(crate) struct LiveBeaconConfig {
    pub(crate) seeds: super::seed_index::SeedIndex,
    pub(crate) keys: KeyIndex,
    pub(crate) ceremony: CeremonyStore,
    pub(crate) acquire: Option<super::artifact::AcquireMint>,
    pub(crate) metrics: BeaconMetrics,
    pub(crate) chain_id: u64,
    pub(crate) artifacts: ArtifactStore,
    pub(crate) geometry: watch::Receiver<Option<(u64, u64)>>,
}

impl LiveBeacon {
    /// Concrete on purpose: the plane needs the `Arc<LiveBeacon>` to hand the
    /// promoter its late-verdict sink, and the `Arc<dyn Beacon>` every consumer
    /// holds is one unsize-coercion away.
    pub(crate) fn build(cfg: LiveBeaconConfig) -> Arc<Self> {
        let LiveBeaconConfig {
            seeds,
            keys,
            ceremony,
            acquire,
            metrics,
            chain_id,
            artifacts,
            geometry,
        } = cfg;
        Arc::new(Self {
            seeds,
            keys,
            ceremony,
            acquire,
            reported_refusal: Arc::default(),
            metrics,
            chain_id,
            artifacts,
            geometry,
            faults: LateFaults::new(),
            want: OnceLock::new(),
        })
    }

    /// Wire the key want — see [`LiveBeacon::want`]. Called by
    /// [`super::plane::build_follower`] on the `Arc` it just built and before that
    /// `Arc` is handed to anything, which is why a write-once cell is enough and
    /// why this is not a [`LiveBeaconConfig`] field.
    pub(super) fn wire_want(&self, want: mpsc::Sender<u64>) {
        if self.want.set(want).is_err() {
            // Unreachable through the one caller; loud rather than silent if a
            // second builder ever appears.
            error!("beacon key want is already wired; the second wiring is ignored");
        }
    }

    /// This beacon's counters, for the tests of the builders in [`super::plane`].
    #[cfg(test)]
    pub(super) fn metrics(&self) -> &BeaconMetrics {
        &self.metrics
    }

    /// Re-check every σ the index is holding, now that a key has landed, and file
    /// the late verdict for what fails.
    ///
    /// The `KeyAvailable` edge is its whole trigger: a dedicated task waiting on a
    /// second subscription of the store's notifier raced the wake-up bridge, so a
    /// consumer woken by `KeyAvailable` could re-read the index before the settle
    /// had run. Called from the bridge ahead of the publish, the wake-up now means
    /// "the key landed and the σ it unlocks is filed".
    ///
    /// Per epoch because resolution is per epoch: one `PK_e` settles every round of
    /// its epoch at once, and an epoch whose key still does not resolve stays held.
    pub(crate) fn settle_pending(&self) {
        for epoch in self.seeds.pending_epochs() {
            let Some(oracle) = Randomness::oracle_for(self, epoch) else {
                continue;
            };
            let (promoted, refused) = self.seeds.settle_epoch(epoch, oracle.as_ref());
            if promoted > 0 || refused > 0 {
                tracing::info!(epoch, promoted, refused, "beacon: settled held seeds");
            }
            // The late verdict, on the channel that may not lose it. The error line
            // `settle_epoch` already writes stays: this is the machine-readable half,
            // dropped on the floor until a consumer has taken the receiver.
            if refused > 0 {
                self.faults.report(DataFault { epoch, refused });
            }
        }
    }

    /// One resolve of the material this node may sign `epoch` with: the artifact's
    /// polynomial plus this node's own share, both keyed by the minting epoch the
    /// chain names.
    ///
    /// Private: what is left of it is the promote gate, a decision that never
    /// leaves the plane.
    fn material(&self, epoch: u64) -> Option<BeaconKey> {
        let (minted_at, sharing) = self.keys.sharing_at(epoch)?;
        let share = self.ceremony.read().ok()?.get(&minted_at).cloned();
        Some((
            sharing,
            share,
            beacon_bls::seed_namespace(&fluentbase_bls::fluent_namespace(self.chain_id)),
        ))
    }

    /// The oracle for `epoch`, bound to this node's consensus seat when it has
    /// one. `me = None` is the verifier flavour: it never signs a partial.
    fn oracle_at(&self, epoch: u64, me: Option<Participant>) -> Arc<dyn SeedOracle> {
        Arc::new(BeaconOracle {
            epoch,
            ceremony: self.ceremony.clone(),
            keys: self.keys.clone(),
            namespace: beacon_bls::seed_namespace(&fluentbase_bls::fluent_namespace(self.chain_id)),
            me,
            warned_threshold_mismatch: Arc::new(AtomicBool::new(false)),
            warned_seat_mismatch: Arc::new(AtomicBool::new(false)),
            metrics: self.metrics.clone(),
        })
    }
}

impl Randomness for LiveBeacon {
    /// The index's publisher, not one of this type's own: `SeedIndex::record` is
    /// what fires the seed class, and [`LiveBeacon::settle_pending`] records
    /// through it too.
    fn events(&self) -> &broadcast::Sender<BeaconEvent> {
        self.seeds.events()
    }

    fn record_seed(&self, verified: VerifiedSeed) {
        self.seeds.record(verified);
    }

    /// The `Pending` state, and the key want with it on the class that has one —
    /// see [`LiveBeacon::want`]. Raised on every seed hold, so a follower whose
    /// certificates arrive through the by-height door asks for the key too.
    fn hold_seed(&self, round: Round, seed: BlsSignature) {
        self.seeds.hold(round, seed);
        if let Some(want) = self.want.get() {
            let _ = want.try_send(round.epoch().get());
        }
    }

    /// Latch only, one line per epoch.
    fn first_seed_refusal(&self, epoch: u64) -> bool {
        match self.reported_refusal.lock() {
            Ok(mut seen) => seen.insert(epoch),
            // A poisoned latch must not silence a real witness.
            Err(_) => true,
        }
    }

    fn seed_for(&self, round: Round) -> Option<Seed> {
        self.seeds.seed(round).map(|signature| Seed {
            target_round: round,
            signature,
        })
    }

    fn share_probe(&self, epoch: Epoch) -> ShareProbe {
        // The cheap half of the promote decision: both checks below are store
        // reads, so this can sit ahead of the caller's boundary-block lookup and
        // spare a member that cannot participate a marshal read on every
        // participation edge.
        let material = self.material(epoch.get());
        // The gate is the share, not the polynomial: `material()` resolves the
        // polynomial out of the artifact, which a node can hold without ever having
        // had a share of it — a non-member that acquired the mint, or a member whose
        // share file is gone after a restart. A member with the artifact and no
        // share would pass this gate, spawn a participating engine, and cast votes
        // with no seed partial, subtracting itself from the quorum.
        let signable = material
            .as_ref()
            .is_some_and(|(_, share, _)| share.is_some());
        if Randomness::mandatory_at(self, epoch.get()) && !signable {
            // Unfrozen geometry refines this arm; it does not stand ahead of it. The
            // reason matters and the verdict must not: `NoUsableShare` reads as "the
            // ceremony ran and left me nothing", the wrong story when the plane has
            // not frozen the geometry and no ceremony could have run.
            //
            // Ahead of the material read it would change the verdict: the ceremony
            // store is filled from disk inside `build`, while the geometry watch
            // starts `None` and is published only by the node's poller, so a
            // validator restarted mid-epoch with its share on disk has material here
            // while geometry is still `None`. A withhold at that moment does not
            // self-heal. Reached only with the material absent, the recovery edge is
            // the one that already exists: the ceremony that geometry unblocks fills
            // the store and fires the participation wake-up.
            if material.is_none() && self.geometry.borrow().is_none() {
                self.metrics.engine_demoted_geometry_unfrozen.inc();
                return ShareProbe::Withheld(WithheldReason::GeometryUnfrozen);
            }
            self.metrics.engine_demoted_no_polynomial.inc();
            return ShareProbe::Withheld(WithheldReason::NoUsableShare);
        }
        // The polynomial and the attested key come from one object, so there is no
        // second value to compare.
        ShareProbe::Ready
    }

    fn signer_scheme(
        &self,
        epoch: Epoch,
        snap: &ValidatorSetSnapshot,
        keypair: &ValidatorBlsKeypair,
    ) -> SignerVerdict {
        // The `mandatory_at` gate. Below `DETERMINISTIC_BOOTSTRAP_EPOCH` there is no
        // key to resolve and a pure-multisig signer is legal, so the material has to
        // be `None` there — an oracle attached to a pre-beacon epoch refuses every
        // legal seedless certificate of it. `minted_at` also returns `None` below the
        // bootstrap, so the property holds in two places; removing this gate would
        // lose it for every caller of `oracle_for`, which is the one door.
        let material = Randomness::mandatory_at(self, epoch.get())
            .then(|| self.material(epoch.get()))
            .flatten();
        // The share gate again, over a different sample: the probe answers before
        // the caller's boundary-block lookup and this runs after it, so the material
        // can vanish across that await (an agreement artifact landing makes the
        // carry-divergence guard refuse it, or the ceremony-store prune drops the
        // mint). Every guard below is written `if let Some(material)`, so on `None`
        // they pass vacuously and `build_signer` returns a working pure-multisig
        // scheme; without this check a beacon-active epoch would get
        // `Signs(seedless)` and the node would silently subtract itself from the
        // quorum while believing it signs.
        // The share, for the reason spelled out in `share_probe`: since the
        // polynomial comes from the artifact, it says nothing about whether this node
        // holds a share of it.
        if Randomness::mandatory_at(self, epoch.get())
            && !material
                .as_ref()
                .is_some_and(|(_, share, _)| share.is_some())
        {
            self.metrics.engine_demoted_no_polynomial.inc();
            return SignerVerdict::Withheld(WithheldReason::NoUsableShare);
        }
        // The gates run over one sample, and the scheme is built from the same one —
        // that coherence is why the two are not separate operations.
        if let Err(reason) = promote_gates(&self.metrics, material.as_ref(), epoch) {
            return SignerVerdict::Withheld(reason);
        }
        // A key that lands after the scheme is built is picked up on the next vote,
        // because an oracle reads the store live.
        let committee = match crate::scheme::epoch_committee_from_snapshot(snap) {
            Ok(c) => c,
            Err(e) => return SignerVerdict::InvalidCommittee(e),
        };
        let namespace = fluentbase_bls::fluent_namespace(self.chain_id);
        // The seat has to be known before the oracle is built, because the oracle
        // checks the share's index against it — taking it from the share would make
        // that check compare a value against itself. A probe scheme is how this node
        // asks the committee where it sits.
        let seat = fluentbase_bls::scheme::build_signer(
            &namespace,
            committee.bimap.clone(),
            keypair,
            epoch.get(),
            None,
        )
        .and_then(|probe| {
            use commonware_cryptography::certificate::Scheme as _;
            probe.me()
        });
        let Some(seat) = seat else {
            // Not in the committee's map at all — a misconfiguration safety net
            // whose engine is destroyed at the next reconcile. It still carries the
            // epoch's oracle: verify-only because there is no share, but still
            // checking the seed slot of every certificate of the epoch.
            //
            // This is a producer obligation: `Committee::upgrade_scheme` only checks
            // an occupied scheme slot, and an empty one is filled unconditionally.
            return SignerVerdict::RotatedKey(fluentbase_bls::scheme::build_verifier(
                &namespace,
                committee.bimap,
                epoch.get(),
                Randomness::oracle_for(self, epoch.get()),
            ));
        };
        // `oracle_at`, not `oracle_for`: the seat must ride the oracle here, and
        // `oracle_for` builds the `me: None` flavour. The pre-beacon refusal is
        // inherited from the `mandatory_at` gate above — `material` is `None` below
        // the bootstrap epoch and `then` never fires, so the two must move together.
        let oracle = material
            .is_some()
            .then(|| self.oracle_at(epoch.get(), Some(seat)));
        match fluentbase_bls::scheme::build_signer(
            &namespace,
            committee.bimap,
            keypair,
            epoch.get(),
            oracle,
        ) {
            Some(scheme) => SignerVerdict::Signs(scheme),
            None => unreachable!("the seat probe above already proved membership"),
        }
    }

    fn oracle_for(&self, epoch: u64) -> Option<Arc<dyn SeedOracle>> {
        // The beacon-active rule, enforced here so there is one door instead of
        // three: an oracle means "beacon-active" to `verify_certificate`, so one on
        // a pre-beacon epoch rejects every legal certificate there.
        Randomness::mandatory_at(self, epoch).then(|| self.oracle_at(epoch, None))
    }

    fn ensure_key(&self, epoch: u64, effort: PinEffort) -> BoxFuture<'_, bool> {
        Box::pin(async move {
            // Nothing to acquire below the bootstrap epoch: no key was ever minted
            // there, and `oracle_for` refuses to attach one anyway.
            if !Randomness::mandatory_at(self, epoch) {
                return false;
            }
            // The whole ladder, in two lines: is the mint's artifact local, and — off
            // the vote path only — may we spend one bounded fetch for it.
            let Some(minted_at) = self.keys.minted_at(epoch) else {
                return false;
            };
            if self.keys.holds_mint_of(epoch) == Some(true) {
                return true;
            }
            // The only thing the effort decides: may this call spend a peer
            // round-trip. `Local` callers run against a ~1 s verify budget and the
            // network rung's is seconds.
            let (PinEffort::Thorough, Some(acquire)) = (effort, self.acquire.as_ref()) else {
                return false;
            };
            acquire.fetch(minted_at).await;
            self.keys.holds_mint_of(epoch) == Some(true)
        })
    }

    fn artifact_bytes(&self, epoch: u64) -> Option<Vec<u8>> {
        self.artifacts
            .get(epoch)
            .map(|a| super::artifact::encode_artifact(&a))
    }

    /// The artifact store's one retention owner, on both node classes: the
    /// follower has no `DkgActor` to sweep from, and the frontier the core
    /// reports is the same epoch clock either way.
    fn observe_epoch(&self, _reconciled: Epoch, entered_frontier: Epoch) {
        self.artifacts.retain_mints_for_window(
            entered_frontier.get(),
            crate::SCHEME_RETENTION_EPOCHS as u64,
        );
    }

    fn faults(&self) -> Option<mpsc::UnboundedReceiver<DataFault>> {
        self.faults.take()
    }
}
