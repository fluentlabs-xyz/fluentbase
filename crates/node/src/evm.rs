//! Ethereum EVM implementation.

use crate::{
    consensus::FluentConsensusBuilder,
    payload::{FluentPayloadAttributesBuilder, FluentPayloadBuilder},
};
use alloy_consensus::{Header, TxType};
use alloy_evm::{
    block::{
        BlockExecutionError, BlockExecutionResult, BlockExecutor, BlockExecutorFactory, GasOutput,
        OnStateHook, StateDB,
    },
    env::EvmEnv,
    eth::{EthBlockExecutionCtx, EthBlockExecutor, EthTxResult},
    evm::EvmFactory,
    precompiles::PrecompilesMap,
    Database, Evm,
};
use alloy_primitives::{Address, Bytes, B256};
use alloy_rpc_types_engine::{ExecutionData, PayloadAttributes as EthPayloadAttributes};
use core::{
    fmt::Debug,
    ops::{Deref, DerefMut},
};
use fluentbase_revm::{
    revm::{
        context::{BlockEnv, CfgEnv, TxEnv},
        context_interface::result::{EVMError, HaltReason, ResultAndState},
        handler::{instructions::EthInstructions, EthPrecompiles, PrecompileProvider},
        inspector::NoOpInspector,
        interpreter::{interpreter::EthInterpreter, InterpreterResult},
        primitives::hardfork::SpecId,
        Context, ExecuteEvm, InspectEvm, Inspector, SystemCallEvm,
    },
    DefaultRwasm, RwasmBuilder, RwasmEvm, RwasmFrame, RwasmPrecompiles,
};
use reth_chainspec::ChainSpec;
use reth_ethereum_engine_primitives::{EthBuiltPayload, EthEngineTypes};
use reth_ethereum_primitives::{EthPrimitives, Receipt, TransactionSigned};
use reth_evm::{
    block::ExecutableTx, ConfigureEngineEvm, ConfigureEvm, EvmEnvFor, ExecutableTxIterator,
    ExecutionCtxFor, NextBlockEnvAttributes,
};
use reth_evm_ethereum::{EthBlockAssembler, EthEvmConfig, RethReceiptBuilder};
use reth_node_api::{FullNodeComponents, FullNodeTypes};
use reth_node_builder::{
    components::{BasicPayloadServiceBuilder, ComponentsBuilder, ExecutorBuilder},
    BuilderContext, DebugNode, Node, NodeAdapter,
};
use reth_node_ethereum::{
    EthereumAddOns, EthereumEngineValidatorBuilder, EthereumEthApiBuilder, EthereumNetworkBuilder,
    EthereumPoolBuilder,
};
use reth_node_types::NodeTypes;
use reth_payload_primitives::{PayloadAttributesBuilder, PayloadTypes};
use reth_primitives_traits::{BlockTy, SealedBlock, SealedHeader};
use reth_provider::providers::ProviderFactoryBuilder;
use reth_storage_api::EthStorage;
use std::{convert::Infallible, sync::Arc};

/// The Ethereum EVM context type.
pub type EthRwasmContext<DB> = Context<BlockEnv, TxEnv, CfgEnv, DB>;

/// Ethereum EVM implementation.
///
/// This is a wrapper type around the `revm` ethereum evm with optional [`Inspector`] (tracing)
/// support. [`Inspector`] support is configurable at runtime because it's part of the underlying
/// `RwasmEvm` type.
#[expect(missing_debug_implementations)]
pub struct FluentEvmExecutor<DB: Database, I, PRECOMPILE = EthPrecompiles> {
    inner: RwasmEvm<
        EthRwasmContext<DB>,
        I,
        EthInstructions<EthInterpreter, EthRwasmContext<DB>>,
        PRECOMPILE,
        RwasmFrame,
    >,
    inspect: bool,
}

impl<DB: Database, I, PRECOMPILE> FluentEvmExecutor<DB, I, PRECOMPILE> {
    /// Creates a new Ethereum EVM instance.
    ///
    /// The `inspect` argument determines whether the configured [`Inspector`] of the given
    /// `RwasmEvm` should be invoked on `Evm::transact`.
    pub const fn new(
        evm: RwasmEvm<
            EthRwasmContext<DB>,
            I,
            EthInstructions<EthInterpreter, EthRwasmContext<DB>>,
            PRECOMPILE,
        >,
        inspect: bool,
    ) -> Self {
        Self {
            inner: evm,
            inspect,
        }
    }

    /// Consumes self and return the inner EVM instance.
    pub fn into_inner(
        self,
    ) -> RwasmEvm<
        EthRwasmContext<DB>,
        I,
        EthInstructions<EthInterpreter, EthRwasmContext<DB>>,
        PRECOMPILE,
        RwasmFrame,
    > {
        self.inner
    }

    /// Provides a reference to the EVM context.
    pub fn ctx(&self) -> &EthRwasmContext<DB> {
        &self.inner.0.ctx
    }

    /// Provides a mutable reference to the EVM context.
    pub fn ctx_mut(&mut self) -> &mut EthRwasmContext<DB> {
        &mut self.inner.0.ctx
    }
}

impl<DB: Database, I, PRECOMPILE> Deref for FluentEvmExecutor<DB, I, PRECOMPILE> {
    type Target = EthRwasmContext<DB>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.ctx()
    }
}

impl<DB: Database, I, PRECOMPILE> DerefMut for FluentEvmExecutor<DB, I, PRECOMPILE> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.ctx_mut()
    }
}

impl<DB, I, PRECOMPILE> Evm for FluentEvmExecutor<DB, I, PRECOMPILE>
where
    DB: Database,
    I: Inspector<EthRwasmContext<DB>>,
    PRECOMPILE: PrecompileProvider<EthRwasmContext<DB>, Output = InterpreterResult>,
{
    type DB = DB;
    type Tx = TxEnv;
    type Error = EVMError<DB::Error>;
    type HaltReason = HaltReason;
    type Spec = SpecId;
    type BlockEnv = BlockEnv;
    type Precompiles = PRECOMPILE;
    type Inspector = I;

    fn block(&self) -> &BlockEnv {
        &self.block
    }

    fn cfg_env(&self) -> &CfgEnv<Self::Spec> {
        &self.cfg
    }

    fn chain_id(&self) -> u64 {
        self.cfg.chain_id
    }

    fn transact_raw(&mut self, tx: Self::Tx) -> Result<ResultAndState, Self::Error> {
        if self.inspect {
            self.inner.inspect_tx(tx)
        } else {
            self.inner.transact(tx)
        }
    }

    fn transact_system_call(
        &mut self,
        caller: Address,
        contract: Address,
        data: Bytes,
    ) -> Result<ResultAndState, Self::Error> {
        self.inner.system_call_with_caller(caller, contract, data)
    }

    fn db_mut(&mut self) -> &mut Self::DB {
        &mut self.journaled_state.database
    }

    fn finish(self) -> (Self::DB, EvmEnv<Self::Spec>) {
        let Context {
            block: block_env,
            cfg: cfg_env,
            journaled_state,
            ..
        } = self.inner.0.ctx;

        (journaled_state.database, EvmEnv { block_env, cfg_env })
    }

    fn set_inspector_enabled(&mut self, enabled: bool) {
        self.inspect = enabled;
    }

    fn precompiles(&self) -> &Self::Precompiles {
        &self.inner.0.precompiles
    }

    fn precompiles_mut(&mut self) -> &mut Self::Precompiles {
        &mut self.inner.0.precompiles
    }

    fn inspector(&self) -> &Self::Inspector {
        &self.inner.0.inspector
    }

    fn inspector_mut(&mut self) -> &mut Self::Inspector {
        &mut self.inner.0.inspector
    }

    fn components(&self) -> (&Self::DB, &Self::Inspector, &Self::Precompiles) {
        (
            &self.inner.0.ctx.journaled_state.database,
            &self.inner.0.inspector,
            &self.inner.0.precompiles,
        )
    }

    fn components_mut(&mut self) -> (&mut Self::DB, &mut Self::Inspector, &mut Self::Precompiles) {
        (
            &mut self.inner.0.ctx.journaled_state.database,
            &mut self.inner.0.inspector,
            &mut self.inner.0.precompiles,
        )
    }
}

