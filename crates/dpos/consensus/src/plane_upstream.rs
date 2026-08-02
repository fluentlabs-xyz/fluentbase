//! Plane-native [`CertUpstream`] over `commonware_resolver::p2p` (Gap A).
//!
//! A plain `--dpos` validator with no WS `--dpos.follower-upstream` still needs the
//! ONE capability the consensus plane lacks natively: FRONTIER DISCOVERY — the
//! cold-start / steady-state JUMP consumes [`CertUpstream::get_latest`] to obtain the
//! tip `(finalization, OrderBlock)` it drives reth EL-sync toward, and commonware's
//! marshal `Request` enum has no over-the-wire `Latest` variant (`BlockID::Latest`
//! resolves via a LOCAL archive read). This module adds that transport as a tiny
//! Fluent-side resolver channel (`FRONTIER_CHANNEL`) built on the same
//! `commonware_resolver::p2p::Engine` the beacon DKG-log resolver rides
//! (`beacon/log_resolver.rs`), serving each peer THIS node's LOCAL marshal tip /
//! archive — no execution, no marshal-serving change (`get_info`/`get_finalization`/
//! `get_block` are public, local-only Mailbox reads).
//!
//! [`PlaneUpstreamHandle`] implements [`CertUpstream`] verbatim, so it drops into the
//! `U: CertUpstream` generic (`cold_start_jump` / `ReJump` / `DposLayerConfig`) with
//! zero signature churn: a plane validator then uses the exact same
//! `MarshalResolver::Hybrid { plane, upstream }` path a WS validator uses today, only
//! the concrete `U` changes.
//!
//! Trust model (verbatim from the WS path): `get_latest` is trusted for EL-sync ONLY.
//! A malicious peer can at most inflate the tip → the jump runs a wasted backfill that
//! then FAILS CLOSED at `verify_jump_authenticated` (post-sync committee BLS) →
//! `rotate()`. The finalization returned still passes `verify_jump_structural` and the
//! post-sync committee BLS. So single-peer `get_latest` + rotate is safe (f+1
//! corroboration is optional hardening, dropped as YAGNI).

use crate::{
    cert_follow::{CertUpstream, UpstreamFinalized},
    digest::Digest,
    order_block::OrderBlock,
    outer::MarshalMailbox,
};
use bytes::{Buf, BufMut, Bytes};
use commonware_codec::{
    Decode as _, Encode as _, EncodeSize, Error as CodecError, Read, ReadExt as _, Write,
};
use commonware_consensus::{marshal::Identifier, simplex::types::Finalization, types::Height};
use commonware_cryptography::ed25519::PublicKey;
use commonware_resolver::{p2p::Producer, Consumer, Resolver as _};
use commonware_utils::{channel::oneshot as cw_oneshot, Span};
use fluentbase_bls::Scheme as BlsScheme;
use std::{
    collections::HashMap,
    fmt::{Debug, Display, Formatter},
    future::Future,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
use tokio::sync::oneshot;

/// The finalization-certificate type the frontier wire carries (marshal-format,
/// identical to the WS [`crate::certified_block`] pair the upstream serves).
type Cert = Finalization<BlsScheme, Digest>;

/// The client resolver mailbox `PlaneUpstreamHandle` issues fetches on.
pub type FrontierResolverMailbox = commonware_resolver::p2p::Mailbox<FrontierKey, PublicKey>;

/// Per-key correlation map: a delivered `(cert, block)` is fanned to every waiting
/// `get_latest`/`get_finalization` call for that key. Shared between the client
/// [`PlaneUpstreamHandle`] (registers waiters) and the serve-side [`FrontierHandler`]
/// (resolves them on `deliver`).
type Waiters = Arc<Mutex<HashMap<FrontierKey, Vec<oneshot::Sender<UpstreamFinalized>>>>>;

/// How long a single `get_latest`/`get_finalization` awaits a plane delivery before
/// returning `None`. Bounds the jump's single-shot `get_latest` so an isolated /
/// not-yet-tracked node returns `None` (→ `Lagging` → boot on the treadmill) rather
/// than hanging; the resolver's own retry/multi-peer fallback delivers well inside
/// this window when a tracked peer holds the data.
const FRONTIER_FETCH_TIMEOUT: Duration = Duration::from_secs(8);

/// Resolver request key: two subjects, fixed-layout `Span`.
///
/// `Latest` answers are time-varying — never cached; `Finalized { height }` is the
/// by-height arm the `MarshalResolver::Hybrid` upstream path pulls. `Ord`/`Hash`
/// derive from the fields (`Latest` orders before any `Finalized`); the codec is
/// fixed-layout (`Cfg = ()`) so it round-trips byte-identically network-wide.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FrontierKey {
    /// The peer's highest local finalized tip `(finalization, block)`.
    Latest,
    /// The `(finalization, block)` at an explicit height.
    Finalized { height: u64 },
}

