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
    beacon::types::Seed,
    executor,
    executor::{Command, Notarized},
};
use commonware_consensus::{simplex::types::Activity, Reporter};
use fluentbase_bls::Scheme as BlsScheme;
use tracing::{error, Span};

type Digest = crate::digest::Digest;

/// Reporter sink that converts `Activity::Notarization` into the executor's
/// speculative command. All other activities are ignored.
#[derive(Clone)]
pub struct Mailbox {
    executor: executor::Mailbox,
}

impl Mailbox {
    pub fn new(executor: executor::Mailbox) -> Self {
        Self { executor }
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
        let msg = executor::Message {
            cause: Span::current(),
            command: Command::SpecNotarized(Box::new(Notarized {
                round: n.proposal.round,
                digest: n.proposal.payload,
                seed,
            })),
        };
        if self.executor.send(msg).is_err() {
            error!("executor mailbox closed; dropping notarization");
        }
    }
}
