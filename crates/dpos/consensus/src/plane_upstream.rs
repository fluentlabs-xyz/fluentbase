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
//! Trust model: [`FrontierHandler::deliver`] is the SINGLE point of trust for
//! everything this channel carries (§5.2). A frontier answer — to `Latest` or to
//! `Finalized{h}` — passes five checks before anything downstream sees it: decode
//! under a bounded cap, the requested height (and the pair's own
//! payload↔digest bind), `epoch_of(height) == round.epoch` over the committee
//! module's ONE geometry, `committee[round.epoch]` from that module, and the 2f+1
//! BLS multisig under a VERIFY-ONLY scheme built from that record. Only then does
//! the pair reach anything downstream.
//!
//! The multisig is checked under a scheme this file builds from the record it
//! just read (`build_verifier(namespace, record.bls.bimap, epoch, None)`), NOT
//! under [`Committee::scheme`]. The module's scheme carries the epoch's beacon
//! SEED ORACLE, and `CombinedScheme::verify_certificate` runs that oracle after
//! the multisig quorum: a node whose local `PK_epoch` came from a different mint
//! answers `SeedCheck::Invalid` on an HONEST σ and the whole `verify` fails
//! (`bls/src/combined_scheme.rs:428-444`). On this path that verdict would be a
//! permanent ban of an honest peer, and the frontier has no beacon to resolve the
//! key with first the way the cert inlet does (`cert_inlet.rs:620`). Without an
//! oracle the same certificate is judged on exactly what §5.2 asks for — 2f+1
//! under `committee[round.epoch]`, plus the epoch binding — and nothing on this
//! path consumes σ.
//!
//! The RETURN VALUE is a second decision, and the split is the whole point.
//! commonware reads `false` as "this peer lied": it `block!`s the peer and drops
//! it into the fetcher's `excluded` set, which is never cleared for the life of
//! the resolver engine (`resolver/p2p/engine.rs:436-437`, `fetcher.rs:516`,
//! `:242`; `reconcile` does not touch it, `:500-505`). So `false` is returned ONLY
//! on a signal of a LIE — undecodable bytes, a foreign height under
//! `Finalized{h}`, a cert whose payload is not the served body's digest, a height
//! whose epoch is not the certificate's round epoch, or a multisig that fails
//! under a committee this node CAN read. Everything else — an epoch outside the
//! read window, a committee this node cannot read yet, no frozen geometry yet —
//! is "I cannot check this", and `deliver` returns `true` there: the fetch closes
//! and the peer is not punished. The ANSWER is then COUNTED and PASSED ON — for
//! BOTH keys — and that is where §5.2 as written does not survive contact with
//! this codebase. §5.2 says to throw such an answer away with a metric; neither
//! arm can afford it today, for two different reasons:
//!
//! * `Finalized{h}` — its retry driver is `UpstreamResolver::spawn_finalized`, a
//!   ONE-SHOT spawned pull re-armed only by the marshal's `try_repair_gaps`, which
//!   runs per STORED finalization. Withholding the answer removes the very event
//!   that would ask for it again. Measured: with the drop in place the Д3
//!   precondition's node 3 lands at 180 and never fetches another height (journal
//!   §0(6)(б), Д-72).
//! * `Latest` — nothing is ORPHANED by dropping it (`fetch_one` clears its own
//!   waiter and cancels the fetch on the timeout, `:534-553`, and the next probe
//!   tick asks again). Since pass Б1 its height is no longer a TRIGGER input at
//!   all: the jump trigger reads the marshal tip alone and the jump's target is a
//!   pair out of the local archive (`executor::maybe_re_jump`). What a `Latest`
//!   answer still buys is one `hint_finalization(frontier)` — a by-height fetch
//!   the marshal then verifies and stores itself — and the probe's own "was I
//!   served" bit. So the honest reason it is still passed on is smaller than it
//!   was and it is the SAME as the by-height arm's: the consumer below has its own
//!   gate, and nothing above believes the height. Dropping it out of the read
//!   window is §5.2's letter and pass Б2's work, together with the cold-start
//!   jump — the last consumer of an unverified `Latest` as a TARGET.
//!
//! Passing an answer on trusts nothing extra: a by-height pair still meets the
//! marshal's own `verify_delivered` (BLS under the same committee module), and a
//! tip still meets `cold_start_jump`'s POST-sync `verify_jump_authenticated` at
//! the landing's own state. What the four checks above add is everything that does
//! NOT need a committee — which is what closes R-009 (a foreign height), R-004's
//! inflated one, and a swapped body under a real certificate.
//!
//! A peer with no data never reaches `deliver` at all (it answers
//! `Payload::Error` ⇒ `add_retry`, no ban).

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

