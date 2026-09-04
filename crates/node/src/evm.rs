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
use alloy_primitives::{Address, Bytes};
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
/// Carries the operator-supplied `staking_address` so
/// [`FluentBlockExecutor::apply_pre_execution_changes`] can issue the
/// `recordProduction` / `commitEpochCommittee` system calls. Non-DPoS chains
/// pass [`Address::ZERO`] and the whole section short-circuits.
#[derive(Debug, Clone, Copy, Default)]
pub struct FluentExecutorBuilder {
    staking_address: Address,
}

impl FluentExecutorBuilder {
    pub const fn new(staking_address: Address) -> Self {
        Self { staking_address }
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
        );
        Ok(evm_config)
    }
}

#[derive(Debug, Clone)]
pub struct FluentEvmConfig {
    /// Inner evm config
    pub inner: EthEvmConfig<ChainSpec, FluentEvmFactory>,
    /// The staking system contract (per-network, operator-supplied via
    /// `StakingReaderConfig.staking_address`). Registry, epoch committee,
    /// chain-configuration views and the liveness recorder are one contract, so
    /// this one address is every DPoS system call's target.
    /// [`Address::ZERO`] disables the whole section (non-DPoS chains).
    staking_address: Address,
}

impl FluentEvmConfig {
    /// Create a new [`FluentEvmConfig`] with the given chain spec, EVM factory,
    /// and the operator-supplied staking address.
    pub fn new(
        chain_spec: Arc<ChainSpec>,
        evm_factory: FluentEvmFactory,
        staking_address: Address,
    ) -> Self {
        let inner = EthEvmConfig::new_with_evm_factory(chain_spec.clone(), evm_factory);
        Self {
            inner,
            staking_address,
        }
    }
}

impl FluentEvmConfig {
    /// Create a new [`FluentEvmConfig`] with the given chain spec and default
    /// EVM factory. The staking address defaults to [`Address::ZERO`] (non-DPoS
    /// path).
    pub fn new_with_default_factory(chain_spec: Arc<ChainSpec>) -> Self {
        Self::new(chain_spec, FluentEvmFactory::default(), Address::ZERO)
    }

    /// Returns the chain spec
    pub const fn chain_spec(&self) -> &Arc<ChainSpec> {
        self.inner.chain_spec()
    }

    /// Returns the inner EVM config
    pub const fn inner(&self) -> &EthEvmConfig<ChainSpec, FluentEvmFactory> {
        &self.inner
    }

