//! DPoS host adapter: spawns the dedicated OS thread + commonware-tokio
//! runtime, loads operator keys and configs from disk, builds the
//! `RethHandle` + `DposLayerConfig`, calls
//! [`fluentbase_consensus::dpos::DposLayer::launch`], then runs the
//! shutdown supervisor `select!`.

use crate::consensus_rpc::{feed_actor::FeedActor, FeedStateHandle};
// `Clock` (ctx.current/sleep in the finalized-height poller) + `Metrics`
// (ctx.with_label, ctx.encode) + `Spawner` (ctx.spawn) — used by the always-on
// beacon plane, the cert-feed actor, and the feature-gated devnet metrics endpoint.
use commonware_consensus::types::Height;
use commonware_p2p::{
    utils::mux::{Builder, Muxer},
    Ingress,
};
use commonware_runtime::{tokio::Context, Clock as _, Handle, Metrics as _, Spawner as _};
use eyre::{eyre, OptionExt as _, WrapErr as _};
use fluentbase_consensus::dpos::{
    DposLayer, DposLayerConfig, DposLayerHandle, ResettableForward, RethHandle, SharedBeaconPlane,
    VoteBackupItem,
};
pub use fluentbase_consensus::FeedSink;
use fluentbase_p2p::{FluentP2P, FluentP2PConfig};
use fluentbase_staking_reader::{
    reader::RethStakingStateReader, EpochTransition, ValidatorSetCache,
};
use reth_chain_state::CanonicalInMemoryState;
use reth_chainspec::EthChainSpec as _;
use reth_ethereum_engine_primitives::EthEngineTypes;
use reth_ethereum_primitives::{Block as RethBlock, EthPrimitives};
use reth_node_api::{FullNodeComponents, FullNodeTypes};
use reth_node_builder::{rpc::RethRpcAddOns, FullNode, PayloadBuilderConfig};
use reth_provider::providers::{BlockchainProvider, ProviderNodeTypes};
use reth_storage_api::{
    BlockHashReader, BlockIdReader, BlockNumReader, BlockReader, HeaderProvider,
    StateProviderFactory,
};
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

/// Bridge trait exposing reth's `canonical_in_memory_state` snapshot to
/// the generic host adapter. No reth trait exposes this method —
/// `canonical_in_memory_state` is an inherent method on the concrete
/// `BlockchainProvider<N>`. Implemented below for that type so the
/// host adapter's `where` clause can require it.
pub trait CanonicalStateAccess: Send + Sync {
    fn canonical_state(&self) -> CanonicalInMemoryState<EthPrimitives>;
}

impl<N> CanonicalStateAccess for BlockchainProvider<N>
where
    N: ProviderNodeTypes<Primitives = EthPrimitives>,
{
    fn canonical_state(&self) -> CanonicalInMemoryState<EthPrimitives> {
        self.canonical_in_memory_state()
    }
}

use crate::ordering::{PoolAssembler, ProviderExecutedChain};

/// Operator-supplied DPoS configuration (parsed from CLI/env in `main.rs`).
///
/// Not `Clone`/`Debug`: it carries the move-only cert-feed `Receiver` (single
/// consumer) and is built once then moved into the validator overlay of
/// `spawn_node_stack`. The validator-only payload of [`NodeStackCfg`].
pub struct DposConfig {
    /// DEV/TEST-ONLY plaintext hex BLS private key file. `load_bls_keypair`
    /// rejects it on deployed networks (devnet/testnet/mainnet chain_ids);
    /// production must use `bls_keystore_path`. Mutually exclusive with it.
    pub bls_key_path: Option<PathBuf>,
    /// EIP-2335 keystore JSON for the validator BLS key. Mutually
    /// exclusive with the dev/test-only `bls_key_path`. Password is read
    /// from `bls_keystore_password_file` (mode-checked).
    pub bls_keystore_path: Option<PathBuf>,
    /// Password file for the BLS keystore — file mode must satisfy
    /// `mode & 0o077 == 0`.
    pub bls_keystore_password_file: Option<PathBuf>,
    pub peer_key_path: PathBuf,
    pub staking_config_path: PathBuf,
    /// JSON file with `[{peer_pubkey, socket}, ...]` for cold-start peer
    /// discovery.
    pub bootstrappers_path: PathBuf,
    pub p2p_port: u16,
    pub dialable: Option<SocketAddr>,
    /// EIP-2335 / Web3 Secret Storage v3 keystore JSON for the slasher EOA.
    pub slasher_keystore_path: Option<PathBuf>,
    pub slasher_keystore_password_file: Option<PathBuf>,
    /// DEVNET-ONLY: serve commonware consensus metrics (prometheus text) on this
    /// host port for the smoke regression suite. `None` = disabled (prod default).
    pub metrics_port: Option<u16>,
    /// Cert-feed wiring for the `consensus` RPC namespace. `None` = node does not
    /// serve the cert feed (e.g. unit tests). Set on every production node.
    pub cert_feed: Option<CertFeed>,
    /// `--dpos.follower-upstream` WS URLs. Non-empty arms the cert-inlet as a
    /// SECOND producer into this validator's own marshal: while in-committee its
    /// locally-formed certs lead, and once rotated out `reconcile_roles` keeps it
    /// a Verifier following the inlet-fed base — no restarts. Empty = plain
    /// `--dpos` (signer-or-silent-verifier, no inlet).
    pub follower_upstreams: Vec<String>,
    /// DEVNET/TEST-ONLY byzantine mode string (`--dpos.byzantine`). Gated behind
    /// `dpos-devnet-byzantine`; the field does not exist in a production build.
    #[cfg(feature = "dpos-devnet-byzantine")]
    pub byzantine_mode: Option<String>,
}

/// The node-side cert-feed wiring threaded from `main.rs`: the `FeedSink` goes
/// down into the marshal (2nd `Reporter`), while the receiver + state handle drive
/// the node-side [`FeedActor`] + `consensus` RPC.
pub struct CertFeed {
    pub sink: FeedSink,
    pub rx: mpsc::UnboundedReceiver<Height>,
    pub handle: FeedStateHandle,
}

