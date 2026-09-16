//! Beacon observability counters, registered on the commonware metrics registry
//! (scraped at `:19100` via `Metrics::register`, not the `metrics::` macro
//! recorder, which lands on reth's registry and is invisible there).
//!
//! A single [`BeaconMetrics`] is created and registered once per node class in
//! [`crate::beacon::plane`], then cloned into the DKG actor, the agreement plane
//! and the randomness provider. Each metric is `Arc`-backed, so the struct is
//! cheap to clone and every clone shares one counter.
//!
//! This struct holds only the families the randomness subsystem owns; other
//! subsystems' counters live with their owners. Exactly one owner registers each
//! family per node class.

use commonware_runtime::Metrics;
use prometheus_client::{
    encoding::{EncodeLabelSet, EncodeLabelValue, LabelValueEncoder},
    metrics::{counter::Counter, family::Family, gauge::Gauge},
};

/// Why an epoch is stalled — the `reason` label of `dpos_dkg_stalled` and the
/// payload of the DKG actor's `Stalled{reason}` event. Latched per `(epoch,
/// reason)` by the actor: one log line, one gauge step, until the epoch keys or
/// ages out.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum StallReason {
    /// Every pinned body is held and no dealer quorum is selectable within the
    /// agreed set: more than `f` dealers are absent from what the network pinned.
    QuorumMissing,
    /// The agreed set names a body this node does not hold yet (being fetched).
    BodyMissing,
    /// The agreement instance certified a payload whose body never arrived.
    BodyLost,
    /// The epoch's boundary is here or past and this node holds no artifact for it.
    NoArtifact,
    /// A share's disk write failed; it is not adopted.
    PersistFailed,
    /// A share does not lie on the certified artifact's polynomial at this node's
    /// index: refused, the epoch heals over its journal.
    OffPolynomial,
    /// The share cannot be derived here (a journal acking a dealing it no longer
    /// holds, or a ceremony that cannot be rebuilt over the roster).
    Unrecoverable,
    /// Two different quorum-certified artifacts for one epoch.
    Conflict,
    /// The journal recompute over every pinned body could not run (the retained
    /// journal is gone or torn) or failed with a non-terminal error: the heal is
    /// parked, visibly, until an input changes.
    HealFailed,
}

impl StallReason {
    /// Every reason, in declaration order — what the gauge's HELP enumerates, so
    /// the registry text cannot drift from the enum.
    pub const ALL: [StallReason; 9] = [
        Self::QuorumMissing,
        Self::BodyMissing,
        Self::BodyLost,
        Self::NoArtifact,
        Self::PersistFailed,
        Self::OffPolynomial,
        Self::Unrecoverable,
        Self::Conflict,
        Self::HealFailed,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::QuorumMissing => "quorum_missing",
            Self::BodyMissing => "body_missing",
            Self::BodyLost => "body_lost",
            Self::NoArtifact => "no_artifact",
            Self::PersistFailed => "persist_failed",
            Self::OffPolynomial => "off_polynomial",
            Self::Unrecoverable => "unrecoverable",
            Self::Conflict => "conflict",
            Self::HealFailed => "heal_failed",
        }
    }
}

impl EncodeLabelValue for StallReason {
    fn encode(&self, encoder: &mut LabelValueEncoder) -> Result<(), std::fmt::Error> {
        EncodeLabelValue::encode(&self.as_str(), encoder)
    }
}

/// The `{reason=...}` label set of the `dpos_dkg_stalled` gauge family.
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
struct StallLabels {
    reason: StallReason,
}

