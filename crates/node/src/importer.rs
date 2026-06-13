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
//! `FLUENT_DPOS_IMPORT_MODE=new-payload` keeps the Phase A path (reth
//! re-executes and would reject a derivation whose roots diverge) as the
//! conformance / operator escape hatch.

use crate::derive::DerivedExecution;
use alloy_rpc_types_engine::{
    ForkchoiceState, ForkchoiceUpdated, PayloadStatus, PayloadStatusEnum,
};
use crossbeam_channel::Sender;
use fluentbase_consensus::BeaconEngineLike;
use reth_chain_state::{ComputedTrieData, ExecutedBlock};
use reth_engine_primitives::ConsensusEngineHandle;
use reth_engine_tree::engine::{EngineApiRequest, FromEngine};
use reth_ethereum_engine_primitives::EthEngineTypes;
use reth_ethereum_primitives::EthPrimitives;
use reth_payload_primitives::PayloadTypes;
use std::sync::Arc;

type TreeTx = Sender<
    FromEngine<EngineApiRequest<EthEngineTypes, EthPrimitives>, reth_ethereum_primitives::Block>,
>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImportMode {
    InsertExecuted,
    NewPayload,
}

#[derive(Clone, Debug)]
pub struct RethImporter {
    engine: ConsensusEngineHandle<EthEngineTypes>,
    tree: Option<TreeTx>,
    mode: ImportMode,
}

impl RethImporter {
    /// Resolve the import mode from `FLUENT_DPOS_IMPORT_MODE` and claim the
    /// engine-tree sender from the launch escrow. Fails loud when
    /// single-execution import is requested but the escrow is empty (a fork
    /// without the deposit, or a second claim in one process).
    pub fn from_env(engine: ConsensusEngineHandle<EthEngineTypes>) -> eyre::Result<Self> {
        let mode = match std::env::var("FLUENT_DPOS_IMPORT_MODE").as_deref() {
            Ok("new-payload") => ImportMode::NewPayload,
            Ok("insert") | Err(_) => ImportMode::InsertExecuted,
            Ok(other) => eyre::bail!(
                "FLUENT_DPOS_IMPORT_MODE={other:?} — expected \"insert\" or \"new-payload\""
            ),
        };
        let tree = reth_engine_tree::launch::tree_sender_escrow::take::<TreeTx>();
        if mode == ImportMode::InsertExecuted && tree.is_none() {
            eyre::bail!(
                "single-execution import requested but the engine-tree sender escrow is \
                 empty — reth fork without the launch deposit, or the sender was already \
                 claimed in this process (set FLUENT_DPOS_IMPORT_MODE=new-payload to fall \
                 back to re-execution)"
            );
        }
        tracing::info!(?mode, "DPoS block import mode");
        Ok(Self { engine, tree, mode })
    }
}

impl BeaconEngineLike for RethImporter {
    type ExecutionData = DerivedExecution;

    async fn fork_choice_updated(&self, state: ForkchoiceState) -> eyre::Result<ForkchoiceUpdated> {
        self.engine
            .fork_choice_updated(state, None)
            .await
            .map_err(|e| eyre::eyre!(e.to_string()))
    }

    async fn import_derived(&self, data: DerivedExecution) -> eyre::Result<PayloadStatus> {
        match self.mode {
            ImportMode::InsertExecuted => {
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
                    .as_ref()
                    .expect("checked at construction")
                    .send(FromEngine::Request(EngineApiRequest::InsertExecutedBlock(
                        executed,
                    )))
                    .map_err(|_| eyre::eyre!("engine tree channel closed"))?;
                // The insert is fire-and-forget into the tree's FIFO; the FCU
                // that follows it (same FIFO) surfaces any rejection.
                Ok(PayloadStatus::from_status(PayloadStatusEnum::Valid))
            }
            ImportMode::NewPayload => {
                let sealed = data.recovered.into_sealed_block();
                let payload = <EthEngineTypes as PayloadTypes>::block_to_payload(sealed);
                self.engine
                    .new_payload(payload)
                    .await
                    .map_err(|e| eyre::eyre!(e.to_string()))
            }
        }
    }
}