/// Factory producing [`FluentEvmExecutor`].
#[derive(Debug, Default, Clone, Copy)]
#[non_exhaustive]
pub struct FluentEvmFactory;

impl EvmFactory for FluentEvmFactory {
    type Evm<DB: Database, I: Inspector<EthRwasmContext<DB>>> =
        FluentEvmExecutor<DB, I, Self::Precompiles>;
    type Context<DB: Database> = Context<BlockEnv, TxEnv, CfgEnv, DB>;
    type Tx = TxEnv;
    type Error<DBError: core::error::Error + Send + Sync + 'static> = EVMError<DBError>;
    type HaltReason = HaltReason;
    type Spec = SpecId;
    type BlockEnv = BlockEnv;
    type Precompiles = PrecompilesMap;

    fn create_evm<DB: Database>(&self, db: DB, input: EvmEnv) -> Self::Evm<DB, NoOpInspector> {
        let spec_id = input.cfg_env.spec;
        FluentEvmExecutor {
            inner: Context::rwasm()
                .with_block(input.block_env)
                .with_cfg(input.cfg_env)
                .with_db(db)
                .build_rwasm_with_inspector(NoOpInspector {})
                .with_precompiles(PrecompilesMap::from_static(
                    RwasmPrecompiles::new_with_spec(spec_id).precompiles(),
                )),
            inspect: false,
        }
    }

    fn create_evm_with_inspector<DB: Database, I: Inspector<Self::Context<DB>>>(
        &self,
        db: DB,
        input: EvmEnv,
        inspector: I,
    ) -> Self::Evm<DB, I> {
        let spec_id = input.cfg_env.spec;
        FluentEvmExecutor {
            inner: Context::rwasm()
                .with_block(input.block_env)
                .with_cfg(input.cfg_env)
                .with_db(db)
                .build_rwasm_with_inspector(inspector)
                .with_precompiles(PrecompilesMap::from_static(
                    RwasmPrecompiles::new_with_spec(spec_id).precompiles(),
                )),
            inspect: true,
        }
    }
}

/// Builds a regular ethereum block executor that uses the custom EVM.
///
/// Carries operator-supplied `staking_address` + `chain_config_address`
/// so [`FluentBlockExecutor::apply_pre_execution_changes`] can issue the
/// `commitEpochCommittee` system call at epoch boundaries. Non-DPoS chains
/// pass [`Address::ZERO`] and the system call short-circuits.
#[derive(Debug, Clone, Copy)]
pub struct FluentExecutorBuilder {
    staking_address: Address,
    chain_config_address: Address,
    liveness_slashing_address: Address,
}

impl Default for FluentExecutorBuilder {
    fn default() -> Self {
        Self {
            staking_address: Address::ZERO,
            chain_config_address: Address::ZERO,
            liveness_slashing_address: fluentbase_types::PRECOMPILE_LIVENESS_SLASHING,
        }
    }
}

impl FluentExecutorBuilder {
    pub const fn new(
        staking_address: Address,
        chain_config_address: Address,
        liveness_slashing_address: Address,
    ) -> Self {
        Self {
            staking_address,
            chain_config_address,
            liveness_slashing_address,
        }
    }
}

impl<Node> ExecutorBuilder<Node> for FluentExecutorBuilder
where
    Node: FullNodeTypes<Types: NodeTypes<ChainSpec = ChainSpec, Primitives = EthPrimitives>>,
{
    type EVM = FluentEvmConfig;

    async fn build_evm(self, ctx: &BuilderContext<Node>) -> eyre::Result<Self::EVM> {
        let evm_config = FluentEvmConfig::new(
            ctx.chain_spec(),
            FluentEvmFactory::default(),
            self.staking_address,
            self.chain_config_address,
            self.liveness_slashing_address,
        );
        Ok(evm_config)
    }
}

#[derive(Debug, Clone)]
pub struct FluentEvmConfig {
    /// Inner evm config
    pub inner: EthEvmConfig<ChainSpec, FluentEvmFactory>,
    /// Staking contract address (per-network, operator-supplied via
    /// `StakingReaderConfig.staking_address`). `Address::ZERO` disables the
    /// `commitEpochCommittee` system call (non-DPoS chains).
    staking_address: Address,
    /// ChainConfig contract address. `Address::ZERO` disables the epoch
    /// system call.
    chain_config_address: Address,
    /// Liveness predeploy address the `recordProduction` system call
    /// targets. Operator-supplied via `StakingReaderConfig`; defaults to the
    /// canonical predeploy slot.
    liveness_slashing_address: Address,
}

impl FluentEvmConfig {
    /// Create a new [`FluentEvmConfig`] with the given chain spec, EVM factory,
    /// and the operator-supplied staking + chain_config + liveness addresses.
    pub fn new(
        chain_spec: Arc<ChainSpec>,
        evm_factory: FluentEvmFactory,
        staking_address: Address,
        chain_config_address: Address,
        liveness_slashing_address: Address,
    ) -> Self {
        let inner = EthEvmConfig::new_with_evm_factory(chain_spec.clone(), evm_factory);
        Self {
            inner,
            staking_address,
            chain_config_address,
            liveness_slashing_address,
        }
    }
}

impl FluentEvmConfig {
    /// Create a new [`FluentEvmConfig`] with the given chain spec and default
    /// EVM factory. Staking + ChainConfig addresses default to
    /// [`Address::ZERO`] (non-DPoS path); the liveness address defaults to the
    /// canonical predeploy slot.
    pub fn new_with_default_factory(chain_spec: Arc<ChainSpec>) -> Self {
        Self::new(
            chain_spec,
            FluentEvmFactory::default(),
            Address::ZERO,
            Address::ZERO,
            fluentbase_types::PRECOMPILE_LIVENESS_SLASHING,
        )
    }

    /// Returns the chain spec
    pub const fn chain_spec(&self) -> &Arc<ChainSpec> {
        self.inner.chain_spec()
    }

    /// Returns the inner EVM config
    pub const fn inner(&self) -> &EthEvmConfig<ChainSpec, FluentEvmFactory> {
        &self.inner
    }

    /// Returns the Staking contract address for `commitEpochCommittee` calls.
    pub const fn staking_address(&self) -> Address {
        self.staking_address
    }

    /// Returns the ChainConfig contract address (used to read
    /// `epochBlockInterval` at the same pre-execution state).
    pub const fn chain_config_address(&self) -> Address {
        self.chain_config_address
    }
}

impl BlockExecutorFactory for FluentEvmConfig {
    type EvmFactory = FluentEvmFactory;
    type TxExecutionResult = EthTxResult<HaltReason, TxType>;
    type ExecutionCtx<'a> = EthBlockExecutionCtx<'a>;
    type Transaction = TransactionSigned;
    type Receipt = Receipt;
    type Executor<'a, DB: StateDB, I: Inspector<<Self::EvmFactory as EvmFactory>::Context<DB>>> =
        FluentBlockExecutor<'a, FluentEvmExecutor<DB, I, PrecompilesMap>>;

    fn evm_factory(&self) -> &Self::EvmFactory {
        self.inner.evm_factory()
    }

    fn create_executor<'a, DB, I>(
        &'a self,
        evm: FluentEvmExecutor<DB, I, PrecompilesMap>,
        ctx: EthBlockExecutionCtx<'a>,
    ) -> Self::Executor<'a, DB, I>
    where
        DB: StateDB,
        I: Inspector<<Self::EvmFactory as EvmFactory>::Context<DB>>,
    {
        FluentBlockExecutor {
            inner: EthBlockExecutor::new(
                evm,
                ctx,
                self.inner.chain_spec(),
                self.inner.executor_factory.receipt_builder(),
            ),
            staking_address: self.staking_address,
            chain_config_address: self.chain_config_address,
            liveness_slashing_address: self.liveness_slashing_address,
        }
    }
}

impl ConfigureEvm for FluentEvmConfig {
    type Primitives = EthPrimitives;
    type Error = Infallible;
    type NextBlockEnvCtx = NextBlockEnvAttributes;
    type BlockExecutorFactory = Self;
    type BlockAssembler = EthBlockAssembler<ChainSpec>;

