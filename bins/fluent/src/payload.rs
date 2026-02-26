use alloy_consensus::{BlockHeader, Header};
use alloy_primitives::B256;
use fluentbase_types::PRECOMPILE_FEE_MANAGER;
use reth_ethereum_engine_primitives::EthPayloadAttributes;
use reth_payload_primitives::PayloadAttributesBuilder;
use reth_primitives_traits::SealedHeader;

/// The attributes builder for local Fluent payload.
#[derive(Default, Debug)]
pub struct FluentPayloadAttributesBuilder;

impl PayloadAttributesBuilder<EthPayloadAttributes, Header> for FluentPayloadAttributesBuilder {
    fn build(&self, parent: &SealedHeader<Header>) -> EthPayloadAttributes {
        let mut timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        timestamp = std::cmp::max(parent.timestamp().saturating_add(1), timestamp);
        EthPayloadAttributes {
            timestamp,
            prev_randao: B256::random(),
            suggested_fee_recipient: PRECOMPILE_FEE_MANAGER,
            withdrawals: Default::default(),
            parent_beacon_block_root: None,
        }
    }
}
