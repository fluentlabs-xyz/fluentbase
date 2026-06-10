use std::sync::Arc;

use alloy_consensus::{BlockHeader, Header};
use alloy_primitives::{Bytes, B256};
use alloy_rpc_types_engine::PayloadAttributes as EthPayloadAttributes;
use alloy_rpc_types_engine::PayloadId;
use dashmap::DashMap;
use fluentbase_consensus::PayloadAttrsBuilderLike;
use fluentbase_types::PRECOMPILE_FEE_MANAGER;
use reth_basic_payload_builder::{
    BuildArguments, BuildOutcome, MissingPayloadBehaviour, PayloadBuilder, PayloadConfig,
};
use reth_chainspec::EthereumHardforks;
use reth_chainspec::{ChainSpecProvider, EthChainSpec};
use reth_ethereum_engine_primitives::EthBuiltPayload;
use reth_ethereum_payload_builder::{default_ethereum_payload, EthereumBuilderConfig};
use reth_ethereum_primitives::EthPrimitives;
use reth_evm::{ConfigureEvm, NextBlockEnvAttributes};
use reth_node_api::{FullNodeTypes, NodeTypes, PrimitivesTy, TxTy};
use reth_node_builder::{
    components::PayloadBuilderBuilder, BuilderContext, PayloadBuilderConfig, PayloadTypes,
};
use reth_payload_builder_primitives::PayloadBuilderError;
use reth_payload_primitives::PayloadAttributesBuilder;
use reth_primitives_traits::SealedHeader;
use reth_storage_api::StateProviderFactory;
use reth_transaction_pool::{PoolTransaction, TransactionPool};

/// The attributes builder for local Fluent payload.
#[derive(Default, Debug, Clone)]
pub struct FluentPayloadAttributesBuilder;

impl FluentPayloadAttributesBuilder {
    fn build_attrs(parent_timestamp: u64) -> EthPayloadAttributes {
        let mut timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before UNIX_EPOCH")
            .as_secs();
        timestamp = std::cmp::max(parent_timestamp.saturating_add(1), timestamp);
        EthPayloadAttributes {
            timestamp,
            prev_randao: B256::random(),
            suggested_fee_recipient: PRECOMPILE_FEE_MANAGER,
            withdrawals: Default::default(),
            parent_beacon_block_root: Some(B256::ZERO),
            slot_number: None,
        }
    }
}

impl PayloadAttributesBuilder<EthPayloadAttributes, Header> for FluentPayloadAttributesBuilder {
    fn build(&self, parent: &SealedHeader<Header>) -> EthPayloadAttributes {
        Self::build_attrs(parent.timestamp())
    }
}

// `PayloadAttrsBuilderLike` is the consensus-side trait
// used by `FluentApp::propose`, which calls `build(parent.header())` on a raw
// `&Header` (not `&SealedHeader<Header>`; see `SealedBlock::header` returns
// `&B::Header`). This impl shares the body with the reth-side
// `PayloadAttributesBuilder` impl via `build_attrs`.
impl PayloadAttrsBuilderLike for FluentPayloadAttributesBuilder {
    type Attrs = EthPayloadAttributes;
    type Header = Header;

    fn build(&self, parent: &Header) -> EthPayloadAttributes {
        Self::build_attrs(parent.timestamp())
    }

    fn payload_id(&self, parent_hash: B256, attrs: &EthPayloadAttributes) -> PayloadId {
        reth_payload_primitives::payload_id(&parent_hash, attrs)
    }
}

/// `PayloadBuilderBuilder` unit struct — the type plugged into
/// `BasicPayloadServiceBuilder<_>`. Carries the shared
/// `extra_data_registry` so that the concrete `FluentPayloadBuilderImpl`
/// built at node-init time has access to it.
///
/// Mirrors reth's upstream `EthereumPayloadBuilder` unit struct shape
/// (`reth/crates/ethereum/node/src/payload.rs`) but routes through the
/// Fluent impl that consults the registry per `PayloadId`.
#[derive(Clone, Debug)]
pub struct FluentPayloadBuilder {
    extra_data_registry: Arc<DashMap<PayloadId, Bytes>>,
    dpos_active: bool,
}

impl FluentPayloadBuilder {
    pub fn new(extra_data_registry: Arc<DashMap<PayloadId, Bytes>>, dpos_active: bool) -> Self {
        Self {
            extra_data_registry,
            dpos_active,
        }
    }
}