/// `dpos_frontier_unauthenticated_total{reason}` — a frontier answer that passed
/// every check this node could MAKE (decode, the requested height, the
/// payload↔digest bind, the height↔epoch bind) but whose committee it cannot
/// read, so the 2f+1 multisig went unchecked HERE. Not a fault and not a drop:
/// it is the ordinary shape of a node whose anchor has not reached the epoch the
/// answer belongs to, and the answer is passed on to the gate that CAN check it
/// (the marshal's own `verify_delivered` for a by-height pair, the jump's
/// post-sync `verify_jump_authenticated` for a tip). §5.2 asks for a DROP here
/// and the module docs record, with the measurement, why pass А could not make
/// one. A rate that does not fall as a node catches up is a node that never gets
/// inside its own read window.
const FRONTIER_UNAUTHENTICATED: &str = "dpos_frontier_unauthenticated_total";

/// `dpos_frontier_rejected_total{reason}` — a frontier answer that carried a
/// SIGNAL OF A LIE (`deliver` ⇒ `false`), so commonware excluded its sender from
/// this channel's fetches for the life of the resolver engine. Every increment is
/// a peer lost on purpose.
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
/// returning `None`. Bounds the jump's single-shot `get_latest` so an isolated /
/// not-yet-tracked node returns `None` (→ `Lagging` → boot on the treadmill) rather
/// than hanging; the resolver's own retry/multi-peer fallback delivers well inside
/// this window when a tracked peer holds the data. Measured on the runtime
/// [`Clock`] (the tokio timer in production, virtual time under the
/// deterministic runner), never on a tokio timer directly.
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

/// Everything [`FrontierHandler`] needs of a marshal, and nothing else: the two
/// LOCAL archive reads the serve side answers with, and the two driving calls a
/// VERIFIED delivery makes (inherited from [`MarshalSink`], the seam the cert
/// inlet already owns).
///
/// A trait rather than the concrete [`MarshalMailbox`] for one reason: the
/// mailbox's constructor is `pub(crate)` upstream, so a unit test cannot build
/// one — and "a verified delivery drives `verified` then `report`, an unverified
/// one drives NOTHING" is exactly the property this file has to pin.
pub trait FrontierMarshal: MarshalSink + Clone + Sync + 'static {
    /// The `(finalization, block)` pair of this node's highest LOCAL finalized
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
        // Re-read the block AT `h`, not `Identifier::Latest`: a block finalizing
        // between the two awaits would pair `fin@h` with `block@h+1`, which the
        // client's payload↔digest bind rejects as a lie — and the peer that
        // served it would be excluded for a race it did not cause.
        let block = self.get_block(h).await?;
        Some((fin, block))
    }

    /// Both reads at the SAME explicit height, never `Identifier::Latest`: a block
    /// finalizing between the two awaits would otherwise pair `fin@h` with
    /// `block@h+1`, exactly as [`Self::latest_pair`] documents.
    ///
    /// THE one body for this concrete type. `executor::BlockFetcher::pair_at` —
    /// the executor's erased seam, over the same `marshal::core::Mailbox` — calls
    /// straight into this one rather than repeating it (review B1-15).
    async fn pair_at(&self, height: Height) -> Option<(Cert, OrderBlock)> {
        let fin = self.get_finalization(height).await?;
        let block = self.get_block(height).await?;
        Some((fin, block))
    }
}

