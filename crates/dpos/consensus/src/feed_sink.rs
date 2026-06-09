//! [`FeedSink`] — a passive consensus `Reporter` that forwards each finalized
//! block height to a node-side feed actor over a channel.
//!
//! It lives in this crate (not the node crate) so it can be wired as the
//! marshal's second application-`Reporter` (`Reporters::from((app, feed))`)
//! without the consensus crate naming any node type. A trait object is
//! impossible — commonware `Reporter` is `Clone + Send` with `report(..) ->
//! impl Future` (RPITIT), so it is not object-safe — and a generic feed param
//! would balloon the already-8-generic `OuterEngine`; a concrete channel sink
//! is the minimal wiring.

use crate::block::Block;
use commonware_consensus::{marshal::Update, types::Height, Heightable as _, Reporter};
use commonware_utils::Acknowledgement as _;
use tokio::sync::mpsc;

/// The marshal-side end of the feed channel: forwards finalized heights to the
/// node feed actor, which fetches `(cert, block)` and serves the `consensus` RPC.
#[derive(Clone)]
pub struct FeedSink {
    tx: mpsc::UnboundedSender<Height>,
}

impl FeedSink {
    /// Build a sink plus the receiver the node feed actor drains.
    pub fn channel() -> (Self, mpsc::UnboundedReceiver<Height>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Self { tx }, rx)
    }
}

impl Reporter for FeedSink {
    type Activity = Update<Block>;

    async fn report(&mut self, activity: Update<Block>) {
        match activity {
            // Forward the height, then acknowledge immediately: the feed is a
            // passive observer and must never add backpressure (the marshal
            // still gates delivery on the executor's slower ack). Dropping the
            // `Exact` instead would trip marshal's fatal-shutdown cascade.
            Update::Block(block, ack) => {
                let _ = self.tx.send(block.height());
                ack.acknowledge();
            }
            Update::Tip(_, height, _) => {
                let _ = self.tx.send(height);
            }
        }
    }
}
