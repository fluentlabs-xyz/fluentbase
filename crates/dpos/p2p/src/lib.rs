//! Fluent DPoS p2p layer: commonware_p2p::authenticated::discovery wiring.
//!
//! Injection-style library: every collaborator is a constructor
//! parameter, so this compiles and unit-tests today against the pinned
//! commonware (`monorepo @ v2026.4.0`); only the live instances (the
//! node-assembly thread + `tokio::Runner`, Reth-handle injection, the
//! epoch_manager mux consumption) are deferred.
//!
//! Public API:
//! - [`FluentP2P::build`] — sync constructor (Network::new + 9× register); returns the Network owner + [`FluentP2PHandles`] (clone-into-04/06).
//! - [`FluentP2P::start`] — consumes self, calls `Network::start`.
//! - [`FluentP2PHandles`] — Fluent-domain handles (no commonware-p2p types leaked into 04's / 06's public API beyond the `OracleHandle` newtype and the `DiscSender` / `DiscReceiver` type aliases, which are intentional re-exports).

pub mod bootstrappers;
pub mod config;
pub mod constants;
pub mod ingress;

pub use ingress::parse_ingress;

use commonware_cryptography::ed25519;
use commonware_p2p::{
    authenticated::discovery::{Network, Oracle, Receiver, Sender},
    Blocker, Manager, PeerSetSubscription, Provider, TrackedPeers,
};
use commonware_runtime::{
    BufferPooler, Clock, Handle, Metrics, Network as RNetwork, Resolver, Spawner, Storage,
};
use fluentbase_bls::PeerPubkey;
use fluentbase_staking_reader::{PeerSetSink, TrackedPeers as TrackedPeersOf};
use rand_core::CryptoRngCore;
use std::sync::{Arc, RwLock};

pub use config::FluentP2PConfig;

/// Load a commonware Ed25519 `PrivateKey` from a hex-encoded text file.
/// Symmetric to `fluentbase_bls::ValidatorBlsKeypair::read_from_file`.
/// Accepts `0x`-prefixed or bare hex; trims surrounding whitespace.
/// On Unix, rejects files with group/other permissions set
/// (`mode & 0o077 != 0`) to prevent silent loading of world-readable
/// peer keys.
pub fn read_ed25519_key_from_file<P: AsRef<std::path::Path>>(
    path: P,
) -> eyre::Result<ed25519::PrivateKey> {
    use commonware_codec::DecodeExt as _;
    let path_ref = path.as_ref();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let meta = std::fs::metadata(path_ref)
            .map_err(|e| eyre::eyre!("failed stat-ing peer key file: {e}"))?;
        let mode = meta.permissions().mode();
        if mode & 0o077 != 0 {
            return Err(eyre::eyre!(
                "peer key file {} has insecure permissions {:o}; \
                 chmod 600 required (group/other access denied)",
                path_ref.display(),
                mode & 0o777,
            ));
        }
    }
    // Wrap the plaintext key material so it is scrubbed on drop — the ed25519
    // signing scalar must not linger in freed heap (mirrors the BLS plaintext
    // loader, `bls/src/keys.rs`; audit P2-9). We strip whitespace + an optional
    // `0x` into a Zeroizing buffer and call `from_hex` DIRECTLY:
    // `from_hex_formatted` does an internal `hex.replace(...)` that would leave a
    // NON-zeroized copy of the key hex in freed heap. `PrivateKey::decode` copies
    // into a zeroizing `Secret`, so these buffers are then the only residue.
    let raw = zeroize::Zeroizing::new(
        std::fs::read_to_string(path_ref)
            .map_err(|e| eyre::eyre!("failed reading peer key file: {e}"))?,
    );
    let cleaned = zeroize::Zeroizing::new(
        raw.chars()
            .filter(|c| !c.is_ascii_whitespace())
            .collect::<String>(),
    );
    let hex = cleaned.strip_prefix("0x").unwrap_or(&cleaned);
    let bytes = zeroize::Zeroizing::new(
        commonware_utils::from_hex(hex)
            .ok_or_else(|| eyre::eyre!("peer key file contents not valid hex"))?,
    );
    ed25519::PrivateKey::decode(bytes.as_slice())
        .map_err(|e| eyre::eyre!("failed decoding peer key: {e:?}"))
}

