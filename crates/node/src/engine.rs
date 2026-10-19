//! Fluent Engine API policy and add-ons.

use crate::{evm::FluentNode, types::FLUENT_MAXIMUM_EXTRA_DATA_SIZE};
use alloy_rpc_types_engine::{ExecutionData, PayloadAttributes};
use reth_chainspec::{EthereumHardforks, Hardforks};
use reth_engine_primitives::EngineTypes;
use reth_ethereum_primitives::EthPrimitives;
use reth_node_api::{AddOnsContext, FullNodeComponents};
use reth_node_builder::rpc::{
    BasicEngineApiBuilder, BasicEngineValidatorBuilder, EthApiBuilder, Identity,
    PayloadValidatorBuilder, RpcAddOns,
};
use reth_node_ethereum::{EthereumAddOns, EthereumEngineValidator, EthereumEthApiBuilder};
use reth_node_types::NodeTypes;
use reth_payload_primitives::PayloadTypes;
use std::sync::Arc;

/// Applies Fluent's extra-data policy to both Engine RPC validation and block import.
#[derive(Debug, Default, Clone, Copy)]
pub struct FluentEngineValidatorBuilder;

impl FluentEngineValidatorBuilder {
    fn validator<C>(chain_spec: Arc<C>) -> EthereumEngineValidator<C> {
        EthereumEngineValidator::new(chain_spec)
            .with_max_extra_data_size(FLUENT_MAXIMUM_EXTRA_DATA_SIZE)
    }
}

impl<Node, Types> PayloadValidatorBuilder<Node> for FluentEngineValidatorBuilder
where
    Types: NodeTypes<
        ChainSpec: Hardforks + EthereumHardforks + Clone + 'static,
        Payload: EngineTypes<ExecutionData = ExecutionData>
                     + PayloadTypes<PayloadAttributes = PayloadAttributes>,
        Primitives = EthPrimitives,
    >,
    Node: FullNodeComponents<Types = Types>,
{
    type Validator = EthereumEngineValidator<Types::ChainSpec>;

    async fn build(self, ctx: &AddOnsContext<'_, Node>) -> eyre::Result<Self::Validator> {
        Ok(Self::validator(ctx.config.chain.clone()))
    }
}

/// Uses the same Fluent validator for Engine RPC, the execution tree and Flashbots validation.
/// SSZ Engine transport uses Ethereum's fixed 32-byte schema and remains disabled for Fluent.
pub fn fluent_add_ons<N>() -> EthereumAddOns<N, EthereumEthApiBuilder, FluentEngineValidatorBuilder>
where
    N: FullNodeComponents<Types = FluentNode>,
    EthereumEthApiBuilder: EthApiBuilder<N>,
{
    EthereumAddOns::new(RpcAddOns::new(
        EthereumEthApiBuilder::default(),
        FluentEngineValidatorBuilder,
        BasicEngineApiBuilder::default(),
        BasicEngineValidatorBuilder::default(),
        Default::default(),
        Identity::new(),
    ))
    .with_engine_ssz_enabled(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::B256;
    use alloy_rpc_types_engine::PayloadError;
    use reth_chainspec::ChainSpecBuilder;
    use reth_engine_primitives::PayloadValidator;
    use reth_ethereum_engine_primitives::EthEngineTypes;
    use reth_ethereum_primitives::Block;
    use reth_primitives_traits::Block as _;

    fn payload(extra_data_size: usize) -> ExecutionData {
        let mut block = Block::default();
        block.header.base_fee_per_gas = Some(1);
        block.header.extra_data = vec![42; extra_data_size].into();
        <EthEngineTypes as PayloadTypes>::block_to_payload(block.seal_slow(), None)
    }

    #[test]
    fn engine_payload_roundtrip_preserves_fluent_extra_data_and_hash_validation() {
        let spec = Arc::new(ChainSpecBuilder::mainnet().build());
        let validator = FluentEngineValidatorBuilder::validator(spec);
        // The execution tree, Engine RPC and Flashbots all call this PayloadValidator surface.
        let validator: &dyn PayloadValidator<EthEngineTypes, Block = Block> = &validator;
        for size in [32, 33, 65] {
            let payload = payload(size);
            let expected_hash = payload.block_hash();
            let block = validator.convert_payload_to_block(payload).unwrap();
            assert_eq!(block.header().extra_data.len(), size);
            assert_eq!(block.hash(), expected_hash);
        }
        let err = validator.convert_payload_to_block(payload(66)).unwrap_err();
        assert!(matches!(
            err,
            reth_payload_primitives::NewPayloadError::Eth(PayloadError::ExtraData(_))
        ));

        let mut bad_hash = payload(65);
        bad_hash.payload.as_v1_mut().block_hash = B256::ZERO;
        let err = validator.convert_payload_to_block(bad_hash).unwrap_err();
        assert!(matches!(
            err,
            reth_payload_primitives::NewPayloadError::Eth(PayloadError::BlockHash { .. })
        ));

        // Sender recovery is the entry point used by Flashbots when no shared cache is present.
        assert!(validator.ensure_well_formed_payload(payload(65)).is_ok());
        assert!(validator.ensure_well_formed_payload(payload(66)).is_err());
    }
    #[test]
    fn prague_engine_payload_roundtrip_retains_cancun_and_requests_fields() {
        let spec = Arc::new(ChainSpecBuilder::mainnet().prague_activated().build());
        let validator = FluentEngineValidatorBuilder::validator(spec);
        let validator: &dyn PayloadValidator<EthEngineTypes, Block = Block> = &validator;
        for size in [65, 66] {
            let mut block = Block::default();
            block.header.base_fee_per_gas = Some(1);
            block.header.extra_data = vec![42; size].into();
            block.header.withdrawals_root = Some(alloy_consensus::constants::EMPTY_WITHDRAWALS);
            block.body.withdrawals = Some(Default::default());
            block.header.blob_gas_used = Some(0);
            block.header.excess_blob_gas = Some(0);
            block.header.parent_beacon_block_root = Some(B256::ZERO);
            block.header.requests_hash = Some(alloy_primitives::b256!(
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
            ));
            let expected = block.seal_slow();
            let payload =
                <EthEngineTypes as PayloadTypes>::block_to_payload(expected.clone(), None);
            assert!(payload.sidecar.cancun().is_some());
            assert!(payload.sidecar.prague().is_some());
            let result = validator.convert_payload_to_block(payload);
            if size == 65 {
                assert_eq!(result.unwrap(), expected);
            } else {
                assert!(matches!(
                    result,
                    Err(reth_payload_primitives::NewPayloadError::Eth(
                        PayloadError::ExtraData(_)
                    ))
                ));
            }
        }
    }
}
