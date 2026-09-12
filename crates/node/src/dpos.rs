//! DPoS host adapter: spawns the dedicated OS thread + commonware-tokio
//! runtime, loads operator keys and configs from disk, builds the
//! `RethHandle` + `DposLayerConfig`, calls
//! [`fluentbase_consensus::dpos::DposLayer::launch`], then runs the
//! shutdown supervisor `select!`.

use crate::consensus_rpc::{feed_actor::FeedActor, FeedStateHandle};
// `Metrics` (ctx.with_label, ctx.encode) + `Spawner` (ctx.spawn) — used by the
// always-on beacon plane, the cert-feed actor, and the feature-gated devnet
// metrics endpoint.
use commonware_consensus::types::{Epoch, Height};
use commonware_cryptography::Signer as _;
use commonware_p2p::{
    utils::mux::{Builder, Muxer},
    Blocker as _, Ingress, Receiver as _, Recipients, Sender as _,
};
use commonware_runtime::{tokio::Context, Handle, IoBuf, Metrics as _, Spawner as _};
use eyre::{eyre, OptionExt as _, WrapErr as _};
use fluentbase_bls::PeerPubkey;
use fluentbase_consensus::dpos::{
    DposLayer, DposLayerConfig, DposLayerHandle, ResettableForward, RethHandle, SharedBeaconPlane,
    VoteBackupItem,
};
pub use fluentbase_consensus::FeedSink;
use fluentbase_p2p::{
    bootstrappers::{classify_spec, load_from_dns, load_from_json_path, BootstrapperSpec},
    constants::epoch_from_subchannel,
    FluentP2P, FluentP2PConfig,
};
use fluentbase_staking_reader::{reader::RethStakingStateReader, EpochTransition};
use reth_chain_state::CanonicalInMemoryState;
use reth_chainspec::EthChainSpec as _;
use reth_ethereum_engine_primitives::EthEngineTypes;
use reth_ethereum_primitives::{Block as RethBlock, EthPrimitives};
use reth_network_api::PeersInfo;
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
use tracing::{debug, error, info, warn};

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
    /// AWS KMS key id/ARN/alias wrapping the validator BLS key (envelope-at-rest).
    /// Mutually exclusive with `bls_key_path`/`bls_keystore_path`.
    pub bls_kms_key_id: Option<String>,
    /// Local file holding the `kms:Encrypt` CiphertextBlob of the BLS scalar
    /// (non-secret). Companion of `bls_kms_key_id`.
    pub bls_kms_ciphertext_path: Option<PathBuf>,
    pub peer_key_path: PathBuf,
    pub staking_config_path: PathBuf,
    /// Cold-start peer discovery source: a bootstrappers JSON file path, or
    /// `dns:<domain>` for DNS TXT seed discovery. Dispatched via
    /// `fluentbase_p2p::bootstrappers::classify_spec` at node assembly.
    pub bootstrappers: String,
    pub p2p_port: u16,
    /// Advertised ingress as `"host:port"` (IP literal or, under the
    /// network-wide `ALLOW_DNS` policy, a DNS hostname). Parsed at config-build
    /// time via `fluentbase_p2p::parse_ingress`; `None` = advertise the listen
    /// socket.
    pub dialable: Option<String>,
    /// EIP-2335 / Web3 Secret Storage v3 keystore JSON for the slasher EOA.
    pub slasher_keystore_path: Option<PathBuf>,
    pub slasher_keystore_password_file: Option<PathBuf>,
    /// AWS KMS key id/ARN/alias for the slasher EOA — true remote signing.
    /// Mutually exclusive with `slasher_keystore_path`.
    pub slasher_kms_key_id: Option<String>,
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

    /// AWS KMS key id/ARN/alias for the validator BLS key (envelope-at-rest:
    /// `kms:Decrypt` the `--dpos.bls-kms-ciphertext-path` blob at boot; signing stays
    /// local — KMS cannot sign BLS12-381). Allowed on deployed networks.
    #[arg(
        long = "dpos.bls-kms-key-id",
        env = "FLUENT_DPOS_BLS_KMS_KEY_ID",
        conflicts_with_all = ["dpos_bls_key_path", "dpos_bls_keystore_path"],
        requires = "dpos_bls_kms_ciphertext_path",
        group = "bls"
    )]
    pub dpos_bls_kms_key_id: Option<String>,

    /// Local file holding the `kms:Encrypt` CiphertextBlob of the BLS scalar
    /// (non-secret; produced offline via `aws kms encrypt`). Companion of
    /// `--dpos.bls-kms-key-id`.
    #[arg(
        long = "dpos.bls-kms-ciphertext-path",
        env = "FLUENT_DPOS_BLS_KMS_CIPHERTEXT_PATH"
    )]
    pub dpos_bls_kms_ciphertext_path: Option<PathBuf>,

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

    /// Cold-start peer discovery source. Required when `--dpos` is set — no
    /// in-tree per-chain defaults. Two forms:
    /// - a **JSON file path** with `[{peer_pubkey, socket}, ...]` (pass an empty
    ///   `[]` for a genesis bootstrap event — explicit operator intent for the
    ///   first bootnode in a new network); or
    /// - `dns:<domain>` — resolve the domain's TXT records, each one a
    ///   `pubkey@host:port` seed (retried with backoff for ~2 min at startup).
    #[arg(
        long = "dpos.bootstrappers",
        env = "FLUENT_DPOS_BOOTSTRAPPERS",
        required_if_eq("dpos", "true")
    )]
    pub dpos_bootstrappers: Option<String>,

    /// Listen port for commonware p2p (default 9000).
    #[arg(
        long = "dpos.p2p-port",
        env = fluentbase_p2p::constants::LISTEN_PORT_ENV_VAR,
        default_value_t = fluentbase_p2p::constants::DEFAULT_LISTEN_PORT,
    )]
    pub dpos_p2p_port: u16,

    /// Override dialable address (`host:port`, what we tell peers); default =
    /// listen. Accepts an IP literal or, under the network-wide `ALLOW_DNS`
    /// policy, a DNS hostname (e.g. `validator-3:9000`). Parsed at startup.
    #[arg(long = "dpos.dialable", env = "FLUENT_DPOS_DIALABLE")]
    pub dpos_dialable: Option<String>,

    /// DEVNET-ONLY: serve commonware consensus metrics (prometheus text) on this
    /// host port for the smoke regression suite. Unset = disabled (prod default).
    #[arg(long = "dpos.metrics-port", env = "FLUENT_DPOS_METRICS_PORT")]
    pub dpos_metrics_port: Option<u16>,

    /// DEPRECATED (kept functional as an escape hatch — remove for validators in a
    /// follow-up once the plane path is smoke-proven). Upstream `consensus` WS URL(s)
    /// for the validator-side cert-inlet (repeatable — failover list). As of Gap A a
    /// plain `--dpos` validator syncs plane-natively (frontier resolver +
    /// `MarshalResolver::Hybrid`), so this WS upstream is NO LONGER REQUIRED; set it
    /// only for an explicit WS path (a not-yet-plane-tracked node, or sentry-only
    /// serving). Presence arms the inlet as a second producer into this node's marshal
    /// and selects `ValidatorUpstream::Ws` instead of the default `::Plane`.
    #[arg(
        long = "dpos.follower-upstream",
        env = "FLUENT_DPOS_FOLLOWER_UPSTREAM",
        action = clap::ArgAction::Append
    )]
    pub dpos_follower_upstream: Vec<String>,

    /// EIP-2335 / Web3 Secret Storage v3 keystore JSON for the slasher EOA.
    /// Mutually exclusive with `--dpos.slasher-kms-key-id`.
    #[arg(
        long = "dpos.slasher-keystore-path",
        env = "FLUENT_DPOS_SLASHER_KEYSTORE_PATH",
        requires = "dpos_slasher_keystore_password_file",
        group = "slasher"
    )]
    pub dpos_slasher_keystore_path: Option<PathBuf>,

    /// AWS KMS key id/ARN/alias for the slasher EOA (secp256k1) — true remote signing,
    /// key never leaves KMS. Mutually exclusive with `--dpos.slasher-keystore-path`.
    #[arg(
        long = "dpos.slasher-kms-key-id",
        env = "FLUENT_DPOS_SLASHER_KMS_KEY_ID",
        group = "slasher"
    )]
    pub dpos_slasher_kms_key_id: Option<String>,

    /// Password file for `--dpos.slasher-keystore-path`. Mode must be
    /// `0o600` (or stricter); fail-stops on world/group readable bits.
    #[arg(
        long = "dpos.slasher-keystore-password-file",
        env = "FLUENT_DPOS_SLASHER_KEYSTORE_PASSWORD_FILE"
    )]
    pub dpos_slasher_keystore_password_file: Option<PathBuf>,

    /// DEVNET/TEST-ONLY byzantine validator mode (`equivocate`). Compiled in ONLY
    /// with the `dpos-devnet-byzantine` cargo feature; the flag does not exist in
    /// a production build. Used by the byzantine equivocation smoke to prove the
    /// slasher jails a double-signer.
    #[cfg(feature = "dpos-devnet-byzantine")]
    #[arg(long = "dpos.byzantine", env = "FLUENT_DPOS_BYZANTINE")]
    pub dpos_byzantine: Option<String>,
}

