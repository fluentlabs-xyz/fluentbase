//! DKG-log recovery over `commonware_resolver::p2p`.
//!
//! A committee member that restarts mid-window resumes its ceremony player-only
//! but may still lack peer dealer logs it never received. Those logs are re-fetched
//! here through the architecture's own recovery engine, keyed by
//! `{epoch, dealer, hash}`: the exact body the agreement pinned, never "some log of
//! that dealer". The engine owns retry, multi-peer fallback, `fetch_targeted`,
//! rate-limiting and blocked-peer eviction, and serves one dealer log per key.
//!
//! The beacon plane's `EpochTransition` tracks
//! `committee[E-1] ∪ committee[E] ∪ committee[E+1]` as primary on the same oracle
//! handle the resolver's provider reads, so the log holders are reachable during
//! the epoch whose committee is dealing.
//!
//! [`LogHandler`] implements both `Producer` and `Consumer`, forwarding each to the
//! single-threaded `DkgActor` run loop over an mpsc channel and a oneshot reply, so
//! the actor stays the sole owner of ceremony state.

use alloy_primitives::B256;
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

/// Resolver key for one signed dealer log in one ceremony: `{epoch, dealer, hash}`.
///
/// The hash is part of the identity, not a hint: a dealer can sign two valid logs,
/// and a fetch answered with "a log of that dealer" could deliver the one the
/// network did not pin. The server answers a key with the body under that exact
/// `(dealer, hash)` or with nothing; the requester takes the hash from the pinned
/// set it is finalizing over. The codec is a fixed 72 bytes, so keys round-trip
/// byte-identically network-wide.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DkgLogKey {
    pub epoch: u64,
    pub dealer: PeerPubkey,
    pub hash: B256,
}

impl Debug for DkgLogKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "DkgLogKey{{epoch={}, dealer={}, hash={}}}",
            self.epoch, self.dealer, self.hash
        )
    }
}

impl Display for DkgLogKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "dkg-log[e{}/{}/{}]", self.epoch, self.dealer, self.hash)
    }
}

impl Write for DkgLogKey {
    fn write(&self, buf: &mut impl BufMut) {
        self.epoch.write(buf);
        self.dealer.write(buf);
        buf.put_slice(self.hash.as_slice());
    }
}

impl EncodeSize for DkgLogKey {
    fn encode_size(&self) -> usize {
        size_of::<u64>() + self.dealer.encode_size() + B256::len_bytes()
    }
}

impl Read for DkgLogKey {
    type Cfg = ();

    fn read_cfg(buf: &mut impl Buf, _: &()) -> Result<Self, CodecError> {
        let epoch = u64::read(buf)?;
        let dealer = PeerPubkey::read(buf)?;
        let hash = B256::from(<[u8; 32]>::read(buf)?);
        Ok(Self {
            epoch,
            dealer,
            hash,
        })
    }
}

impl Span for DkgLogKey {}

/// Everything this node pulls on `BEACON_RESOLVER_CHANNEL`.
///
/// Two subjects on one engine: the epoch-key artifact needs a pull seam at the
/// beacon resolver's quota, which already tracks the right peer set and is already
/// wired, so adding a top-level channel would buy a second engine for traffic that
/// belongs on this one.
///
/// The tag byte is the marshal request codec's shape.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BeaconFetchKey {
    /// One dealer's public log in one ceremony. Boxed: the 72-byte log key beside
    /// the 8-byte artifact key would otherwise size every key to the larger arm.
    Log(Box<DkgLogKey>),
    /// The quorum-signed agreement artifact for one target epoch — the only
    /// source of `PK_epoch` for a node that never ran the ceremony.
    Artifact { epoch: u64 },
}

