use crate::types::{CallInputs, CallOutcome, CreateOutcome, InterpreterResult, SharedMemory};
use crate::{
    handler::mainnet,
    primitives::{EVMError, Spec},
    CallFrame, Context, CreateFrame, Frame, FrameOrResult, FrameResult,
};
use fluentbase_types::{ExitCode, IJournaledTrie};
use std::{boxed::Box, sync::Arc};

/// Handles first frame return handle.
pub type LastFrameReturnHandle<'a, EXT, DB> =
    Arc<dyn Fn(&mut Context<EXT, DB>, &mut FrameResult) -> Result<(), EVMError<ExitCode>> + 'a>;

/// Handle sub call.
pub type FrameCallHandle<'a, EXT, DB> = Arc<
    dyn Fn(&mut Context<EXT, DB>, Box<CallInputs>) -> Result<FrameOrResult, EVMError<ExitCode>>
        + 'a,
>;

/// Handle call return
pub type FrameCallReturnHandle<'a, EXT, DB> = Arc<
    dyn Fn(
            &mut Context<EXT, DB>,
            Box<CallFrame>,
            InterpreterResult,
        ) -> Result<CallOutcome, EVMError<ExitCode>>
        + 'a,
>;

/// Insert call outcome to the parent
pub type InsertCallOutcomeHandle<'a, EXT, DB> = Arc<
    dyn Fn(
            &mut Context<EXT, DB>,
            &mut Frame,
            &mut SharedMemory,
            CallOutcome,
        ) -> Result<(), EVMError<ExitCode>>
        + 'a,
>;

/// Handle create return
pub type FrameCreateReturnHandle<'a, EXT, DB> = Arc<
    dyn Fn(
            &mut Context<EXT, DB>,
            Box<CreateFrame>,
            InterpreterResult,
        ) -> Result<CreateOutcome, EVMError<ExitCode>>
        + 'a,
>;

/// Insert call outcome to the parent
pub type InsertCreateOutcomeHandle<'a, EXT, DB> = Arc<
    dyn Fn(&mut Context<EXT, DB>, &mut Frame, CreateOutcome) -> Result<(), EVMError<ExitCode>> + 'a,
>;

/// Handles related to stack frames.
pub struct ExecutionHandler<'a, EXT, DB: IJournaledTrie> {
    /// Handles last frame return, modified gas for refund and
    /// sets tx gas limit.
    pub last_frame_return: LastFrameReturnHandle<'a, EXT, DB>,
    /// Frame call
    pub call: FrameCallHandle<'a, EXT, DB>,
    /// Call return
    pub call_return: FrameCallReturnHandle<'a, EXT, DB>,
    /// Crate return
    pub create_return: FrameCreateReturnHandle<'a, EXT, DB>,
}

impl<'a, EXT: 'a, DB: IJournaledTrie + 'a> ExecutionHandler<'a, EXT, DB> {
    /// Creates mainnet ExecutionHandler.
    pub fn new<SPEC: Spec + 'a>() -> Self {
        Self {
            last_frame_return: Arc::new(mainnet::last_frame_return::<SPEC, EXT, DB>),
            call: Arc::new(mainnet::call::<SPEC, EXT, DB>),
            call_return: Arc::new(mainnet::call_return::<EXT, DB>),
            create_return: Arc::new(mainnet::create_return::<SPEC, EXT, DB>),
        }
    }
}

impl<'a, EXT, DB: IJournaledTrie> ExecutionHandler<'a, EXT, DB> {
    /// Handle call return, depending on instruction result gas will be reimbursed or not.
    #[inline]
    pub fn last_frame_return(
        &self,
        context: &mut Context<EXT, DB>,
        frame_result: &mut FrameResult,
    ) -> Result<(), EVMError<ExitCode>> {
        (self.last_frame_return)(context, frame_result)
    }

    /// Call frame call handler.
    #[inline]
    pub fn call(
        &self,
        context: &mut Context<EXT, DB>,
        inputs: Box<CallInputs>,
    ) -> Result<FrameOrResult, EVMError<ExitCode>> {
        (self.call)(context, inputs.clone())
    }

    /// Call registered handler for call return.
    #[inline]
    pub fn call_return(
        &self,
        context: &mut Context<EXT, DB>,
        frame: Box<CallFrame>,
        interpreter_result: InterpreterResult,
    ) -> Result<CallOutcome, EVMError<ExitCode>> {
        (self.call_return)(context, frame, interpreter_result)
    }

    /// Call handler for create return.
    #[inline]
    pub fn create_return(
        &self,
        context: &mut Context<EXT, DB>,
        frame: Box<CreateFrame>,
        interpreter_result: InterpreterResult,
    ) -> Result<CreateOutcome, EVMError<ExitCode>> {
        (self.create_return)(context, frame, interpreter_result)
    }
}