/// Beacon counters. See the module docs for the registration + clone topology.
#[derive(Clone, Debug, Default)]
pub struct BeaconMetrics {
    /// A live-DKG ceremony finalized (`PK_E` + share computed + stored).
    pub dkg_ceremony_ok: Counter,
    /// A live-DKG ceremony failed to finalize after the ready-probe (epoch beacon
    /// stalls until a reshare / next ceremony).
    pub dkg_ceremony_fail: Counter,
    /// `dpos_dkg_stalled{reason}` — how many retained epochs carry a `Stalled{reason}`
    /// latch right now (the DKG actor's state machine: `+1` when the latch is raised,
    /// `−1` when the epoch keys or ages out). The paired log line is bounded to one
    /// per `(epoch, reason)`; this is what scales with the condition.
    dkg_stalled: Family<StallLabels, Gauge<i64>>,
    /// Two different quorum-certified artifacts reached the DKG actor for one epoch
    /// — `≥ 2q−n` signers certified both. The epoch is `Conflict` on this node and
    /// its signing stops.
    pub dkg_artifact_conflict: Counter,
    /// `Conflict` verdicts whose durable marker could not be written by the DKG
    /// actor: the verdict holds in this process, the share is dropped regardless,
    /// and a restart re-judges the epoch from the artifact store's own witness.
    pub dkg_conflict_marker_failed: Counter,
    /// Artifact hand-offs to the DKG actor's write-back that were refused (the
    /// mailbox full or gone). Not a lost fact: the store owns the artifact and the
    /// actor reads it on its next height tick, so this counts ticks of latency.
    pub dkg_artifact_handoff_lost: Counter,
    /// A dealer was proven to have signed two distinct valid logs for one epoch on
    /// this node — the pair is journaled as evidence and the dealer is locally
    /// banned from gossip for that epoch. Counted once per `(epoch, dealer)` on the
    /// node that saw both.
    pub dkg_dealer_equivocation: Counter,
    /// A per-epoch engine self-demoted to the cert-follow plane because it holds no
    /// local beacon polynomial for the epoch (`NoBeaconPolynomial`).
    pub engine_demoted_no_polynomial: Counter,
    /// A would-be signer was demoted to verify-only by the promote share gate: its
    /// resolved DKG share does not verify against its own sharing. Load-bearing
    /// because every nullify carries a seed partial, so with `t == quorum` one such
    /// member makes the nullify quorum unreachable during a stall.
    pub engine_demoted_bad_share: Counter,
    /// Participation withheld because the plane has not frozen its
    /// `(dpos_activation, epoch_interval)` yet — no ceremony has been able to run.
    pub engine_demoted_geometry_unfrozen: Counter,
    /// A consensus-pinned dealer-log index named a position outside the committed
    /// committee, and the ceremony skipped it. Nothing can satisfy such an entry, so
    /// before the skip it held `all_held=false` forever and wedged the epoch's DKG
    /// in silence. With the codec bound in place an honest chain cannot produce one:
    /// non-zero means either a Byzantine proposer got a block past the vote gate, or
    /// the committee the ceremony reads disagrees with the one the logs were
    /// numbered against.
    pub dkg_pinned_idx_out_of_range: Counter,
    /// A ceremony holds an agreed pinned set it cannot yet finalize over and
    /// deferred to the epoch boundary. Counted once per epoch per reason.
    pub dkg_finalize_deferred: Counter,
    /// Dealer logs this node held, body-checked, that the agreed pinned set left
    /// out. Observability only — the agreement's acceptance predicate must never
    /// read local state, so this influences no vote. A rare non-zero is a delivery
    /// race; a persistent non-zero on one target is a proposer systematically
    /// dropping entries.
    pub dkg_agree_logs_omitted: Counter,
    /// An agreement instance held a finalization certificate and never obtained the
    /// body it names, so it tore itself down without producing an artifact. The
    /// restart case: the certificate is journaled, the body buffer is in memory
    /// only, and no live sender re-broadcasts a decided proposal.
    pub dkg_agree_body_lost: Counter,
    /// Target epochs whose agreement could not propose because too few members had
    /// confirmed they hold the pinned dealer logs. Counted once per epoch per
    /// reason, beside the warn naming which of the two it was. Never fatal — the
    /// plane keeps trying.
    pub dkg_agree_bar_unmet: Counter,
    /// Artifact requests this node answered with the artifact itself.
    pub dkg_artifact_served: Counter,
    /// Artifact requests this node answered `NotYet`. The normal answer for most of
    /// an epoch, so a high count is not a fault on its own; read it against
    /// `served`.
    pub dkg_artifact_not_yet: Counter,
    /// Served artifacts refused as proven misbehaviour: bytes that do not decode,
    /// an answer about another epoch, or a certificate that fails against
    /// `committee[epoch]`. The only path that costs a peer its standing on this
    /// engine.
    pub dkg_artifact_rejected: Counter,
    /// Served artifacts this node could not check, because `committee[epoch]` was
    /// not readable — dropped without storing and without blaming the peer. A
    /// property of this node's chain view.
    pub dkg_artifact_unverifiable: Counter,
    /// Pulls that ended with nobody answering inside the window. This is the
    /// exhausted one-pass walk, surfaced instead of the resolver's silent
    /// unbounded retry.
    pub dkg_artifact_pull_exhausted: Counter,
    /// Pulls that came back holding the artifact, counted on the pulling node. The
    /// serving side has its own `dkg_artifact_served`.
    pub dkg_artifact_pull_ok: Counter,
    /// Epochs whose share is provably unrecoverable here — this node acked a
    /// dealer's private point and no longer holds it. Non-zero means one epoch is
    /// sat out as a verifier, not that the node is unhealthy; persistently climbing
    /// across epochs means the share directory is being destroyed under a running
    /// node.
    pub dkg_share_unrecoverable: Counter,
    /// Shares refused by the share-on-polynomial self-check inside
    /// `DkgActor::adopt_share`, the one gate every adoption path passes through. On
    /// the live finalize path it means this node computed a share that does not lie
    /// on the certified artifact's polynomial; on the recompute-heal path it is the
    /// ordinary "keep fetching" verdict for an incomplete dealer-log set. Either way
    /// the share is not adopted.
    pub dkg_share_off_polynomial: Counter,
    /// Shares refused because their disk write failed. The share is not adopted, so
    /// the node is verify-only for the epoch; the alternative is "signing now, mute
    /// after a restart". The retry is the recompute-heal on the next height tick.
    pub dkg_share_persist_failed: Counter,
    /// Artifacts a `--cert-follow` follower adopted from its cert upstream after
    /// checking them against `committee[epoch]` read from its own chain state.
    /// The follower's only key producer, so a flat zero here and a climbing
    /// `dpos_cert_vote_only_admissions_total` is the symptom.
    pub follower_artifact_adopted: Counter,
    /// Epochs a follower wanted an artifact for and its upstream did not serve —
    /// including an upstream too old to know the method at all. Not a fault: the
    /// artifact of a live epoch legitimately does not exist yet, and the trigger
    /// re-asks on the next certificate.
    pub follower_artifact_miss: Counter,
    /// Assembled seeds this node checked against `PK_epoch` and accepted
    /// ([`fluentbase_bls::oracle::SeedCheck::Valid`]).
    ///
    /// The positive edge for "this epoch left vote-only admission": a positive
    /// reading rather than the absence of a vote-only admission, because an absence
    /// is green whenever certificates merely stopped arriving.
    ///
    /// Global, not per-epoch: the one consumer asserts a `0 -> non-zero` edge over a
    /// window in which the node verifies a single epoch.
    pub seed_verify_ok: Counter,
    /// Assembled seeds this node could not check because it holds no key for the
    /// epoch ([`fluentbase_bls::oracle::SeedCheck::NoKey`]). The certificate is
    /// admitted on its multisig quorum alone and nobody consumes its σ.
    ///
    /// The consensus-plane twin of `dpos_cert_vote_only_admissions_total`; a plain
    /// validator runs no cert inlet, so this is the only place its keyless window is
    /// visible.
    pub seed_verify_no_key: Counter,
    /// Assembled seeds this node checked against `PK_epoch` and refused
    /// ([`fluentbase_bls::oracle::SeedCheck::Invalid`]).
    ///
    /// The error line is latched per epoch, so repeat refusals are silent; this
    /// counter is not, so a repeat forger stays visible. Counted at the oracle, so it
    /// covers both the synchronous refusal inside `observe_certificate` and the late
    /// one the settle reaches when a key lands on a held σ. Global, not per-epoch.
    pub seed_verify_invalid: Counter,
}