impl BeaconFetchKey {
    const TAG_LOG: u8 = 0;
    const TAG_ARTIFACT: u8 = 1;
    /// Retired tag, reserved rather than reusable, so an old peer's seed request does
    /// not decode as a future subject.
    ///
    /// It carried a by-round σ request. Nothing asks any more: σ rides the certificate
    /// and is filed at the certificate's own round on every door, so a node that pulls
    /// a boundary finalization obtains exactly the σ this request existed to fetch.
    const TAG_SEED_RETIRED: u8 = 2;
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
            Self::TAG_LOG => Ok(Self::Log(Box::new(DkgLogKey::read(buf)?))),
            Self::TAG_ARTIFACT => Ok(Self::Artifact {
                epoch: u64::read(buf)?,
            }),
            // Retired, and refused rather than ignored: silently accepting a tag
            // whose subject no longer exists would leave the requester's fetch
            // hanging on a responder that can never answer.
            Self::TAG_SEED_RETIRED => Err(CodecError::Invalid(
                "BeaconFetchKey",
                "seed requests are retired; σ rides the certificate",
            )),
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
        self.0.fetch(BeaconFetchKey::Log(Box::new(key))).await;
    }

    async fn fetch_all(&mut self, keys: Vec<Self::Key>) {
        self.0
            .fetch_all(
                keys.into_iter()
                    .map(|key| BeaconFetchKey::Log(Box::new(key)))
                    .collect(),
            )
            .await;
    }

    async fn fetch_targeted(&mut self, key: Self::Key, targets: NonEmptyVec<Self::PublicKey>) {
        self.0
            .fetch_targeted(BeaconFetchKey::Log(Box::new(key)), targets)
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
                    .map(|(key, targets)| (BeaconFetchKey::Log(Box::new(key)), targets))
                    .collect(),
            )
            .await;
    }

    async fn cancel(&mut self, key: Self::Key) {
        self.0.cancel(BeaconFetchKey::Log(Box::new(key))).await;
    }

    async fn clear(&mut self) {
        // Deliberately not `inner.clear()`: this handle speaks for the ceremony,
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
/// `deliver` returns `false` only for proven misbehaviour and `true` for everything
/// an honest peer can legitimately send, which keeps commonware's un-removable
/// `excluded` set out of this engine.
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
            BeaconFetchKey::Log(key) => self.logs.deliver(*key, value).await,
            BeaconFetchKey::Artifact { epoch } => self.artifacts.deliver(epoch, value.as_ref()),
        }
    }

    // `failed` fires only on Cancel/Retain/Clear and never on an error or timeout, so
    // a seam that relied on it would retry silently forever. The artifact side learns
    // from a delivered `NotYet` or its own bounded window; the log side re-issues its
    // missing targets each tick.
    async fn failed(&mut self, key: Self::Key, failure: Self::Failure) {
        if let BeaconFetchKey::Log(key) = key {
            self.logs.failed(*key, failure).await;
        }
    }
}

impl Producer for BeaconFetchHandler {
    type Key = BeaconFetchKey;

