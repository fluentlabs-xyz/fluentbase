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
use commonware_resolver::{p2p::Producer, Consumer, Resolver};
use commonware_utils::{
    channel::{mpsc, oneshot},
    vec::NonEmptyVec,
    Span,
};
use core::mem::size_of;
use fluentbase_bls::PeerPubkey;
use std::fmt::{Debug, Display, Formatter};
use tracing::error;

use crate::beacon::artifact::ArtifactBridge;

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
        write!(
            f,
            "DkgLogKey{{epoch={}, dealer={}}}",
            self.epoch, self.dealer
        )
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

/// Everything this node pulls on `BEACON_RESOLVER_CHANNEL`.
///
/// Two subjects on ONE engine, deliberately. The epoch-key artifact needs a pull
/// seam at a 16/s quota and never a 128/s consensus one, because an inbound
/// over-quota SLEEPS THE WHOLE CONNECTION to a peer and stalls its other
/// channels; `BEACON_RESOLVER_CHANNEL` is already that quota, already tracks the
/// right peer set (`registry ∪ committee`), and is already wired. Adding a
/// top-level channel would have bought a second engine and a network-wide
/// coordinated release for traffic that belongs on this one.
///
/// The tag byte is the marshal request codec's shape (`resolver/handler.rs`
/// keys). It is a WIRE change on this channel — a key that used to be a bare
/// `{epoch, dealer}` now leads with a discriminant — and therefore a coordinated
/// release, which the `OrderBlock` shrink makes anyway.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BeaconFetchKey {
    /// One dealer's public log in one ceremony.
    Log(DkgLogKey),
    /// The quorum-signed agreement artifact for one target epoch — the ONLY
    /// source of `PK_epoch` for a node that never ran the ceremony.
    Artifact { epoch: u64 },
}

impl BeaconFetchKey {
    const TAG_LOG: u8 = 0;
    const TAG_ARTIFACT: u8 = 1;
}

impl Debug for BeaconFetchKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Log(key) => write!(f, "BeaconFetchKey::{key:?}"),
            Self::Artifact { epoch } => write!(f, "BeaconFetchKey::Artifact{{epoch={epoch}}}"),
        }
    }
}

impl Display for BeaconFetchKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Log(key) => write!(f, "{key}"),
            Self::Artifact { epoch } => write!(f, "dkg-artifact[e{epoch}]"),
        }
    }
}

impl Write for BeaconFetchKey {
    fn write(&self, buf: &mut impl BufMut) {
        match self {
            Self::Log(key) => {
                Self::TAG_LOG.write(buf);
                key.write(buf);
            }
            Self::Artifact { epoch } => {
                Self::TAG_ARTIFACT.write(buf);
                epoch.write(buf);
            }
        }
    }
}

impl EncodeSize for BeaconFetchKey {
    fn encode_size(&self) -> usize {
        size_of::<u8>()
            + match self {
                Self::Log(key) => key.encode_size(),
                Self::Artifact { epoch } => epoch.encode_size(),
            }
    }
}

impl Read for BeaconFetchKey {
    type Cfg = ();

    fn read_cfg(buf: &mut impl Buf, _: &()) -> Result<Self, CodecError> {
        match u8::read(buf)? {
            Self::TAG_LOG => Ok(Self::Log(DkgLogKey::read(buf)?)),
            Self::TAG_ARTIFACT => Ok(Self::Artifact {
                epoch: u64::read(buf)?,
            }),
            tag => Err(CodecError::InvalidEnum(tag)),
        }
    }
}

impl Span for BeaconFetchKey {}

/// The dealer-log half of the shared resolver, presented as a resolver whose key
/// space is only [`DkgLogKey`].
///
/// The ceremony fetches logs and nothing else, so it keeps the narrow key it
/// always had; the artifact seam takes the other arm. Without this adapter the
/// widened key would leak into every `fetch` in the DKG actor and the agreement
/// automaton, where an `Artifact` key is not a thing that can be asked for.
#[derive(Clone)]
pub struct LogFetcher<R>(R);

impl<R> LogFetcher<R> {
    pub const fn new(inner: R) -> Self {
        Self(inner)
    }
}

impl<R: Resolver<Key = BeaconFetchKey>> Resolver for LogFetcher<R> {
    type Key = DkgLogKey;
    type PublicKey = R::PublicKey;

