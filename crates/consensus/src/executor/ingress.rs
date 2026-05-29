//! Executor mailbox + command/message types.
//!
//! Uses `tokio::sync::mpsc::unbounded_channel` (NOT
//! `futures::channel::mpsc`). Sync `send`; trivially convertible to async
//! for `Reporter::report`.

use alloy_primitives::Bytes;
use alloy_rpc_types_engine::PayloadId;
use commonware_consensus::{marshal::Update, types::Height};
use futures::channel::oneshot;
use tokio::sync::mpsc;
use tracing::Span;

use crate::{block::Block, digest::Digest};

/// Typed error returned by the executor on canonicalize commands.
/// Replaces the previous opaque `eyre::Result` so callers can
/// distinguish backfill-rejected commands from genuine engine failures.
#[derive(Debug, thiserror::Error)]
pub enum CanonicalizeError {
    #[error("executor is backfilling; retry after backfill completes")]
    BackfillInProgress,
    #[error("FCU succeeded but engine returned no PayloadId")]
    PayloadIdMissing,
    #[error("engine error: {0}")]
    EngineError(#[source] eyre::Report),
}

// **** давай схлопним 3 файла в 1

/// One executor command paired with its tracing span (preserves the
/// causal `parent` for `#[instrument]` in `actor.rs`).
pub struct Message<Attrs> {
    pub cause: Span,
    pub command: Command<Attrs>,
}

pub enum Command<Attrs> {
    /// FCU + build payload (propose path).
    CanonicalizeAndBuild(CanonicalizeAndBuild<Attrs>),
    /// Forward a finalized block to the EL (`Update::Block`) or
    /// just refresh the finalized tip (`Update::Tip`).
    Finalize(Box<Update<Block>>),
}

pub struct CanonicalizeAndBuild<Attrs> {
    pub height: Height,
    pub digest: Digest,
    pub attributes: Box<Attrs>,
    /// Liveness-cert bytes to insert into the shared registry against
    /// the engine-assigned PayloadId, atomically between FCU return and
    /// the response.send. Empty → skip insert (cold-start / non-DPoS).
    /// Closes the race window: the FluentPayloadBuilder's
    /// try_build runs on a `tokio::task::spawn_blocking` worker
    /// (reth basic/lib.rs:352) that may begin executing before this
    /// command's response is delivered to FluentApp::propose; inserting
    /// on the FCU-await task itself shrinks the race to microseconds.
    /// `MissingPayloadBehaviour::RaceEmptyPayload` (payload.rs:183-188)
    /// bounds the worst case to one Nullify view per lost race.
    pub extra_data: Bytes,
    pub response: oneshot::Sender<Result<PayloadId, CanonicalizeError>>,
}

pub struct Mailbox<Attrs> {
    tx: mpsc::UnboundedSender<Message<Attrs>>,
}

// Manual Clone impl — `Attrs` need not be Clone for the mailbox itself
// (`UnboundedSender` is Clone unconditionally).
impl<Attrs> Clone for Mailbox<Attrs> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
        }
    }
}

impl<Attrs> Mailbox<Attrs> {
    pub(super) fn new(tx: mpsc::UnboundedSender<Message<Attrs>>) -> Self {
        Self { tx }
    }

    /// Test-only constructor used by `application.rs` unit tests to inject a
    /// drain-only mailbox without spawning a real executor.
    #[cfg(test)]
    pub(crate) fn new_for_test(tx: mpsc::UnboundedSender<Message<Attrs>>) -> Self {
        Self { tx }
    }

    /// Sync send — `tokio::sync::mpsc::UnboundedSender::send` never blocks.
    // SendError<Message> carries the rejected message verbatim so the
    // caller can retry; boxing solely to silence the lint would add an
    // alloc on the hot path.
    #[allow(clippy::result_large_err)]
    pub fn send(&self, msg: Message<Attrs>) -> Result<(), mpsc::error::SendError<Message<Attrs>>> {
        self.tx.send(msg)
    }
}