impl<Types, Node, Pool, Evm> PayloadBuilderBuilder<Node, Pool, Evm> for FluentPayloadBuilder
where
    Types: NodeTypes<ChainSpec: EthereumHardforks, Primitives = EthPrimitives>,
    Node: FullNodeTypes<Types = Types>,
    Pool: TransactionPool<Transaction: PoolTransaction<Consensus = TxTy<Node::Types>>>
        + Unpin
        + 'static,
    Evm: ConfigureEvm<Primitives = PrimitivesTy<Types>, NextBlockEnvCtx = NextBlockEnvAttributes>
        + 'static,
    Types::Payload:
        PayloadTypes<BuiltPayload = EthBuiltPayload, PayloadAttributes = EthPayloadAttributes>,
{
    type PayloadBuilder = FluentPayloadBuilderImpl<Pool, Node::Provider, Evm>;

    async fn build_payload_builder(
        self,
        ctx: &BuilderContext<Node>,
        pool: Pool,
        evm_config: Evm,
    ) -> eyre::Result<Self::PayloadBuilder> {
        let conf = ctx.payload_builder_config();
        let chain = ctx.chain_spec().chain();
        let gas_limit = conf.gas_limit_for(chain);
        // DPoS stamps the liveness cert into header extra_data per payload via
        // the registry; the base extra_data MUST be empty or a registry miss
        // (`config_for` falls back to base_config) ships the operator's
        // `--builder.extradata` default (~27 B), which the verifier rejects
        // (LengthMismatch) and stalls block production. Force-empty here so the
        // operator can't brick the node by forgetting `--builder.extradata=""`.
        let extra_data = if self.dpos_active {
            if !conf.extra_data().is_empty() {
                tracing::warn!(
                    "DPoS mode: overriding non-empty --builder.extradata to empty \
                     (liveness-cert stamping requires empty base extra_data)"
                );
            }
            Bytes::new()
        } else {
            conf.extra_data()
        };
        let base_config = EthereumBuilderConfig::new()
            .with_gas_limit(gas_limit)
            .with_max_blobs_per_block(conf.max_blobs_per_block())
            .with_extra_data(extra_data);
        Ok(FluentPayloadBuilderImpl {
            client: ctx.provider().clone(),
            pool,
            evm_config,
            base_config,
            extra_data_registry: self.extra_data_registry,
        })
    }
}

/// Concrete payload builder — runs at `try_build` time per requested
/// payload. Looks up `extra_data_registry[args.config.attributes.id]` and,
/// when present, swaps the `base_config.extra_data` for it; otherwise
/// falls back to `base_config.extra_data`. Body otherwise mirrors
/// reth's `EthereumPayloadBuilder::try_build`.
#[derive(Clone, Debug)]
pub struct FluentPayloadBuilderImpl<Pool, Client, EvmConfig> {
    client: Client,
    pool: Pool,
    evm_config: EvmConfig,
    base_config: EthereumBuilderConfig,
    extra_data_registry: Arc<DashMap<PayloadId, Bytes>>,
}

impl<Pool, Client, EvmConfig> FluentPayloadBuilderImpl<Pool, Client, EvmConfig> {
    fn config_for(&self, id: PayloadId) -> EthereumBuilderConfig {
        let mut cfg = self.base_config.clone();
        if let Some(extra) = self.extra_data_registry.get(&id) {
            cfg = cfg.with_extra_data(extra.value().clone());
        }
        cfg
    }
}

impl<Pool, Client, EvmConfig> PayloadBuilder for FluentPayloadBuilderImpl<Pool, Client, EvmConfig>
where
    EvmConfig: ConfigureEvm<Primitives = EthPrimitives, NextBlockEnvCtx = NextBlockEnvAttributes>,
    Client: StateProviderFactory + ChainSpecProvider<ChainSpec: EthereumHardforks> + Clone,
    Pool: TransactionPool<
            Transaction: PoolTransaction<Consensus = reth_ethereum_primitives::TransactionSigned>,
        > + Clone,
{
    type Attributes = EthPayloadAttributes;
    type BuiltPayload = EthBuiltPayload;

    fn try_build(
        &self,
        args: BuildArguments<EthPayloadAttributes, EthBuiltPayload>,
    ) -> Result<BuildOutcome<EthBuiltPayload>, PayloadBuilderError> {
        let cfg = self.config_for(args.config.payload_id);
        let pool = self.pool.clone();
        default_ethereum_payload(
            self.evm_config.clone(),
            self.client.clone(),
            self.pool.clone(),
            cfg,
            args,
            move |attrs| pool.best_transactions_with_attributes(attrs),
        )
    }

    fn on_missing_payload(
        &self,
        _args: BuildArguments<Self::Attributes, Self::BuiltPayload>,
    ) -> MissingPayloadBehaviour<Self::BuiltPayload> {
        MissingPayloadBehaviour::RaceEmptyPayload
    }

    fn build_empty_payload(
        &self,
        config: PayloadConfig<Self::Attributes>,
    ) -> Result<EthBuiltPayload, PayloadBuilderError> {
        let cfg = self.config_for(config.payload_id);
        let pool = self.pool.clone();
        let args = BuildArguments::new(
            Default::default(),
            Default::default(),
            None,
            config,
            Default::default(),
            None,
        );
        default_ethereum_payload(
            self.evm_config.clone(),
            self.client.clone(),
            self.pool.clone(),
            cfg,
            args,
            move |attrs| pool.best_transactions_with_attributes(attrs),
        )?
        .into_payload()
        .ok_or_else(|| PayloadBuilderError::MissingPayload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_hit_swaps_extra_data() {
        // We can't easily instantiate the full reth client/pool/evm chain
        // in a unit test, but we CAN exercise `config_for` directly to
        // assert the registry lookup logic.
        let registry: Arc<DashMap<PayloadId, Bytes>> = Arc::new(DashMap::new());
        let id = PayloadId::new([1u8; 8]);
        let custom = Bytes::from_static(b"liveness-bitmap");
        registry.insert(id, custom.clone());

        // Use the bare type with stub generics — we only exercise
        // `config_for`, which is generic-free at the call site.
        let builder = FluentPayloadBuilderImpl::<(), (), ()> {
            client: (),
            pool: (),
            evm_config: (),
            base_config: EthereumBuilderConfig::new()
                .with_extra_data(Bytes::from_static(b"default")),
            extra_data_registry: registry.clone(),
        };
        let hit = builder.config_for(id);
        assert_eq!(hit.extra_data, custom);

        let missing_id = PayloadId::new([2u8; 8]);
        let miss = builder.config_for(missing_id);
        assert_eq!(miss.extra_data, Bytes::from_static(b"default"));
    }
}