    async fn produce(&mut self, key: Self::Key) -> oneshot::Receiver<Bytes> {
        match key {
            BeaconFetchKey::Log(key) => self.logs.produce(*key).await,
            BeaconFetchKey::Artifact { epoch } => {
                // Always an answer, never a dropped responder: a producer that
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
    /// request). The actor replies with the encoded log bytes, or drops the
    /// responder (→ the resolver sends an empty "no data" response → the requester
    /// retries another peer).
    Produce {
        key: DkgLogKey,
        response: oneshot::Sender<Bytes>,
    },
    /// Re-`check`-ingest a fetched log for `key`. The actor replies `true` iff the
    /// log is valid (recorded → fetch complete) or `false` for a genuine forgery
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
        // No-op: the resolver retries on its own and the actor re-issues the
        // missing targets each tick, so a transiently-unavailable log is
        // re-fetched to completion rather than sat out.
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

    /// Both subjects round-trip, an unknown tag is refused, and no `Log` key decodes as
    /// an `Artifact` key or the reverse.
    #[test]
    fn the_shared_key_space_separates_its_two_subjects() {
        let mut rng = StdRng::seed_from_u64(11);
        let dealer = PrivateKey::random(&mut rng).public_key();
        let log = BeaconFetchKey::Log(Box::new(DkgLogKey {
            epoch: 7,
            dealer: dealer.clone(),
            hash: B256::repeat_byte(0x7a),
        }));
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
            BeaconFetchKey::decode([9u8, 0, 0, 0, 0, 0, 0, 0, 0].as_slice()).is_err(),
            "an unknown subject tag was accepted"
        );
    }

    /// Tag 2 is retired, not free: a peer on an older binary can still send it, and the
    /// refusal tells it so instead of leaving its fetch waiting. The next subject must
    /// take tag 3.
    #[test]
    fn the_retired_seed_tag_is_refused_rather_than_reused() {
        // A full pre-retirement `Seed{from,to}` frame: tag 2 plus four u64s.
        let mut frame = vec![2u8];
        frame.extend(std::iter::repeat_n(0u8, 32));
        let err = BeaconFetchKey::decode(frame.as_slice()).expect_err("retired tag");
        assert!(
            matches!(err, CodecError::Invalid(_, m) if m.contains("retired")),
            "expected the retired-tag refusal, got {err:?}"
        );
    }

    /// The ceremony's handle speaks only for dealer logs, so its `retain`/`clear` must
    /// not drop an artifact pull it does not own.
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
                hash: B256::ZERO,
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
            logs.fetch(DkgLogKey {
                epoch: 9,
                dealer,
                hash: B256::ZERO,
            })
            .await;
            logs.clear().await;
            assert_eq!(
                *spy.0.lock().unwrap(),
                vec![BeaconFetchKey::Artifact { epoch: 9 }],
                "clear dropped the artifact fetch this handle does not own"
            );
        });
    }

    /// The wire layout of the log key, byte for byte: `u64_be(epoch) ‖ dealer(32) ‖
    /// hash(32)`, 72 bytes, no length prefixes. Two keys that differ only in the hash
    /// are different keys, which makes "fetch the pinned body of this dealer"
    /// expressible on the wire.
    #[test]
    fn dkg_log_key_round_trips_and_orders_by_epoch_then_dealer() {
        let mut rng = StdRng::seed_from_u64(3);
        let a = PrivateKey::random(&mut rng).public_key();
        let b = PrivateKey::random(&mut rng).public_key();
        let hash = B256::repeat_byte(0xC3);
        let key = DkgLogKey {
            epoch: 7,
            dealer: a.clone(),
            hash,
        };
        let bytes = key.encode();
        assert_eq!(bytes.len(), 72, "fixed layout: 8 + 32 + 32");
        assert_eq!(key.encode_size(), 72);
        assert_eq!(&bytes[..8], &7u64.to_be_bytes(), "epoch leads, big-endian");
        assert_eq!(&bytes[8..40], a.encode().as_ref(), "the dealer key follows");
        assert_eq!(
            &bytes[40..],
            hash.as_slice(),
            "the content hash closes the key"
        );
        let decoded = DkgLogKey::decode(bytes.as_ref()).expect("decode");
        assert_eq!(decoded, key);
        let envelope = BeaconFetchKey::Log(Box::new(key.clone()));
        let wire = envelope.encode();
        assert_eq!(wire.len(), 73, "tag + 72");
        assert_eq!(envelope.encode_size(), 73);
        assert_eq!(wire[0], BeaconFetchKey::TAG_LOG, "the subject tag leads");
        assert_eq!(&wire[1..], bytes.as_ref(), "the key follows verbatim");
        assert_eq!(
            BeaconFetchKey::decode(wire.as_ref()).expect("decode"),
            envelope
        );
        assert_eq!(decoded.epoch, 7);
        assert_eq!(decoded.dealer, a);
        assert_eq!(decoded.hash, hash);
        assert!(
            DkgLogKey::decode(&bytes[..71]).is_err(),
            "a key short of its hash does not decode"
        );
        let lo = DkgLogKey {
            epoch: 6,
            dealer: b.clone(),
            hash,
        };
        let hi = DkgLogKey {
            epoch: 7,
            dealer: b.clone(),
            hash,
        };
        assert!(lo < hi, "lower epoch orders first regardless of dealer");
        let other_body = DkgLogKey {
            epoch: 7,
            dealer: b,
            hash: B256::repeat_byte(0xC4),
        };
        assert_ne!(
            hi, other_body,
            "same epoch + dealer, other hash: another key"
        );
        assert_ne!(hi.encode(), other_body.encode());
        assert!(hi < other_body, "the hash is the last sort key");
    }
}