/// Generate a fresh, OS-randomness ed25519 keypair for an EPHEMERAL peer
/// identity. Used by a standalone `--cert-follow` follower, which carries no
/// operator peer key but still builds ONE gossip-idle broadcast `Muxer` (it
/// never gossips — no consensus bootstrappers — so the identity is never
/// persisted or advertised). NOT for a validator (those load a durable key from
/// disk via [`read_ed25519_key_from_file`]).
pub fn generate_ephemeral_ed25519_key() -> ed25519::PrivateKey {
    use commonware_math::algebra::Random as _;
    ed25519::PrivateKey::random(&mut rand_08::rngs::OsRng)
}

/// Local newtype wrapper around the concrete commonware discovery
/// `Oracle`. Required because Rust's orphan rule
/// forbids `impl PeerSetSink for Oracle<…>` directly (both trait and
/// type are foreign to this crate). The inner `Oracle` is re-exported
/// from `commonware_p2p::authenticated::discovery::Oracle` (NOT the
/// deeper `actors::tracker::Oracle` path — prefer the shortest valid
/// public path). Implements `Clone` cheaply (Oracle's
/// `UnboundedMailbox` is Arc-shared internally).
#[derive(Clone, Debug)]
pub struct OracleHandle {
    inner: Oracle<ed25519::PublicKey>,
    /// The set this handle last registered, kept for the channel ingress checks.
    /// Shared with every clone — there is one `EpochTransition` and one writer.
    window: TrackedWindow,
}

/// Concrete commonware-p2p sender/receiver for our channel layout.
/// `Sender`/`Receiver` here are the **structs** re-exported at
/// `discovery::{Sender, Receiver}` (NOT the top-level traits of the
/// same name — name collision is intentional in the lib; the structs
/// implement the traits).
pub type DiscSender<E> = Sender<ed25519::PublicKey, E>;
pub type DiscReceiver = Receiver<ed25519::PublicKey>;

/// Fluent p2p layer state. Owns the discovery `Network` privately.
///
/// The per-epoch demux for vote/cert/resolver lives inside the Fluent
/// `epoch_manager` (see
/// `crates/consensus/src/epoch_manager.rs`), which builds its own
/// `Muxer`s over the raw channels. So 05 exposes
/// the raw 9-channel `(Sender, Receiver)` pairs and does NOT pre-mux
/// vote/cert/resolver here — pre-muxing was redundant and would
/// double-demux.
pub struct FluentP2P<E>
where
    E: BufferPooler + Clock + CryptoRngCore + Spawner + Storage + Metrics + RNetwork + Resolver,
{
    network: Network<E, ed25519::PrivateKey>,
}

/// Handles injected into 04 / 06.
///
/// NOT `Clone`: each `(Sender, Receiver)` pair is move-only (wraps an
/// mpsc Receiver) and single-consumer by design. Only `OracleHandle` is
/// cloneable (`oracle.clone()` distributes the Oracle across multiple
/// consumers — 03 EpochTransition + future 04 `Config.participants`).
pub struct FluentP2PHandles<E>
where
    E: BufferPooler + Clock + CryptoRngCore + Spawner + Storage + Metrics + RNetwork + Resolver,
{
    pub oracle: OracleHandle,

    // VOTE / CERT / RESOLVER: per-epoch demuxed by EpochManager
    // (Muxers built inside `epoch_manager::run`). 05 returns raw channels.
    pub vote_sender: DiscSender<E>,
    pub vote_receiver: DiscReceiver,
    pub cert_sender: DiscSender<E>,
    pub cert_receiver: DiscReceiver,
    pub resolver_sender: DiscSender<E>,
    pub resolver_receiver: DiscReceiver,

    // Per 04: BROADCAST + MARSHAL are global one-instance channels
    // (not Muxed). Consumed once by `buffered::Engine` /
    // `marshal::resolver::p2p::init` in 04's OuterEngine.
    pub broadcast_sender: DiscSender<E>,
    pub broadcast_receiver: DiscReceiver,
    pub marshal_sender: DiscSender<E>,
    pub marshal_receiver: DiscReceiver,

    // BEACON: global one-instance channel for the randomness-beacon DKG
    // ceremony (the recovered seed rides in the consensus cert, NOT here — the
    // old seed side-channel was deleted). Consumed once by the live `DkgActor`
    // in 04's launch. Per-epoch Muxing is deferred.
    pub beacon_sender: DiscSender<E>,
    pub beacon_receiver: DiscReceiver,

    // BEACON_RESOLVER: global one-instance channel for the DKG-log recovery
    // resolver (`commonware_resolver::p2p`). Consumed once by the beacon-plane
    // resolver engine in `node/dpos.rs::build_beacon_plane`.
    pub beacon_resolver_sender: DiscSender<E>,
    pub beacon_resolver_receiver: DiscReceiver,

    // FRONTIER: global one-instance channel for the plane-native `CertUpstream`
    // frontier/by-height resolver (`commonware_resolver::p2p`). Consumed once by the
    // frontier resolver engine in `node/dpos.rs::build_beacon_plane`.
    pub frontier_sender: DiscSender<E>,
    pub frontier_receiver: DiscReceiver,

    // EVIDENCE: global one-instance channel for equivocation-evidence gossip —
    // the votes a node republishes for a decided round so the two halves of a
    // split-delivered equivocation can meet. Consumed once by the evidence task
    // in `node/dpos.rs::build_beacon_plane`.
    pub evidence_sender: DiscSender<E>,
    pub evidence_receiver: DiscReceiver,
}

