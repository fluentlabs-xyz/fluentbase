//! Fluent ExEx: generates block witnesses and pushes them to a [`WitnessHub`].
//!
//! The main entry point is [`exex_main_loop`], which listens for committed
//! chains from Reth, re-executes each block on a blocking thread to build
//! a witness, and pushes the serialized result into the hub for gRPC delivery.
//!
//! ## Architecture decisions
//!
//! **Synchronous processing with post-ack.**
//! `FinishedHeight` is sent only after all blocks in the batch are processed.
//! Reth's pruner is gated by `FinishedHeight` — it will not prune any state
//! above the last acked height. This gives us an ironclad guarantee that
//! `history_by_block_number(N-1)` will succeed for every block we process,
//! without needing to pin providers or use channels.
//!
//! **Parallel processing with ordered delivery.**
//! All witness tasks in a batch are spawned immediately on the blocking thread
//! pool. Results are collected in block number order so the hub sees a
//! gap-free stream. Slow historical blocks do not stall later blocks'
//! computation — only their delivery is ordered.
//!
//! **Tip age as the only skip criterion.**
//! We do NOT skip based on batch size. On an L2 with 1s block time, the ExEx
//! can legitimately lag by 10-50 blocks during heavy periods — these are fresh
//! blocks with available state that must be witnessed. Only chains whose tip
//! is older than [`MAX_TIP_AGE_SECS`] are skipped (indicates initial sync or
//! prolonged network partition).
//!
use std::sync::Arc;

use alloy_consensus::BlockHeader;
use alloy_eips::BlockNumHash;
use futures::TryStreamExt;
use reth_exex::{ExExContext, ExExEvent};
use reth_node_api::FullNodeComponents;
use reth_primitives_traits::{Block as BlockTrait, NodePrimitives};
use reth_provider::{BlockReader, HeaderProvider, StateProviderFactory};
use revm_primitives::{Address, B256};
use tracing::{error, info, warn};

use rsp_client_executor::io::ClientExecutorInput;
use rsp_client_executor::{BlockValidator, IntoInput};
use rsp_primitives::genesis::Genesis;

use reth_chainspec::ChainSpec;
use reth_evm::ConfigureEvm;
use reth_evm_ethereum::EthEvmConfig;
use rsp_client_executor::evm::FluentEvmFactory;
use rsp_host_executor::{EthHostExecutor, HostError};

use witness_courier::hub::WitnessHub;
use witness_courier::types::ProveRequest;


// ---------------------------------------------------------------------------
// Type aliases
// ---------------------------------------------------------------------------

/// EVM primitives type alias used throughout the ExEx.
pub type FluentPrimitives = <EthEvmConfig<ChainSpec, FluentEvmFactory> as ConfigureEvm>::Primitives;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Skip committed chains whose tip is older than this (seconds).
/// Chains this stale come from initial sync or prolonged partitions — the
/// provider won't have contiguous historical state for them.
const MAX_TIP_AGE_SECS: u64 = 120;

// ---------------------------------------------------------------------------
// Main loop
// ---------------------------------------------------------------------------