impl DposConfig {
    /// Build from parsed [`DposArgs`] plus the runtime-wired `cert_feed`. Only reached when `--dpos` is set, so the
    /// `required_if_eq("dpos", "true")` clap rules guarantee `peer_key_path` /
    /// `staking_config_path` / `bootstrappers` are `Some`.
    pub fn from_args(args: &DposArgs, cert_feed: Option<CertFeed>) -> Self {
        Self {
            bls_key_path: args.dpos_bls_key_path.clone(),
            bls_keystore_path: args.dpos_bls_keystore_path.clone(),
            bls_keystore_password_file: args.dpos_bls_keystore_password_file.clone(),
            bls_kms_key_id: args.dpos_bls_kms_key_id.clone(),
            bls_kms_ciphertext_path: args.dpos_bls_kms_ciphertext_path.clone(),
            peer_key_path: args
                .dpos_peer_key_path
                .clone()
                .expect("required_if_eq guarantees --dpos.peer-key-path"),
            staking_config_path: args
                .dpos_staking_config
                .clone()
                .expect("required_if_eq guarantees --dpos.staking-config"),
            bootstrappers: args
                .dpos_bootstrappers
                .clone()
                .expect("required_if_eq guarantees --dpos.bootstrappers"),
            p2p_port: args.dpos_p2p_port,
            dialable: args.dpos_dialable.clone(),
            slasher_keystore_path: args.dpos_slasher_keystore_path.clone(),
            slasher_keystore_password_file: args.dpos_slasher_keystore_password_file.clone(),
            slasher_kms_key_id: args.dpos_slasher_kms_key_id.clone(),
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
/// mapped inlet in v1 (the standalone bare `--cert-upstream` trust relay is the
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
    /// DEVNET-ONLY (`--dpos.metrics-port`): serve the commonware registry as
    /// prometheus text on this host port. `None` = disabled (prod default).
    ///
    /// Process-level, NOT per-overlay: the endpoint binds one socket and encodes
    /// the ONE registry the whole runtime shares, and both node classes own
    /// families in it (the follower is the sole owner of the
    /// `dpos_follower_artifact_*` families, via `beacon::build_follower`). A copy
    /// per overlay payload would be two sources for one socket.
    pub metrics_port: Option<u16>,
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
        metrics_port,
    } = cfg;

    spawn_devnet_metrics(&ctx, metrics_port);

    // DIVERGENCE: the overlay. Validator = full 5-Muxer beacon plane +
    // local-BFT engine (+ optional inlet as a 2nd producer); follower =
    // near-planeless broadcast Muxer + inlet-only engine. Both return the
    // same `(DposLayerHandle, supervised handles)` shape so the supervisor is
    // a single path.
    let (mut engine, supervised): (DposLayerHandle, Vec<SupervisedHandle>) = if is_validator {
        let dpos_cfg = validator.ok_or_else(|| {
            eyre!("run_node_stack: is_validator=true requires a validator config")
        })?;
        launch_validator_overlay(ctx, node, dpos_cfg, cert_inlet, shutdown_token.clone()).await?
    } else {
        let follow_cfg = follower.ok_or_else(|| {
            eyre!("run_node_stack: is_validator=false requires a follower config")
        })?;
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

    // SHARED SPINE: the shutdown supervisor over the engine + every overlay
    // handle, uniform for both modes. On any unexpected exit cancel the shared
    // token (so reth/main bring everything down) then abort the survivors.
    let exit_reason = supervise(&shutdown_token, &mut engine.consensus_handle, supervised).await;

    // ORDERING INVARIANT, and it is the entire reason this call sits HERE.
    // Every drain writer is a `while let Some(_) = rx.recv().await` loop, so each
    // one exits only once the LAST sender for its channel is gone. The rule for
    // adding a drain participant is therefore about SENDERS, not about where the
    // task was spawned: at the instant `drain_shutdown_tasks` is entered, no
    // sender feeding it may still be reachable from anything alive. Miss that and
    // the drain cannot do anything but burn `SHUTDOWN_DRAIN_TIMEOUT` and warn —
    // and that warning's whole job is to mean "the disk is stuck".
    //
    // THREE writer senders now live inside ONE object — the `Arc<dyn Beacon>`,
    // which owns the seed store, the key store AND the artifact store (the last
    // one moved in with `LiveBeaconConfig.artifacts`). So the condition is no
    // longer "the store clone died" but "EVERY `Arc<dyn Beacon>` clone died", and
    // the four that exist in this process are:
    //
    //  - The consensus layer's, moved into the engine's config, and the cert
    //    inlet's (`plane.shared.randomness.clone()` at the inlet spawn). Both are
    //    reachable only from tasks `supervise` above aborts. Awaiting the drains
    //    BEFORE that would deadlock. The mirror-image constraint lives in
    //    `beacon::build`: all three writers are spawned there, outside any engine
    //    task's spawn lineage, so `engine.abort()` cannot kill the task it is
    //    supposed to be releasing — see
    //    `the_journal_writer_survives_the_engine_abort_only_outside_its_spawn_lineage`.
    //
    //  - `launch`'s own local inside the consensus crate, which it drops when it
    //    returns its `DposLayerHandle` — the validator handle carries no beacon
    //    (`artifact_bytes: None`, `drain_on_shutdown: vec![]`).
    //
    //  - The HOST's own clone is the one nothing aborts, and it is released one
    //    frame down, at the end of `launch_validator_overlay` — see the explicit
    //    `drop(plane.shared)` there and why it is explicit.
    //
    // The RPC artifact feed is deliberately NOT on that list: it holds a `Weak`
    // (see where `artifact_bytes` is built in `build_beacon_plane`), because
    // nothing aborts reth's module registry.
    let drains = std::mem::take(&mut engine.drain_on_shutdown);
    drain_shutdown_tasks(drains).await;

    info!(reason = exit_reason, "node thread exiting");
    Ok(())
}

/// One graceful-shutdown drain task: a label (for the drain log) + its handle.
///
/// Structurally identical to [`SupervisedHandle`] and deliberately a SEPARATE
/// alias, because the semantics are opposite: a supervised handle resolving
/// means "a subsystem died, take the node down", while a drain handle resolving
/// means "this task finished the work it owed, shutdown may proceed". The two
/// lists must never be merged.
pub(crate) type DrainHandle = (&'static str, Handle<()>);

/// How long ONE shutdown-drain task gets before the node stops waiting for it.
/// Bounded on purpose: a wedged writer — a stuck fsync on a dying disk — must
/// not be able to hang the node's exit. Generous next to the work actually owed
/// (a final drain is at most a few hundred 68-byte appends plus one fsync per
/// touched section), so it fires only on a genuinely stuck device.
const SHUTDOWN_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

/// Datadir file name of the fork-safety halt marker. Its EXISTENCE is the halt
/// record; its one line names the [`fluentbase_consensus::sync_metrics`] reason.
/// An operator deletes it — after the fork is resolved on L1 — to let this node
/// participate again; nothing in the node ever removes it.
pub const SAFETY_HALT_MARKER: &str = "dpos_safety_halt";

/// Let every graceful-shutdown drain task finish, each under its own bounded
/// timeout, then return regardless.
///
/// On timeout we `warn` and proceed rather than refuse to exit: what these tasks
/// flush is a durability optimisation, and losing a seed-journal tail degrades
/// to post-restart store MISSES (the pre-durability behaviour), never to a wrong
/// σ. Hanging the node's shutdown would be the strictly worse failure.
///
/// MUST be called only once whatever owns each task's input channel is down —
/// see the ordering invariant at the call site.
async fn drain_shutdown_tasks(drains: Vec<DrainHandle>) {
    for (label, handle) in drains {
        match tokio::time::timeout(SHUTDOWN_DRAIN_TIMEOUT, handle).await {
            Ok(Ok(())) => info!(task = label, "shutdown drain finished"),
            Ok(Err(e)) => {
                warn!(task = label, error = ?e, "shutdown drain task did not finish cleanly")
            }
            Err(_) => warn!(
                task = label,
                timeout = ?SHUTDOWN_DRAIN_TIMEOUT,
                "shutdown drain timed out; exiting without it"
            ),
        }
    }
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
    let (bls_keypair, share_seal_key) = load_bls_keypair(&cfg, chain_id).await?;

    // Cert-inlet: a SECOND producer into this validator's own marshal, armed
    // whenever `--dpos.follower-upstream` URLs are configured (the production-path
    // fix for a rotated-out validator: it follows the inlet-fed base while
    // `reconcile_roles` keeps it a Verifier, then re-promotes in place when it
    // rejoins the committee). Constructed BEFORE `launch_dpos_layer` consumes
    // `ctx`/`cert_feed`. `None` when no upstream URLs are configured.
    let inlet_setup = cert_inlet
        .as_ref()
        .map(|inlet| (ctx.clone(), inlet.urls.clone()));

    // Always-on beacon plane (one FluentP2P + 5 persistent Muxers + persistent
    // DkgActor + shared store), built ONCE — the engine CLONES the shared plane.
    let plane = build_beacon_plane(&ctx, &node, &cfg, share_seal_key, bls_keypair.clone()).await?;
    // Rule Y: ONE shared upstream-frontier atomic threaded into BOTH the validator
    // inlet tee (writer) and the layer's `executor::ReJump` (reader), so an
    // inlet-fed joiner validator self-heals off the frontier exactly like a
    // follower. Created unconditionally — harmless for a no-upstream validator
    // (no inlet writes it; `re_jump` is `None` so nothing reads it).
    let upstream_frontier = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    // Cloned before `launch_dpos_layer` consumes `ctx` — used to spawn the
    // frontier-resolver keepalive task below.
    let keepalive_ctx = ctx.clone();
    let mut handle = launch_dpos_layer(
        ctx,
        &node,
        &cfg,
        bls_keypair,
        beacon_engine,
        cert_feed,
        plane.shared.clone(),
        plane.plane_upstream.clone(),
        plane.evidence.clone(),
        upstream_frontier.clone(),
        plane.agreement_intake,
        plane.finalized_cursor.clone(),
        plane.committee.clone(),
        plane.tracked_epoch.clone(),
        plane.artifact_bytes,
        plane.epoch_transition,
        shutdown_token,
    )
    .await?;

    // Fill the DkgActor's deferred marshal READ handle now that the layer launch has
    // created the marshal mailbox — the demote-heal recompute (§8.11.1) reads pinned
    // boundary outcomes through it. `set` succeeds exactly once (idempotent-safe).
    let _ = plane.marshal_slot.set(handle.cert_mailbox.clone());

    // Spawn the cert-inlet against the SAME marshal the local engine drives
    // (`handle.cert_mailbox`). `None` when no upstreams are configured. The inlet
    // tees the LIVE upstream cert frontier into the DkgActor deal clock (re-homed
    // from the deleted unified supervisor) — so a still-catching-up validator
    // deals its DKG share at the live tip rather than K blocks late. It no longer
    // tees anything into a committee-read cursor: this node reads every committee
    // through the module, at its own ordering-finalized anchor, and `live_height`
    // is write-only on this path until 4.2 removes it.
    let inlet_handle = inlet_setup.map(|(inlet_ctx, urls)| {
        crate::cert_inlet::spawn_cert_inlet(
            inlet_ctx,
            handle.cert_mailbox.clone(),
            plane.committee.clone(),
            urls,
            fluentbase_consensus::cert_inlet::LiveFrontierTee {
                live_height: plane.live_height.clone(),
                // Rule Y: the SAME atomic shared with this node's `executor::ReJump`
                // (the inlet⇄executor signal). When this validator is an inlet-fed
                // joiner (rotated out, following the inlet base) its re-jump self-heals
                // off the climbing frontier exactly like a follower — symmetric wiring.
                // See [`LiveFrontierTee::upstream_frontier`].
                upstream_frontier: upstream_frontier.clone(),
                dkg_height_tx: plane.dkg_height_tx.clone(),
                plane_clock: plane.shared.plane_clock.clone(),
            },
            // The SAME provider the consensus layer holds, not a second one over
            // a private store: `observe_cert` prunes what the key ladder reads, and a
            // split store would make the pruning a silent no-op.
            plane.shared.randomness.clone(),
        )
    });

    // Collect every overlay handle for the shared supervisor. Total upstream
    // loss / inner committee fatal resolves the inlet handle → the supervisor
    // cancels the shared token (fail-closed-on-total-loss, Risk-3).
    let mut supervised: Vec<SupervisedHandle> = vec![
        ("network", plane.net_handle),
        // ONE entry for the whole beacon: which of its children died is in the
        // line the beacon's own supervisor writes, and aborting this aborts them.
        ("beacon", plane.beacon_supervised),
        ("poller", plane.poller_handle),
        ("frontier_resolver", plane.frontier_resolver_handle),
        ("evidence", plane.evidence_handle),
    ];
    for h in plane.mux_handles {
        supervised.push(("mux", h));
    }
    if let Some(h) = inlet_handle {
        supervised.push(("inlet", h));
    }
    // Plus the layer-internal tasks spawned below the crate boundary
    // (`epoch_bridge` in the consensus crate, `cert_feed` in `launch_dpos_layer`):
    // detaching them made a panic in either invisible — committee rotation or the
    // RPC feed dead with the node still passing liveness. See
    // [`DposLayerHandle::supervised`].
    supervised.append(&mut handle.supervised);
    // Every journal writer the beacon owns, behind ONE drain handle. A DRAIN
    // handle and not a supervised one: it returns when the stores' last senders
    // drop, which only happens at shutdown, and its last act is the fsync that
    // makes the newest artifact and the newest journal tails survive the restart.
    handle
        .drain_on_shutdown
        .push(("beacon", plane.beacon_drain));

    // Frontier-resolver keepalive: the always-on plane frontier resolver
    // (`FRONTIER_CHANNEL`) is a `commonware_resolver::p2p::Engine` whose event
    // loop exits the instant its LAST client `PlaneUpstreamHandle` (mailbox
    // sender) drops — it logs "mailbox closed" and the supervisor tears the node
    // down. A Ws-mode validator (`--dpos.follower-upstream`) never stores a
    // `PlaneUpstreamHandle` (it syncs over WS), and a plane-mode validator holds
    // one only transitively via the executor's `ReJumpFn`; neither is a
    // guaranteed node-lifetime owner. But EVERY validator must keep SERVING the
    // frontier channel so a plane-native joiner can deep-sync off it. Hold one
    // clone in a parked task supervised for the node's lifetime (aborted with the
    // rest at shutdown) so the engine's serving side never collapses.
    let frontier_keepalive = plane.plane_upstream.clone();
    let keepalive_handle =
        keepalive_ctx
            .with_label("frontier_keepalive")
            .spawn(move |_| async move {
                let _hold = frontier_keepalive;
                std::future::pending::<()>().await;
            });
    supervised.push(("frontier_keepalive", keepalive_handle));

    // THE HOST MUST NOT OUTLIVE ITS OWN PROVIDER CLONE. All THREE journal writers
    // drain by observing their last sender drop, and all three senders live inside
    // the `Arc<dyn Beacon>` that `plane.shared` holds — the seed store, the key
    // store and, since the artifact store moved in with it, the artifact writer
    // too. Every OTHER strong clone dies with a task the supervisor aborts (the
    // RPC feed holds a `Weak` on purpose); this one is owned by a plain local, so
    // nothing aborts it. Explicit rather than left to scope exit: `plane` is
    // partially moved above, so "it drops at the end of the function" is a fact
    // about this function's body that the next edit can silently change — and the
    // symptom would be three drains burning their timeout behind a warning that
    // says "the disk is stuck". That exact failure is what `91a2f1d9` fixed for
    // one writer; there are three behind one handle now.
    drop(plane.shared);

    Ok((handle, supervised))
}

/// DEVNET-ONLY metrics endpoint (feature-gated so prod binaries can't serve
/// it). Spawned on a child of the commonware runtime context; children share
/// the runtime's prometheus registry, so `c.encode()` includes every family
/// registered on any context of this runtime — the p2p tracker
/// `connected`/`tracked` gauges the overlay registers later (the smoke
/// `smoke-peers` scrapes them), and on a follower the `dpos_follower_artifact_*`
/// families `beacon::build_follower` registers.
///
/// Called from [`run_node_stack`] BEFORE the overlay branch, for both node
/// classes: it binds one socket, so it must run exactly once per process, and a
/// follower that never reached it was the only node class moving the
/// follower-artifact counters with no registry to scrape them from.
pub(crate) fn spawn_devnet_metrics(ctx: &Context, metrics_port: Option<u16>) {
    #[cfg(feature = "dpos-devnet-metrics")]
    if let Some(port) = metrics_port {
        warn!(
            port,
            "DEVNET: serving commonware consensus metrics over HTTP (do not enable in prod)"
        );
        // LOUD-EXIT, deliberately NOT supervised. `serve_metrics` returns on a
        // bind failure (busy port) as well as on a dead accept loop; registering
        // this handle with `supervise` would turn "devnet metrics port taken"
        // into "node exits", a behaviour change on the one task whose death
        // costs no consensus. So the exit is only made audible — a panic under
        // `with_catch_panics(true)` is otherwise a single anonymous line and the
        // scrape just goes quiet forever. No counter either: the only consumer
        // of this process's registry is the endpoint that just died, so a metric
        // here would be unscrapeable by construction; the log line is the signal.
        drop(ctx.with_label("metrics_http").spawn(move |c| async move {
            // Catch the unwind HERE (same idiom as the boundary re-poke loop): a
            // panic inside `serve_metrics` would otherwise skip the exit log and
            // leave only the runtime's anonymous "task panicked" line.
            use futures::FutureExt as _;
            let exit = std::panic::AssertUnwindSafe(serve_metrics(c, port))
                .catch_unwind()
                .await;
            error!(
                port,
                panicked = exit.is_err(),
                "DEVNET: metrics_http task exited (bind failure, dead accept loop, or a panic); \
                 the metrics endpoint is dead for the rest of this process"
            );
        }));
    }
    #[cfg(not(feature = "dpos-devnet-metrics"))]
    if metrics_port.is_some() {
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
pub(crate) struct BeaconPlane<Provider, EvmConfig> {
    /// The single network's start handle — aborted ONLY at process shutdown.
    pub net_handle: Handle<()>,
    /// The beacon's own supervisor: the DKG actor, the beacon-log resolver
    /// engine, the quarantine promoter, the agreement launcher, the agreement
    /// write-back and the wake-up bridge, behind ONE handle. Resolving means one
    /// of them died; aborting it aborts all of them.
    pub beacon_supervised: Handle<()>,
    /// The finalized-height poller driving the plane's ET + `dkg_height` clock.
    /// The poller owns its own `dkg_height_tx` clone (feeding the LOCAL
    /// ordering-finalized height `fin + K`); the live-cert-frontier tee is
    /// re-homed onto the cert-inlet (`live_height` + `dkg_height_tx` below),
    /// fed only on an upstream-configured node.
    pub poller_handle: Handle<()>,
    /// The plane-native `CertUpstream` frontier resolver engine
    /// (`commonware_resolver::p2p`) on FRONTIER_CHANNEL — aborted ONLY at process
    /// shutdown (it serves peers' tip/by-height fetches and drives this node's own
    /// `get_latest`/`get_finalization` for the whole process).
    pub frontier_resolver_handle: Handle<()>,
    /// The evidence-gossip task on EVIDENCE_CHANNEL — aborted ONLY at process
    /// shutdown. It owns both channel halves for the life of the process: peers'
    /// republished votes come in through it and this node's own go out through
    /// it.
    pub evidence_handle: Handle<()>,
    /// The slasher's end of that task. Threaded into the consensus layer, which
    /// binds its slasher mailbox into it at launch.
    pub evidence: fluentbase_consensus::slasher::EvidenceBridge,
    /// The plane-native `CertUpstream` client handle. Threaded into the validator
    /// overlay as the `U: CertUpstream` when no `--dpos.follower-upstream` is set, so a
    /// plain `--dpos` validator runs the jump / Hybrid backfill plane-natively.
    pub plane_upstream: fluentbase_consensus::PlaneUpstreamHandle<Context>,
    /// The 5 persistent non-beacon `Muxer` broker tasks (vote/cert/resolver/
    /// broadcast/marshal), the vote-backup forwarder, and the four route-miss
    /// observers — aborted ONLY at process shutdown (they outlive every
    /// per-promotion signer engine).
    pub mux_handles: Vec<Handle<()>>,
    /// Every journal writer the beacon owns — the artifact store's, the key
    /// journal's and the seed journal's — behind ONE drain handle. Resolving means
    /// all of them have flushed and returned.
    ///
    /// A DRAIN handle, not a supervised one, and the distinction is the whole
    /// reason the two lists never merge: a supervised handle resolving means "a
    /// subsystem died, take the node down", this one means "the work owed is
    /// done, shutdown may proceed". The senders it waits on live inside the
    /// `Arc<dyn Beacon>`, which is why the explicit `drop(plane.shared)` below
    /// has to happen first.
    pub beacon_drain: Handle<()>,
    /// Supervisor handles of the agreement instances the launcher starts, for
    /// `epoch_manager` to adopt so they prune on the engine cutoff. Move-only, so
    /// it is handed over exactly once — into `launch_dpos_layer`.
    pub agreement_intake: mpsc::Receiver<(commonware_consensus::types::Epoch, Handle<()>)>,
    /// Shared store + committee resolver + Oracle + metrics + the 5 `MuxHandle`s +
    /// the vote-backup forwarder, CLONED into the signer engine per promotion (the
    /// engine never re-builds any of these or re-binds the network).
    pub shared: SharedBeaconPlane,
    /// Serve one held epoch-key artifact over `consensus_getEpochArtifact`, as a
    /// closure over `Beacon::artifact_bytes` — the artifact store itself stays
    /// behind the beacon.
    pub artifact_bytes: crate::consensus_rpc::state::ArtifactSource,
    /// THE node's ordering-finalized cursor, created with the plane because the
    /// committee module anchors on it. `launch_dpos_layer` builds its
    /// [`ProviderExecutedChain`](crate::ordering::ProviderExecutedChain) over
    /// this same cursor, so the executor advances exactly the height the
    /// committee reads at — one cursor, not two.
    pub finalized_cursor: fluentbase_consensus::FinalizedCursor,
    /// Every per-epoch committee read in the process, as one frozen record per
    /// epoch at one anchor. Cloned into the layer launch (the slasher, and the
    /// executor's "the anchor moved" wake-up) and already backing the beacon's
    /// `CommitteeReads` facade above.
    pub committee: Arc<dyn fluentbase_consensus::Committee>,
    /// The cert-inlet's live upstream frontier tee. Written only by an
    /// upstream-configured validator's inlet and read by nothing on this path
    /// any more — the committee module reads at the ordering-finalized anchor.
    /// Removed with `upstream_frontier` in the frontier step.
    pub live_height: Arc<std::sync::atomic::AtomicU64>,
    /// `T` — the epoch this node last handed to `track`
    /// (`EpochTransition::last_tracked_epoch`), mirrored into ONE cell with ONE
    /// reader: the executor's frontier probe, which names the ladder step
    /// `Finalized{last(T+1))}` and its addressees and hands both to the marshal
    /// (`executor.rs:656`, read at `:2099-2103`). `PlaneUpstreamHandle` does NOT
    /// read it — its fetches are untargeted, and `plane_upstream.rs` records why
    /// re-deriving targets there starves the contiguous catch-up. Created here
    /// because the handle below is built here, WRITTEN by the layer's
    /// boundary-bridge forwarder — the one place that sees exactly the epochs the
    /// transition tracked. `u64::MAX` is the "nothing tracked yet" sentinel.
    pub tracked_epoch: Arc<std::sync::atomic::AtomicU64>,
    /// The DkgActor deal clock. The inlet ALSO tees the live frontier here so a
    /// still-catching-up early-joiner deals its first epoch's DKG share at the
    /// live tip (the vrf-rotation early-join fix), not K blocks late. The
    /// finalized poller feeds it `fin + K`; the DkgActor `on_height` clamps both
    /// feeders to its running max (never rewound).
    pub dkg_height_tx: mpsc::Sender<u64>,
    /// Deferred marshal READ handle for the plane-native frontier resolver, which
    /// serves peers this node's LOCAL marshal tip/archive. Created EMPTY here (the
    /// marshal mailbox does not exist until the later layer launch);
    /// `run_dpos_stack` fills it with `handle.cert_mailbox` after
    /// `launch_dpos_layer`, at which point the handler starts answering. A
    /// `OnceLock` — set exactly once, read-only thereafter (no lock on the read
    /// path).
    pub marshal_slot: Arc<std::sync::OnceLock<fluentbase_consensus::MarshalMailbox>>,
    /// THE process's `EpochTransition` and the receiving half of the boundary bridge
    /// it was built with. Built HERE — before the engine — because the geometry it
    /// freezes is what the `DkgActor` and the committee module read, and handed DOWN
    /// to the layer launch, which owns the two things only it has: the per-block
    /// delivery driver and the executor's read-floor seam. The plane keeps the cold
    /// start (its poller runs it until the geometry freezes) and nothing else: a
    /// coalescing watch may not drive epoch boundaries, because boundary detection is
    /// pointwise and a coalesced step skips them outright.
    pub epoch_transition: fluentbase_consensus::dpos::PlaneEpochTransition<Provider, EvmConfig>,
}

/// A frame the muxer could not route: the RAW wire sub-channel id and the message.
/// Mirrors commonware's `BackupResponse` — the id is a `u64` off the wire, not an
/// epoch.
type RouteMissFrame = (u64, (PeerPubkey, IoBuf));

/// Route misses, split by the top-level channel they arrived on and by which id
/// space the unrouted sub-channel belongs to: `agreement` for the epoch-key
/// agreement slice (`DKG_SUBCHANNEL_BASE | E`), `unknown` for the epoch space.
///
/// `channel="vote"` only ever pairs with `kind="agreement"`: an unrouted id that
/// IS in the epoch space is forwarded as a corroboration attempt rather than
/// counted, so the vote path reaches [`record_route_miss`] only for ids outside
/// it. The four observer channels emit both kinds.
const ROUTE_MISS_TOTAL: &str = "dpos_subchannel_route_miss_total";

const VOTE_LABEL: &str = "vote";
const CERT_LABEL: &str = "cert";
const RESOLVER_LABEL: &str = "resolver";
const BROADCAST_LABEL: &str = "broadcast";
const MARSHAL_LABEL: &str = "marshal";

/// Count one route miss, logging at a power-of-two rate limit. A member that is
/// still broadcasting into an agreement instance the rest of the committee has
/// already torn down emits these by design, so the log has to be bounded rather
/// than per-frame. `seen` is the calling loop's own tally — the limit is per task.
fn record_route_miss(channel: &'static str, subchannel: u64, seen: &mut u64) {
    let kind = if epoch_from_subchannel(subchannel).is_some() {
        "unknown"
    } else {
        "agreement"
    };
    metrics::counter!(ROUTE_MISS_TOTAL, "channel" => channel, "kind" => kind).increment(1);
    *seen += 1;
    if seen.is_power_of_two() {
        warn!(
            channel,
            subchannel,
            kind,
            seen = *seen,
            "sub-channel route miss: frame dropped"
        );
    }
}

/// Drain a Muxer's backup channel into the counter and NOTHING else. The frame
/// stays dropped: commonware's backup channel is the only hook that makes an
/// unrouted frame observable, so attaching one changes what is visible, not what
/// is routed. Nothing downstream of consensus reads this.
async fn observe_route_misses(channel: &'static str, mut rx: mpsc::Receiver<RouteMissFrame>) {
    let mut seen = 0u64;
    while let Some((subchannel, _)) = rx.recv().await {
        record_route_miss(channel, subchannel, &mut seen);
    }
}

/// Drain the vote Muxer's backup channel into the CURRENTLY-active `EpochManager`.
///
/// The id shares its `u64` with the epoch-key agreement slice, so it is classified
/// here before it can become an [`Epoch`] — this is the ingress that makes
/// `VoteBackupItem`'s `Epoch` true. An agreement id is dropped and counted, and
/// its sender is neither disconnected nor penalised: honest committee members
/// produce those frames by design while an instance tears down.
async fn forward_vote_backup(
    mut rx: mpsc::Receiver<RouteMissFrame>,
    slot: Arc<Mutex<Option<mpsc::Sender<VoteBackupItem>>>>,
) {
    let mut seen = 0u64;
    while let Some((subchannel, msg)) = rx.recv().await {
        let Some(epoch) = epoch_from_subchannel(subchannel) else {
            record_route_miss(VOTE_LABEL, subchannel, &mut seen);
            continue;
        };
        let guard = slot.lock().await;
        if let Some(tx) = guard.as_ref() {
            let _ = tx.try_send((Epoch::new(epoch), msg));
        }
    }
}

/// Build the always-on beacon plane for a registered `--dpos` validator. Mirrors
/// the network/EpochTransition/DkgActor construction that used to live inside
/// `DposLayer::launch`, lifted UP so it persists across the consensus role switch.
///
/// The beacon proper is built by [`fluentbase_consensus::beacon::build`]; what is
/// assembled here is everything around it — the one `FluentP2P`, the 5 persistent
/// `Muxer` brokers, the finalized-height poller driving the EpochTransition, the
/// evidence gossip, the frontier resolver, and the staking-state closures the
/// beacon takes as inputs.
pub(crate) async fn build_beacon_plane<N, AddOns>(
    ctx: &Context,
    node: &FullNode<N, AddOns>,
    cfg: &DposConfig,
    share_seal_key: Option<fluentbase_bls::ShareSealKey>,
    bls_keypair: fluentbase_bls::keys::ValidatorBlsKeypair,
) -> eyre::Result<BeaconPlane<<N as FullNodeTypes>::Provider, <N as FullNodeComponents>::Evm>>
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
    let bootstrappers = match classify_spec(&cfg.bootstrappers) {
        BootstrapperSpec::Dns(domain) => load_from_dns(domain)
            .await
            .wrap_err_with(|| format!("failed loading bootstrappers from DNS {domain:?}"))?,
        BootstrapperSpec::JsonPath(path) => load_from_json_path(path)
            .wrap_err_with(|| format!("failed loading bootstrappers from {path}"))?,
    };

    // The ONE network: build + start ONCE. The beacon halves go to the DkgActor;
    // the non-beacon halves are handed down to the (first) signer engine; the oracle
    // (the only Clone handle) is shared by the plane's ET and the engine's blocker.
    let listen = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), cfg.p2p_port);
    let dialable = match cfg.dialable.as_deref() {
        Some(s) => fluentbase_p2p::parse_ingress(s)
            .wrap_err_with(|| format!("invalid --dpos.dialable {s:?}"))?,
        None => Ingress::Socket(listen),
    };
    let (p2p, handles) = FluentP2P::build(
        ctx.clone(),
        FluentP2PConfig {
            crypto: peer_keypair.clone(),
            chain_id,
            listen,
            dialable,
            bootstrappers,
        },
    );
    let net_handle = p2p.start();

    // The 5 persistent non-beacon channel Muxers. The plane owns these brokers for
    // the whole process; each promotion's signer engine CLONES the `MuxHandle`s and
    // registers per-epoch (vote/cert/resolver) / subchannel-0 (broadcast/marshal)
    // sub-channels. A demoted engine drops its `SubReceiver`s (auto-deregister); the
    // next promotion re-registers against the SAME brokers — restart-free.
    //
    // The vote Muxer's backup channel carries catch-up hints; the plane classifies
    // and forwards them to the currently-active engine via a re-settable forwarder
    // (`vote_backup`). The other four take a backup channel purely as OBSERVERS:
    // cert/resolver/broadcast also carry the agreement plane's per-instance
    // sub-channels, and marshal's route 0 is registered by the signer engine and
    // dies with it — a validator rotated out of the committee still sits in its
    // peers' tracked set (registry ∪ committee ∪ committee+1), so their marshal
    // frames keep arriving at a broker with no route 0. The backup channel is the
    // only hook commonware exposes for seeing any of that.
    //
    // The one mux with no observer is the `--cert-follow` follower's broadcast mux
    // (`cert_follow`): that follower mints an ephemeral identity with no
    // bootstrappers and is in nobody's tracked peer set, so no agreement-plane
    // frame ever reaches it.
    let mux_mailbox = 256usize;
    let (mux_vote, vote_mux, vote_backup_rx) = Muxer::builder(
        ctx.with_label("plane_vote_mux"),
        handles.vote_sender,
        handles.vote_receiver,
        mux_mailbox,
    )
    .with_backup()
    .build();
    let (mux_cert, cert_mux, cert_backup_rx) = Muxer::builder(
        ctx.with_label("plane_cert_mux"),
        handles.cert_sender,
        handles.cert_receiver,
        mux_mailbox,
    )
    .with_backup()
    .build();
    let (mux_res, resolver_mux, resolver_backup_rx) = Muxer::builder(
        ctx.with_label("plane_resolver_mux"),
        handles.resolver_sender,
        handles.resolver_receiver,
        mux_mailbox,
    )
    .with_backup()
    .build();
    let (mux_bcast, broadcast_mux, broadcast_backup_rx) = Muxer::builder(
        ctx.with_label("plane_broadcast_mux"),
        handles.broadcast_sender,
        handles.broadcast_receiver,
        mux_mailbox,
    )
    .with_backup()
    .build();
    let (mux_marshal, marshal_mux, marshal_backup_rx) = Muxer::builder(
        ctx.with_label("plane_marshal_mux"),
        handles.marshal_sender,
        handles.marshal_receiver,
        mux_mailbox,
    )
    .with_backup()
    .build();
    // Share each MuxHandle behind Arc<Mutex> right here (the per-promotion Clone +
    // the transient register-lock; the move-only DiscReceiver makes the bare
    // MuxHandle un-Clone-able). Done at construction rather than at the return
    // because the epoch-key agreement launcher below registers its own
    // sub-channels against the same four brokers.
    let vote_mux = Arc::new(Mutex::new(vote_mux));
    let cert_mux = Arc::new(Mutex::new(cert_mux));
    let resolver_mux = Arc::new(Mutex::new(resolver_mux));
    let broadcast_mux = Arc::new(Mutex::new(broadcast_mux));
    let marshal_mux = Arc::new(Mutex::new(marshal_mux));

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
    for (label, task, rx) in [
        (CERT_LABEL, "plane_cert_route_miss", cert_backup_rx),
        (
            RESOLVER_LABEL,
            "plane_resolver_route_miss",
            resolver_backup_rx,
        ),
        (
            BROADCAST_LABEL,
            "plane_broadcast_route_miss",
            broadcast_backup_rx,
        ),
        (MARSHAL_LABEL, "plane_marshal_route_miss", marshal_backup_rx),
    ] {
        mux_handles.push(
            ctx.with_label(task)
                .spawn(move |_| observe_route_misses(label, rx)),
        );
    }

    // Vote-backup re-settable forwarder: the plane owns the move-only backup
    // receiver and re-broadcasts each catch-up item to the CURRENTLY-active
    // EpochManager (`subscribe()`d fresh per promotion). While no engine is up the
    // parked sender is `None`/closed and items are dropped — a follower needs no
    // catch-up hint.
    let vote_backup: ResettableForward<VoteBackupItem> = ResettableForward::new(mux_mailbox);
    mux_handles.push({
        let slot = vote_backup.slot();
        ctx.with_label("plane_vote_backup_fwd")
            .spawn(move |_| forward_vote_backup(vote_backup_rx, slot))
    });

    // The per-epoch DKG share files, and the agreement artifacts persisted beside
    // them. Reloaded ONCE, inside the beacon build.
    let beacon_dir = node.data_dir.data_dir().join("beacon");

    // Live cursor (consensus-finalized ≈ EL-finalized + K). The cert-inlet (on
    // upstream-configured nodes) tees the live upstream cert frontier here. It is
    // NO LONGER a committee-read cursor: the committee module reads at this
    // node's own ordering-finalized anchor and at nothing else, so this atomic is
    // now write-only on this path — kept because the inlet still writes it and
    // 4.2 removes it together with `upstream_frontier`.
    let live_height = Arc::new(std::sync::atomic::AtomicU64::new(0));

    // Under the 2-epoch committee warm-up the DKG ceremony roster IS the committed
    // slot: `committee[target]` is frozen a full epoch before its DKG runs (at its
    // `target−2` selection block), so the single `committee` read above feeds both
    // the verify/consensus paths AND the DkgActor (ceremony roster + the `next`
    // side of `maybe_start`'s change-test + the AM5 idx→pubkey mapping). The former
    // candidate/stash reader and the separate `active_committee_for` are gone; the
    // DkgActor's default `active_committee_for` covers the `cur` side of the
    // change-test against the same committed slot.

    // The `dkg_height` clock and the epoch-geometry freeze, both fed by a persistent
    // finalized-height poller (reth `finalized_block_number`) — a source that exists
    // in BOTH the follower and signer phases, unlike the per-engine boundary hook.
    // The Oracle peer set is NOT fed from here any more: `oracle.track` has exactly
    // one production caller (`EpochTransition::track_and_trigger`), reached from the
    // bootstrap the layer's cold start owns and from the boundary walk the delivery
    // hook drives. The poller cannot have either without becoming a second
    // bootstrapper on the wrong height scale.
    //
    // Committee members observed slashed for equivocation. Created here, filled by
    // the finalized-height poller below (the sole writer) and published on
    // `SharedBeaconPlane` so every promoted engine's `FluentApp` reads the writer's
    // own handle. Empty at process start and refilled from chain state on the first
    // poll — which is exactly why the reaction survives a restart, where a list of
    // who this node personally caught misbehaving would not.
    let tombstones = fluentbase_consensus::slasher::TombstoneSet::default();
    let (dkg_height_tx, dkg_height_rx) = mpsc::channel::<u64>(256);
    let et_reader = RethStakingStateReader::new(
        node.provider.clone(),
        node.evm_config.clone(),
        staking_config.clone(),
    );
    let provider_for_et = node.provider.clone();
    // Boundary bridge, built HERE so the transition carries its sender from
    // construction: the receiving half rides down to the layer launch, where
    // `OuterEngine::boundary_sender()` exists and the `epoch_bridge` forwarder drains
    // it. 64 slots, as the layer's own channel was — the cold start queues the
    // starting epoch on it long before the forwarder exists, and the buffer is what
    // holds it until then.
    let (bridge_tx, bridge_rx) =
        mpsc::channel::<(u64, fluentbase_staking_reader::reader::ValidatorSetSnapshot)>(64);
    // THE process's `EpochTransition` — one instance, one `last_tracked_epoch`, one
    // `anchor_height`, one `oracle.track` per epoch. Built here because the geometry it
    // freezes is the plane's single in-plane epoch-geometry source (the `DkgActor` and
    // the committee module both read it) and that has to exist before the engine.
    let epoch_transition = EpochTransition::new(
        et_reader,
        handles.oracle.clone(),
        fluentbase_p2p::constants::MAX_REGISTRY_PEER_SET as usize,
        Some(bridge_tx),
        Arc::new(move |n| fluentbase_consensus::executed_state_hash(&provider_for_et, n)),
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
    // The plane's frozen `(dpos_activation, epoch_interval)`, published by the
    // poller the instant the ET freezes it — the EpochTransition is the single
    // in-plane geometry source, and the DkgActor takes it from here rather than
    // re-reading the chain.
    //
    // A WATCH, where this used to be a one-shot `Notify` the actor's spawn wrapper
    // awaited before reading `frozen_geometry()` once: a wake-up that raced the
    // freeze read `None`, logged, and left the node with no `DkgActor` for the life
    // of the process. A watch cannot lose the value — the actor waits for the first
    // `Some` and starts then — and `None` is a named state on the beacon side
    // (`WithheldReason::GeometryUnfrozen`) rather than a fatal read.
    let (geometry_tx, geometry_rx) = tokio::sync::watch::channel(None);

    // THE node's ordering-finalized cursor, created ONCE and here — before both
    // of the things that need it. The executor advances it (through
    // `ProviderExecutedChain`, built with this very cursor in
    // `launch_dpos_layer`) and the committee module anchors every staking read
    // on it. A second cursor is the defect `crate::ordering::
    // ProviderExecutedChain::with_cursor` exists to make unspellable.
    let finalized_cursor = fluentbase_consensus::FinalizedCursor::default();

    // EVERY per-epoch committee read this process makes, as ONE frozen value
    // per epoch, read at ONE anchor: `executed_state_hash(ordering_finalized)`.
    // The four closures and the `PlaneCommitteeReads` cursor
    // (`max(EL-finalized, live)` over a HEADER probe) are gone; what the beacon,
    // the evidence gate and the slasher see is one map with one retention.
    //
    // The verify-only scheme of every epoch is built by the store too, from the
    // record it just installed — the ONE producer, where there used to be four.
    // `beacon_slot` is filled the moment `beacon::build` returns below; it has to
    // be a slot rather than a value because the beacon is constructed FROM this
    // store (it takes the facade), so a value is impossible by build order.
    let beacon_slot: fluentbase_consensus::BeaconSlot = Arc::new(std::sync::OnceLock::new());
    let committee: Arc<dyn fluentbase_consensus::Committee> =
        Arc::new(fluentbase_consensus::CommitteeStore::new(
            RethStakingStateReader::new(
                node.provider.clone(),
                node.evm_config.clone(),
                staking_config.clone(),
            ),
            Arc::new(fluentbase_consensus::RethAnchor::new(
                finalized_cursor.clone(),
                node.provider.clone(),
            )),
            geometry_rx.clone(),
            fluentbase_consensus::epoch_verifier(chain_id, beacon_slot.clone()),
        ));
    // The beacon still speaks `CommitteeReads`; the facade is that surface
    // answered from the module, holding no cursor of its own.
    let committees: Arc<dyn fluentbase_consensus::beacon::CommitteeReads> = Arc::new(
        fluentbase_consensus::CommitteeReadsFacade::new(committee.clone()),
    );

    // Finalized-height poller. It feeds the `dkg_height` clock `fin + K`
    // (ORDERING-finalized): the executor sets the EL-finalized height =
    // `result_final_height(tip, floor) = ordering_finalized − K`
    // (`order_block.rs::result_final_height`, `K = fluentbase_consensus::K`), so
    // `fin + K` is the ordering-finalized height that produced `fin`. The DkgActor's
    // seal deadline + `epoch_start` geometry are ORDERING-chain quantities; feeding it
    // the raw EL-finalized `fin` would silently shorten the `DKG_MARGIN_BLOCKS` window
    // by K (the epoch-2 boundary wedge). For every epoch ≥ 1 the cold-start floor
    // clamp is inactive, so `fin + K` == the ordering tip exactly.
    //
    // It also drives the transition's GEOMETRY FREEZE until it takes — and nothing
    // after that, and nothing else ever: the epoch bootstrap (the starting epoch on
    // the bridge, the read floor) belongs to the layer's cold start, which owns an
    // ordering-scale post-jump anchor. It may not drive boundaries: `borrow_and_update` below takes
    // the newest finalized height and drops the ones in between, while boundary
    // detection is pointwise (`is_epoch_boundary(number)`), so a coalesced step over a
    // terminal height loses the epoch enter outright — no track, no bridge trigger, and
    // not even a parked boundary, since the park replays a boundary that WAS detected
    // (`staking-reader`'s `a_coalesced_driver_skips_the_boundary_a_stepping_one_enters`).
    // The driver that cannot skip is the layer's delivery hook, which fires on every
    // ordering-finalized block, and it owns the one transition. Nothing is lost here:
    // every boundary this poller could have seen the hook sees, one height at a time
    // and in the ordering scale the transition's `read_height_for` expects.
    //
    // The clock pair is REGISTERED here, where the registry is, but neither gauge
    // is written here any more: the `DkgActor` publishes the DKG half off the
    // clamp that merges this feeder with the cert inlet's frontier and marshal's
    // ordering tip, and `FluentApp` publishes the ordering half off that same tip.
    // This task keeps only the drop counter for the sends it fails to place.
    let plane_clock = fluentbase_consensus::sync_metrics::PlaneClock::default();
    plane_clock.register(ctx);
    let poller_handle = {
        let provider = node.provider.clone();
        let et = et_arc.clone();
        let dkg_tx = dkg_height_tx.clone();
        let plane_clock = plane_clock.clone();
        let geometry_tx = geometry_tx.clone();
        // The committee module, for ONE call: the freeze below is the second of
        // the two events `Committee::subscribe` promises (the other being the
        // anchor advance the executor drives). Before it every epoch answers
        // `NotReadable { ready_at: 0 }`, so a consumer parked on the watch would
        // sleep through the moment the whole window became readable.
        let committee_wake = committee.clone();
        let tombstones = tombstones.clone();
        let tombstone_reader = RethStakingStateReader::new(
            node.provider.clone(),
            node.evm_config.clone(),
            staking_config.clone(),
        );
        let mut blocker = handles.oracle.clone();
        let me = peer_keypair.public_key();
        ctx.with_label("beacon_plane_poller")
            .spawn(move |_c| async move {
                // Event-driven, not a poll: reth publishes a watch on the finalized
                // marker. `borrow_and_update` MARKS the current value seen, so the
                // `changed()` at the bottom waits for the NEXT change instead of
                // returning immediately and re-processing the same height. Taking
                // the value here rather than awaiting a change first also keeps the
                // persisted-marker-surfacing race closed (see the `cs_fin_num` seed
                // below): if reth has already surfaced the marker we act on it now.
                //
                // The gauge is NOT written here. The DkgActor writes it off the
                // clamp where all three feeders meet, so a node whose cert inlet or
                // ordering tip runs ahead of `fin + K` no longer publishes a clock
                // lower than the one it runs on.
                let mut finalized_rx = provider.canonical_state().subscribe_finalized_block();
                let mut sent = cs_fin_num;
                // Publish-once latch for the frozen geometry. Separate from "did I
                // freeze it", because the freeze has two possible authors and the
                // publication has exactly one.
                let mut geometry_published = false;
                // Track-once latch for the FIRST peer set (see the block below). One
                // success is the whole contract: from there the epoch machine owns
                // every later registration, at the boundaries.
                let mut peers_tracked = false;
                let _ = dkg_tx.try_send(cs_fin_num + fluentbase_consensus::K);
                loop {
                    // Bound out of the `match` scrutinee so the watch guard is
                    // dropped before the `changed()` await below — a `watch::Ref`
                    // held across an await makes the whole task non-`Send`.
                    let latest = finalized_rx.borrow_and_update().as_ref().map(|h| h.number);
                    let fin = match latest {
                        Some(n) => n,
                        None => {
                            if finalized_rx.changed().await.is_err() {
                                // The provider is gone: the node is shutting down.
                                break;
                            }
                            continue;
                        }
                    };
                    // Coalesced: ONE message carrying the newest height, not one per
                    // missed height. `on_height` clamps to a running max at entry
                    // (`beacon/actor.rs`), so every intermediate tick is discarded by
                    // the consumer anyway — sending them made a catch-up run the whole
                    // `on_height` body hundreds of times, each with uncached committee
                    // reads, and overflowed this 256-slot channel (measured: an EL jump
                    // of 338 blocks dropped 85 ticks in one burst).
                    //
                    // The cursor advances ONLY on a successful send. Advancing it on a
                    // drop is what made the old shape lose a height permanently: on a
                    // restart with a gap wider than the buffer the newest value is the
                    // one that does not fit, and nothing ever re-sent it — convergence
                    // rested on the chain continuing to produce.
                    if sent < fin {
                        if dkg_tx.try_send(fin + fluentbase_consensus::K).is_ok() {
                            sent = fin;
                        } else {
                            plane_clock.note_height_drop();
                        }
                    }
                    // Geometry drive (event-driven on THIS existing poll, no second
                    // timer): until the geometry is frozen, resolve it off the LIVE
                    // finalized cursor — freezing the instant that cursor names a
                    // readable, DPoS-scheduled block (`freeze_geometry` is
                    // codeless-tolerant, so a too-early tick defers and the next
                    // finalized change re-attempts at a fresh height).
                    //
                    // The transition owes this poller exactly two things — the frozen
                    // GEOMETRY here and the FIRST peer-set registration below — plus
                    // the tombstone read further down, which goes to the reader, not to
                    // the transition. It deliberately does not `cold_start` and does
                    // not drive boundaries: the cold start picks the starting epoch
                    // (the one value the epoch manager ever learns, over the bridge) and
                    // the write-once bootstrap gate hands it to whoever calls first,
                    // while this cursor is the EL-finalized height — `K` below the
                    // ordering chain, and far below a re-jump landing. The one
                    // bootstrapper is the layer, on its post-jump ordering anchor
                    // (`consensus/src/dpos.rs` `DposLayer::launch`), and it also raises
                    // the read floor there. `staking-reader`'s
                    // `an_el_scale_bootstrap_in_the_k_window_after_a_boundary_loses_the_epoch`
                    // is what that costs when the EL-scale caller wins.
                    if !geometry_published {
                        let frozen_before = { et.lock().await.frozen_geometry().is_some() };
                        if !frozen_before {
                            // No `finalized_block_hash`-by-number on the provider here,
                            // so resolve the hash from the height we already have. When
                            // the body behind the freshly-finalized marker is not
                            // readable yet, fall through: the tombstone read below is
                            // guarded the same way and degrades to a no-op.
                            if let Ok(Some(hash)) = provider.block_hash(fin) {
                                if let Err(e) = et.lock().await.freeze_geometry(hash) {
                                    warn!(
                                        finalized = fin,
                                        error = ?e,
                                        "beacon plane: ET freeze_geometry failed"
                                    );
                                }
                            }
                        }
                        // Publish on the FACT of the freeze, never on "I was the one who
                        // froze it". The layer's cold start freezes the same instance
                        // through the same path and can get there first (it takes its
                        // anchor from the archive / a jump landing, not from this watch),
                        // and a publication nested inside this poller's own freeze branch
                        // would then never run: the DkgActor parks forever on the first
                        // `Some` (`consensus/src/beacon/plane.rs`), no DKG, no share, and
                        // the share gate demotes the node to verify-only for the life of
                        // the process — with nothing above `debug` anywhere.
                        //
                        // `send_replace` rather than a one-shot signal: the receiver
                        // reads the newest value, so there is no ordering to get wrong
                        // between the freeze and the read.
                        if let Some(frozen) = et.lock().await.frozen_geometry() {
                            geometry_tx.send_replace(Some(frozen));
                            // The store reads the watch on every call, so it is
                            // answering already — but nobody parked on its
                            // wake-up knows that, and the next anchor advance is
                            // a finalized derive away. Publish the ceiling now,
                            // from the anchor the store already holds (the call
                            // takes no height, so it cannot publish one the
                            // anchor does not).
                            committee_wake.anchor_advanced();
                            geometry_published = true;
                        }
                    }

                    // THE FIRST peer-set registration — and the only one this
                    // process can get before its consensus layer exists.
                    //
                    // `DposLayer::launch` runs its cold-start jump loop
                    // (`consensus/src/dpos.rs:1845-1893`) BEFORE it cold-starts the
                    // transition (`:2108-2113`), and an empty-archive validator whose
                    // EL is past epoch 0 cannot leave that loop until a PLANE peer
                    // serves it a frontier. The frontier resolver dials only peers the
                    // Oracle is tracking (`peer_provider: handles.oracle`, the
                    // `commonware_resolver::p2p::Engine` built below this task), and
                    // commonware dials only PRIMARY — i.e. tracked — peers. So the
                    // registration has to happen HERE: this task is spawned inside
                    // `build_beacon_plane`, which the caller awaits BEFORE
                    // `launch_dpos_layer`, and the jump loop waits on `EL_SYNC_TICK`
                    // (2s) between attempts, so a tick of this poller lands inside it.
                    //
                    // `track_peers` is NOT a bootstrap: it moves no bootstrap state,
                    // so the layer's `cold_start` still takes the write-once branch and
                    // still picks the starting epoch off its own post-jump ORDERING
                    // anchor. Re-registering the same epoch index there is a no-op at
                    // the Oracle (a `track` for an already-registered index is
                    // ignored), and a higher one is the ordinary advance.
                    //
                    // STATE-GATED like the tombstone read below, and for the same
                    // reason: the committee snapshot is an EVM read, so the cursor must
                    // be a hash whose state this node has EXECUTED, not merely a header
                    // it has imported.
                    if !peers_tracked {
                        if let Some(at) =
                            fluentbase_consensus::executed_state_hash(&provider, fin)
                                .ok()
                                .flatten()
                        {
                            match et.lock().await.track_peers(at, fin).await {
                                Ok(Some(epoch)) => {
                                    info!(
                                        epoch,
                                        finalized = fin,
                                        "beacon plane: peer set tracked — the plane can dial \
                                         before the layer launches"
                                    );
                                    peers_tracked = true;
                                }
                                // Geometry not frozen yet, or `committee[epoch]` not
                                // readable at this height: retry on the next finalized
                                // change, exactly as the freeze above does.
                                Ok(None) => {}
                                Err(e) => warn!(
                                    finalized = fin,
                                    error = ?e,
                                    "beacon plane: ET track_peers failed; retrying on the \
                                     next finalized change"
                                ),
                            }
                        }
                    }

                    // Tombstone watch — event-driven on THIS existing poll, no
                    // second timer (same discipline as the cold-start drive
                    // above). The committee snapshot now carries each member's
                    // equivocation verdict, so one read arms both reactions: the
                    // refuse-to-bind gate in `FluentApp::verify_block`, and the
                    // transport severance below.
                    //
                    // Severing the transport is the load-bearing half, not
                    // hygiene. The batcher's inactivity rule keys on
                    // `latest_seen`, which `record_activity` refreshes on ANY
                    // accepted message from a participant — before signature
                    // verification and regardless of role — so a slashed member
                    // that merely keeps voting stays "active" indefinitely and
                    // its leader deadline is never collapsed to now. Only
                    // freezing `latest_seen` lets `is_active` go false after
                    // `skip` views, at which point its slots stop costing a
                    // timeout at all.
                    //
                    // This blocks through an `OracleHandle` clone rather than by
                    // arming the wired `NoopBlocker`: `block!` discards its
                    // reason string, so arming that switch would ban a peer on
                    // any verdict — including a transient batch-verify failure —
                    // and a ban is four hours of GLOBAL transport severance.
                    // Here the verdict is on chain and permanent, so the ban is
                    // proportionate; the delta from `observe` is what keeps it to
                    // one call per peer instead of one per tick.
                    //
                    // STATE-GATED: the snapshot is an EVM read, so the cursor
                    // has to be a hash whose STATE this node has executed, not
                    // merely a header it has imported. `block_hash(fin)`
                    // answered a header on a backfilling node and the read then
                    // failed with `StateNotMaterialized` every tick;
                    // `executed_state_hash` answers `Ok(None)` there and the
                    // poller simply skips the tick.
                    let epoch = et.lock().await.epoch_at(fin);
                    let tombstone_at = fluentbase_consensus::executed_state_hash(&provider, fin)
                        .ok()
                        .flatten();
                    if let (Some(epoch), Some(hash)) = (epoch, tombstone_at) {
                        match tombstone_reader.epoch_committee_snapshot(epoch, hash) {
                            Ok(snap) => {
                                for peer in tombstones.observe(&snap) {
                                    // Never sever our own transport: a node that
                                    // has been slashed still has to follow the
                                    // chain, and blocking itself would cut the
                                    // connectivity it needs to do that. The
                                    // consensus consequences of its own tombstone
                                    // are the network's to apply, not its own.
                                    if peer == me {
                                        error!(
                                            epoch,
                                            "this validator is tombstoned for equivocation on chain"
                                        );
                                        continue;
                                    }
                                    warn!(
                                        ?peer,
                                        epoch,
                                        "validator tombstoned for equivocation — severing its transport"
                                    );
                                    blocker.block(peer).await;
                                }
                            }
                            Err(e) => debug!(
                                epoch,
                                error = ?e,
                                "beacon plane: tombstone read failed; retrying on the next poll"
                            ),
                        }
                    }

                    if finalized_rx.changed().await.is_err() {
                        break;
                    }
                }
            })
    };

    // Deferred marshal READ handle for the plane-native frontier resolver: the
    // marshal mailbox is created by the LATER layer launch (`launch_dpos_layer`), so
    // create the slot EMPTY here and fill it in `run_dpos_stack` once the mailbox
    // exists. Until filled the handler serves nothing (safe).
    let marshal_slot: Arc<std::sync::OnceLock<fluentbase_consensus::MarshalMailbox>> =
        Arc::new(std::sync::OnceLock::new());

    // See `BeaconPlane::tracked_epoch`. One cell, one writer (the layer's
    // boundary-bridge forwarder), one reader (the executor's frontier probe).
    let tracked_epoch: Arc<std::sync::atomic::AtomicU64> =
        Arc::new(std::sync::atomic::AtomicU64::new(u64::MAX));

    // Plane-native `CertUpstream` frontier resolver (`commonware_resolver::p2p`) on
    // FRONTIER_CHANNEL — the seam that lets a plain `--dpos` validator run the cold-start
    // / steady-state JUMP without `--dpos.follower-upstream`. The `FrontierHandler` serves
    // peers THIS node's LOCAL marshal tip/archive (late-bound through the SAME
    // `marshal_slot` the DkgActor reads) and resolves this node's own `get_latest` /
    // `get_finalization` awaits; the returned `PlaneUpstreamHandle` is threaded into the
    // validator overlay as the `U: CertUpstream`. `Blocker` is an isolated `NoopBlocker`
    // (NOT the shared oracle): a `deliver=false` on an undecodable tip must never
    // partition a peer from consensus channels (mirrors the beacon log resolver).
    // `committee` is the process's ONE committee module (built above): it is what
    // makes `FrontierHandler::deliver` able to JUDGE an answer — the geometry for
    // the height↔epoch bind and the record for the read window. `chain_id` gives
    // it the certificate namespace, because the 2f+1 multisig is checked under a
    // VERIFY-ONLY scheme `deliver` builds from that record rather than under the
    // module's own scheme, which carries the epoch's seed oracle (see the
    // `plane_upstream` module docs, Д-75).
    let (frontier_handler, frontier_waiters) = fluentbase_consensus::plane_upstream::new_bridge(
        marshal_slot.clone(),
        committee.clone(),
        ctx.clone(),
        chain_id,
    );
    let (frontier_engine, frontier_mailbox) = commonware_resolver::p2p::Engine::new(
        ctx.with_label("frontier_resolver"),
        commonware_resolver::p2p::Config {
            peer_provider: handles.oracle.clone(),
            blocker: fluentbase_p2p::NoopBlocker,
            consumer: frontier_handler.clone(),
            producer: frontier_handler,
            mailbox_size: 256,
            me: Some(peer_keypair.public_key()),
            // Mirror the MARSHAL / beacon-log resolver backfill cadence — a frontier
            // fetch is rare (one per jump attempt / re-jump edge / by-height hole).
            initial: Duration::from_millis(100),
            timeout: Duration::from_secs(5),
            fetch_retry_timeout: Duration::from_millis(500),
            priority_requests: false,
            priority_responses: false,
        },
    );
    let plane_upstream = fluentbase_consensus::PlaneUpstreamHandle::new(
        ctx.clone(),
        frontier_mailbox,
        frontier_waiters,
    );
    let frontier_resolver_handle =
        frontier_engine.start((handles.frontier_sender, handles.frontier_receiver));

    // EVIDENCE plane: the votes a node republishes for a round decided against
    // them, so the two halves of a split-delivered equivocation meet somewhere
    // (`consensus/slasher/gossip.rs`). This task owns BOTH channel halves.
    // Inbound it verifies every forwarded vote against the epoch committee
    // before the slasher's vote store sees it — `Activity::verified()` is false
    // for every vote variant, so a peer can hand us a structurally valid vote
    // carrying a bogus signature under an honest validator's name. Outbound it
    // broadcasts what the slasher chose to publish; the slasher cannot do that
    // itself because the p2p sender lives in this crate.
    let (evidence_bridge, mut evidence_rx) = fluentbase_consensus::slasher::EvidenceBridge::new();
    let evidence_committee_for: fluentbase_consensus::slasher::EvidenceCommitteeFor = {
        // Through the module, with no memo of its own: the module's map IS the
        // cache (one record per epoch, write-once, retained for the whole read
        // window), so the one-slot memo this closure used to keep was a second
        // authoritative copy of one epoch's committee — the defect class the
        // module exists to close — and a strictly worse cache besides.
        let committee = committee.clone();
        Arc::new(move |epoch: u64| committee.committee(epoch).ok().map(|r| r.bls.clone()))
    };
    let evidence_handle = {
        let bridge = evidence_bridge.clone();
        let mut sender = handles.evidence_sender;
        let mut receiver = handles.evidence_receiver;
        ctx.with_label("evidence_gossip").spawn(move |_| async move {
            // Two independent loops, joined rather than `select!`ed: neither
            // feeds the other, so there is no reason to drop a half-polled
            // `recv()` every time the other side fires.
            let inbound = async move {
                loop {
                    match receiver.recv().await {
                        Ok((_from, buf)) => {
                            // The bridge carries both what ingest needs: the
                            // gossip half of the slasher mailbox (absent until
                            // the consensus layer launches) and the epoch cursor
                            // that bounds what a peer may claim.
                            fluentbase_consensus::slasher::gossip::ingest_batch(
                                buf.as_ref(),
                                chain_id,
                                &evidence_committee_for,
                                &bridge,
                            );
                        }
                        Err(e) => {
                            warn!(error = ?e, "evidence channel receiver failed; ingest stopped");
                            break;
                        }
                    }
                }
            };
            let outbound = async move {
                while let Some(batch) = evidence_rx.recv().await {
                    if let Err(e) = sender.send(Recipients::All, batch, true).await {
                        debug!(error = ?e, "evidence publication failed");
                    }
                }
            };
            tokio::join!(inbound, outbound);
        })
    };

    // The beacon proper: the persistent `DkgActor` (committee[E] deals during E-1
    // regardless of this node's current consensus role), the dealer-log + artifact
    // recovery seam, the durable artifact store and the epoch-key agreement
    // launcher. Everything inside it is the beacon module's; what stays here is the
    // network, the mux brokers, the finalized-height poller and the three
    // staking-state closures — the dependencies that run the other way.
    //
    // The actor waits for the poller to publish a frozen `(activation, interval)`
    // on `geometry_rx` and takes it from there — the EpochTransition stays the
    // single in-plane source and the actor never re-reads the chain, so there is no
    // codeless/genesis-fallback race in this path. Height ticks accumulate in
    // `dkg_height_rx` meanwhile (bounded buffer) and are drained by `on_height`'s
    // monotone-max clamp once the actor runs; the first epoch boundary is one
    // interval away (≫ the ~one-tick freeze latency), so no deal/seal is missed.
    let (beacon, beacon_tasks) = fluentbase_consensus::beacon::build(
        ctx,
        fluentbase_consensus::beacon::ValidatorInputs {
            chain_id,
            peer_keypair,
            bls_keypair,
            share_dir: beacon_dir,
            share_seal_key,
            peers: handles.oracle.clone(),
            beacon_channel: (handles.beacon_sender, handles.beacon_receiver),
            resolver_channel: (
                handles.beacon_resolver_sender,
                handles.beacon_resolver_receiver,
            ),
            vote_mux: vote_mux.clone(),
            cert_mux: cert_mux.clone(),
            resolver_mux: resolver_mux.clone(),
            bodies_mux: broadcast_mux.clone(),
            committees,
            heights: dkg_height_rx,
            plane_clock: plane_clock.clone(),
            geometry: geometry_rx,
            partition_prefix: String::new(),
        },
    )
    .await?;

    // The committee module's scheme producer can build now: every epoch it reads
    // from here on gets its verify-only scheme bound to THIS beacon's oracle, and
    // the few epochs it may have read before this line pick theirs up on the next
    // `Committee::scheme`.
    let _ = beacon_slot.set(Arc::downgrade(&beacon));

    // The `dkgQual[e]` bit is now set DETERMINISTICALLY by the contract at
    // `commitEpochCommittee` (= committee[e] != committee[e−1]); there is no
    // permissionless marker tx and no node-side marker relayer to publish here.

    info!(
        listen = %listen,
        "always-on beacon plane built (one FluentP2P, persistent DkgActor; geometry frozen by the plane EpochTransition)"
    );

    // WEAK, and that is a shutdown property rather than a style choice. This
    // closure is handed to the RPC feed (`set_artifact_source`), which lives in
    // reth's module registry for the whole process — nothing aborts it. A strong
    // `Arc<dyn Beacon>` in it would keep the beacon alive past `drop(plane.shared)`
    // and therefore keep the seed, key and artifact journals' SENDERS alive, so
    // every drain writer would sit on a channel that never closes and the node
    // would burn the full `SHUTDOWN_DRAIN_TIMEOUT` on every clean exit behind a
    // warning whose whole job is to mean "the disk is stuck".
    //
    // A failed upgrade answers `None`, which is the right answer: the beacon is
    // gone, so this node holds no artifact to serve.
    let artifact_bytes: crate::consensus_rpc::state::ArtifactSource = {
        let beacon = Arc::downgrade(&beacon);
        Arc::new(move |epoch: u64| beacon.upgrade()?.artifact_bytes(epoch))
    };
    Ok(BeaconPlane {
        net_handle,
        beacon_supervised: beacon_tasks.supervised,
        beacon_drain: beacon_tasks.drain,
        poller_handle,
        frontier_resolver_handle,
        evidence_handle,
        evidence: evidence_bridge,
        plane_upstream,
        mux_handles,
        agreement_intake: beacon_tasks.agreement_intake,
        shared: SharedBeaconPlane {
            oracle: handles.oracle,
            randomness: beacon,
            vote_mux,
            cert_mux,
            resolver_mux,
            broadcast_mux,
            marshal_mux,
            vote_backup,
            tombstones,
            plane_clock,
            dkg_height_tx: dkg_height_tx.clone(),
        },
        artifact_bytes,
        finalized_cursor,
        committee,
        live_height,
        tracked_epoch,
        dkg_height_tx,
        marshal_slot,
        epoch_transition: fluentbase_consensus::dpos::PlaneEpochTransition {
            transition: et_arc,
            bridge_rx,
        },
    })
}

/// The validator's marshal-backfill + jump `CertUpstream`, unified so
/// `DposLayerConfig<_, _, _, U>` keeps ONE concrete `U` regardless of config:
/// [`Plane`](Self::Plane) (plane-native, the NEW default — no WS URL) or
/// [`Ws`](Self::Ws) (the deprecated `--dpos.follower-upstream` escape). Delegates
/// [`CertUpstream`] verbatim, so the jump / `MarshalResolver::Hybrid` / re-jump paths
/// are reused unchanged — only the concrete backend differs.
#[derive(Clone)]
pub(crate) enum ValidatorUpstream {
    Ws(crate::cert_follow::upstream::UpstreamHandle),
    Plane(fluentbase_consensus::PlaneUpstreamHandle<Context>),
}

impl fluentbase_consensus::CertUpstream for ValidatorUpstream {
    fn get_finalization(
        &self,
        height: Height,
    ) -> impl std::future::Future<Output = Option<fluentbase_consensus::UpstreamFinalized>> + Send
    {
        let this = self.clone();
        async move {
            match this {
                Self::Ws(u) => u.get_finalization(height).await,
                Self::Plane(u) => u.get_finalization(height).await,
            }
        }
    }

    fn get_latest(
        &self,
    ) -> impl std::future::Future<Output = Option<fluentbase_consensus::UpstreamFinalized>> + Send
    {
        let this = self.clone();
        async move {
            match this {
                Self::Ws(u) => u.get_latest().await,
                Self::Plane(u) => u.get_latest().await,
            }
        }
    }

    fn rotate(&self) -> impl std::future::Future<Output = ()> + Send {
        let this = self.clone();
        async move {
            match this {
                Self::Ws(u) => u.rotate().await,
                Self::Plane(u) => u.rotate().await,
            }
        }
    }
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
    plane_upstream: fluentbase_consensus::PlaneUpstreamHandle<Context>,
    evidence: fluentbase_consensus::slasher::EvidenceBridge,
    upstream_frontier: std::sync::Arc<std::sync::atomic::AtomicU64>,
    agreement_intake: mpsc::Receiver<(commonware_consensus::types::Epoch, Handle<()>)>,
    // The plane's ONE ordering-finalized cursor and the committee module built
    // over it. The executed-chain view below is constructed with this cursor
    // rather than minting its own, which is what makes "the executor advances
    // the height the committee reads at" a type-level fact.
    finalized_cursor: fluentbase_consensus::FinalizedCursor,
    committee: std::sync::Arc<dyn fluentbase_consensus::Committee>,
    // The `T` cell (see `BeaconPlane::tracked_epoch`): this launch's boundary
    // forwarder is its ONE writer and the executor's frontier probe is its ONE
    // reader.
    tracked_epoch: std::sync::Arc<std::sync::atomic::AtomicU64>,
    // The beacon plane's serving read for `consensus_getEpochArtifact` — wired
    // into the feed beside `set_marshal` below, so a follower can obtain
    // `PK_epoch` from this validator over the SAME namespace it already takes
    // certificates from.
    artifact_bytes: crate::consensus_rpc::state::ArtifactSource,
    // THE process's `EpochTransition` (built by the plane, before the engine) and the
    // receiving half of its boundary bridge. This launch supplies the per-block
    // delivery driver and the executor's read-floor seam; it builds no transition of
    // its own, so a validator runs exactly one.
    epoch_transition: fluentbase_consensus::dpos::PlaneEpochTransition<
        <N as FullNodeTypes>::Provider,
        <N as FullNodeComponents>::Evm,
    >,
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
    let slasher_signer = load_slasher_signer(cfg, chain_id).await?;

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
    // Read-only devp2p peer-count probe for the EL-sync no-peers net
    // (`cold_start_jump::RethElSync`). `node.network` is `FullNetwork: PeersInfo`.
    let peer_count: Arc<dyn Fn() -> usize + Send + Sync> = {
        let net = node.network.clone();
        Arc::new(move || net.num_connected_peers())
    };
    let reth = RethHandle {
        provider: node.provider.clone(),
        evm_config: node.evm_config.clone(),
        beacon_engine_handle: beacon_engine,
        chain_id,
        canonical_state,
        genesis_hash,
        peer_count,
    };

    // Deferred-execution collaborators, all over the node's own provider/EVM:
    // derive (reth-evm BlockBuilder), the derived-chain view, and the
    // pool-backed ordering assembler. The beacon is always-on live-DKG; the
    // deriver computes prev_randao = H(seed) directly from the cert-recovered
    // seed (no on-chain PK_E read — that layer is gone, DPOS_ARCHITECTURE §8.11).
    let deriver =
        crate::derive::RethBlockDeriver::new(node.provider.clone(), node.evm_config.clone());
    let executed = ProviderExecutedChain::with_cursor(node.provider.clone(), finalized_cursor);
    let assembler = Arc::new(PoolAssembler::new(node.pool.clone(), executed.clone()));
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
        // RETIRED with the epoch key's departure from `OrderBlock`: the mode
        // forged the `beacon_outcome` a change-boundary block asserted, and no
        // block asserts a key any more. Fail LOUD rather than run honest — a
        // smoke that asks for a byzantine node and silently gets an honest one
        // false-passes, which is exactly what this match arm exists to prevent.
        Some("forge-beacon-pk") => eyre::bail!(
            "--dpos.byzantine forge-beacon-pk is retired: the epoch key no longer \
             rides `OrderBlock`, so there is no asserted PK_E to forge"
        ),
        Some("equivocate") => {
            tracing::warn!("DEVNET BYZANTINE MODE ACTIVE: equivocate — NEVER use in production");
            Some(fluentbase_consensus::byzantine::ByzantineMode::Equivocate)
        }
        Some(other) => {
            eyre::bail!(
                "unknown --dpos.byzantine mode: {other:?} \
                 (expected `equivocate`)"
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
    let upstream = Some(if cfg.follower_upstreams.is_empty() {
        // Plane-native (NEW default): no WS URL — the frontier resolver built in
        // `build_beacon_plane` discovers the frontier + pulls by-height finalizations
        // over the consensus plane. `upstream.is_some()` now holds for a plain
        // validator, so the cold-start jump + `MarshalResolver::Hybrid` + steady-state
        // re-jump all activate plane-natively.
        ValidatorUpstream::Plane(plane_upstream)
    } else {
        // Deprecated `--dpos.follower-upstream` escape: this WS serves ONLY the marshal's
        // by-height resolver pulls — its live stream + connection-generation token are
        // unused here (the validator's live-stream inlet has its OWN WS actor in
        // `spawn_cert_inlet`).
        let (ws_actor, handle, _live_rx, _conn_gen) =
            crate::cert_follow::upstream::init(ctx.clone(), cfg.follower_upstreams.clone());
        drop(ws_actor.start());
        ValidatorUpstream::Ws(handle)
    });

    let layer_cfg = DposLayerConfig {
        bls_keypair,
        peer_keypair,
        // Every per-epoch committee read the consensus layer makes — the
        // slasher's evidence resolve today, and the executor's
        // "the anchor moved" wake-up. The SAME `Arc` the beacon plane's facade
        // is built over, so the two node-side planes cannot drift.
        committee,
        // The ONE `T` cell — this layer's boundary forwarder writes it; the
        // frontier probe and `PlaneUpstreamHandle` read it.
        tracked_epoch,
        slasher_sink,
        evidence,
        staking_config,
        // Fork-safety halt marker: beside the beacon shares, in the datadir this
        // node's chain state lives in — a halt is a property of THIS disk's view
        // of the chain, so a fresh datadir is (correctly) a fresh start.
        halt_marker: Some(node.data_dir.data_dir().join(SAFETY_HALT_MARKER)),
        upstream,
        deriver,
        executed,
        assembler,
        target_gas_limit,
        feed: feed_sink,
        // Mid-epoch promotion trigger: the executor fires it on each finalized-advance
        // and the EpochManager re-checks parked spawns. Created here, internal to the
        // layer (executor producer + EpochManager consumer share this one Arc).
        // SINGLE-CONSUMER Notify (notify_one): the ONLY awaiter is EpochManager::run
        // (epoch_manager.rs select). A second .notified() consumer would silently swallow the
        // executor's mid-epoch promotion-unblock wakes.
        spawn_unblocked: std::sync::Arc::new(tokio::sync::Notify::new()),
        // The always-on beacon plane (shared store + committee resolver + the single
        // network's oracle + the once-registered metrics + the 5 non-beacon MuxHandles
        // + the vote-backup forwarder). The signer engine CLONES these per promotion;
        // it never rebuilds the network, re-spawns the DkgActor, or consumes a raw
        // channel half — so a demote→re-promote re-clones with no rebuild.
        beacon_plane: shared_beacon,
        // The plane's agreement instances, adopted by `epoch_manager` so they are
        // pruned on the same frontier cutoff as the per-epoch engines.
        agreement_intake: Some(agreement_intake),
        // Rule Y: the SAME atomic the validator inlet tee writes (created once in
        // `launch_validator_overlay`, threaded into both sinks) — the validator's
        // `executor::ReJump` reads it, mirroring the follower's inlet⇄executor signal.
        upstream_frontier,
        #[cfg(feature = "dpos-devnet-byzantine")]
        byzantine,
    };

    // Spawn the cert-feed actor on a child of the runtime context BEFORE `launch`
    // consumes `ctx`. It blocks on the channel until finalizations flow (post-launch),
    // by which point `set_marshal` (below) has run. Keep the handle for `set_marshal`.
    //
    // SUPERVISED, not detached: `FeedActor::run` returns only when the `FeedSink`
    // drops (node shutdown), so ANY earlier exit — a panic, which
    // `with_catch_panics(true)` would otherwise reduce to one log line — means
    // the consensus RPC feed is dead for the rest of the process while the node
    // keeps serving. The handle rides up on [`DposLayerHandle::supervised`] to
    // `supervise`, which treats a clean exit and a panic alike as fatal.
    let (feed_handle, cert_feed_task) = match feed_actor_wiring {
        Some((rx, handle)) => {
            let actor_handle = handle.clone();
            let task = ctx.with_label("cert_feed").spawn(move |_| async move {
                FeedActor::new(rx, actor_handle).run().await;
            });
            (Some(handle), Some(task))
        }
        None => (None, None),
    };

    let mut handle: DposLayerHandle =
        DposLayer::launch(ctx, reth, layer_cfg, epoch_transition, shutdown_token).await?;
    if let Some(task) = cert_feed_task {
        handle.supervised.push(("cert_feed", task));
    }

    // Hand the marshal mailbox to the feed state (node-side, respecting the crate
    // boundary — consensus never names node types). Until this runs the RPC returns
    // ServiceUnavailable; the window is sub-finalization so no event is lost.
    if let Some(fh) = feed_handle {
        fh.set_marshal(handle.cert_mailbox.clone());
        fh.set_artifact_source(artifact_bytes);
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

// Host-only key loading helpers (filesystem syscalls + permission checks)

/// Load the validator BLS keypair AND, IFF it came from an EIP-2335 keystore (an
/// off-disk operator secret exists), the HKDF-derived [`ShareSealKey`] that seals
/// the per-epoch DKG shares at rest (E2). The plaintext-dev branch
/// (`--dpos.bls-key-path`) returns `None` — its validator key is already plaintext
/// beside the datadir, so a key derived from it buys nothing ⇒ shares stay
/// `TAG_PLAINTEXT`.
async fn load_bls_keypair(
    cfg: &DposConfig,
    chain_id: u64,
) -> eyre::Result<(
    fluentbase_bls::keys::ValidatorBlsKeypair,
    Option<fluentbase_bls::ShareSealKey>,
)> {
    match (
        cfg.bls_keystore_path.as_deref(),
        cfg.bls_key_path.as_deref(),
        cfg.bls_kms_key_id.as_deref(),
    ) {
        (Some(keystore_path), None, None) => {
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
        (None, Some(plain_path), None) => {
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
        (None, None, Some(key_id)) => {
            let ct = cfg
                .bls_kms_ciphertext_path
                .as_deref()
                .ok_or_eyre("--dpos.bls-kms-key-id requires --dpos.bls-kms-ciphertext-path")?;
            let scalar = crate::kms::decrypt_bls_scalar(key_id, ct).await?;
            let keypair = fluentbase_bls::keys::ValidatorBlsKeypair::from_secret_bytes(&scalar)?;
            let seal_key = keypair.derive_share_seal_key(chain_id);
            info!(chain_id, key_id, "loaded BLS key via AWS KMS decrypt");
            Ok((keypair, Some(seal_key)))
        }
        _ => Err(eyre!(
            "exactly one of --dpos.bls-keystore-path | --dpos.bls-key-path | \
             --dpos.bls-kms-key-id must be set"
        )),
    }
}

async fn load_slasher_signer(
    cfg: &DposConfig,
    chain_id: u64,
) -> eyre::Result<crate::slasher_sink::SlasherSigner> {
    match (
        cfg.slasher_keystore_path.as_deref(),
        cfg.slasher_kms_key_id.as_deref(),
    ) {
        (Some(keystore_path), None) => {
            let password_path = cfg.slasher_keystore_password_file.as_deref().ok_or_eyre(
                "--dpos.slasher-keystore-path requires --dpos.slasher-keystore-password-file",
            )?;
            let password = read_password_file(password_path, "slasher keystore")?;
            let signer =
                alloy_signer_local::LocalSigner::decrypt_keystore(keystore_path, password.trim())
                    .map_err(|e| eyre!("failed decrypting slasher keystore: {e}"))?;
            Ok(crate::slasher_sink::SlasherSigner::Local(signer))
        }
        (None, Some(key_id)) => Ok(crate::slasher_sink::SlasherSigner::Kms(
            crate::kms::slasher_signer(key_id, chain_id).await?,
        )),
        _ => Err(eyre!(
            "exactly one of --dpos.slasher-keystore-path | --dpos.slasher-kms-key-id must be set"
        )),
    }
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
            bls_kms_key_id: None,
            bls_kms_ciphertext_path: None,
            peer_key_path: PathBuf::new(),
            staking_config_path: PathBuf::new(),
            bootstrappers: String::new(),
            p2p_port: 0,
            dialable: None,
            slasher_keystore_path: None,
            slasher_keystore_password_file: None,
            slasher_kms_key_id: None,
            cert_feed: None,
            follower_upstreams: vec![],
            #[cfg(feature = "dpos-devnet-byzantine")]
            byzantine_mode: None,
        }
    }

    #[tokio::test]
    async fn plaintext_bls_rejected_on_deployed_networks() {
        for cid in [
            FLUENT_DEVNET_CHAIN_ID,
            FLUENT_TESTNET_CHAIN_ID,
            FLUENT_MAINNET_CHAIN_ID,
        ] {
            let cfg = cfg_with_plaintext_bls("/nonexistent/bls.hex");
            let err = load_bls_keypair(&cfg, cid).await.unwrap_err().to_string();
            assert!(
                err.contains("forbidden on deployed network"),
                "chain_id {cid}: expected deployed-network rejection, got: {err}"
            );
        }
    }

    #[tokio::test]
    async fn plaintext_bls_gate_bypassed_on_local_network() {
        // chain_id 1337 (localnet) is not in the deployed set, so the gate
        // must NOT fire; loading then proceeds to file I/O and fails there
        // (nonexistent path) — proving the rejection is chain_id-scoped.
        let cfg = cfg_with_plaintext_bls("/nonexistent/bls.hex");
        let err = load_bls_keypair(&cfg, 1337).await.unwrap_err().to_string();
        assert!(
            !err.contains("forbidden on deployed network"),
            "local chain_id must bypass the deployed-network gate, got: {err}"
        );
    }

    /// [`drain_shutdown_tasks`]'s contract, both directions: a drain writer leaves
    /// its loop when `recv()` returns `None`, which needs EVERY sender gone —
    /// including the one still reachable through a handle somebody holds.
    ///
    /// **WHAT THIS DOES NOT REACH, stated because it was once claimed.** It does
    /// NOT pin the production ordering — that `drop(plane.shared)` in
    /// `launch_validator_overlay` runs before `run_node_stack` awaits the drains.
    /// That arrangement needs a whole reth node to exercise, and no unit test in
    /// this file gets near it. Both halves of the fixture are stand-ins: the
    /// beacon's writer task and its stores are private to the consensus crate.
    ///
    /// Nor does it catch a beacon clone escaping into a LONG-LIVED registry, which
    /// is the failure that actually happened: the RPC feed's artifact-source
    /// closure held a strong `Arc<dyn Beacon>`, so no drop at the overlay could
    /// release the senders and every clean shutdown burnt the full timeout. That
    /// class is prevented structurally now — the closure holds a `Weak` — not by
    /// this test.
    ///
    /// What it DOES pin: `drain_shutdown_tasks` waits (a still-held sender leaves
    /// the writer parked and the drain unfinished) and returns once the last
    /// sender is gone. The negative half is what stops the positive half from
    /// passing on a fixture whose sender was never wired.
    ///
    /// Runs on the commonware `tokio` runtime, not the deterministic one, because
    /// that is what the node runs on and what gives `drain_shutdown_tasks` a real
    /// timer.
    #[test]
    fn a_drain_finishes_only_once_the_last_sender_is_gone() {
        use commonware_runtime::{tokio::Runner as TokioRunner, Runner as _};
        use std::{
            future::Future as _,
            pin::Pin,
            sync::atomic::{AtomicBool, Ordering},
            task::{Context, Poll},
        };

        TokioRunner::default().start(|ctx| async move {
            let (persist, mut records) = mpsc::unbounded_channel::<()>();
            let writer_exited = Arc::new(AtomicBool::new(false));
            let writer_ran = Arc::new(AtomicBool::new(false));

            let exited = writer_exited.clone();
            let ran = writer_ran.clone();
            let mut writer = ctx
                .with_label("key_journal_writer")
                .spawn(move |_| async move {
                    while records.recv().await.is_some() {
                        ran.store(true, Ordering::SeqCst);
                    }
                    exited.store(true, Ordering::SeqCst);
                });

            // One record, so "the writer ran" becomes an OBSERVED fact rather than
            // an elapsed-time guess.
            persist.send(()).expect("the writer is up");
            // A held sender, in the shape production has it: reachable only
            // THROUGH a handle somebody else owns.
            let host_provider: Arc<dyn std::any::Any + Send + Sync> = Arc::new(persist);

            // NEGATIVE DIRECTION, and asserted against the HANDLE rather than a
            // timer: a fixed sleep says only "not yet by then". Drive the runtime
            // until the writer has demonstrably consumed the record, then poll its
            // handle — still `Pending` is the claim, and a fixture whose sender was
            // never wired (the writer never runs) or a writer that exits on an
            // empty channel both fail here.
            for _ in 0..1024 {
                if writer_ran.load(Ordering::SeqCst) {
                    break;
                }
                tokio::task::yield_now().await;
            }
            assert!(
                writer_ran.load(Ordering::SeqCst),
                "the writer task never ran; the negative half below would be vacuous"
            );
            let waker = futures::task::noop_waker();
            let mut cx = Context::from_waker(&waker);
            assert!(
                matches!(Pin::new(&mut writer).poll(&mut cx), Poll::Pending),
                "a writer whose channel still has a live sender must stay parked"
            );
            assert!(!writer_exited.load(Ordering::SeqCst));

            // POSITIVE DIRECTION.
            drop(host_provider);
            drain_shutdown_tasks(vec![("key_journal_writer", writer)]).await;
            assert!(
                writer_exited.load(Ordering::SeqCst),
                "dropping the last sender is what lets the writer resolve, and \
                 `drain_shutdown_tasks` is what waits for it"
            );
        });
    }
    /// The property the whole seed-journal drain rests on, and the one the unit
    /// test above cannot reach: the writer must survive `engine.abort()` and only
    /// then flush its tail.
    ///
    /// TWO-SIDED ON PURPOSE. A one-sided "the writer survived" assertion passes
    /// vacuously — verified, not assumed: an earlier version of this test moved
    /// the writer under a context labelled `outer_engine` and still passed,
    /// because commonware's supervision tree is built from SPAWN LINEAGE, not
    /// from label strings. What actually dies with a task is what that task
    /// spawned from its OWN context; a task spawned from the same context object
    /// the engine was spawned from is its sibling and survives.
    ///
    /// So this asserts both directions. The descendant arm is what makes the
    /// survivor arm mean something: it proves the abort really does kill, on this
    /// runtime, in this test.
    #[test]
    fn the_journal_writer_survives_the_engine_abort_only_outside_its_spawn_lineage() {
        use commonware_runtime::{tokio::Runner as TokioRunner, Metrics as _, Runner as _};
        use std::sync::atomic::{AtomicU32, Ordering};

        TokioRunner::default().start(|ctx| async move {
            let outside_flushed = Arc::new(AtomicU32::new(0));
            let inside_flushed = Arc::new(AtomicU32::new(0));
            let (outside_tx, mut outside_rx) = mpsc::unbounded_channel::<u32>();
            let (inside_tx, mut inside_rx) = mpsc::unbounded_channel::<u32>();

            // The production arrangement: spawned OUTSIDE the engine's task, from
            // the beacon's context.
            let out = outside_flushed.clone();
            let writer = ctx
                .with_label("seed_journal_writer")
                .spawn(move |_| async move {
                    while let Some(n) = outside_rx.recv().await {
                        out.fetch_add(n, Ordering::SeqCst);
                    }
                });

            // The mistake this guards against: a writer spawned from INSIDE the
            // engine task, off its own context — a true descendant.
            let inside = inside_flushed.clone();
            let engine = ctx.with_label("outer_engine").spawn(move |c| async move {
                drop(
                    c.with_label("seed_journal_writer")
                        .spawn(move |_| async move {
                            while let Some(n) = inside_rx.recv().await {
                                inside.fetch_add(n, Ordering::SeqCst);
                            }
                        }),
                );
                std::future::pending::<()>().await;
            });

            engine.abort();
            drop(engine.await);

            // Queued AFTER the abort, so only a writer that is still alive can
            // ever account for it.
            outside_tx.send(2).expect("the outside writer is alive");
            outside_tx.send(3).expect("the outside writer is alive");
            let _ = inside_tx.send(5);
            drop(outside_tx);
            drop(inside_tx);

            drain_shutdown_tasks(vec![("seed_journal_writer", writer)]).await;

            assert_eq!(
                outside_flushed.load(Ordering::SeqCst),
                5,
                "the writer outside the engine's spawn lineage drained its tail after the abort"
            );
            assert_eq!(
                inside_flushed.load(Ordering::SeqCst),
                0,
                "premise: a writer spawned inside the engine task IS killed by the abort — \
                 without this the assertion above proves nothing"
            );
        });
    }
}

/// The plane's vote-backup ingress and the route-miss observers that sit on the
/// other four Muxers. The muxer itself is upstream code and is not under test —
/// what is under test is that a raw wire sub-channel id can no longer become an
/// [`Epoch`], and that the drops are counted where they happen.
#[cfg(test)]
mod route_miss_tests {
    use super::*;
    use commonware_cryptography::ed25519::PrivateKey as Ed25519PrivateKey;
    use fluentbase_consensus::dpos::ResettableForward;
    use fluentbase_p2p::constants::DKG_SUBCHANNEL_BASE;
    use metrics::{SharedString, Unit};
    use metrics_util::{
        debugging::{DebugValue, DebuggingRecorder},
        CompositeKey,
    };

    /// The id observed in production: an epoch-key agreement instance for epoch 2.
    const AGREEMENT_ID: u64 = DKG_SUBCHANNEL_BASE | 2;

    fn peer(seed: u64) -> PeerPubkey {
        Ed25519PrivateKey::from_seed(seed).public_key()
    }

    fn frame(subchannel: u64, from: PeerPubkey) -> RouteMissFrame {
        (subchannel, (from, IoBuf::from(vec![0u8; 4])))
    }

    /// One materialised reading of the recorder — the row shape
    /// `Snapshotter::snapshot` hands back, which [`route_miss_count`] indexes.
    type MetricCell = (CompositeKey, Option<Unit>, Option<SharedString>, DebugValue);

    fn route_miss_count(snap: &[MetricCell], channel: &str, kind: &str) -> u64 {
        snap.iter()
            .filter(|(k, ..)| {
                let key = k.key();
                key.name() == ROUTE_MISS_TOTAL
                    && key
                        .labels()
                        .any(|l| l.key() == "channel" && l.value() == channel)
                    && key.labels().any(|l| l.key() == "kind" && l.value() == kind)
            })
            .map(|(.., v)| match v {
                DebugValue::Counter(c) => *c,
                _ => 0,
            })
            .sum()
    }

    /// Run the production forwarder over `frames` to completion — dropping the
    /// sender is what ends its loop — and return everything it handed the
    /// `EpochManager`'s end of the re-settable forwarder.
    fn drain_forwarder(frames: Vec<RouteMissFrame>) -> Vec<VoteBackupItem> {
        let (tx, rx) = mpsc::channel(16);
        for f in frames {
            tx.try_send(f).expect("test channel has room");
        }
        drop(tx);
        let forward: ResettableForward<VoteBackupItem> = ResettableForward::new(16);
        let mut consumer = futures::executor::block_on(forward.subscribe());
        futures::executor::block_on(forward_vote_backup(rx, forward.slot()));
        let mut delivered = Vec::new();
        while let Ok(item) = consumer.try_recv() {
            delivered.push(item);
        }
        delivered
    }

    /// FLU-1170. Two distinct senders is the f+1 corroboration bar at n=4, so this
    /// is exactly the traffic that used to carry `DKG_SUBCHANNEL_BASE | 2` into
    /// `highest_observed_epoch` as epoch 4294967298 — soft-entering the live epoch
    /// verify-only and dropping the committee below quorum. Nothing reaching the
    /// manager is the strong form of "the frontier did not move, no span was
    /// registered, no marshal hint was sent": those are all downstream of this
    /// channel.
    #[test]
    fn an_agreement_subchannel_never_reaches_the_epoch_manager() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let delivered = metrics::with_local_recorder(&recorder, || {
            drain_forwarder(vec![
                frame(AGREEMENT_ID, peer(1)),
                frame(AGREEMENT_ID, peer(2)),
            ])
        });
        let epochs: Vec<u64> = delivered.iter().map(|(e, _)| e.get()).collect();
        assert!(
            epochs.is_empty(),
            "an agreement sub-channel id reached the frontier as an epoch: {epochs:?}"
        );
        let metrics = snapshotter.snapshot().into_vec();
        assert_eq!(
            route_miss_count(&metrics, VOTE_LABEL, "agreement"),
            2,
            "both drops must be counted"
        );
    }

    /// The control for the test above: if catch-up itself broke, that test would
    /// pass for the wrong reason.
    #[test]
    fn a_real_epoch_from_two_senders_still_reaches_the_epoch_manager() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let delivered = metrics::with_local_recorder(&recorder, || {
            drain_forwarder(vec![frame(7, peer(1)), frame(7, peer(2))])
        });
        let epochs: Vec<u64> = delivered.iter().map(|(e, _)| e.get()).collect();
        assert_eq!(epochs, vec![7, 7], "a live-frontier hint must still arrive");
        let metrics = snapshotter.snapshot().into_vec();
        assert_eq!(route_miss_count(&metrics, VOTE_LABEL, "agreement"), 0);
    }

    /// Classification is per-frame, not per-sender: two out-of-space ids and one
    /// real epoch from the SAME sender must reach the manager as the real epoch
    /// alone. A build that forwards raw ids delivers all three.
    #[test]
    fn only_the_epoch_space_id_of_a_sender_reaches_the_epoch_manager() {
        let sender = peer(3);
        let delivered = drain_forwarder(vec![
            frame(DKG_SUBCHANNEL_BASE | 2, sender.clone()),
            frame(DKG_SUBCHANNEL_BASE | 3, sender.clone()),
            frame(9, sender.clone()),
        ]);
        let pins: Vec<(u64, PeerPubkey)> = delivered
            .into_iter()
            .map(|(e, (from, _))| (e.get(), from))
            .collect();
        assert_eq!(pins, vec![(9, sender)]);
    }

    /// The four observer muxes: they count and log, and hand nothing on — the
    /// task has no output at all. Both id spaces have to be distinguishable in the
    /// counter, because only one of them is expected traffic.
    #[test]
    fn route_miss_observers_count_by_channel_and_kind() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let observed = [CERT_LABEL, RESOLVER_LABEL, BROADCAST_LABEL, MARSHAL_LABEL];
        metrics::with_local_recorder(&recorder, || {
            for label in observed {
                let (tx, rx) = mpsc::channel(4);
                tx.try_send(frame(AGREEMENT_ID, peer(1))).expect("room");
                tx.try_send(frame(5, peer(2))).expect("room");
                drop(tx);
                futures::executor::block_on(observe_route_misses(label, rx));
            }
        });
        let metrics = snapshotter.snapshot().into_vec();
        for label in observed {
            assert_eq!(route_miss_count(&metrics, label, "agreement"), 1, "{label}");
            assert_eq!(route_miss_count(&metrics, label, "unknown"), 1, "{label}");
        }
        assert_eq!(route_miss_count(&metrics, VOTE_LABEL, "agreement"), 0);
    }
}