impl<E> FluentP2P<E>
where
    E: BufferPooler + Clock + CryptoRngCore + Spawner + Storage + Metrics + RNetwork + Resolver,
{
    /// Build the p2p layer: instantiate commonware Network, register
    /// 9 top-level channels. Per-epoch demux for vote/cert/resolver is
    /// handled inside the consensus `EpochManager` (it builds its own
    /// `Muxer`s), so this layer returns raw channels here.
    pub fn build(ctx: E, cfg: FluentP2PConfig) -> (Self, FluentP2PHandles<E>) {
        let commonware_cfg = cfg.into_commonware_config();
        let (mut network, oracle) = Network::new(ctx.with_label("p2p_network"), commonware_cfg);

        // Register 3 per-epoch-demuxed channels (consumed by the consensus EpochManager).
        let (vote_s, vote_r) = network.register(
            constants::VOTE_CHANNEL,
            constants::VOTE_QUOTA,
            constants::VOTE_BACKLOG,
        );
        let (cert_s, cert_r) = network.register(
            constants::CERT_CHANNEL,
            constants::CERT_QUOTA,
            constants::CERT_BACKLOG,
        );
        let (res_s, res_r) = network.register(
            constants::RESOLVER_CHANNEL,
            constants::RESOLVER_QUOTA,
            constants::RESOLVER_BACKLOG,
        );

        // Register 6 global one-instance channels:
        //    BROADCAST (block-data via buffered::Engine) +
        //    MARSHAL (backfill via marshal::resolver::p2p::init) +
        //    BEACON (randomness-beacon DKG + per-height seed partials) +
        //    BEACON_RESOLVER (DKG-log recovery) +
        //    FRONTIER (plane-native CertUpstream frontier/by-height pull) +
        //    EVIDENCE (equivocation-evidence gossip).
        let (br_s, br_r) = network.register(
            constants::BROADCAST_CHANNEL,
            constants::BROADCAST_QUOTA,
            constants::BROADCAST_BACKLOG,
        );
        let (mr_s, mr_r) = network.register(
            constants::MARSHAL_CHANNEL,
            constants::MARSHAL_QUOTA,
            constants::MARSHAL_BACKLOG,
        );
        let (beacon_s, beacon_r) = network.register(
            constants::BEACON_CHANNEL,
            constants::BEACON_QUOTA,
            constants::BEACON_BACKLOG,
        );
        let (beacon_res_s, beacon_res_r) = network.register(
            constants::BEACON_RESOLVER_CHANNEL,
            constants::BEACON_RESOLVER_QUOTA,
            constants::BEACON_RESOLVER_BACKLOG,
        );
        let (frontier_s, frontier_r) = network.register(
            constants::FRONTIER_CHANNEL,
            constants::FRONTIER_QUOTA,
            constants::FRONTIER_BACKLOG,
        );
        let (evidence_s, evidence_r) = network.register(
            constants::EVIDENCE_CHANNEL,
            constants::EVIDENCE_QUOTA,
            constants::EVIDENCE_BACKLOG,
        );

        let handles = FluentP2PHandles {
            oracle: OracleHandle {
                inner: oracle,
                window: TrackedWindow::default(),
            },
            vote_sender: vote_s,
            vote_receiver: vote_r,
            cert_sender: cert_s,
            cert_receiver: cert_r,
            resolver_sender: res_s,
            resolver_receiver: res_r,
            broadcast_sender: br_s,
            broadcast_receiver: br_r,
            marshal_sender: mr_s,
            marshal_receiver: mr_r,
            beacon_sender: beacon_s,
            beacon_receiver: beacon_r,
            beacon_resolver_sender: beacon_res_s,
            beacon_resolver_receiver: beacon_res_r,
            frontier_sender: frontier_s,
            frontier_receiver: frontier_r,
            evidence_sender: evidence_s,
            evidence_receiver: evidence_r,
        };
        let me = Self { network };
        (me, handles)
    }

    /// Consume self, start the discovery network's actor tree. Returns
    /// the `Handle<()>` for shutdown coordination. After this point no
    /// more `Network::register` is allowed.
    pub fn start(self) -> Handle<()> {
        self.network.start()
    }
}

