//!Handler related to a Fluent chain

use crate::{types::SystemInterruptionOutcome, RwasmFrame, RwasmHaltReason};
use revm::{
    context::{ContextTr, JournalTr},
    handler::{EvmTr, EvmTrError, FrameResult, FrameTr, Handler},
    inspector::{InspectorEvmTr, InspectorHandler},
    interpreter::{interpreter::EthInterpreter, interpreter_action::FrameInit},
    state::EvmState,
    Inspector,
};

/// Rwasm handler that implements the default [`Handler`] trait for the Evm.
#[derive(Debug, Clone)]
pub struct RwasmHandler<CTX, ERROR, FRAME> {
    /// Phantom data to hold the generic type parameters.
    pub _phantom: core::marker::PhantomData<(CTX, ERROR, FRAME)>,
}

impl<EVM, ERROR, FRAME> Handler for RwasmHandler<EVM, ERROR, FRAME>
where
    EVM: EvmTr<Context: ContextTr<Journal: JournalTr<State = EvmState>>, Frame = FRAME>,
    ERROR: EvmTrError<EVM>,
    FRAME: FrameTr<FrameResult = FrameResult, FrameInit = FrameInit>,
{
    type Evm = EVM;
    type Error = ERROR;
    type HaltReason = RwasmHaltReason;
}

impl<CTX, ERROR, FRAME> Default for RwasmHandler<CTX, ERROR, FRAME> {
    fn default() -> Self {
        Self {
            _phantom: core::marker::PhantomData,
        }
    }
}

impl<EVM, ERROR> InspectorHandler<SystemInterruptionOutcome>
    for RwasmHandler<EVM, ERROR, RwasmFrame>
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
}
