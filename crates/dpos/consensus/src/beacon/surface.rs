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
    Arc, Mutex,
};
use tokio::sync::{broadcast, mpsc, watch};
use tracing::{error, warn};

/// Per-epoch threshold material this node holds: the public polynomial, its
/// share (`None` for a verifier-only node) and the seed namespace.
///
/// `pub(crate)` and no wider. It used to be `fluentbase_bls::scheme::BeaconKey`,
/// because the scheme was handed a copy of it; the scheme reads through the
/// oracle now, so what is left of this is the promote gates' input and it never
/// crosses the crate edge.
pub(crate) type BeaconKey = (Sharing<MinSig>, Option<Share>, Vec<u8>);

/// Why participation was withheld. Named rather than a bool because each arm
/// owns its own metric family, and those names are scraped by the devnet smoke.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum WithheldReason {
    NoUsableShare,
    BadShare,
    /// The plane has not frozen `(dpos_activation, epoch_interval)` yet, so no
    /// ceremony has been able to run. Distinct from [`Self::NoUsableShare`], which
    /// it used to be reported as: that one says "the ceremony ran and this node
    /// holds nothing from it", this one says "there has been no ceremony".
    GeometryUnfrozen,
}

/// The cheap half of the participation question, answered where the share-gate
/// sits today — BEFORE the boundary-block lookup, so a shareless member returns
/// without paying a marshal read.
///
/// Deliberately NOT [`SignerVerdict`]: at the probe's position there is no
/// scheme to hand back. The committee has not been checked and the keypair is
/// not one of the probe's inputs.
#[derive(Debug, PartialEq, Eq)]
pub enum ShareProbe {
    Ready,
    Withheld(WithheldReason),
}

/// May this node sign at an epoch, and with what scheme?
pub enum SignerVerdict {
    /// Sign with this scheme. ALSO the answer where [`Randomness::mandatory_at`]
    /// is false: a pre-beacon epoch has a legitimate pure-multisig signer, so
    /// this arm means "may sign", not "beacon-active".
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

/// How much a key acquisition may spend in LATENCY. Nothing else: the provenance
/// floor is the same at both efforts, because it follows from what the key is
/// USED for rather than from how hard the caller was willing to look — see
/// [`Randomness::ensure_key`]'s implementation for the asymmetry that fixes it.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum PinEffort {
    /// Store plus locally held material. No network. The variant a vote path or
    /// a per-certificate path may call.
    Local,
    /// May additionally spend one bounded peer pull. Off-path callers only.
    Thorough,
}

/// The ONE door the consensus core and the node reach the beacon through.
///
/// A trait rather than a struct because the substitution seam is the point: the
/// testbed swaps a node's beacon for a static or withholding one, and production
/// has exactly one implementation. What is NOT on it is as load-bearing as what
/// is — how the epoch key is agreed, where the artifact is stored, how a share is
/// derived, how a peer is served, and every handle those need, all stay behind
/// [`super::build`]'s [`Tasks`](super::Tasks).
///
/// The PRODUCTION implementations (`LiveBeacon`, `FollowerRandomness`) reach it
/// through a blanket impl over [`Randomness`], the module-internal trait they
/// still speak while the internals are moved epoch by epoch
/// (`.dpos-study/PLAN.md` rows 5.1-5.4). The test implementations do NOT: they
/// implement this trait directly, so `Randomness` is reachable from nowhere but
/// `beacon/` and shrinks with the rows that delete it. The one rule they share is
/// [`certificate_verdict`], which every `observe_certificate` routes through.
pub trait Beacon: Send + Sync {
    /// The verified seed in force at `round`, if this node has it.
    ///
    /// SYNC, and every caller depends on it: the executor's derive and the
    /// crash-survivor replay both read it without an await. Answers ONLY for the
    /// exact round asked — the terminal pin is an eviction rule inside the index,
    /// never a substitute round.
    fn seed(&self, round: Round) -> Option<Seed>;

    /// σ of `round`, answered ONLY when `round` is this node's pinned terminal
    /// round for its epoch — the boundary base of the NEXT epoch, which outlives
    /// the round window [`Self::seed`] reads.
    fn terminal_seed(&self, round: Round) -> Option<Seed>;

    /// Is randomness mandatory at `epoch`? CONSENSUS-AGREED DATA, not a local
    /// capability: every node of one network must answer identically or the
    /// derive splits the network.
    fn mandatory_at(&self, epoch: u64) -> bool;

    /// Can this node participate at `epoch` at all?
    ///
    /// The CHEAP probe, and it keeps its own operation rather than folding into
    /// [`Self::signer`]: it is asked on every reconcile edge, ahead of the
    /// boundary-block read, and folding it in would make a shareless member pay a
    /// full resolve plus a marshal read per edge.
    fn can_participate(&self, epoch: Epoch) -> ShareProbe;

    /// May this node sign at `epoch`, and with what scheme?
    fn signer(
        &self,
        epoch: Epoch,
        snap: &ValidatorSetSnapshot,
        keypair: &ValidatorBlsKeypair,
    ) -> SignerVerdict;

    /// The beacon's threshold face for `epoch`, for a scheme that only VERIFIES.
    /// SYNC: consulted inline from the simplex batcher.
    fn oracle_for(&self, epoch: u64) -> Option<Arc<dyn SeedOracle>>;

    /// Try to make `epoch`'s group key locally resolvable, and report whether it
    /// now is. ACQUISITION, not verification — the key never leaves the beacon.
    fn ensure_key(&self, epoch: u64, effort: PinEffort) -> BoxFuture<'_, bool>;

    /// Take the σ a verified certificate carries and return the verdict HERE.
    ///
    /// One operation instead of the four the ingresses used to compose
    /// (`oracle_for` + `VerifiedSeed::check` + `record_seed`/`quarantine_seed` +
    /// `on_invalid_seed`): the verdict rule is the beacon's, and spreading it
    /// over the callers is what let two ingresses judge the same σ differently.
    fn observe_certificate(&self, cert: ObservedCertificate<'_>) -> Observed;

    /// One held artifact's wire bytes, for `consensus_getEpochArtifact`.
    ///
    /// Safe to serve unauthenticated: the artifact is self-authenticating against
    /// `committee[epoch]`, so handing one to anyone leaks nothing a staking read
    /// would not.
    fn artifact_bytes(&self, epoch: u64) -> Option<Vec<u8>>;

    /// The core reconciled `reconciled` while its highest registered epoch is
    /// `entered_frontier`.
    ///
    /// TRANSITIONAL: row 5.1 took its KEY leg (W3, and the key store it pruned);
    /// what is left is the two σ windows, and row 5.2 deletes those. It survives
    /// because the retention it still drives is real.
    fn observe_epoch(&self, reconciled: Epoch, entered_frontier: Epoch);

    /// The cert-inlet ingested a verified certificate for `epoch`. Transitional
    /// for the same reason as [`Self::observe_epoch`], and a SECOND observation
    /// rather than a redundancy: the inlet prunes from its live-upstream
    /// frontier, the epoch manager from its entered one.
    fn observe_cert(&self, epoch: u64);

    /// Wake-ups, not facts: on every one the consumer RE-READS what it needs
    /// through the queries above.
    ///
    /// SUBSCRIBE BEFORE THE FIRST READ, not merely before the loop. A `broadcast`
    /// buffers from the subscription onward and drops a send with no receiver,
    /// where the `notify_one` permits this replaces were stored — so the ordering
    /// that used to be free has to be arranged: subscribe, then read, then wait.
    /// Every consumer already reads before it waits, because the wake-up carries
    /// no value and a re-read is the only way to learn anything from it.
    ///
    /// `RecvError::Lagged` is a wake-up like any other: the consumer re-reads
    /// state, which is all it would have done for each of the messages it missed.
    fn subscribe(&self) -> broadcast::Receiver<BeaconEvent>;

    /// The LATE `Refused` verdict — a σ admitted with no key, refused once the
    /// key landed. A fact rather than a wake-up, so it may not be lost.
    ///
    /// At most one consumer: the first call takes the receiver, later calls get
    /// `None`. Nothing sends until a receiver has been taken, so an unread
    /// channel cannot grow.
    fn faults(&self) -> Option<mpsc::UnboundedReceiver<DataFault>>;
}

/// The certificate an ingress hands to [`Beacon::observe_certificate`].
///
/// The two arms are NOT interchangeable, which is why this is an enum and not a
/// bare `(Round, Option<BlsSignature>)` — but what separates them is the DOOR,
/// not the σ's provenance, and the difference matters:
///
/// - [`Self::Finalization`] is the door that must not DROP a value it could
///   re-check later, so a failure there consults key provenance
///   (`on_invalid_seed`) and may quarantine.
/// - [`Self::Notarization`] is the speculation door. Its σ never reaches the
///   served map on a failure, and there is nothing later to re-check it against
///   at this round, so a failure is logged and refused outright.
///
/// NOT a provenance claim, checked rather than assumed: commonware reports
/// `Activity::Notarization` for a certificate RECEIVED FROM THE WIRE exactly as
/// for one this node assembled. A wire certificate reaches
/// `Message::Verified(Certificate::Notarization(..))`
/// (`simplex/actors/voter/actor.rs:996-1007`), is stored by `handle_notarization`,
/// and is then re-read out of state and reported by `try_broadcast_notarization`
/// (`:500-531`); the journal replay path reports it too (`:745-755`), and
/// `Round::broadcast_notarization` hands back whatever was stored regardless of
/// how it got there (`voter/round.rs:438-447`, whose own test builds the
/// certificate "entirely from remote votes"). The reported activity carries no
/// provenance field, so no caller of this enum can supply one — which is why the
/// arms are named after the doors and the verdict is justified by the door.
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
    /// Verified-INVALID under a key whose provenance makes that an accusation.
    Refused,
    /// Nothing to do: the certificate carries no σ, or the epoch is not
    /// beacon-active.
    Inactive,
}

