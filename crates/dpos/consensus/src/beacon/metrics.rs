//! Beacon observability counters, registered on the commonware metrics registry
//! (scraped at `:19100` via `Metrics::register`, NOT the `metrics::` macro recorder
//! which lands on reth's registry and is invisible there).
//!
//! A single [`BeaconMetrics`] is created + registered once in `dpos.rs::launch`
//! (against the launch context) and cloned into the DKG actor, the agreement
//! plane and the randomness provider. Each metric is `Arc`-backed, so the struct
//! is cheap to clone and every clone shares one counter.
//!
//! This struct holds ONLY the families the randomness subsystem owns. Two groups
//! that used to live here have moved to their real owners, keeping their family
//! names byte-identical:
//!
//! - [`crate::epoch_manager::EpochEngineMetrics`] — the membership /
//!   `Inline::genesis` counters, which are facts about the core's spawn decision
//!   and survive a randomness implementation that has no DKG at all.
//! - [`crate::executor::ExecutorMetrics`] — the per-derived-block seed
//!   observation, incremented by the executor on BOTH node classes.
//!
//! Exactly one owner registers each family per node class; the split is what
//! keeps the core from having to name a beacon type to count its own spawns.

use commonware_runtime::Metrics;
use prometheus_client::metrics::counter::Counter;

