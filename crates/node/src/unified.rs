//! Unified `--dpos` supervisor: a validator node that never restarts.
//!
//! The consensus thread alternates between two phases on the SAME process /
//! datadir / reth instance:
//!
//! - **Follower phase** — the lean cert-follow plane drives reth from upstream
//!   finality certs in epoch-long "laps": each lap stops at the last block of
//!   the next epoch, where the supervisor reads the ahead-committed
//!   `committee(E+1)` from its OWN executed state. Not a member → next lap;
//!   member → promote, exactly at the boundary.
//! - **Signer phase** — the full DPoS stack ([`launch_dpos_layer`]) with an
//!   explicit `Promotion` cold-start (anchor = EL tip, discriminator
//!   heuristics bypassed — the follower phase just verified the chain).
//!   Rotation-out ([`ModeEvent::RotatedOut`]) demotes back to the follower
//!   phase; any other stack exit is fatal (fail-closed, as legacy).
//!
//! Entry rule: a node whose key is ALREADY in the current committee (normal
//! validator restart, fresh migration) goes straight to the signer phase with
//! the LEGACY cold-start discriminator (`Restart` / `FreshMigration`).

use commonware_cryptography::Signer as _;
use commonware_runtime::{tokio::Context, Metrics as _, Spawner as _};
use eyre::{bail, WrapErr as _};
use fluentbase_bls::PeerPubkey;
use fluentbase_consensus::{
    peek_consensus_archive_last_finalized, CertFollowConfig, CertFollowLayer, CertFollowRethHandle,
    FollowExit, ModeEvent,
};
use fluentbase_staking_reader::reader::{
    epoch_of_block, RethStakingStateReader, ValidatorSetSnapshot,
};
use reth_chainspec::EthChainSpec as _;
use reth_ethereum_engine_primitives::EthEngineTypes;
use reth_ethereum_primitives::{Block as RethBlock, EthPrimitives};
use reth_node_api::{FullNodeComponents, FullNodeTypes};
use reth_node_builder::{rpc::RethRpcAddOns, FullNode};
use reth_storage_api::{
    BlockHashReader, BlockIdReader, BlockNumReader, BlockReader, HeaderProvider,
    StateProviderFactory,
};
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::{
    cert_follow::upstream,
    consensus_rpc::FeedStateHandle,
    dpos::{launch_dpos_layer, CanonicalStateAccess, CertFeed, DposConfig},
};

/// Which phase the supervisor enters on the current loop iteration, and the
/// cold-start kind it implies. `SignerFirst` is reachable only on the FIRST
/// iteration (in-committee at startup with consensus state to resume → the
/// legacy `Restart`/`FreshMigration` discriminator); every subsequent
/// iteration is `FollowFirst` (run a follower lap, then promote via the
/// explicit `Promotion` cold-start). One consumed-once value replaces the prior
/// `skip_follower` + `promote_with_promotion_kind` flag pair.
#[derive(Clone, Copy)]
enum Entry {
    SignerFirst,
    FollowFirst,
}

fn is_member(snap: &ValidatorSetSnapshot, peer: &PeerPubkey) -> bool {
    snap.validators.iter().any(|v| v.keys.peer_pubkey == *peer)
}