/// Main ExEx loop: listens for committed chains, builds witnesses, pushes to hub.
///
/// # Pruning safety
///
/// Reth's pruner only prunes state up to the lowest `FinishedHeight` across
/// all registered ExExes. We send `FinishedHeight` **after** processing, so
/// all state reads during witness generation are guaranteed to succeed.
pub async fn exex_main_loop<Node>(
    mut ctx: ExExContext<Node>,
    custom_beneficiary: Option<Address>,
    hub: Arc<WitnessHub>,
) -> eyre::Result<()>
where
    Node: FullNodeComponents,
    Node::Provider: StateProviderFactory
        + HeaderProvider<Header = alloy_consensus::Header>
        + BlockReader<Block = <FluentPrimitives as NodePrimitives>::Block>
        + Clone
        + Unpin
        + std::fmt::Debug
        + Send
        + Sync
        + 'static,
    Node::Evm: ConfigureEvm<Primitives = FluentPrimitives> + 'static,
    FluentPrimitives: IntoInput
        + BlockValidator<ChainSpec>
        + NodePrimitives<BlockHeader = alloy_consensus::Header>
        + serde::Serialize,
{
    let genesis = Genesis::Fluent;
    let chain_spec = Arc::new(ChainSpec::try_from(&genesis).map_err(|e| eyre::eyre!(e))?);
    let executor = Arc::new(EthHostExecutor::eth(chain_spec, None));
    let provider = ctx.provider().clone();

    // Carries the last processed block's state_root across notifications to
    // avoid a provider lookup for the first block of each batch.
    let mut last_state_root: Option<B256> = None;

    while let Some(notification) = ctx.notifications.try_next().await? {
        // ---------------------------------------------------------------
        // 1. Process new committed chain (if any).
        // ---------------------------------------------------------------
        let Some(new_chain) = notification.committed_chain() else {
            // No committed chain. This can happen after a hard restart when
            // Reth rolls back a few blocks to restore DB consistency.
            // Ack the parent of the first reverted block so the pruner
            // does not retain stale state above the rollback point.
            if let Some(reverted) = notification.reverted_chain() {
                let first = reverted.first();
                let safe_height = BlockNumHash::new(
                    first.number().saturating_sub(1),
                    first.header().parent_hash(),
                );
                ack_finished_height(&mut ctx, safe_height)?;
            }
            continue;
        };

        let chain_first = new_chain.first().number();
        let chain_tip = new_chain.tip().number();
        let chain_len = new_chain.len();

        // ---------------------------------------------------------------
        // 2. Stale chain check.
        //
        //    Bypassed when start_block is set: historical blocks are
        //    legitimately old (tip_age >> MAX_TIP_AGE_SECS).
        //
        //    `saturating_sub` + fallback on broken clock (NTP glitch,
        //    uninitialised clock in Nitro Enclave, etc).
        // ---------------------------------------------------------------
        let tip_timestamp = new_chain.tip().timestamp();
        let now_secs = now_unix_secs(tip_timestamp);
        let tip_age = now_secs.saturating_sub(tip_timestamp);

        if tip_age > MAX_TIP_AGE_SECS {
            info!(
                tip = chain_tip,
                blocks = chain_len,
                age_secs = tip_age,
                "Skipping stale chain (>{MAX_TIP_AGE_SECS}s behind wall clock)"
            );
            ack_finished_height(&mut ctx, new_chain.tip().num_hash())?;
            continue;
        }

        if chain_len > 1 {
            info!(
                first = chain_first,
                tip = chain_tip,
                blocks = chain_len,
                age_secs = tip_age,
                "Processing multi-block batch (ExEx lagging)"
            );
        }

        // ---------------------------------------------------------------
        // 3. Parallel block processing.
        //
        //    - Pre-compute parent state roots from block headers (no
        //      provider needed after the first block — roots are in the
        //      committed chain data).
        //    - Spawn all witness tasks immediately (they run in parallel
        //      on the blocking thread pool).
        //    - Collect results IN ORDER and deliver to hub (gap-free stream).
        //    - `FinishedHeight` is sent AFTER the loop, not inside.
        //      This means reth's pruner cannot touch state we need.
        // ---------------------------------------------------------------

        // ── Pre-compute parent state roots from block headers ──────────
        let blocks: Vec<_> = new_chain.blocks_iter().collect();
        let mut parent_roots: Vec<B256> = Vec::with_capacity(blocks.len());
        let mut broke_at_precompute = false;

        for (i, block) in blocks.iter().enumerate() {
            let root = if i == 0 {
                match last_state_root {
                    Some(r) => r,
                    None => match resolve_parent_state_root(&provider, block.number()) {
                        Ok(r) => r,
                        Err(e) => {
                            error!(block_number = block.number(), err = %e,
                                "Cannot resolve parent state root — aborting batch");
                            broke_at_precompute = true;
                            break;
                        }
                    },
                }
            } else {
                blocks[i - 1].header().state_root()
            };
            parent_roots.push(root);
        }

        if broke_at_precompute {
            ack_finished_height(&mut ctx, new_chain.tip().num_hash())?;
            continue;
        }

        // ── Spawn all witness tasks in parallel ────────────────────────
        // All blocks get a JoinHandle that starts executing immediately
        // on the blocking thread pool.
        let mut handles: Vec<(u64, tokio::task::JoinHandle<Result<ClientExecutorInput<FluentPrimitives>, WitnessError>>)> =
            Vec::with_capacity(blocks.len());

        for (block, parent_root) in blocks.iter().zip(parent_roots.iter().copied()) {
            let block_number = block.number();

            let current_block: <FluentPrimitives as NodePrimitives>::Block =
                BlockTrait::new(block.header().clone(), block.body().clone());

            let handle = tokio::task::spawn_blocking({
                let executor = Arc::clone(&executor);
                let provider = provider.clone();
                let genesis = genesis.clone();
                move || {
                    process_block_with_data(
                        &executor, current_block, parent_root,
                        provider, genesis, custom_beneficiary,
                    )
                    .map_err(WitnessError::Execution)
                }
            });
            handles.push((block_number, handle));
        }

        // ── Collect in ORDER and push to hub ──────────────────────────
        // Awaiting in block number order guarantees hub sees a gap-free
        // stream. Slow blocks hold up delivery of later blocks but not
        // their computation.
        let mut blocks_succeeded: u64 = 0;
        let start_collect = std::time::Instant::now();

        for (block_number, handle) in handles {
            let input = match handle.await {
                Ok(Ok(input)) => input,
                Ok(Err(e)) => {
                    error!(block_number, err = %e,
                        elapsed_ms = start_collect.elapsed().as_millis() as u64,
                        "Witness failed — aborting batch");
                    break;
                }
                Err(e) => {
                    error!(block_number, "Witness task panicked: {e}");
                    break;
                }
            };

            let encoded = match encode_witness(&input, block_number) {
                Ok(data) => data,
                Err(()) => break,
            };

            hub.push(Arc::new(ProveRequest { block_number, payload: encoded })).await;
            last_state_root = Some(
                blocks
                    .iter()
                    .find(|b| b.number() == block_number)
                    .map(|b| b.header().state_root())
                    .unwrap_or_default(),
            );
            blocks_succeeded += 1;

            info!(
                block_number,
                total_ms = start_collect.elapsed().as_millis() as u64,
                remaining = chain_len as u64 - blocks_succeeded,
                "Witness dispatched"
            );
        }

        // ---------------------------------------------------------------
        // 4. Log partial-batch outcomes.
        // ---------------------------------------------------------------
        let witnessed_targets = chain_len as u64;
        if witnessed_targets > 0 && blocks_succeeded == 0 {
            error!(
                first = chain_first,
                tip = chain_tip,
                blocks = chain_len,
                "Entire witnessable batch failed — no witnesses produced"
            );
        } else if blocks_succeeded < witnessed_targets {
            warn!(
                succeeded = blocks_succeeded,
                total = witnessed_targets,
                first = chain_first,
                tip = chain_tip,
                "Batch partially processed — downstream has gap"
            );
        }

        // ---------------------------------------------------------------
        // 5. Ack FinishedHeight.
        //
        //    ALWAYS ack the tip regardless of partial failure. If we don't,
        //    reth's ExEx pipeline stalls — it won't send more notifications,
        //    the pruner freezes, and the node's disk usage grows unbounded.
        //
        //    The tradeoff: pruner may clean state for blocks we failed to
        //    witness. This is acceptable because:
        //    (a) Reth won't re-send committed chains — they're committed.
        //    (b) The witness pipeline must have a separate retry/recovery
        //        mechanism for missed blocks (e.g. fallback to RPC path).
        //    (c) Blocking the node is worse than missing a witness.
        // ---------------------------------------------------------------
        ack_finished_height(&mut ctx, new_chain.tip().num_hash())?;
    }

    info!("ExEx notification stream ended — shutting down");
    Ok(())
}

