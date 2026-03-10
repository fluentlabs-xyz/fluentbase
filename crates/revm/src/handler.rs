//!Handler related to a Fluent chain

use crate::{
    bridge::{apply_bridge_post_invocation_hook, apply_bridge_pre_invocation_hook},
    types::SystemInterruptionOutcome,
    RwasmFrame, RwasmHaltReason,
};
use revm::{
    context::{result::ExecutionResult, ContextTr, JournalTr},
    handler::{EvmTr, EvmTrError, Handler},
    inspector::{InspectorEvmTr, InspectorHandler},
    interpreter::interpreter::EthInterpreter,
    state::EvmState,
    Inspector,
};

/// Rwasm handler that implements the default [`Handler`] trait for the Evm.
#[derive(Debug, Clone)]
pub struct RwasmHandler<CTX, ERROR> {
    /// Phantom data to hold the generic type parameters.
    pub _phantom: core::marker::PhantomData<(CTX, ERROR)>,
}

impl<EVM, ERROR> Handler for RwasmHandler<EVM, ERROR>
where
    EVM: EvmTr<Context: ContextTr<Journal: JournalTr<State = EvmState>>, Frame = RwasmFrame>,
    ERROR: EvmTrError<EVM>,
{
    type Evm = EVM;
    type Error = ERROR;
    type HaltReason = RwasmHaltReason;

    #[inline]
    fn run_without_catch_error(
        &mut self,
        evm: &mut Self::Evm,
    ) -> Result<ExecutionResult<Self::HaltReason>, Self::Error> {
        let init_and_floor_gas = self.validate(evm)?;
        let eip7702_refund = self.pre_execution(evm)? as i64;

        // Apply fluent bridge hook that mints/burns native tokens
        apply_bridge_pre_invocation_hook::<EVM, ERROR>(evm)?;

        let mut exec_result = self.execution(evm, &init_and_floor_gas)?;
        self.post_execution(evm, &mut exec_result, init_and_floor_gas, eip7702_refund)?;

        // Apply fluent bridge hook that mints/burns native tokens
        apply_bridge_post_invocation_hook::<EVM, ERROR>(evm, &exec_result)?;

        // Prepare the output
        self.execution_result(evm, exec_result)
    }
}

impl<CTX, ERROR> Default for RwasmHandler<CTX, ERROR> {
    fn default() -> Self {
        Self {
            _phantom: core::marker::PhantomData,
        }
    }
}

impl<EVM, ERROR> InspectorHandler<SystemInterruptionOutcome> for RwasmHandler<EVM, ERROR>
where
    EVM: InspectorEvmTr<
        SystemInterruptionOutcome,
        Context: ContextTr<Journal: JournalTr<State = EvmState>>,
        Frame = RwasmFrame,
        Inspector: Inspector<<<Self as Handler>::Evm as EvmTr>::Context, EthInterpreter>,
    >,
    ERROR: EvmTrError<EVM>,
{
    type IT = EthInterpreter;

    fn inspect_run_without_catch_error(
        &mut self,
        evm: &mut Self::Evm,
    ) -> Result<ExecutionResult<Self::HaltReason>, Self::Error> {
        let init_and_floor_gas = self.validate(evm)?;
        let eip7702_refund = self.pre_execution(evm)? as i64;

        // Apply fluent bridge hook that mints/burns native tokens
        apply_bridge_pre_invocation_hook::<EVM, ERROR>(evm)?;

        let mut frame_result = self.inspect_execution(evm, &init_and_floor_gas)?;
        self.post_execution(evm, &mut frame_result, init_and_floor_gas, eip7702_refund)?;

        // Apply fluent bridge hook that mints/burns native tokens
        apply_bridge_post_invocation_hook::<EVM, ERROR>(evm, &frame_result)?;

        self.execution_result(evm, frame_result)
    }
}
