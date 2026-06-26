//! DKG-log recovery over `commonware_resolver::p2p` (§8.11.1).
//!
//! A committee member that restarts mid-window resumes its ceremony PLAYER-ONLY
//! (`ceremony::resume`) but may still LACK some peer dealer logs it never received
//! (the public, signed, self-verifying half of the ceremony). Those logs are
//! re-fetched here via the architecture's OWN recovery primitive — the same
//! `commonware_resolver::p2p` engine the marshal rides for cert backfill — keyed by
//! `{epoch, dealer}` (a key the node already knows from the committee roster). This
//! REPLACES the former best-effort `BEACON_CHANNEL` `LogRequest`/`LogResponse`
//! gossip pull: the resolver owns retry / multi-peer fallback / `fetch_targeted` /
//! rate-limiting / blocked-peer eviction, and serves ONE ~8.4 KiB log per key (never
//! a 430 KiB blob).
//!
//! Reachability (verified): the beacon plane's `EpochTransition` tracks
//! `active_registry_peers ∪ committee[E]` on the SAME `OracleHandle` the resolver's
//! `Provider` reads, so during E-1 (when committee[E] is dealing) the log holders are
//! in `latest.primary` via the registry union; targeted fetches aim at the known
//! roster.
//!
//! Wiring mirrors `marshal::resolver::handler`: [`LogHandler`] implements both
//! `Producer` (serve a `SignedDealerLog` from the live ceremony's `signed_logs` +
//! the persisted journal) and `Consumer` (re-`check`-ingest a fetched log via the
//! ceremony's existing peer-Reveal path), forwarding each to the single-threaded
//! [`crate::beacon::actor::DkgActor`] run loop over an mpsc channel + a oneshot reply
//! (so the actor stays the sole owner of ceremony state, no shared locks).

use bytes::{Buf, BufMut, Bytes};
use commonware_codec::{EncodeSize, Error as CodecError, Read, ReadExt as _, Write};
use commonware_resolver::{p2p::Producer, Consumer};
use commonware_utils::{
    channel::{mpsc, oneshot},
    Span,
};
use core::mem::size_of;
use fluentbase_bls::PeerPubkey;
use std::fmt::{Debug, Display, Formatter};
use tracing::error;

/// Resolver key for one dealer's public log in one ceremony: `{epoch, dealer}`.
///
/// A composite `Span` (variable-size `Ord + Hash + Codec<Cfg = ()>` key): `u64`
/// epoch ‖ 32-byte ed25519 dealer pubkey. NOT a content digest — the node already
/// knows the roster, so it enumerates exactly the dealer keys it lacks and fetches
/// those (no broadcast-and-hope). `Ord`/`Hash` derive from the fields; the codec is
/// fixed-layout so it round-trips byte-identically network-wide.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DkgLogKey {
    pub epoch: u64,
    pub dealer: PeerPubkey,
}

impl Debug for DkgLogKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "DkgLogKey{{epoch={}, dealer={}}}", self.epoch, self.dealer)
    }
}

impl Display for DkgLogKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "dkg-log[e{}/{}]", self.epoch, self.dealer)
    }
}

impl Write for DkgLogKey {
    fn write(&self, buf: &mut impl BufMut) {
        self.epoch.write(buf);
        self.dealer.write(buf);
    }
}

impl EncodeSize for DkgLogKey {
    fn encode_size(&self) -> usize {
        size_of::<u64>() + self.dealer.encode_size()
    }
}

impl Read for DkgLogKey {
    type Cfg = ();

    fn read_cfg(buf: &mut impl Buf, _: &()) -> Result<Self, CodecError> {
        let epoch = u64::read(buf)?;
        let dealer = PeerPubkey::read(buf)?;
        Ok(Self { epoch, dealer })
    }
}

impl Span for DkgLogKey {}

/// A request from the resolver engine's `Producer`/`Consumer` to the `DkgActor`.
/// The actor owns the ceremony state single-threaded; these cross the boundary so
/// the resolver never touches it directly.
pub enum LogMessage {
    /// Serve a `SignedDealerLog` for `key` (the resolver received an inbound
    /// request). The actor replies with the encoded log bytes, or DROPS the
    /// responder (→ the resolver sends an empty "no data" response → the requester
    /// retries another peer).
    Produce {
        key: DkgLogKey,
        response: oneshot::Sender<Bytes>,
    },
    /// Re-`check`-ingest a fetched log for `key`. The actor replies `true` iff the
    /// log is valid (recorded → fetch complete) or `false` for a GENUINE forgery
    /// (the resolver then blocks the lying peer). An honest-but-unusable log (e.g.
    /// the ceremony already finalized/evicted) replies `true` to avoid blocking an
    /// honest peer.
    Deliver {
        key: DkgLogKey,
        value: Bytes,
        response: oneshot::Sender<bool>,
    },
}

/// Bridges the resolver engine's `Producer`/`Consumer` traits to the `DkgActor`
/// run loop. Cloned into the engine for both roles, like marshal's `Handler`.
#[derive(Clone)]
pub struct LogHandler {
    sender: mpsc::Sender<LogMessage>,
}

impl LogHandler {
    pub const fn new(sender: mpsc::Sender<LogMessage>) -> Self {
        Self { sender }
    }
}

impl Consumer for LogHandler {
    type Key = DkgLogKey;
    type Value = Bytes;
    type Failure = ();

    async fn deliver(&mut self, key: Self::Key, value: Self::Value) -> bool {
        let (response, receiver) = oneshot::channel();
        if self
            .sender
            .send(LogMessage::Deliver {
                key,
                value,
                response,
            })
            .await
            .is_err()
        {
            error!("dkg log resolver: deliver to DkgActor failed (receiver dropped)");
            return false;
        }
        receiver.await.unwrap_or(false)
    }

    async fn failed(&mut self, _: Self::Key, _: Self::Failure) {
        // No-op: the resolver retries on its own; a permanently-unavailable log is
        // the accepted off-chain residual (the member sits out that epoch).
    }
}

impl Producer for LogHandler {
    type Key = DkgLogKey;

    async fn produce(&mut self, key: Self::Key) -> oneshot::Receiver<Bytes> {
        let (response, receiver) = oneshot::channel();
        if self
            .sender
            .send(LogMessage::Produce { key, response })
            .await
            .is_err()
        {
            error!("dkg log resolver: produce to DkgActor failed (receiver dropped)");
        }
        receiver
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_codec::{DecodeExt as _, Encode as _};
    use commonware_cryptography::{ed25519::PrivateKey, Signer as _};
    use commonware_math::algebra::Random as _;
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng as _;

    #[test]
    fn dkg_log_key_round_trips_and_orders_by_epoch_then_dealer() {
        let mut rng = StdRng::seed_from_u64(3);
        let a = PrivateKey::random(&mut rng).public_key();
        let b = PrivateKey::random(&mut rng).public_key();
        let key = DkgLogKey {
            epoch: 7,
            dealer: a.clone(),
        };
        let decoded = DkgLogKey::decode(key.encode().as_ref()).expect("decode");
        assert_eq!(decoded.epoch, 7);
        assert_eq!(decoded.dealer, a);
        // Epoch is the primary sort key (the u64 leads the layout).
        let lo = DkgLogKey {
            epoch: 6,
            dealer: b.clone(),
        };
        let hi = DkgLogKey {
            epoch: 7,
            dealer: b,
        };
        assert!(lo < hi, "lower epoch orders first regardless of dealer");
    }
}