/// A σ that was admitted with no key and refused once the key landed.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct DataFault {
    pub epoch: u64,
    /// How many quarantined rounds of `epoch` failed the re-check.
    pub refused: usize,
}

/// Which beacon state may have changed. A WAKE-UP: the variant names the class,
/// never the value — the consumer re-reads the value through the queries.
///
/// No payload, and that is the point rather than an omission. Two of the three
/// producers do not have one to give (the artifact store's insert edge and the
/// ceremony's participation edge both fire a bare `Notify` from tasks that do not
/// name an epoch), and a consumer that acted on a payload instead of re-reading would be
/// wrong the moment a `Lagged` collapsed two of them.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum BeaconEvent {
    /// A σ was filed. Consumer: the executor's held-height release.
    SeedRecorded,
    /// A key landed that was not resolvable before. Consumer: the epoch manager's
    /// repair sweep.
    KeyAvailable,
    /// This node's ability to participate may have changed, in EITHER direction.
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

/// The verdict rule of [`Beacon::observe_certificate`], in ONE body.
///
/// Every implementation of the trait routes here: the live plane through the
/// blanket impl over [`Randomness`], and each test fixture from its own
/// `observe_certificate`. The three sinks are closures rather than a second
/// trait because that is all the rule needs from the implementation, and
/// because a fixture whose sink is a no-op should say so at the call site
/// instead of carrying an empty impl block. What must NOT be per-implementation
/// is the rule itself — spreading it over the callers is what let two ingresses
/// judge the same σ differently, and a fixture that judged it by a third rule
/// would make the ingress tests about the fixture.
pub(crate) fn certificate_verdict(
    cert: ObservedCertificate<'_>,
    oracle_for: impl FnOnce(u64) -> Option<Arc<dyn SeedOracle>>,
    record: impl FnOnce(VerifiedSeed),
    quarantine: impl FnOnce(Round, BlsSignature),
    first_refusal: impl FnOnce(u64) -> bool,
) -> Observed {
    // `speculation` names the DOOR, never the σ's provenance — see
    // [`ObservedCertificate`] for why a notarization off the wire arrives here
    // indistinguishable from one assembled locally.
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
        // Not a fault: the epoch key is not resolvable HERE yet. A member
        // normally holds its own epoch key, but there is a window before the
        // key lands where it does not. Hold the value rather than drop it.
        Err(SeedCheck::NoKey) => {
            quarantine(round, seed);
            Observed::Pending
        }
        Err(check @ SeedCheck::Invalid) if speculation => {
            // The speculation door. Refused outright rather than quarantined:
            // nothing later re-checks a notarization's σ at this round, so a held
            // value would only accumulate. `check` is named in the line because it
            // is the only field that would tell this apart from a THIRD failure
            // class if one is ever added to `SeedCheck`. Text kept verbatim from
            // `HEAD:spec_exec.rs:79-84` — the log line is greppable in soaks.
            error!(
                ?round,
                ?check,
                "locally recovered seed did not verify under its own epoch key"
            );
            Observed::Refused
        }
        // OFF THE WIRE, A FAILURE IS NOW ALWAYS A WITNESS, and that is П-3's doing
        // rather than a hardening. It used to consult key PROVENANCE and QUARANTINE
        // on a miss, because the key it failed against could be this node's own
        // reconstruction (`KeySource::LocalDkg`), and an honest σ fails against a
        // diverged local key — accusing the sender there would both punish honest
        // peers and DISCARD a value the attested key could still promote. There is
        // no such tier any more: the only key this rule can fail against is one a
        // `committee[minted_at]` quorum certified, so the failure says something
        // about the SENDER by construction. The unresolvable case keeps its own arm
        // — it is `NoKey`, above, and it is still a hold rather than a drop.
        //
        // Loud ONCE per epoch: this runs per CERTIFICATE, so an unlatched line
        // would emit one a second for the epoch's life.
        Err(SeedCheck::Invalid) => {
            if first_refusal(round.epoch().get()) {
                error!(
                    ?round,
                    "certificate seed does not verify under its epoch key"
                );
            }
            Observed::Refused
        }
        // Unreachable by construction, not by argument:
        // [`VerifiedSeed::check`](crate::beacon::verified_seed::VerifiedSeed::check)
        // maps `SeedCheck::Valid` onto `Ok` and only the other variants onto
        // `Err` (`verified_seed.rs:48-51`), so this arm can be reached only by
        // changing that function. It panics on BOTH certificate paths, which is
        // the shape the finalization ingress already had.
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

    fn terminal_seed(&self, round: Round) -> Option<Seed> {
        Randomness::terminal_seed_at(self, round)
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
            |round, seed| self.quarantine_seed(round, seed),
            |epoch| self.first_seed_refusal(epoch),
        )
    }

    fn artifact_bytes(&self, epoch: u64) -> Option<Vec<u8>> {
        Randomness::artifact_bytes(self, epoch)
    }

    fn observe_epoch(&self, reconciled: Epoch, entered_frontier: Epoch) {
        Randomness::observe_epoch(self, reconciled, entered_frontier)
    }

    fn observe_cert(&self, epoch: u64) {
        Randomness::observe_cert(self, epoch)
    }

    fn subscribe(&self) -> broadcast::Receiver<BeaconEvent> {
        Randomness::events(self).subscribe()
    }

    fn faults(&self) -> Option<mpsc::UnboundedReceiver<DataFault>> {
        Randomness::faults(self)
    }
}

/// The module-internal face the two PRODUCTION implementations still speak.
///
/// NOT the boundary — [`Beacon`] is, and it is blanket-implemented over this.
/// `pub(super)` because `beacon/` is now the whole of its audience: the test
/// implementations moved onto [`Beacon`] itself, so nothing outside this module
/// names it and nothing outside it can. It shrinks with PLAN rows 5.1-5.4 rather
/// than being a second boundary to maintain.
pub(super) trait Randomness: Send + Sync {
    /// Hand over a seed that verified against its epoch key.
    ///
    /// The witness carries its own round, so there is no second argument to get
    /// wrong, and no implementation can be handed a σ nobody checked.
    ///
    /// ORDERING-CRITICAL: the caller invokes this SYNCHRONOUSLY, from inside the
    /// simplex reporter. Sync by signature so no implementation can move the
    /// record behind an await — the voter awaits that reporter before advancing
    /// the view, and the executor derives each height from this memo at the
    /// height's OWN round. (Until FLU-1204 the next view's leader read it to
    /// embed a `parent_seed` witness; nothing embeds anything now, but the
    /// synchronous constraint is unchanged — the derive reads what this wrote.)
    /// `spec_exec.rs` carries the derivation, including which half of the older
    /// constraint turned out not to be load-bearing.
    fn record_seed(&self, verified: VerifiedSeed);

    /// Hold a σ that arrived from the network for an epoch whose key is not
    /// resolvable here yet.
    ///
    /// Separate from [`record_seed`](Self::record_seed) by design: the two take
    /// different types because they mean different things, and no caller can
    /// reach the served map with a value it did not check.
    fn quarantine_seed(&self, round: Round, seed: BlsSignature);

    /// Is this the FIRST σ refusal reported for `epoch`? The latch behind the ERROR
    /// line, and all that is left of the provenance question P-3 removed — see
    /// [`certificate_verdict`]'s `Invalid` arm.
    fn first_seed_refusal(&self, epoch: u64) -> bool;