/// The unified consensus-thread body. `cfg.follower_upstreams` is non-empty
/// (the spawn router guarantees it).
pub(crate) async fn run_unified_stack<N, AddOns>(
    ctx: Context,
    node: FullNode<N, AddOns>,
    mut cfg: DposConfig,
    process_token: CancellationToken,
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
    crate::dpos::spawn_devnet_metrics(&ctx, &cfg);

    // Claim the single-execution import escrow ONCE per process — `from_env`
    // takes a launch-deposited engine-tree sender that cannot be re-claimed;
    // every phase below CLONES this importer instead of re-claiming.
    let beacon_engine =
        crate::importer::RethImporter::from_env(node.add_ons_handle.beacon_engine_handle.clone())?;

    let my_peer: PeerPubkey = fluentbase_p2p::read_ed25519_key_from_file(&cfg.peer_key_path)
        .wrap_err_with(|| {
            format!(
                "failed loading peer key from {}",
                cfg.peer_key_path.display()
            )
        })?
        .public_key();
    let staking_config = fluentbase_staking_reader::reader::StakingReaderConfig::from_json_path(
        &cfg.staking_config_path,
    )
    .wrap_err_with(|| {
        format!(
            "failed loading staking config from {}",
            cfg.staking_config_path.display()
        )
    })?;
    let reader = RethStakingStateReader::new(
        node.provider.clone(),
        node.evm_config.clone(),
        staking_config.clone(),
    );

    // The RPC feed handle is process-stable (the namespace closure captured
    // it); the per-phase sink/rx pairs are minted fresh below — drop the
    // launch-time pair from node_modes.
    let feed_state: Option<FeedStateHandle> = cfg.cert_feed.take().map(|cf| cf.handle);

    // Public threshold-beacon polynomial (`PK_epoch`), parsed once. The
    // follower-lap verifier + deriver need it to verify the seed half of each
    // combined cert and reproduce `prev_randao = H(seed)`; absent on a
    // pre-beacon chain. (The signer lap's deriver gets its beacon key from the
    // legacy `spawn_dpos` path it delegates to.)
    let beacon_seed_namespace = fluentbase_consensus::beacon::seed::seed_namespace(
        &fluentbase_bls::fluent_namespace(node.chain_spec().chain_id()),
    );
    let beacon_sharing = cfg
        .beacon_sharing_path
        .as_ref()
        .map(crate::cert_follow::parse_beacon_sharing)
        .transpose()?;

    // Entry rule: in the CURRENT committee → signer first, legacy
    // discriminator (Restart / FreshMigration). Codeless ChainConfig or
    // unscheduled activation → there is no committee to be in yet.
    let mut entry = {
        // ONE finalized read: num and hash must come from the SAME block, or
        // epoch_of_block(num) and the committee snapshot at hash disagree
        // across a boundary. Genesis fallback (pre-finality) keeps both fields
        // consistent (number 0, genesis hash).
        let (fin_num, fin_hash) = node
            .provider
            .finalized_block_num_hash()
            .wrap_err("provider.finalized_block_num_hash at unified startup")?
            .map(|nh| (nh.number, nh.hash))
            .unwrap_or_else(|| (0, node.chain_spec().genesis_hash()));
        match reader.scheduled_dpos_activation(fin_hash)? {
            None => Entry::FollowFirst,
            Some(act) => {
                let interval = reader.epoch_block_interval(fin_hash)?;
                eyre::ensure!(interval > 0, "epoch_block_interval must be > 0");
                let cur = epoch_of_block(fin_num, interval, act);
                let snap = reader.epoch_committee_snapshot(cur, fin_hash)?;
                if !is_member(&snap, &my_peer) {
                    Entry::FollowFirst
                } else {
                    // Signer-first only if there is consensus state to resume.
                    // A node in the committee but with an EMPTY consensus
                    // archive while its EL is already past epoch 0 (e.g. a
                    // fresh validator that synced reth via devp2p but never ran
                    // consensus) would hit the legacy discriminator's fatal
                    // "empty archive + EL past epoch 0" branch. It must FOLLOW
                    // first to build its archive, then promote via Promotion —
                    // so route it to the follower lap instead.
                    let archive_finalized = peek_consensus_archive_last_finalized(&ctx).await?;
                    let empty_archive = archive_finalized <= act;
                    let el_past_epoch_0 = fin_num >= act + interval as u64;
                    if empty_archive && el_past_epoch_0 {
                        info!(
                            fin_num,
                            act,
                            "in committee but empty consensus archive + EL past epoch 0 — \
                             following first to build the archive before promoting"
                        );
                        Entry::FollowFirst
                    } else {
                        Entry::SignerFirst
                    }
                }
            }
        }
    };
    info!(
        signer_first = matches!(entry, Entry::SignerFirst),
        "unified --dpos supervisor starting"
    );

    loop {
        let promotion = match entry {
            // Signer-first (in-committee at startup): legacy cold-start
            // discriminator, NOT Promotion.
            Entry::SignerFirst => false,
            Entry::FollowFirst => {
                // ---- follower lap ----
                let (ws_actor, upstream_handle, finalized_rx) =
                    upstream::init(ctx.clone(), cfg.follower_upstreams.clone());
                let mut ws_handle = ws_actor.start();

                let verified_tx = feed_state.clone().map(|handle| {
                    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<
                        fluentbase_consensus::UpstreamFinalized,
                    >();
                    let window: crate::consensus_rpc::state::CertWindow = Default::default();
                    handle.set_window(window.clone());
                    drop(ctx.with_label("window_feed").spawn(move |_| async move {
                        while let Some(uf) = rx.recv().await {
                            let height = uf.block.height;
                            let cb = std::sync::Arc::new(
                                crate::certified_block::CertifiedBlock::from_parts(
                                    &uf.finalization,
                                    &uf.block,
                                ),
                            );
                            {
                                let mut w = window.write().expect("cert window poisoned");
                                w.insert(height, cb.clone());
                                while w.len() as u64
                                    > fluentbase_consensus::cert_follow::JUMP_THRESHOLD
                                {
                                    w.pop_first();
                                }
                            }
                            handle.record_finalized(cb, crate::consensus_rpc::now_ms());
                        }
                    }));
                    tx
                });

                let reth = CertFollowRethHandle {
                    provider: node.provider.clone(),
                    evm_config: node.evm_config.clone(),
                    beacon_engine_handle: beacon_engine.clone(),
                    chain_id: node.chain_spec().chain_id(),
                    canonical_state: node.provider.canonical_state(),
                    genesis_hash: node.chain_spec().genesis_hash(),
                };
                let deriver_base = crate::derive::RethBlockDeriver::new(
                    node.provider.clone(),
                    node.evm_config.clone(),
                );
                let (deriver, follow_beacon) = match &beacon_sharing {
                    Some(sharing) => (
                        deriver_base.with_beacon_key(
                            Some(*sharing.public()),
                            beacon_seed_namespace.clone(),
                        ),
                        Some((sharing.clone(), None, beacon_seed_namespace.clone())),
                    ),
                    None => (deriver_base, None),
                };
                let executed = crate::ordering::ProviderExecutedChain(node.provider.clone());
                let follow_cfg = CertFollowConfig {
                    staking_config: staking_config.clone(),
                    l1_checkpoint_hash: None,
                    fcu_heartbeat_interval: Duration::from_secs(8),
                    stop_at_next_boundary: true,
                    beacon: follow_beacon,
                };
                let mut handle = CertFollowLayer::launch(
                    ctx.clone(),
                    reth,
                    follow_cfg,
                    deriver,
                    executed,
                    upstream_handle,
                    finalized_rx,
                    verified_tx,
                    process_token.clone(),
                )
                .await?;

                let stopped = tokio::select! {
                    biased;
                    _ = process_token.cancelled() => {
                        handle.consensus_handle.abort();
                        ws_handle.abort();
                        info!("unified supervisor exiting (shutdown) during follower phase");
                        return Ok(());
                    }
                    _ = &mut handle.consensus_handle => {
                        // Clean boundary stop sent the exit BEFORE the task ended;
                        // anything else is the fail-closed path (token already
                        // cancelled by the layer — surface the error).
                        match handle.stopped_rx.try_recv() {
                            Ok(exit) => exit,
                            Err(_) => {
                                ws_handle.abort();
                                bail!("follower phase failed (fail-closed)");
                            }
                        }
                    }
                    res = &mut ws_handle => {
                        error!(?res, "follower WS upstream died");
                        handle.consensus_handle.abort();
                        process_token.cancel();
                        bail!("follower WS upstream died");
                    }
                };
                ws_handle.abort();

                let FollowExit::StoppedAt { height, evm_hash } = stopped;
                // The lap ran past the launch checkpoint, so geometry is
                // guaranteed readable at the stop state (launch computed the
                // boundary from it). The promotion anchor is the FINALIZED tier
                // (cursor − K): a valid promotion requires it to sit EXACTLY on
                // the last block of an epoch (the per-epoch Inline needs that
                // block as the "starting epoch block", and the EpochTransition
                // boundary-resume rule then bootstraps the NEXT epoch — the one
                // a fresh member belongs to). A jump overshoot can land the
                // anchor mid-epoch: not promotable — re-lap instead.
                let act = reader
                    .dpos_activation_block(evm_hash)
                    .wrap_err("activation read at follower stop point")?;
                let interval = reader.epoch_block_interval(evm_hash)?;
                eyre::ensure!(interval > 0, "epoch_block_interval must be > 0");
                let anchor = height.saturating_sub(fluentbase_consensus::K);
                let anchor_is_boundary = (anchor + 1).saturating_sub(act) % interval as u64 == 0;
                if !anchor_is_boundary {
                    info!(
                        height,
                        anchor, "follower lap overshot the boundary (jump) — re-lapping"
                    );
                    continue;
                }
                let enter_epoch = epoch_of_block(anchor, interval, act) + 1;
                let snap = reader.epoch_committee_snapshot(enter_epoch, evm_hash)?;
                let mine = is_member(&snap, &my_peer);
                info!(height, enter_epoch, member = mine, "follower lap complete");
                if !mine {
                    continue;
                }
                // Followed and verified up to the boundary → Promotion cold-start.
                true
            }
        };
        // Every iteration after this one follows first (demotion loops back
        // here with `entry` already FollowFirst).
        entry = Entry::FollowFirst;

        // ---- signer phase ----
        let (mode_tx, mut mode_rx) = tokio::sync::mpsc::unbounded_channel::<ModeEvent>();
        let phase_token = process_token.child_token();
        let cert_feed = feed_state.clone().map(|handle| {
            let (sink, rx) = fluentbase_consensus::FeedSink::channel();
            CertFeed { sink, rx, handle }
        });
        let mut handle = launch_dpos_layer(
            ctx.clone(),
            &node,
            &cfg,
            beacon_engine.clone(),
            cert_feed,
            promotion,
            Some(mode_tx),
            phase_token.clone(),
        )
        .await?;

        let demoted = tokio::select! {
            biased;
            _ = process_token.cancelled() => {
                handle.network_handle.abort();
                handle.consensus_handle.abort();
                info!("unified supervisor exiting (shutdown) during signer phase");
                return Ok(());
            }
            ev = mode_rx.recv() => {
                match ev {
                    Some(ModeEvent::RotatedOut { epoch }) => {
                        info!(epoch, "rotated out of committee — demoting to follower phase");
                        true
                    }
                    None => {
                        // All engines (the senders) died without a rotation
                        // signal — treat as a stack fault below.
                        false
                    }
                }
            }
            _ = phase_token.cancelled() => {
                // The layer's internal fatal path (e.g. 3 consecutive
                // on_finalized errors) cancels its token.
                handle.network_handle.abort();
                handle.consensus_handle.abort();
                process_token.cancel();
                bail!("signer phase failed (layer cancelled its token)");
            }
            _ = &mut handle.consensus_handle => { false }
            _ = &mut handle.network_handle => { false }
        };

        phase_token.cancel();
        handle.network_handle.abort();
        handle.consensus_handle.abort();

        if !demoted {
            process_token.cancel();
            bail!("signer phase exited unexpectedly (fail-closed)");
        }
        // Demoted: loop back to a follower lap (`entry` is FollowFirst).
    }
}
