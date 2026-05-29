//! Slasher mailbox: unbounded Reporter sink for Activity events.
//!
//! `mpsc::UnboundedSender` + `error!` on disconnect.
//! Accountability-critical — dropping evidence = silent safety regression;
//! queue growth is bounded by adversarial event rate.

// **** наверное можно пренести в больший файл

use commonware_consensus::{simplex::types::Activity, Reporter};
use fluentbase_bls::Scheme as BlsScheme;
use tokio::sync::mpsc;
use tracing::error;

use crate::digest::Digest;

/// One Activity event delivered from the simplex engine.
pub type Message = Activity<BlsScheme, Digest>;

/// Reporter sink for the slasher actor. Implements
/// [`commonware_consensus::Reporter`] so it can be installed as the
/// second arm of the simplex `Reporters` multiplex.
#[derive(Clone, Debug)]
pub struct Mailbox {
    tx: mpsc::UnboundedSender<Message>,
}

impl Mailbox {
    pub(super) fn new(tx: mpsc::UnboundedSender<Message>) -> Self {
        Self { tx }
    }
}

/// Test-only constructor: build a `Mailbox` directly from a sender, bypassing
/// `Actor::init` (which constructs an HTTP `DynProvider`, incompatible with
/// the commonware deterministic runtime). Used by `tests/slasher_integration.rs`
/// to exercise the simplex Reporter multiplex without spinning up a full
/// alloy-provider stack.
/// **** а почему не #[test]  ?
#[doc(hidden)]
pub fn test_only_mailbox(tx: mpsc::UnboundedSender<Message>) -> Mailbox {
    Mailbox::new(tx)
}

impl Reporter for Mailbox {
    type Activity = Message;

    async fn report(&mut self, activity: Self::Activity) {
        if self.tx.send(activity).is_err() {
            error!("slasher mailbox closed; dropping activity");
        }
    }
}
