//! Plane-native [`CertUpstream`] over `commonware_resolver::p2p`.
//!
//! A `--dpos` validator with no WS upstream still needs frontier discovery: the
//! frozen-tip ladder probe consumes [`CertUpstream::get_latest`] (and
//! `get_finalization`) to learn there is a `(finalization, OrderBlock)` above its
//! own tip, and commonware's marshal `Request` enum has no over-the-wire `Latest`
//! variant. This module adds that transport as a Fluent-side resolver channel
//! (`FRONTIER_CHANNEL`) built on the same `commonware_resolver::p2p::Engine` the
//! beacon DKG-log resolver rides, serving each peer this node's local marshal
//! tip/archive.
//!
//! [`PlaneUpstreamHandle`] implements [`CertUpstream`] verbatim, so it drops into
//! the `U: CertUpstream` generic with no signature churn.
//!
//! [`FrontierHandler::deliver`] is the single point of trust. An answer to `Latest`
//! or `Finalized{h}` passes five checks before anything downstream sees it: decode
//! under a bounded cap, the requested height and the payload/digest bind,
//! `epoch_of(height) == round.epoch`, `committee[round.epoch]`, and the 2f+1 BLS
//! multisig under a verify-only scheme built from that record.
//!
//! The scheme is built here with `oracle = None`, not taken from
//! [`Committee::scheme`]: the module's scheme carries the epoch's seed oracle, so a
//! node whose local `PK_epoch` came from a different mint would fail an honest
//! certificate on the seed half and permanently ban an honest peer. Without an
//! oracle the certificate is judged on the 2f+1 quorum and the epoch binding alone,
//! which is what this path needs; nothing here consumes sigma.
//!
//! `deliver` returning `false` tells commonware this peer lied, which excludes it
//! from the channel for the life of the resolver engine. So `false` is returned
//! only for a lie: undecodable bytes, a foreign height, a payload/digest mismatch,
//! an epoch mismatch, or a failed multisig under a readable committee. An answer
//! this node cannot authenticate — an epoch outside its read window, an unreadable
//! committee, no frozen geometry — returns `true` (the peer is not punished) and is
//! dropped, with `dpos_frontier_dropped_total{reason}` counting it and the waiter
//! answered `None`. Dropping removes the key's waiter entry, so `fetch_one` resolves
//! at once; the executor's frozen-tip probe asks again on its next tick.

