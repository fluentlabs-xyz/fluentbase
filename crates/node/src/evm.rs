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
use alloy_rpc_types_engine::{ExecutionData, PayloadAttributes as EthPayloadAttributes, PayloadId};
use core::{
    fmt::Debug,
    ops::{Deref, DerefMut},
};
use dashmap::DashMap;
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
#[derive(Debug, Default, Clone, Copy)]
pub struct FluentExecutorBuilder {
    staking_address: Address,
    chain_config_address: Address,
}

impl FluentExecutorBuilder {
    pub const fn new(staking_address: Address, chain_config_address: Address) -> Self {
        Self {
            staking_address,
            chain_config_address,
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
}

impl FluentEvmConfig {
    /// Create a new [`FluentEvmConfig`] with the given chain spec, EVM factory,
    /// and the operator-supplied staking + chain_config contract addresses.
    pub fn new(
        chain_spec: Arc<ChainSpec>,
        evm_factory: FluentEvmFactory,
        staking_address: Address,
        chain_config_address: Address,
    ) -> Self {
        let inner = EthEvmConfig::new_with_evm_factory(chain_spec.clone(), evm_factory);
        Self {
            inner,
            staking_address,
            chain_config_address,
        }
    }

    /// Create a new [`FluentEvmConfig`] with the given chain spec and default
    /// EVM factory. Staking + ChainConfig addresses default to
    /// [`Address::ZERO`] (non-DPoS path).
    pub fn new_with_default_factory(chain_spec: Arc<ChainSpec>) -> Self {
        Self::new(
            chain_spec,
            FluentEvmFactory::default(),
            Address::ZERO,
            Address::ZERO,
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
/// `FluentNode` is **stateful**: it carries an
/// `Arc<DashMap<PayloadId, Bytes>>` that is shared between
/// [`FluentPayloadBuilder`] (reader) and `FluentApp::propose` (writer
/// via `OuterBuilder.extra_data_registry`). `Default` constructs an
/// empty map (non-DPoS / debug-RPC modes still work — empty extra_data
/// causes the executor's `processBitmap` system call to no-op on
/// `committeeSize == 0`).
#[derive(Debug, Clone)]
pub struct FluentNode {
    extra_data_registry: Arc<DashMap<PayloadId, Bytes>>,
    /// When true, the payload builder force-empties base extra_data so a
    /// registry miss can't ship a non-empty default that the verifier
    /// rejects. Set from `!staking_address.is_zero()` at the launch site.
    dpos_active: bool,
}

impl Default for FluentNode {
    fn default() -> Self {
        Self {
            extra_data_registry: Arc::new(DashMap::new()),
            dpos_active: false,
        }
    }
}

impl FluentNode {
    /// Construct a `FluentNode` sharing the supplied `extra_data_registry`
    /// instance with another consumer (e.g., DPoS's
    /// `OuterBuilder.extra_data_registry`). DPoS validators wire BOTH the
    /// payload-builder (reader) and `FluentApp::propose` (writer) to the
    /// same `Arc<DashMap>` so per-`PayloadId` extra_data injection is
    /// race-free.
    pub fn with_extra_data_registry(
        extra_data_registry: Arc<DashMap<PayloadId, Bytes>>,
        dpos_active: bool,
    ) -> Self {
        Self {
            extra_data_registry,
            dpos_active,
        }
    }

    /// Returns a clone of the registry handle. Used by `dpos.rs` to share
    /// the same `Arc<DashMap>` between the payload builder (already
    /// instantiated when `components()` ran) and `OuterBuilder`.
    pub fn extra_data_registry(&self) -> Arc<DashMap<PayloadId, Bytes>> {
        self.extra_data_registry.clone()
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
                self.extra_data_registry.clone(),
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
        // Capture self's registry so the FluentPayloadBuilder
        // instantiated below sees the SAME `Arc<DashMap>` instance that
        // dpos.rs / OuterBuilder will write into.
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

// Inline ABI bindings for the `LivenessSlashing` predeploy. Mirrors
// `solidity-contracts/contracts/staking/LivenessSlashing.sol` — keep these
// signatures in sync (no `ILivenessSlashing.sol` interface in V1; this
// `sol!()` macro IS the ABI source of truth on the Rust
// side).
alloy_sol_types::sol! {
    function processBitmap(
        uint64 epoch,
        uint64 blockNumber,
        uint8 committeeSize,
        bytes calldata signersBitmap
    ) external;

    // Transient revert selectors on `processBitmap`'s slash sub-path
    // (fires only at MISS_THRESHOLD consecutive misses). Signatures mirror
    // `IStakingContext.sol` (`ValidatorNotFound(address)` :140,
    // `EpochCommitteeNotCommitted(uint64)` :179) — keep byte-identical or
    // the selector match below silently misclassifies them as fail-loud.
    error EpochCommitteeNotCommitted(uint64 epoch);
    error ValidatorNotFound(address validator);

    // `Staking.commitEpochCommittee(address[])` + reads required to
    // derive the on-chain canonical committee Rust-side. Kept in sync with
    // `solidity-contracts/contracts/staking/Staking.sol` (`commitEpochCommittee`,
    // `getValidatorsWithKeys`) and
    // `solidity-contracts/contracts/staking/ChainConfig.sol`
    // (`getEpochBlockInterval`).
    function commitEpochCommittee(address[] calldata committee) external;

    struct EpochConsensusKeys {
        bytes blsPubkey;
        bytes32 peerPubkey;
        uint64 activationEpoch;
    }
    function getValidatorsWithKeys() external view
        returns (address[] memory validators, EpochConsensusKeys[] memory keys);

    // Ahead-commit pipeline (PoS spec §4.4): committee[N] is committed one epoch
    // ahead from EffBal(N-1). `nextEpochToCommit` = the next-uncommitted epoch N;
    // `committeeSelectionEpoch` = N-1 (0 at genesis) = the epoch whose set the
    // executor must derive + submit so it matches the contract's verification.
    function getValidatorsWithKeysAt(uint64 epoch) external view
        returns (address[] memory validators, EpochConsensusKeys[] memory keys);
    function nextEpochToCommit() external view returns (uint64);
    function committeeSelectionEpoch() external view returns (uint64);

    function getEpochBlockInterval() external view returns (uint32);
    function getDposActivationBlock() external view returns (uint64);
}

fn encode_process_bitmap_call(
    epoch: u64,
    block_number: u64,
    committee_size: u8,
    bitmap: &[u8],
) -> Vec<u8> {
    use alloy_sol_types::SolCall;
    processBitmapCall {
        epoch,
        blockNumber: block_number,
        committeeSize: committee_size,
        signersBitmap: Bytes::from(bitmap.to_vec()),
    }
    .abi_encode()
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

/// Read `ChainConfig.getDposActivationBlock()` — origin for relative DPoS epoch
/// numbering (zero ⇒ absolute) — via system call at the pre-execution state.
fn read_dpos_activation_block<E>(
    evm: &mut E,
    chain_config_address: Address,
) -> Result<u64, BlockExecutionError>
where
    E: Evm,
{
    use alloy_sol_types::SolCall;
    let calldata = getDposActivationBlockCall {}.abi_encode().into();
    let output = transact_view(evm, chain_config_address, calldata, "dpos_activation_block")?;
    getDposActivationBlockCall::abi_decode_returns(&output)
        .map_err(|e| BlockExecutionError::msg(format!("dpos_activation_block decode: {e:?}")))
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
/// committee to commit (= `nextEpochToCommit()-1`, 0 at genesis; PoS spec §4.4).
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
/// `epoch` is the `committeeSelectionEpoch()` (= committed-epoch − 1, §4.4).
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

        // DPoS-gated block: both the liveness-bitmap decoder and the
        // commitEpochCommittee system call are bypassed on non-DPoS
        // chains (staking_address and chain_config_address both zero).
        // Prior to this gate the decoder ran unconditionally and
        // mapped reth's default `"reth/v..."` extra_data to a fail-loud
        // BlockExecutionError, stalling every non-DPoS block at #1.
        if !self.staking_address.is_zero() && !self.chain_config_address.is_zero() {
            // System-call the `LivenessSlashing` predeploy with the
            // previous finalized
            // cert's bitmap decoded from `block.header.extra_data`.
            // Cold-start blocks carry empty `extra_data` → decoder
            // returns `None` → no system call. Decode errors fail the
            // block (consensus-side `verify` already structurally decoded
            // the `extra_data`, so a decode failure here means a non-DPoS
            // block reached this DPoS executor — fail-loud).
            let extra_data = self.inner.ctx.extra_data.clone();
            let decoded = fluentbase_consensus::extra_data::decode_simplex_attestation(&extra_data)
                .map_err(|e| BlockExecutionError::msg(format!("liveness decode: {e}")))?;

            if let Some(d) = decoded {
                use fluentbase_revm::revm::context_interface::Block as _;
                use fluentbase_revm::revm::DatabaseCommit as _;
                let block_number: u64 = self.inner.evm().block().number().saturating_to();
                let epoch = d.round.epoch().get();
                let calldata =
                    encode_process_bitmap_call(epoch, block_number, d.committee_size, &d.bitmap);
                use alloy_sol_types::SolError as _;
                use fluentbase_revm::revm::context_interface::result::ExecutionResult;
                let ras = self
                    .inner
                    .evm_mut()
                    .transact_system_call(
                        fluentbase_types::SYSTEM_ADDRESS,
                        fluentbase_types::PRECOMPILE_LIVENESS_SLASHING,
                        calldata.into(),
                    )
                    .map_err(|e| BlockExecutionError::msg(format!("liveness sys call: {e:?}")))?;
                // A Solidity revert lands inside `Ok(ras)` with a
                // non-Success `ras.result` and rolled-back state, so the prior
                // unconditional `commit(ras.state)` silently no-op'd it.
                // Classify: the transient slash sub-path selectors
                // (committee-not-yet-committed / victim-removed) are tolerated
                // (skip this block's liveness accounting rather than wedge the
                // chain); every other revert/halt is a deterministic caller bug
                // and fails the block.
                match ras.result {
                    ExecutionResult::Success { .. } => {
                        self.inner.evm_mut().db_mut().commit(ras.state)
                    }
                    ExecutionResult::Revert { output, .. } => {
                        let sel = output.get(..4);
                        if sel == Some(EpochCommitteeNotCommitted::SELECTOR.as_slice())
                            || sel == Some(ValidatorNotFound::SELECTOR.as_slice())
                        {
                            tracing::error!(
                                epoch,
                                block_number,
                                revert = %alloy_primitives::hex::encode(&output),
                                "processBitmap transient revert (slash sub-path); \
                                 liveness accounting skipped this block"
                            );
                            tracing::warn!(
                                target: "fluentbase::liveness",
                                "liveness_processbitmap_transient_revert"
                            );
                        } else {
                            // InvalidBitmapLength / onlySystemCall / unknown =
                            // deterministic caller bug — fail loud rather than
                            // committing a rolled-back no-op.
                            return Err(BlockExecutionError::msg(format!(
                                "processBitmap reverted (caller bug, selector {sel:?}): 0x{}",
                                alloy_primitives::hex::encode(&output)
                            )));
                        }
                    }
                    ExecutionResult::Halt { reason, .. } => {
                        return Err(BlockExecutionError::msg(format!(
                            "processBitmap halted: {reason:?}"
                        )))
                    }
                }
            }

            // Commit the canonical committee one epoch ahead (PoS spec §4.4):
            // catch up every uncommitted epoch within the lookahead horizon
            // (`nextEpochToCommit() <= currentEpoch+1`), deriving each set from
            // `committeeSelectionEpoch()` (= the committed epoch's N-1) so it
            // matches the contract's `_getValidatorsAt(selectionEpoch)`
            // verification. Steady state: one commit per epoch; genesis/migration:
            // catches up a small backlog.
            use alloy_sol_types::SolCall;
            use fluentbase_revm::revm::context_interface::result::ExecutionResult;
            use fluentbase_revm::revm::context_interface::Block as _;
            use fluentbase_revm::revm::DatabaseCommit as _;
            let block_number: u64 = self.inner.evm().block().number().saturating_to();
            let interval =
                read_epoch_block_interval(self.inner.evm_mut(), self.chain_config_address)?;
            // Relative epoch numbering: the contract's commit cursor counts
            // epochs from `dposActivationBlock` (Staking._currentEpoch), so the
            // ahead-commit horizon must match or the catch-up loop misfires.
            // `saturating_sub` mirrors the contract's pre-activation clamp; zero
            // activation ⇒ absolute (degenerate, pre-migration).
            let activation =
                read_dpos_activation_block(self.inner.evm_mut(), self.chain_config_address)?;
            // `interval == 0` is unreachable on a live chain: ChainConfig
            // requires epochBlockInterval > 0 on both init and every setter
            // (ChainConfig.sol:128,217), so the `else { 0 }` guard is purely
            // defensive. `epoch_transition`'s reader div-by-zero-panics on the
            // same malformed config; that divergence cannot fire in practice.
            let current_epoch = if interval > 0 {
                block_number.saturating_sub(activation) / interval as u64
            } else {
                0
            };
            let mut prev_committed: Option<u64> = None;
            loop {
                let next = read_next_epoch_to_commit(self.inner.evm_mut(), self.staking_address)?;
                if next > current_epoch + 1 {
                    break; // nothing more committable within the lookahead horizon
                }
                // Termination guard: the only loop exit is `next > current_epoch+1`,
                // which relies on every successful `commitEpochCommittee` advancing
                // the on-chain cursor. A Success that does NOT bump `nextEpochToCommit`
                // (contract bug / unexpected idempotent branch) would otherwise
                // re-derive + re-commit the same epoch forever, hanging block
                // execution with no error. Require the cursor to strictly increase
                // across iterations and fail loud otherwise.
                if let Some(p) = prev_committed {
                    if next <= p {
                        return Err(BlockExecutionError::msg(format!(
                            "commitEpochCommittee cursor stuck at epoch {next} (last \
                             committed {p}): nextEpochToCommit did not advance after a \
                             successful commit"
                        )));
                    }
                }
                let sel =
                    read_committee_selection_epoch(self.inner.evm_mut(), self.staking_address)?;
                let committee =
                    derive_committee_at(self.inner.evm_mut(), self.staking_address, sel)?;
                let calldata = commitEpochCommitteeCall { committee }.abi_encode();
                // FAIL-LOUD: the commit is liveness-critical (a missing committee
                // deadlocks the epoch boundary) and derives from deterministic
                // state, so a revert/halt is a real bug — surface it rather than
                // silently retrying (which never advances `lastCommittedEpochP1`
                // and stalls). NB: a revert lands inside `Ok(ras)` with a non-Success
                // `ras.result`; committing that state would be a no-op that also
                // never advances the cursor, so check the result explicitly.
                let ras = self
                    .inner
                    .evm_mut()
                    .transact_system_call(
                        fluentbase_types::SYSTEM_ADDRESS,
                        self.staking_address,
                        calldata.into(),
                    )
                    .map_err(|e| {
                        BlockExecutionError::msg(format!(
                            "commitEpochCommittee(epoch {next}) sys call failed: {e:?}"
                        ))
                    })?;
                match ras.result {
                    ExecutionResult::Success { .. } => {
                        self.inner.evm_mut().db_mut().commit(ras.state);
                        prev_committed = Some(next);
                    }
                    other => {
                        return Err(BlockExecutionError::msg(format!(
                            "commitEpochCommittee(epoch {next}) did not succeed: {other:?}"
                        )))
                    }
                }
            }
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