impl BeaconMetrics {
    /// Raise `dpos_dkg_stalled{reason}` by one (an epoch latched the reason).
    pub fn stalled(&self, reason: StallReason) {
        self.dkg_stalled
            .get_or_create(&StallLabels { reason })
            .inc();
    }

    /// Lower `dpos_dkg_stalled{reason}` by one (the latched epoch keyed or aged out).
    pub fn stall_cleared(&self, reason: StallReason) {
        self.dkg_stalled
            .get_or_create(&StallLabels { reason })
            .dec();
    }

    /// Test support: the current `dpos_dkg_stalled{reason}` — for the balance
    /// assertions (every raise is paired with a clear by the time an epoch keys or
    /// ages out).
    #[cfg(test)]
    pub fn stalled_gauge(&self, reason: StallReason) -> i64 {
        self.dkg_stalled
            .get_or_create(&StallLabels { reason })
            .get()
    }

    /// Register every counter on the commonware registry. Call once per node class,
    /// against the launch context.
    pub fn register(&self, ctx: &impl Metrics) {
        let reasons: Vec<&str> = StallReason::ALL.iter().map(|r| r.as_str()).collect();
        ctx.register(
            "dpos_dkg_stalled",
            format!(
                "Retained epochs whose DKG is stalled, by reason ({}). One log line per \
                 (epoch, reason); this gauge scales with the condition.",
                reasons.join(", ")
            ),
            self.dkg_stalled.clone(),
        );
        ctx.register(
            "dpos_dkg_artifact_conflict_total",
            "Epochs for which two DIFFERENT quorum-certified artifacts reached the DKG actor \
             (≥ 2q−n signers certified both); the epoch's signing is stopped on this node.",
            self.dkg_artifact_conflict.clone(),
        );
        ctx.register(
            "dpos_dkg_conflict_marker_failed_total",
            "Conflict verdicts whose durable marker the DKG actor could not write; the share \
             is dropped regardless and a restart re-judges the epoch from the artifact store.",
            self.dkg_conflict_marker_failed.clone(),
        );
        ctx.register(
            "dpos_dkg_artifact_handoff_lost_total",
            "Artifact hand-offs to the DKG actor's write-back refused because the mailbox was \
             full or gone; the store keeps the artifact and the actor reads it on its next tick.",
            self.dkg_artifact_handoff_lost.clone(),
        );
        ctx.register(
            "dkg_agree_body_lost_total",
            "Agreement instances that certified a payload whose body never arrived, so no \
             artifact was produced and the target epoch must re-agree.",
            self.dkg_agree_body_lost.clone(),
        );
        ctx.register(
            "dkg_agree_bar_unmet_total",
            "Target epochs whose agreement could not propose because too few members confirmed \
             they hold the pinned dealer logs (once per epoch per reason). The plane keeps \
             trying; the paired warn says whether the quorum or only the margin was missing.",
            self.dkg_agree_bar_unmet.clone(),
        );
        ctx.register(
            "dkg_agree_logs_omitted_total",
            "Body-checked dealer logs this node held that the agreed pinned set omitted. \
             Vote-neutral; a persistent non-zero means a censoring proposer.",
            self.dkg_agree_logs_omitted.clone(),
        );
        ctx.register(
            "dkg_ceremony_ok_total",
            "Live-DKG ceremonies that finalized (PK_E + share stored).",
            self.dkg_ceremony_ok.clone(),
        );
        ctx.register(
            "dpos_dkg_dealer_equivocation_total",
            "Dealers proven on this node to have signed two distinct valid DKG logs for one \
             epoch: the pair is kept as evidence and the dealer is locally banned from gossip \
             for that epoch. Nothing is sent on-chain.",
            self.dkg_dealer_equivocation.clone(),
        );
        ctx.register(
            "dkg_ceremony_fail_total",
            "Live-DKG ceremonies that failed to finalize after the ready-probe.",
            self.dkg_ceremony_fail.clone(),
        );
        ctx.register(
            "epoch_engine_demoted_no_polynomial_total",
            "Per-epoch engines self-demoted to cert-follow for lack of a local beacon polynomial.",
            self.engine_demoted_no_polynomial.clone(),
        );
        ctx.register(
            "epoch_engine_demoted_geometry_unfrozen_total",
            "Would-be signers withheld because the beacon plane has not frozen its \
             (dpos_activation, epoch_interval) yet, so no ceremony has run.",
            self.engine_demoted_geometry_unfrozen.clone(),
        );
        ctx.register(
            "epoch_engine_demoted_bad_share_total",
            "Would-be signers demoted to verify-only because the resolved DKG share does not \
             verify against its own sharing.",
            self.engine_demoted_bad_share.clone(),
        );
        ctx.register(
            "dpos_dkg_pinned_idx_out_of_range_total",
            "Consensus-pinned dealer-log indices with no position in the committed committee, \
             skipped by the ceremony. 0 on a healthy chain.",
            self.dkg_pinned_idx_out_of_range.clone(),
        );
        ctx.register(
            "dpos_dkg_finalize_deferred_total",
            "Ceremonies that could not finalize over their agreed pinned set and deferred \
             to the epoch boundary (once per epoch per reason).",
            self.dkg_finalize_deferred.clone(),
        );
        ctx.register(
            "dpos_dkg_artifact_served_total",
            "Agreement-artifact requests answered with the artifact.",
            self.dkg_artifact_served.clone(),
        );
        ctx.register(
            "dpos_dkg_artifact_not_yet_total",
            "Agreement-artifact requests answered NotYet (the target epoch has not converged \
             here yet). The normal answer for most of an epoch, not a fault.",
            self.dkg_artifact_not_yet.clone(),
        );
        ctx.register(
            "dpos_dkg_artifact_rejected_total",
            "Served artifacts refused as proven misbehaviour (undecodable, wrong epoch, or a \
             certificate that fails against committee[epoch]). The only path that costs a peer \
             its standing on this engine.",
            self.dkg_artifact_rejected.clone(),
        );
        ctx.register(
            "dpos_dkg_artifact_unverifiable_total",
            "Served artifacts dropped unchecked because committee[epoch] was not readable here. \
             A property of this node's chain view, never a verdict on the peer.",
            self.dkg_artifact_unverifiable.clone(),
        );
        ctx.register(
            "dpos_dkg_artifact_pull_exhausted_total",
            "Artifact pulls that ended with no peer answering inside the window.",
            self.dkg_artifact_pull_exhausted.clone(),
        );
        ctx.register(
            "dpos_dkg_artifact_pull_ok_total",
            "Artifact pulls that came back holding the artifact, counted on the PULLING node.",
            self.dkg_artifact_pull_ok.clone(),
        );
        ctx.register(
            "dpos_dkg_share_unrecoverable_total",
            "Epochs whose DKG share is provably unrecoverable here (this node acked a dealing \
             it no longer holds), so nothing is retried for them and the epoch is sat out as \
             a verifier.",
            self.dkg_share_unrecoverable.clone(),
        );
        ctx.register(
            "dpos_dkg_share_off_polynomial_total",
            "Shares refused by the share-on-polynomial self-check that gates every adoption \
             path: the share does not lie on the artifact-pinned polynomial at this node's \
             index, so it is never adopted and the epoch stays verify-only.",
            self.dkg_share_off_polynomial.clone(),
        );
        ctx.register(
            "dpos_dkg_share_persist_failed_total",
            "Shares refused because the disk write failed: the share is not adopted and the \
             epoch stays verify-only rather than signing with material no restart can reload.",
            self.dkg_share_persist_failed.clone(),
        );
        ctx.register(
            "dpos_follower_artifact_adopted_total",
            "Epoch-key artifacts a cert-follow follower adopted after verifying them against \
             committee[epoch] read from its own chain state.",
            self.follower_artifact_adopted.clone(),
        );
        ctx.register(
            "dpos_follower_artifact_miss_total",
            "Epochs a cert-follow follower asked its upstream for and got no artifact for \
             (including an upstream too old to know the method). Not a fault.",
            self.follower_artifact_miss.clone(),
        );
        ctx.register(
            "dpos_seed_verify_ok_total",
            "Assembled seeds checked against PK_epoch and accepted. The positive edge for \
             'this epoch's certificates left vote-only admission' — it moves only when a seed \
             slot was actually verified, never merely because certificates stopped arriving.",
            self.seed_verify_ok.clone(),
        );
        ctx.register(
            "dpos_seed_verify_no_key_total",
            "Assembled seeds this node could not check because it holds no key for the epoch. \
             The certificate is admitted on its multisig quorum alone and nobody consumes its \
             seed.",
            self.seed_verify_no_key.clone(),
        );
        ctx.register(
            "dpos_seed_verify_invalid_total",
            "Assembled seeds REFUSED under the epoch's attested key. The paired ERROR line is \
             latched per epoch; this is not, so a repeat forger stays visible.",
            self.seed_verify_invalid.clone(),
        );
    }
}
