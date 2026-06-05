//! Executor mailbox + command/message types.
//!
//! Uses `tokio::sync::mpsc::unbounded_channel` (NOT
//! `futures::channel::mpsc`). Sync `send`; trivially convertible to async
//! for `Reporter::report`.

use crate::{block::Block, digest::Digest};
use alloy_rpc_types_engine::PayloadId;
use commonware_consensus::{marshal::Update, types::Height};
use futures::channel::oneshot;
use tokio::sync::mpsc;
use tracing::Span;

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
