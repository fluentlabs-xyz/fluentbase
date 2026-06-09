//! Production adapter impls bridging `crates/consensus`'s `PayloadBuilderLike`
//! / `BeaconEngineLike` traits to reth's concrete handles (`PayloadBuilderHandle`,
//! `ConsensusEngineHandle`). The third trait `PayloadAttrsBuilderLike` is
//! implemented for `FluentPayloadAttributesBuilder` in the node crate (orphan
//! rule — the type lives there).
//!
//! These impls are required by `OuterBuilder::build`, which takes
//! `PB: PayloadBuilderLike` etc. as generic params; without these production
//! impls, only the test `Fake*` types would satisfy the bounds.

use crate::application::{BeaconEngineLike, PayloadBuilderLike};
use alloy_rpc_types_engine::{ForkchoiceState, ForkchoiceUpdated, PayloadStatus};
use reth_engine_primitives::ConsensusEngineHandle;
use reth_ethereum_primitives::Block as RethBlock;
use reth_payload_builder::PayloadBuilderHandle;
use reth_payload_primitives::{BuiltPayload, PayloadKind, PayloadTypes};
use reth_primitives_traits::{NodePrimitives, SealedBlock};

impl<T> PayloadBuilderLike for PayloadBuilderHandle<T>
where
    T: PayloadTypes + Send + Sync + 'static,
    T::BuiltPayload:
        BuiltPayload<Primitives: NodePrimitives<Block = RethBlock>> + Send + Sync + 'static,
{
    type BuiltSealed = SealedBlock<RethBlock>;

    async fn resolve_kind(
        &self,
        id: alloy_rpc_types_engine::PayloadId,
        kind: PayloadKind,
    ) -> Option<eyre::Result<Self::BuiltSealed>> {
        let opt = PayloadBuilderHandle::<T>::resolve_kind(self, id, kind).await;
        opt.map(|res| match res {
            Ok(payload) => Ok(payload.block().clone()),
            Err(e) => Err(eyre::eyre!(e.to_string())),
        })
    }
}

impl<T> BeaconEngineLike for ConsensusEngineHandle<T>
where
    T: PayloadTypes + Send + Sync + 'static,
    T::PayloadAttributes: Send + 'static,
    T::BuiltPayload: BuiltPayload<Primitives: NodePrimitives<Block = RethBlock>>,
{
    type PayloadAttrs = T::PayloadAttributes;
    type ExecutionData = SealedBlock<RethBlock>;

    async fn fork_choice_updated(
        &self,
        state: ForkchoiceState,
        attrs: Option<Self::PayloadAttrs>,
    ) -> eyre::Result<ForkchoiceUpdated> {
        ConsensusEngineHandle::<T>::fork_choice_updated(self, state, attrs)
            .await
            .map_err(|e| eyre::eyre!(e.to_string()))
    }

    async fn new_payload(&self, data: Self::ExecutionData) -> eyre::Result<PayloadStatus> {
        // `T::block_to_payload` converts the EL-domain SealedBlock into the
        // engine-domain ExecutionData (e.g. wraps with sidecar for EthEngineTypes).
        let exec = <T as PayloadTypes>::block_to_payload(data);
        ConsensusEngineHandle::<T>::new_payload(self, exec)
            .await
            .map_err(|e| eyre::eyre!(e.to_string()))
    }
}