/// DPoS validator CLI flags (`--dpos.*`). Flattened into the node binary's args
/// via `#[command(flatten)]`; the cross-field clap rules below
/// (`required_if_eq("dpos", "true")`, `conflicts_with`, `requires`) resolve
/// against the merged command, so they keep working even though `--dpos` itself
/// lives in the parent args struct.
#[derive(Debug, Clone, Default, clap::Args)]
#[non_exhaustive]
pub struct DposArgs {
    /// Plaintext hex BLS private key file. DEV/TEST-ONLY — rejected at
    /// startup on deployed networks (devnet/testnet/mainnet). Production
    /// MUST use `--dpos.bls-keystore-path` (import an externally-generated
    /// EIP-2335 keystore). Mutually exclusive with the keystore flag.
    #[arg(
        long = "dpos.bls-key-path",
        env = "FLUENT_DPOS_BLS_KEY_PATH",
        conflicts_with = "dpos_bls_keystore_path",
        group = "bls"
    )]
    pub dpos_bls_key_path: Option<PathBuf>,

    /// EIP-2335 keystore JSON for the validator BLS key. Preferred over
    /// the deprecated `--dpos.bls-key-path`.
    #[arg(
        long = "dpos.bls-keystore-path",
        env = "FLUENT_DPOS_BLS_KEYSTORE_PATH",
        conflicts_with = "dpos_bls_key_path",
        requires = "dpos_bls_keystore_password_file",
        group = "bls"
    )]
    pub dpos_bls_keystore_path: Option<PathBuf>,

    /// Password file for `--dpos.bls-keystore-path`. Mode must be
    /// `0o600` (or stricter); fail-stops on world/group readable bits.
    #[arg(
        long = "dpos.bls-keystore-password-file",
        env = "FLUENT_DPOS_BLS_KEYSTORE_PASSWORD_FILE"
    )]
    pub dpos_bls_keystore_password_file: Option<PathBuf>,

    /// Path to Ed25519 peer signing key file (hex-encoded).
    #[arg(
        long = "dpos.peer-key-path",
        env = "FLUENT_DPOS_PEER_KEY_PATH",
        required_if_eq("dpos", "true")
    )]
    pub dpos_peer_key_path: Option<PathBuf>,

    /// Path to staking-reader JSON config (staking + chain-config addresses).
    #[arg(
        long = "dpos.staking-config",
        env = "FLUENT_DPOS_STAKING_CONFIG",
        required_if_eq("dpos", "true")
    )]
    pub dpos_staking_config: Option<PathBuf>,

    /// JSON file with `[{peer_pubkey, socket}, ...]` for cold-start peer
    /// discovery. Required when `--dpos` is set — no in-tree per-chain
    /// defaults. For genesis bootstrap event pass an empty `[]` JSON
    /// (explicit operator intent for the first bootnode in a new network).
    #[arg(
        long = "dpos.bootstrappers",
        env = "FLUENT_DPOS_BOOTSTRAPPERS",
        required_if_eq("dpos", "true")
    )]
    pub dpos_bootstrappers: Option<PathBuf>,

    /// Listen port for commonware p2p (default 9000).
    #[arg(
        long = "dpos.p2p-port",
        env = fluentbase_p2p::constants::LISTEN_PORT_ENV_VAR,
        default_value_t = fluentbase_p2p::constants::DEFAULT_LISTEN_PORT,
    )]
    pub dpos_p2p_port: u16,

    /// Override dialable address (what we tell peers); default = listen.
    #[arg(long = "dpos.dialable", env = "FLUENT_DPOS_DIALABLE")]
    pub dpos_dialable: Option<SocketAddr>,

    /// DEVNET-ONLY: serve commonware consensus metrics (prometheus text) on this
    /// host port for the smoke regression suite. Unset = disabled (prod default).
    #[arg(long = "dpos.metrics-port", env = "FLUENT_DPOS_METRICS_PORT")]
    pub dpos_metrics_port: Option<u16>,

    /// Upstream `consensus` WS URL(s) for the validator-side cert-inlet
    /// (repeatable — failover list). Presence arms the inlet as a second producer
    /// into this node's marshal: the node follows the inlet-fed base while its key
    /// is outside the committee and re-promotes in place when it rejoins (via
    /// `reconcile_roles`) — no restarts. Absent = plain `--dpos` (no inlet).
    #[arg(
        long = "dpos.follower-upstream",
        env = "FLUENT_DPOS_FOLLOWER_UPSTREAM",
        action = clap::ArgAction::Append
    )]
    pub dpos_follower_upstream: Vec<String>,

    /// EIP-2335 / Web3 Secret Storage v3 keystore JSON for the slasher EOA.
    #[arg(
        long = "dpos.slasher-keystore-path",
        env = "FLUENT_DPOS_SLASHER_KEYSTORE_PATH",
        requires = "dpos_slasher_keystore_password_file",
        required_if_eq("dpos", "true")
    )]
    pub dpos_slasher_keystore_path: Option<PathBuf>,

    /// Password file for `--dpos.slasher-keystore-path`. Mode must be
    /// `0o600` (or stricter); fail-stops on world/group readable bits.
    #[arg(
        long = "dpos.slasher-keystore-password-file",
        env = "FLUENT_DPOS_SLASHER_KEYSTORE_PASSWORD_FILE"
    )]
    pub dpos_slasher_keystore_password_file: Option<PathBuf>,

    /// DEVNET/TEST-ONLY byzantine validator mode (e.g. `forge-beacon-pk`). Compiled
    /// in ONLY with the `dpos-devnet-byzantine` cargo feature; the flag does not
    /// exist in a production build. Used by the byzantine-vrf smoke to prove the
    /// beacon's safety against a forged `PK_E`.
    #[cfg(feature = "dpos-devnet-byzantine")]
    #[arg(long = "dpos.byzantine", env = "FLUENT_DPOS_BYZANTINE")]
    pub dpos_byzantine: Option<String>,
}

impl DposConfig {
    /// Build from parsed [`DposArgs`] plus the runtime-wired `extra_data_registry`
    /// and `cert_feed`. Only reached when `--dpos` is set, so the
    /// `required_if_eq("dpos", "true")` clap rules guarantee `peer_key_path` /
    /// `staking_config_path` / `bootstrappers_path` are `Some`.
    pub fn from_args(args: &DposArgs, cert_feed: Option<CertFeed>) -> Self {
        Self {
            bls_key_path: args.dpos_bls_key_path.clone(),
            bls_keystore_path: args.dpos_bls_keystore_path.clone(),
            bls_keystore_password_file: args.dpos_bls_keystore_password_file.clone(),
            peer_key_path: args
                .dpos_peer_key_path
                .clone()
                .expect("required_if_eq guarantees --dpos.peer-key-path"),
            staking_config_path: args
                .dpos_staking_config
                .clone()
                .expect("required_if_eq guarantees --dpos.staking-config"),
            bootstrappers_path: args
                .dpos_bootstrappers
                .clone()
                .expect("required_if_eq guarantees --dpos.bootstrappers"),
            p2p_port: args.dpos_p2p_port,
            dialable: args.dpos_dialable,
            slasher_keystore_path: args.dpos_slasher_keystore_path.clone(),
            slasher_keystore_password_file: args.dpos_slasher_keystore_password_file.clone(),
            metrics_port: args.dpos_metrics_port,
            cert_feed,
            follower_upstreams: args.dpos_follower_upstream.clone(),
            #[cfg(feature = "dpos-devnet-byzantine")]
            byzantine_mode: args.dpos_byzantine.clone(),
        }
    }
}

pub type DposSpawn<N, AddOns> = crate::utils::ConsensusSpawn<N, AddOns>;

/// A WS cert-inlet upstream: a SECOND producer into this node's marshal. The
/// driver field of [`NodeStackCfg`]: present for a `--cert-follow` follower
/// (its sole producer) and for an upstream-configured `--dpos` validator (a
/// rotated-out validator follows the inlet-fed base). Absent = a plain
/// signer-or-silent validator (no inlet). `verify` is always `true` on every
/// mapped inlet in v1 (the standalone `--sequencer-url` trust relay is the
/// separate `launch_consensus_node` path, NOT an inlet).
pub struct CertInletCfg {
    pub urls: Vec<String>,
}

/// The ONE node-stack config (process-mode unification, Phase 5). A node is a
/// single process body ([`run_node_stack`]) configured by two drivers —
/// `is_validator` (has signer keys + the full beacon plane) and `cert_inlet`
/// (an optional WS upstream producer) — plus the per-overlay payload behind one
/// of `validator`/`follower`. `node_modes.rs` is the pure CLI-flag → this-config
/// mapping; the spine branches on `is_validator` only at the plane build + the
/// engine-start variant (the two `DposLayer::launch*` overlay shapes).
pub struct NodeStackCfg {
    /// `--dpos`: this node holds signer keys, runs the full 5-Muxer beacon plane
    /// and the local-BFT engine. `false` = a near-planeless cert-follower.
    pub is_validator: bool,
    /// Optional WS cert-inlet upstream (deep catch-up jump + live self-heal).
    /// `Some` for `--cert-follow` (always) and for a `--dpos` validator with
    /// `--dpos.follower-upstream`; `None` for a plain validator. The inlet always
    /// BLS-verifies (no no-verify mode in v1).
    pub cert_inlet: Option<CertInletCfg>,
    /// Validator overlay payload (BLS/peer/slasher keys, bootstrappers, ports,
    /// cert-feed). `Some` iff `is_validator`.
    pub validator: Option<DposConfig>,
    /// Follower overlay payload (L1 checkpoint, cert-feed, staking config).
    /// `Some` iff `!is_validator`.
    pub follower: Option<FollowerCfg>,
}

/// Follower-overlay payload (the non-validator `--cert-follow` bits). The shared
/// `cert_inlet.urls` carries the upstream WS list; this carries everything else
/// the near-planeless follower needs.
pub struct FollowerCfg {
    /// `consensus` RPC state handle (serving side, D4). `Some` ⇒ verified pairs
    /// feed a bounded window behind the same WS namespace validators serve.
    pub feed: Option<FeedStateHandle>,
    /// Staking system-contract config: per-epoch committee reads for the inlet.
    pub staking_config_path: PathBuf,
    /// L1 Rollup checkpoint source (D2). `None` = devnet fallback.
    pub l1: Option<crate::cert_follow::l1::L1CheckpointConfig>,
}

/// Spawn the unified node-stack thread. The thread blocks on `handle_tx` until
/// the reth `FullNode` is delivered, then constructs the commonware tokio
/// runtime and calls [`run_node_stack`]. ONE thread-spawn + ONE body for both
/// the `--dpos` validator and the `--cert-follow` follower modes; the executor
/// is the sole reth writer in every mode.
pub fn spawn_node_stack<N, AddOns>(
    cfg: NodeStackCfg,
    shutdown_token: CancellationToken,
) -> DposSpawn<N, AddOns>
where
    N: FullNodeComponents<
            Types: reth_node_api::NodeTypes<
                Payload = EthEngineTypes,
                Primitives = reth_ethereum_primitives::EthPrimitives,
            >,
        > + 'static,
    AddOns: RethRpcAddOns<N> + 'static,
    <N as FullNodeTypes>::Provider: Clone
        + BlockReader<Block = RethBlock>
        + BlockHashReader
        + BlockNumReader
        + BlockIdReader
        + StateProviderFactory
        + HeaderProvider<Header = alloy_consensus::Header>
        + CanonicalStateAccess
        + Send
        + Sync
        + 'static,
    <N as FullNodeComponents>::Evm: reth_evm::ConfigureEvm<
            Primitives = EthPrimitives,
            NextBlockEnvCtx = reth_evm::NextBlockEnvAttributes,
        > + Clone
        + Send
        + Sync
        + 'static,
{
    crate::utils::spawn_consensus_thread("node", move |ctx, node| {
        run_node_stack(ctx, node, cfg, shutdown_token)
    })
}

