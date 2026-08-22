//! [`RethImporter`] — the production [`BeaconEngineLike`]: FCU passthrough to
//! the consensus-engine handle plus single-execution block import.
//!
//! Default mode hands the engine tree the pre-executed artifacts via
//! `EngineApiRequest::InsertExecutedBlock` — reth re-executes nothing and
//! recomputes no state root (the derivation already did both; correctness is
//! covered by the committee's `result` attestation K blocks later). The
//! subsequent two-tier FCU canonicalizes from tree state. Ordering is safe by
//! construction: the insert is enqueued on the tree's channel before the FCU
//! even enters the beacon channel, and both funnel into the same FIFO.
//!
//! `FLUENT_DPOS_IMPORT_MODE=new-payload` used to keep the Phase A re-execution
//! path as the conformance / operator escape hatch. It is now REFUSED at
//! startup: `insert` is the only import mode DPoS is validated on, and the
//! deferred executor was never exercised on the re-execution path. See
//! [`RethImporter::from_env`].

use crate::derive::DerivedExecution;
use alloy_rpc_types_engine::{
    ForkchoiceState, ForkchoiceUpdated, PayloadStatus, PayloadStatusEnum,
};
use crossbeam_channel::Sender;
use fluentbase_consensus::{BeaconEngineLike, EngineError};
use reth_chain_state::{ComputedTrieData, ExecutedBlock};
use reth_engine_primitives::{BeaconForkChoiceUpdateError, ConsensusEngineHandle};
use reth_engine_tree::engine::{EngineApiRequest, FromEngine};
use reth_ethereum_engine_primitives::EthEngineTypes;
use reth_ethereum_primitives::EthPrimitives;
use std::sync::Arc;

/// Engine-tree request sender. `tree_sender_escrow::take` downcasts by EXACT
/// type, and a mismatch is indistinguishable from "nothing was deposited" — so
/// this alias must keep denoting the type reth deposits at launch.
type TreeTx = Sender<
    FromEngine<EngineApiRequest<EthEngineTypes, EthPrimitives>, reth_ethereum_primitives::Block>,
>;

#[derive(Clone, Debug)]
pub struct RethImporter {
    engine: ConsensusEngineHandle<EthEngineTypes>,
    tree: TreeTx,
}

impl RethImporter {
    /// Validate `FLUENT_DPOS_IMPORT_MODE` and claim the engine-tree sender from
    /// the launch escrow. Fails loud when the escrow is empty (a reth fork
    /// without the deposit, or a second claim in one process).
    ///
    /// `insert` is the only supported mode: it is the only route the deferred
    /// executor has ever been validated on, so the former `new-payload` escape
    /// hatch pointed operators at an unexercised path and is refused at startup
    /// rather than at the first divergence.
    pub fn from_env(engine: ConsensusEngineHandle<EthEngineTypes>) -> eyre::Result<Self> {
        match std::env::var("FLUENT_DPOS_IMPORT_MODE").as_deref() {
            Ok("insert") | Err(_) => {}
            Ok(other) => {
                eyre::bail!("FLUENT_DPOS_IMPORT_MODE={other:?} — expected \"insert\"")
            }
        }
        let Some(tree) = reth_engine_tree::launch::tree_sender_escrow::take::<TreeTx>() else {
            eyre::bail!(
                "single-execution import requires the engine-tree sender escrow, which is \
                 empty — reth fork without the launch deposit, or the sender was already \
                 claimed in this process"
            );
        };
        tracing::info!("DPoS block import: single-execution (InsertExecutedBlock)");
        Ok(Self { engine, tree })
    }
}

impl BeaconEngineLike for RethImporter {
    type ExecutionData = DerivedExecution;

    async fn fork_choice_updated(
        &self,
        state: ForkchoiceState,
    ) -> Result<ForkchoiceUpdated, EngineError> {
        // The FCU verdict (incl. `Invalid`) rides in the `Ok`. The `Err` half is
        // the concrete-error → typed taxonomy boundary — and it is NOT one class:
        // reth's error enum mixes "the engine never saw this request" with "the
        // engine saw it and rejected the forkchoice STATE we named", which have
        // opposite dispositions.
        self.engine
            .fork_choice_updated(state, None)
            .await
            .map_err(|error| match error {
                // reth PROCESSED the update and refused the state: the head hash
                // was zero, or it cannot find our `finalized`/`safe` hash in its
                // own canonical chain (engine/tree/src/tree/mod.rs
                // `validate_forkchoice_state` / `update_finalized_block` /
                // `update_safe_block` → `OnForkChoiceUpdated::invalid_state`).
                // That is a structurally PERMANENT local condition: every retry
                // re-sends the same unresolvable hashes. Classified
                // `Corruption` — loud actor death, latch NOT engaged — rather
                // than fork-safety, because it says this node's own anchor
                // disagrees with this node's own EL, not that the network
                // disagrees with the chain. It reached the executor as a
                // transport error before, i.e. it was retried forever.
                BeaconForkChoiceUpdateError::ForkchoiceUpdateError(inner) => {
                    EngineError::anchor_inconsistent(format_args!(
                        "reth rejected the forkchoice state: {inner}"
                    ))
                }
                // The engine task is gone / an internal reth error swallowed the
                // request: no verdict was rendered, so retrying is honest.
                // Deliberately NOT collapsed with the arm above.
                other => EngineError::transport(other),
            })
    }

    async fn import_derived(&self, data: DerivedExecution) -> Result<PayloadStatus, EngineError> {
        let executed = ExecutedBlock::new(
            Arc::new(data.recovered),
            Arc::new(data.output),
            ComputedTrieData {
                hashed_state: Arc::new(data.hashed_state.into_sorted()),
                trie_updates: Arc::new(data.trie_updates.into_sorted()),
                anchored_trie_input: None,
            },
        );
        self.tree
            .send(FromEngine::Request(EngineApiRequest::InsertExecutedBlock(
                executed,
            )))
            // A closed tree channel is a TRANSPORT failure — same class as
            // the FCU's engine-handle error. The executor degrades + defers
            // to reconvergence instead of actor-death (Decision A).
            .map_err(|_| EngineError::transport("engine tree channel closed"))?;
        // The insert is fire-and-forget into the tree's FIFO; the FCU
        // that follows it (same FIFO) surfaces any rejection.
        Ok(PayloadStatus::from_status(PayloadStatusEnum::Valid))
    }
}
