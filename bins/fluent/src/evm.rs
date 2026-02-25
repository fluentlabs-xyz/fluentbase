//! Ethereum EVM implementation.

use alloy_evm::{env::EvmEnv, evm::EvmFactory, precompiles::PrecompilesMap, Database, Evm};
use alloy_primitives::{Address, Bytes};
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
use reth_ethereum_primitives::EthPrimitives;
use reth_evm_ethereum::EthEvmConfig;
use reth_node_api::FullNodeTypes;
use reth_node_builder::{components::ExecutorBuilder, BuilderContext};
use reth_node_types::NodeTypes;

/// The Ethereum EVM context type.
pub type EthRwasmContext<DB> = Context<BlockEnv, TxEnv, CfgEnv, DB>;

/// Ethereum EVM implementation.
///
/// This is a wrapper type around the `revm` ethereum evm with optional [`Inspector`] (tracing)
/// support. [`Inspector`] support is configurable at runtime because it's part of the underlying
/// [`RevmEvm`] type.
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
    /// [`RevmEvm`] should be invoked on [`Evm::transact`].
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
#[derive(Debug, Default, Clone, Copy)]
#[non_exhaustive]
pub struct FluentExecutorBuilder;

impl<Node> ExecutorBuilder<Node> for FluentExecutorBuilder
where
    Node: FullNodeTypes<Types: NodeTypes<ChainSpec = ChainSpec, Primitives = EthPrimitives>>,
{
    type EVM = EthEvmConfig<ChainSpec, FluentEvmFactory>;

    async fn build_evm(self, ctx: &BuilderContext<Node>) -> eyre::Result<Self::EVM> {
        let evm_config =
            EthEvmConfig::new_with_evm_factory(ctx.chain_spec(), FluentEvmFactory::default());
        Ok(evm_config)
    }
}