/// The ONE node-stack body — runs entirely on the commonware tokio runtime. A
/// single spine (devnet metrics → reth importer → WS upstream init → overlay
/// build → uniform supervisor) with exactly ONE branch point: the overlay
/// (`is_validator` selects the full beacon plane + `DposLayer::launch` vs the
/// near-planeless broadcast Muxer + `DposLayer::launch_follower`). Each overlay
/// returns the same `(DposLayerHandle, supervised handles)` shape, so the
/// shutdown supervisor `select!` is one path.
async fn run_node_stack<N, AddOns>(
    ctx: Context,
    node: FullNode<N, AddOns>,
    cfg: NodeStackCfg,
    shutdown_token: CancellationToken,
) -> eyre::Result<()>
where
    N: FullNodeComponents<
        Types: reth_node_api::NodeTypes<
            Payload = EthEngineTypes,
            Primitives = reth_ethereum_primitives::EthPrimitives,
        >,
    >,
    AddOns: RethRpcAddOns<N>,
    <N as FullNodeTypes>::Provider: Clone
        + BlockReader<Block = RethBlock>
        + BlockHashReader
        + BlockNumReader
        + BlockIdReader
        + StateProviderFactory
        + HeaderProvider<Header = alloy_consensus::Header>
        + CanonicalStateAccess
        + Send
        + Sync
        + 'static,
    <N as FullNodeComponents>::Evm: reth_evm::ConfigureEvm<
            Primitives = EthPrimitives,
            NextBlockEnvCtx = reth_evm::NextBlockEnvAttributes,
        > + Clone
        + Send
        + Sync
        + 'static,
{
    let NodeStackCfg {
        is_validator,
        cert_inlet,
        validator,
        follower,
    } = cfg;

    // ── DIVERGENCE: the overlay. Validator = full 5-Muxer beacon plane +
    //    local-BFT engine (+ optional inlet as a 2nd producer); follower =
    //    near-planeless broadcast Muxer + inlet-only engine. Both return the
    //    same `(DposLayerHandle, supervised handles)` shape so the supervisor is
    //    a single path. ──
    let (mut engine, supervised): (DposLayerHandle, Vec<SupervisedHandle>) = if is_validator {
        let dpos_cfg = validator
            .ok_or_else(|| eyre!("run_node_stack: is_validator=true requires a validator config"))?;
        launch_validator_overlay(ctx, node, dpos_cfg, cert_inlet, shutdown_token.clone()).await?
    } else {
        let follow_cfg = follower
            .ok_or_else(|| eyre!("run_node_stack: is_validator=false requires a follower config"))?;
        let inlet = cert_inlet
            .ok_or_else(|| eyre!("run_node_stack: a follower requires a cert_inlet upstream"))?;
        crate::cert_follow::launch_follower_overlay(
            ctx,
            node,
            follow_cfg,
            inlet,
            shutdown_token.clone(),
        )
        .await?
    };

    // ── SHARED SPINE: the shutdown supervisor over the engine + every overlay
    //    handle, uniform for both modes. On any unexpected exit cancel the shared
    //    token (so reth/main bring everything down) then abort the survivors. ──
    let exit_reason = supervise(&shutdown_token, &mut engine.consensus_handle, supervised).await;
    info!(reason = exit_reason, "node thread exiting");
    Ok(())
}

/// One supervised overlay task: a label (for the exit log) + its abortable
/// handle. The validator overlay yields the plane net/dkg/poller/mux handles
/// (+ optional inlet); the follower overlay yields the WS / net / broadcast-mux
/// handles. Collected into one `Vec` so the supervisor `select!` is mode-blind.
pub(crate) type SupervisedHandle = (&'static str, Handle<()>);

/// The shared shutdown supervisor: race the shutdown token, the engine handle,
/// and every overlay handle. The FIRST resolution cancels the shared token (so
/// reth + `main` tear down too) and aborts the rest. Returns a reason string for
/// the exit log. Mode-blind — the only per-mode input is the `supervised` Vec.
async fn supervise(
    shutdown_token: &CancellationToken,
    engine: &mut Handle<()>,
    mut supervised: Vec<SupervisedHandle>,
) -> &'static str {
    use futures::future::{select_all, FutureExt as _};

    // Labels are `&'static str` (Copy) — snapshot them BEFORE the mutable borrow
    // of `supervised` for `select_all`, so the two borrows don't overlap.
    let labels: Vec<&'static str> = supervised.iter().map(|(l, _)| *l).collect();

    let reason = tokio::select! {
        _ = shutdown_token.cancelled() => {
            info!("node thread received shutdown signal, exiting");
            "shutdown_token"
        }
        res = &mut *engine => {
            match res {
                Ok(()) => warn!("OuterEngine exited cleanly (unexpected)"),
                Err(e) => error!(error = ?e, "OuterEngine task failed"),
            }
            shutdown_token.cancel();
            "consensus_exit"
        }
        // Any overlay handle resolving is a fatal exit (network down, total
        // upstream loss, mux failure). An empty `supervised` Vec can't happen
        // (every overlay yields at least the network handle), but guard anyway.
        (res, label) = async {
            let futs: Vec<_> = supervised.iter_mut().map(|(_, h)| h.boxed()).collect();
            if futs.is_empty() {
                std::future::pending::<()>().await;
                unreachable!()
            }
            let (res, idx, _rest) = select_all(futs).await;
            (res, labels[idx])
        } => {
            match res {
                Ok(()) => warn!(handle = label, "node overlay task exited cleanly (unexpected)"),
                Err(e) => error!(handle = label, error = ?e, "node overlay task failed"),
            }
            shutdown_token.cancel();
            label
        }
    };

    // Abort the engine + every overlay handle so the runtime releases its
    // resources (matches the per-mode bodies' explicit aborts before this merge).
    engine.abort();
    for (_, h) in &supervised {
        h.abort();
    }
    reason
}