    fn block_executor_factory(&self) -> &Self::BlockExecutorFactory {
        self
    }

    fn block_assembler(&self) -> &Self::BlockAssembler {
        self.inner.block_assembler()
    }

    fn evm_env(&self, header: &Header) -> Result<EvmEnvFor<Self>, Self::Error> {
        self.inner.evm_env(header)
    }

    fn next_evm_env(
        &self,
        parent: &Header,
        attributes: &Self::NextBlockEnvCtx,
    ) -> Result<EvmEnvFor<Self>, Self::Error> {
        self.inner.next_evm_env(parent, attributes)
    }

    fn context_for_block<'a>(
        &self,
        block: &'a SealedBlock<BlockTy<Self::Primitives>>,
    ) -> Result<EthBlockExecutionCtx<'a>, Self::Error> {
        self.inner.context_for_block(block)
    }

    fn context_for_next_block(
        &self,
        parent: &SealedHeader<Header>,
        attributes: Self::NextBlockEnvCtx,
    ) -> Result<EthBlockExecutionCtx<'_>, Self::Error> {
        self.inner.context_for_next_block(parent, attributes)
    }
}

impl ConfigureEngineEvm<ExecutionData> for FluentEvmConfig {
    fn evm_env_for_payload(&self, payload: &ExecutionData) -> Result<EvmEnvFor<Self>, Self::Error> {
        self.inner.evm_env_for_payload(payload)
    }

    fn context_for_payload<'a>(
        &self,
        payload: &'a ExecutionData,
    ) -> Result<ExecutionCtxFor<'a, Self>, Self::Error> {
        self.inner.context_for_payload(payload)
    }

    fn tx_iterator_for_payload(
        &self,
        payload: &ExecutionData,
    ) -> Result<impl ExecutableTxIterator<Self>, Self::Error> {
        self.inner.tx_iterator_for_payload(payload)
    }
}

/// Type configuration for a regular Fluent node.
///
/// Carries one bit of state, `dpos_active`, which the payload builder needs: a
/// DPoS chain's headers must leave `extra_data` empty, because the only writer of
/// that field is DPoS derivation (`derive.rs`), never the payload builder.
#[derive(Debug, Clone, Default)]
pub struct FluentNode {
    /// When true, the payload builder force-empties base `extra_data`. Set from
    /// `!staking_address.is_zero()` at the launch site — which is also true for
    /// the PRE-DPoS sequencer, and that is the case it exists for: the sequencer
    /// produces the block AT the activation height, the executor decodes from
    /// `>= activation`, and reth's default `extra_data` is a non-empty version
    /// string that would fail-loud-decode there.
    dpos_active: bool,
}

impl FluentNode {
    /// Construct a `FluentNode` for a chain with DPoS predeploys configured.
    pub fn with_dpos_active(dpos_active: bool) -> Self {
        Self { dpos_active }
    }

    /// Returns a [`ComponentsBuilder`] configured for a regular Ethereum node.
    pub fn components<Node>(
        &self,
    ) -> ComponentsBuilder<
        Node,
        EthereumPoolBuilder,
        BasicPayloadServiceBuilder<FluentPayloadBuilder>,
        EthereumNetworkBuilder,
        FluentExecutorBuilder,
        FluentConsensusBuilder,
    >
    where
        Node: FullNodeTypes<Types: NodeTypes<ChainSpec = ChainSpec, Primitives = EthPrimitives>>,
        <Node::Types as NodeTypes>::Payload:
            PayloadTypes<BuiltPayload = EthBuiltPayload, PayloadAttributes = EthPayloadAttributes>,
    {
        ComponentsBuilder::default()
            .node_types::<Node>()
            .pool(EthereumPoolBuilder::default())
            .executor(FluentExecutorBuilder::default())
            .payload(BasicPayloadServiceBuilder::new(FluentPayloadBuilder::new(
                self.dpos_active,
            )))
            .network(EthereumNetworkBuilder::default())
            .consensus(FluentConsensusBuilder::default())
    }

    pub fn provider_factory_builder() -> ProviderFactoryBuilder<Self> {
        ProviderFactoryBuilder::default()
    }
}

impl NodeTypes for FluentNode {
    type Primitives = EthPrimitives;
    type ChainSpec = ChainSpec;
    type Storage = EthStorage;
    type Payload = EthEngineTypes;
}

impl<N> Node<N> for FluentNode
where
    N: FullNodeTypes<Types = Self>,
{
    type ComponentsBuilder = ComponentsBuilder<
        N,
        EthereumPoolBuilder,
        BasicPayloadServiceBuilder<FluentPayloadBuilder>,
        EthereumNetworkBuilder,
        FluentExecutorBuilder,
        FluentConsensusBuilder,
    >;

    type AddOns =
        EthereumAddOns<NodeAdapter<N>, EthereumEthApiBuilder, EthereumEngineValidatorBuilder>;

    fn components_builder(&self) -> Self::ComponentsBuilder {
        self.components()
    }

    fn add_ons(&self) -> Self::AddOns {
        EthereumAddOns::default()
    }
}

impl<N: FullNodeComponents<Types = Self>> DebugNode<N> for FluentNode {
    type RpcBlock = alloy_rpc_types_eth::Block;

    fn rpc_to_primitive_block(rpc_block: Self::RpcBlock) -> reth_ethereum_primitives::Block {
        rpc_block.into_consensus().convert_transactions()
    }

    fn local_payload_attributes_builder(
        _chain_spec: &Self::ChainSpec,
    ) -> impl PayloadAttributesBuilder<<Self::Payload as PayloadTypes>::PayloadAttributes> {
        FluentPayloadAttributesBuilder {}
    }
}

// ***** мне кажется все же лучше ничего не добавлять в evm.rs, а поместить изменнения в отдельный модуль

// Inline ABI bindings for the `ProductionLiveness` predeploy. Mirrors
// `solidity-contracts/contracts/staking/ProductionLiveness.sol` — keep these
// signatures in sync; this `sol!()` macro IS the ABI source of truth on the Rust
// side.
alloy_sol_types::sol! {
    // The ONE liveness system call: who produced this block. Replaces
    // `processBitmap`, whose four arguments the 2-byte record supplies none of.
    // The epoch close — verdicts, exclusion stamps AND the stipend settlement —
    // is driven from inside the contract, so there is no second call.
    function recordProduction(uint64 blockNumber, uint8 leaderIndex) external;

    // Close-time events emitted by `recordProduction` (system call) — decoded from
    // the Success `ras.logs` for node-side observability only. System calls produce
    // no receipt, so these logs are otherwise invisible. Keep byte-identical to
    // `ProductionLiveness.sol` or the topic match below silently never fires.
    //
    // `PartialEpoch` is the load-bearing one: the partial-epoch taint is DERIVED
    // from the recorded block count, so ONE unrecorded block silently costs a whole
    // epoch its verdicts and the tier reads as enabled while judging nothing. Alert
    // on any occurrence outside epoch 0 (partial by construction).
    event PartialEpoch(uint64 indexed epoch, uint32 recorded, uint32 expected);
    event ProductionVerdictFailed(
        uint64 indexed epoch,
        address indexed validator,
        uint32 produced,
        uint256 due
    );
    event CorrelatedFailureEpoch(uint64 indexed epoch, uint256 newFailures, uint256 tolerance);
    event StipendLegSkipped(uint64 indexed epoch);

    // `Staking.commitEpochCommittee(address[])` + reads required to
    // derive the on-chain canonical committee Rust-side. Kept in sync with
    // `solidity-contracts/contracts/staking/Staking.sol` (`commitEpochCommittee`)
    // and `solidity-contracts/contracts/staking/ChainConfig.sol`
    // (`getEpochBlockInterval`).
    function commitEpochCommittee(address[] calldata committee) external;

    // Stipend-settlement events emitted (in `Staking`'s context, via the
    // `StakingRewards` DELEGATECALL) by the settle leg the epoch CLOSE now drives —
    // so they arrive on the `recordProduction` system call's logs, once per epoch
    // instead of once per block. Decoded for node-side observability only (system
    // calls produce no receipt). Keep byte-identical to `IStaking.sol:23,27`.
    event EpochBlendRewardsCommitted(uint64 indexed epoch, uint256 blendAmount);
    event StipendSkipped(uint64 indexed epoch);

    // The committed committee for `epoch` (incumbent). Read to tell a genuine
    // membership CHANGE from a no-change carry (the mint bit, which decides whether a
    // fresh beacon key is dealt). Under the 2-epoch warm-up there is no deferred
    // commit and no candidate stash: the commit re-derives the top-k fresh from
    // EffBal(target-2) and the contract verifies it against its own
    // `getValidatorsWithKeysAt` view, so a mismatch is a genuine fork witness. That
    // view is deterministic on all three of its inputs — visibility stamps, the stake
    // snapshot, and (since 2026-07-30) the committee-size cap, which is now
    // epoch-addressed via `ChainConfig.getActiveValidatorsLengthAt`. While the cap was
    // read live, re-deriving a PAST epoch's view could return a different set than the
    // one that was committed.
    function getEpochCommittee(uint64 epoch) external view returns (address[] memory);

    struct EpochConsensusKeys {
        bytes blsPubkey;
        bytes32 peerPubkey;
        uint64 activationEpoch;
    }

    // Ahead-commit pipeline (2-epoch committee warm-up): committee[N] is committed
    // TWO epochs ahead from EffBal(N-2). `nextEpochToCommit` = the next-uncommitted
    // epoch N; `committeeSelectionEpoch` = N-2 (clamped to 0 at genesis) = the epoch
    // whose set the executor must derive + submit so it matches contract verification.
    function getValidatorsWithKeysAt(uint64 epoch) external view
        returns (address[] memory validators, EpochConsensusKeys[] memory keys);
    function nextEpochToCommit() external view returns (uint64);
    function committeeSelectionEpoch() external view returns (uint64);

    function getEpochBlockInterval() external view returns (uint32);
    function getDposActivationBlock() external view returns (uint64);
}

