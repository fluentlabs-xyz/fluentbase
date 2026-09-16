//! Speculative-execution Reporter: the notarization arm of the simplex
//! `Reporters` multiplex.
//!
//! On `Activity::Notarization` (round-1 2f+1 quorum) it forwards a
//! `SpecNotarized` command to the executor so the EL head can advance ahead of
//! finalization, hiding execution latency under the finalization rounds. The seed
//! is recovered from the notarization certificate; the body is fetched by the
//! executor from the marshal buffer by digest.
//!
//! It is a thin adapter: it only translates `Activity` into `executor::Command`
//! and never blocks the voter (the executor mailbox is an unbounded channel). All
//! speculative work runs in the executor's single-threaded loop, serialized with
//! finalized delivery, so there is no cross-actor race on the speculative state.

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
    /// The beacon this reporter hands the notarization to. The certificate
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
        // Ordering-critical: this hand-over must stay synchronous, never behind an
        // await and never in a spawned task. The voter awaits `report()` inline
        // before it advances the view, so anything done synchronously here
        // happens-before the next view exists; the block finalized moments later is
        // derived from σ at the round recorded on the next line, and a miss there is
        // held until the beacon's seed wake-up. Deferring the record would lose that
        // race against the next finalization and leave execution a wake behind
        // consensus. The order relative to the executor send below is not
        // load-bearing: both are synchronous, and the executor reads the store only
        // on a spin-round mismatch for the canonical round recorded by an earlier
        // report.
        let observed = self
            .beacon
            .observe_certificate(ObservedCertificate::Notarization(n.proposal.round, &n));
        // A σ the beacon would not file is a σ this node may not speculate on.
        // `Refused` was recovered from partials this node had already verified and
        // still fails the epoch's own group key; `Pending` had no `PK_E` resolvable
        // here, so the certificate took vote-only admission and its σ slot was never
        // checked — speculating on it is the seed-blind divergence class, because
        // `prev_randao` rides the state root and a wrong σ forks rather than wastes
        // an attempt. Skipping costs no liveness: speculation is best-effort, the
        // height is derived from the finalized tier off the seed index, and a
        // `Pending` σ is settled on the `KeyAvailable` edge. Blanking the seed
        // instead would be the defect, because a beacon-active height derived with
        // `None` re-rolls `prev_randao`. `Inactive` is the ordinary pre-beacon
        // answer and must not skip.
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