impl Debug for FrontierKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Latest => write!(f, "FrontierKey::Latest"),
            Self::Finalized { height } => write!(f, "FrontierKey::Finalized{{height={height}}}"),
        }
    }
}

impl Display for FrontierKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Latest => write!(f, "frontier[latest]"),
            Self::Finalized { height } => write!(f, "frontier[h{height}]"),
        }
    }
}

impl Write for FrontierKey {
    fn write(&self, buf: &mut impl BufMut) {
        match self {
            Self::Latest => 0u8.write(buf),
            Self::Finalized { height } => {
                1u8.write(buf);
                height.write(buf);
            }
        }
    }
}

impl EncodeSize for FrontierKey {
    fn encode_size(&self) -> usize {
        match self {
            Self::Latest => 1,
            Self::Finalized { height } => 1 + height.encode_size(),
        }
    }
}

impl Read for FrontierKey {
    type Cfg = ();

    fn read_cfg(buf: &mut impl Buf, _: &()) -> Result<Self, CodecError> {
        match u8::read(buf)? {
            0 => Ok(Self::Latest),
            1 => Ok(Self::Finalized {
                height: u64::read(buf)?,
            }),
            tag => Err(CodecError::InvalidEnum(tag)),
        }
    }
}

impl Span for FrontierKey {}

/// Serve THIS node's local marshal tip/archive for `key` — no network, no execution.
/// Encodes `(finalization, block)` exactly as the WS upstream / marshal resolver do
/// (`cert_inlet.rs`), so `decode_frontier` on the client side round-trips it.
async fn serve(marshal: &MarshalMailbox, key: FrontierKey) -> Option<Bytes> {
    let (fin, block) = match key {
        FrontierKey::Latest => {
            let (h, _digest) = marshal.get_info(Identifier::Latest).await?;
            let fin = marshal.get_finalization(h).await?;
            // Re-read the block AT `h`, not `Identifier::Latest`: a block finalizing
            // between the two awaits would pair `fin@h` with `block@h+1`, which the
            // client's `verify_jump_structural` rejects as `BadTarget` — and on the
            // empty-archive onboarding path a single BadTarget aborts the launch.
            let block = marshal.get_block(h).await?;
            (fin, block)
        }
        FrontierKey::Finalized { height } => {
            let h = Height::new(height);
            let fin = marshal.get_finalization(h).await?;
            let block = marshal.get_block(h).await?;
            (fin, block)
        }
    };
    Some((fin, block).encode())
}

/// Decode a delivered frontier value into an [`UpstreamFinalized`]. The certificate
/// signer-bitmap is decoded with a BOUNDED cap (`MAX_COMMITTEE_SIZE`) — the value comes
/// from an untrusted peer, and the unbounded decoder eagerly allocates from a tiny
/// length prefix (audit R4-5, same guard as `CertifiedBlock::into_parts`). Exact
/// participant validation still happens at cert-verify time against the per-epoch
/// scheme.
fn decode_frontier(value: &[u8]) -> Option<UpstreamFinalized> {
    let cap = fluentbase_p2p::constants::MAX_COMMITTEE_SIZE as usize;
    let (finalization, block) = <(Cert, OrderBlock)>::decode_cfg(value, &(cap, ())).ok()?;
    Some(UpstreamFinalized {
        finalization,
        block,
    })
}