/// Build the VALIDATOR overlay: the always-on 5-Muxer beacon plane + the
/// local-BFT engine (via `DposLayer::launch`), plus the optional cert-inlet as a
/// SECOND producer into the same marshal. Returns the engine handle and the
/// plane/inlet handles for the shared supervisor. Extracted from the former
/// `run_dpos_stack` so the node body is one spine with one branch point.
async fn launch_validator_overlay<N, AddOns>(
    ctx: Context,
    node: FullNode<N, AddOns>,
    mut cfg: DposConfig,
    cert_inlet: Option<CertInletCfg>,
    shutdown_token: CancellationToken,
) -> eyre::Result<(DposLayerHandle, Vec<SupervisedHandle>)>
where
    N: FullNodeComponents<
        Types: reth_node_api::NodeTypes<
            Payload = EthEngineTypes,
            Primitives = reth_ethereum_primitives::EthPrimitives,
        >,
    >,
    AddOns: RethRpcAddOns<N>,
    <N as FullNodeTypes>::Provider: Clone
        + BlockReader<Block = RethBlock>
        + BlockHashReader
        + BlockNumReader
        + BlockIdReader
        + StateProviderFactory
        + HeaderProvider<Header = alloy_consensus::Header>
        + CanonicalStateAccess
        + Send
        + Sync
        + 'static,
    <N as FullNodeComponents>::Evm: reth_evm::ConfigureEvm<
            Primitives = EthPrimitives,
            NextBlockEnvCtx = reth_evm::NextBlockEnvAttributes,
        > + Clone
        + Send
        + Sync
        + 'static,
{
    spawn_devnet_metrics(&ctx, &cfg);

    let cert_feed = cfg.cert_feed.take();
    // Claim the single-execution import escrow ONCE per process.
    let beacon_engine =
        crate::importer::RethImporter::from_env(node.add_ons_handle.beacon_engine_handle.clone())?;

    // Load the validator BLS keypair UP FRONT (R2 reorder): the per-epoch
    // DKG-share at-rest seal key (E2) is HKDF-derived from it, and the beacon
    // plane's DkgActor + startup `load_all` both need it — but the plane is built
    // BEFORE the layer. `build_beacon_plane` only reads the peer key / staking /
    // bootstrappers (NOT this BLS key), so loading here changes failure ORDER
    // only (a bad keystore now fails before the network binds — strictly better).
    // The keypair itself flows DOWN into `launch_dpos_layer` (passed, not
    // re-loaded). `Some(seal_key)` ⇒ keystore mode ⇒ shares persist TAG_ENCRYPTED;
    // `None` ⇒ plaintext-dev ⇒ TAG_PLAINTEXT.
    let chain_id = node.chain_spec().chain_id();
    let (bls_keypair, share_seal_key) = load_bls_keypair(&cfg, chain_id)?;
    let share_state = match share_seal_key {
        Some(key) => fluentbase_consensus::beacon::share_state::ShareState::Encrypted(key),
        None => fluentbase_consensus::beacon::share_state::ShareState::Plaintext,
    };

    // Cert-inlet: a SECOND producer into this validator's own marshal, armed
    // whenever `--dpos.follower-upstream` URLs are configured (the production-path
    // fix for a rotated-out validator: it follows the inlet-fed base while
    // `reconcile_roles` keeps it a Verifier, then re-promotes in place when it
    // rejoins the committee). Constructed BEFORE `launch_dpos_layer` consumes
    // `ctx`/`cert_feed`. `None` when no upstream URLs are configured.
    let inlet_setup = cert_inlet
        .as_ref()
        .map(|inlet| {
            let chain_id = node.chain_spec().chain_id();
            let staking_config =
                fluentbase_staking_reader::reader::StakingReaderConfig::from_json_path(
                    &cfg.staking_config_path,
                )
                .wrap_err_with(|| {
                    format!(
                        "cert-inlet: failed loading staking config from {}",
                        cfg.staking_config_path.display()
                    )
                })?;
            // Committees are epoch-frozen and content-invariant across any
            // in-epoch executed hash, so the inlet reads committee[E] at the
            // node's CURRENT FINALIZED tip — a guaranteed-committed block where
            // committee[E] is committed (ahead-committed at epoch-(E-1)-start),
            // with a bounded retry while the executor (which the inlet feeds)
            // drains the queue that far.
            let finalized_hash: std::sync::Arc<
                dyn Fn() -> Option<alloy_primitives::B256> + Send + Sync,
            > = {
                let p = node.provider.clone();
                std::sync::Arc::new(move || {
                    let n = p.finalized_block_number().ok()??;
                    p.block_hash(n).ok().flatten()
                })
            };
            let committees = crate::cert_inlet::committee_source(
                node.provider.clone(),
                node.evm_config.clone(),
                staking_config,
                chain_id,
                finalized_hash,
            );
            eyre::Ok((committees, ctx.clone(), inlet.urls.clone()))
        })
        .transpose()?;

    // Always-on beacon plane (one FluentP2P + 5 persistent Muxers + persistent
    // DkgActor + shared store), built ONCE — the engine CLONES the shared plane.
    let plane = build_beacon_plane(&ctx, &node, &cfg, share_state).await?;
    let handle = launch_dpos_layer(
        ctx,
        &node,
        &cfg,
        bls_keypair,
        beacon_engine,
        cert_feed,
        plane.shared.clone(),
        shutdown_token,
    )
    .await?;

    // Spawn the cert-inlet against the SAME marshal the local engine drives
    // (`handle.cert_mailbox`). `None` when no upstreams are configured. The inlet
    // also tees the LIVE upstream cert frontier into the beacon plane's
    // `committee_for` read cursor + DkgActor deal clock (re-homed from the
    // deleted unified supervisor) — so a still-catching-up validator resolves
    // committee[E+1] and deals its DKG share at the live tip, not its lagging
    // EL-finalized state.
    let inlet_handle = inlet_setup.map(|(committees, inlet_ctx, urls)| {
        crate::cert_inlet::spawn_cert_inlet(
            inlet_ctx,
            handle.cert_mailbox.clone(),
            committees,
            urls,
            fluentbase_consensus::cert_inlet::LiveFrontierTee {
                live_height: plane.live_height.clone(),
                dkg_height_tx: plane.dkg_height_tx.clone(),
            },
        )
    });

    // Collect every overlay handle for the shared supervisor. Total upstream
    // loss / inner committee fatal resolves the inlet handle → the supervisor
    // cancels the shared token (fail-closed-on-total-loss, Risk-3).
    let mut supervised: Vec<SupervisedHandle> = vec![
        ("network", plane.net_handle),
        ("dkg", plane.dkg_handle),
        ("poller", plane.poller_handle),
    ];
    for h in plane.mux_handles {
        supervised.push(("mux", h));
    }
    if let Some(h) = inlet_handle {
        supervised.push(("inlet", h));
    }

    Ok((handle, supervised))
}

/// DEVNET-ONLY metrics endpoint (feature-gated so prod binaries can't serve
/// it). Spawned on a child of the commonware runtime context; children share
/// the runtime's prometheus registry, so `c.encode()` includes the p2p
/// tracker `connected`/`tracked` gauges the DposLayer registers later (the
/// smoke `case-peers.sh` scrapes them). Must bind exactly ONCE per process, so
/// it lives in the thread body, not inside [`launch_dpos_layer`].
pub(crate) fn spawn_devnet_metrics(ctx: &Context, cfg: &DposConfig) {
    #[cfg(feature = "dpos-devnet-metrics")]
    if let Some(port) = cfg.metrics_port {
        warn!(
            port,
            "DEVNET: serving commonware consensus metrics over HTTP (do not enable in prod)"
        );
        drop(ctx.with_label("metrics_http").spawn(move |c| async move {
            serve_metrics(c, port).await;
        }));
    }
    #[cfg(not(feature = "dpos-devnet-metrics"))]
    if cfg.metrics_port.is_some() {
        let _ = ctx;
        warn!(
            "--dpos.metrics-port set but this binary was built without the \
             `dpos-devnet-metrics` feature; metrics endpoint disabled"
        );
    }
}

/// The always-on beacon/DKG plane, built ONCE per process (before the
/// follower↔signer loop) and kept alive across every phase switch. It owns the
/// single `FluentP2P` (so there is exactly one network / `listen` bind / peer set),
/// the 5 persistent non-beacon channel `Muxer`s (the brokers the per-promotion
/// signer engine registers sub-channels against — so a demote→re-promote needs NO
/// network rebuild), the persistent `DkgActor` (committee[E] deals during E-1
/// regardless of this node's current consensus role), the shared `ceremony_store`
/// (reloaded from `<datadir>/beacon/` once), and an `EpochTransition`-driven Oracle
/// peer set fed by a finalized-height poller (the DKG clock that keeps ticking while
/// this node is a FOLLOWER). The signer engine CLONES the shared `Arc`s +
/// `MuxHandle`s per promotion; only its consensus engine is aborted on a phase
/// switch — the plane's network/broker/DkgActor handles survive until process
/// shutdown.
pub(crate) struct BeaconPlane {
    /// The single network's start handle — aborted ONLY at process shutdown.
    pub net_handle: Handle<()>,
    /// The persistent `DkgActor` task — aborted ONLY at process shutdown.
    pub dkg_handle: Handle<()>,
    /// The finalized-height poller driving the plane's ET + `dkg_height` clock.
    /// The poller owns its own `dkg_height_tx` clone (feeding the LOCAL
    /// ordering-finalized height `fin + K`); the live-cert-frontier tee is
    /// re-homed onto the cert-inlet (`live_height` + `dkg_height_tx` below),
    /// fed only on an upstream-configured node.
    pub poller_handle: Handle<()>,
    /// The 5 persistent non-beacon `Muxer` broker tasks (vote/cert/resolver/
    /// broadcast/marshal) + the vote-backup forwarder — aborted ONLY at process
    /// shutdown (they outlive every per-promotion signer engine).
    pub mux_handles: Vec<Handle<()>>,
    /// Shared store + committee resolver + Oracle + metrics + the 5 `MuxHandle`s +
    /// the vote-backup forwarder, CLONED into the signer engine per promotion (the
    /// engine never re-builds any of these or re-binds the network).
    pub shared: SharedBeaconPlane,
    /// The `committee_for` live-read cursor (`max(EL-finalized, live_height)`).
    /// Handed to an upstream-configured validator's cert-inlet so it tees the
    /// LIVE upstream cert frontier here — committee[E+1] then resolves at the
    /// live tip, not this node's lagging EL-finalized state (the production-path
    /// "Option A" fix). Stays `0` on a no-upstream validator (no inlet to feed
    /// it; its executor IS the tip).
    pub live_height: Arc<std::sync::atomic::AtomicU64>,
    /// The DkgActor deal clock. The inlet ALSO tees the live frontier here so a
    /// still-catching-up early-joiner deals its first epoch's DKG share at the
    /// live tip (the vrf-rotation early-join fix), not K blocks late. The
    /// finalized poller feeds it `fin + K`; the DkgActor `on_height` clamps both
    /// feeders to its running max (never rewound).
    pub dkg_height_tx: mpsc::Sender<u64>,
}

