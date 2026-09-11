//! Speculative-execution Reporter: the notarization arm of the simplex
//! `Reporters` multiplex.
//!
//! On `Activity::Notarization` (round-1 2f+1 quorum), it forwards a
//! `SpecNotarized` command to the executor so the EL head can advance ahead of
//! finalization — hiding execution latency under the finalization rounds at
//! the 1 block/s target. The seed is recovered straight from the notarization
//! certificate (the combined consensus scheme); the body is fetched by the
//! executor from the marshal buffer by digest.
//!
//! This is a THIN adapter: it only translates `Activity` → `executor::Command`
//! and never blocks the voter (the executor mailbox is an unbounded channel).
//! All speculative work — fetch, derive, import, reconcile, rollback — runs in
//! the executor's single-threaded loop, serialized with finalized delivery, so
//! there is no cross-actor race on the speculative state.

use crate::{
    beacon::{Beacon, ObservedCertificate, Seed},
    executor,
    executor::{Command, Notarized},
};
use commonware_consensus::{simplex::types::Activity, Reporter};
use fluentbase_bls::Scheme as BlsScheme;
use std::sync::Arc;
use tracing::{error, Span};

type Digest = crate::digest::Digest;

/// Reporter sink that converts `Activity::Notarization` into the executor's
/// speculative command. All other activities are ignored.
#[derive(Clone)]
pub struct Mailbox {
    executor: executor::Mailbox,
    /// The beacon this reporter HANDS the notarization to. The certificate
    /// carries σ; the beacon owns the verdict and where the value is kept.
    beacon: Arc<dyn Beacon>,
}

impl Mailbox {
    pub fn new(executor: executor::Mailbox, beacon: Arc<dyn Beacon>) -> Self {
        Self { executor, beacon }
    }
}

impl Reporter for Mailbox {
    type Activity = Activity<BlsScheme, Digest>;

    async fn report(&mut self, activity: Self::Activity) {
        let Activity::Notarization(n) = activity else {
            return;
        };
        let seed = n.certificate.seed().map(|signature| Seed {
            target_round: n.proposal.round,
            signature,
        });
        // ORDERING-CRITICAL: this hand-over MUST stay SYNCHRONOUS — never behind an
        // await, never in a spawned task. Re-derived from scratch, because the
        // justification this comment used to carry was stale: it cited
        // `certify.rs seed_certify_verdict`, a function deleted with the certify
        // gate.
        //
        // WHAT IT PROTECTS NOW is the propose path's liveness, in three steps:
        //
        //  1. The voter `await`s `report()` INLINE before it advances the view
        //     (commonware `voter/actor.rs:529-531`; reporter backpressure is
        //     consensus-critical by design). So anything done synchronously here
        //     happens-before the next view exists.
        //  2. The block just notarized is finalized moments later, and the
        //     executor derives it from σ at ITS OWN round — exactly the round this
        //     line records, `Round(epoch(h), block.proposal_view)`.
        //  3. A miss there is not a wrong derive — the height is HELD
        //     (`awaiting_seed`) until the record fires the beacon's seed wake-up.
        //     Deferring this record would lose that race against the very next
        //     finalization, turning a rare hold into a per-block one and putting
        //     the whole execution pipeline one wake behind consensus.
        //
        // WHAT IT DOES NOT PROTECT, checked rather than assumed: the old comment
        // also demanded this run BEFORE the executor send below. That half is not
        // load-bearing. The executor consults the store only on a spin-round
        // mismatch, and then for the CANONICAL round — a round recorded by an
        // earlier report, not by this one (`Actor::spec_seed_for`, the
        // `seed(canonical)` read on the re-canonicalisation arm); when the rounds
        // match it uses the seed carried in the command and reads nothing. Both
        // statements are synchronous anyway, so the order between them is
        // unobservable. Kept adjacent for readability, not for correctness.
        //
        // The verdict is the BEACON's — including the fact that a σ this node
        // recovered from partials it had already verified must still be checked
        // through the epoch's own oracle. One pairing per round is the cheaper
        // half of that trade against a constructor that skipped the check for the
        // local path and became the one door a later writer reached for.
        let _observed = self
            .beacon
            .observe_certificate(ObservedCertificate::Notarization(n.proposal.round, &n));
        // PLAN row 5.2 wires the verdict: `Pending` becomes `ReplaySeed::Defer`
        // on the crash-replay path and `Refused` an inlet data fault. Until then
        // nobody reads it here — named rather than `_` so the three sites that
        // owe 5.2 a reader are greppable.
        let msg = executor::Message {
            cause: Span::current(),
            command: Command::SpecNotarized(Box::new(Notarized {
                digest: n.proposal.payload,
                seed,
            })),
        };
        if self.executor.send(msg).is_err() {
            error!("executor mailbox closed; dropping notarization");
        }
    }
}