    /// Returns the staking system contract address every DPoS system call
    /// targets.
    pub const fn staking_address(&self) -> Address {
        self.staking_address
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

// Inline ABI bindings for the ONE rWasm staking system contract: the registry,
// the epoch committee, the chain-configuration views and the liveness recorder
// all live at `StakingReaderConfig.staking_address`. Verified against
// `contracts/staking/src` in the FLU-989 worktree (`consts.rs:160,204,208` the
// selectors, `events.rs:144-221` the close events); artefact provenance in
// `devnet/local-dpos-smoke/contracts/STAKING_ARTEFACT.md`. This `sol!()` macro IS
// the ABI source of truth on the Rust side.
alloy_sol_types::sol! {
    // The ONE liveness system call: who produced this block. The height is NOT an
    // argument — the contract reads `block.number` from its own context
    // (`liveness.rs:53`) and uses it as both idempotency key and epoch cursor,
    // because a height it derives cannot disagree with the block it executes in.
    // `leaderIndex` is the asymmetric half, underivable on-chain, which is why it
    // is verified at vote time instead. The epoch close — verdicts, exclusion
    // stamps AND the stipend settlement — is driven from inside the contract, so
    // there is no second call.
    function recordProduction(uint8 leaderIndex) external;

    // Close-time events emitted by `recordProduction` (system call) — decoded from
    // the Success `ras.logs` for node-side observability only. System calls produce
    // no receipt, so these logs are otherwise invisible. Keep byte-identical to
    // `contracts/staking/src/events.rs` or the topic match below silently never
    // fires.
    //
    // `PartialEpoch` is the load-bearing one: the partial-epoch taint is DERIVED
    // from the recorded block count, so ONE unrecorded block silently costs a whole
    // epoch its verdicts and the tier reads as enabled while judging nothing. Alert
    // on any occurrence outside epoch 0 (partial by construction).
    event PartialEpoch(uint64 indexed epoch, uint32 recorded, uint32 expected);
    // The contract keeps only the last `WEIGHT_RING_EPOCHS` epochs of frozen
    // leader weights, so an epoch closed far enough behind can be neither judged
    // nor paid. It forfeits rather than reverting — the close is a pre-execution
    // system call — and this event is the ONLY thing that says so: on chain the
    // result is indistinguishable from the four legitimate zero-epochs. Should
    // be unreachable; an occurrence means the bound behind that constant is wrong.
    event EpochWeightsUnavailable(uint64 indexed epoch, uint32 members);
    event ProductionVerdictFailed(
        uint64 indexed epoch,
        address indexed validator,
        uint32 produced,
        uint256 due
    );
    event CorrelatedFailureEpoch(uint64 indexed epoch, uint256 newFailures, uint256 tolerance);
    event StipendLegSkipped(uint64 indexed epoch);

    // The epoch-committee freeze, system-caller only. It takes NO argument: the
    // contract derives the committee itself and sorts it ascending by peer pubkey
    // (`consensus.rs:502,538`), which IS the consensus index space `leaderIndex`
    // and the slash `signerIdx` are resolved against. The node used to derive the
    // same set and pass it in, but the contract's own derivation was the authority
    // the check compared against — so the only disagreement it could detect was
    // with the caller, and its only remedy was to revert. With one derivation
    // there is nothing left to disagree.
    function commitEpochCommittee() external;

    // The commit found fewer than MIN_COMMITTEE_LENGTH eligible validators and
    // re-seated the previous epoch's committee rather than reverting — the
    // revert being a pre-execution block-execution error on every node, which
    // one validator owner could trigger by withdrawing their own stake.
    //
    // Decoded here for the same reason the close events are: logs emitted inside
    // a pre-execution system call never become receipts, so `eth_getLogs` shows
    // NOTHING of them (checked on the devnet: a 1,296-block chain with ~40
    // commits carried exactly two staking logs, both from an ordinary
    // transaction). This line and its counter are therefore the ONLY way anyone
    // learns the chain is in the carried state — and it is a state that must be
    // steered out of, because the seats stay filled by validators the selection
    // would no longer choose and are not replaced until the population recovers.
    event CommitteeCarriedOver(uint64 indexed epoch, uint32 eligible, uint32 members);

    // `slashEquivocation(uint64,uint32)` — the equivocation VERDICT,
    // system-caller only. It carries no evidence and the contract verifies none:
    // every committee member checked the charge against the evidence in the
    // OrderBlock before voting for the block, so the chain applies what the
    // committee already agreed. The evidence cannot travel here — a node syncing
    // the EL from peers has no OrderBlock — which is why only the one-byte
    // verdict rides in `extra_data`, verbatim into the header.
    function slashEquivocation(uint64 epoch, uint32 signerIdx) external;

    // Stipend-settlement events emitted by the settle leg the epoch CLOSE drives —
    // so they arrive on the `recordProduction` system call's logs, once per epoch
    // instead of once per block. Decoded for node-side observability only (system
    // calls produce no receipt). Same contract as the liveness events above, which
    // is why the router below matches on TOPIC and not on `log.address`. Keep
    // byte-identical to `contracts/staking/src/events.rs:211-221`.
    event EpochBlendRewardsCommitted(uint64 indexed epoch, uint256 blendAmount);
    event StipendSkipped(uint64 indexed epoch);

    // Ahead-commit pipeline (2-epoch committee warm-up): committee[N] is committed
    // TWO epochs ahead from EffBal(N-2). `nextEpochToCommit` = the next-uncommitted
    // epoch N, and with the commit call now argument-free it is the ONLY node-side
    // evidence that a commit did anything — the ahead-commit loop's termination and
    // its cursor-stuck guard both rest on it.
    function nextEpochToCommit() external view returns (uint64);

    // Chain-configuration views. Formerly a separate `ChainConfig` predeploy; same
    // contract now, so the same address.
    function getEpochBlockInterval() external view returns (uint32);
    function getDposActivationBlock() external view returns (uint64);
}

fn encode_record_production_call(leader_index: u8) -> Vec<u8> {
    use alloy_sol_types::SolCall;
    recordProductionCall {
        leaderIndex: leader_index,
    }
    .abi_encode()
}

/// `accused` is a committee position (`u8` on the wire, always < 51); the
/// contract takes it as `uint32`, so the widening is total.
fn encode_slash_equivocation_call(epoch: u64, accused: u8) -> Vec<u8> {
    use alloy_sol_types::SolCall;
    slashEquivocationCall {
        epoch,
        signerIdx: accused.into(),
    }
    .abi_encode()
}

/// Surface the close-time events of the `recordProduction` system call in node
/// logs + metrics. System calls produce no receipt and the executor commits and
/// discards `ras.logs`, so these are otherwise invisible without forensic
/// archaeology. Pure observability — reads the emitted logs, mutates no state,
/// runs identically on every node.
///
/// The liveness events AND the stipend events arrive on this one call from the
/// one staking contract, so they are told apart by TOPIC, not by `log.address`.
/// `SolEvent::decode_log` verifies topic0 itself, so this decode chain IS the
/// topic match: the seven signatures are distinct — including the same-arity pair
/// `StipendLegSkipped` / `StipendSkipped`, which differ by name and therefore by
/// topic0 — and a log that matches none of them falls through untouched.
///
/// The chain being CLOSED is the trap worth naming: adding an event to the
/// contract means adding an arm here too, or it is emitted into silence on the
/// one call path that would otherwise have surfaced it.
/// Surface the one `commitEpochCommittee` outcome an operator has to act on.
///
/// The commit's other event (`EpochCommitteeCommitted`) is the ordinary case and
/// says nothing actionable, so it is deliberately not decoded. This one says the
/// selection came back below the floor: the chain kept going on the previous
/// committee, and it will keep doing that, epoch after epoch, until enough
/// validators are registered, keyed and activated again. `error!` and not `warn!`
/// because nothing else reports it — a system call's logs never reach a receipt.
fn emit_commit_observability(logs: &[alloy_primitives::Log]) {
    use alloy_sol_types::SolEvent;
    for log in logs {
        if let Ok(carried) = CommitteeCarriedOver::decode_log(log) {
            tracing::error!(
                target: "fluentbase::consensus",
                epoch = carried.epoch,
                eligible = carried.eligible,
                members = carried.members,
                "epoch_committee_carried_over"
            );
            metrics::counter!("dpos_epoch_committee_carried_over_total").increment(1);
        }
    }
}

fn emit_close_observability(logs: &[alloy_primitives::Log]) {
    use alloy_sol_types::SolEvent;
    for log in logs {
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
        } else if let Ok(committed) = EpochBlendRewardsCommitted::decode_log(log) {
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
        } else if let Ok(lost) = EpochWeightsUnavailable::decode_log(log) {
            // `error!`, not `warn!`: the epoch forfeited BOTH its verdicts and
            // its stipend, and on chain the outcome is byte-identical to an epoch
            // that legitimately owed nothing. This line is the only thing that
            // tells them apart.
            tracing::error!(
                target: "fluentbase::rewards",
                epoch = lost.epoch,
                members = lost.members,
                "epoch_weights_unavailable"
            );
            metrics::counter!("dpos_epoch_weights_unavailable_total").increment(1);
        }
    }
}

/// Read `getEpochBlockInterval()` via system call at the current pre-execution
/// state.
fn read_epoch_block_interval<E>(
    evm: &mut E,
    staking_address: Address,
) -> Result<u32, BlockExecutionError>
where
    E: Evm,
{
    use alloy_sol_types::SolCall;
    let calldata = getEpochBlockIntervalCall {}.abi_encode().into();
    let output = transact_view(evm, staking_address, calldata, "epoch_block_interval")?;
    getEpochBlockIntervalCall::abi_decode_returns(&output)
        .map_err(|e| BlockExecutionError::msg(format!("epoch_block_interval decode: {e:?}")))
}

/// DPoS activation height as a *scheduling state*, read resiliently against the
/// pre-execution state — the executor-side mirror of
/// [`fluentbase_staking_reader::reader::RethStakingStateReader::scheduled_dpos_activation`].
///
/// Returns:
/// - `Ok(None)` when the staking contract is not (yet) a readably-scheduled DPoS
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
/// definition of "this contract is not a scheduled DPoS staking contract yet".
/// Once `Some(h)` is observed the contract is fully initialized, so every
/// subsequent read in the DPoS section (interval, commit cursor) stays fail-loud.
fn scheduled_dpos_activation<E>(
    evm: &mut E,
    staking_address: Address,
) -> Result<Option<u64>, BlockExecutionError>
where
    E: Evm,
{
    use alloy_sol_types::SolCall;
    use fluentbase_revm::revm::context_interface::result::{ExecutionResult, Output};

    let calldata = getDposActivationBlockCall {}.abi_encode().into();
    let ras = evm
        .transact_system_call(fluentbase_types::SYSTEM_ADDRESS, staking_address, calldata)
        .map_err(|e| {
            BlockExecutionError::msg(format!("dpos_activation_block read failed: {e:?}"))
        })?;
    let output = match ras.result {
        // A CODELESS account also lands here — Success with EMPTY output, not a
        // revert — and `classify_scheduled_activation` folds it to `None`.
        ExecutionResult::Success { output, .. } => Some(match output {
            Output::Call(b) | Output::Create(b, _) => b,
        }),
        // A proxy whose impl isn't coded yet / a contract mid-runtime-deploy →
        // "not a scheduled DPoS staking contract at this state". Skip the whole
        // DPoS section rather than wedging the payload builder.
        ExecutionResult::Revert { .. } | ExecutionResult::Halt { .. } => None,
    };
    classify_scheduled_activation(output)
}

/// Pure decode+classify step of [`scheduled_dpos_activation`], split out so the
/// gate's decision logic is unit-testable without a live EVM. Every "the staking
/// contract is not a readable, scheduled DPoS config at this state" case folds
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

/// The two on-chain effects the ahead-commit loop drives, abstracted so the
/// loop's horizon/termination CONTROL FLOW is unit-testable without a live
/// Staking deployment (the migration/backlog behaviour is the loop's logic, not
/// the EVM plumbing). The production impl ([`EvmAheadCommit`]) reads
/// `nextEpochToCommit()` and issues the `commitEpochCommittee` system call (which
/// advances the on-chain cursor); the test impl models the cursor in memory.
trait AheadCommitDriver {
    /// Read the on-chain commit cursor (`nextEpochToCommit()`).
    fn read_next_epoch(&mut self) -> Result<u64, BlockExecutionError>;
    /// Commit `committee[target]` (advances the on-chain cursor to `target+1` on
    /// success). `target` is the cursor value this iteration read; the call
    /// itself carries no argument, so it is diagnostic context only.
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
        // 2-epoch committee warm-up: the contract selects from FINAL
        // `EffBal(target−2)` and deterministically sets
        // `dkgQual[target] = (derived set != committee[target−1])` inside the
        // commit — no deferral, no qualify-before-commit branch. `committee[target]`
        // is thus frozen a full epoch before its DKG runs (during target−1). The
        // node supplies nothing: the contract's own derivation is the authority,
        // so a set passed in could only ever disagree with it.
        let calldata = commitEpochCommitteeCall {}.abi_encode();
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
            ExecutionResult::Success { ref logs, .. } => {
                emit_commit_observability(logs);
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
    /// The staking system contract every DPoS system call targets.
    /// [`Address::ZERO`] disables the whole pre-execution DPoS section.
    staking_address: Address,
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
        // commitEpochCommittee system call are bypassed on non-DPoS chains
        // (`staking_address` zero). Prior to this gate the decoder ran
        // unconditionally and mapped reth's default `"reth/v..."` extra_data to a
        // fail-loud BlockExecutionError, stalling every non-DPoS block at #1.
        if !self.staking_address.is_zero() {
            use fluentbase_revm::revm::context_interface::result::ExecutionResult;
            use fluentbase_revm::revm::context_interface::Block as _;
            use fluentbase_revm::revm::DatabaseCommit as _;

            // SINGLE gate (P2-2): the whole DPoS epoch-commit section engages ONLY
            // once DPoS activation is both SCHEDULED and READABLE at this
            // pre-execution state. `scheduled_dpos_activation` folds three
            // pre-DPoS states into the same `None` → skip:
            //   - the staking contract has no code yet (migrated prod chain
            //     resyncing history from genesis where the predeploys aren't in
            //     the chainspec; or a fresh chain before the runtime deploy);
            //   - it exists but `getDposActivationBlock()` reverts (a
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
                scheduled_dpos_activation(self.inner.evm_mut(), self.staking_address)?
            else {
                return Ok(());
            };

            let block_number: u64 = self.inner.evm().block().number().saturating_to();
            // Relative epoch numbering: the contract's commit cursor counts
            // epochs from `dposActivationBlock`, so the ahead-commit horizon must
            // match or the catch-up loop misfires. Safe to read fail-loud now — a
            // scheduled activation implies an initialized contract with
            // `epochBlockInterval > 0`.
            let interval = read_epoch_block_interval(self.inner.evm_mut(), self.staking_address)?;

            // `interval == 0` is unreachable on a live chain: the contract requires
            // epochBlockInterval > 0 on both init and every setter, so the
            // `else { 0 }` guard is purely defensive (`epoch_of_block` divides).
            // Shared activation-relative epoch math (the single definition in
            // `staking-reader`) so the ahead-commit horizon below and the
            // equivocation verdict can never drift from the consensus/cold-start
            // epocher.
            //
            // Computed HERE, above the recorder, because the verdict leg needs it
            // and it is a pure function of values already in hand — no EVM read,
            // so hoisting it moves no state access across the recorder. The
            // ahead-commit driver's own ordering constraint (see NOTE ON ORDER
            // below) is about the driver, not this arithmetic, and is untouched.
            let current_epoch = if interval > 0 {
                fluentbase_staking_reader::reader::epoch_of_block(
                    block_number,
                    interval,
                    activation,
                )
            } else {
                0
            };

            // System-call the staking contract with THIS block's producer, decoded
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
            //   len == 0 ⇒ no syscall;  len == PRODUCTION_RECORD_LEN ⇒ decode +
            //   syscall;  else ⇒ fail loud.
            if block_number >= activation {
                let extra_data = self.inner.ctx.extra_data.clone();
                let record =
                    fluentbase_consensus::extra_data::decode_production_record(&extra_data)
                        .map_err(|e| BlockExecutionError::msg(format!("production record: {e}")))?;
                if let Some(record) = record {
                    let calldata = encode_record_production_call(record.leader_index);
                    let ras = self
                        .inner
                        .evm_mut()
                        .transact_system_call(
                            fluentbase_types::SYSTEM_ADDRESS,
                            self.staking_address,
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
                            emit_close_observability(&logs);
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

                    // Apply the equivocation verdict this block carries, if any.
                    // The accused index comes from the SAME header bytes as the
                    // producer index above, which is what lets a node syncing the
                    // EL from peers — it has no OrderBlock and so no evidence —
                    // reach the identical post-state.
                    //
                    // SOFT, unlike `recordProduction` above, and deliberately so.
                    // A revert or halt here is a deterministic function of the same
                    // call against the same pre-block state on every node, so
                    // folding it to a skip cannot diverge the state root. What it
                    // buys is that neither of the two benign races this design
                    // ACCEPTS can halt the chain: two proposers may carry the same
                    // charge, and the epoch-boundary fallback transaction may land
                    // beside a block-borne one. The contract already returns
                    // `Ok(())` on an already-tombstoned victim, so the ordinary
                    // duplicate does not even reach these arms — they cover the
                    // residue (an index the epoch's committee array cannot resolve,
                    // an out-of-gas). Fail-loud there, as `recordProduction` does,
                    // would convert a lost slash into a stalled chain, which is the
                    // strictly worse trade for a penalty the fallback route can
                    // still land.
                    if let Some(accused) = record.accused {
                        let calldata = encode_slash_equivocation_call(current_epoch, accused);
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
                                    "slashEquivocation sys call: {e:?}"
                                ))
                            })?;
                        match ras.result {
                            ExecutionResult::Success { .. } => {
                                self.inner.evm_mut().db_mut().commit(ras.state)
                            }
                            // Loud in the log, silent on the state — the fold is
                            // the only thing standing between a mis-encoded
                            // selector and an unexplained missing slash.
                            ExecutionResult::Revert { output, .. } => {
                                tracing::warn!(
                                    target: "fluentbase::slashing",
                                    epoch = current_epoch,
                                    accused,
                                    output = %alloy_primitives::hex::encode(&output),
                                    "slash_equivocation_reverted"
                                );
                                metrics::counter!("dpos_slash_equivocation_skipped_total")
                                    .increment(1);
                            }
                            ExecutionResult::Halt { reason, .. } => {
                                tracing::warn!(
                                    target: "fluentbase::slashing",
                                    epoch = current_epoch,
                                    accused,
                                    ?reason,
                                    "slash_equivocation_halted"
                                );
                                metrics::counter!("dpos_slash_equivocation_skipped_total")
                                    .increment(1);
                            }
                        }
                    }
                }
            }