fn encode_record_production_call(block_number: u64, leader_index: u8) -> Vec<u8> {
    use alloy_sol_types::SolCall;
    recordProductionCall {
        blockNumber: block_number,
        leaderIndex: leader_index,
    }
    .abi_encode()
}

/// Surface the close-time events of the `recordProduction` system call in node
/// logs + metrics. System calls produce no receipt and the executor commits and
/// discards `ras.logs`, so these are otherwise invisible without forensic
/// archaeology. Pure observability — reads the emitted logs, mutates no state,
/// runs identically on every node.
///
/// Both the liveness predeploy's own events AND the stipend events now arrive on
/// this one call: the epoch close drives the settle itself, and the settle runs in
/// `Staking`'s context (`StakingRewards` is DELEGATECALL'd), so the two are told
/// apart by `log.address`.
fn emit_close_observability(
    logs: &[alloy_primitives::Log],
    liveness_addr: Address,
    staking_addr: Address,
) {
    use alloy_sol_types::SolEvent;
    for log in logs {
        if log.address == liveness_addr {
            if let Ok(partial) = PartialEpoch::decode_log(log) {
                // Not diagnostic. A partial epoch means NO verdicts were evaluated
                // for it, and the taint is derived, so nothing else says so.
                tracing::error!(
                    target: "fluentbase::liveness",
                    epoch = partial.epoch,
                    recorded = partial.recorded,
                    expected = partial.expected,
                    "production_partial_epoch"
                );
                metrics::counter!("dpos_production_partial_epoch_total").increment(1);
            } else if let Ok(failed) = ProductionVerdictFailed::decode_log(log) {
                tracing::warn!(
                    target: "fluentbase::liveness",
                    epoch = failed.epoch,
                    validator = %failed.validator,
                    produced = failed.produced,
                    due = %failed.due,
                    "production_verdict_failed"
                );
                metrics::counter!("dpos_production_verdict_failed_total").increment(1);
            } else if let Ok(corr) = CorrelatedFailureEpoch::decode_log(log) {
                tracing::error!(
                    target: "fluentbase::liveness",
                    epoch = corr.epoch,
                    new_failures = %corr.newFailures,
                    tolerance = %corr.tolerance,
                    "production_correlated_failure_epoch"
                );
                metrics::counter!("dpos_production_correlated_failure_total").increment(1);
            } else if let Ok(skipped) = StipendLegSkipped::decode_log(log) {
                tracing::error!(
                    target: "fluentbase::rewards",
                    epoch = skipped.epoch,
                    "production_stipend_leg_skipped"
                );
                metrics::counter!("dpos_stipend_leg_skipped_total").increment(1);
            }
        } else if log.address == staking_addr {
            if let Ok(committed) = EpochBlendRewardsCommitted::decode_log(log) {
                tracing::info!(
                    target: "fluentbase::rewards",
                    epoch = committed.epoch,
                    blend_amount = %committed.blendAmount,
                    "epoch_blend_rewards_committed"
                );
                metrics::counter!("dpos_epoch_blend_rewards_committed_total").increment(1);
            } else if let Ok(skipped) = StipendSkipped::decode_log(log) {
                tracing::debug!(
                    target: "fluentbase::rewards",
                    epoch = skipped.epoch,
                    "epoch_stipend_skipped"
                );
                metrics::counter!("dpos_epoch_stipend_skipped_total").increment(1);
            }
        }
    }
}

/// Read `ChainConfig.getEpochBlockInterval()` via system call at the current
/// pre-execution state.
fn read_epoch_block_interval<E>(
    evm: &mut E,
    chain_config_address: Address,
) -> Result<u32, BlockExecutionError>
where
    E: Evm,
{
    use alloy_sol_types::SolCall;
    let calldata = getEpochBlockIntervalCall {}.abi_encode().into();
    let output = transact_view(evm, chain_config_address, calldata, "epoch_block_interval")?;
    getEpochBlockIntervalCall::abi_decode_returns(&output)
        .map_err(|e| BlockExecutionError::msg(format!("epoch_block_interval decode: {e:?}")))
}

/// DPoS activation height as a *scheduling state*, read resiliently against the
/// pre-execution state — the executor-side mirror of
/// [`fluentbase_staking_reader::reader::RethStakingStateReader::scheduled_dpos_activation`].
///
/// Returns:
/// - `Ok(None)` when `ChainConfig` is not (yet) a readably-scheduled DPoS
///   contract at this state: no code, OR `getDposActivationBlock()` reverts/halts
///   (the contract exists but is mid-runtime-deploy / a proxy whose impl isn't
///   coded yet), OR it returns the `0` "not scheduled" sentinel.
/// - `Ok(Some(h))` once governance has stored a nonzero activation height (the
///   setter requires `newValue >= block.number`, so a live chain never stores 0).
///
/// This is the SINGLE gate the DPoS epoch-commit pre-execution engages on. A
/// pre-DPoS (sequencer-era) sequencer launched with `--dpos.staking-config`
/// pointing at predicted-but-not-yet-deployed addresses must touch NO DPoS
/// contract field until activation is both scheduled AND readable — otherwise a
/// per-block read of a contract that is mid-runtime-deploy reverts, fails the
/// payload, stalls the chain, and so prevents the very deploy txns that would
/// finish the contract from ever mining (self-reinforcing deadlock). Reading
/// the *scheduling* discriminator first, and treating "unreadable" exactly like
/// "unscheduled", keeps that sequencer inert until DPoS is real.
///
/// A revert here is NOT swallowed error-handling on a hot read: it is the
/// definition of "this contract is not a scheduled DPoS ChainConfig yet". Once
/// `Some(h)` is observed the contract is fully initialized, so every subsequent
/// read in the DPoS section (interval, cursors, committee) stays fail-loud.
fn scheduled_dpos_activation<E>(
    evm: &mut E,
    chain_config_address: Address,
) -> Result<Option<u64>, BlockExecutionError>
where
    E: Evm,
{
    use alloy_sol_types::SolCall;
    use fluentbase_revm::revm::context_interface::result::{ExecutionResult, Output};

    let calldata = getDposActivationBlockCall {}.abi_encode().into();
    let ras = evm
        .transact_system_call(
            fluentbase_types::SYSTEM_ADDRESS,
            chain_config_address,
            calldata,
        )
        .map_err(|e| {
            BlockExecutionError::msg(format!("dpos_activation_block read failed: {e:?}"))
        })?;
    let output = match ras.result {
        ExecutionResult::Success { output, .. } => Some(match output {
            Output::Call(b) | Output::Create(b, _) => b,
        }),
        // Codeless account / proxy whose impl isn't coded yet / not-yet-deployed
        // contract → "not a scheduled DPoS ChainConfig at this state". Skip the
        // whole DPoS section rather than wedging the payload builder.
        ExecutionResult::Revert { .. } | ExecutionResult::Halt { .. } => None,
    };
    classify_scheduled_activation(output)
}

