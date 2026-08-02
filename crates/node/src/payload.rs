use alloy_consensus::{BlockHeader, Header};
use alloy_primitives::{Bytes, B256};
use alloy_rpc_types_engine::PayloadAttributes as EthPayloadAttributes;
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

/// `PayloadBuilderBuilder` unit struct — the type plugged into
/// `BasicPayloadServiceBuilder<_>`. Mirrors reth's upstream
/// `EthereumPayloadBuilder` shape (`reth/crates/ethereum/node/src/payload.rs`),
/// differing only in force-emptying `extra_data` on a DPoS chain.
#[derive(Clone, Debug)]
pub struct FluentPayloadBuilder {
    dpos_active: bool,
}

impl FluentPayloadBuilder {
    pub fn new(dpos_active: bool) -> Self {
        Self { dpos_active }
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
        // On a DPoS chain the ONLY writer of header `extra_data` is DPoS
        // derivation (`derive.rs` copies the agreed `OrderBlock.extra_data`
        // verbatim); this builder never produces a block that should carry one.
        // But it DOES produce the block at the activation height — `launcher.rs`
        // halts the pre-DPoS sequencer only once its head has REACHED activation —
        // and the executor decodes `extra_data` from `>= activation`. reth's
        // default here is a non-empty `"reth/v…"` version string (~27 B), which
        // would fail-loud-decode as a malformed production record and halt every
        // node at the swap block. Force-empty so a forgotten
        // `--builder.extradata=""` cannot brick the bring-up.
        let extra_data = if self.dpos_active {
            if !conf.extra_data().is_empty() {
                tracing::warn!(
                    "DPoS mode: overriding non-empty --builder.extradata to empty \
                     (the activation-height block must carry an empty production \
                      record — a non-empty value fails the executor's decode)"
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
        })
    }
}

/// Concrete payload builder — runs at `try_build` time per requested payload.
/// Body mirrors reth's `EthereumPayloadBuilder::try_build`.
#[derive(Clone, Debug)]
pub struct FluentPayloadBuilderImpl<Pool, Client, EvmConfig> {
    client: Client,
    pool: Pool,
    evm_config: EvmConfig,
    base_config: EthereumBuilderConfig,
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
        let cfg = self.base_config.clone();
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
        let cfg = self.base_config.clone();
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
