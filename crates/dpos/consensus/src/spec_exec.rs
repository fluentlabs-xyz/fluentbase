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
    beacon::{certify::SeedStore, types::Seed},
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
    /// Shared `round → recovered seed` map for the Stage-2 beacon certify gate
    /// ([`crate::beacon::certify`]). This reporter is the WRITER: the
    /// notarization carries the recovered seed, and it fires (via the voter's
    /// `notify` → `try_broadcast_notarization`) BEFORE the next loop iteration
    /// calls `certify` for the same round. `None` ⇒ no certify gate wired
    /// (tests / pre-beacon configs).
    seed_store: Option<SeedStore>,
}

impl Mailbox {
    pub fn new(executor: executor::Mailbox, seed_store: Option<SeedStore>) -> Self {
        Self {
            executor,
            seed_store,
        }
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
        // Record the recovered seed for the certify gate (round-keyed). Fired
        // here, before `certify(round, _)` runs next loop iteration.
        if let (Some(store), Some(s)) = (self.seed_store.as_ref(), seed.as_ref()) {
            store.record(s.target_round, s.signature);
        }
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