    async fn fetch(&mut self, key: Self::Key) {
        self.0.fetch(BeaconFetchKey::Log(key)).await;
    }

    async fn fetch_all(&mut self, keys: Vec<Self::Key>) {
        self.0
            .fetch_all(keys.into_iter().map(BeaconFetchKey::Log).collect())
            .await;
    }

    async fn fetch_targeted(&mut self, key: Self::Key, targets: NonEmptyVec<Self::PublicKey>) {
        self.0
            .fetch_targeted(BeaconFetchKey::Log(key), targets)
            .await;
    }

    async fn fetch_all_targeted(
        &mut self,
        requests: Vec<(Self::Key, NonEmptyVec<Self::PublicKey>)>,
    ) {
        self.0
            .fetch_all_targeted(
                requests
                    .into_iter()
                    .map(|(key, targets)| (BeaconFetchKey::Log(key), targets))
                    .collect(),
            )
            .await;
    }

    async fn cancel(&mut self, key: Self::Key) {
        self.0.cancel(BeaconFetchKey::Log(key)).await;
    }

    async fn clear(&mut self) {
        // Deliberately NOT `inner.clear()`: this handle speaks for the ceremony,
        // and cancelling the artifact seam's in-flight pull on its behalf would
        // strand a caller that is waiting on a key this side does not own.
        self.0
            .retain(|key| !matches!(key, BeaconFetchKey::Log(_)))
            .await;
    }

    async fn retain(&mut self, predicate: impl Fn(&Self::Key) -> bool + Send + 'static) {
        self.0
            .retain(move |key| match key {
                BeaconFetchKey::Log(log) => predicate(log),
                // Same reason as `clear`: not this side's fetch to drop.
                BeaconFetchKey::Artifact { .. } => true,
            })
            .await;
    }
}

/// The shared resolver's `Producer`/`Consumer`, dispatching each key to the half
/// that owns it.
///
/// Both halves obey the same rule and it is the one that keeps commonware's
/// un-removable `excluded` set out of this engine: `deliver` returns `false` ONLY
/// for proven misbehaviour — a genuine forgery — and `true` for everything an
/// honest peer can legitimately send, including a log this node can no longer use
/// and an artifact it does not have.
#[derive(Clone)]
pub struct BeaconFetchHandler {
    logs: LogHandler,
    artifacts: ArtifactBridge,
}

impl BeaconFetchHandler {
    pub const fn new(logs: LogHandler, artifacts: ArtifactBridge) -> Self {
        Self { logs, artifacts }
    }
}

impl Consumer for BeaconFetchHandler {
    type Key = BeaconFetchKey;
    type Value = Bytes;
    type Failure = ();

    async fn deliver(&mut self, key: Self::Key, value: Self::Value) -> bool {
        match key {
            BeaconFetchKey::Log(key) => self.logs.deliver(key, value).await,
            BeaconFetchKey::Artifact { epoch } => self.artifacts.deliver(epoch, value.as_ref()),
        }
    }

    async fn failed(&mut self, key: Self::Key, failure: Self::Failure) {
        // Nothing here waits on `failed`, and that is the point: it fires solely
        // on Cancel/Retain/Clear and never on an error or a timeout, so a seam
        // that relied on it would leave its caller in unbounded silent retry. The
        // artifact seam's caller learns from a delivered `NotYet` or from its own
        // bounded window; the log side re-issues its missing targets each tick.
        if let BeaconFetchKey::Log(key) = key {
            self.logs.failed(key, failure).await;
        }
    }
}

impl Producer for BeaconFetchHandler {
    type Key = BeaconFetchKey;