// ---------------------------------------------------------------------------
// Block processing
// ---------------------------------------------------------------------------

fn process_block_with_data<P>(
    executor: &EthHostExecutor,
    current_block: <FluentPrimitives as NodePrimitives>::Block,
    parent_state_root: B256,
    provider: P,
    genesis: Genesis,
    custom_beneficiary: Option<Address>,
) -> Result<ClientExecutorInput<FluentPrimitives>, HostError>
where
    P: StateProviderFactory
        + HeaderProvider<Header = alloy_consensus::Header>
        + BlockReader<Block = <FluentPrimitives as NodePrimitives>::Block>
        + Clone
        + std::fmt::Debug,
{
    executor.execute_exex_with_block(
        current_block,
        parent_state_root,
        provider,
        genesis,
        custom_beneficiary,
    )
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns the current unix timestamp in seconds.
///
/// Falls back to `fallback` if the system clock is unavailable or broken
/// (e.g. inside a Nitro Enclave before NTP sync). This prevents panicking
/// on `SystemTime::duration_since(UNIX_EPOCH).unwrap()`.
fn now_unix_secs(fallback: u64) -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(fallback)
}

/// Resolves the parent state root by reading the parent header from the
/// provider. Used for the first block in a committed chain, where there
/// is no `last_state_root` from a previous iteration.
fn resolve_parent_state_root<P>(provider: &P, block_number: u64) -> Result<B256, String>
where
    P: HeaderProvider<Header = alloy_consensus::Header>,
{
    let parent_number = block_number.saturating_sub(1);
    provider
        .header_by_number(parent_number)
        .map_err(|e| format!("provider error for block {parent_number}: {e}"))?
        .map(|hdr| hdr.state_root)
        .ok_or_else(|| format!("parent header {parent_number} not found in provider"))
}