    /// The seed in force at `round`, if this node has it. Sync: every caller —
    /// the executor's derive and the crash-replay walk — reads it without an
    /// await.
    fn seed_for(&self, round: Round) -> Option<Seed>;

    /// σ of `round`, answered ONLY when `round` is the round this node pinned as
    /// its epoch's terminal one.
    ///
    /// Separate from [`seed_for`](Self::seed_for) because the ask has a different
    /// lifetime: `seed_for` reads the trailing `SEED_RETENTION` window, and the
    /// one legitimate ask that outlives that window is the boundary base for the
    /// NEXT epoch — σ of E-1's terminal round, up to a full epoch old against a
    /// window measured in rounds. The pin is what survives eviction for it.
    ///
    /// The CALLER names the round, from agreed data; a neighbouring round is a
    /// MISS, never a substitute. The pin may not be trusted to name the terminal
    /// round itself — a hard kill between a record and the journal's sync can
    /// leave it one round low, and a valid σ for the wrong round is
    /// indistinguishable from the right one to whoever trusts it to name.
    fn terminal_seed_at(&self, round: Round) -> Option<Seed>;

    /// The wake-up publisher this implementation fires, and the ONE fan-out for
    /// all three classes — see [`Beacon::subscribe`], which is what consumers
    /// hold.
    ///
    /// A `broadcast` rather than the three `Notify` handles it replaces, and for
    /// a reason the `Notify` shape could not give: `notify_one` wakes exactly ONE
    /// waiter, so every consumer needed its own handle from every producer or it
    /// silently swallowed another consumer's wake-up. Adding a second consumer of
    /// any class is now a `subscribe()`, not a new fan-out on the producer.
    fn events(&self) -> &broadcast::Sender<BeaconEvent>;

    /// Is randomness mandatory at `epoch`?
    ///
    /// CONSENSUS-AGREED DATA, not a local capability: every node of one network
    /// must answer identically, or the derive splits the network: this predicate
    /// decides, BEFORE the store is consulted, whether a height derives from σ or
    /// from `None`. (It used to gate the witness-required WIRE rule as well; the
    /// field left the wire in FLU-1204 and the predicate outlived it.) This is the
    /// one operation [`absent`] answers TRUTHFULLY rather than negatively.
    ///
    /// The default body IS that agreed answer — the single deterministic
    /// bootstrap edge every node of this network compiles in. Override it only
    /// if your module owns a DIFFERENT bootstrap edge (today: the test provider,
    /// which parametrises it); never override it to report a local capability,
    /// because that is exactly the split this operation exists to prevent.
    fn mandatory_at(&self, epoch: u64) -> bool {
        epoch >= super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH
    }

    /// Can this node participate at `epoch` at all? See [`ShareProbe`].
    fn share_probe(&self, epoch: Epoch) -> ShareProbe;

    /// May this node sign at `epoch`, and with what scheme?
    ///
    /// Sync, and that is load-bearing rather than incidental: the implementation
    /// publishes this node's key internally before returning, and the returned
    /// scheme is a required input of the engine spawn — so
    /// publish-happens-before-spawn is a DATA DEPENDENCY, not an ordering
    /// convention a refactor can silently break.
    fn signer_scheme(
        &self,
        epoch: Epoch,
        snap: &ValidatorSetSnapshot,
        keypair: &ValidatorBlsKeypair,
    ) -> SignerVerdict;

    /// The beacon's threshold face for `epoch`, for a scheme that only VERIFIES.
    ///
    /// Sync, and it must stay so: the returned oracle is consulted inline from
    /// the simplex batcher, so anything it needs has to be resolvable without an
    /// await. Acquiring what it reads is [`Self::ensure_key`]'s job, off-path.
    ///
    /// CONTRACT, binding on every implementation: this MUST answer `None` where
    /// [`Self::mandatory_at`] is false. An oracle's PRESENCE is what tells
    /// `verify_certificate` "this epoch is beacon-active", so one attached to a
    /// pre-beacon epoch rejects every LEGAL seedless certificate there. This is
    /// the rule the write-once seed pin used to carry, at the same source and for
    /// the same reason — except an oracle is no longer write-once, so the damage
    /// is no longer permanent.
    fn oracle_for(&self, epoch: u64) -> Option<Arc<dyn SeedOracle>>;

    /// Try to make `epoch`'s group key locally resolvable, and report whether it
    /// now is. ACQUISITION, not verification: the value never leaves the beacon
    /// — what reads it is the oracle above.
    ///
    /// Async because [`PinEffort::Thorough`] may spend one bounded peer
    /// round-trip. [`PinEffort::Local`] is contractually network-free — it is
    /// the variant a vote path or a per-certificate path may call.
    fn ensure_key(&self, epoch: u64, effort: PinEffort) -> BoxFuture<'_, bool>;

    /// The core reconciled `reconciled` while its highest registered epoch is
    /// `entered_frontier`.
    fn observe_epoch(&self, reconciled: Epoch, entered_frontier: Epoch);

    /// The cert-inlet ingested a verified certificate for `epoch`.
    ///
    /// A SECOND observation and not a redundancy: the inlet prunes from its own
    /// live-upstream frontier, the epoch manager from its entered frontier, and
    /// each is the right one for its caller. Collapsing them would make the
    /// implementation guess.
    fn observe_cert(&self, epoch: u64);

    /// One held artifact's wire bytes. Defaults to "this provider holds no
    /// artifact store" — the plane and the follower override it, every test
    /// provider takes the default.
    fn artifact_bytes(&self, _epoch: u64) -> Option<Vec<u8>> {
        None
    }

    /// The late-`Refused` channel's receiver, at most once. Defaults to "this
    /// provider never files a late verdict".
    fn faults(&self) -> Option<mpsc::UnboundedReceiver<DataFault>> {
        None
    }
}

/// The total, permanently-negative provider.
///
/// "Not yet" is the wrong answer for a node class whose rungs are absent for the
/// LIFE OF THE PROCESS rather than until something resolves. This turns that
/// contract from prose into a type.
///
/// It no longer describes `--cert-follow`: that class carries keys since FLU-1167
/// and its own `SeedStore` since the follower seed store landed, so it runs
/// `FollowerRandomness` (`crates/dpos/consensus/src/beacon/follower.rs:394`), not this.
/// What is left here is a struct-literal DEFAULT that no consumer observes:
/// `CertInlet::new` has `with_randomness` called on it before first use on every
/// production path, and `FluentApp` no longer carries a randomness handle at all
/// (the field and its setter had no reader and were removed). Plus the
/// executor's test module. Do not read it as naming a live node class.
///
/// Takes a metrics context because registration is context-scoped and commonware
/// prefixes each family with the context's label path — a context-free
/// constructor could not register anything.
#[cfg(test)]
pub(crate) fn absent(ctx: &impl Metrics) -> Arc<dyn Beacon> {
    let metrics = BeaconMetrics::default();
    metrics.register(ctx);
    absent_unregistered()
}

/// The same negative provider without the registration.
///
/// For instances that publish nothing by design — every test, and a
/// [`crate::application::FluentApp`] before its provider is attached. Splitting
/// the registration out is what lets those exist without a metrics context and
/// without silently claiming a second owner for a family.
pub(crate) fn absent_unregistered() -> Arc<dyn Beacon> {
    Arc::new(Absent {
        events: idle_events(),
    })
}

/// THE ACCEPTANCE TEST'S IMPLEMENTATION: randomness with no DKG, no agreement
/// plane, no network and no store.
///
/// The ticket's bar is that an implementation computing the seed from a hash be
/// substitutable here without a single change above this file. This is that
/// implementation, and its existence is the proof — if the surface still leaked
/// a beacon concept, this type could not satisfy it.
///
/// **It hashes into a SHARING, not into each σ, and that is forced rather than
/// stylistic.** `BlsScheme` is a concrete `CombinedScheme`: a seed is a BLS
/// threshold signature that real committee members recover from real partials,
/// so a mock that returned `H(round)` as a "signature" would be rejected by
/// `verify_seed` on the very first vote. What CAN be made static is the DKG's
/// output: seed a deterministic RNG from `H(chain_id ‖ epoch ‖ committee)`, deal
/// once, and every node of that committee derives the identical `PK_epoch` and
/// share set without exchanging a message. Everything downstream is then the
/// SHIPPED crypto, which is what makes this a substitute rather than a bypass.
///
/// Being able to deal means it can also produce σ for any round on demand — sign
/// `t` partials and recover — so `record_seed` is a no-op and `seed_for` needs no
/// memo. That is the "no store" half of the requirement.
#[cfg(test)]
pub(crate) struct StaticRandomness {
    chain_id: u64,
    namespace: Vec<u8>,
    /// The committee this implementation is randomness FOR. A static
    /// implementation legitimately knows it: that is the whole input its `PK` is
    /// derived from, and the surface's own operations do not carry a snapshot on
    /// every call.
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
        // Sorted, so the digest does not depend on snapshot iteration order — the
        // same reason `constant_fallback_seed` sorts.
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