// PeerSetSink adapter.
//
// `EpochTransition` is p2p-agnostic; the one-liner adapter lives here,
// where the OracleHandle is in scope.

/// Adapter: `EpochTransition` calls `track(epoch, TrackedPeers)`; the two tiers
/// go to `commonware_p2p::Manager::track` as commonware's own `TrackedPeers`,
/// and the same value is kept in [`TrackedWindow`] so a channel's ingress check
/// can classify a sender against THE set that was registered, without asking
/// commonware for it per frame. Commonware sorts the Sets internally
/// (`Set::from_iter_dedup`) — no caller-side `.sort()`; the canonical byte-lex
/// order is pinned by `crates/bls/tests/ed25519_ordering_conformance.rs`.
impl PeerSetSink for OracleHandle {
    // `async fn` here matches the trait's `-> impl Future + Send`
    // (Rust auto-promotes the future to `Send` when all captures are
    // Send — `&mut self.inner` is Send via `Oracle: Send`). Manager::track
    // is also `async fn` but body is `send_lossy` (no real await pressure).
    async fn track(&mut self, epoch: u64, peers: TrackedPeersOf) {
        // Record BEFORE registering: the window is what every ingress check
        // reads, and a frame from a member of the new set may arrive the instant
        // commonware applies it.
        self.window.record(epoch, &peers);
        let registered = TrackedPeers::new(peers.primary(), peers.secondary);
        Manager::track(&mut self.inner, epoch, registered).await
    }
}

/// "Is this peer tombstoned on chain" — injected rather than typed, because the
/// `TombstoneSet` lives in the consensus crate, which is ABOVE this one in the
/// dependency graph.
pub type TombstonePredicate = Arc<dyn Fn(&PeerPubkey) -> bool + Send + Sync>;

/// The last peer set this node registered, readable by every channel's ingress
/// check. Cloneable and shared: one writer (the `PeerSetSink` adapter above,
/// driven by the single `EpochTransition`), many readers.
///
/// Why not `Provider::peer_set` / `Provider::subscribe`: commonware keeps only
/// the flat union (`latest.primary`), and the question a channel asks is
/// per-epoch — "which of `C[E−1]`, `C[E]`, `C[E+1]` does this sender sit in" —
/// which the union cannot answer. It also costs a mailbox round-trip per call.
#[derive(Clone, Default)]
pub struct TrackedWindow {
    /// `(tracked epoch, the set)`. `None` until the first `track`.
    latest: Arc<RwLock<Option<(u64, TrackedPeersOf)>>>,
    /// Peers observed tombstoned on-chain. Unset ⇒ nobody is tombstoned, which is
    /// the honest answer for a node that has not wired the slasher.
    tombstoned: Option<TombstonePredicate>,
}

impl std::fmt::Debug for TrackedWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrackedWindow")
            .field(
                "epoch",
                &self.latest.read().map(|g| g.as_ref().map(|(e, _)| *e)).ok(),
            )
            .finish_non_exhaustive()
    }
}

impl TrackedWindow {
    /// A window that also drops tombstoned senders. One call per process, at the
    /// wiring site that owns the `TombstoneSet`.
    pub fn with_tombstones(mut self, tombstoned: TombstonePredicate) -> Self {
        self.tombstoned = Some(tombstoned);
        self
    }