/// Beacon counters. See the module docs for the registration + clone topology.
#[derive(Clone, Debug, Default)]
pub struct BeaconMetrics {
    /// A live-DKG ceremony finalized (`PK_E` + share computed + stored).
    pub dkg_ceremony_ok: Counter,
    /// A live-DKG ceremony failed to finalize after the ready-probe (epoch beacon
    /// stalls until a reshare / next ceremony).
    pub dkg_ceremony_fail: Counter,
    /// A per-epoch engine self-demoted to the cert-follow plane because it holds no
    /// local beacon polynomial for the epoch (`NoBeaconPolynomial`).
    pub engine_demoted_no_polynomial: Counter,
    /// A would-be signer was demoted to verify-only by the promote VALUE-gate:
    /// its locally-resolved `PK_epoch` differs from the quorum-attested key (the
    /// agreement artifact's entry). Non-zero = a diverged local key
    /// reconstruction was caught before it could sign/publish (soak 2026-07-14
    /// class).
    pub engine_demoted_key_divergence: Counter,
    /// A would-be signer was demoted to verify-only by the promote SHARE-gate: its
    /// resolved DKG share does not verify against its own sharing. Distinct from
    /// the VALUE gate above, which compares the GROUP key against the network and
    /// is a no-op before the network has attested one. Load-bearing since every
    /// Nullify carries a seed partial: with `t == quorum`, one such member on the
    /// plane makes the nullify quorum unreachable during a stall, when there are no
    /// proposals to expose the bad share on the notarize path.
    pub engine_demoted_bad_share: Counter,
    /// A consensus-pinned dealer-log index named a position outside the committed
    /// committee, and the ceremony skipped it. Nothing can ever satisfy such an entry
    /// (the resolver fetches per-DEALER), so before the skip it held `all_held=false`
    /// forever and wedged the epoch's DKG in silence. With the `order_block` codec
    /// bound in place an honest chain cannot produce one: non-zero means either a
    /// Byzantine proposer got a block past an accept-biased vote gate, or the
    /// committee the ceremony reads disagrees with the one the logs were numbered
    /// against — the 2026-07-21 idx-stall class. 0 on a healthy chain.
    pub dkg_pinned_idx_out_of_range: Counter,
    /// A ceremony holds an agreed pinned set it cannot yet finalize over, and
    /// deferred to the epoch boundary. Previously this wait was completely silent,
    /// so any stall of this family could only be diagnosed post-mortem from a wedged
    /// boundary. Counted once per epoch per reason.
    pub dkg_finalize_deferred: Counter,
    /// Dealer logs this node held, body-checked, that the AGREED pinned set left
    /// out. Observability only — the agreement's acceptance predicate must never
    /// read local state, so this counter influences no vote. A rare non-zero is a
    /// delivery race (the proposer had not received that log yet); a persistent
    /// non-zero on one target epoch is a proposer systematically dropping
    /// entries, which is the only signal that separates the two.
    pub dkg_agree_logs_omitted: Counter,
    /// An agreement instance held a finalization certificate and never obtained the
    /// body it names, so it tore itself down without producing an artifact. The
    /// restart case: the certificate is journaled, the body buffer is in memory
    /// only, and no live sender re-broadcasts a decided proposal. Non-zero means
    /// that target epoch has to re-agree on a fresh instance.
    pub dkg_agree_body_lost: Counter,
    /// Target epochs whose agreement could not propose because too few members had
    /// confirmed they hold the pinned dealer logs. Counted once per epoch per
    /// reason, beside the warn that names which of the two it was: below the
    /// quorum the members are genuinely absent, between quorum and the bar the
    /// epoch starts on its own at the margin release view. Never fatal — the plane
    /// keeps trying.
    pub dkg_agree_bar_unmet: Counter,
    /// Artifact requests this node answered with the artifact itself.
    pub dkg_artifact_served: Counter,
    /// Artifact requests this node answered `NotYet`. The NORMAL answer for most
    /// of an epoch — a peer asking before the target's plane has converged — so a
    /// high count is not a fault on its own; read it against `served`.
    pub dkg_artifact_not_yet: Counter,
    /// Served artifacts refused as proven misbehaviour: bytes that do not decode,
    /// an answer about another epoch, or a certificate that fails against
    /// `committee[epoch]`. The ONLY path that costs a peer its standing on this
    /// engine (commonware's resolver `excluded` set has no removal path), so a
    /// non-zero here is a real accusation and should be rare.
    pub dkg_artifact_rejected: Counter,
    /// Served artifacts this node could not CHECK, because `committee[epoch]` was
    /// not readable — dropped without storing and without blaming the peer. A
    /// property of this node's chain view, not of the artifact.
    pub dkg_artifact_unverifiable: Counter,
    /// Pulls that ended with nobody answering inside the window. This is the
    /// exhausted one-pass walk, surfaced instead of the resolver's silent
    /// unbounded retry.
    pub dkg_artifact_pull_exhausted: Counter,
    /// Pulls that came back holding the artifact. The pulling side had no success
    /// counter at all — only `served` existed, and that is the SERVING node's
    /// view, so a member whose live-epoch pull is what unblocks it was invisible
    /// on its own metrics.
    pub dkg_artifact_pull_ok: Counter,
    /// Epochs whose share is provably unrecoverable on this node — it acked a
    /// dealer's private point and no longer holds it (`MissingPlayerDealing`).
    /// Non-zero means one epoch is sat out as a verifier, NOT that the node is
    /// unhealthy: the next epoch's ceremony is untouched. Persistently climbing
    /// across epochs is the real alert, and it means the share directory is being
    /// destroyed under a running node.
    pub dkg_share_unrecoverable: Counter,
    /// Artifacts a `--cert-follow` follower adopted from its cert upstream after
    /// checking them against `committee[epoch]` read from its OWN chain state.
    /// The follower's only key producer, so a flat zero here and a climbing
    /// `dpos_cert_vote_only_admissions_total` is the whole of FLU-1167's symptom.
    pub follower_artifact_adopted: Counter,
    /// Epochs a follower wanted an artifact for and its upstream did not serve —
    /// including an upstream too old to know the method at all. Not a fault: the
    /// artifact of a live epoch legitimately does not exist yet, and the trigger
    /// re-asks on the next certificate.
    pub follower_artifact_miss: Counter,
}

impl BeaconMetrics {
    /// Register every counter on the commonware registry. Call once, against the
    /// launch context (mirrors `executor.rs`'s `pending_finalizations` gauge).
    pub fn register(&self, ctx: &impl Metrics) {
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
            "epoch_engine_demoted_key_divergence_total",
            "Would-be signers demoted to verify-only because the locally-resolved PK_epoch \
             differs from the network-attested key.",
            self.engine_demoted_key_divergence.clone(),
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
    }
}
