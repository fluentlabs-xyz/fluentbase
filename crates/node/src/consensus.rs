use crate::types::FLUENT_MAXIMUM_EXTRA_DATA_SIZE;
use alloy_evm::block::BlockExecutionResult;
use fluentbase_types::PRECOMPILE_FEE_MANAGER;
use reth_chainspec::{EthChainSpec, EthereumHardforks};
use reth_consensus::{Consensus, ConsensusError, FullConsensus, HeaderValidator, ReceiptRootBloom};
use reth_ethereum_consensus::EthBeaconConsensus;
use reth_ethereum_primitives::EthPrimitives;
use reth_node_api::FullNodeTypes;
use reth_node_builder::{components::ConsensusBuilder, BuilderContext};
use reth_node_types::NodeTypes;
use reth_primitives_traits::{
    Block, BlockHeader, NodePrimitives, RecoveredBlock, SealedBlock, SealedHeader,
};
use std::{fmt::Debug, sync::Arc};

#[derive(Debug, Default, Clone, Copy)]
pub struct FluentConsensusBuilder {}

impl<Node> ConsensusBuilder<Node> for FluentConsensusBuilder
where
    Node: FullNodeTypes<
        Types: NodeTypes<ChainSpec: EthChainSpec + EthereumHardforks, Primitives = EthPrimitives>,
    >,
{
    type Consensus = Arc<FluentConsensus<<Node::Types as NodeTypes>::ChainSpec>>;

    async fn build_consensus(self, ctx: &BuilderContext<Node>) -> eyre::Result<Self::Consensus> {
        Ok(Arc::new(FluentConsensus::new(ctx.chain_spec())))
    }
}

#[derive(Debug, Clone)]
pub struct FluentConsensus<ChainSpec> {
    inner: EthBeaconConsensus<ChainSpec>,
}

impl<ChainSpec: EthChainSpec + EthereumHardforks> FluentConsensus<ChainSpec> {
    /// Create a new instance of [`EthBeaconConsensus`]
    pub const fn new(chain_spec: Arc<ChainSpec>) -> Self {
        Self {
            inner: EthBeaconConsensus::new(chain_spec)
                .with_max_extra_data_size(FLUENT_MAXIMUM_EXTRA_DATA_SIZE),
        }
    }

    /// Returns the chain spec associated with this consensus engine.
    pub const fn chain_spec(&self) -> &Arc<ChainSpec> {
        self.inner.chain_spec()
    }
}

impl<ChainSpec, N> FullConsensus<N> for FluentConsensus<ChainSpec>
where
    ChainSpec: Send + Sync + EthChainSpec<Header = N::BlockHeader> + EthereumHardforks + Debug,
    N: NodePrimitives,
{
    fn validate_block_post_execution(
        &self,
        block: &RecoveredBlock<N::Block>,
        result: &BlockExecutionResult<N::Receipt>,
        receipt_root_bloom: Option<ReceiptRootBloom>,
    ) -> Result<(), ConsensusError> {
        <EthBeaconConsensus<ChainSpec> as FullConsensus<N>>::validate_block_post_execution(
            &self.inner,
            block,
            result,
            receipt_root_bloom,
        )
    }
}

impl<B, ChainSpec> Consensus<B> for FluentConsensus<ChainSpec>
where
    B: Block,
    ChainSpec: EthChainSpec<Header = B::Header> + EthereumHardforks + Debug + Send + Sync,
{
    fn validate_body_against_header(
        &self,
        body: &B::Body,
        header: &SealedHeader<B::Header>,
    ) -> Result<(), ConsensusError> {
        <EthBeaconConsensus<ChainSpec> as Consensus<B>>::validate_body_against_header(
            &self.inner,
            body,
            header,
        )
    }

    fn validate_block_pre_execution(&self, block: &SealedBlock<B>) -> Result<(), ConsensusError> {
        self.inner.validate_block_pre_execution(block)?;

        // Make sure a header has correct coinbase, all fees must be accumulated
        // inside fee manager smart contract
        use alloy_consensus::BlockHeader;
        if block.header().beneficiary() != PRECOMPILE_FEE_MANAGER {
            return Err(ConsensusError::Other("malformed beneficiary".to_owned()));
        }

        Ok(())
    }
}

impl<H, ChainSpec> HeaderValidator<H> for FluentConsensus<ChainSpec>
where
    H: BlockHeader,
    ChainSpec: EthChainSpec<Header = H> + EthereumHardforks + Debug + Send + Sync,
{
    fn validate_header(&self, header: &SealedHeader<H>) -> Result<(), ConsensusError> {
        self.inner.validate_header(header)?;

        // Make sure a header has correct coinbase, all fees must be accumulated
        // inside fee manager smart contract
        if header.header().beneficiary() != PRECOMPILE_FEE_MANAGER {
            return Err(ConsensusError::Other("malformed beneficiary".to_owned()));
        }

        Ok(())
    }

    fn validate_header_against_parent(
        &self,
        header: &SealedHeader<H>,
        parent: &SealedHeader<H>,
    ) -> Result<(), ConsensusError> {
        self.inner.validate_header_against_parent(header, parent)
    }
}
