//! Fluent DPoS p2p layer: commonware_p2p::authenticated::discovery wiring.
//!
//! Injection-style library: every collaborator is a constructor
//! parameter, so this compiles and unit-tests today against the pinned
//! commonware (`monorepo @ v2026.4.0`); only the live instances (the
//! node-assembly thread + `tokio::Runner`, Reth-handle injection, the
//! epoch_manager mux consumption) are deferred.
//!
//! Public API:
//! - [`FluentP2P::build`] — sync constructor (Network::new + 5× register); returns the Network owner + [`FluentP2PHandles`] (clone-into-04/06).
//! - [`FluentP2P::start`] — consumes self, calls `Network::start`.
//! - [`FluentP2PHandles`] — Fluent-domain handles (no commonware-p2p types leaked into 04's / 06's public API beyond the `OracleHandle` newtype and the `DiscSender` / `DiscReceiver` type aliases, which are intentional re-exports).

pub mod bootstrappers;
pub mod config;
pub mod constants;

use commonware_cryptography::ed25519;
use commonware_p2p::{
    authenticated::discovery::{Network, Oracle, Receiver, Sender},
    Blocker, Manager, PeerSetSubscription, Provider, TrackedPeers,
};
use commonware_runtime::{
    BufferPooler, Clock, Handle, Metrics, Network as RNetwork, Resolver, Spawner, Storage,
};
use commonware_utils::ordered::Set;
use fluentbase_bls::PeerPubkey;
use fluentbase_staking_reader::PeerSetSink;
use rand_core::CryptoRngCore;

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
    // loader, `bls/src/keys.rs`; audit P2-9). `PrivateKey::decode` copies into a
    // zeroizing `Secret`, so these source buffers are the only residue.
    let raw = zeroize::Zeroizing::new(
        std::fs::read_to_string(path_ref)
            .map_err(|e| eyre::eyre!("failed reading peer key file: {e}"))?,
    );
    let bytes = zeroize::Zeroizing::new(
        commonware_utils::from_hex_formatted(raw.trim())
            .ok_or_else(|| eyre::eyre!("peer key file contents not valid hex"))?,
    );
    ed25519::PrivateKey::decode(bytes.as_slice())
        .map_err(|e| eyre::eyre!("failed decoding peer key: {e:?}"))
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
/// the raw 5-channel `(Sender, Receiver)` pairs and does NOT pre-mux
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
}

impl<E> FluentP2P<E>
where
    E: BufferPooler + Clock + CryptoRngCore + Spawner + Storage + Metrics + RNetwork + Resolver,
{
    /// Build the p2p layer: instantiate commonware Network, register
    /// 5 top-level channels. Per-epoch demux for vote/cert/resolver is
    /// handled inside the consensus `EpochManager` (it builds its own
    /// `Muxer`s), so this layer returns raw channels here.
    pub fn build(ctx: E, cfg: FluentP2PConfig) -> (Self, FluentP2PHandles<E>) {
        let commonware_cfg = cfg.to_commonware_config();
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

        // Register 2 global one-instance channels:
        //    BROADCAST (block-data via buffered::Engine) +
        //    MARSHAL (backfill via marshal::resolver::p2p::init).
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

        let handles = FluentP2PHandles {
            oracle: OracleHandle { inner: oracle },
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

/// Adapter: `EpochTransition` calls `track(epoch, Set)`, which we
/// forward verbatim to `commonware_p2p::Manager::track` on the Oracle.
/// Commonware sorts the Set internally (`Set::from_iter_dedup`) — no
/// caller-side `.sort()`; the canonical byte-lex order is pinned by
/// `crates/bls/tests/ed25519_ordering_conformance.rs`.
impl PeerSetSink for OracleHandle {
    // `async fn` here matches the trait's `-> impl Future + Send`
    // (Rust auto-promotes the future to `Send` when all captures are
    // Send — `&mut self.inner` is Send via `Oracle: Send`). Manager::track
    // is also `async fn` but body is `send_lossy` (no real await pressure).
    async fn track(&mut self, epoch: u64, peers: Set<PeerPubkey>) {
        Manager::track(&mut self.inner, epoch, peers).await
    }
}

// Blocker + Provider delegating impls.
//
// `OuterBuilder` takes `B: Blocker<PublicKey = ed25519::PublicKey>` and
// `P: Provider<PublicKey = ed25519::PublicKey>`. The inner `Oracle<C>`
// satisfies both upstream; this newtype delegates verbatim so the node
// can pass `handles.oracle.clone()` directly to the Builder.

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
    use commonware_utils::ordered::Set;
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

            // PeerSetSink impl forwards to Manager::track.
            let mut sink = handles.oracle.clone();
            <OracleHandle as PeerSetSink>::track(&mut sink, 7, Set::default()).await;

            // All 5 channels are exposed as raw (sender, receiver). Bind by
            // move to prove they are owned and usable.
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
                ..
            } = handles;

            // Start the network (consumes p2p); returns a Handle we drop.
            let _network_handle = p2p.start();
        });
    }
}