/// Pure decode+classify step of [`scheduled_dpos_activation`], split out so the
/// gate's decision logic is unit-testable without a live EVM. Every "the
/// ChainConfig is not a readable, scheduled DPoS config at this state" case folds
/// to `Ok(None)` (skip the DPoS section), so a pre-DPoS / mid-runtime-deploy
/// sequencer never wedges its payload builder:
/// - `None` (the read reverted/halted ⇒ proxy mid-deploy) → `Ok(None)`;
/// - `Some(empty)` — a CODELESS / not-yet-deployed account returns `Success` with
///   EMPTY output (no revert); "no return data" ⇒ unreadable ⇒ `Ok(None)` (NOT a
///   decode error — decoding empty bytes Overruns, which previously froze the
///   pre-deploy sequencer at block 0);
/// - `Some(bytes)` decoding to `0` (the unscheduled sentinel) → `Ok(None)`;
/// - `Some(bytes)` decoding to a nonzero height → `Ok(Some(height))`.
///
/// `0` is the unscheduled sentinel: `setDposActivationBlock` requires
/// `newValue >= block.number`, so a live chain never stores 0 — there is no DPoS
/// epoch to account for yet.
fn classify_scheduled_activation(
    output: Option<Bytes>,
) -> Result<Option<u64>, BlockExecutionError> {
    use alloy_sol_types::SolCall;
    let Some(output) = output else {
        return Ok(None);
    };
    if output.is_empty() {
        return Ok(None);
    }
    let activation = getDposActivationBlockCall::abi_decode_returns(&output)
        .map_err(|e| BlockExecutionError::msg(format!("dpos_activation_block decode: {e:?}")))?;
    Ok((activation != 0).then_some(activation))
}

/// Execute a `view` system call and return its raw output bytes (fail-loud on
/// revert/halt). Used by the ahead-commit cursor reads below.
fn transact_view<E>(
    evm: &mut E,
    to: Address,
    calldata: Bytes,
    what: &str,
) -> Result<Bytes, BlockExecutionError>
where
    E: Evm,
{
    use fluentbase_revm::revm::context_interface::result::{ExecutionResult, Output};
    let ras = evm
        .transact_system_call(fluentbase_types::SYSTEM_ADDRESS, to, calldata)
        .map_err(|e| BlockExecutionError::msg(format!("{what} read failed: {e:?}")))?;
    match ras.result {
        ExecutionResult::Success { output, .. } => Ok(match output {
            Output::Call(b) | Output::Create(b, _) => b,
        }),
        ExecutionResult::Revert { output, .. } => Err(BlockExecutionError::msg(format!(
            "{what} reverted: 0x{}",
            alloy_primitives::hex::encode(output)
        ))),
        ExecutionResult::Halt { reason, .. } => Err(BlockExecutionError::msg(format!(
            "{what} halted: {reason:?}"
        ))),
    }
}

/// `Staking.nextEpochToCommit()` — the next-uncommitted epoch (commit cursor).
fn read_next_epoch_to_commit<E>(
    evm: &mut E,
    staking_address: Address,
) -> Result<u64, BlockExecutionError>
where
    E: Evm,
{
    use alloy_sol_types::SolCall;
    let out = transact_view(
        evm,
        staking_address,
        nextEpochToCommitCall {}.abi_encode().into(),
        "nextEpochToCommit",
    )?;
    nextEpochToCommitCall::abi_decode_returns(&out)
        .map_err(|e| BlockExecutionError::msg(format!("nextEpochToCommit decode: {e:?}")))
}

/// `Staking.committeeSelectionEpoch()` — the epoch whose EffBal selects the next
/// committee to commit (= `nextEpochToCommit()-2`, clamped to 0 for target<2; the
/// 2-epoch committee warm-up; PoS spec §4.4).
fn read_committee_selection_epoch<E>(
    evm: &mut E,
    staking_address: Address,
) -> Result<u64, BlockExecutionError>
where
    E: Evm,
{
    use alloy_sol_types::SolCall;
    let out = transact_view(
        evm,
        staking_address,
        committeeSelectionEpochCall {}.abi_encode().into(),
        "committeeSelectionEpoch",
    )?;
    committeeSelectionEpochCall::abi_decode_returns(&out)
        .map_err(|e| BlockExecutionError::msg(format!("committeeSelectionEpoch decode: {e:?}")))
}

/// Derive the canonical Rust-side committee for `epoch`, identical to the
/// on-chain `commitEpochCommittee` verification predicate against
/// `_getValidatorsAt(epoch)`:
/// - read `getValidatorsWithKeysAt(epoch)` via system call
/// - filter keyless members (peerPubkey == bytes32(0))
/// - sort strictly ascending by `peerPubkey` raw bytes (matches Solidity
///   `bytes32 <` unsigned byte-lex)
/// - project to `Vec<Address>`
///
/// `epoch` is the `committeeSelectionEpoch()` (= committed-epoch − 2, the
/// 2-epoch committee warm-up).
/// Every node executes the same code on the same pre-block state, so the
/// derived committee is identical → identical state_root after the
/// `commitEpochCommittee` system call.
fn derive_committee_at<E>(
    evm: &mut E,
    staking_address: Address,
    epoch: u64,
) -> Result<Vec<Address>, BlockExecutionError>
where
    E: Evm,
{
    use alloy_sol_types::SolCall;
    use fluentbase_revm::revm::context_interface::result::{ExecutionResult, Output};
    let calldata = getValidatorsWithKeysAtCall { epoch }.abi_encode().into();
    let ras = evm
        .transact_system_call(fluentbase_types::SYSTEM_ADDRESS, staking_address, calldata)
        .map_err(|e| {
            BlockExecutionError::msg(format!("getValidatorsWithKeysAt read failed: {e:?}"))
        })?;
    let output = match ras.result {
        ExecutionResult::Success { output, .. } => match output {
            Output::Call(b) | Output::Create(b, _) => b,
        },
        ExecutionResult::Revert { output, .. } => {
            return Err(BlockExecutionError::msg(format!(
                "getValidatorsWithKeysAt reverted: 0x{}",
                alloy_primitives::hex::encode(output)
            )))
        }
        ExecutionResult::Halt { reason, .. } => {
            return Err(BlockExecutionError::msg(format!(
                "getValidatorsWithKeysAt halted: {reason:?}"
            )))
        }
    };
    let ret = getValidatorsWithKeysAtCall::abi_decode_returns(&output)
        .map_err(|e| BlockExecutionError::msg(format!("getValidatorsWithKeysAt decode: {e:?}")))?;
    let mut keyed: Vec<(Address, B256)> = ret
        .validators
        .into_iter()
        .zip(ret.keys)
        .filter(|(_, k)| k.peerPubkey != B256::ZERO)
        .map(|(addr, k)| (addr, k.peerPubkey))
        .collect();
    keyed.sort_unstable_by(|(_, a), (_, b)| a.as_slice().cmp(b.as_slice()));
    Ok(keyed.into_iter().map(|(addr, _)| addr).collect())
}

/// The two on-chain effects the ahead-commit loop drives, abstracted so the
/// loop's horizon/termination CONTROL FLOW is unit-testable without a live
/// Staking deployment (the migration/backlog behaviour is the loop's logic, not
/// the EVM plumbing). The production impl ([`EvmAheadCommit`]) reads
/// `nextEpochToCommit()` and issues the `commitEpochCommittee` system call (which
/// advances the on-chain cursor); the test impl models the cursor in memory.
trait AheadCommitDriver {
    /// Read the on-chain commit cursor (`Staking.nextEpochToCommit()`).
    fn read_next_epoch(&mut self) -> Result<u64, BlockExecutionError>;
    /// Derive + commit `committee[target]` (advances the on-chain cursor to
    /// `target+1` on success).
    fn commit_epoch(&mut self, target: u64) -> Result<(), BlockExecutionError>;
}