/// Build the always-on beacon plane for a registered `--dpos` validator. Mirrors
/// the network/EpochTransition/DkgActor construction that used to live inside
/// `DposLayer::launch`, lifted UP so it persists across the consensus role switch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn build_beacon_plane<N, AddOns>(
    ctx: &Context,
    node: &FullNode<N, AddOns>,
    cfg: &DposConfig,
    share_state: fluentbase_consensus::beacon::share_state::ShareState,
) -> eyre::Result<BeaconPlane>
where
    N: FullNodeComponents<
        Types: reth_node_api::NodeTypes<
            Payload = EthEngineTypes,
            Primitives = reth_ethereum_primitives::EthPrimitives,
        >,
    >,
    AddOns: RethRpcAddOns<N>,
    <N as FullNodeTypes>::Provider: Clone
        + BlockReader<Block = RethBlock>
        + BlockHashReader
        + BlockNumReader
        + BlockIdReader
        + StateProviderFactory
        + HeaderProvider<Header = alloy_consensus::Header>
        + CanonicalStateAccess
        + Send
        + Sync
        + 'static,
    <N as FullNodeComponents>::Evm:
        reth_evm::ConfigureEvm<Primitives = EthPrimitives> + Clone + Send + Sync + 'static,
{
    let chain_id = node.chain_spec().chain_id();
    let peer_keypair = fluentbase_p2p::read_ed25519_key_from_file(&cfg.peer_key_path)
        .wrap_err_with(|| {
            format!(
                "failed loading peer key from {}",
                cfg.peer_key_path.display()
            )
        })?;
    let staking_config = fluentbase_staking_reader::reader::StakingReaderConfig::from_json_path(
        &cfg.staking_config_path,
    )
    .wrap_err_with(|| {
        format!(
            "failed loading staking config from {}",
            cfg.staking_config_path.display()
        )
    })?;
    let bootstrappers = fluentbase_p2p::bootstrappers::load_from_json_path(&cfg.bootstrappers_path)
        .wrap_err_with(|| {
            format!(
                "failed loading bootstrappers from {}",
                cfg.bootstrappers_path.display()
            )
        })?;

    // The ONE network: build + start ONCE. The beacon halves go to the DkgActor;
    // the non-beacon halves are handed down to the (first) signer engine; the oracle
    // (the only Clone handle) is shared by the plane's ET and the engine's blocker.
    let listen = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), cfg.p2p_port);
    let dialable = cfg.dialable.unwrap_or(listen);
    let (p2p, handles) = FluentP2P::build(
        ctx.clone(),
        FluentP2PConfig {
            crypto: peer_keypair.clone(),
            chain_id,
            listen,
            dialable: Ingress::Socket(dialable),
            bootstrappers,
        },
    );
    let net_handle = p2p.start();

    // The 5 persistent non-beacon channel Muxers. The plane owns these brokers for
    // the whole process; each promotion's signer engine CLONES the `MuxHandle`s and
    // registers per-epoch (vote/cert/resolver) / subchannel-0 (broadcast/marshal)
    // sub-channels. A demoted engine drops its `SubReceiver`s (auto-deregister); the
    // next promotion re-registers against the SAME brokers — restart-free. The vote
    // Muxer carries a backup channel for catch-up hints; the plane forwards it to the
    // currently-active engine via a re-settable forwarder (`vote_backup`).
    let mux_mailbox = 256usize;
    let (mux_vote, vote_mux, vote_backup_rx) = Muxer::builder(
        ctx.with_label("plane_vote_mux"),
        handles.vote_sender,
        handles.vote_receiver,
        mux_mailbox,
    )
    .with_backup()
    .build();
    let (mux_cert, cert_mux) = Muxer::new(
        ctx.with_label("plane_cert_mux"),
        handles.cert_sender,
        handles.cert_receiver,
        mux_mailbox,
    );
    let (mux_res, resolver_mux) = Muxer::new(
        ctx.with_label("plane_resolver_mux"),
        handles.resolver_sender,
        handles.resolver_receiver,
        mux_mailbox,
    );
    let (mux_bcast, broadcast_mux) = Muxer::new(
        ctx.with_label("plane_broadcast_mux"),
        handles.broadcast_sender,
        handles.broadcast_receiver,
        mux_mailbox,
    );
    let (mux_marshal, marshal_mux) = Muxer::new(
        ctx.with_label("plane_marshal_mux"),
        handles.marshal_sender,
        handles.marshal_receiver,
        mux_mailbox,
    );
    // Adopt each Muxer's run-handle under a thin shim so all plane-broker handles
    // share one `Handle<()>` shutdown-abort type (the Muxer's `start()` returns a
    // `Handle<Result<(), Error>>`; aborting the shim aborts the awaited muxer task).
    let adopt = |label: &str, h: commonware_runtime::Handle<_>| -> Handle<()> {
        ctx.with_label(label).spawn(move |_| async move {
            if let Ok(Err(e)) = h.await {
                warn!(error = ?e, "plane Muxer p2p receiver failed");
            }
        })
    };
    let mut mux_handles: Vec<Handle<()>> = vec![
        adopt("plane_vote_mux_sup", mux_vote.start()),
        adopt("plane_cert_mux_sup", mux_cert.start()),
        adopt("plane_resolver_mux_sup", mux_res.start()),
        adopt("plane_broadcast_mux_sup", mux_bcast.start()),
        adopt("plane_marshal_mux_sup", mux_marshal.start()),
    ];

    // Vote-backup re-settable forwarder: the plane owns the move-only backup
    // receiver and re-broadcasts each catch-up item to the CURRENTLY-active
    // EpochManager (`subscribe()`d fresh per promotion). While no engine is up the
    // parked sender is `None`/closed and items are dropped — a follower needs no
    // catch-up hint.
    let vote_backup: ResettableForward<VoteBackupItem> = ResettableForward::new(mux_mailbox);
    mux_handles.push({
        let slot = vote_backup.slot();
        ctx.with_label("plane_vote_backup_fwd")
            .spawn(move |_| async move {
                let mut rx = vote_backup_rx;
                while let Some(item) = rx.recv().await {
                    let guard = slot.lock().await;
                    if let Some(tx) = guard.as_ref() {
                        let _ = tx.try_send(item);
                    }
                }
            })
    });

    // Shared live-DKG store, reloaded from `<datadir>/beacon/` ONCE.
    let beacon_dir = node.data_dir.data_dir().join("beacon");
    let ceremony_store: fluentbase_consensus::beacon::actor::CeremonyStore =
        Arc::new(std::sync::RwLock::new(std::collections::BTreeMap::new()));
    // Edge-trigger the DkgActor fires when a share lands; the EpochManager's
    // `enter` waits on it (instead of polling) so a signer that reaches the
    // boundary before its share is memoized wakes the instant it lands.
    let share_notify = Arc::new(tokio::sync::Notify::new());
    let reloaded = fluentbase_consensus::beacon::share_state::load_all(&beacon_dir, &share_state);
    if !reloaded.is_empty() {
        if let Ok(mut store) = ceremony_store.write() {
            for (epoch, output, share) in reloaded {
                info!(epoch, "beacon: reloaded persisted live-DKG share from disk");
                store.insert(epoch, (output, share));
            }
        }
    }

    // Live cursor (consensus-finalized ≈ EL-finalized + K) for catch-up reads.
    // The cert-inlet (on upstream-configured nodes) tees the live upstream cert
    // frontier here so a still-catching-up newcomer resolves committee[E] — and
    // therefore deals its first epoch's DKG share — at the live tip rather than
    // its lagging EL-finalized state (the early-join wedge: committee[E] is
    // ahead-committed during epoch E-1, but a lagging EL-finalized read returns it
    // too late, after the deal deadline). Stays 0 on a plain --dpos validator (its
    // executor IS the tip), so committee reads fall back to EL-finalized unchanged.
    let live_height = Arc::new(std::sync::atomic::AtomicU64::new(0));
    // On-chain committee resolver (deal/carry-forward set), shared by the DkgActor
    // and the per-engine verify gate. Reads committee[E] at max(EL-finalized, live
    // cursor) — committee[E] is content-invariant across any in-epoch executed hash
    // and the cursor is cert-finalized (no reorg), so reading at the executed-but-
    // not-yet-EL-finalized tip is sound and surfaces an ahead-committed committee[E]
    // K blocks sooner.
    let committee_for: fluentbase_consensus::beacon::actor::CommitteeFor = {
        let reader = RethStakingStateReader::new(
            node.provider.clone(),
            node.evm_config.clone(),
            staking_config.clone(),
        );
        let provider = node.provider.clone();
        let live_height = live_height.clone();
        Arc::new(move |epoch: u64| {
            let fin = provider.finalized_block_number().ok().flatten();
            let live = live_height.load(std::sync::atomic::Ordering::Relaxed);
            // No finalized marker AND no live cursor yet ⇒ not readable. `unwrap_or(0)`
            // here would read committee at genesis (`block_hash(0)`) → the wrong
            // committee on a genesis-committed devnet during the startup race.
            if fin.is_none() && live == 0 {
                return None;
            }
            let fin = fin.unwrap_or(0);
            let read_at = fin.max(live);
            // Fall back to the finalized hash if reth has not yet imported the
            // cursor block (the cert can land a beat before the EL-sync import).
            let hash = provider
                .block_hash(read_at)
                .ok()
                .flatten()
                .or_else(|| provider.block_hash(fin).ok().flatten())?;
            let snap = reader.epoch_committee_snapshot(epoch, hash).ok()?;
            if snap.validators.is_empty() {
                return None;
            }
            Some(commonware_utils::ordered::Set::from_iter_dedup(
                snap.validators.iter().map(|v| v.keys.peer_pubkey.clone()),
            ))
        })
    };

    // Beacon counters — registered ONCE here (the persistent layer); cloned (never
    // re-registered) into the DkgActor + each per-epoch signer engine.
    let beacon_metrics = fluentbase_consensus::beacon::metrics::BeaconMetrics::default();
    beacon_metrics.register(ctx);

    // EpochTransition-driven Oracle peer set + the `dkg_height` clock, both fed by a
    // persistent finalized-height poller (reth `finalized_block_number`) — a source
    // that exists in BOTH the follower and signer phases, unlike the per-engine
    // boundary hook. cold_start tracks the initial committee's peer set so the node
    // is connected on BEACON_CHANNEL from block 1 of its follower phase.
    let (dkg_height_tx, dkg_height_rx) = mpsc::channel::<u64>(256);
    let cache = Arc::new(Mutex::new(
        ValidatorSetCache::init(ctx.with_label("beacon_plane_cache"))
            .await
            .wrap_err("failed initializing beacon-plane ValidatorSetCache")?,
    ));
    let et_reader = RethStakingStateReader::new(
        node.provider.clone(),
        node.evm_config.clone(),
        staking_config.clone(),
    );
    let provider_for_et = node.provider.clone();
    let epoch_transition = EpochTransition::new(
        et_reader,
        cache,
        handles.oracle.clone(),
        fluentbase_p2p::constants::MAX_REGISTRY_PEER_SET as usize,
        None,
        Arc::new(move |n| provider_for_et.block_hash(n).ok().flatten()),
        fluentbase_consensus::K,
    );
    // Seed only the height-clock cursor — NO synchronous initial `cold_start`. At
    // process start reth may not have surfaced its persisted finalized marker yet
    // (`finalized_block_num_hash` then falls back to GENESIS, where a runtime-deployed
    // ChainConfig is still codeless ⇒ the geometry read reverts), so a cold-start
    // HERE would race that fallback. The plane is uniformly POLLER-driven: the 500ms
    // poller below runs the first `cold_start` off the LIVE finalized cursor — by its
    // first tick reth has surfaced the marker, so the geometry freezes from a readable
    // block (and `apply_at` stays codeless-tolerant for the rare slow tick).
    let cs_fin_num = node
        .provider
        .finalized_block_number()
        .ok()
        .flatten()
        .unwrap_or(0);
    let et_arc = Arc::new(Mutex::new(epoch_transition));
    // Fired ONCE by the poller the instant the ET freezes the geometry — the
    // event the DkgActor's spawn wrapper awaits before it constructs the actor
    // with plain `(activation, interval)`. Event-driven (not a poll/timer): the
    // EpochTransition is the single in-plane geometry source, and the actor takes
    // the value it already resolved instead of re-reading the chain itself.
    let geometry_ready = Arc::new(tokio::sync::Notify::new());

    // Finalized-height poller, feeding TWO sinks off the SAME EL-finalized cursor:
    //   - `dkg_height` ← `fin + K` (ORDERING-finalized): the executor sets the
    //     EL-finalized height = `result_final_height(tip, floor) = ordering_finalized
    //     − K` (`order_block.rs::result_final_height`, `K = fluentbase_consensus::K`),
    //     so `fin + K` is the ordering-finalized height that produced `fin`. The
    //     DkgActor's seal deadline + `epoch_start` geometry are ORDERING-chain
    //     quantities; feeding it the raw EL-finalized `fin` would silently shorten
    //     the `DKG_MARGIN_BLOCKS` window by K (the epoch-2 boundary wedge). For every
    //     epoch ≥ 1 the cold-start floor clamp is inactive, so `fin + K` == the
    //     ordering tip exactly.
    //   - `et.on_finalized(fin)` ← raw EL-finalized `fin`: the peer-set tracker's
    //     `read_height_for(n) = n − K` contract assumes a finalized input; do NOT
    //     shift it.
    let poller_handle = {
        let provider = node.provider.clone();
        let et = et_arc.clone();
        let dkg_tx = dkg_height_tx.clone();
        let geometry_ready = geometry_ready.clone();
        ctx.with_label("beacon_plane_poller")
            .spawn(move |c| async move {
                let mut sent = cs_fin_num;
                let _ = dkg_tx.try_send(cs_fin_num + fluentbase_consensus::K);
                loop {
                    c.sleep(Duration::from_millis(500)).await;
                    let Ok(Some(fin)) = provider.finalized_block_number() else {
                        continue;
                    };
                    while sent < fin {
                        sent += 1;
                        let _ = dkg_tx.try_send(sent + fluentbase_consensus::K);
                    }
                    // Bootstrap drive (event-driven on THIS existing poll, no second
                    // timer): until the geometry is frozen, `cold_start` off the LIVE
                    // finalized cursor — anchoring to the now-readable finalized block
                    // and freezing the instant it is a readable, DPoS-scheduled block
                    // (`apply_at` is codeless-tolerant, so a too-early tick defers). The
                    // instant it freezes, signal `geometry_ready` so the DkgActor's
                    // spawn wrapper takes the frozen `(activation, interval)`. Once
                    // frozen, switch to the steady `on_finalized` boundary walk (which
                    // REQUIRES the freeze).
                    let frozen_before = { et.lock().await.frozen_geometry().is_some() };
                    let outcome = if frozen_before {
                        // Drive the boundary detection; errors here are non-fatal to the
                        // beacon plane (the engine's own ET is the authoritative boundary
                        // path) — log and keep the peer set tracking.
                        et.lock().await.on_finalized(fin).await
                    } else {
                        // No `finalized_block_hash`-by-number on the provider here, so
                        // resolve the hash from the height we already have.
                        let Ok(Some(hash)) = provider.block_hash(fin) else {
                            continue;
                        };
                        let out = et.lock().await.cold_start(hash, fin).await;
                        // Freshly frozen on THIS tick ⇒ wake the DkgActor wrapper once.
                        if et.lock().await.frozen_geometry().is_some() {
                            geometry_ready.notify_one();
                        }
                        out
                    };
                    if let Err(e) = outcome {
                        warn!(finalized = fin, error = ?e, "beacon plane: ET on_finalized/cold_start failed");
                    }
                }
            })
    };

    // The persistent DkgActor — spawned ONCE, runs for the whole process. It is
    // constructed AFTER the poller has frozen the geometry, so it takes plain
    // `(activation, interval)` from the EpochTransition (the single in-plane source)
    // and never re-reads the chain — there is no codeless/genesis-fallback race in
    // this path. The wrapper awaits `geometry_ready` (the poller's freeze signal);
    // height ticks accumulate in `dkg_height_rx` meanwhile (bounded buffer) and are
    // drained by `on_height`'s monotone-max clamp once the actor runs — the first
    // epoch boundary is one interval away (≫ the ~one-tick freeze latency), so no
    // deal/seal is missed.
    let dkg_namespace = fluentbase_consensus::beacon::seed::seed_namespace(
        &fluentbase_bls::fluent_namespace(chain_id),
    );
    let dkg_handle = {
        let et = et_arc.clone();
        let geometry_ready = geometry_ready.clone();
        // Clones for the actor (the originals flow into `BeaconPlane.shared`).
        let committee_for = committee_for.clone();
        let ceremony_store = ceremony_store.clone();
        let share_notify = share_notify.clone();
        let beacon_metrics = beacon_metrics.clone();
        ctx.with_label("dkg_actor").spawn(move |c| async move {
            geometry_ready.notified().await;
            let Some((activation, interval)) = et.lock().await.frozen_geometry() else {
                // `notify_one` is only fired post-freeze, so this is unreachable for a
                // healthy node. A validator whose geometry is unreadable/unscheduled
                // already failed loud at launch (the raw-`0` guard + the ChainConfig
                // `?` in `DposLayer::launch`, which runs after this wrapper is spawned
                // and tears the process down), so this soft path is reached only by a
                // mis-configured non-validator — where staying network/Muxers up
                // (follower connectivity) with no DKG is the intended fail-soft.
                error!("beacon plane: geometry_ready fired but geometry unfrozen; DkgActor not started");
                return;
            };
            let dkg_actor = fluentbase_consensus::beacon::actor::DkgActor::new(
                dkg_namespace,
                peer_keypair,
                handles.beacon_sender,
                handles.beacon_receiver,
                committee_for,
                ceremony_store,
                share_notify,
                activation,
                interval,
                beacon_metrics,
                Some(beacon_dir),
                share_state,
            );
            dkg_actor.run(dkg_height_rx, c).await
        })
    };

    info!(
        listen = %listen,
        "always-on beacon plane built (one FluentP2P, persistent DkgActor; geometry frozen by the plane EpochTransition)"
    );

    Ok(BeaconPlane {
        net_handle,
        dkg_handle,
        poller_handle,
        mux_handles,
        shared: SharedBeaconPlane {
            oracle: handles.oracle,
            ceremony_store,
            share_notify,
            committee_for,
            beacon_metrics,
            // Share each MuxHandle behind Arc<Mutex> (the per-promotion Clone + the
            // transient register-lock; the move-only DiscReceiver makes the bare
            // MuxHandle un-Clone-able).
            vote_mux: Arc::new(Mutex::new(vote_mux)),
            cert_mux: Arc::new(Mutex::new(cert_mux)),
            resolver_mux: Arc::new(Mutex::new(resolver_mux)),
            broadcast_mux: Arc::new(Mutex::new(broadcast_mux)),
            marshal_mux: Arc::new(Mutex::new(marshal_mux)),
            vote_backup,
        },
        live_height,
        dkg_height_tx,
    })
}