    async fn produce(&mut self, key: Self::Key) -> oneshot::Receiver<Bytes> {
        match key {
            BeaconFetchKey::Log(key) => self.logs.produce(key).await,
            BeaconFetchKey::Artifact { epoch } => {
                // ALWAYS an answer, never a dropped responder: a producer that
                // has nothing says `NotYet`, which is what lets the requester
                // distinguish "not converged yet" from "nobody answered".
                let (response, receiver) = oneshot::channel();
                drop(response.send(self.artifacts.produce(epoch)));
                receiver
            }
        }
    }
}

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
        // No-op retry: the resolver retries on its own AND the actor re-issues the
        // missing `{epoch, dealer}` targets each tick (off the live ceremony in-window
        // and off `recompute_pending` past the boundary, within the journal-retention
        // window), so a transiently-unavailable log is re-fetched to completion rather
        // than sat out. A log NO peer holds simply backs off with the epoch's age-out
        // (the fetch set is bounded to `dealers(E) − held`), so this is not a storm.
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
    use commonware_runtime::Runner as _;
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng as _;

    /// The shared key space: both subjects round-trip, an unknown tag is refused,
    /// and no `Log` key can ever decode as an `Artifact` key or the reverse — the
    /// two halves ride ONE engine (`BEACON_RESOLVER_CHANNEL`, 16/s) and the
    /// discriminant is the only thing keeping them apart on the wire.
    #[test]
    fn the_shared_key_space_separates_its_two_subjects() {
        let mut rng = StdRng::seed_from_u64(11);
        let dealer = PrivateKey::random(&mut rng).public_key();
        let log = BeaconFetchKey::Log(DkgLogKey {
            epoch: 7,
            dealer: dealer.clone(),
        });
        let artifact = BeaconFetchKey::Artifact { epoch: 7 };
        assert_eq!(
            BeaconFetchKey::decode(log.encode().as_ref()).expect("decode"),
            log
        );
        assert_eq!(
            BeaconFetchKey::decode(artifact.encode().as_ref()).expect("decode"),
            artifact
        );
        assert_ne!(log, artifact);
        assert_ne!(log.encode(), artifact.encode());
        assert!(
            BeaconFetchKey::decode([2u8, 0, 0, 0, 0, 0, 0, 0, 0].as_slice()).is_err(),
            "an unknown subject tag was accepted"
        );
    }

    /// The ceremony's handle speaks only for dealer logs. Its `retain`/`clear`
    /// must not drop an artifact pull, which belongs to a caller this side does
    /// not know about: cancelling it would strand that caller on a key nobody
    /// re-requests.
    #[test]
    fn the_log_fetcher_never_cancels_the_artifact_arm() {
        let runner = commonware_runtime::deterministic::Runner::default();
        runner.start(|_| async move {
            #[derive(Clone, Default)]
            struct Spy(std::sync::Arc<std::sync::Mutex<Vec<BeaconFetchKey>>>);

            impl Resolver for Spy {
                type Key = BeaconFetchKey;
                type PublicKey = PeerPubkey;
                async fn fetch(&mut self, key: Self::Key) {
                    self.0.lock().unwrap().push(key);
                }
                async fn fetch_all(&mut self, keys: Vec<Self::Key>) {
                    self.0.lock().unwrap().extend(keys);
                }
                async fn fetch_targeted(&mut self, _: Self::Key, _: NonEmptyVec<Self::PublicKey>) {}
                async fn fetch_all_targeted(
                    &mut self,
                    _: Vec<(Self::Key, NonEmptyVec<Self::PublicKey>)>,
                ) {
                }
                async fn cancel(&mut self, _: Self::Key) {}
                async fn clear(&mut self) {
                    self.0.lock().unwrap().clear();
                }
                async fn retain(
                    &mut self,
                    predicate: impl Fn(&Self::Key) -> bool + Send + 'static,
                ) {
                    self.0.lock().unwrap().retain(|k| predicate(k));
                }
            }

            let mut rng = StdRng::seed_from_u64(12);
            let dealer = PrivateKey::random(&mut rng).public_key();
            let spy = Spy::default();
            let mut inner = spy.clone();
            inner.fetch(BeaconFetchKey::Artifact { epoch: 9 }).await;
            let mut logs = LogFetcher::new(spy.clone());
            logs.fetch(DkgLogKey {
                epoch: 9,
                dealer: dealer.clone(),
            })
            .await;
            assert_eq!(spy.0.lock().unwrap().len(), 2);

            // The ceremony drops every log it no longer wants; the artifact pull
            // survives both the predicate and the wholesale clear.
            logs.retain(|_| false).await;
            assert_eq!(
                *spy.0.lock().unwrap(),
                vec![BeaconFetchKey::Artifact { epoch: 9 }],
                "retain dropped the artifact fetch this handle does not own"
            );
            logs.fetch(DkgLogKey { epoch: 9, dealer }).await;
            logs.clear().await;
            assert_eq!(
                *spy.0.lock().unwrap(),
                vec![BeaconFetchKey::Artifact { epoch: 9 }],
                "clear dropped the artifact fetch this handle does not own"
            );
        });
    }

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
