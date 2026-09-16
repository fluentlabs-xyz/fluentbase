//! Slasher mailbox: unbounded sink for Activity events, and the provenance
//! stamped on each one.
//!
//! Unbounded because accountability is critical — dropping evidence is a silent
//! safety regression, and queue growth is bounded by the adversarial event rate.

use crate::digest::Digest;
use commonware_consensus::{simplex::types::Activity, Reporter};
use fluentbase_bls::Scheme as BlsScheme;
use tokio::sync::mpsc;
use tracing::error;

/// One Activity event delivered from the simplex engine.
pub type Message = Activity<BlsScheme, Digest>;

/// Where a mailbox entry came from.
///
/// Load-bearing, not bookkeeping. The slasher's epoch cursor
/// ([`crate::slasher::actor::EpochCursor`]) is the pivot the vote store retains
/// around and the charge queue drains on, and only this node's own simplex engine is
/// evidence that consensus has actually reached an epoch. A peer-forwarded vote
/// merely names an epoch of its signer's choosing: signing is not bound to the live
/// view, and the two-epoch ahead-commit horizon puts committees for `E+1` and `E+2`
/// on chain, so one member of a future committee could sign a single genuine
/// `Notarize` for `(E+2, view 0)` and drive every receiver's cursor two epochs
/// forward.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provenance {
    /// Reported by this node's own simplex engine through [`Reporter`]. The only
    /// provenance permitted to move the epoch cursor.
    Engine,
    /// Forwarded by a peer on the evidence channel
    /// ([`crate::slasher::gossip`]). May add votes; never moves the cursor.
    Gossip,
}

/// One mailbox entry: an activity and where it came from.
#[derive(Clone, Debug)]
pub struct Envelope {
    pub activity: Message,
    pub provenance: Provenance,
}

/// Reporter sink for the slasher actor. Implements
/// [`commonware_consensus::Reporter`] so it can be installed as the second arm of
/// the simplex `Reporters` multiplex.
///
/// Everything arriving through the [`Reporter`] impl is stamped
/// [`Provenance::Engine`], and this is the only type that can stamp it.
#[derive(Clone, Debug)]
pub struct Mailbox {
    tx: mpsc::UnboundedSender<Envelope>,
}

impl Mailbox {
    pub(super) fn new(tx: mpsc::UnboundedSender<Envelope>) -> Self {
        Self { tx }
    }

    /// The gossip-side half of this mailbox.
    ///
    /// A distinct type rather than a second method on `Mailbox`: the evidence
    /// consumer is handed one of these and therefore cannot stamp
    /// [`Provenance::Engine`].
    pub(super) fn gossip_sink(&self) -> GossipSink {
        GossipSink {
            tx: self.tx.clone(),
        }
    }
}

impl Reporter for Mailbox {
    type Activity = Message;

    async fn report(&mut self, activity: Self::Activity) {
        if self
            .tx
            .send(Envelope {
                activity,
                provenance: Provenance::Engine,
            })
            .is_err()
        {
            error!("slasher mailbox closed; dropping activity");
        }
    }
}

/// The evidence channel's write end into the slasher mailbox.
///
/// Deliberately not a [`Reporter`]: `Reporter::report` is the engine's word for
/// "this happened", and a forwarded vote is only ever "a peer says this was signed".
#[derive(Clone, Debug)]
pub struct GossipSink {
    tx: mpsc::UnboundedSender<Envelope>,
}

impl GossipSink {
    /// Hand one peer-forwarded, already signature-verified vote to the slasher's
    /// vote store.
    ///
    /// Public, and safe to be: everything entering here is stamped
    /// [`Provenance::Gossip`], so a holder can add votes and can never move the epoch
    /// cursor.
    pub fn report_gossiped(&self, activity: Message) {
        if self
            .tx
            .send(Envelope {
                activity,
                provenance: Provenance::Gossip,
            })
            .is_err()
        {
            error!("slasher mailbox closed; dropping forwarded evidence");
        }
    }
}

/// Test-only constructor: build a `Mailbox` directly from a sender, bypassing
/// `Actor::init` (which constructs an HTTP `DynProvider`, incompatible with the
/// commonware deterministic runtime). Used to exercise the simplex Reporter
/// multiplex without spinning up a full alloy-provider stack.
#[doc(hidden)]
pub fn test_only_mailbox(tx: mpsc::UnboundedSender<Envelope>) -> Mailbox {
    Mailbox::new(tx)
}
