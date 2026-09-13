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
    beacon::{Beacon, Observed, ObservedCertificate, Seed},
    executor,
    executor::{Command, Notarized},
};
use commonware_consensus::{simplex::types::Activity, Reporter};
use fluentbase_bls::Scheme as BlsScheme;
use std::sync::Arc;
use tracing::{error, warn, Span};

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
        let observed = self
            .beacon
            .observe_certificate(ObservedCertificate::Notarization(n.proposal.round, &n));
        // THE VERDICT, READ. A σ the beacon would not FILE is a σ this node may not
        // SPECULATE on, and the two outcomes that say so are both about the value
        // rather than about the door:
        //
        //  - `Refused` — recovered from partials this node had already verified and
        //    still failing the epoch's own group key. The verdict logs it; there is
        //    nothing here worth executing against.
        //  - `Pending` — no `PK_E` resolvable here, so the certificate took
        //    vote-only admission and its σ slot was never checked. Speculating on it
        //    is the seed-blind divergence class: `prev_randao` rides the state root,
        //    so a wrong σ is a fork rather than a wasted attempt.
        //
        // SKIPPING THE SPECULATION IS THE WHOLE CONSEQUENCE, and it costs no
        // liveness because speculation is best-effort by construction: the height is
        // derived from the FINALIZED tier off the seed index, and a σ filed
        // `Pending` here is settled on the `KeyAvailable` edge — which wakes the
        // executor's own arm. Blanking the seed instead would be the defect: a
        // beacon-active height derived with `None` re-rolls `prev_randao`.
        //
        // THE SKIP ALSO DROPS THIS ROUND'S `SpecNotarized` MESSAGE, and with it
        // the `try_drain_parked` the executor runs after `spec_execute`
        // (`executor.rs:2478`). An out-of-order notarization parked below this
        // round therefore waits for the next round that DOES speculate — a delay,
        // not a park: the drain has two other drivers (`executor.rs:2920`,
        // `:3892`), both on delivery paths this skip does not touch (review C-12).
        //
        // `Inactive` is the ordinary pre-beacon answer and must NOT skip — those
        // epochs legitimately derive from `None`.
        if matches!(observed, Observed::Refused | Observed::Pending) {
            warn!(
                round = %n.proposal.round,
                ?observed,
                "beacon will not file this round's σ; skipping speculation (the finalized tier \
                 derives the height once the σ is filed)"
            );
            return;
        }
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