            // Commit the canonical committee two epochs ahead (the 2-epoch
            // committee warm-up): catch up every uncommitted epoch within the
            // lookahead horizon (`nextEpochToCommit() <= currentEpoch+2`). The
            // contract selects each set itself, from the committed epoch's N-2, so
            // the node only paces the calls and watches the cursor advance.
            // Steady state: one commit per epoch; genesis/migration:
            // catches up a small backlog. Runs pre-activation too — the pre-swap
            // sequencer commits committees so the first DPoS epoch's set is already
            // on-chain at activation. Its horizon reads the `current_epoch`
            // computed above the recorder.

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
    /// The DPoS epoch-commit pre-execution gate must be INERT whenever the
    /// activation read is unreadable or unscheduled — the root fix for the
    /// runtime-deploy deadlock (a pre-DPoS sequencer launched with
    /// `--dpos.staking-config` whose staking contract is mid-runtime-deploy must
    /// NOT fail-loud per block, or it stalls the chain and the deploy txns can
    /// never mine). `None` = the read reverted/halted (codeless / proxy whose
    /// impl isn't coded yet); a decoded `0` = the unscheduled sentinel. Both map
    /// to `Ok(None)` (skip); only a nonzero height engages the section.
    #[test]
    fn unreadable_or_unscheduled_staking_contract_is_inert() {
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

    /// The node-side close observability reads the staking contract's close
    /// events out of the discarded `recordProduction` system-call logs. The Rust
    /// `sol!` event ABI must stay byte-identical to
    /// `contracts/staking/src/events.rs` or the `decode_log` topic match silently
    /// never fires — and for `PartialEpoch` and `EpochWeightsUnavailable` that
    /// silence is the whole failure mode they exist to break. Pin all seven
    /// canonical signatures and prove a fabricated log decodes to the exact field
    /// values.
    ///
    /// Every log is fabricated at ONE address, because that is now the truth: the
    /// liveness recorder and the stipend settlement are the same contract, so the
    /// router matches on topic alone. A fixture that spread them across two
    /// addresses would pass for the wrong reason.
    #[test]
    fn close_events_decode_from_fabricated_logs() {
        use super::{
            CommitteeCarriedOver, CorrelatedFailureEpoch, EpochBlendRewardsCommitted,
            EpochWeightsUnavailable, PartialEpoch, ProductionVerdictFailed, StipendLegSkipped,
            StipendSkipped,
        };
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
        assert_eq!(StipendLegSkipped::SIGNATURE, "StipendLegSkipped(uint64)");
        assert_eq!(
            EpochBlendRewardsCommitted::SIGNATURE,
            "EpochBlendRewardsCommitted(uint64,uint256)"
        );
        assert_eq!(StipendSkipped::SIGNATURE, "StipendSkipped(uint64)");
        assert_eq!(
            EpochWeightsUnavailable::SIGNATURE,
            "EpochWeightsUnavailable(uint64,uint32)"
        );
        // The COMMIT-path event, decoded by `emit_commit_observability` rather
        // than by the close router. Its topic0 was read straight off the
        // contract (`events::CommitteeCarriedOver::SELECTOR`) and pinned here as
        // a literal, not recomputed from this signature: recomputing would make
        // both halves of the pin come from the same side, which is precisely the
        // failure this class of test keeps producing.
        assert_eq!(
            CommitteeCarriedOver::SIGNATURE,
            "CommitteeCarriedOver(uint64,uint32,uint32)"
        );
        assert_eq!(
            CommitteeCarriedOver::SIGNATURE_HASH,
            alloy_primitives::b256!(
                "ed59ae1f26b3006ee27f22cdcc4adb48a0c039c2dee83b0d80de5795bfecb8e5"
            )
        );

        // The same-arity pair. Only the NAME separates them, so only topic0 can —
        // which is the property the address-free router now rests on.
        assert_ne!(
            StipendLegSkipped::SIGNATURE_HASH,
            StipendSkipped::SIGNATURE_HASH
        );

        const CONTRACT: alloy_primitives::Address = alloy_primitives::Address::repeat_byte(0xcc);
        let fabricate = |data| alloy_primitives::Log {
            address: CONTRACT,
            data,
        };
        let validator = alloy_primitives::address!("00000000000000000000000000000000000000aa");

        let partial_log = fabricate(
            PartialEpoch {
                epoch: 7,
                recorded: 31,
                expected: 32,
            }
            .encode_log_data(),
        );
        let decoded = PartialEpoch::decode_log(&partial_log).expect("fabricated log must decode");
        assert_eq!(decoded.epoch, 7);
        assert_eq!(decoded.recorded, 31);
        assert_eq!(decoded.expected, 32);

        let lost_log = fabricate(
            EpochWeightsUnavailable {
                epoch: 11,
                members: 51,
            }
            .encode_log_data(),
        );
        let decoded =
            EpochWeightsUnavailable::decode_log(&lost_log).expect("fabricated log must decode");
        assert_eq!(decoded.epoch, 11);
        assert_eq!(decoded.members, 51);

        let carried_log = fabricate(
            CommitteeCarriedOver {
                epoch: 20,
                eligible: 3,
                members: 4,
            }
            .encode_log_data(),
        );
        let decoded_carried =
            CommitteeCarriedOver::decode_log(&carried_log).expect("fabricated log must decode");
        assert_eq!(decoded_carried.epoch, 20);
        assert_eq!(decoded_carried.eligible, 3);
        assert_eq!(decoded_carried.members, 4);

        let failed_log = fabricate(
            ProductionVerdictFailed {
                epoch: 9,
                validator,
                produced: 4,
                due: alloy_primitives::U256::from(100u64),
            }
            .encode_log_data(),
        );
        let decoded_failed =
            ProductionVerdictFailed::decode_log(&failed_log).expect("fabricated log must decode");
        assert_eq!(decoded_failed.epoch, 9);
        assert_eq!(decoded_failed.validator, validator);
        assert_eq!(decoded_failed.produced, 4);

        let corr_log = fabricate(
            CorrelatedFailureEpoch {
                epoch: 11,
                newFailures: alloy_primitives::U256::from(3u64),
                tolerance: alloy_primitives::U256::from(2u64),
            }
            .encode_log_data(),
        );
        let decoded_corr =
            CorrelatedFailureEpoch::decode_log(&corr_log).expect("fabricated log must decode");
        assert_eq!(decoded_corr.epoch, 11);
        assert_eq!(decoded_corr.newFailures, alloy_primitives::U256::from(3u64));

        let leg_log = fabricate(StipendLegSkipped { epoch: 13 }.encode_log_data());
        assert_eq!(
            StipendLegSkipped::decode_log(&leg_log)
                .expect("fabricated log must decode")
                .epoch,
            13
        );

        let committed_log = fabricate(
            EpochBlendRewardsCommitted {
                epoch: 15,
                blendAmount: alloy_primitives::U256::from(4200u64),
            }
            .encode_log_data(),
        );
        let decoded_committed = EpochBlendRewardsCommitted::decode_log(&committed_log)
            .expect("fabricated log must decode");
        assert_eq!(decoded_committed.epoch, 15);
        assert_eq!(
            decoded_committed.blendAmount,
            alloy_primitives::U256::from(4200u64)
        );

        let skipped_log = fabricate(StipendSkipped { epoch: 17 }.encode_log_data());
        assert_eq!(
            StipendSkipped::decode_log(&skipped_log)
                .expect("fabricated log must decode")
                .epoch,
            17
        );

        // Distinct topic0 — one event's log must never decode as another's, and
        // this is the whole of the routing now that `log.address` is out of it.
        assert!(PartialEpoch::decode_log(&failed_log).is_err());
        assert!(ProductionVerdictFailed::decode_log(&partial_log).is_err());
        assert!(StipendLegSkipped::decode_log(&skipped_log).is_err());
        assert!(StipendSkipped::decode_log(&leg_log).is_err());
    }

    /// The one syscall the executor injects per block. Its selector and argument
    /// packing are the whole contract interface, and a silent drift mis-credits
    /// production with no other symptom. The height is gone from the wire — the
    /// contract reads `block.number` itself — so the leader index is now the
    /// FIRST and only word, which is what the executor tests below read back.
    #[test]
    fn record_production_calldata_is_pinned() {
        use alloy_sol_types::SolCall;
        assert_eq!(
            super::recordProductionCall::SIGNATURE,
            "recordProduction(uint8)"
        );
        assert_eq!(
            super::recordProductionCall::SELECTOR,
            [0x17, 0x52, 0x91, 0x0e]
        );
        // cast calldata "recordProduction(uint8)" 50
        let encoded = super::encode_record_production_call(50);
        assert_eq!(
            encoded,
            alloy_primitives::hex!(
                "1752910e"
                "0000000000000000000000000000000000000000000000000000000000000032"
            )
        );
        let decoded = super::recordProductionCall::abi_decode(&encoded).expect("roundtrip");
        assert_eq!(decoded.leaderIndex, 50);
    }

    /// The committee freeze is fail-loud on a syscall path, so a selector drift
    /// is a halted chain rather than a missed effect. It takes no argument, which
    /// leaves the selector as the entire wire — and the on-chain cursor as the
    /// only evidence the call did anything.
    #[test]
    fn commit_epoch_committee_selector_is_pinned() {
        use alloy_sol_types::SolCall;
        assert_eq!(
            super::commitEpochCommitteeCall::SIGNATURE,
            "commitEpochCommittee()"
        );
        assert_eq!(
            super::commitEpochCommitteeCall::SELECTOR,
            [0xe5, 0x05, 0xb2, 0x49]
        );
        assert_eq!(
            super::commitEpochCommitteeCall {}.abi_encode(),
            alloy_primitives::hex!("e505b249")
        );
    }

    /// The ahead-commit cursor read. Argument-free, so the selector is the
    /// entire wire; it is also the only node-side evidence a commit did
    /// anything, and the commit loop's termination and stuck-cursor guard both
    /// rest on it. A rename would leave the loop reading a revert.
    ///
    /// Pinned literally against the contract's `SIG_NEXT_EPOCH_TO_COMMIT`
    /// (`cast sig "nextEpochToCommit()"` == `0xc06a82de`, matching
    /// `contracts/staking/src/consts.rs` on `feat/flu-989-port-solidity-delta`).
    /// This one AGREES with the contract today — no drift to record.
    #[test]
    fn next_epoch_to_commit_selector_is_pinned() {
        use alloy_sol_types::SolCall;
        assert_eq!(
            super::nextEpochToCommitCall::SIGNATURE,
            "nextEpochToCommit()"
        );
        assert_eq!(
            super::nextEpochToCommitCall::SELECTOR,
            [0xc0, 0x6a, 0x82, 0xde],
            "node-side nextEpochToCommit selector drifted from the contract's \
             SIG_NEXT_EPOCH_TO_COMMIT (0xc06a82de, feat/flu-989-port-solidity-delta)"
        );
        assert_eq!(
            super::nextEpochToCommitCall {}.abi_encode(),
            alloy_primitives::hex!("c06a82de")
        );
    }

    /// The verdict syscall is SOFT-failed, so a selector or argument drift
    /// against `contracts/staking` costs nothing at execution time and shows up
    /// only as slashes that never land. Pin the signature, the 4-byte selector,
    /// and the `u8 → uint32` widening of the committee position.
    ///
    /// **The contract has no counterpart at all** — `slashEquivocation(uint64,
    /// uint32)` is dispatched by neither `feat/flu-989-port-solidity-delta` nor
    /// `origin/feat/flu-989-rust-staking` (verified 2026-08-14: zero hits for
    /// the signature in `consts.rs` on every branch in this repo that carries
    /// the contract). So this pin is one-sided by necessity: it holds the node
    /// still and names the gap, and cannot be made two-sided until the verdict
    /// path exists on-chain. Merge checklist:
    /// `crates/dpos/consensus/src/slasher/actor.rs`, entry 4.
    #[test]
    fn slash_equivocation_calldata_is_pinned() {
        use alloy_sol_types::SolCall;
        assert_eq!(
            super::slashEquivocationCall::SIGNATURE,
            "slashEquivocation(uint64,uint32)"
        );
        assert_eq!(
            super::slashEquivocationCall::SELECTOR,
            [0xdc, 0x6f, 0xb3, 0xf2],
            "node-side slashEquivocation(uint64,uint32) selector drifted from the pinned \
             0xdc6fb3f2; the contract side is ABSENT on every branch carrying \
             contracts/staking (checked feat/flu-989-port-solidity-delta and \
             origin/feat/flu-989-rust-staking), so there is nothing to re-derive it from"
        );
        // cast calldata "slashEquivocation(uint64,uint32)" 9 50
        let encoded = super::encode_slash_equivocation_call(9, 50);
        assert_eq!(
            encoded,
            alloy_primitives::hex!(
                "dc6fb3f2"
                "0000000000000000000000000000000000000000000000000000000000000009"
                "0000000000000000000000000000000000000000000000000000000000000032"
            )
        );
        let decoded = super::slashEquivocationCall::abi_decode(&encoded).expect("roundtrip");
        assert_eq!(decoded.epoch, 9);
        assert_eq!(decoded.signerIdx, 50);
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

    /// EVM-bytecode stand-in for the staking contract. The real contract is
    /// rWasm built in a separate repository, so a node-crate test cannot deploy
    /// it; this answers exactly the reads the pre-execution section makes and
    /// records the calls it issues, which is what lets the tests below drive the
    /// real [`FluentBlockExecutor`] rather than a mock.
    ///
    /// One account serves all five entry points, because that is now the shape
    /// of the deployment. A constant-answer stub cannot: the three views want
    /// three different numbers, so the answers are seeded per selector into
    /// storage and the code dispatches by reading its own slot.
    mod stub {
        use alloy_primitives::Bytes;

        /// A genesis account holds rWasm, so EVM bytecode reaches one only
        /// wrapped as an ownable account owned by the EVM runtime — exactly what
        /// `crates/genesis/build.rs` does for the real predeploys. Without the
        /// wrapper the executor rejects the account with `NotSupportedBytecode`
        /// and every read below folds to "no DPoS here", silently passing the
        /// tests for the wrong reason.
        pub fn as_genesis_code(evm_bytecode: Vec<u8>) -> Bytes {
            use fluentbase_evm::EthereumMetadata;
            use fluentbase_revm::revm::bytecode::Bytecode;
            Bytecode::new_ownable_account(
                fluentbase_types::PRECOMPILE_EVM_RUNTIME,
                EthereumMetadata::new_analyzed(Bytes::from(evm_bytecode)).write_to_bytes(),
            )
            .bytes()
        }

        /// Answers `sload(selector)` and, on the way, records the call:
        /// `sstore(selector, 1)` and `sstore(selector + 1, calldata_word_1)`.
        /// The selector-keyed slots are what the assertions read — slot
        /// occupancy proves the call was issued, and the neighbouring slot
        /// proves which arguments it carried.
        ///
        /// The answer is loaded BEFORE the receipt overwrites the slot, so one
        /// slot serves both roles: seeded, it is a view's answer; unseeded, it
        /// is the receipt flag a system call sets. A view's receipt never
        /// survives anyway — the executor discards those frames' state.
        pub fn recording() -> Vec<u8> {
            vec![
                // selector = calldataload(0) >> 224
                0x60, 0x00, 0x35, 0x60, 0xe0, 0x1c, //
                // answer = sload(selector), read before the receipt clobbers it
                0x80, 0x54, 0x90, //
                // sstore(selector, 1)
                0x80, 0x60, 0x01, 0x90, 0x55, //
                // sstore(selector + 1, calldataload(4))
                0x60, 0x04, 0x35, 0x90, 0x60, 0x01, 0x01, 0x55, //
                // return(0, 32) of the answer
                0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3,
            ]
        }

        /// [`recording`], but reverting on a 68-byte calldata — the exact width
        /// of `slashEquivocation(uint64,uint32)`, and the only 68-byte call the
        /// pre-execution section makes. Models the residue the soft fold exists
        /// for: a verdict the contract refuses.
        pub fn recording_rejecting_the_verdict() -> Vec<u8> {
            reverting_on_calldata_size(0x44)
        }

        /// [`recording`] with a leading `calldatasize == size ⇒ revert` guard.
        fn reverting_on_calldata_size(size: u8) -> Vec<u8> {
            let body = recording();
            let jumpdest = 7 + body.len();
            assert!(jumpdest < 256, "single-byte jump target");
            // if calldatasize == size { jump to the revert }
            let mut code = vec![0x60, size, 0x36, 0x14, 0x60, jumpdest as u8, 0x57];
            code.extend_from_slice(&body);
            code.extend_from_slice(&[0x5b, 0x60, 0x00, 0x60, 0x00, 0xfd]);
            code
        }
    }

    /// One block executed through the real pre-execution section against the
    /// [`stub`] staking contract.
    struct StubbedBlock {
        block: reth_primitives_traits::RecoveredBlock<reth_ethereum_primitives::Block>,
        bundle: fluentbase_revm::revm::database::BundleState,
        state_root: alloy_primitives::B256,
    }

    /// Fixed position for the stub. Deliberately NOT the canonical predeploy
    /// slot — nothing here mirrors a real deployment.
    const STUB_STAKING: alloy_primitives::Address = alloy_primitives::Address::repeat_byte(0xa1);

    /// `getDposActivationBlock()` AND `getEpochBlockInterval()` both answer this,
    /// so the block's epoch is `block_number - 1` — a value distinct from the
    /// block number, which is what makes the recorded epoch argument load-bearing
    /// rather than incidental.
    const STUB_ACTIVATION_AND_INTERVAL: u64 = 1;
    /// The stub's answer to `nextEpochToCommit()`. Far beyond the
    /// `current_epoch + 2` horizon, so `drive_ahead_commit` exits on its first
    /// read and the committee leg stays out of these tests.
    const STUB_NEXT_EPOCH_TO_COMMIT: u64 = 100_000;
    /// The parent height. High enough that the block's epoch is unmistakably
    /// neither zero nor the block number.
    const STUB_PARENT_NUMBER: u64 = 100;
    /// The leader index the fabricated `extra_data` carries. Distinct from the
    /// block number and from its epoch, so the word `recordProduction` records
    /// can only be this one.
    const STUB_LEADER_INDEX: u8 = 3;

    /// The genesis, chain spec and provider the stub lives in, plus the
    /// `FluentEvmConfig` pointed at it. `staking_code` is the only variable —
    /// the tests differ in how the stub answers the verdict.
    fn stub_chain(
        staking_code: Vec<u8>,
    ) -> (
        super::FluentEvmConfig,
        reth_provider::providers::BlockchainProvider<
            reth_provider::test_utils::MockNodeTypesWithDB,
        >,
        alloy_primitives::B256,
        reth_primitives_traits::SealedHeader,
    ) {
        stub_chain_at_interval(staking_code, STUB_ACTIVATION_AND_INTERVAL)
    }

    /// [`stub_chain`] with `getEpochBlockInterval()` answering `interval`
    /// instead of [`STUB_ACTIVATION_AND_INTERVAL`]. Only the epoch-boundary
    /// height gate cares; everything else in the section reads the activation,
    /// which stays put.
    fn stub_chain_at_interval(
        staking_code: Vec<u8>,
        interval: u64,
    ) -> (
        super::FluentEvmConfig,
        reth_provider::providers::BlockchainProvider<
            reth_provider::test_utils::MockNodeTypesWithDB,
        >,
        alloy_primitives::B256,
        reth_primitives_traits::SealedHeader,
    ) {
        use alloy_genesis::GenesisAccount;
        use alloy_primitives::{B256, U256};
        use alloy_sol_types::SolCall as _;
        use reth_chainspec::{
            make_genesis_header, BaseFeeParams, BaseFeeParamsKind, Chain, ChainSpec, DEV_HARDFORKS,
        };
        use reth_db_common::init::init_genesis;
        use reth_provider::test_utils::create_test_provider_factory_with_chain_spec;

        use super::{FluentEvmConfig, FluentEvmFactory};

        // The three view answers, keyed by the selector the stub dispatches on.
        // Taken from the `sol!` types rather than written out, so a signature
        // drift moves the fixture with the caller instead of silently answering
        // a selector nothing asks for.
        let answer = |selector: [u8; 4], value: u64| {
            (
                B256::from(U256::from(u32::from_be_bytes(selector))),
                B256::from(U256::from(value)),
            )
        };
        let genesis = fluentbase_genesis::local_genesis_from_file().extend_accounts([(
            STUB_STAKING,
            GenesisAccount::default()
                .with_code(Some(stub::as_genesis_code(staking_code)))
                .with_storage(Some(
                    [
                        answer(
                            super::getDposActivationBlockCall::SELECTOR,
                            STUB_ACTIVATION_AND_INTERVAL,
                        ),
                        answer(super::getEpochBlockIntervalCall::SELECTOR, interval),
                        answer(
                            super::nextEpochToCommitCall::SELECTOR,
                            STUB_NEXT_EPOCH_TO_COMMIT,
                        ),
                    ]
                    .into_iter()
                    .collect(),
                )),
        )]);
        let hardforks = DEV_HARDFORKS.clone();
        let chain_spec: std::sync::Arc<ChainSpec> = std::sync::Arc::new(ChainSpec {
            chain: Chain::from(1337u64),
            genesis_header: reth_primitives_traits::SealedHeader::new_unhashed(
                make_genesis_header(&genesis, &hardforks),
            ),
            genesis,
            paris_block_and_final_difficulty: Some((0, U256::ZERO)),
            hardforks,
            base_fee_params: BaseFeeParamsKind::Constant(BaseFeeParams::ethereum()),
            deposit_contract: None,
            ..Default::default()
        });

        let factory = create_test_provider_factory_with_chain_spec(chain_spec.clone());
        let genesis_hash = init_genesis(&factory).expect("init genesis");
        let provider =
            reth_provider::providers::BlockchainProvider::new(factory).expect("provider");

        // The parent header is fabricated at a height the DB does not hold: the
        // pre-execution section reads only the STATE (taken at genesis, where
        // the stub lives) and `block.number`, so lifting the height is the
        // cheapest way to get a nonzero epoch without chaining blocks.
        let mut parent = chain_spec.genesis_header().clone();
        parent.number = STUB_PARENT_NUMBER;
        let parent = reth_primitives_traits::SealedHeader::new(parent, genesis_hash);

        let evm_config =
            FluentEvmConfig::new(chain_spec, FluentEvmFactory::default(), STUB_STAKING);
        (evm_config, provider, genesis_hash, parent)
    }

    /// Build the one block these tests execute: parent at
    /// [`STUB_PARENT_NUMBER`], no transactions, `extra_data` carrying the given
    /// production record. Returns the sealed block, the bundle the executor
    /// produced and the state root it committed to.
    fn build_stubbed_block(staking_code: Vec<u8>, accused: Option<u8>) -> StubbedBlock {
        use alloy_primitives::Bytes;
        use fluentbase_revm::revm::database::State;
        use reth_evm::{execute::BlockBuilder as _, ConfigureEvm as _, NextBlockEnvAttributes};
        use reth_revm::database::StateProviderDatabase;
        use reth_storage_api::StateProviderFactory as _;

        let (evm_config, provider, genesis_hash, parent) = stub_chain(staking_code);
        let state_provider = provider
            .state_by_block_hash(genesis_hash)
            .expect("genesis state");
        let mut db = State::builder()
            .with_database(StateProviderDatabase::new(state_provider.as_ref()))
            .with_bundle_update()
            .build();

        let attrs = NextBlockEnvAttributes {
            timestamp: parent.timestamp + 1,
            suggested_fee_recipient: alloy_primitives::Address::repeat_byte(0x77),
            prev_randao: alloy_primitives::B256::repeat_byte(0x42),
            gas_limit: parent.gas_limit,
            parent_beacon_block_root: Some(alloy_primitives::B256::ZERO),
            withdrawals: None,
            extra_data: Bytes::from(fluentbase_consensus::extra_data::encode_production_record(
                STUB_LEADER_INDEX,
                accused,
            )),
            slot_number: None,
        };

        let mut builder = evm_config
            .builder_for_next_block(&mut db, &parent, attrs)
            .expect("builder");
        builder
            .apply_pre_execution_changes()
            .expect("the pre-execution section must not fail the block");
        let outcome = builder.finish(&state_provider, None).expect("finish");
        let state_root = outcome.block.header().state_root;
        StubbedBlock {
            block: outcome.block,
            bundle: db.take_bundle(),
            state_root,
        }
    }

    /// Read the stub's receipt for a selector: `Some(first_argument_word)` when
    /// the call was issued AND its state committed, `None` when it never
    /// arrived. Only committing calls leave a trace — the three view reads go
    /// through frames whose state the executor discards by design, so their
    /// receipts (and their seeded answer slots) never reach the bundle.
    fn stub_receipt(
        bundle: &fluentbase_revm::revm::database::BundleState,
        selector: [u8; 4],
    ) -> Option<alloy_primitives::U256> {
        use alloy_primitives::U256;
        let slot = U256::from(u32::from_be_bytes(selector));
        let account = bundle.state.get(&STUB_STAKING)?;
        let called = account.storage.get(&slot)?;
        if called.present_value.is_zero() {
            return None;
        }
        Some(
            account
                .storage
                .get(&(slot + U256::from(1u64)))
                .map(|s| s.present_value)
                .unwrap_or_default(),
        )
    }

    /// The verdict half of the header's production record drives the
    /// `slashEquivocation` system call, and the epoch it carries is the BLOCK'S
    /// epoch — not its height. Absent a verdict the call is never issued, which
    /// matters because `NO_CHARGE` decodes to `None` on every ordinary block.
    ///
    /// What this cannot assert is the victim actually being tombstoned: that is
    /// the contract's half, it lives in a separate repository as rWasm, and it
    /// is pinned by that crate's own tests. The same goes for a duplicate
    /// verdict being a no-op — the idempotency is a contract property, and the
    /// node side of it is the soft fold covered below.
    #[test]
    fn a_verdict_in_extra_data_issues_the_slash_system_call() {
        use alloy_primitives::U256;
        use alloy_sol_types::SolCall;

        let charged = build_stubbed_block(stub::recording(), Some(7));
        let epoch = stub_receipt(&charged.bundle, super::slashEquivocationCall::SELECTOR)
            .expect("a block carrying a verdict must issue the slash system call");
        // interval == activation == 1, so epoch = number − 1. Distinct from the
        // block height, which is what proves the epoch argument is the epoch.
        assert_eq!(charged.block.header().number, STUB_PARENT_NUMBER + 1);
        assert_eq!(epoch, U256::from(STUB_PARENT_NUMBER));

        let clean = build_stubbed_block(stub::recording(), None);
        assert!(
            stub_receipt(&clean.bundle, super::slashEquivocationCall::SELECTOR).is_none(),
            "a block with no verdict must issue no slash system call"
        );
        // The verdict is an ADDITION to the section, not a branch around it:
        // both blocks still credited production, and with this block's leader.
        // The recorded word is the LEADER INDEX now — the height left the wire
        // when the contract started reading `block.number` itself.
        for bundle in [&charged.bundle, &clean.bundle] {
            assert_eq!(
                stub_receipt(bundle, super::recordProductionCall::SELECTOR),
                Some(U256::from(STUB_LEADER_INDEX))
            );
        }
    }

    /// The soft fold. A verdict the contract refuses must cost the chain
    /// nothing: the block still builds, the section still completes, and only
    /// the slash is lost. Fail-loud here — `recordProduction`'s model — would
    /// turn a raced or duplicate charge into a stalled chain.
    #[test]
    fn a_refused_verdict_is_skipped_rather_than_failing_the_block() {
        use alloy_primitives::U256;
        use alloy_sol_types::SolCall;
        // The block builds at all — `build_stubbed_block` unwraps
        // `apply_pre_execution_changes` — which is the whole property: a refused
        // verdict is not a failed payload.
        let refused = build_stubbed_block(stub::recording_rejecting_the_verdict(), Some(7));
        assert!(
            stub_receipt(&refused.bundle, super::slashEquivocationCall::SELECTOR).is_none(),
            "a reverted verdict must leave no state behind"
        );
        // And the section really did reach the verdict leg — the recorder ahead
        // of it committed — so the absence above is a fold, not a skipped block.
        assert_eq!(
            stub_receipt(&refused.bundle, super::recordProductionCall::SELECTOR),
            Some(U256::from(STUB_LEADER_INDEX))
        );
    }

    /// THE property the two-channel split exists for. A node syncing the EL from
    /// peers has no OrderBlock and therefore no evidence — it rebuilds its
    /// execution context from the header alone. Re-execute the block built above
    /// through that path and require the same state root: if the verdict leg
    /// read anything the header does not carry, the two roots diverge and the
    /// synced node rejects a block the committee finalized.
    #[test]
    fn re_execution_from_the_header_alone_reaches_the_same_state_root() {
        use alloy_sol_types::SolCall;
        use reth_evm::{execute::Executor as _, ConfigureEvm as _};
        use reth_revm::database::StateProviderDatabase;
        use reth_storage_api::{HashedPostStateProvider as _, StateProviderFactory as _};

        let built = build_stubbed_block(stub::recording(), Some(7));

        // A second, identical chain — the re-executing node's own — so nothing
        // the builder left in memory can carry over.
        let (evm_config, provider, genesis_hash, _parent) = stub_chain(stub::recording());
        let state_provider = provider
            .state_by_block_hash(genesis_hash)
            .expect("genesis state");

        // The EL-sync path: nothing but the block goes in — no OrderBlock, no
        // evidence, no attributes.
        let output = evm_config
            .executor(StateProviderDatabase::new(state_provider.as_ref()))
            .execute(&built.block)
            .expect("re-execution from the block alone");

        let hashed = state_provider.hashed_post_state(&output.state);
        let re_executed_root = state_provider.state_root(hashed).expect("state root");
        assert_eq!(
            re_executed_root, built.state_root,
            "a node re-executing from EVM data alone must reach the same state root"
        );
        // Belt: the root would also match if the verdict leg had run on NEITHER
        // path. Prove it ran on the re-execution path too.
        assert!(
            stub_receipt(&output.state, super::slashEquivocationCall::SELECTOR).is_some(),
            "the re-executing node must have issued the same slash system call"
        );
    }
}