    /// Publish the set just handed to `track`.
    pub fn record(&self, epoch: u64, peers: &TrackedPeersOf) {
        if let Ok(mut slot) = self.latest.write() {
            *slot = Some((epoch, peers.clone()));
        }
    }

    /// Classify a frame's sender. `None` means "no peer set registered yet" —
    /// NOT "drop": before the first `track` this node has no membership opinion
    /// at all, and a check that turned that into a drop would silence the plane
    /// for the whole of cold start.
    pub fn classify<'a>(&self, peer: &'a PeerPubkey) -> Option<Ingress<'a>> {
        if self.tombstoned.as_ref().is_some_and(|t| t(peer)) {
            return Some(Ingress::Dropped);
        }
        let guard = self.latest.read().ok()?;
        let (_, peers) = guard.as_ref()?;
        let mask = EpochMask::of(peers, peer);
        Some(if mask.is_empty() {
            if peers.secondary.position(peer).is_some() {
                Ingress::Tracked(peer)
            } else {
                Ingress::Dropped
            }
        } else {
            Ingress::Member { peer, epochs: mask }
        })
    }
}

/// What a frame's sender is, on the peer set this node last registered.
///
/// No penalty and no counter rides on this: a `Dropped` frame is dropped and
/// counted on the channel's own metric, nothing more. The only global ban is the
/// on-chain tombstone (PLAN §8 п.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ingress<'a> {
    /// In at least one carried committee record — `epochs` says which.
    Member {
        peer: &'a PeerPubkey,
        epochs: EpochMask,
    },
    /// Tier 2: registered, served, but in no committee.
    Tracked(&'a PeerPubkey),
    /// Not in the registered set at all, or tombstoned.
    Dropped,
}

impl Ingress<'_> {
    /// Whether the sender is a member of `epoch`'s committee record.
    pub fn member_of(&self, epoch: u64) -> bool {
        matches!(self, Ingress::Member { epochs, .. } if epochs.contains(epoch))
    }

    /// The metric's `reason` label for a frame this classification refuses.
    pub const fn refusal(&self) -> &'static str {
        match self {
            Ingress::Member { .. } => "none",
            Ingress::Tracked(_) => "secondary",
            Ingress::Dropped => "untracked",
        }
    }
}

/// The epochs of the tracked window whose committee record holds a given peer.
/// At most three by construction (`C[E−1]`, `C[E]`, `C[E+1]`), so it is a fixed
/// array — no allocation on a per-frame path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EpochMask {
    slots: [u64; 3],
    len: u8,
}

impl EpochMask {
    /// The mask of the records in `peers` that hold `peer`.
    ///
    /// Silently capped at three, and `debug_assert`ed so the cap can never become
    /// a SILENT truncation in a test run: the invariant is
    /// `TrackedPeers::committees` carrying at most three records with distinct
    /// epochs (see its own doc), which `assemble_tracked_peers` guarantees by
    /// construction. A fourth record would mean the window grew and this fixed
    /// array has to grow with it — dropping the overflow instead would answer
    /// "not a member" for an epoch the peer IS in.
    fn of(peers: &TrackedPeersOf, peer: &PeerPubkey) -> Self {
        debug_assert!(
            peers.committees.len() <= 3,
            "the tracked window is at most three committee records, got {}",
            peers.committees.len()
        );
        let mut mask = Self::default();
        for epoch in peers.epochs_of(peer) {
            if (mask.len as usize) < mask.slots.len() {
                mask.slots[mask.len as usize] = epoch;
                mask.len += 1;
            } else {
                debug_assert!(false, "more than three epochs in one membership mask");
            }
        }
        mask
    }