use crate::{
    cert_follow::{CertUpstream, UpstreamFinalized},
    cert_inlet::MarshalSink,
    committee::{Committee, CommitteeError},
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
use commonware_parallel::Sequential;
use commonware_resolver::{p2p::Producer, Consumer, Resolver as _};
use commonware_runtime::Clock;
use commonware_utils::{channel::oneshot as cw_oneshot, Span};
use fluentbase_bls::Scheme as BlsScheme;
use rand_core::CryptoRngCore;
use std::{
    collections::HashMap,
    fmt::{Debug, Display, Formatter},
    future::Future,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
use tokio::sync::oneshot;
use tracing::warn;

/// `dpos_frontier_dropped_total{reason}` — a frontier answer that passed every
/// check this node could make (decode, requested height, payload/digest bind,
/// height/epoch bind) but whose committee it cannot read. Dropped, not a fault: the
/// peer keeps the channel, because this is a statement about this node's lag.
const FRONTIER_DROPPED: &str = "dpos_frontier_dropped_total";

/// `dpos_frontier_rejected_total{reason}` — a frontier answer that carried a signal
/// of a lie (`deliver` returned `false`), so commonware excluded its sender from this
/// channel's fetches for the life of the resolver engine.
const FRONTIER_REJECTED: &str = "dpos_frontier_rejected_total";

// `reason` labels. One constant per arm so the dashboard and the tests name the
// same string.
const REASON_UNDECODABLE: &str = "undecodable";
const REASON_WRONG_HEIGHT: &str = "wrong_height";
const REASON_PAYLOAD_MISMATCH: &str = "payload_mismatch";
const REASON_EPOCH_MISMATCH: &str = "epoch_mismatch";
const REASON_BLS: &str = "bls";
const REASON_NO_GEOMETRY: &str = "no_geometry";
const REASON_OUT_OF_WINDOW: &str = "out_of_window";
const REASON_NOT_READABLE: &str = "not_readable";
const REASON_READ_FAILED: &str = "read_failed";

/// The finalization-certificate type the frontier wire carries (marshal-format,
/// identical to the WS [`crate::certified_block`] pair the upstream serves).
type Cert = Finalization<BlsScheme, Digest>;

/// The client resolver mailbox `PlaneUpstreamHandle` issues fetches on.
pub type FrontierResolverMailbox = commonware_resolver::p2p::Mailbox<FrontierKey, PublicKey>;

/// One-slot memo of the verify-only scheme `deliver` judges a certificate under,
/// keyed by the epoch it was built for — see [`FrontierHandler::verify_scheme`].
type VerifySchemeSlot = Arc<Mutex<Option<(u64, Arc<BlsScheme>)>>>;

/// Per-key correlation map: a delivered `(cert, block)` is fanned to every waiting
/// `get_latest`/`get_finalization` call for that key. Shared between the client
/// [`PlaneUpstreamHandle`] (registers waiters) and the serve-side [`FrontierHandler`]
/// (resolves them on `deliver`).
type Waiters = Arc<Mutex<HashMap<FrontierKey, Vec<oneshot::Sender<UpstreamFinalized>>>>>;

/// How long a single `get_latest`/`get_finalization` awaits a plane delivery before
/// returning `None`. Bounds the ladder probe so an isolated node is a no-op rather
/// than hanging. Measured on the runtime [`Clock`] (virtual time under the
/// deterministic runner), never on a tokio timer directly.
const FRONTIER_FETCH_TIMEOUT: Duration = Duration::from_secs(8);

/// Resolver request key: two subjects, fixed-layout `Span`.
///
/// `Latest` answers are time-varying and never cached; `Finalized { height }` is the
/// by-height arm the hybrid upstream path pulls. The codec is fixed-layout
/// (`Cfg = ()`) so it round-trips byte-identically network-wide.
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

/// Everything [`FrontierHandler`] needs of a marshal: the two local archive reads
/// the serve side answers with, plus the two driving calls a verified delivery makes
/// (inherited from [`MarshalSink`]).
///
/// A trait rather than the concrete [`MarshalMailbox`] because the mailbox
/// constructor is `pub(crate)` upstream, so a unit test cannot build one.
pub trait FrontierMarshal: MarshalSink + Clone + Sync + 'static {
    /// The `(finalization, block)` pair of this node's highest local finalized
    /// tip, or `None` when the archive holds none.
    fn latest_pair(&self) -> impl Future<Output = Option<(Cert, OrderBlock)>> + Send;

    /// The `(finalization, block)` pair at an explicit height, or `None` on an
    /// archive miss.
    fn pair_at(&self, height: Height) -> impl Future<Output = Option<(Cert, OrderBlock)>> + Send;
}

impl FrontierMarshal for MarshalMailbox {
    async fn latest_pair(&self) -> Option<(Cert, OrderBlock)> {
        let (h, _digest) = self.get_info(Identifier::Latest).await?;
        let fin = self.get_finalization(h).await?;
        // Re-read the block at `h`, not `Identifier::Latest`: a block finalizing
        // between the two awaits would pair `fin@h` with `block@h+1`, which the
        // client's payload/digest bind would reject as a lie.
        let block = self.get_block(h).await?;
        Some((fin, block))
    }

    /// Both reads at the same explicit height, never `Identifier::Latest`, for the
    /// same pairing reason as [`Self::latest_pair`].
    async fn pair_at(&self, height: Height) -> Option<(Cert, OrderBlock)> {
        let fin = self.get_finalization(height).await?;
        let block = self.get_block(height).await?;
        Some((fin, block))
    }
}

/// Serve this node's local marshal tip/archive for `key` — no network, no execution.
/// Encodes `(finalization, block)` as the WS upstream and marshal resolver do, so
/// `decode_frontier` round-trips it.
async fn serve<M: FrontierMarshal>(marshal: &M, key: FrontierKey) -> Option<Bytes> {
    let pair = match key {
        FrontierKey::Latest => marshal.latest_pair().await?,
        FrontierKey::Finalized { height } => marshal.pair_at(Height::new(height)).await?,
    };
    Some(pair.encode())
}

/// Decode a delivered frontier value. The signer bitmap is decoded with a bounded
/// cap, because the value comes from an untrusted peer and the unbounded decoder
/// allocates from a tiny length prefix. Exact participant validation still happens at
/// cert-verify time.
fn decode_frontier(value: &[u8]) -> Option<UpstreamFinalized> {
    let cap = fluentbase_p2p::constants::MAX_COMMITTEE_SIZE as usize;
    let (finalization, block) = <(Cert, OrderBlock)>::decode_cfg(value, &(cap, ())).ok()?;
    Some(UpstreamFinalized {
        finalization,
        block,
    })
}

/// Bridges the resolver engine's `Producer` (serve local tip) and `Consumer` (the
/// single point of trust for the frontier) to the shared [`Waiters`] map, the
/// committee module and the late-bound marshal.
#[derive(Clone)]
pub struct FrontierHandler<E, M = MarshalMailbox> {
    waiters: Waiters,
    /// The node's own marshal, late-bound (the marshal is created by the later layer
    /// launch). Until filled, `produce` serves "no data" and a verified delivery
    /// reaches nothing, which is safe: the only thing lost is a tip advance the next
    /// probe asks for again.
    marshal_slot: Arc<OnceLock<M>>,
    /// The process's one committee module: the geometry the height/epoch bind is taken
    /// over and the record the window admits.
    committee: Arc<dyn Committee>,
    /// The chain namespace every certificate is signed under, the same value
    /// [`crate::committee::epoch_verifier`] closes over. Held because this file builds
    /// its own verify-only scheme and the record does not carry it.
    namespace: Arc<Vec<u8>>,
    /// One-slot memo of the verify-only scheme, keyed by epoch. Consecutive answers
    /// name the same epoch for a whole epoch's worth of heights, so one slot is the
    /// whole cache and cannot grow with the chain.
    verify_scheme: VerifySchemeSlot,
    /// The commonware context the certificate `verify()` draws its batch
    /// randomness from — the same `CryptoRngCore` seam the cert inlet holds.
    ctx: E,
}

impl<E, M> FrontierHandler<E, M>
where
    E: CryptoRngCore + Clone + Send + Sync + 'static,
    M: FrontierMarshal,
{
    /// One rejected answer: counted, warned, and `deliver` answers `false`, which
    /// costs the sender this channel for the life of the resolver engine.
    fn reject(reason: &'static str, height: u64) -> bool {
        metrics::counter!(FRONTIER_REJECTED, "reason" => reason).increment(1);
        warn!(
            reason,
            height,
            "frontier answer REJECTED as a lie — commonware excludes this peer from \
             frontier fetches until the resolver engine restarts"
        );
        false
    }

    /// The verify-only scheme for `epoch`, built from the record `deliver` just read
    /// and memoized.
    ///
    /// `oracle = None` is the point: the frontier judges "2f+1 under `committee[epoch]`,
    /// bound to `epoch`" and nothing else, because it has no beacon with which to tell
    /// an honest sigma it cannot check from a forged one.
    fn verifier_for(
        &self,
        epoch: u64,
        record: &crate::committee::CommitteeRecord,
    ) -> Arc<BlsScheme> {
        let mut slot = self.verify_scheme.lock().unwrap();
        if let Some((cached_epoch, scheme)) = slot.as_ref() {
            if *cached_epoch == epoch {
                return scheme.clone();
            }
        }
        let scheme = Arc::new(fluentbase_bls::scheme::build_verifier(
            &self.namespace,
            record.bls.bimap.clone(),
            epoch,
            None,
        ));
        *slot = Some((epoch, scheme.clone()));
        scheme
    }
}

impl<E, M> Consumer for FrontierHandler<E, M>
where
    E: CryptoRngCore + Clone + Send + Sync + 'static,
    M: FrontierMarshal,
{
    type Key = FrontierKey;
    type Value = Bytes;
    type Failure = ();

    /// The five steps, in order; nothing downstream runs until all five pass. See the
    /// module docs for why the returned `bool` is a second decision.
    async fn deliver(&mut self, key: Self::Key, value: Self::Value) -> bool {
        let Some(uf) = decode_frontier(value.as_ref()) else {
            return Self::reject(REASON_UNDECODABLE, 0);
        };
        let round = uf.finalization.proposal.round;
        let height = uf.block.height;

        // The answer is the answer to the question. An honest marshal serves
        // `Finalized{h}` from exactly `h`, so a foreign height is a substitution, not a
        // race. The payload/digest bind is the same substitution one level down.
        if let FrontierKey::Finalized { height: asked } = key {
            if height != asked {
                return Self::reject(REASON_WRONG_HEIGHT, height);
            }
        }
        if crate::cold_start_jump::verify_jump_structural(&uf).is_err() {
            return Self::reject(REASON_PAYLOAD_MISMATCH, height);
        }

        // Height/epoch bind over the module's one geometry. Before the freeze no height
        // means anything, so there is nothing to judge — an unauthenticated answer, not
        // a lie.
        let epoch = round.epoch().get();
        let mut unauthenticated = None;
        match self.committee.epoch_of(height) {
            None => unauthenticated = Some(REASON_NO_GEOMETRY),
            Some(height_epoch) if height_epoch != epoch => {
                return Self::reject(REASON_EPOCH_MISMATCH, height);
            }
            Some(_) => {}
        }

        // The committee, from the module. A failure here is never about the peer — an
        // epoch outside the read window, an anchor below the commit height, a faulted
        // read — so it can never produce a `false`; it produces an answer this node
        // cannot authenticate, and the drop path below decides what to do with one.
        if unauthenticated.is_none() {
            unauthenticated = match self.committee.committee(epoch) {
                Err(CommitteeError::OutOfWindow { .. }) => Some(REASON_OUT_OF_WINDOW),
                Err(CommitteeError::NotReadable { .. }) => Some(REASON_NOT_READABLE),
                Err(CommitteeError::Read(_)) => Some(REASON_READ_FAILED),
                // Built here from the record, verify-only and without the epoch's seed
                // oracle: taking `Committee::scheme` instead would make an honest
                // certificate fail on this node's stale `PK_epoch` and ban the peer.
                Ok(record) => {
                    let scheme = self.verifier_for(epoch, &record);
                    if !uf.finalization.verify(&mut self.ctx, &scheme, &Sequential) {
                        // The committee is readable, so this is a statement about the
                        // certificate, not about this node's lag.
                        return Self::reject(REASON_BLS, height);
                    }
                    None
                }
            };
        }

        // An answer this node cannot authenticate is dropped: the certificate carries
        // a 2f+1 claim nobody here can check, so nothing downstream gets to see it.
        //
        // The peer is not punished (`true` below): an out-of-window epoch, an anchor
        // below the commit height or a faulted read are statements about this node, and
        // excluding an honest peer would cost the channel for this node's own lag.
        //
        // The waiter is answered now rather than left to time out: removing the entry
        // drops its sender, so `fetch_one` resolves `None` immediately. No `cancel` is
        // issued and none is needed — the `true` returned below already completes the
        // fetch commonware-side. The retry driver is the executor's frozen-tip probe.
        if let Some(reason) = unauthenticated {
            metrics::counter!(FRONTIER_DROPPED, "reason" => reason).increment(1);
            drop(self.waiters.lock().unwrap().remove(&key));
            return true;
        }
        // The admitted answer is not written into the marshal from here: on every path
        // that ends in the marshal, this value is handed to the marshal's own resolver
        // handler, which decodes, BLS-verifies under the same committee module and
        // stores it through `store_finalization` — the single writer. Reporting here as
        // well would make this file a second writer of the same finalization.
        let waiting = self.waiters.lock().unwrap().remove(&key);
        // Only a five-step-verified answer reaches the waiters; an unauthenticatable
        // one was dropped above, so nothing downstream consumes an unchecked frontier.
        for tx in waiting.into_iter().flatten() {
            let _ = tx.send(uf.clone());
        }
        true
    }

    async fn failed(&mut self, _: Self::Key, _: Self::Failure) {
        // No-op: the awaiting call times out on its own, and the marshal or jump
        // re-requests on the next repair sweep or re-jump edge.
    }
}

impl<E, M> Producer for FrontierHandler<E, M>
where
    E: CryptoRngCore + Clone + Send + Sync + 'static,
    M: FrontierMarshal,
{
    type Key = FrontierKey;

    async fn produce(&mut self, key: Self::Key) -> cw_oneshot::Receiver<Bytes> {
        let (response, receiver) = cw_oneshot::channel();
        // Read the local marshal inline. A missing slot (marshal not yet launched) or
        // an archive miss drops `response` unsent, so the resolver relays "no data" and
        // the requester retries a peer.
        if let Some(marshal) = self.marshal_slot.get() {
            if let Some(bytes) = serve(marshal, key).await {
                let _ = response.send(bytes);
            }
        }
        receiver
    }
}

/// Build the frontier bridge: the [`FrontierHandler`] (Producer + Consumer) and the
/// shared [`Waiters`] map the resulting [`PlaneUpstreamHandle`] registers waiters on.
/// `marshal_slot` is the same `OnceLock` the node fills after the layer launch;
/// `committee` is the process's one committee module; `chain_id` derives the
/// certificate namespace, so the scheme built here differs from the process's own in
/// exactly the absent seed oracle.
pub fn new_bridge<E, M>(
    marshal_slot: Arc<OnceLock<M>>,
    committee: Arc<dyn Committee>,
    ctx: E,
    chain_id: u64,
) -> (FrontierHandler<E, M>, Waiters)
where
    E: CryptoRngCore + Clone + Send + Sync + 'static,
    M: FrontierMarshal,
{
    let waiters: Waiters = Arc::new(Mutex::new(HashMap::new()));
    (
        FrontierHandler {
            waiters: waiters.clone(),
            marshal_slot,
            committee,
            namespace: Arc::new(fluentbase_bls::fluent_namespace(chain_id)),
            verify_scheme: Arc::new(Mutex::new(None)),
            ctx,
        },
        waiters,
    )
}

/// A plane-native [`CertUpstream`] over the frontier resolver. Holds the runtime clock
/// to bound a fetch, the client resolver mailbox to issue fetches, and the shared
/// [`Waiters`] map to await the resolver's delivery.
#[derive(Clone)]
pub struct PlaneUpstreamHandle<E: Clock> {
    context: E,
    mailbox: FrontierResolverMailbox,
    waiters: Waiters,
}

impl<E: Clock> PlaneUpstreamHandle<E> {
    pub fn new(context: E, mailbox: FrontierResolverMailbox, waiters: Waiters) -> Self {
        Self {
            context,
            mailbox,
            waiters,
        }
    }

    /// Register a waiter, issue the fetch, and await the delivery with a bounded
    /// timeout. Untargeted: the resolver picks and rotates peers itself, and with an
    /// empty tracked set the fetch is a no-op and this returns `None`.
    ///
    /// The ladder step's `fetch_targeted` is deliberately not issued here: a targeted
    /// fetch has no fallback, and re-deriving targets from the height inside this call
    /// retargets whichever by-height repair pull lands on `last(T+1)`, which starves
    /// the contiguous catch-up of a node that has just landed a jump. The step is
    /// addressed where it is issued.
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
        // The bound runs on the runtime `Clock`: a tokio timer would need a tokio
        // reactor, which the deterministic runner does not provide.
        let answer = tokio::select! {
            answer = rx => answer.ok(),
            () = self.context.sleep(FRONTIER_FETCH_TIMEOUT) => None,
        };
        match answer {
            Some(uf) => {
                tracing::debug!(%key, height = uf.block.height, "frontier fetch delivered");
                Some(uf)
            }
            None => {
                tracing::debug!(%key, "frontier fetch timed out (no tracked peer served it)");
                // Timed out: prune the closed waiter and, if no waiters remain, cancel the
                // in-flight fetch so the resolver stops probing peers for it.
                //
                // A `deliver` drop takes neither branch: it removed the whole entry, so
                // `get_mut` is `None`, `empty` stays false and no `cancel` is sent —
                // `deliver` returned `true`, which already closed that fetch.
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

impl<E: Clock> CertUpstream for PlaneUpstreamHandle<E> {
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
        // re-issues one. The resolver already does multi-peer fallback, so there is no
        // per-peer cursor to advance.
        let mut mailbox = self.mailbox.clone();
        async move {
            mailbox.cancel(FrontierKey::Latest).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        committee::{testing::SchemeCommittee, CommitteeRecord, Geometry, Member},
        scheme::epoch_committee_from_snapshot,
    };
    use alloy_primitives::{Address, B256};
    use commonware_codec::DecodeExt as _;
    use commonware_consensus::{
        simplex::types::{Finalize, Proposal},
        types::{Epoch, Round, View},
    };
    use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer as _};
    use commonware_math::algebra::Random as _;
    use commonware_runtime::{deterministic, Runner as _};
    use commonware_utils::{ordered::Set, TryFromIterator as _};
    use fluentbase_bls::{
        fluent_namespace,
        keys::ValidatorBlsKeypair,
        scheme::{build_signer, build_verifier},
        BlsPubkey, PeerPubkey,
    };
    use fluentbase_staking_reader::reader::{
        ConsensusKeys, ValidatorSetSnapshot, ValidatorWithKeys,
    };
    use rand_08::rngs::StdRng;
    use rand_core::SeedableRng as _;
    use std::sync::Mutex as StdMutex;

    const CHAIN_ID: u64 = 20_994;
    const COMMITTEE_N: usize = 4;
    /// Activation 0, 32-block epochs — so `epoch_of(h) = h / 32` and epoch 1 owns
    /// heights 32..=63.
    const EPOCH_LEN: u64 = 32;

    /// A real committee: peer keys, BLS keys, and the `(signers, verifier)` pair a
    /// genuine 2f+1 finalization needs.
    struct Fixture {
        peers: Vec<PeerPubkey>,
        bls_kps: Vec<ValidatorBlsKeypair>,
        bls_pks: Vec<BlsPubkey>,
        namespace: Vec<u8>,
    }

    fn fixture(seed: u64) -> Fixture {
        let mut rng = StdRng::seed_from_u64(seed);
        let peer_sks: Vec<_> = (0..COMMITTEE_N)
            .map(|_| Ed25519PrivateKey::random(&mut rng))
            .collect();
        let bls_kps: Vec<_> = (0..COMMITTEE_N)
            .map(|_| ValidatorBlsKeypair::generate(&mut rng))
            .collect();
        let bls_pks: Vec<BlsPubkey> = bls_kps
            .iter()
            .map(|kp| BlsPubkey::decode(kp.public_bytes().as_slice()).unwrap())
            .collect();
        Fixture {
            peers: peer_sks.iter().map(|p| p.public_key()).collect(),
            bls_kps,
            bls_pks,
            namespace: fluent_namespace(CHAIN_ID),
        }
    }

    impl Fixture {
        fn snapshot(&self, epoch: u64) -> ValidatorSetSnapshot {
            ValidatorSetSnapshot {
                block_hash: B256::ZERO,
                block_number: 0,
                epoch,
                validators: self
                    .peers
                    .iter()
                    .zip(self.bls_pks.iter())
                    .map(|(peer, bls)| ValidatorWithKeys {
                        address: Address::ZERO,
                        keys: ConsensusKeys {
                            peer_pubkey: peer.clone(),
                            bls_pubkey: *bls,
                            activation_epoch: 0,
                        },
                        tombstoned: false,
                    })
                    .collect(),
                weights: Some(vec![1u128; COMMITTEE_N]),
            }
        }

        fn record(&self, epoch: u64) -> CommitteeRecord {
            let snap = self.snapshot(epoch);
            CommitteeRecord {
                epoch,
                members: self
                    .peers
                    .iter()
                    .zip(self.bls_pks.iter())
                    .map(|(peer, bls)| Member {
                        address: Address::ZERO,
                        peer: peer.clone(),
                        bls: *bls,
                    })
                    .collect(),
                weights: vec![1u128; COMMITTEE_N],
                changed: false,
                snapshot: (0, B256::ZERO),
                participants: Set::try_from_iter(self.peers.iter().cloned()).unwrap(),
                bls: epoch_committee_from_snapshot(&snap).unwrap(),
            }
        }

        fn verifier(&self, epoch: u64) -> BlsScheme {
            let record = self.record(epoch);
            build_verifier(&self.namespace, record.bls.bimap, epoch, None)
        }

        /// A real 2f+1 finalization over `block`, under `epoch`'s committee.
        fn certify(&self, epoch: u64, block: &OrderBlock) -> UpstreamFinalized {
            let record = self.record(epoch);
            let round = Round::new(Epoch::new(epoch), View::new(block.height));
            let prop = Proposal::new(
                round,
                View::new(block.height.saturating_sub(1)),
                block.digest(),
            );
            let finalizes: Vec<_> = self
                .bls_kps
                .iter()
                .map(|kp| {
                    let signer =
                        build_signer(&self.namespace, record.bls.bimap.clone(), kp, epoch, None)
                            .expect("member");
                    Finalize::sign(&signer, prop.clone()).expect("sign")
                })
                .collect();
            let finalization = Finalization::from_finalizes(
                &self.verifier(epoch),
                finalizes.iter(),
                &commonware_parallel::Sequential,
            )
            .expect("quorum");
            UpstreamFinalized {
                finalization,
                block: block.clone(),
            }
        }
    }

    fn order_at(height: u64) -> OrderBlock {
        OrderBlock {
            parent: Digest(B256::repeat_byte(7)),
            height,
            proposal_view: 0,
            timestamp: 1_700_000_000 + height,
            gas_limit: 30_000_000,
            extra_data: alloy_primitives::Bytes::new(),
            result: B256::ZERO,
            txs: Vec::new(),
            equivocation: None,
        }
    }

    /// Records the marshal driving calls in order — the only witness that a verified
    /// delivery drives `verify` then `report` and that a refused one drives nothing.
    #[derive(Clone, Default)]
    struct FakeMarshal {
        calls: Arc<StdMutex<Vec<&'static str>>>,
    }

    impl FakeMarshal {
        fn calls(&self) -> Vec<&'static str> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl MarshalSink for FakeMarshal {
        async fn verify_block(
            &mut self,
            _round: commonware_consensus::types::Round,
            _block: OrderBlock,
        ) {
            self.calls.lock().unwrap().push("verified");
        }
        async fn report_finalization(&mut self, _f: Cert) {
            self.calls.lock().unwrap().push("report");
        }
    }

    impl FrontierMarshal for FakeMarshal {
        async fn latest_pair(&self) -> Option<(Cert, OrderBlock)> {
            None
        }
        async fn pair_at(&self, _height: Height) -> Option<(Cert, OrderBlock)> {
            None
        }
    }

    /// The five-step gate under test, wired the way production wires it: a real
    /// committee module double, a recording marshal, and the deterministic runner's
    /// context as the verify RNG.
    fn gate(
        ctx: deterministic::Context,
        committee: Arc<dyn Committee>,
    ) -> (
        FrontierHandler<deterministic::Context, FakeMarshal>,
        FakeMarshal,
        Waiters,
    ) {
        let marshal = FakeMarshal::default();
        let slot = Arc::new(OnceLock::new());
        let _ = slot.set(marshal.clone());
        let (handler, waiters) = new_bridge(slot, committee, ctx, CHAIN_ID);
        (handler, marshal, waiters)
    }

    /// A module that answers `epoch` with the fixture's record and verifier, over a
    /// frozen 32-block geometry and the window `[lo, hi]`.
    fn module(f: &Fixture, window: (u64, u64)) -> Arc<dyn Committee> {
        let verifier_f = fixture(1);
        let record_f = fixture(1);
        let _ = f;
        SchemeCommittee::with_window(
            move |epoch| Some(verifier_f.verifier(epoch)),
            move |epoch| Some(record_f.record(epoch)),
            Geometry::new(0, EPOCH_LEN),
            window,
        )
    }

    /// The node's local beacon key for the epoch came from a different mint: it
    /// resolves and disagrees with the sigma an honest quorum recovered, so every
    /// assembled seed is invalid.
    #[derive(Debug)]
    struct StaleEpochKeyOracle;

    impl fluentbase_bls::oracle::SeedOracle for StaleEpochKeyOracle {
        fn sign_partial(&self, _round: Round) -> Option<fluentbase_bls::BlsSignature> {
            None
        }
        fn verify_partial(
            &self,
            _round: Round,
            _index: commonware_utils::Participant,
            _value: &fluentbase_bls::BlsSignature,
        ) -> bool {
            false
        }
        fn recover(
            &self,
            _partials: &[(commonware_utils::Participant, fluentbase_bls::BlsSignature)],
            _threshold: u32,
        ) -> Option<fluentbase_bls::BlsSignature> {
            None
        }
        fn verify_seed(
            &self,
            _round: Round,
            _seed: &fluentbase_bls::BlsSignature,
        ) -> fluentbase_bls::oracle::SeedCheck {
            fluentbase_bls::oracle::SeedCheck::Invalid
        }
    }

    /// The module as a beacon-active epoch holds it: the same record, but the scheme
    /// carries a seed oracle whose key is the wrong one.
    fn module_with_a_stale_epoch_key(window: (u64, u64)) -> Arc<dyn Committee> {
        let verifier_f = fixture(1);
        let record_f = fixture(1);
        SchemeCommittee::with_window(
            move |epoch| {
                let record = verifier_f.record(epoch);
                Some(build_verifier(
                    &verifier_f.namespace,
                    record.bls.bimap,
                    epoch,
                    Some(Arc::new(StaleEpochKeyOracle)
                        as Arc<dyn fluentbase_bls::oracle::SeedOracle>),
                ))
            },
            move |epoch| Some(record_f.record(epoch)),
            Geometry::new(0, EPOCH_LEN),
            window,
        )
    }

    /// A real BLS signature in the seed slot that belongs to no sharing this node
    /// knows — shape-valid so the certificate round-trips, unverifiable so a wrong-key
    /// oracle answers invalid on it.
    fn unvouchable_seed(round: Round) -> fluentbase_bls::BlsSignature {
        use commonware_cryptography::bls12381::primitives::group::{Private, Share};
        let mut rng = StdRng::seed_from_u64(0x5EED);
        let share = Share::new(
            commonware_utils::Participant::new(0),
            Private::random(&mut rng),
        );
        fluentbase_bls::beacon::sign_seed_partial(&share, b"some-other-mint", round).value
    }

    /// Register a waiter for `key` and hand back the receiving half, so a test can
    /// assert whether the answer was fanned out or dropped.
    fn waiter(waiters: &Waiters, key: FrontierKey) -> oneshot::Receiver<UpstreamFinalized> {
        let (tx, rx) = oneshot::channel();
        waiters.lock().unwrap().entry(key).or_default().push(tx);
        rx
    }

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

    /// The honest answer: height matches the key, its epoch matches the certificate's
    /// round epoch, and the multisig verifies under the committee this node can read.
    /// `deliver` answers `true` and the waiter is resolved, and the marshal is not
    /// driven from here — the admitted value is handed to the marshal's own resolver
    /// handler, the single writer.
    #[test]
    fn an_honest_by_height_answer_is_admitted_without_a_second_marshal_writer() {
        deterministic::Runner::default().start(|ctx| async move {
            let f = fixture(1);
            let (mut handler, marshal, waiters) = gate(ctx, module(&f, (0, 3)));
            let height = 40; // epoch 1 under a 32-block geometry
            let uf = f.certify(1, &order_at(height));
            let rx = waiter(&waiters, FrontierKey::Finalized { height });
            let ok = handler
                .deliver(
                    FrontierKey::Finalized { height },
                    (uf.finalization, uf.block).encode(),
                )
                .await;
            assert!(ok, "an honest answer must not cost the peer the channel");
            assert!(rx.await.is_ok(), "the awaiting call was not resolved");
            assert!(
                marshal.calls().is_empty(),
                "deliver drove the marshal — a second writer of the same finalization: {:?}",
                marshal.calls()
            );
        });
    }

    /// The answer to `Finalized{h}` carries a wholly real, self-consistent
    /// finalization of a different height. An honest marshal serves exactly `h`, so this
    /// is a substitution: `deliver` answers `false`, drives nothing, and leaves the
    /// waiter unresolved.
    #[test]
    fn a_foreign_height_under_a_by_height_key_is_a_lie() {
        deterministic::Runner::default().start(|ctx| async move {
            let f = fixture(1);
            let (mut handler, marshal, waiters) = gate(ctx, module(&f, (0, 3)));
            // Asked for 40, served a real, correctly-signed 39.
            let uf = f.certify(1, &order_at(39));
            let mut rx = waiter(&waiters, FrontierKey::Finalized { height: 40 });
            let ok = handler
                .deliver(
                    FrontierKey::Finalized { height: 40 },
                    (uf.finalization, uf.block).encode(),
                )
                .await;
            assert!(!ok, "a foreign height must be refused as a lie");
            assert!(marshal.calls().is_empty(), "the marshal was driven");
            assert!(
                rx.try_recv().is_err(),
                "the wrong-height pair was fanned out"
            );
        });
    }

    /// The height/epoch bind: a certificate whose round epoch is not the epoch the
    /// served height belongs to. The per-epoch engine only proposes inside its own
    /// height range, so this is malformed and a lie signal. An inflated `Latest` height
    /// lands here long before it leaves the read window.
    #[test]
    fn a_height_outside_the_certificates_epoch_is_a_lie() {
        deterministic::Runner::default().start(|ctx| async move {
            let f = fixture(1);
            let (mut handler, marshal, _waiters) = gate(ctx, module(&f, (0, 3)));
            // Height 40 is epoch 1; the certificate is signed for epoch 2.
            let uf = f.certify(2, &order_at(40));
            let ok = handler
                .deliver(FrontierKey::Latest, (uf.finalization, uf.block).encode())
                .await;
            assert!(!ok, "a cross-epoch pair must be refused as a lie");
            assert!(marshal.calls().is_empty(), "the marshal was driven");
        });
    }

    /// An epoch the read window does not admit, delivered under `Latest` — an honest
    /// peer far ahead of this node. Unverifiable is not a lie, so `true` and the peer
    /// keeps the channel, but the answer is dropped: it carries a 2f+1 claim nobody
    /// here can check, and no consumer needs it. The waiter is resolved with nothing.
    #[test]
    fn an_out_of_window_latest_is_dropped_without_punishing_the_peer() {
        deterministic::Runner::default().start(|ctx| async move {
            let f = fixture(1);
            // Window [0, 3]; the answer belongs to epoch 9 (heights 288..=319).
            let (mut handler, marshal, waiters) = gate(ctx, module(&f, (0, 3)));
            let uf = f.certify(9, &order_at(9 * EPOCH_LEN + 4));
            let rx = waiter(&waiters, FrontierKey::Latest);
            let ok = handler
                .deliver(FrontierKey::Latest, (uf.finalization, uf.block).encode())
                .await;
            assert!(
                ok,
                "an out-of-window answer must not cost the peer the channel"
            );
            assert!(marshal.calls().is_empty(), "the marshal was driven");
            assert!(
                rx.await.is_err(),
                "the answer reached the caller — an unauthenticatable certificate was admitted"
            );
        });
    }

    /// The same out-of-window answer under a by-height key: same verdict and same drop.
    /// The executor's frozen-tip probe names `Finalized{last(T+1)}` on every frozen
    /// tick, so withholding the answer does not remove the event that asks again.
    #[test]
    fn an_out_of_window_by_height_answer_is_dropped_without_punishing_the_peer() {
        deterministic::Runner::default().start(|ctx| async move {
            let f = fixture(1);
            let height = 9 * EPOCH_LEN + 4;
            // Window [0, 3]; the answer belongs to epoch 9 (heights 288..=319).
            let (mut handler, marshal, waiters) = gate(ctx, module(&f, (0, 3)));
            let uf = f.certify(9, &order_at(height));
            let rx = waiter(&waiters, FrontierKey::Finalized { height });
            let ok = handler
                .deliver(
                    FrontierKey::Finalized { height },
                    (uf.finalization, uf.block).encode(),
                )
                .await;
            assert!(
                ok,
                "an out-of-window answer must not cost the peer the channel"
            );
            assert!(marshal.calls().is_empty(), "the marshal was driven");
            assert!(
                rx.await.is_err(),
                "the answer reached the caller — an unauthenticatable certificate was admitted"
            );
        });
    }

    /// The epoch is inside the window but this node's anchor has not reached its commit
    /// height — `NotReadable`. Same verdict as out-of-window and for the same reason: a
    /// statement about this node, not the peer, and the answer is dropped because an
    /// unchecked 2f+1 claim is not admissible whatever the reason it is unchecked.
    #[test]
    fn an_unreadable_committee_is_dropped_without_punishing_the_peer() {
        deterministic::Runner::default().start(|ctx| async move {
            let f = fixture(1);
            let verifier_f = fixture(1);
            // In-window, but the record half answers `None` — NotReadable.
            let committee: Arc<dyn Committee> = SchemeCommittee::with_window(
                move |epoch| Some(verifier_f.verifier(epoch)),
                |_| None,
                Geometry::new(0, EPOCH_LEN),
                (0, 3),
            );
            let (mut handler, marshal, waiters) = gate(ctx, committee);
            let uf = f.certify(1, &order_at(40));
            let rx = waiter(&waiters, FrontierKey::Finalized { height: 40 });
            let ok = handler
                .deliver(
                    FrontierKey::Finalized { height: 40 },
                    (uf.finalization, uf.block).encode(),
                )
                .await;
            assert!(
                ok,
                "an unreadable committee must not cost the peer the channel"
            );
            assert!(marshal.calls().is_empty(), "the marshal was driven");
            assert!(
                rx.await.is_err(),
                "the answer reached the caller — an unauthenticatable certificate was admitted"
            );
        });
    }

    /// The committee is readable and the multisig does not verify under it — the one arm
    /// where a failing signature is a statement about the certificate. `false`, nothing
    /// driven.
    ///
    /// The forgery swaps the served body and re-points the certificate's payload at the
    /// new digest, so the structural bind still passes and the multisig over the original
    /// payload is the only thing left to catch it.
    #[test]
    fn a_multisig_that_fails_under_a_readable_committee_is_a_lie() {
        deterministic::Runner::default().start(|ctx| async move {
            let f = fixture(1);
            let (mut handler, marshal, _waiters) = gate(ctx, module(&f, (0, 3)));
            let honest = f.certify(1, &order_at(40));
            let mut forged_block = honest.block.clone();
            forged_block.timestamp += 1;
            assert_ne!(forged_block, honest.block, "the tamper changed nothing");
            let forged = Finalization::<BlsScheme, Digest> {
                proposal: Proposal {
                    round: honest.finalization.proposal.round,
                    parent: honest.finalization.proposal.parent,
                    payload: forged_block.digest(),
                },
                certificate: honest.finalization.certificate.clone(),
            };
            let ok = handler
                .deliver(FrontierKey::Latest, (forged, forged_block).encode())
                .await;
            assert!(
                !ok,
                "a failing multisig under a readable committee is a lie"
            );
            assert!(marshal.calls().is_empty(), "the marshal was driven");
        });
    }

    /// An honest 2f+1 certificate of the right height and epoch, carrying the sigma of a
    /// beacon-active epoch, on a node whose local key for that epoch came from a
    /// different mint. Judged under the module's oracle-carrying scheme it fails
    /// entirely, which on this path would permanently exclude an honest peer. `deliver`
    /// builds a verify-only scheme from the record instead, so the certificate is judged
    /// on the 2f+1 quorum and the epoch binding alone.
    ///
    /// Not vacuous: the test first verifies the same certificate under the module's
    /// scheme and asserts it fails there.
    #[test]
    fn an_honest_certificate_is_admitted_when_this_nodes_epoch_key_is_stale() {
        deterministic::Runner::default().start(|ctx| async move {
            let f = fixture(1);
            let height = 40; // epoch 1 under a 32-block geometry
            let honest = f.certify(1, &order_at(height));
            assert!(
                honest.finalization.certificate.seed.is_none(),
                "the fixture signs without an oracle — the seed slot must start empty"
            );
            // The certificate as a beacon-active epoch puts it on the wire.
            let round = honest.finalization.proposal.round;
            let beacon_active = Finalization::<BlsScheme, Digest> {
                proposal: honest.finalization.proposal.clone(),
                certificate: fluentbase_bls::combined_scheme::CombinedCertificate {
                    vote: honest.finalization.certificate.vote.clone(),
                    seed: Some(unvouchable_seed(round)),
                },
            };
            assert!(
                beacon_active.certificate.seed.is_some(),
                "the seed slot was not filled — the stale-key arm is not exercised"
            );

            let module = module_with_a_stale_epoch_key((0, 3));
            // Under the module's own scheme this honest certificate does not verify:
            // that verdict is what bans the peer.
            let mut probe_ctx = ctx.clone();
            let with_oracle = module
                .scheme(1)
                .expect("the module has a scheme for epoch 1");
            assert!(
                !beacon_active.verify(&mut probe_ctx, &with_oracle, &Sequential),
                "the module's scheme accepted the certificate — the stale epoch key is not \
                 modelled and this test proves nothing"
            );

            let (mut handler, marshal, waiters) = gate(ctx, module);
            let rx = waiter(&waiters, FrontierKey::Finalized { height });
            let ok = handler
                .deliver(
                    FrontierKey::Finalized { height },
                    (beacon_active, honest.block).encode(),
                )
                .await;
            assert!(
                ok,
                "an honest 2f+1 certificate was refused as a lie because THIS node's epoch \
                 key is stale — the peer loses the frontier channel for good"
            );
            assert!(marshal.calls().is_empty(), "the marshal was driven");
            assert!(rx.await.is_ok(), "the answer was withheld");
        });
    }

    /// Undecodable bytes: `false`, nothing driven, and the awaiting waiter kept so the
    /// resolver can try another peer.
    #[test]
    fn an_undecodable_delivery_is_a_lie_and_keeps_the_waiters() {
        deterministic::Runner::default().start(|ctx| async move {
            let f = fixture(1);
            let (mut handler, marshal, waiters) = gate(ctx, module(&f, (0, 3)));
            let mut rx = waiter(&waiters, FrontierKey::Latest);
            let ok = handler
                .deliver(FrontierKey::Latest, Bytes::from_static(b"garbage"))
                .await;
            assert!(!ok, "undecodable delivery must return false");
            assert!(marshal.calls().is_empty(), "the marshal was driven");
            assert!(
                waiters.lock().unwrap().contains_key(&FrontierKey::Latest),
                "an undecodable delivery must not drop the awaiting waiters"
            );
            assert!(rx.try_recv().is_err());
        });
    }

    /// Before the geometry is frozen no height means anything, so an answer cannot be
    /// judged: counted and passed on with `true`, never refused. Answering `false` would
    /// exclude every peer a node talks to between its start and its first readable,
    /// DPoS-scheduled block.
    #[test]
    fn an_answer_before_the_geometry_is_frozen_is_not_punished() {
        deterministic::Runner::default().start(|ctx| async move {
            let f = fixture(1);
            let verifier_f = fixture(1);
            let record_f = fixture(1);
            let committee: Arc<dyn Committee> = SchemeCommittee::with_geometry(
                move |epoch| Some(verifier_f.verifier(epoch)),
                move |epoch| Some(record_f.record(epoch)),
                None,
            );
            let (mut handler, marshal, _waiters) = gate(ctx, committee);
            let uf = f.certify(1, &order_at(40));
            let ok = handler
                .deliver(FrontierKey::Latest, (uf.finalization, uf.block).encode())
                .await;
            assert!(
                ok,
                "an unjudgeable answer must not cost the peer the channel"
            );
            assert!(marshal.calls().is_empty(), "the marshal was driven");
        });
    }
}
