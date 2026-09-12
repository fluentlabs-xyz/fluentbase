use crate::types::FLUENT_MAXIMUM_EXTRA_DATA_SIZE;
use alloy_evm::block::BlockExecutionResult;
use fluentbase_types::{
    FLUENT_TESTNET_CHAIN_ID, PRECOMPILE_FEE_MANAGER, TESTNET_FEE_MANAGER_BLOCK,
};
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

    fn validate_beneficiary<H: BlockHeader>(&self, header: &H) -> Result<(), ConsensusError> {
        // Preserve canonical Testnet headers before fee-manager enforcement. All other
        // networks and subsequent Testnet blocks retain the strict rule.
        if self.chain_spec().chain().id() == FLUENT_TESTNET_CHAIN_ID
            && header.number() < TESTNET_FEE_MANAGER_BLOCK
        {
            return Ok(());
        }
        if header.beneficiary() != PRECOMPILE_FEE_MANAGER {
            return Err(ConsensusError::msg("malformed beneficiary".to_owned()));
        }
        Ok(())
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

        self.validate_beneficiary(block.header())
    }
}

impl<H, ChainSpec> HeaderValidator<H> for FluentConsensus<ChainSpec>
where
    H: BlockHeader,
    ChainSpec: EthChainSpec<Header = H> + EthereumHardforks + Debug + Send + Sync,
{
    fn validate_header(&self, header: &SealedHeader<H>) -> Result<(), ConsensusError> {
        self.inner.validate_header(header)?;

        self.validate_beneficiary(header.header())
    }

    fn validate_header_against_parent(
        &self,
        header: &SealedHeader<H>,
        parent: &SealedHeader<H>,
    ) -> Result<(), ConsensusError> {
        self.inner.validate_header_against_parent(header, parent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::Header;
    use alloy_primitives::{address, Address};
    use reth_chainspec::{Chain, ChainSpecBuilder};
    use reth_ethereum_primitives::Block;

    #[test]
    fn beneficiary_rule_preserves_testnet_history_in_both_validation_paths() {
        // Beneficiary from canonical Testnet block 21,755,351:
        // 0x30aac77c8bfc53b5f6e7bd80fa1c023f4a7df6ded0a22bbc51a97bfdb87b2354.
        let old_beneficiary = address!("6659090f873cf50ea000e89ec5616150a6d7f6e4");
        for chain_id in [FLUENT_TESTNET_CHAIN_ID, 25363, 1337] {
            let chain = ChainSpecBuilder::default()
                .chain(Chain::from(chain_id))
                .genesis(Default::default())
                .london_activated()
                .build();
            let consensus = FluentConsensus::new(Arc::new(chain));
            for number in [21_755_351, 21_755_352, 21_755_353] {
                for beneficiary in [old_beneficiary, PRECOMPILE_FEE_MANAGER, Address::ZERO] {
                    let header = Header {
                        number,
                        beneficiary,
                        gas_limit: 30_000_000,
                        base_fee_per_gas: Some(7),
                        ..Default::default()
                    };
                    let expected = beneficiary == PRECOMPILE_FEE_MANAGER
                        || (chain_id == FLUENT_TESTNET_CHAIN_ID && number == 21_755_351);
                    let sealed_header = SealedHeader::new_unhashed(header.clone());
                    let sealed_block = SealedBlock::new_unhashed(Block {
                        header,
                        body: Default::default(),
                    });
                    for result in [
                        consensus.validate_header(&sealed_header),
                        consensus.validate_block_pre_execution(&sealed_block),
                    ] {
                        assert_eq!(result.is_ok(), expected, "{chain_id}:{number}: {result:?}");
                        if let Err(error) = result {
                            assert!(error.to_string().contains("malformed beneficiary"));
                        }
                    }
                }
            }
        }
    }
}