/// Bridges the resolver engine's `Producer` (serve local tip) + `Consumer` (resolve
/// awaiting client calls) to the shared [`Waiters`] map + the late-bound marshal.
/// Cloned into the engine for both roles, like `beacon::log_resolver::LogHandler`.
#[derive(Clone)]
pub struct FrontierHandler {
    waiters: Waiters,
    /// The node's own marshal, late-bound via the `marshal_slot` `OnceLock` (the
    /// marshal is created by the later layer launch). Until filled, `produce` serves
    /// "no data".
    marshal_slot: Arc<OnceLock<MarshalMailbox>>,
}

impl Consumer for FrontierHandler {
    type Key = FrontierKey;
    type Value = Bytes;
    type Failure = ();

    async fn deliver(&mut self, key: Self::Key, value: Self::Value) -> bool {
        // Never cache a delivered `Latest` (or any key): a valid decode fans the value
        // to the awaiting calls and removes them; an UNDECODABLE delivery returns
        // `false` (keeps the fetch alive so the resolver tries another peer, blocks the
        // bad source) — returning `true` on garbage would permanently satisfy the fetch.
        let Some(uf) = decode_frontier(value.as_ref()) else {
            return false;
        };
        if let Some(waiters) = self.waiters.lock().unwrap().remove(&key) {
            for tx in waiters {
                let _ = tx.send(uf.clone());
            }
        }
        true
    }

    async fn failed(&mut self, _: Self::Key, _: Self::Failure) {
        // No-op: the awaiting call times out on its own and cleans its waiter; the
        // marshal / jump re-requests on the next repair sweep / re-jump edge.
    }
}

impl Producer for FrontierHandler {
    type Key = FrontierKey;

    async fn produce(&mut self, key: Self::Key) -> cw_oneshot::Receiver<Bytes> {
        let (response, receiver) = cw_oneshot::channel();
        // Read the local marshal inline (rare, O(1), local mpsc round-trips). A missing
        // slot (marshal not yet launched) or an archive miss drops `response` unsent →
        // the resolver relays a "no data" response and the requester retries a peer.
        if let Some(marshal) = self.marshal_slot.get() {
            if let Some(bytes) = serve(marshal, key).await {
                let _ = response.send(bytes);
            }
        }
        receiver
    }
}

/// Build the frontier bridge: the [`FrontierHandler`] (Producer + Consumer, passed into
/// `commonware_resolver::p2p::Engine::new`) and the shared [`Waiters`] map that the
/// resulting [`PlaneUpstreamHandle`] registers waiters on. `marshal_slot` is the SAME
/// `OnceLock` the node fills after the layer launch (`node/dpos.rs`).
pub fn new_bridge(marshal_slot: Arc<OnceLock<MarshalMailbox>>) -> (FrontierHandler, Waiters) {
    let waiters: Waiters = Arc::new(Mutex::new(HashMap::new()));
    (
        FrontierHandler {
            waiters: waiters.clone(),
            marshal_slot,
        },
        waiters,
    )
}

/// A plane-native [`CertUpstream`] over the frontier resolver. Holds the client
/// resolver mailbox (to issue fetches) + the shared [`Waiters`] map (to await the
/// resolver's delivery). Cloneable (mpsc + `Arc`), `Send + Sync + 'static`.
#[derive(Clone)]
pub struct PlaneUpstreamHandle {
    mailbox: FrontierResolverMailbox,
    waiters: Waiters,
}

impl PlaneUpstreamHandle {
    pub fn new(mailbox: FrontierResolverMailbox, waiters: Waiters) -> Self {
        Self { mailbox, waiters }
    }

    /// Register a waiter, issue an (untargeted) fetch, and await the delivery with a
    /// bounded timeout. The resolver picks + rotates peers from the tracked set on its
    /// own, so this is `fetch` (not `fetch_targeted`); with an empty tracked set the
    /// fetch is a no-op and this returns `None` after the timeout.
    async fn fetch_one(&self, key: FrontierKey) -> Option<UpstreamFinalized> {
        let (tx, rx) = oneshot::channel();
        self.waiters
            .lock()
            .unwrap()
            .entry(key)
            .or_default()
            .push(tx);
        let mut mailbox = self.mailbox.clone();
        mailbox.fetch(key).await;
        match tokio::time::timeout(FRONTIER_FETCH_TIMEOUT, rx).await {
            Ok(Ok(uf)) => {
                tracing::debug!(%key, height = uf.block.height, "frontier fetch delivered");
                Some(uf)
            }
            _ => {
                tracing::debug!(%key, "frontier fetch timed out (no tracked peer served it)");
                // Timed out (or the sender was dropped): prune the now-closed waiter and,
                // if this key has no remaining waiters, cancel the in-flight fetch so the
                // resolver stops probing peers for it.
                let empty = {
                    let mut waiters = self.waiters.lock().unwrap();
                    let empty = match waiters.get_mut(&key) {
                        Some(v) => {
                            v.retain(|s| !s.is_closed());
                            v.is_empty()
                        }
                        None => false,
                    };
                    if empty {
                        waiters.remove(&key);
                    }
                    empty
                };
                if empty {
                    mailbox.cancel(key).await;
                }
                None
            }
        }
    }
}