/// Drain every uncommitted epoch within the 2-epoch committee warm-up horizon
/// (`nextEpochToCommit() <= current_epoch + 2`), committing each IMMEDIATELY.
///
/// Steady state commits one epoch per block; a genesis/backlog block catches up a
/// small backlog; the one-time `+1 → +2` horizon MIGRATION commits exactly ONE
/// extra epoch (reading FINAL `EffBal`) then terminates. The only loop exit is
/// `next > current_epoch + 2`, which relies on every successful commit advancing
/// the cursor — so the cursor MUST strictly increase across iterations or we
/// fail loud (a contract bug that left the cursor pinned would otherwise re-commit
/// the same epoch forever and hang block execution with no error).
fn drive_ahead_commit(
    driver: &mut impl AheadCommitDriver,
    current_epoch: u64,
) -> Result<(), BlockExecutionError> {
    let mut prev_committed: Option<u64> = None;
    loop {
        let next = driver.read_next_epoch()?;
        if next > current_epoch + 2 {
            break; // nothing more committable within the 2-epoch warm-up horizon
        }
        if let Some(p) = prev_committed {
            if next <= p {
                return Err(BlockExecutionError::msg(format!(
                    "commitEpochCommittee cursor stuck at epoch {next} (last \
                     committed {p}): nextEpochToCommit did not advance after a \
                     successful commit"
                )));
            }
        }
        driver.commit_epoch(next)?;
        prev_committed = Some(next);
    }
    Ok(())
}

/// Production [`AheadCommitDriver`] over a live EVM: reads the cursor and issues
/// the `commitEpochCommittee` system call against the Staking predeploy.
struct EvmAheadCommit<'e, E> {
    evm: &'e mut E,
    staking_address: Address,
}

impl<E> AheadCommitDriver for EvmAheadCommit<'_, E>
where
    E: Evm<Tx = TxEnv>,
    <E as Evm>::DB: StateDB,
{
    fn read_next_epoch(&mut self) -> Result<u64, BlockExecutionError> {
        read_next_epoch_to_commit(self.evm, self.staking_address)
    }

    fn commit_epoch(&mut self, target: u64) -> Result<(), BlockExecutionError> {
        use alloy_sol_types::SolCall;
        use fluentbase_revm::revm::{
            context_interface::result::ExecutionResult, DatabaseCommit as _,
        };
        // 2-epoch committee warm-up: `committeeSelectionEpoch()` (= target−2)
        // reads FINAL `EffBal(target−2)`, and the contract deterministically sets
        // `dkgQual[target] = (derived set != committee[target−1])` inside the
        // commit — no deferral, no qualify-before-commit branch. `committee[target]`
        // is thus frozen a full epoch before its DKG runs (during target−1).
        let sel = read_committee_selection_epoch(self.evm, self.staking_address)?;
        let committee = derive_committee_at(self.evm, self.staking_address, sel)?;
        let calldata = commitEpochCommitteeCall { committee }.abi_encode();
        // FAIL-LOUD: the commit is liveness-critical (a missing committee
        // deadlocks the epoch boundary) and derives from deterministic state, so a
        // revert/halt is a real bug — surface it rather than silently retrying
        // (which never advances `lastCommittedEpochP1` and stalls). NB: a revert
        // lands inside `Ok(ras)` with a non-Success `ras.result`; committing that
        // state would be a no-op that also never advances the cursor, so check the
        // result explicitly.
        let ras = self
            .evm
            .transact_system_call(
                fluentbase_types::SYSTEM_ADDRESS,
                self.staking_address,
                calldata.into(),
            )
            .map_err(|e| {
                BlockExecutionError::msg(format!(
                    "commitEpochCommittee(epoch {target}) sys call failed: {e:?}"
                ))
            })?;
        match ras.result {
            ExecutionResult::Success { .. } => {
                self.evm.db_mut().commit(ras.state);
                Ok(())
            }
            other => Err(BlockExecutionError::msg(format!(
                "commitEpochCommittee(epoch {target}) did not succeed: {other:?}"
            ))),
        }
    }
}

#[derive(Debug)]
pub struct FluentBlockExecutor<'a, Evm> {
    /// Inner Ethereum execution strategy.
    inner: EthBlockExecutor<'a, Evm, &'a Arc<ChainSpec>, &'a RethReceiptBuilder>,
    /// Staking predeploy address. [`Address::ZERO`] disables the
    /// epoch-boundary `commitEpochCommittee` system call.
    staking_address: Address,
    /// ChainConfig predeploy address. [`Address::ZERO`] disables the epoch
    /// system call (paired with [`Self::staking_address`]).
    chain_config_address: Address,
    /// Liveness predeploy address the `recordProduction` system call
    /// targets (configurable so the whole staking cluster can be runtime-
    /// deployed; defaults to the canonical predeploy slot).
    liveness_slashing_address: Address,
}