/// Serve THIS node's local marshal tip/archive for `key` — no network, no execution.
/// Encodes `(finalization, block)` exactly as the WS upstream / marshal resolver do
/// (`cert_inlet.rs`), so `decode_frontier` on the client side round-trips it.
async fn serve<M: FrontierMarshal>(marshal: &M, key: FrontierKey) -> Option<Bytes> {
    let pair = match key {
        FrontierKey::Latest => marshal.latest_pair().await?,
        FrontierKey::Finalized { height } => marshal.pair_at(Height::new(height)).await?,
    };
    Some(pair.encode())
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

/// Bridges the resolver engine's `Producer` (serve local tip) + `Consumer` (the
/// SINGLE point of trust for the frontier) to the shared [`Waiters`] map, the
/// committee module and the late-bound marshal. Cloned into the engine for both
/// roles, like the beacon's dealer-log resolver handler.
#[derive(Clone)]
pub struct FrontierHandler<E, M = MarshalMailbox> {
    waiters: Waiters,
    /// The node's own marshal, late-bound via the `marshal_slot` `OnceLock` (the
    /// marshal is created by the later layer launch). Until filled, `produce` serves
    /// "no data" and a verified delivery reaches nothing — which is safe, because
    /// the only thing lost is a tip advance the next probe asks for again.
    marshal_slot: Arc<OnceLock<M>>,
    /// The ONE committee module of this process: the geometry the height↔epoch
    /// bind is taken over and the record the window admits. NOT a second copy of
    /// either.
    committee: Arc<dyn Committee>,
    /// The chain namespace every certificate on this chain is signed under —
    /// `fluent_namespace(chain_id)`, the same value [`crate::committee::epoch_verifier`]
    /// closes over. Held because this file builds its own VERIFY-ONLY scheme (see
    /// the module docs) and the record does not carry the namespace.
    namespace: Arc<Vec<u8>>,
    /// One-slot memo of the verify-only scheme, keyed by the epoch it was built
    /// for. A frontier answer is judged against the epoch it names, and
    /// consecutive answers name the same epoch for a whole epoch's worth of
    /// heights, so a single slot is the whole cache — and it cannot grow with the
    /// chain the way a map keyed by epoch would. A miss costs one
    /// `VoteScheme::verifier` (a `BiMap` clone), which is noise beside the 2f+1
    /// pairing check that follows it.
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
    /// One rejected answer: counted, WARNed once per occurrence, and `deliver`
    /// answers `false` — which costs the sender this channel for the life of the
    /// resolver engine. Loud on purpose: every increment is a peer lost.
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

    /// The VERIFY-ONLY scheme for `epoch`, built from the record `deliver` just
    /// read and memoized in the one-slot cache.
    ///
    /// `oracle = None` is the whole point (see the module docs): the frontier
    /// judges "2f+1 under `committee[epoch]`, bound to `epoch`" and nothing else,
    /// because it has no beacon with which to tell an honest σ it cannot check
    /// from a forged one.
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

    /// The five steps of §5.2, in order, and nothing downstream of them runs
    /// until all five pass. See the module docs for why the `bool` is a SECOND
    /// decision rather than a restatement of the verdict.
    async fn deliver(&mut self, key: Self::Key, value: Self::Value) -> bool {
        // (1) DECODE, under the bounded signer-bitmap cap.
        let Some(uf) = decode_frontier(value.as_ref()) else {
            return Self::reject(REASON_UNDECODABLE, 0);
        };
        let round = uf.finalization.proposal.round;
        let height = uf.block.height;

        // (2) THE ANSWER IS THE ANSWER TO THE QUESTION. An honest marshal serves
        // `Finalized{h}` from exactly `h` (`marshal/core/actor.rs:808-818`), so a
        // foreign height is a substitution, not a race. The payload↔digest bind
        // belongs to the same step: a real certificate paired with a swapped body
        // is the same substitution one level down.
        if let FrontierKey::Finalized { height: asked } = key {
            if height != asked {
                return Self::reject(REASON_WRONG_HEIGHT, height);
            }
        }
        if crate::cold_start_jump::verify_jump_structural(&uf).is_err() {
            return Self::reject(REASON_PAYLOAD_MISMATCH, height);
        }

        // (3) HEIGHT ↔ EPOCH, over the module's ONE geometry. Before the freeze no
        // height means anything, so there is nothing to judge — that is an
        // UNAUTHENTICATED answer, not a lie.
        let epoch = round.epoch().get();
        let mut unauthenticated = None;
        match self.committee.epoch_of(height) {
            None => unauthenticated = Some(REASON_NO_GEOMETRY),
            Some(height_epoch) if height_epoch != epoch => {
                return Self::reject(REASON_EPOCH_MISMATCH, height);
            }
            Some(_) => {}
        }

        // (4) THE COMMITTEE, from the module. A failure here is never about the
        // PEER — an epoch outside this node's read window, an anchor below the
        // commit height, a read that faulted (the module already shouts) — so it
        // can never produce a `false`. What it produces is a certificate this node
        // CANNOT AUTHENTICATE, and step (5) decides what may be done with one.
        if unauthenticated.is_none() {
            unauthenticated = match self.committee.committee(epoch) {
                Err(CommitteeError::OutOfWindow { .. }) => Some(REASON_OUT_OF_WINDOW),
                Err(CommitteeError::NotReadable { .. }) => Some(REASON_NOT_READABLE),
                Err(CommitteeError::Read(_)) => Some(REASON_READ_FAILED),
                // The scheme is built HERE from the record, verify-only and
                // WITHOUT the epoch's seed oracle — see the module docs. Taking
                // `Committee::scheme` instead would make an honest certificate
                // fail on this node's own stale `PK_epoch` and ban the peer that
                // served it.
                Ok(record) => {
                    let scheme = self.verifier_for(epoch, &record);
                    if !uf.finalization.verify(&mut self.ctx, &scheme, &Sequential) {
                        // The committee IS readable, so this is a statement
                        // about the CERTIFICATE and not about this node's lag.
                        return Self::reject(REASON_BLS, height);
                    }
                    None
                }
            };
        }

        // (5) WHAT HAPPENS TO AN ANSWER THIS NODE CANNOT AUTHENTICATE. It is
        // COUNTED AND PASSED ON — not dropped — on BOTH arms, and that is the one
        // place where §5.2 as written does not survive contact with this codebase.
        // The module docs carry the two measurements; the short form is: the
        // by-height arm loses its only retry driver if the answer is withheld
        // (journal §0(6)(б), Д-72), and the `Latest` arm can be withheld safely
        // but its height is the only input of the deep re-jump trigger, so
        // dropping it wedges every node more than two epochs behind until that
        // trigger becomes tip-only (review A2-04).
        //
        // Passing it on trusts nothing extra. Every consumer below this line has
        // its own gate against the same committee module: a by-height pair goes
        // to the marshal's `verify_delivered` (BLS under `EpochSchemeProvider`,
        // the same map), and a tip goes to `cold_start_jump`, whose POST-sync
        // `verify_jump_authenticated` reads `committee[E]` at the LANDING's own
        // state — the one anchor at which a far-ahead epoch IS readable. What the
        // four checks above add is everything that does NOT need a committee, and
        // that is what closes R-009 (a foreign height), R-004 (an inflated one)
        // and a swapped body under a real certificate.
        if let Some(reason) = unauthenticated {
            metrics::counter!(FRONTIER_UNAUTHENTICATED, "reason" => reason).increment(1);
        }
        // The answer is admitted. It is NOT written into the marshal from here:
        // on every path that ends in the marshal, this value is handed on to the
        // marshal's OWN resolver handler
        // (`cert_inlet::UpstreamResolver::spawn_finalized` →
        // `handler.deliver(MarshalRequest::Finalized{h})`), which decodes,
        // BLS-verifies under the same committee module and stores it through
        // `store_finalization` — the single writer §5.2 names. Reporting here as
        // well would make this file a SECOND writer of the same finalization;
        // measured on the stand, that moves every jump landing and fires
        // `epoch_transition.rs`'s "two boundaries pending at once" debug assert.
        let waiting = self.waiters.lock().unwrap().remove(&key);
        // The awaiting `get_latest` / `get_finalization` calls. They are still here
        // in pass A — what changed is that ONLY a five-step-verified answer can
        // reach them, so nothing downstream of this file consumes an unchecked
        // frontier any more.
        for tx in waiting.into_iter().flatten() {
            let _ = tx.send(uf.clone());
        }
        true
    }

    async fn failed(&mut self, _: Self::Key, _: Self::Failure) {
        // No-op: the awaiting call times out on its own and cleans its waiter; the
        // marshal / jump re-requests on the next repair sweep / re-jump edge.
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
/// `OnceLock` the node fills after the layer launch (`node/dpos.rs`); `committee` is the
/// process's ONE committee module, which is what makes `deliver` able to judge an answer
/// at all; `chain_id` is what the certificate namespace is derived from — the same
/// `fluent_namespace(chain_id)` [`crate::committee::epoch_verifier`] uses, so the scheme
/// `deliver` builds and the scheme the rest of the process holds differ in exactly one
/// thing, the absent seed oracle.
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

/// A plane-native [`CertUpstream`] over the frontier resolver. Holds the runtime
/// clock (to bound a fetch), the client resolver mailbox (to issue fetches) + the
/// shared [`Waiters`] map (to await the resolver's delivery). Cloneable (mpsc +
/// `Arc`), `Send + Sync + 'static`.
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
    /// timeout. UNTARGETED: the resolver picks + rotates peers from the tracked
    /// set on its own, and with an empty tracked set the fetch is a no-op and this
    /// returns `None` after the timeout.
    ///
    /// The ladder step's `fetch_targeted` is NOT issued here, and trying to put it
    /// here is what this comment exists to stop anyone repeating: a targeted fetch
    /// has no fallback, and re-deriving targets from the HEIGHT inside this call
    /// retargets whichever ordinary by-height repair pull happens to land on
    /// `last(T+1)` — measured on the stand, that starves the contiguous catch-up
    /// of a node that has just landed a jump and it never finishes.
    ///
    /// Carrying the step's OWN target list down to here — the A2-05 / B1-10 route
    /// — was built and MEASURED in the third pass, and rolled back: see
    /// `cert_inlet::UpstreamResolver::fetch_targeted` for the number. The step is
    /// addressed where it is ISSUED (`executor::probe_frontier` →
    /// `marshal.hint_finalization(height, committee[T+1])`); how far those targets
    /// travel on each resolver shape is the journal's §7.
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
        // The bound runs on the runtime `Clock` (the `beacon/artifact.rs` pull
        // seam's form): a tokio timer would need a tokio reactor, which the
        // deterministic runner does not provide.
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

    /// A real committee: peer keys, BLS keys, and the `(signers, verifier)` pair
    /// a genuine 2f+1 finalization needs. Verbatim in shape from the cert-inlet
    /// unit tests — a certificate that is not really signed proves nothing about
    /// a gate whose whole job is to check the signature.
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

        /// A REAL 2f+1 finalization over `block`, under `epoch`'s committee.
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

    /// Records the marshal driving calls in order — the ONLY witness that a
    /// verified delivery drives `verified` then `report` and that a refused one
    /// drives NOTHING.
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
    /// committee module double, a recording marshal, and the deterministic
    /// runner's context as the certificate-verify RNG.
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

    /// A module that answers `epoch` with the fixture's record + verifier, over
    /// a frozen 32-block geometry and the window `[lo, hi]`.
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

    /// The node's local beacon key for the epoch came from a DIFFERENT mint —
    /// Д-75's shape. It resolves (so this is not `NoKey`) and it disagrees with
    /// the σ an honest quorum recovered, so every assembled seed is `Invalid`.
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

    /// The module as a node in a BEACON-ACTIVE epoch holds it: the same record,
    /// but the scheme carries a seed oracle — and that oracle's key is the wrong
    /// one ([`StaleEpochKeyOracle`]).
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
    /// knows — shape-valid so the certificate round-trips, unverifiable so a
    /// wrong-key oracle answers `Invalid` on it. Stands in for the honest σ of a
    /// beacon-active epoch as seen by a node whose `PK_epoch` is stale.
    fn unvouchable_seed(round: Round) -> fluentbase_bls::BlsSignature {
        use commonware_cryptography::bls12381::primitives::group::{Private, Share};
        let mut rng = StdRng::seed_from_u64(0x5EED);
        let share = Share::new(
            commonware_utils::Participant::new(0),
            Private::random(&mut rng),
        );
        fluentbase_bls::beacon::sign_seed_partial(&share, b"some-other-mint", round).value
    }

    /// Register a waiter for `key` and hand back the receiving half, so a test
    /// can assert whether the answer was FANNED OUT or dropped.
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

    /// (а) The honest answer: height matches the key, its epoch matches the
    /// certificate's round epoch, and the multisig verifies under the committee
    /// this node can read. `deliver` answers `true` and the awaiting call is
    /// resolved.
    ///
    /// And the marshal is NOT driven from here — asserted, not merely absent.
    /// §5.2's step (5) puts `verify_block` + `report_finalization` in `deliver`;
    /// by the code the value this call admits is handed on to the marshal's OWN
    /// resolver handler, which BLS-verifies it under the same committee module and
    /// stores it through `store_finalization`. Driving the marshal here as well
    /// makes this file a second writer of the same finalization (journal §0(6)).
    ///
    /// Falsifier: a `false`; an unresolved waiter; ANY marshal call.
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

    /// (б) R-009: the answer to `Finalized{h}` carries a wholly real,
    /// self-consistent finalization of a DIFFERENT height. An honest marshal
    /// serves exactly `h` (`marshal/core/actor.rs:808-818`), so this is a
    /// substitution — `deliver` must answer `false` (commonware then excludes the
    /// sender), drive NOTHING into the marshal, and leave the waiter unresolved.
    ///
    /// RED before this change: HEAD's `deliver` never compared the key to the
    /// delivered height — it returned `true` and fanned the wrong-height pair to
    /// the waiter.
    ///
    /// Falsifier: a `true`; any marshal call; a resolved waiter.
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

    /// (в) The height↔epoch bind: a certificate whose round epoch is not the
    /// epoch the served height belongs to. The per-epoch engine only ever
    /// proposes inside its own height range, so this is a malformed / cross-epoch
    /// certificate and a lie signal — `false`, nothing driven. This is also the
    /// arm an INFLATED `Latest` (R-004) lands on: adding `10^6` to the height
    /// moves it out of the certificate's epoch long before it moves out of the
    /// read window.
    ///
    /// Falsifier: a `true`; any marshal call.
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

    /// (г1) An epoch the read window does not admit, delivered under `Latest` —
    /// an honest peer far ahead of this node. Unverifiable is NOT a lie: `deliver`
    /// answers `true`, the peer keeps the channel, and the answer is COUNTED and
    /// PASSED ON to the gate that can check it (`cold_start_jump`'s POST-sync
    /// `verify_jump_authenticated`, at the landing's own state).
    ///
    /// §5.2 says to DROP it here, and unlike the by-height arm (г2) nothing would
    /// be ORPHANED by that — `fetch_one` clears its own waiter and cancels the
    /// fetch on the timeout, and the next probe tick asks again. What stopped it
    /// in pass А was the CONSUMER: this height was the only input of the deep
    /// re-jump trigger. Pass Б1 took that consumer away (the trigger reads the
    /// marshal tip, the target comes out of the local archive), so the height now
    /// buys one `hint_finalization` and the probe's "was I served" bit. The
    /// remaining consumer of an unverified `Latest` as a TARGET is the PRE-ENGINE
    /// cold-start jump on an empty archive, which is what pass Б2 rebuilds — and
    /// the drop lands with it (review A2-04, §6 п.2-3).
    ///
    /// Falsifier: a `false` (the honest peer would be excluded for this node's own
    /// lag); a marshal call; an unresolved waiter.
    #[test]
    fn an_out_of_window_latest_is_passed_on_unauthenticated() {
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
                rx.await.is_ok(),
                "the answer was withheld — the deep re-jump trigger has no other input"
            );
        });
    }

    /// (г2) The SAME out-of-window answer under a BY-HEIGHT key. Same verdict, and
    /// a second reason for it that (г1) does not have: `Finalized{h}` is driven by
    /// `UpstreamResolver::spawn_finalized`, a one-shot spawned pull re-armed only
    /// by the marshal's `try_repair_gaps`, which runs per STORED finalization — so
    /// withholding the answer removes the event that would ask for it again, and a
    /// node that stores nothing never asks again (journal §0(6)(б), Д-72; measured
    /// as the Д3 precondition's node 3 never fetching another height after its
    /// landing). The pair still meets the marshal's own `verify_delivered` below.
    ///
    /// Falsifier: a `false`; a marshal call; an UNRESOLVED waiter.
    #[test]
    fn an_out_of_window_by_height_answer_is_passed_on_unauthenticated() {
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
                rx.await.is_ok(),
                "the answer was withheld — the by-height path has no other retry driver"
            );
        });
    }

    /// (д) The epoch is inside the window but this node's anchor has not reached
    /// its commit height — `CommitteeError::NotReadable`. Same verdict as (г) and
    /// for the same reason: this is a statement about THIS node, not about the
    /// peer, and withholding it would strand the fetch.
    ///
    /// Falsifier: a `false`; a marshal call; an unresolved waiter.
    #[test]
    fn an_unreadable_committee_passes_the_answer_on_without_punishing_the_peer() {
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
            assert!(rx.await.is_ok(), "the answer was withheld");
        });
    }

    /// (е) The committee IS readable and the multisig does not verify under it —
    /// the one arm where a failing signature is a statement about the
    /// CERTIFICATE. `false`, nothing driven.
    ///
    /// The forgery is the R-004/R-001 shape: the served body is swapped and the
    /// certificate's payload re-pointed at the new digest, so the structural
    /// payload↔digest bind still passes and the multisig — a real signature over
    /// the ORIGINAL payload — is the only thing left to catch it.
    ///
    /// Falsifier: a `true`; a marshal call; the tamper not actually changing the
    /// served block.
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

    /// (е2) Д-75, the honest-peer ban this pass closes. The certificate is a REAL
    /// 2f+1 multisig of the right height in the right epoch, carrying the σ of a
    /// beacon-active epoch. This node's local key for that epoch came from a
    /// different mint, so its oracle calls the σ `Invalid` — and
    /// `CombinedScheme::verify_certificate` fails the WHOLE certificate on it
    /// (`bls/src/combined_scheme.rs:431-438`), which on this path means a
    /// permanent exclusion of an honest peer from the frontier channel.
    ///
    /// `deliver` therefore builds its own VERIFY-ONLY scheme from the record it
    /// just read, with no oracle: 2f+1 under `committee[epoch]` plus the epoch
    /// binding is exactly what §5.2 asks the frontier to check, nothing on this
    /// path consumes σ, and the frontier has no beacon to resolve the key with
    /// first the way the cert inlet does (`cert_inlet.rs:620`).
    ///
    /// NOT VACUOUS: the test proves the arm is live by verifying the same
    /// certificate under the MODULE's scheme first and asserting it FAILS there.
    ///
    /// RED before this change: `deliver` took `Committee::scheme(epoch)`, so this
    /// honest answer returned `false`.
    ///
    /// Falsifier: the module's scheme accepting the certificate (then the stale
    /// key is not modelled); a `false`; an unresolved waiter.
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
            // THE WITNESS: under the module's own scheme this honest certificate
            // does not verify, and that verdict is what used to ban the peer.
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

    /// (ж) Undecodable bytes: `false`, nothing driven, the awaiting waiter kept
    /// (the fetch stays alive so the resolver tries another peer). The one arm
    /// that behaved this way before this change.
    ///
    /// Falsifier: a `true`; a marshal call; a resolved or dropped waiter.
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

    /// Before the geometry is frozen no height means anything, so an answer
    /// cannot be judged at all: counted and passed on with `true`, never refused.
    /// Answering `false` here would exclude every peer a node talks to during the
    /// window between its start and its first readable, DPoS-scheduled block.
    ///
    /// Falsifier: a `false`; a marshal call.
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
