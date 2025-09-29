//! Implementation of the [`ExecuteEvm`] trait for the [`RwasmEvm`].
use crate::{api::RwasmFrame, evm::RwasmEvm, handler::RwasmHandler, RwasmHaltReason, RwasmSpecId};
use fluentbase_runtime::{default_runtime_executor, RuntimeExecutor};
use revm::{
    context::{
        result::{ExecResultAndState, InvalidTransaction},
        ContextSetters, Transaction,
    },
    context_interface::{
        result::{EVMError, ExecutionResult},
        Cfg, ContextTr, Database, JournalTr,
    },
    handler::{
        instructions::EthInstructions, system_call::SystemCallEvm, Handler, PrecompileProvider,
        SystemCallTx,
    },
    inspector::{InspectCommitEvm, InspectEvm, Inspector, InspectorHandler, JournalExt},
    interpreter::{interpreter::EthInterpreter, InterpreterResult},
    primitives::{Address, Bytes},
    state::EvmState,
    DatabaseCommit, ExecuteCommitEvm, ExecuteEvm,
};

/// Type alias for Optimism context
pub trait RwasmContextTr:
    ContextTr<
    Journal: JournalTr<State = EvmState>,
    Tx: Transaction,
    Cfg: Cfg<Spec = RwasmSpecId>,
    Chain = (),
>
{
}

impl<T> RwasmContextTr for T where
    T: ContextTr<
        Journal: JournalTr<State = EvmState>,
        Tx: Transaction,
        Cfg: Cfg<Spec = RwasmSpecId>,
        Chain = (),
    >
{
}

/// Type alias for the error type of the RwasmEvm.
pub type RwasmError<CTX> =
    EVMError<<<CTX as ContextTr>::Db as Database>::Error, InvalidTransaction>;

impl<CTX, INSP, PRECOMPILE> ExecuteEvm
    for RwasmEvm<CTX, INSP, EthInstructions<EthInterpreter, CTX>, PRECOMPILE>
where
    CTX: RwasmContextTr + ContextSetters,
    PRECOMPILE: PrecompileProvider<CTX, Output = InterpreterResult>,
{
    type Tx = <CTX as ContextTr>::Tx;
    type Block = <CTX as ContextTr>::Block;
    type State = EvmState;
    type Error = RwasmError<CTX>;
    type ExecutionResult = ExecutionResult<RwasmHaltReason>;

    fn set_block(&mut self, block: Self::Block) {
        self.0.ctx.set_block(block);
    }

    fn transact_one(&mut self, tx: Self::Tx) -> Result<Self::ExecutionResult, Self::Error> {
        default_runtime_executor().reset_call_id_counter();
        self.0.ctx.set_tx(tx);
        let mut h = RwasmHandler::<_, _, RwasmFrame>::default();
        h.run(self)
    }

    fn finalize(&mut self) -> Self::State {
        self.0.ctx.journal_mut().finalize()
    }

    fn replay(
        &mut self,
    ) -> Result<ExecResultAndState<Self::ExecutionResult, Self::State>, Self::Error> {
        default_runtime_executor().reset_call_id_counter();
        let mut h = RwasmHandler::<_, _, RwasmFrame>::default();
        h.run(self).map(|result| {
            let state = self.finalize();
            ExecResultAndState::new(result, state)
        })
    }
}

impl<CTX, INSP, PRECOMPILE> ExecuteCommitEvm
    for RwasmEvm<CTX, INSP, EthInstructions<EthInterpreter, CTX>, PRECOMPILE>
where
    CTX: RwasmContextTr<Db: DatabaseCommit> + ContextSetters,
    PRECOMPILE: PrecompileProvider<CTX, Output = InterpreterResult>,
{
    fn commit(&mut self, state: Self::State) {
        self.0.ctx.db_mut().commit(state);
    }
}

impl<CTX, INSP, PRECOMPILE> InspectEvm
    for RwasmEvm<CTX, INSP, EthInstructions<EthInterpreter, CTX>, PRECOMPILE>
where
    CTX: RwasmContextTr<Journal: JournalExt> + ContextSetters,
    INSP: Inspector<CTX, EthInterpreter>,
    PRECOMPILE: PrecompileProvider<CTX, Output = InterpreterResult>,
{
    type Inspector = INSP;

    fn set_inspector(&mut self, inspector: Self::Inspector) {
        self.0.inspector = inspector;
    }

    fn inspect_one_tx(&mut self, tx: Self::Tx) -> Result<Self::ExecutionResult, Self::Error> {
        default_runtime_executor().reset_call_id_counter();
        self.0.ctx.set_tx(tx);
        let mut h = RwasmHandler::<_, _, RwasmFrame>::default();
        h.inspect_run(self)
    }
}

impl<CTX, INSP, PRECOMPILE> InspectCommitEvm
    for RwasmEvm<CTX, INSP, EthInstructions<EthInterpreter, CTX>, PRECOMPILE>
where
    CTX: RwasmContextTr<Journal: JournalExt, Db: DatabaseCommit> + ContextSetters,
    INSP: Inspector<CTX, EthInterpreter>,
    PRECOMPILE: PrecompileProvider<CTX, Output = InterpreterResult>,
{
}

impl<CTX, INSP, PRECOMPILE> SystemCallEvm
    for RwasmEvm<CTX, INSP, EthInstructions<EthInterpreter, CTX>, PRECOMPILE>
where
    CTX: RwasmContextTr<Tx: SystemCallTx> + ContextSetters,
    PRECOMPILE: PrecompileProvider<CTX, Output = InterpreterResult>,
{
    fn transact_system_call_with_caller(
        &mut self,
        caller: Address,
        system_contract_address: Address,
        data: Bytes,
    ) -> Result<Self::ExecutionResult, Self::Error> {
        default_runtime_executor().reset_call_id_counter();
        self.0.ctx.set_tx(CTX::Tx::new_system_tx_with_caller(
            caller,
            system_contract_address,
            data,
        ));
        let mut h = RwasmHandler::<_, _, RwasmFrame>::default();
        h.run_system_call(self)
    }
}