impl<'a, E> BlockExecutor for FluentBlockExecutor<'a, E>
where
    E: Evm<Tx = TxEnv>,
    <E as Evm>::DB: StateDB,
    EthBlockExecutor<'a, E, &'a Arc<ChainSpec>, &'a RethReceiptBuilder>: BlockExecutor<
        Transaction = TransactionSigned,
        Receipt = Receipt,
        Evm = E,
        Result = EthTxResult<E::HaltReason, TxType>,
    >,
{
    type Transaction = TransactionSigned;
    type Receipt = Receipt;
    type Evm = E;
    type Result = EthTxResult<E::HaltReason, TxType>;

    fn apply_pre_execution_changes(&mut self) -> Result<(), BlockExecutionError> {
        // Note: Ideally, this shouldn't be required if there are no memory leaks, but supporting a
        //  memory allocator inside virtual runtime brings overhead.
        // Instead, we can just re-create the store to make sure all data is pruned.
        fluentbase_runtime::runtime::SystemRuntime::reset_cached_runtimes();
        // Invoke parent method
        self.inner.apply_pre_execution_changes()?;

        // DPoS-gated block: both the production-record decoder and the
        // commitEpochCommittee system call are bypassed on non-DPoS
        // chains (staking_address and chain_config_address both zero).
        // Prior to this gate the decoder ran unconditionally and
        // mapped reth's default `"reth/v..."` extra_data to a fail-loud
        // BlockExecutionError, stalling every non-DPoS block at #1.
        if !self.staking_address.is_zero() && !self.chain_config_address.is_zero() {
            use fluentbase_revm::revm::context_interface::result::ExecutionResult;
            use fluentbase_revm::revm::context_interface::Block as _;
            use fluentbase_revm::revm::DatabaseCommit as _;

            // SINGLE gate (P2-2): the whole DPoS epoch-commit section engages ONLY
            // once DPoS activation is both SCHEDULED and READABLE at this
            // pre-execution state. `scheduled_dpos_activation` folds three
            // pre-DPoS states into the same `None` → skip:
            //   - `ChainConfig` has no code yet (migrated prod chain resyncing
            //     history from genesis where the predeploys aren't in the
            //     chainspec; or a fresh chain before the runtime deploy);
            //   - `ChainConfig` exists but `getDposActivationBlock()` reverts (a
            //     proxy mid-runtime-deploy whose impl isn't coded yet) — this is
            //     the case that used to wedge a `--dpos.staking-config` sequencer
            //     mid-deploy: a per-block read reverted → payload failed → chain
            //     stalled → the deploy txns couldn't mine → frozen forever;
            //   - activation is the `0` "not scheduled" sentinel.
            // Reading the SCHEDULING discriminator first (and treating unreadable
            // exactly like unscheduled) means a pre-DPoS / pre-deploy sequencer
            // touches NO other DPoS contract field. The probe is a deterministic
            // pure function of pre-block state, so every node skips the same
            // blocks identically (state-root symmetry). Once `Some(activation)` is
            // observed the contract is fully initialized, so every read below
            // stays fail-loud.
            let Some(activation) =
                scheduled_dpos_activation(self.inner.evm_mut(), self.chain_config_address)?
            else {
                return Ok(());
            };

            let block_number: u64 = self.inner.evm().block().number().saturating_to();
            // Relative epoch numbering: the contract's commit cursor counts
            // epochs from `dposActivationBlock` (Staking._currentEpoch), so the
            // ahead-commit horizon must match or the catch-up loop misfires. Safe
            // to read fail-loud now — a scheduled activation implies an
            // initialized ChainConfig with `epochBlockInterval > 0`.
            let interval =
                read_epoch_block_interval(self.inner.evm_mut(), self.chain_config_address)?;

            // System-call the liveness predeploy with THIS block's producer, decoded
            // from `block.header.extra_data`, but ONLY at/after DPoS activation.
            //
            // Three arms, and the empty one is not slack — it is the ONE block the
            // pre-DPoS sequencer produces AT `block_number == activation`.
            // `launcher.rs` halts sequencer production only once its head has
            // REACHED activation, and `payload.rs` force-empties `extra_data` under
            // DPoS, so that block carries zero bytes and this gate (`>=`) still
            // decodes it. Fail-loud there would be a deterministic, every-node,
            // unrecoverable failure at the swap block of every bring-up. It is not a
            // hole either: verify REJECTS an empty field, so no block can reach this
            // arm through consensus.
            //
            //   len == 0 ⇒ no syscall;  len == 2 ⇒ decode + syscall;  else ⇒ fail loud.
            if block_number >= activation {
                let extra_data = self.inner.ctx.extra_data.clone();
                let record =
                    fluentbase_consensus::extra_data::decode_production_record(&extra_data)
                        .map_err(|e| BlockExecutionError::msg(format!("production record: {e}")))?;
                if let Some(record) = record {
                    let calldata = encode_record_production_call(block_number, record.leader_index);
                    let ras = self
                        .inner
                        .evm_mut()
                        .transact_system_call(
                            fluentbase_types::SYSTEM_ADDRESS,
                            self.liveness_slashing_address,
                            calldata.into(),
                        )
                        .map_err(|e| {
                            BlockExecutionError::msg(format!("liveness sys call: {e:?}"))
                        })?;
                    // FAIL-LOUD on every revert and halt, keeping `processBitmap`'s
                    // reasoning verbatim: a Solidity revert lands inside `Ok(ras)`
                    // with a non-Success result and ROLLED-BACK state, so committing
                    // it would silently no-op the whole liveness leg. There is no
                    // transient-selector whitelist any more — the two it used to
                    // carry belonged to the slash sub-path this change deletes, and
                    // the not-yet-committed-committee case they covered is now a
                    // plain in-contract park arm that returns normally.
                    //
                    // The STIPEND is not a second leg to tolerate here: it lives
                    // inside the contract's close behind its own gas-bounded `try`,
                    // which is the one place this design prefers a frozen payment to
                    // any chance of a frozen chain.
                    match ras.result {
                        ExecutionResult::Success { logs, .. } => {
                            emit_close_observability(
                                &logs,
                                self.liveness_slashing_address,
                                self.staking_address,
                            );
                            self.inner.evm_mut().db_mut().commit(ras.state)
                        }
                        ExecutionResult::Revert { output, .. } => {
                            return Err(BlockExecutionError::msg(format!(
                                "recordProduction reverted (deterministic caller bug): 0x{}",
                                alloy_primitives::hex::encode(&output)
                            )))
                        }
                        ExecutionResult::Halt { reason, .. } => {
                            return Err(BlockExecutionError::msg(format!(
                                "recordProduction halted: {reason:?}"
                            )))
                        }
                    }
                }
            }

            // Commit the canonical committee two epochs ahead (the 2-epoch
            // committee warm-up): catch up every uncommitted epoch within the
            // lookahead horizon (`nextEpochToCommit() <= currentEpoch+2`), deriving
            // each set from `committeeSelectionEpoch()` (= the committed epoch's N-2)
            // so it matches the contract's `_getValidatorsAt(selectionEpoch)`
            // verification. Steady state: one commit per epoch; genesis/migration:
            // catches up a small backlog. Runs pre-activation too — the pre-swap
            // sequencer commits committees so the first DPoS epoch's set is already
            // on-chain at activation.
            //
            // `interval == 0` is unreachable on a live chain: ChainConfig requires
            // epochBlockInterval > 0 on both init and every setter, so the
            // `else { 0 }` guard is purely defensive.
            // Shared activation-relative epoch math (the single definition in
            // `staking-reader`) so this ahead-commit horizon can never drift
            // from the consensus/cold-start epocher. The `else { 0 }` keeps the
            // defensive interval==0 guard (`epoch_of_block` divides, see above).
            let current_epoch = if interval > 0 {
                fluentbase_staking_reader::reader::epoch_of_block(
                    block_number,
                    interval,
                    activation,
                )
            } else {
                0
            };

            // NOTE ON ORDER: the ahead-commit driver runs AFTER the recorder, and
            // must stay there. It is why epoch 0's first blocks park in the
            // recorder's committee-less arm — accepted, and covered by the
            // partial-epoch taint exempting epoch 0.
            let mut driver = EvmAheadCommit {
                evm: self.inner.evm_mut(),
                staking_address: self.staking_address,
            };
            drive_ahead_commit(&mut driver, current_epoch)?;
        }
        Ok(())
    }

    fn execute_transaction_without_commit(
        &mut self,
        tx: impl ExecutableTx<Self>,
    ) -> Result<Self::Result, BlockExecutionError> {
        self.inner.execute_transaction_without_commit(tx)
    }

    fn commit_transaction(&mut self, output: Self::Result) -> GasOutput {
        self.inner.commit_transaction(output)
    }

    fn finish(self) -> Result<(Self::Evm, BlockExecutionResult<Receipt>), BlockExecutionError> {
        self.inner.finish()
    }

    fn set_state_hook(&mut self, _hook: Option<Box<dyn OnStateHook>>) {
        self.inner.set_state_hook(_hook)
    }

    fn evm_mut(&mut self) -> &mut Self::Evm {
        self.inner.evm_mut()
    }

    fn evm(&self) -> &Self::Evm {
        self.inner.evm()
    }

    fn receipts(&self) -> &[Self::Receipt] {
        self.inner.receipts()
    }
}