/// Serializes a witness with bincode.
///
/// Returns `Err(())` on failure — errors are logged internally to keep
/// the call site clean (caller just does `break`).
fn encode_witness(
    input: &ClientExecutorInput<FluentPrimitives>,
    block_number: u64,
) -> Result<Vec<u8>, ()>
where
    FluentPrimitives: serde::Serialize,
{
    let encoded = bincode::serialize(input).map_err(|e| {
        error!(block_number, err = %e, "Bincode serialization failed");
    })?;

    info!(
        block_number,
        payload_bytes = encoded.len(),
        "Witness encoded"
    );

    Ok(encoded)
}


/// Sends `FinishedHeight` to reth's ExEx manager.
///
/// If the event channel is closed, the node is shutting down — we return
/// `Err` so the main loop exits cleanly.
fn ack_finished_height<Node>(
    ctx: &mut ExExContext<Node>,
    tip: BlockNumHash,
) -> eyre::Result<()>
where
    Node: FullNodeComponents,
{
    if let Err(e) = ctx.events.send(ExExEvent::FinishedHeight(tip)) {
        error!(
            block = tip.number,
            err = %e,
            "Failed to send FinishedHeight — node shutting down?"
        );
        return Err(eyre::eyre!("ExEx event channel closed"));
    }
    Ok(())
}


#[derive(Debug)]
enum WitnessError {
    TaskPanicked(tokio::task::JoinError),
    Execution(HostError),
}

impl std::fmt::Display for WitnessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TaskPanicked(e) => write!(f, "witness task panicked: {e}"),
            Self::Execution(e) => write!(f, "execution failed: {e}"),
        }
    }
}
