//! Fluent DPoS p2p layer: commonware_p2p::authenticated::discovery wiring.
//!
//! Every collaborator is a constructor parameter, so the layer unit-tests
//! against the pinned commonware without node assembly.

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

/// Load a commonware Ed25519 `PrivateKey` from a hex-encoded text file
/// (`0x`-prefixed or bare, surrounding whitespace trimmed).
///
/// On Unix, rejects files with group/other permissions set (`mode & 0o077 != 0`)
/// so a world-readable peer key cannot load silently.
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
    // Zeroizing buffers: the ed25519 signing scalar must not linger in freed
    // heap. `from_hex` is called directly because `from_hex_formatted` does an
    // internal `hex.replace(...)` that would leave a non-zeroized copy of the key
    // hex behind; `PrivateKey::decode` copies into a zeroizing `Secret`.
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

/// Fresh OS-randomness ed25519 keypair for an ephemeral peer identity: a
/// standalone `--cert-follow` follower builds one gossip-idle broadcast `Muxer`
/// and never persists or advertises the key. Validators load a durable key via
/// [`read_ed25519_key_from_file`] instead.
pub fn generate_ephemeral_ed25519_key() -> ed25519::PrivateKey {
    use commonware_math::algebra::Random as _;
    ed25519::PrivateKey::random(&mut rand_08::rngs::OsRng)
}

/// Newtype around the concrete discovery `Oracle`, required by the orphan rule:
/// both `PeerSetSink` and `Oracle` are foreign to this crate.
#[derive(Clone, Debug)]
pub struct OracleHandle {
    inner: Oracle<ed25519::PublicKey>,
    window: TrackedWindow,
}

/// Concrete commonware-p2p sender/receiver structs, not the top-level traits of
/// the same name (the structs implement the traits).
pub type DiscSender<E> = Sender<ed25519::PublicKey, E>;
pub type DiscReceiver = Receiver<ed25519::PublicKey>;

pub struct FluentP2P<E>
where
    E: BufferPooler + Clock + CryptoRngCore + Spawner + Storage + Metrics + RNetwork + Resolver,
{
    network: Network<E, ed25519::PrivateKey>,
}

/// Handles injected into the consensus and node layers.
///
/// Not `Clone`: each `(Sender, Receiver)` pair wraps an mpsc receiver and is
/// single-consumer by design. Only [`OracleHandle`] is cloneable, which shares
/// the `Oracle` across consumers.
pub struct FluentP2PHandles<E>
where
    E: BufferPooler + Clock + CryptoRngCore + Spawner + Storage + Metrics + RNetwork + Resolver,
{
    pub oracle: OracleHandle,

    // Vote/cert/resolver: the consensus `EpochManager` builds its own per-epoch
    // `Muxer`s over these raw channels, so pre-muxing here would double-demux.
    pub vote_sender: DiscSender<E>,
    pub vote_receiver: DiscReceiver,
    pub cert_sender: DiscSender<E>,
    pub cert_receiver: DiscReceiver,
    pub resolver_sender: DiscSender<E>,
    pub resolver_receiver: DiscReceiver,

    // Broadcast and marshal: process-global channels, one instance each rather
    // than per-epoch.
    pub broadcast_sender: DiscSender<E>,
    pub broadcast_receiver: DiscReceiver,
    pub marshal_sender: DiscSender<E>,
    pub marshal_receiver: DiscReceiver,

    // Beacon: the randomness-beacon DKG ceremony channel; the recovered seed
    // rides in the consensus cert, not here.
    pub beacon_sender: DiscSender<E>,
    pub beacon_receiver: DiscReceiver,

    // Beacon-resolver: the DKG-log recovery resolver (`commonware_resolver::p2p`).
    pub beacon_resolver_sender: DiscSender<E>,
    pub beacon_resolver_receiver: DiscReceiver,

    // Frontier: the plane-native `CertUpstream` by-height resolver
    // (`commonware_resolver::p2p`).
    pub frontier_sender: DiscSender<E>,
    pub frontier_receiver: DiscReceiver,

    // Evidence: the votes a node republishes for a decided round, so the two
    // halves of a split-delivered equivocation can meet.
    pub evidence_sender: DiscSender<E>,
    pub evidence_receiver: DiscReceiver,
}