#[cfg(test)]
mod tests {
    /// The staking-reader crate inlines the liveness predeploy slot as a literal
    /// (it can't depend on `fluentbase-types`); pin its serde default to the
    /// canonical constant so the inlined value can't silently drift.
    #[test]
    fn reader_liveness_default_matches_canonical_precompile() {
        let json = r#"{
            "staking_address": "0x0000000000000000000000000000000000520010",
            "chain_config_address": "0x0000000000000000000000000000000000520011"
        }"#;
        let cfg: fluentbase_staking_reader::reader::StakingReaderConfig =
            serde_json::from_str(json).expect("config must parse");
        assert_eq!(
            cfg.liveness_slashing_address,
            fluentbase_types::PRECOMPILE_LIVENESS_SLASHING
        );
    }

    /// The DPoS epoch-commit pre-execution gate must be INERT whenever the
    /// `ChainConfig` activation read is unreadable or unscheduled — the root fix
    /// for the runtime-deploy deadlock (a pre-DPoS sequencer launched with
    /// `--dpos.staking-config` whose `ChainConfig` is mid-runtime-deploy must
    /// NOT fail-loud per block, or it stalls the chain and the deploy txns can
    /// never mine). `None` = the read reverted/halted (codeless / proxy whose
    /// impl isn't coded yet); a decoded `0` = the unscheduled sentinel. Both map
    /// to `Ok(None)` (skip); only a nonzero height engages the section.
    #[test]
    fn unreadable_or_unscheduled_chainconfig_is_inert() {
        use super::classify_scheduled_activation;
        use alloy_sol_types::SolValue;

        // Read reverted/halted ⇒ skip (this is the deadlock-dissolving arm).
        assert_eq!(
            classify_scheduled_activation(None).expect("revert must not be fatal"),
            None
        );

        // Decoded `0` (unscheduled sentinel) ⇒ skip. A single `uint64` return is
        // ABI-encoded exactly as `u64::abi_encode()`, the wire the gate decodes.
        let zero = alloy_primitives::Bytes::from(0u64.abi_encode());
        assert_eq!(
            classify_scheduled_activation(Some(zero)).expect("zero must decode"),
            None
        );

        // Nonzero scheduled height ⇒ engage with that activation.
        let scheduled = alloy_primitives::Bytes::from(128u64.abi_encode());
        assert_eq!(
            classify_scheduled_activation(Some(scheduled)).expect("nonzero must decode"),
            Some(128)
        );

        // A CODELESS / not-yet-deployed account returns `Success` with EMPTY output
        // (NOT a revert) — the real pre-deploy / mid-runtime-deploy state. Decoding
        // empty bytes Overruns, which previously propagated as a fatal payload error
        // and froze the bare chain at block 0; it MUST fold to `None` (skip).
        assert_eq!(
            classify_scheduled_activation(Some(alloy_primitives::Bytes::new()))
                .expect("empty (codeless) output must not be a fatal decode error"),
            None
        );
    }

    /// The node-side close observability reads `ProductionLiveness`'s events out of
    /// the discarded `recordProduction` system-call logs. The Rust `sol!` event ABI
    /// must stay byte-identical to `ProductionLiveness.sol` or the `decode_log`
    /// topic match silently never fires — and for `PartialEpoch` that silence is the
    /// whole failure mode it exists to break. Pin the canonical signatures and prove
    /// a fabricated log decodes to the exact field values.
    #[test]
    fn close_events_decode_from_fabricated_logs() {
        use super::{CorrelatedFailureEpoch, PartialEpoch, ProductionVerdictFailed};
        use alloy_sol_types::SolEvent;

        assert_eq!(
            PartialEpoch::SIGNATURE,
            "PartialEpoch(uint64,uint32,uint32)"
        );
        assert_eq!(
            ProductionVerdictFailed::SIGNATURE,
            "ProductionVerdictFailed(uint64,address,uint32,uint256)"
        );
        assert_eq!(
            CorrelatedFailureEpoch::SIGNATURE,
            "CorrelatedFailureEpoch(uint64,uint256,uint256)"
        );

        let validator = alloy_primitives::address!("00000000000000000000000000000000000000aa");

        let partial = PartialEpoch {
            epoch: 7,
            recorded: 31,
            expected: 32,
        };
        let partial_log = alloy_primitives::Log {
            address: fluentbase_types::PRECOMPILE_LIVENESS_SLASHING,
            data: partial.encode_log_data(),
        };
        let decoded = PartialEpoch::decode_log(&partial_log).expect("fabricated log must decode");
        assert_eq!(decoded.epoch, 7);
        assert_eq!(decoded.recorded, 31);
        assert_eq!(decoded.expected, 32);

        let failed = ProductionVerdictFailed {
            epoch: 9,
            validator,
            produced: 4,
            due: alloy_primitives::U256::from(100u64),
        };
        let failed_log = alloy_primitives::Log {
            address: fluentbase_types::PRECOMPILE_LIVENESS_SLASHING,
            data: failed.encode_log_data(),
        };
        let decoded_failed =
            ProductionVerdictFailed::decode_log(&failed_log).expect("fabricated log must decode");
        assert_eq!(decoded_failed.epoch, 9);
        assert_eq!(decoded_failed.validator, validator);
        assert_eq!(decoded_failed.produced, 4);

        // Distinct topic0 / arity — one event's log must never decode as another's.
        assert!(PartialEpoch::decode_log(&failed_log).is_err());
        assert!(ProductionVerdictFailed::decode_log(&partial_log).is_err());
    }

    /// The one syscall the executor injects per block. Its selector and argument
    /// packing are the whole contract interface, and a silent drift mis-credits
    /// production with no other symptom.
    #[test]
    fn record_production_calldata_is_pinned() {
        use alloy_sol_types::SolCall;
        assert_eq!(
            super::recordProductionCall::SIGNATURE,
            "recordProduction(uint64,uint8)"
        );
        let encoded = super::encode_record_production_call(1234, 50);
        assert_eq!(encoded.len(), 4 + 32 + 32);
        let decoded = super::recordProductionCall::abi_decode(&encoded).expect("roundtrip");
        assert_eq!(decoded.blockNumber, 1234);
        assert_eq!(decoded.leaderIndex, 50);
    }

    /// In-memory [`AheadCommitDriver`] modelling the on-chain commit cursor:
    /// `commit_epoch(target)` records the target and advances the cursor to
    /// `target+1` (mirroring `commitEpochCommittee` bumping `nextEpochToCommit`),
    /// UNLESS `stuck` is set — the pathological contract bug the loop's
    /// strict-increase guard defends against.
    struct MockCursor {
        next: u64,
        committed: Vec<u64>,
        stuck: bool,
    }

    impl MockCursor {
        fn new(next: u64) -> Self {
            Self {
                next,
                committed: Vec::new(),
                stuck: false,
            }
        }
    }

    impl super::AheadCommitDriver for MockCursor {
        fn read_next_epoch(&mut self) -> Result<u64, super::BlockExecutionError> {
            Ok(self.next)
        }

        fn commit_epoch(&mut self, target: u64) -> Result<(), super::BlockExecutionError> {
            self.committed.push(target);
            if !self.stuck {
                self.next = target + 1;
            }
            Ok(())
        }
    }

    /// The one-time `+1 → +2` horizon MIGRATION: a chain that had been committing
    /// one-ahead (so at a block in epoch E the cursor already sits at E+2, i.e.
    /// committee[E+1] is committed) must, on the switch block, commit EXACTLY ONE
    /// extra epoch (E+2, reading its now-final `EffBal(E)`), advance the cursor by
    /// one to E+3, and then stop — no skip, no double-commit.
    #[test]
    fn ahead_commit_migration_commits_exactly_one_extra_epoch() {
        let current_epoch = 10;
        // One-ahead steady state left the cursor at E+2 (E+1 already committed).
        let mut cursor = MockCursor::new(current_epoch + 2);
        super::drive_ahead_commit(&mut cursor, current_epoch).expect("migration commit");
        // Exactly one commit — committee[E+2] — and the cursor advanced by one.
        assert_eq!(cursor.committed, vec![current_epoch + 2]);
        assert_eq!(cursor.next, current_epoch + 3);
    }

    /// Steady state under the `+2` horizon: the cursor already sits at E+3
    /// (committee[E+1] and committee[E+2] both committed), so the block commits
    /// nothing and terminates immediately.
    #[test]
    fn ahead_commit_steady_state_two_ahead_commits_nothing() {
        let current_epoch = 10;
        let mut cursor = MockCursor::new(current_epoch + 3);
        super::drive_ahead_commit(&mut cursor, current_epoch).expect("steady state");
        assert!(cursor.committed.is_empty());
        assert_eq!(cursor.next, current_epoch + 3);
    }

    /// A genesis/backlog block drains every uncommitted epoch up to the horizon,
    /// in strict ascending order, in a single block.
    #[test]
    fn ahead_commit_backlog_drains_up_to_horizon_in_order() {
        let current_epoch = 5;
        // Fresh chain: nothing committed yet.
        let mut cursor = MockCursor::new(0);
        super::drive_ahead_commit(&mut cursor, current_epoch).expect("backlog drain");
        // Commits epochs 0..=current+2 inclusive, ascending.
        assert_eq!(
            cursor.committed,
            (0..=current_epoch + 2).collect::<Vec<_>>()
        );
        assert_eq!(cursor.next, current_epoch + 3);
    }

    /// Termination guard: a commit that fails to advance the cursor (contract bug)
    /// must fail loud on the next iteration rather than re-committing forever.
    #[test]
    fn ahead_commit_stuck_cursor_fails_loud() {
        let current_epoch = 10;
        let mut cursor = MockCursor::new(current_epoch);
        cursor.stuck = true;
        let err = super::drive_ahead_commit(&mut cursor, current_epoch)
            .expect_err("a non-advancing cursor must be fatal, not an infinite loop");
        assert!(format!("{err}").contains("cursor stuck"));
        // It committed the same epoch at most twice before tripping the guard —
        // never an unbounded loop.
        assert!(cursor.committed.len() <= 2, "guard must trip promptly");
    }
}