    /// A [`SeedOracle`] over the epoch's DERIVED sharing. Deriving instead of
    /// storing is what makes this provider static; the oracle contract is the
    /// same one the plane's implements.
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
/// A [`SeedOracle`] over one fixed sharing — the material a scheme used to hold
/// itself. Test-only: production reads the live ceremony store through
/// [`BeaconOracle`]. Shared with the cert-inlet's beacon fixtures so there is one
/// test oracle rather than one per test module.
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

    /// The network-wide agreed rule, compiled in exactly as the live beacon's
    /// is. Answering a LOCAL capability here would split the derive.
    fn mandatory_at(&self, epoch: u64) -> bool {
        epoch >= super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH
    }

    fn seed(&self, round: Round) -> Option<Seed> {
        Some(Seed {
            target_round: round,
            signature: self.sigma(round),
        })
    }

    /// No pin to keep: σ is recomputable for any round, so nothing here can age
    /// out of a window and the terminal round answers like every other one.
    fn terminal_seed(&self, round: Round) -> Option<Seed> {
        self.seed(round)
    }

    /// Never withheld: this implementation cannot fail to hold a share.
    fn can_participate(&self, _epoch: Epoch) -> ShareProbe {
        ShareProbe::Ready
    }

    /// The shared verdict rule over an EMPTY sink: nothing to record, because σ
    /// is recomputable for any round, so there is no memo that could go stale
    /// and none that has to be fed; nothing to quarantine for the same reason;
    /// and no key store to judge provenance with, so a failure proves nothing
    /// and the value is held rather than accused.
    fn observe_certificate(&self, cert: ObservedCertificate<'_>) -> Observed {
        certificate_verdict(
            cert,
            |epoch| self.oracle_for(epoch),
            |_verified| {},
            |_round, _seed| {},
            |_epoch| true,
        )
    }

    /// No artifact store: this implementation derives its key and stores
    /// nothing, so there is nothing to serve over `consensus_getEpochArtifact`.
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
        // The share index must equal this node's CONSENSUS participant index —
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
            // Mirrors `LiveBeacon`: a rotated-out node still verifies the
            // epoch's seed slot. A fixture that handed `None` here would model
            // the defect instead of the contract.
            return SignerVerdict::RotatedKey(fluentbase_bls::scheme::build_verifier(
                &ns,
                committee.bimap,
                epoch.get(),
                self.oracle_for(epoch.get()),
            ));
        };
        // UNCONDITIONAL, unlike this type's own `oracle_for` (which refuses below
        // `DETERMINISTIC_BOOTSTRAP_EPOCH`) and unlike `LiveBeacon`'s `Signs`
        // arm (which inherits that refusal through `material`). This fixture can
        // DERIVE material for any epoch, so nothing here fails closed for it.
        //
        // Left unconditional deliberately, not by omission: every driver of this
        // arm signs at a beacon-active epoch, and gating it would make the fixture
        // silently answer `Withheld`-shaped nonsense if one ever did not, where
        // today it would fail loudly on the seedless-certificate assertion. But
        // this type is documented as the proof that the surface does not leak, so
        // it IS what a future reader copies — if you add a pre-beacon driver, gate
        // this on `mandatory_at` rather than discovering it through a red.
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
        // The contract's pre-beacon refusal binds every implementation, not just
        // the one wired to a real plane — and being able to DERIVE a key for any
        // epoch is exactly what would make this one violate it silently.
        self.mandatory_at(epoch)
            .then(|| self.dealt_oracle(epoch, None))
    }

    fn ensure_key(&self, epoch: u64, _effort: PinEffort) -> BoxFuture<'_, bool> {
        // Both efforts answer identically: there is no network rung to spend, and
        // a derivable key is always already resolvable.
        Box::pin(async move { self.mandatory_at(epoch) })
    }

    fn observe_epoch(&self, _reconciled: Epoch, _entered_frontier: Epoch) {}

    fn observe_cert(&self, _epoch: u64) {}
}

/// An empty share store, for the entry point below and for every test provider:
/// none of them carries local DKG material.
#[cfg(test)]
fn keyless_ceremony() -> CeremonyStore {
    Arc::new(std::sync::RwLock::new(std::collections::BTreeMap::new()))
}

/// A [`KeyIndex`] over an empty artifact store and a bit history with nothing set:
/// it resolves the bootstrap mint and finds no artifact for it, which is the
/// "keyless" state every provider below wants.
#[cfg(test)]
pub(crate) fn keyless_index() -> KeyIndex {
    super::artifact::key_index_over(ArtifactStore::new(), &[])
}

/// A provider over a seed store alone, for tests that exercise the executor's
/// two seed operations against a REAL store rather than canned answers.
///
/// It builds the same [`LiveBeacon`] production uses, so a test written against
/// it is testing the shipped `seed` / `subscribe`, not a stub that happens to
/// agree with them today.
#[cfg(test)]
pub(crate) fn for_seeds(seeds: super::certify::SeedStore) -> Arc<dyn Beacon> {
    LiveBeacon::build(LiveBeaconConfig {
        seeds,
        keys: keyless_index(),
        ceremony: keyless_ceremony(),
        acquire: None,
        metrics: BeaconMetrics::default(),
        chain_id: 0,
        artifacts: ArtifactStore::new(),
        geometry: watch::channel(Some((0, 1))).1,
    })
}

/// Holds no store, and that is the honest shape. It carried a `BeaconKeys` whose
/// doc said the follower "keeps ONE key store for its ladder and its retention" —
/// but this provider's `ensure_key` never reads a store and no writer can reach one,
/// so the field's only user was its own retention call: pruning a map nothing
/// fills, on behalf of a ladder with no rungs. Removed rather than kept for
/// symmetry, because the doc read as an instruction.
struct Absent {
    events: broadcast::Sender<BeaconEvent>,
}

impl Beacon for Absent {
    fn subscribe(&self) -> broadcast::Receiver<BeaconEvent> {
        self.events.subscribe()
    }

    /// The one operation this type answers TRUTHFULLY rather than negatively:
    /// beacon-activity is network-wide agreed data — it decides whether a height
    /// derives from σ or from `None` — so answering it negatively would split the
    /// network rather than degrade this node.
    fn mandatory_at(&self, epoch: u64) -> bool {
        epoch >= super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH
    }

    fn seed(&self, _round: Round) -> Option<Seed> {
        None
    }

    fn terminal_seed(&self, _round: Round) -> Option<Seed> {
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

    /// `Inactive` at every epoch, and through the shared rule rather than beside
    /// it: with no oracle anywhere, the rule's own second gate is what answers,
    /// so this cannot drift from what the ingress does with a σ it holds no
    /// opinion about. The sinks below it are unreachable for the same reason.
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

    fn observe_epoch(&self, _reconciled: Epoch, _entered_frontier: Epoch) {}

    /// Nothing to prune: there is no store, because nothing could ever fill one.
    fn observe_cert(&self, _epoch: u64) {}
}

/// Test provider. One place for every migrated test to get a [`Beacon`],
/// so call sites stop inventing a stub each.
///
/// It is also the SPY: it records the `(epoch, effort)` of every
/// [`Beacon::ensure_key`] call, which is what lets the repair sweep's tests
/// assert which effort was spent rather than only that a key appeared.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use crate::beacon::{artifact::MintFixture, oracle::KeyOnlyOracle};
    use std::{collections::BTreeMap, sync::Mutex};