impl<E> FluentP2P<E>
where
    E: BufferPooler + Clock + CryptoRngCore + Spawner + Storage + Metrics + RNetwork + Resolver,
{
    /// Build the p2p layer and register its channels.
    pub fn build(ctx: E, cfg: FluentP2PConfig) -> (Self, FluentP2PHandles<E>) {
        let commonware_cfg = cfg.into_commonware_config();
        let (mut network, oracle) = Network::new(ctx.with_label("p2p_network"), commonware_cfg);

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

    /// Returns the [`Handle`] for shutdown coordination. After this point no
    /// more `Network::register` is allowed.
    pub fn start(self) -> Handle<()> {
        self.network.start()
    }
}

/// Callers need no `.sort()`: commonware dedups and orders the set internally.
impl PeerSetSink for OracleHandle {
    async fn track(&mut self, epoch: u64, peers: TrackedPeersOf) {
        // Record before registering: a frame from a member of the new set can
        // arrive the instant commonware applies it, and every ingress check reads
        // the window.
        self.window.record(epoch, &peers);
        let registered = TrackedPeers::new(peers.primary(), peers.secondary);
        Manager::track(&mut self.inner, epoch, registered).await
    }
}

/// "Is this peer tombstoned on chain" — a closure rather than the `TombstoneSet`
/// type, which lives in the consensus crate above this one in the dependency
/// graph.
pub type TombstonePredicate = Arc<dyn Fn(&PeerPubkey) -> bool + Send + Sync>;

/// The last peer set this node registered, readable by every channel's ingress
/// check. Cloneable and shared: one writer (the `PeerSetSink` adapter above),
/// many readers.
///
/// Not `Provider::peer_set`: commonware keeps only the flat union
/// (`latest.primary`), which cannot answer the per-epoch question "which of
/// `C[E−1]`, `C[E]`, `C[E+1]` is this sender in", and it costs a mailbox
/// round-trip per call.
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
    /// One call per process, at the wiring site that owns the `TombstoneSet`.
    pub fn with_tombstones(mut self, tombstoned: TombstonePredicate) -> Self {
        self.tombstoned = Some(tombstoned);
        self
    }

    pub fn record(&self, epoch: u64, peers: &TrackedPeersOf) {
        if let Ok(mut slot) = self.latest.write() {
            *slot = Some((epoch, peers.clone()));
        }
    }

    /// `None` means "no peer set registered yet", not "drop": before the first
    /// `track` this node has no membership opinion, and turning that into a drop
    /// would silence the plane for the whole of cold start.
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
/// No penalty rides on this beyond the channel's own drop metric; the only global
/// ban is the on-chain tombstone.
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
/// array and allocates nothing on a per-frame path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EpochMask {
    slots: [u64; 3],
    len: u8,
}

impl EpochMask {
    /// The invariant is `TrackedPeersOf` holding at most three records with
    /// distinct epochs, guaranteed by construction; a fourth means the window
    /// grew and this array has to grow with it, since dropping the overflow would
    /// answer "not a member" for an epoch the peer is in.
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

impl OracleHandle {
    pub fn window(&self) -> TrackedWindow {
        self.window.clone()
    }
}

// `OuterBuilder` requires both `Blocker` and `Provider`; this newtype delegates
// both to the inner `Oracle` so `handles.oracle.clone()` drops straight in.
impl Blocker for OracleHandle {
    type PublicKey = ed25519::PublicKey;

    async fn block(&mut self, peer: Self::PublicKey) {
        // Every block is an audit-trail event so a misbehaving caller can be
        // traced; the 4h duration is commonware's `Config::block_duration`
        // default, which Fluent leaves alone.
        tracing::warn!(target: "fluentbase_p2p::blocker", ?peer, "peer block requested");
        Blocker::block(&mut self.inner, peer).await
    }
}

/// A [`Blocker`] that does nothing, wired wherever a block verdict is not
/// evidence-backed.
///
/// A block severs the peer from every channel for four hours (commonware's
/// `Config::block_duration` default). The simplex batcher blocks every
/// batch-verify-failed signer with an evidence-free `block!` and no threshold, and
/// the macro discards its reason before calling `Blocker::block`, so nothing here
/// could be reason-selective; slashing is unaffected because equivocation evidence
/// rides `reporter.report(Activity::Conflicting*)` independently of the block.
///
/// The resolver sites reach the same hook through a `deliver=false` verdict on an
/// undecodable or wrong-dealer response, and their excluded-set, inbound-quota and
/// retry defences do not depend on it.
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

            let _oracle_clone = handles.oracle.clone();

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

            let _network_handle = p2p.start();
        });
    }
}