/// Build and launch the DPoS layer once: load operator keys + JSON configs,
/// construct the `PoolTxSink`/deriver/assembler over the node's own
/// provider, hand everything to [`DposLayer::launch`], wire the cert-feed
/// actor and `set_marshal`. Extracted from [`run_dpos_stack`] so the
/// always-on-plane wiring stays separable from the layer launch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn launch_dpos_layer<N, AddOns>(
    ctx: Context,
    node: &FullNode<N, AddOns>,
    cfg: &DposConfig,
    bls_keypair: fluentbase_bls::keys::ValidatorBlsKeypair,
    beacon_engine: crate::importer::RethImporter,
    cert_feed: Option<CertFeed>,
    shared_beacon: SharedBeaconPlane,
    shutdown_token: CancellationToken,
) -> eyre::Result<DposLayerHandle>
where
    N: FullNodeComponents<
        Types: reth_node_api::NodeTypes<
            Payload = EthEngineTypes,
            Primitives = reth_ethereum_primitives::EthPrimitives,
        >,
    >,
    AddOns: RethRpcAddOns<N>,
    <N as FullNodeTypes>::Provider: Clone
        + BlockReader<Block = RethBlock>
        + BlockHashReader
        + BlockNumReader
        + BlockIdReader
        + CanonicalStateAccess
        + Send
        + Sync
        + 'static,
    <N as FullNodeComponents>::Evm: reth_evm::ConfigureEvm<
            Primitives = EthPrimitives,
            NextBlockEnvCtx = reth_evm::NextBlockEnvAttributes,
        > + Clone
        + Send
        + Sync
        + 'static,
{
    let chain_id = node.chain_spec().chain_id();
    let peer_keypair = fluentbase_p2p::read_ed25519_key_from_file(&cfg.peer_key_path)
        .wrap_err_with(|| {
            format!(
                "failed loading peer key from {}",
                cfg.peer_key_path.display()
            )
        })?;
    let slasher_signer = load_slasher_signer(cfg)?;

    let staking_config = fluentbase_staking_reader::reader::StakingReaderConfig::from_json_path(
        &cfg.staking_config_path,
    )
    .wrap_err_with(|| {
        format!(
            "failed loading staking config from {}",
            cfg.staking_config_path.display()
        )
    })?;

    // Build PoolTxSink host-side: PoolTxSink<P, Provider> carries concrete
    // reth-transaction-pool trait bounds (PoolTransaction<Consensus =
    // EthereumTxEnvelope<TxEip4844>>) that can't compile in the consensus
    // crate, so this construction stays here.
    let slasher_sink: Arc<dyn fluentbase_consensus::slasher::actor::SlasherTxSink> =
        Arc::new(crate::slasher_sink::PoolTxSink::new(
            slasher_signer,
            chain_id,
            node.pool.clone(),
            node.provider.clone(),
            node.evm_config.clone(),
        ));

    let canonical_state = node.provider.canonical_state();
    let genesis_hash = node.chain_spec().genesis_hash();
    let reth = RethHandle {
        provider: node.provider.clone(),
        evm_config: node.evm_config.clone(),
        beacon_engine_handle: beacon_engine,
        chain_id,
        canonical_state,
        genesis_hash,
    };

    // Deferred-execution collaborators, all over the node's own provider/EVM:
    // derive (reth-evm BlockBuilder), the derived-chain view, and the
    // pool-backed ordering assembler. The beacon is always-on live-DKG; the
    // deriver computes prev_randao = H(seed) directly from the cert-recovered
    // seed (no on-chain PK_E read — that layer is gone, DPOS_ARCHITECTURE §8.11).
    let deriver =
        crate::derive::RethBlockDeriver::new(node.provider.clone(), node.evm_config.clone());
    let executed = ProviderExecutedChain(node.provider.clone());
    let assembler = Arc::new(PoolAssembler::new(node.pool.clone(), executed.clone()));
    // The protocol fee manager — same recipient the pre-deferred attrs
    // builder used; uniform across honest nodes (agreed data once embedded).
    let fee_recipient = fluentbase_types::PRECOMPILE_FEE_MANAGER;
    // Gas-limit target = the operator's `--builder.gaslimit` (the canonical reth
    // knob — the SAME source the payload builder reads via `gas_limit_for`),
    // falling back to the chain's genesis gas limit when unset. The EIP-1559
    // ±1/1024 step in `application::step_gas_limit` walks the agreed limit toward
    // it; pinning to genesis would make that step a permanent no-op.
    let target_gas_limit = node
        .config
        .builder
        .gas_limit()
        .unwrap_or_else(|| node.chain_spec().genesis().gas_limit);

    // Cert-feed: the FeedSink goes DOWN into the marshal as its 2nd Reporter; the
    // receiver + state handle stay here to drive the node-side feed actor + RPC.
    let (feed_sink, feed_actor_wiring) = match cert_feed {
        Some(cf) => (Some(cf.sink), Some((cf.rx, cf.handle))),
        None => (None, None),
    };

    // DEVNET/TEST-ONLY: parse the byzantine mode string. An unknown value
    // fails-loud rather than silently running honest (a misconfigured smoke must
    // not false-pass). The whole block is gated out of production builds.
    #[cfg(feature = "dpos-devnet-byzantine")]
    let byzantine = match cfg.byzantine_mode.as_deref() {
        None => None,
        Some("forge-beacon-pk") => {
            tracing::warn!(
                "DEVNET BYZANTINE MODE ACTIVE: forge-beacon-pk — NEVER use in production"
            );
            Some(fluentbase_consensus::byzantine::ByzantineMode::ForgeBeaconPk)
        }
        Some("equivocate") => {
            tracing::warn!("DEVNET BYZANTINE MODE ACTIVE: equivocate — NEVER use in production");
            Some(fluentbase_consensus::byzantine::ByzantineMode::Equivocate)
        }
        Some(other) => {
            eyre::bail!(
                "unknown --dpos.byzantine mode: {other:?} \
                 (expected `forge-beacon-pk` or `equivocate`)"
            )
        }
    };

    // Cert upstream for an upstream-configured validator (`--dpos.follower-upstream`),
    // serving TWO consensus-side consumers off ONE WS actor: (1) the single-shot,
    // pre-engine cold-start EL-sync JUMP (`cold_start_jump`) — a deeply-behind
    // external joiner / follower fast-forwards reth before its OuterEngine starts;
    // and (2) the marshal's by-height backfill resolver, which `launch` keeps alive
    // for the engine lifetime so an OUT-OF-COMMITTEE validator (zero consensus-plane
    // connectivity) backfills the cold-start `[floor+1 .. first_live]` gap from the
    // upstream instead of wedging (the validator-with-upstream wedge fix). The actor
    // is started so the handle's `get_latest`/`get_finalization` round-trips work; it
    // stays alive as long as the resolver holds the handle. A no-upstream validator
    // passes `None` and catches up on the consensus-plane treadmill instead. `launch`
    // itself gates: FreshMigration never jumps. NOTE this WS actor is independent of
    // the live-stream cert-inlet's WS actor (`spawn_cert_inlet` in the overlay) — the
    // inlet drives the live frontier; this one serves the marshal's by-height pulls.
    let upstream = (!cfg.follower_upstreams.is_empty()).then(|| {
        // This WS serves ONLY the marshal's by-height resolver pulls — its live
        // stream + connection-generation token are unused here (the validator's
        // live-stream inlet has its OWN WS actor in `spawn_cert_inlet`).
        let (ws_actor, handle, _live_rx, _conn_gen) =
            crate::cert_follow::upstream::init(ctx.clone(), cfg.follower_upstreams.clone());
        drop(ws_actor.start());
        handle
    });

    let layer_cfg = DposLayerConfig {
        bls_keypair,
        peer_keypair,
        slasher_sink,
        staking_config,
        upstream,
        deriver,
        executed,
        assembler,
        fee_recipient,
        target_gas_limit,
        feed: feed_sink,
        // Mid-epoch promotion trigger: the executor fires it on each finalized-advance
        // and the EpochManager re-checks parked spawns. Created here, internal to the
        // layer (executor producer + EpochManager consumer share this one Arc).
        spawn_unblocked: std::sync::Arc::new(tokio::sync::Notify::new()),
        // The always-on beacon plane (shared store + committee resolver + the single
        // network's oracle + the once-registered metrics + the 5 non-beacon MuxHandles
        // + the vote-backup forwarder). The signer engine CLONES these per promotion;
        // it never rebuilds the network, re-spawns the DkgActor, or consumes a raw
        // channel half — so a demote→re-promote re-clones with no rebuild.
        beacon_plane: shared_beacon,
        #[cfg(feature = "dpos-devnet-byzantine")]
        byzantine,
    };

    // Spawn the cert-feed actor on a child of the runtime context BEFORE `launch`
    // consumes `ctx`. It blocks on the channel until finalizations flow (post-launch),
    // by which point `set_marshal` (below) has run. Keep the handle for `set_marshal`.
    let feed_handle = feed_actor_wiring.map(|(rx, handle)| {
        let actor_handle = handle.clone();
        drop(ctx.with_label("cert_feed").spawn(move |_| async move {
            FeedActor::new(rx, actor_handle).run().await;
        }));
        handle
    });

    let handle: DposLayerHandle = DposLayer::launch(ctx, reth, layer_cfg, shutdown_token).await?;

    // Hand the marshal mailbox to the feed state (node-side, respecting the crate
    // boundary — consensus never names node types). Until this runs the RPC returns
    // ServiceUnavailable; the window is sub-finalization so no event is lost.
    if let Some(fh) = feed_handle {
        fh.set_marshal(handle.cert_mailbox.clone());
    }

    Ok(handle)
}

