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
use fluentbase_consensus::{BeaconEngineLike, TransportError};
use reth_chain_state::{ComputedTrieData, ExecutedBlock};
use reth_engine_primitives::ConsensusEngineHandle;
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
    ) -> Result<ForkchoiceUpdated, TransportError> {
        // The engine-handle send error is a TRANSPORT failure; the FCU verdict
        // (incl. `Invalid`) rides in the `Ok`. This is the concrete-error → typed
        // taxonomy boundary: the display is captured HERE, next to the reth type.
        self.engine
            .fork_choice_updated(state, None)
            .await
            .map_err(TransportError::new)
    }

    async fn import_derived(
        &self,
        data: DerivedExecution,
    ) -> Result<PayloadStatus, TransportError> {
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
            .map_err(|_| TransportError::new("engine tree channel closed"))?;
        // The insert is fire-and-forget into the tree's FIFO; the FCU
        // that follows it (same FIFO) surfaces any rejection.
        Ok(PayloadStatus::from_status(PayloadStatusEnum::Valid))
    }
}