    pub fn contains(&self, epoch: u64) -> bool {
        self.slots[..self.len as usize].contains(&epoch)
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

// Blocker + Provider delegating impls.
//
// `OuterBuilder` takes `B: Blocker<PublicKey = ed25519::PublicKey>` and
// `P: Provider<PublicKey = ed25519::PublicKey>`. The inner `Oracle<C>`
// satisfies both upstream; this newtype delegates verbatim so the node
// can pass `handles.oracle.clone()` directly to the Builder.

impl OracleHandle {
    /// The peer-set window this handle writes on every `track`. Clones share the
    /// same storage, so an ingress check built from any clone sees the live set.
    pub fn window(&self) -> TrackedWindow {
        self.window.clone()
    }
}

impl Blocker for OracleHandle {
    type PublicKey = ed25519::PublicKey;

    async fn block(&mut self, peer: Self::PublicKey) {
        // 4-hour block per call (commonware `Config::block_duration` default,
        // not overridden by Fluent);
        // surface every block as an audit-trail event so a misbehaving
        // (or malicious) caller can be traced.
        tracing::warn!(target: "fluentbase_p2p::blocker", ?peer, "peer block requested");
        Blocker::block(&mut self.inner, peer).await
    }
}

/// A [`Blocker`] that does NOTHING. Wired into the BEACON-log recovery resolver
/// (`commonware_resolver::p2p`) AND the consensus vote/cert plane — the two
/// `OuterBuilder.blocker` sites in `crates/dpos/consensus/src/dpos.rs` (the
/// signer/batcher path and the follower path) — in place of the shared network
/// [`OracleHandle`].
///
/// Vote/cert-plane rationale (Bug A): the simplex batcher blocks every
/// batch-verify-failed signer with an evidence-free `block!(self.blocker, ..)` and
/// no threshold. The blockable set there is provably ⊆ `committee[E]` (the batcher
/// only blocks a peer it resolves as a participant), so a transient FALSE verdict
/// (e.g. a CHANGE-epoch PK_E settle-race) would 4h-partition an HONEST committee
/// member on the single global transport tracker — a self-inflicted consensus
/// partition. The `block!` macro discards the reason string before calling
/// `Blocker::block(peer)`, so a Fluent-side blocker cannot be reason-selective;
/// no-oping softens both the evidence-free `"invalid signature"` and the
/// evidence-backed `"conflicting notarize/finalize"`. Slashing is PRESERVED: the
/// equivocation evidence rides `reporter.report(Activity::Conflicting*)` (simplex
/// `batcher/round.rs`) BEFORE and INDEPENDENT of the `block!`, so the on-chain
/// tombstone (immediate jail, drop at E+2) still lands. The follower runs no
/// batcher but shares the marshal cert resolver, so the same no-op closes the
/// cert-plane instance of the self-partition there too.
///
/// The resolver runs `blocker.block(peer)` on a `deliver=false` verdict (an undecodable
/// or wrong-dealer DKG-log response). With the shared `OracleHandle` as blocker that hits
/// the SINGLE global transport tracker, severing the peer from ALL channels (vote / cert /
/// marshal / broadcast / beacon) for the 4-hour block — a self-inflicted consensus
/// partition triggered by a benignly codec-skewed honest peer answering a recovery fetch
/// (review [1013]). A no-op external blocker removes that collateral: the resolver's OWN
/// abuse defense (`fetcher.block` excluded-set + inbound quota + `add_retry`) is
/// INDEPENDENT of this hook and stays intact, so the beacon-log fetch path keeps its DoS
/// protection while consensus connectivity is never collateral. (fix-plan-v3 Fix B.)
#[derive(Clone, Default)]
pub struct NoopBlocker;

impl Blocker for NoopBlocker {
    type PublicKey = ed25519::PublicKey;

    async fn block(&mut self, _peer: Self::PublicKey) {}
}

impl Provider for OracleHandle {
    type PublicKey = ed25519::PublicKey;

    async fn peer_set(&mut self, id: u64) -> Option<TrackedPeers<Self::PublicKey>> {
        Provider::peer_set(&mut self.inner, id).await
    }

    async fn subscribe(&mut self) -> PeerSetSubscription<Self::PublicKey> {
        Provider::subscribe(&mut self.inner).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use commonware_cryptography::ed25519::PrivateKey;
    use commonware_p2p::Ingress;
    use commonware_runtime::{deterministic, Runner};
    use rand_core::SeedableRng as _;
    use std::net::SocketAddr;

    fn make_config(seed: u64, listen: SocketAddr) -> FluentP2PConfig {
        use commonware_math::algebra::Random as _;
        let mut rng = rand_08::rngs::StdRng::seed_from_u64(seed);
        let sk = PrivateKey::random(&mut rng);
        FluentP2PConfig {
            crypto: sk,
            chain_id: 1337,
            listen,
            dialable: Ingress::Socket(listen),
            bootstrappers: vec![],
        }
    }

    #[cfg(unix)]
    fn chmod_600(path: &std::path::Path) {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    #[cfg(not(unix))]
    fn chmod_600(_: &std::path::Path) {}

    #[test]
    fn read_ed25519_key_from_file_round_trips_with_and_without_prefix_and_whitespace() {
        use commonware_codec::Encode as _;
        use commonware_math::algebra::Random as _;
        let mut rng = rand_08::rngs::StdRng::seed_from_u64(13);
        let sk = PrivateKey::random(&mut rng);
        let bytes = sk.encode();
        let hex_bare = hex::encode(&bytes);
        let dir = std::env::temp_dir();

        let path = dir.join(format!("p2p_test_bare_{}.key", std::process::id()));
        std::fs::write(&path, &hex_bare).unwrap();
        chmod_600(&path);
        let loaded = read_ed25519_key_from_file(&path).unwrap();
        assert_eq!(loaded.encode(), sk.encode());

        let path2 = dir.join(format!("p2p_test_prefixed_{}.key", std::process::id()));
        std::fs::write(&path2, format!("0x{hex_bare}\n")).unwrap();
        chmod_600(&path2);
        let loaded2 = read_ed25519_key_from_file(&path2).unwrap();
        assert_eq!(loaded2.encode(), sk.encode());

        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(path2);
    }

    #[test]
    fn read_ed25519_key_from_file_rejects_invalid_hex() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("p2p_test_invhex_{}.key", std::process::id()));
        std::fs::write(&path, "zzznothex").unwrap();
        assert!(read_ed25519_key_from_file(&path).is_err());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn read_ed25519_key_from_file_rejects_wrong_length() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("p2p_test_short_{}.key", std::process::id()));
        std::fs::write(&path, "deadbeef").unwrap();
        assert!(read_ed25519_key_from_file(&path).is_err());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn read_ed25519_key_from_file_missing_path_is_io_error() {
        assert!(read_ed25519_key_from_file("/this/path/does/not/exist/key").is_err());
    }

    #[test]
    fn build_returns_valid_handles_and_raw_channels_are_exposed() {
        let executor = deterministic::Runner::default();
        executor.start(|ctx| async move {
            let cfg = make_config(0, "127.0.0.1:9000".parse().unwrap());
            let (p2p, handles) = FluentP2P::build(ctx, cfg);

            // Oracle handle is clonable (shares UnboundedMailbox).
            let _oracle_clone = handles.oracle.clone();

            // PeerSetSink impl forwards to Manager::track, and the window every
            // ingress check reads is written on the way through. Probed through
            // `classify`, the only thing that reads it: before the first `track`
            // it must say "no opinion" (`None`), and after one it must have one —
            // here `Dropped`, an empty set holding nobody.
            let mut sink = handles.oracle.clone();
            let stranger = {
                use commonware_cryptography::Signer as _;
                use commonware_math::algebra::Random as _;
                let mut rng = rand_08::rngs::StdRng::seed_from_u64(77);
                PrivateKey::random(&mut rng).public_key()
            };
            assert_eq!(
                sink.window().classify(&stranger),
                None,
                "before the first track the window must have no opinion at all"
            );
            <OracleHandle as PeerSetSink>::track(&mut sink, 7, TrackedPeersOf::default()).await;
            assert_eq!(
                sink.window().classify(&stranger),
                Some(super::Ingress::Dropped),
                "the window must keep what was registered — a clone reads the same storage"
            );

            // All 9 channels are exposed as raw (sender, receiver) — bound by move
            // to prove they are owned and usable (BEACON + BEACON_RESOLVER via `..`).
            let FluentP2PHandles {
                vote_sender: _vs,
                vote_receiver: _vr,
                cert_sender: _cs,
                cert_receiver: _cr,
                resolver_sender: _rs,
                resolver_receiver: _rr,
                broadcast_sender: _br_s,
                broadcast_receiver: _br_r,
                marshal_sender: _mr_s,
                marshal_receiver: _mr_r,
                frontier_sender: _fr_s,
                frontier_receiver: _fr_r,
                evidence_sender: _ev_s,
                evidence_receiver: _ev_r,
                ..
            } = handles;

            // Start the network (consumes p2p); returns a Handle we drop.
            let _network_handle = p2p.start();
        });
    }
}