/// DEVNET-ONLY: minimal HTTP/1.0 responder serving the commonware runtime's
/// prometheus metrics (`ctx.encode()`) on every request. Uses `tokio::net` (not
/// `std::net`) so the blocking accept never starves the shared async executor.
#[cfg(feature = "dpos-devnet-metrics")]
async fn serve_metrics(ctx: Context, port: u16) {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port);
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            error!(error = ?e, port, "metrics_http: bind failed; metrics endpoint disabled");
            return;
        }
    };
    loop {
        let mut sock = match listener.accept().await {
            Ok((s, _)) => s,
            Err(e) => {
                warn!(error = ?e, "metrics_http: accept failed");
                continue;
            }
        };
        let mut scratch = [0u8; 1024];
        let _ = sock.read(&mut scratch).await; // drain the request; we ignore it
        let body = ctx.encode();
        let resp = format!(
            "HTTP/1.0 200 OK\r\nContent-Type: text/plain; version=0.0.4\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let _ = sock.write_all(resp.as_bytes()).await;
        let _ = sock.shutdown().await;
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Host-only key loading helpers (filesystem syscalls + permission checks)
// ───────────────────────────────────────────────────────────────────────────

/// Load the validator BLS keypair AND, IFF it came from an EIP-2335 keystore (an
/// off-disk operator secret exists), the HKDF-derived [`ShareSealKey`] that seals
/// the per-epoch DKG shares at rest (E2). The plaintext-dev branch
/// (`--dpos.bls-key-path`) returns `None` — its validator key is already plaintext
/// beside the datadir, so a key derived from it buys nothing ⇒ shares stay
/// `TAG_PLAINTEXT`.
fn load_bls_keypair(
    cfg: &DposConfig,
    chain_id: u64,
) -> eyre::Result<(
    fluentbase_bls::keys::ValidatorBlsKeypair,
    Option<fluentbase_bls::ShareSealKey>,
)> {
    match (
        cfg.bls_keystore_path.as_deref(),
        cfg.bls_key_path.as_deref(),
    ) {
        (Some(keystore_path), None) => {
            let password_path = cfg.bls_keystore_password_file.as_deref().ok_or_eyre(
                "--dpos.bls-keystore-path requires --dpos.bls-keystore-password-file",
            )?;
            let password = read_password_file(password_path, "BLS keystore")?;
            let keypair = fluentbase_bls::keys::ValidatorBlsKeypair::read_from_keystore(
                keystore_path,
                password.trim().as_bytes(),
            )
            .wrap_err_with(|| {
                format!(
                    "failed loading BLS keystore from {}",
                    keystore_path.display()
                )
            })?;
            let seal_key = keypair.derive_share_seal_key(chain_id);
            Ok((keypair, Some(seal_key)))
        }
        (None, Some(plain_path)) => {
            use crate::chainspec::{
                FLUENT_DEVNET_CHAIN_ID, FLUENT_MAINNET_CHAIN_ID, FLUENT_TESTNET_CHAIN_ID,
            };
            if matches!(
                chain_id,
                FLUENT_DEVNET_CHAIN_ID | FLUENT_TESTNET_CHAIN_ID | FLUENT_MAINNET_CHAIN_ID
            ) {
                return Err(eyre!(
                    "--dpos.bls-key-path (plaintext BLS key) is forbidden on deployed network \
                     (chain_id {chain_id}); production must use --dpos.bls-keystore-path with an \
                     EIP-2335 keystore"
                ));
            }
            info!(chain_id, path = %plain_path.display(), "loading dev/test plaintext BLS key");
            check_mode_600(plain_path).wrap_err("plaintext BLS key file mode check")?;
            let keypair = fluentbase_bls::keys::ValidatorBlsKeypair::read_from_file(plain_path)
                .wrap_err_with(|| {
                    format!("failed loading BLS key from {}", plain_path.display())
                })?;
            Ok((keypair, None))
        }
        _ => Err(eyre!(
            "exactly one of --dpos.bls-keystore-path | --dpos.bls-key-path must be set"
        )),
    }
}

fn load_slasher_signer(cfg: &DposConfig) -> eyre::Result<alloy_signer_local::PrivateKeySigner> {
    let keystore_path = cfg.slasher_keystore_path.as_deref().ok_or_eyre(
        "--dpos.slasher-keystore-path is required (with --dpos.slasher-keystore-password-file)",
    )?;
    let password_path = cfg.slasher_keystore_password_file.as_deref().ok_or_eyre(
        "--dpos.slasher-keystore-path requires --dpos.slasher-keystore-password-file",
    )?;
    let password = read_password_file(password_path, "slasher keystore")?;
    alloy_signer_local::LocalSigner::decrypt_keystore(keystore_path, password.trim())
        .map_err(|e| eyre!("failed decrypting slasher keystore: {e}"))
}

/// Read a keystore password file: enforce 0600 (or stricter) mode, then read into
/// a zeroizing buffer cleared on drop. `what` labels the file in error messages.
fn read_password_file(
    path: &std::path::Path,
    what: &str,
) -> eyre::Result<zeroize::Zeroizing<String>> {
    check_mode_600(path).wrap_err_with(|| format!("{what} password file mode check"))?;
    Ok(zeroize::Zeroizing::new(
        std::fs::read_to_string(path)
            .wrap_err_with(|| format!("failed reading {what} password from {}", path.display()))?,
    ))
}

#[cfg(unix)]
fn check_mode_600(path: &std::path::Path) -> eyre::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path)?.permissions().mode();
    if mode & 0o077 != 0 {
        return Err(eyre!(
            "{} has insecure permissions (mode 0o{:03o}); chmod 600",
            path.display(),
            mode & 0o777,
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_mode_600(_path: &std::path::Path) -> eyre::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chainspec::{
        FLUENT_DEVNET_CHAIN_ID, FLUENT_MAINNET_CHAIN_ID, FLUENT_TESTNET_CHAIN_ID,
    };

    fn cfg_with_plaintext_bls(path: &str) -> DposConfig {
        DposConfig {
            bls_key_path: Some(PathBuf::from(path)),
            bls_keystore_path: None,
            bls_keystore_password_file: None,
            peer_key_path: PathBuf::new(),
            staking_config_path: PathBuf::new(),
            bootstrappers_path: PathBuf::new(),
            p2p_port: 0,
            dialable: None,
            slasher_keystore_path: None,
            slasher_keystore_password_file: None,
            metrics_port: None,
            cert_feed: None,
            follower_upstreams: vec![],
            #[cfg(feature = "dpos-devnet-byzantine")]
            byzantine_mode: None,
        }
    }

    #[test]
    fn plaintext_bls_rejected_on_deployed_networks() {
        for cid in [
            FLUENT_DEVNET_CHAIN_ID,
            FLUENT_TESTNET_CHAIN_ID,
            FLUENT_MAINNET_CHAIN_ID,
        ] {
            let cfg = cfg_with_plaintext_bls("/nonexistent/bls.hex");
            let err = load_bls_keypair(&cfg, cid).unwrap_err().to_string();
            assert!(
                err.contains("forbidden on deployed network"),
                "chain_id {cid}: expected deployed-network rejection, got: {err}"
            );
        }
    }

    #[test]
    fn plaintext_bls_gate_bypassed_on_local_network() {
        // chain_id 1337 (localnet) is not in the deployed set, so the gate
        // must NOT fire; loading then proceeds to file I/O and fails there
        // (nonexistent path) — proving the rejection is chain_id-scoped.
        let cfg = cfg_with_plaintext_bls("/nonexistent/bls.hex");
        let err = load_bls_keypair(&cfg, 1337).unwrap_err().to_string();
        assert!(
            !err.contains("forbidden on deployed network"),
            "local chain_id must bypass the deployed-network gate, got: {err}"
        );
    }
}