impl CertUpstream for PlaneUpstreamHandle {
    fn get_finalization(
        &self,
        height: Height,
    ) -> impl Future<Output = Option<UpstreamFinalized>> + Send {
        let this = self.clone();
        async move {
            this.fetch_one(FrontierKey::Finalized {
                height: height.get(),
            })
            .await
        }
    }

    fn get_latest(&self) -> impl Future<Output = Option<UpstreamFinalized>> + Send {
        let this = self.clone();
        async move { this.fetch_one(FrontierKey::Latest).await }
    }

    fn rotate(&self) -> impl Future<Output = ()> + Send {
        // Best-effort: cancel the current frontier fetch so the next `get_latest`
        // re-issues a fresh one (the resolver already does multi-peer fallback, so
        // there is no per-peer cursor to advance — the executor `ReJump.rotate` and
        // inlet data-fault rotation map onto this unchanged).
        let mut mailbox = self.mailbox.clone();
        async move {
            mailbox.cancel(FrontierKey::Latest).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_codec::DecodeExt as _;

    #[test]
    fn frontier_key_round_trips_and_orders_latest_before_finalized() {
        for key in [
            FrontierKey::Latest,
            FrontierKey::Finalized { height: 0 },
            FrontierKey::Finalized { height: 987 },
        ] {
            let decoded = FrontierKey::decode(key.encode().as_ref()).expect("decode");
            assert_eq!(decoded, key);
        }
        assert!(FrontierKey::Latest < FrontierKey::Finalized { height: 0 });
        assert!(FrontierKey::Finalized { height: 5 } < FrontierKey::Finalized { height: 6 });
    }

    #[test]
    fn frontier_key_rejects_unknown_tag() {
        let err = FrontierKey::decode([7u8].as_ref()).unwrap_err();
        assert!(matches!(err, CodecError::InvalidEnum(7)));
    }

    #[test]
    fn decode_frontier_rejects_garbage() {
        assert!(decode_frontier(b"not a certified pair").is_none());
    }

    // A delivered value fans out to every awaiting `get_latest`/`get_finalization`
    // registered under that key, and the waiter map is left empty afterwards (no
    // caching of `Latest`). Exercised without a live resolver by driving the shared
    // `Waiters` map directly (the exact seam `deliver` resolves).
    #[tokio::test]
    async fn undecodable_delivery_is_false_and_keeps_waiters_uncached() {
        let (mut handler, waiters) = new_bridge(Arc::new(OnceLock::new()));

        let (tx1, mut rx1) = oneshot::channel();
        let (tx2, mut rx2) = oneshot::channel();
        waiters
            .lock()
            .unwrap()
            .insert(FrontierKey::Latest, vec![tx1, tx2]);

        // A valid decode requires a real multisig `(Cert, OrderBlock)` fixture (no
        // fluentbase-side constructor — exercised end-to-end in the plane-jump smoke), so
        // this asserts the undecodable path: `deliver` returns `false` (keeps the fetch
        // alive / blocks the bad source) and LEAVES the awaiting waiters registered — the
        // never-cache-a-bad-Latest invariant.
        let ok = handler
            .deliver(FrontierKey::Latest, Bytes::from_static(b"garbage"))
            .await;
        assert!(!ok, "undecodable delivery must return false");
        assert!(
            waiters.lock().unwrap().contains_key(&FrontierKey::Latest),
            "an undecodable delivery must not drop the awaiting waiters"
        );
        assert!(rx1.try_recv().is_err() && rx2.try_recv().is_err());
    }
}