    pub(crate) struct Canned {
        /// Canned mints, reachable through [`Beacon::oracle_for`] exactly as a real
        /// one is: a real [`KeyIndex`] over a real artifact store.
        mints: MintFixture,
        seeds: BTreeMap<Round, Seed>,
        bootstrap: u64,
        efforts: Mutex<Vec<(u64, PinEffort)>>,
        events: broadcast::Sender<BeaconEvent>,
        /// A real store, so a test can assert WHERE a captured σ landed —
        /// served or held — instead of only that the call happened.
        store: crate::beacon::certify::SeedStore,
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
                store: crate::beacon::certify::SeedStore::new(),
                seed_namespace: Vec::new(),
            }
        }

        /// State a MINT rather than a bare key: after П-3 the key is a projection of
        /// the epoch's artifact, so "pin this `GroupPublic` at this epoch" is not a
        /// state a node can be in and the fixture may not offer it.
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

        pub(crate) fn store(&self) -> &crate::beacon::certify::SeedStore {
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

        /// Reads the real `store` — the map [`Beacon::observe_certificate`]
        /// writes — so a test that records a σ and asks for the pin gets it back.
        fn terminal_seed(&self, round: Round) -> Option<Seed> {
            self.store.terminal_at(round).map(|signature| Seed {
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

        fn oracle_for(&self, epoch: u64) -> Option<Arc<dyn SeedOracle>> {
            self.mandatory_at(epoch).then(|| {
                Arc::new(KeyOnlyOracle {
                    epoch,
                    keys: self.mints.keys.clone(),
                    namespace: self.seed_namespace.clone(),
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

        /// The shared verdict rule over the REAL stores this fixture carries: a
        /// recorded σ lands in `store`'s served map, a held one in its quarantine.
        /// That is what lets an ingress test assert WHERE the σ went rather than only
        /// that the call happened. The provenance question the third sink used to
        /// route through a key store is gone (П-3) — every refusal is a witness.
        fn observe_certificate(&self, cert: ObservedCertificate<'_>) -> Observed {
            certificate_verdict(
                cert,
                |epoch| self.oracle_for(epoch),
                |verified| self.store.record(verified),
                |round, seed| self.store.quarantine(round, seed),
                |_epoch| true,
            )
        }

        fn artifact_bytes(&self, _epoch: u64) -> Option<Vec<u8>> {
            None
        }

        fn faults(&self) -> Option<mpsc::UnboundedReceiver<DataFault>> {
            None
        }

        fn observe_epoch(&self, _reconciled: Epoch, _entered_frontier: Epoch) {}

        fn observe_cert(&self, _epoch: u64) {}
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
            // data — it decides whether a height derives from σ or from `None` —
            // so answering it negatively would split the network rather than
            // degrade this node.
            assert!(!r.mandatory_at(super::super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH - 1));
            assert!(r.mandatory_at(super::super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH));
        });
    }

    /// A committee snapshot with real BLS keypairs, plus a REAL DKG outcome dealt
    /// over its own peer set. Returns the keypair at index 0 (a member) and one that
    /// is NOT in the committee, so both the signing and the rotated-out arms are
    /// reachable.
    ///
    /// It deals an `Output` rather than an anonymous `Sharing`, and that is П-3 in
    /// the fixture: the polynomial a node signs with is now a projection of the
    /// epoch's ARTIFACT, and an artifact carries an `Output`. A fixture that could
    /// only produce a bare `Sharing` could not state the state under test.
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

        // Dealt over the committee's OWN peer set, so each player's share index is
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

    /// A live beacon whose key index answers for `epoch` out of a REAL artifact, and
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
            seeds: super::super::certify::SeedStore::new(),
            keys: mints.keys.clone(),
            ceremony,
            acquire: None,
            metrics: BeaconMetrics::default(),
            chain_id: 1,
            artifacts: mints.artifacts.clone(),
            geometry: watch::channel(Some((0, 1))).1,
        })
    }

    /// A live beacon that resolves NO key: the state of a node whose mint artifact
    /// has not reached it.
    fn keyless_provider() -> Arc<dyn Beacon> {
        LiveBeacon::build(LiveBeaconConfig {
            seeds: super::super::certify::SeedStore::new(),
            keys: keyless_index(),
            ceremony: keyless_ceremony(),
            acquire: None,
            metrics: BeaconMetrics::default(),
            chain_id: 1,
            artifacts: ArtifactStore::new(),
            geometry: watch::channel(Some((0, 1))).1,
        })
    }

    /// THE INVERSE OF W1'S OLD GUARANTEE, and the property П-3 needs from this side:
    /// handing out a SIGNING scheme writes nothing anywhere.
    ///
    /// The previous version asserted the opposite — that `PK_epoch` was in a key
    /// store by the time the scheme came back — which was W1 publishing this node's
    /// own reconstruction under `KeySource::LocalDkg`. That write is what made "the
    /// key I rebuilt" and "the key the network attested" two possible values of one
    /// entry. With W1 gone the store has no locally-derived writer at all, so the
    /// property worth pinning is that the signer path is READ-ONLY.
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
            seeds: super::super::certify::SeedStore::new(),
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

    /// A ROTATED-OUT NODE STILL JUDGES THE EPOCH'S SEED SLOT, and this is a
    /// producer obligation that no downstream guard can cover for.
    ///
    /// `Committee::upgrade_scheme`'s three refusals all need an OCCUPIED scheme
    /// slot; an empty one is filled unconditionally. The committee module fills
    /// it with its own beacon-active verifier the moment the epoch is read, so
    /// today an oracle-less verdict here would be REFUSED rather than landed —
    /// but that is a property of the module's install ordering, not of this
    /// producer, and the producer is what this test pins. Land an oracle-less
    /// scheme in an empty slot and it stays for the whole window, admitting every
    /// epoch-E certificate whose seed slot was cleared while
    /// `repair_keyless_schemes` resolves the key and reports the epoch as
    /// upgraded.
    ///
    /// The earlier "exact parity with today's `beacon: None`" reasoning held only
    /// for the attestation arm. On the certificate arm the pre-FLU-1202 scheme
    /// was REPAIRABLE — `apply_pin` attached `PK_epoch` to it later — and nothing
    /// repairs a scheme now, so parity means reproducing the POST-pin state:
    /// verify-only, but seed-checking.
    ///
    /// Reds if the `RotatedKey` arm goes back to `None`.
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
    /// against its own sharing must report `BadShare`, never the value gate's
    /// `KeyDivergence`.
    #[test]
    fn a_share_that_does_not_verify_reports_bad_share() {
        let epoch = Epoch::new(9);
        let (snap, member, _outsider, outcome, _share) = signer_fixture(epoch);
        // The epoch's REAL outcome, paired with a share from an unrelated deal at the
        // same index: the polynomial is right, the share's value is not on it. That
        // is the one shape `adopt_share`'s gate cannot have caught — a share file
        // written by an older binary, or edited under a running node.
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

    /// THE ACCEPTANCE TEST for this ticket.
    ///
    /// Drives the whole randomness contract — the σ a certificate carries in,
    /// the store lookup a derive reads out, the signing scheme, and the
    /// certificate pin — against an implementation that has no DKG, no agreement
    /// plane, no network and no store. If any of those operations still required
    /// a beacon concept, this type could not exist and this test could not
    /// compile.
    ///
    /// Not a mock returning canned answers: every value below is real BLS,
    /// produced by the shipped crypto. What is static is only the DEAL — see
    /// `StaticRandomness`'s doc for why hashing into a sharing is forced rather
    /// than chosen.
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

        // Verify: another node checks that seed under the key in force, and
        // agrees — the two halves derived the same PK from the same committee
        // without exchanging anything. Asked through `oracle_for`, which is the
        // seam that survived the witness gate: it is what the certificate door
        // checks a carried σ with.
        let oracle = r
            .oracle_for(epoch.get())
            .expect("a beacon-active epoch carries an oracle");
        assert!(crate::beacon::verified_seed::VerifiedSeed::check(
            oracle.as_ref(),
            round,
            seed.signature
        )
        .is_ok());

        // A DIFFERENT round's σ must not verify as this one's. Without this the
        // Ok above would also pass for an implementation that ignored the round
        // entirely.
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

    /// THE DIVERGENCE CLASS IS GONE BY CONSTRUCTION, and this is what replaced the
    /// two tests that used to pin its handling.
    ///
    /// They were `material_diverging_from_the_agreed_key_is_withheld_from_signing`
    /// and `the_participation_probe_reports_a_divergence_that_appears_after_the_spawn`:
    /// both staged a key store holding a quorum-attested `PK_E` that DIFFERED from
    /// the polynomial the node had reconstructed, and asserted the value gate
    /// demoted on it. Staging that state is no longer possible — the polynomial and
    /// the attested key are read from ONE object, the artifact of the epoch the
    /// chain says minted the key (`KeyIndex`), so there is no second value to
    /// disagree with. `WithheldReason::KeyDivergence` and its counter are deleted
    /// rather than left unreachable.
    ///
    /// What is still assertable, and what this asserts, is the IDENTITY: the key the
    /// probe and the signer judge by is byte-for-byte the artifact's, for the same
    /// epoch, on both the exact-mint and the CARRIED path — because if those two
    /// ever came from different places the class would be back.
    ///
    /// Reds if `oracle_for`'s key stops being the artifact's, and reds if a carry
    /// epoch resolves to anything but its mint's key.
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
            seeds: super::super::certify::SeedStore::new(),
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

        // The key at the MINT is the artifact's.
        assert_eq!(
            keys.key_at(mint.get()),
            Some(expected),
            "the key at the minting epoch must be the artifact's own"
        );
        // And at a CARRY epoch above it — where no artifact exists at all — it is
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

    /// The share gate must hold at BOTH samples, not just at the probe.
    ///
    /// `share_probe` runs before the caller's boundary-block lookup and
    /// `signer_scheme` after it, so the material can disappear in between — an
    /// agreement artifact landing makes the carry-divergence guard refuse it, or
    /// the ceremony-store prune drops the mint. Every guard inside
    /// `signer_scheme` is written `if let Some(material)`, so on `None` they all
    /// pass vacuously and `build_signer` returns a working PURE-MULTISIG scheme.
    ///
    /// The verdict for a beacon-active epoch would then be `Signs`, the node
    /// would spawn a participating engine, and every vote it cast would be
    /// rejected for the missing seed partial — subtracting itself from the quorum
    /// while believing it signs, with no demote counter moving.
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

        // The premise the assertion rests on: BELOW the bootstrap epoch the same
        // provider, same absent material, legitimately signs pure-multisig. Without
        // this the assertion above would also hold for a provider that refuses
        // everything.
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

    /// THERE IS NO PROVENANCE FLOOR LEFT TO TEST, and this is what replaced the test
    /// of it.
    ///
    /// `no_effort_takes_a_locally_reconstructed_key` asserted that neither effort
    /// would answer from a `KeySource::LocalDkg` store entry while `Carried` was
    /// admitted — a rule that existed because the key store MIXED a locally
    /// reconstructed tier with attested ones. `KeyIndex` has one tier by construction
    /// (П-3: the only source is an artifact a `committee[minted_at]` quorum
    /// certified), so the floor has nothing to exclude and the three-rung ladder it
    /// guarded collapsed to a local probe plus one bounded fetch.
    ///
    /// What survives as a property, and what this asserts, is the ASYMMETRY the floor
    /// existed for: a missing key degrades the epoch to vote-only admission, and
    /// `ensure_key` says so honestly — `false` while the mint's artifact is absent,
    /// `true` once it is there, with no third answer in between and nothing local
    /// able to produce the `true`.
    ///
    /// Reds if `ensure_key` starts answering `true` without an artifact.
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

    /// The beacon-active rule, at the source. An ORACLE tells
    /// `verify_certificate` "this epoch is beacon-active", so one on a PRE-beacon
    /// epoch rejects every legal seedless certificate there.
    ///
    /// The soft-enter path and the cert-inlet both take their oracle from
    /// `oracle_for`, so asserting the refusal here asserts it for all of them.
    #[test]
    fn no_oracle_and_no_key_for_an_epoch_where_randomness_is_not_mandatory() {
        let runner = Runner::default();
        runner.start(|_| async move {
            let epoch = Epoch::new(9);
            let (snap, ..) = signer_fixture(epoch);
            let r = StaticRandomness::build(1, snap);
            let pre = super::super::actor::DETERMINISTIC_BOOTSTRAP_EPOCH - 1;

            // Premise: this provider CAN answer — otherwise the assertions below
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

    /// The other half of the acceptance bar: substituting the implementation
    /// required no change ABOVE the surface.
    ///
    /// Asserted by where the type is NAMED, not by a diff. A diff against `HEAD`
    /// cannot express this claim — this ticket deliberately rewrites the core to
    /// create the surface, so of course files outside `beacon/` changed. What the
    /// bar is actually about is narrower and checkable: dropping a
    /// no-DKG-no-store implementation in needs no core file to know it exists.
    ///
    /// (The first version of this test DID diff against `HEAD`, with a pathspec
    /// that silently matched nothing because it was resolved against the manifest
    /// directory. Its own non-vacuity assertion is what caught it, which is the
    /// argument for writing that assertion.)
    ///
    /// `testbed/` is exempt: it is a `#[cfg(test)]` module, not a core file, and
    /// it is the consumer this substitute exists for (N nodes promoted to
    /// `Signer` with no DKG plane — research E3-2 §2 #4). A core file naming the
    /// type still fails.
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

    /// The spy is what lets the repair sweep's tests assert WHICH effort was
    /// spent rather than only that a key appeared. Prove it records here, before
    /// anything depends on it.
    #[test]
    fn the_test_provider_records_every_key_effort_it_was_asked_for() {
        let runner = Runner::default();
        runner.start(|_| async move {
            let (.., outcome, _share) = signer_fixture(Epoch::new(4));
            let canned = testing::Canned::new().with_mint(4, outcome);

            // 4 is the mint and its artifact is held. 3 is BELOW it, so its own key
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

/// The promote SHARE self-probe — one gate over ONE resolved sample of the beacon
/// material.
///
/// # It publishes nothing, and there is no value gate left either (П-3)
///
/// Two things used to live here and both are gone with the second owner that
/// created them. **W1** wrote this node's own reconstruction into a `BeaconKeys`
/// store under `KeySource::LocalDkg` once per epoch ENTERED (W3 did the same for
/// `E−1` off the reconcile edge), which is what made "the key I rebuilt" and "the
/// key the network attested" two possible values of one entry. **The value gate**
/// then compared the two and demoted on a mismatch. With [`KeyIndex`] the
/// polynomial this node signs with IS the artifact's, so the comparison has no two
/// sides: `WithheldReason::KeyDivergence` and the
/// `epoch_engine_demoted_key_divergence_total` counter that watched it are deleted
/// rather than left unreachable — the counter one pass later than the arm, because
/// removing the only `inc` site left the family REGISTERED and pinned at zero,
/// which an operator reads as "this never happens".
///
/// What a member loses by W1's removal: nothing it used to have. Its epoch key is
/// the artifact its own agreement produced, which is in the store before the
/// finalize that yields the share (`DkgActor::on_artifact`), and the mint memo
/// addresses it without a chain read after the first resolve.
///
/// Free fn over its input so the gate cannot drift apart from the sample the scheme
/// is then built from.
fn promote_gates(
    metrics: &BeaconMetrics,
    beacon: Option<&BeaconKey>,
    epoch: Epoch,
) -> Result<(), WithheldReason> {
    // Promote-gate SHARE check. `CombinedScheme::new` asserts only that
    // the share's INDEX equals this node's participant index — never
    // that its VALUE lies on the sharing. While blocks flow, a bad share
    // is exposed on the notarize path; in a sustained stall there are no
    // proposals, so it is not, and since every Nullify now carries a seed
    // partial and `t == quorum`, one such member on the plane makes the
    // nullify quorum unreachable exactly when nullification is the escape
    // hatch.
    //
    // IT IS NOT REDUNDANT WITH `adopt_share`'s GATE, and the two are not the same
    // check. That one runs once, at adoption, over the artifact's polynomial; this
    // one runs at every promote over whatever the store holds NOW — a share written
    // by an older binary, or a file edited under a running node, reaches here
    // without ever passing the other.
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
/// Deliberately a THIN adapter — every method delegates to the function or store
/// that already implements it. Nothing is reimplemented here, so this phase
/// cannot change behaviour; the bodies relocate in later phases, once this is
/// their only caller.
pub(crate) struct LiveBeacon {
    seeds: super::certify::SeedStore,
    /// The OWNER of `PK_epoch` and the public polynomial: the chain's mint record
    /// plus the artifact store (П-3). It replaces a `BeaconKeys` store, a
    /// `BeaconResolver` closure over the ceremony store, a `DkgQualFor` and the two
    /// `AgreedKeys` ladder rungs — five handles for one fact.
    keys: KeyIndex,
    /// This node's own share per minting epoch, held HERE and nowhere above: a
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
    /// The per-epoch artifact store, for [`Beacon::artifact_bytes`] alone. Held
    /// as the store and re-encoded per call rather than kept as bytes: serving is
    /// a rare off-path request (a follower asks once per epoch it lacks), and a
    /// second copy of every artifact in RAM to save it would be paid for on every
    /// node forever.
    artifacts: ArtifactStore,
    /// The plane's frozen `(dpos_activation, epoch_interval)`, read ONLY to name
    /// the [`WithheldReason::GeometryUnfrozen`] state. `None` ⇒ no ceremony has
    /// been able to start yet, which is a reason worth a metric of its own rather
    /// than the `NoUsableShare` it used to be indistinguishable from.
    geometry: watch::Receiver<Option<(u64, u64)>>,
    /// The late-verdict channel. `armed` gates the SEND, so an unread channel
    /// cannot grow: nothing is queued until a consumer has taken the receiver.
    faults_tx: mpsc::UnboundedSender<DataFault>,
    faults_rx: Mutex<Option<mpsc::UnboundedReceiver<DataFault>>>,
    faults_armed: AtomicBool,
}

/// Everything [`LiveBeacon::build`] needs, in one value.
///
/// One field per handle the provider holds, named after it. A parameter object
/// rather than nine positions: the four `Option`/`Arc` slots in the middle are
/// type-compatible with each other, so a transposed pair compiles and only shows
/// up as a provider that silently answers from the wrong rung.
pub(crate) struct LiveBeaconConfig {
    pub(crate) seeds: super::certify::SeedStore,
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
        let (faults_tx, faults_rx) = mpsc::unbounded_channel();
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
            faults_tx,
            faults_rx: Mutex::new(Some(faults_rx)),
            faults_armed: AtomicBool::new(false),
        })
    }

    /// File a late `Refused` verdict. Dropped on the floor until a consumer has
    /// taken the receiver — see [`Beacon::faults`].
    pub(crate) fn report_fault(&self, fault: DataFault) {
        if !self.faults_armed.load(Ordering::Relaxed) {
            return;
        }
        let _ = self.faults_tx.send(fault);
    }

    /// One resolve of the material this node may SIGN `epoch` with: the artifact's
    /// polynomial plus this node's own share, both keyed by the minting epoch the
    /// chain names.
    ///
    /// PRIVATE, and it stays private: what is left of it is the promote GATE, which
    /// needs the polynomial and the share to judge this node's own fitness — a
    /// decision that never leaves the plane. It replaces `resolve::beacon_share_resolver`,
    /// whose carry arbitration, refusal metrics and divergence guard all collapsed
    /// into [`KeyIndex`]: one owner cannot disagree with itself, and "this node holds
    /// no mint at the chain's key epoch" is now simply a `None` here.
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
    /// The store's, not one of this type's own: `SeedStore::record` is what fires
    /// the seed class, and the promoter records through it too. The plane's bridge
    /// sends the other two classes into this same publisher.
    fn events(&self) -> &broadcast::Sender<BeaconEvent> {
        self.seeds.events()
    }

    fn record_seed(&self, verified: VerifiedSeed) {
        self.seeds.record(verified);
    }

    fn quarantine_seed(&self, round: Round, seed: BlsSignature) {
        self.seeds.quarantine(round, seed);
    }

    /// Latch only, one line per epoch. The provenance question this used to route
    /// through `BeaconKeys` is gone — see [`certificate_verdict`]'s `Invalid` arm.
    fn first_seed_refusal(&self, epoch: u64) -> bool {
        match self.reported_refusal.lock() {
            Ok(mut seen) => seen.insert(epoch),
            // A poisoned latch must not silence a real witness.
            Err(_) => true,
        }
    }

    fn seed_for(&self, round: Round) -> Option<Seed> {
        self.seeds.lookup(round).map(|signature| Seed {
            target_round: round,
            signature,
        })
    }

    /// The pin alone, with no fall-through to the served window: the pin is
    /// written from the same `insert` that fills the window, highest round per
    /// epoch wins, and no round of a CLOSED epoch can exceed its terminal one —
    /// so a σ the window holds for the asked round is already the pinned one.
    fn terminal_seed_at(&self, round: Round) -> Option<Seed> {
        self.seeds.terminal_at(round).map(|signature| Seed {
            target_round: round,
            signature,
        })
    }

    fn share_probe(&self, epoch: Epoch) -> ShareProbe {
        // The CHEAP half of the promote decision: both checks below are store
        // reads, so this can sit ahead of the caller's boundary-block lookup and
        // spare a member that cannot participate a marshal read on every
        // participation edge.
        //
        let material = self.material(epoch.get());
        // THE GATE IS THE SHARE, NOT THE POLYNOMIAL, and П-3 is what made the two
        // separable. `material()` resolves the polynomial out of the ARTIFACT
        // ([`KeyIndex`]), which a node can hold without ever having had a share of
        // it — a non-member that acquired the mint (R-121/R-122), or a member whose
        // share file is gone after a restart. Before П-3 the material came from the
        // ceremony store, so `Some(..)` implied a share and `material.is_none()` was
        // the whole test; keyed on the artifact it no longer is, and a member with
        // the artifact and no share would pass this gate, spawn a PARTICIPATING
        // engine, cast votes with no seed partial and subtract itself from the
        // quorum while believing it signs — the wedge the gate exists to prevent
        // (measured before this line: `restart_without_the_share_dirs_parks_the_chain`
        // parked the chain with `engine_demoted_no_polynomial = 0`).
        let signable = material
            .as_ref()
            .is_some_and(|(_, share, _)| share.is_some());
        if Randomness::mandatory_at(self, epoch.get()) && !signable {
            // UNFROZEN GEOMETRY REFINES THIS ARM; it does not stand ahead of it.
            //
            // The reason matters and the verdict must not: `NoUsableShare` reads
            // as "the ceremony ran and left me nothing", which is the wrong story
            // when the plane has not frozen `(activation, interval)` and no
            // ceremony could have run at all. So it is a second name for the SAME
            // withholding, chosen only where the material is already absent.
            //
            // AHEAD OF THE MATERIAL READ IT WOULD CHANGE THE VERDICT, and that is
            // not hypothetical: `share_state::load_all` fills the ceremony store
            // from disk inside `build` (`beacon/plane.rs`), before this type
            // exists and with no dependence on geometry, while the geometry watch
            // starts `None` and is published only by the node's poller after a
            // successful cold start (`node/src/dpos.rs`). A validator restarted
            // mid-epoch with its share on disk therefore has `Key(..)` here while
            // geometry is still `None` — and a `Withheld` at that moment does not
            // self-heal: `reconcile_roles` soft-enters and returns without adding
            // to `deferred_spawns`, so the `spawn_unblocked` edge stays gated off,
            // and NOTHING publishes a participation wake-up when geometry thaws.
            // The node would sit verify-only until the next epoch boundary.
            //
            // Reached only with the material absent, the recovery edge is the one
            // that already exists: the ceremony that geometry unblocks fills the
            // store and fires `share_notify`, which the plane's bridge turns into
            // `ParticipationChanged`.
            // On `material.is_none()` only: an artifact in hand is proof that a
            // ceremony DID run, so "no ceremony could have run yet" would be a
            // false story for a node that holds the mint and lost the share.
            if material.is_none() && self.geometry.borrow().is_none() {
                self.metrics.engine_demoted_geometry_unfrozen.inc();
                return ShareProbe::Withheld(WithheldReason::GeometryUnfrozen);
            }
            self.metrics.engine_demoted_no_polynomial.inc();
            return ShareProbe::Withheld(WithheldReason::NoUsableShare);
        }
        // THE VALUE CHECK THAT USED TO SIT HERE IS GONE, and it is П-3 that removed
        // it rather than a simplification. It compared the polynomial this node
        // resolved against the key a quorum had attested, and revisited the promote
        // decision when the two disagreed. They come from ONE object now
        // (`KeyIndex`: the artifact of the epoch the chain says minted the key), so
        // there is no second value to disagree with — the state the check watched
        // for is unreachable, not merely unobserved.
        ShareProbe::Ready
    }

    fn signer_scheme(
        &self,
        epoch: Epoch,
        snap: &ValidatorSetSnapshot,
        keypair: &ValidatorBlsKeypair,
    ) -> SignerVerdict {
        // THE `mandatory_at` GATE, AND IT IS NOW THE ONLY ROAD. Below
        // `DETERMINISTIC_BOOTSTRAP_EPOCH` there is no key to resolve and a
        // pure-multisig signer is LEGAL, so the material has to be `None` there —
        // an oracle attached to a pre-beacon epoch refuses every legal seedless
        // certificate of it.
        //
        // That `None` used to ALSO arrive on its own, through
        // `carry::chain_key_epoch_memoised`'s `Some(None)` arm making the share
        // resolver answer `Absent`. The two-layer `Option` is DELETED
        // (`MintIndex::minted_at` is flat now), and this gate — added in 5.0 for
        // exactly that reason — is what STATES the property here instead of
        // inheriting it. The belt did not disappear, it MOVED: `minted_at` returns
        // `None` unconditionally below `DETERMINISTIC_BOOTSTRAP_EPOCH`
        // (`artifact.rs`, first lines of `MintIndex::minted_at`), so the material
        // cannot become `Some` below the bootstrap epoch even with this gate
        // removed. Two independent places, and the mistake to avoid is the opposite
        // one: removing `mandatory_at` here loses the property for EVERY caller of
        // `oracle_for`, which is where the one door lives.
        let material = Randomness::mandatory_at(self, epoch.get())
            .then(|| self.material(epoch.get()))
            .flatten();
        // THE SHARE GATE, AGAIN — and it has to be here, not only in
        // `share_probe`. The two run over DIFFERENT samples: the probe answers
        // before the caller's boundary-block lookup, this runs after it, and the
        // material can vanish across that await (an agreement artifact landing
        // makes the carry-divergence guard refuse it; the ceremony-store prune
        // can drop the mint).
        //
        // Every guard below is written `if let Some(material)`, so on `None` all
        // three pass VACUOUSLY — and `build_signer` with no beacon material
        // happily returns a working PURE-MULTISIG scheme. Without this check the
        // verdict for a beacon-active epoch would be `Signs(seedless)`: the node
        // spawns a participating engine, every vote it casts carries no seed
        // partial, peers reject the whole vote, and it silently subtracts itself
        // from the quorum while believing it is signing. That is exactly the
        // wedge the share gate exists to prevent, re-entered through the back
        // door — and no `engine_demoted_*` counter would move.
        //
        // Regression note: HEAD took ONE sample before the await and fed it to
        // both the gate and the scheme, so this could not arise. Splitting the
        // operation is what created the second sample; the split's own comment
        // reasoned about the Absent→Key direction only.
        // The SHARE, for the reason spelled out in `share_probe`: since П-3 the
        // polynomial comes from the artifact and says nothing about whether THIS
        // node holds a share of it.
        if Randomness::mandatory_at(self, epoch.get())
            && !material
                .as_ref()
                .is_some_and(|(_, share, _)| share.is_some())
        {
            self.metrics.engine_demoted_no_polynomial.inc();
            return SignerVerdict::Withheld(WithheldReason::NoUsableShare);
        }
        // The gates run over ONE sample, and the scheme is built from the SAME
        // one — that coherence is why the two are not separate operations.
        if let Err(reason) = promote_gates(&self.metrics, material.as_ref(), epoch) {
            return SignerVerdict::Withheld(reason);
        }
        // The W1 ordering tripwire that used to sit here is GONE with the copy it
        // guarded: publish-before-handover was a data dependency only because the
        // scheme took a snapshot of the key. An oracle reads the store live, so a
        // key that lands after the scheme is built is picked up on the next vote
        // and the ordering it asserted cannot be violated.
        let committee = match crate::scheme::epoch_committee_from_snapshot(snap) {
            Ok(c) => c,
            Err(e) => return SignerVerdict::InvalidCommittee(e),
        };
        let namespace = fluentbase_bls::fluent_namespace(self.chain_id);
        // The seat has to be known BEFORE the oracle is built, because the oracle
        // checks the share's index against it — taking it from the share would
        // make that check compare a value against itself. A probe scheme is how
        // this node asks the committee where it sits.
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
            // whose engine is destroyed at the next reconcile.
            //
            // IT STILL CARRIES THE EPOCH'S ORACLE, and the earlier decision to
            // hand it `None` "for exact parity with today's `beacon: None`" was
            // wrong. Parity holds on the ATTESTATION arm — no share, no partial,
            // either way — and breaks on the CERTIFICATE arm: under the pin, this
            // scheme was REPAIRABLE (`apply_pin` could attach `PK_epoch` to an
            // already-registered scheme later), and nothing repairs a scheme any
            // more. `oracle_for` reproduces the old POST-pin state exactly:
            // verify-only because there is no share, but still checking the seed
            // slot of every certificate of the epoch.
            //
            // This is a PRODUCER obligation and is not delegated to
            // `Committee::upgrade_scheme`. That guard only runs on an occupied
            // scheme slot; an empty one is filled unconditionally. The committee
            // module happens to fill every slot with its own beacon-active
            // verifier before an engine can ever ask for one, so an oracle-less
            // verdict would be refused there today — but a refusal keeps the
            // WEAKER-free entry by accident of ordering, not because this
            // producer was allowed to be wrong.
            return SignerVerdict::RotatedKey(fluentbase_bls::scheme::build_verifier(
                &namespace,
                committee.bimap,
                epoch.get(),
                Randomness::oracle_for(self, epoch.get()),
            ));
        };
        // `material.is_some()` is what "this node holds a share for the epoch"
        // means, and it is the same sample the gates above just ran over.
        //
        // `oracle_at`, NOT `oracle_for` — the one place on this type that goes
        // round the door `oracle_for` exists to be. It has to, because the seat
        // must ride the oracle here and `oracle_for` builds the `me: None`
        // flavour. So the pre-beacon refusal is INHERITED from the `mandatory_at`
        // gate above rather than applied again: `material` is `None` below the
        // bootstrap epoch and `then` never fires. If that gate is ever removed,
        // this line starts attaching an oracle to a pre-beacon epoch and every
        // LEGAL seedless certificate of it is refused — so the two have to move
        // together. The invariant is already covered, if only as a premise: the
        // pre-beacon half of
        // `a_beacon_active_epoch_with_no_material_is_withheld_and_never_signs_seedless`
        // asserts `Signs(_)` here at `DETERMINISTIC_BOOTSTRAP_EPOCH - 1` under an
        // `Absent` resolver.
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
        // THE BEACON-ACTIVE RULE, enforced HERE so there is one door instead of
        // three. An oracle means "this epoch is beacon-active" to
        // `verify_certificate`, which then rejects every seedless certificate
        // under it — so an oracle on a pre-beacon epoch rejects every LEGAL
        // certificate there. The soft-enter path and the cert-inlet both take
        // their oracle from here, so refusing at the SOURCE makes the invariant
        // hold by construction for every caller.
        Randomness::mandatory_at(self, epoch).then(|| self.oracle_at(epoch, None))
    }

    fn ensure_key(&self, epoch: u64, effort: PinEffort) -> BoxFuture<'_, bool> {
        Box::pin(async move {
            // Nothing to acquire below the bootstrap epoch: no key was ever
            // minted there, and `oracle_for` refuses to attach one anyway.
            if !Randomness::mandatory_at(self, epoch) {
                return false;
            }
            // THE WHOLE LADDER, in two lines. It used to be three rungs over a key
            // store with a provenance FLOOR on the first, because the store mixed a
            // locally reconstructed tier with attested ones and a caller whose write
            // was terminal could not take the weak one. `KeyIndex` has one tier by
            // construction (П-3), so the floor has nothing to exclude and the rungs
            // collapse to: is the mint's artifact local, and — off the vote path
            // only — may we spend one bounded fetch for it.
            let Some(minted_at) = self.keys.minted_at(epoch) else {
                return false;
            };
            if self.keys.holds_mint_of(epoch) == Some(true) {
                return true;
            }
            // The ONLY thing the effort decides, unchanged: may this call spend a
            // peer round-trip. `Local` callers (cert ingress, soft-enter) run
            // against a ~1 s verify budget and the network rung's is seconds.
            let (PinEffort::Thorough, Some(acquire)) = (effort, self.acquire.as_ref()) else {
                return false;
            };
            acquire.fetch(minted_at).await;
            self.keys.holds_mint_of(epoch) == Some(true)
        })
    }

    /// SEED RETENTION ONLY. Its key leg — `w3_backfill`, which published this
    /// node's own reconstruction of `E−1`'s key — was deleted with W1 (П-3), and the
    /// key store it pruned no longer exists: [`KeyIndex`]'s artifact half is
    /// deliberately never evicted (`artifact` module doc) and its mint memo is
    /// bytes per epoch. `reconciled` therefore names an epoch nothing is resolved
    /// FOR; the operation survives because the two σ windows below are still real,
    /// and PLAN row 5.2 is what removes it.
    fn observe_epoch(&self, _reconciled: Epoch, entered_frontier: Epoch) {
        let oldest = entered_frontier
            .get()
            .saturating_sub(crate::SCHEME_RETENTION_EPOCHS as u64);
        // The quarantine rides the scheme-retention window: past that edge no key
        // can arrive any more, so a held σ can never be promoted and is only memory
        // a peer could grow.
        self.seeds.retain_quarantine_from(oldest);
        // And so does the terminal pin: it is asked for by the NEXT epoch, so an
        // epoch past the retention edge has no asker left.
        self.seeds.retain_terminal_from(oldest);
    }

    fn observe_cert(&self, epoch: u64) {
        let oldest = epoch.saturating_sub(crate::SCHEME_RETENTION_EPOCHS as u64);
        self.seeds.retain_quarantine_from(oldest);
        self.seeds.retain_terminal_from(oldest);
    }

    fn artifact_bytes(&self, epoch: u64) -> Option<Vec<u8>> {
        self.artifacts
            .get(epoch)
            .map(|a| super::artifact::encode_artifact(&a))
    }

    fn faults(&self) -> Option<mpsc::UnboundedReceiver<DataFault>> {
        let taken = self.faults_rx.lock().ok()?.take()?;
        // Arm only once a consumer exists, so the queue can never grow behind a
        // receiver nobody reads.
        self.faults_armed.store(true, Ordering::Relaxed);
        Some(taken)
    }
}
