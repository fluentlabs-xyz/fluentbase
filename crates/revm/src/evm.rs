//! Contains the `[RwasmEvm]` type and its implementation of the execution EVM traits.

use crate::{
    api::RwasmFrame, executor::run_rwasm_loop, precompiles::RwasmPrecompiles,
    types::SystemInterruptionOutcome, upgrade::upgrade_runtime_hook,
};
use fluentbase_sdk::{
    resolve_precompiled_runtime_from_input, try_resolve_precompile_account_from_input, Address,
    Bytes, UPDATE_GENESIS_AUTH, UPDATE_GENESIS_PREFIX,
};
use revm::{
    bytecode::{ownable_account::OwnableAccountBytecode, Bytecode},
    context::{ContextError, ContextSetters, Evm, FrameStack, JournalTr},
    context_interface::ContextTr,
    handler::{
        evm::{ContextDbError, FrameInitResult, FrameTr},
        instructions::{EthInstructions, InstructionProvider},
        EvmTr, FrameInitOrResult, FrameResult, ItemOrResult, PrecompileProvider,
    },
    inspector::{
        handler::{frame_end, frame_start},
        inspect_instructions, InspectorEvmTr, JournalExt,
    },
    interpreter::{
        interpreter::{EthInterpreter, ExtBytecode},
        return_ok, return_revert, CallInput, FrameInput, InstructionResult, InterpreterResult,
    },
    Database, Inspector,
};

/// Rwasm EVM extends the [`Evm`] type with Rwasm specific types and logic.
#[derive(Debug, Clone)]
pub struct RwasmEvm<
    CTX,
    INSP = (),
    I = EthInstructions<EthInterpreter, CTX>,
    P = RwasmPrecompiles,
    F = RwasmFrame,
>(
    /// Inner EVM type.
    pub Evm<CTX, INSP, I, P, F>,
);

impl<CTX: ContextTr, INSP>
    RwasmEvm<CTX, INSP, EthInstructions<EthInterpreter, CTX>, RwasmPrecompiles>
{
    /// Create a new Rwasm EVM.
    pub fn new(ctx: CTX, inspector: INSP) -> Self {
        Self(Evm {
            ctx,
            inspector,
            instruction: EthInstructions::new_mainnet(),
            precompiles: RwasmPrecompiles::default(),
            frame_stack: FrameStack::new(),
        })
    }
}

impl<CTX, INSP, I, P> RwasmEvm<CTX, INSP, I, P> {
    /// Consumed self and returns a new Evm type with the given Inspector.
    pub fn with_inspector<OINSP>(self, inspector: OINSP) -> RwasmEvm<CTX, OINSP, I, P> {
        RwasmEvm(self.0.with_inspector(inspector))
    }

    /// Consumes self and returns a new Evm type with given Precompiles.
    pub fn with_precompiles<OP>(self, precompiles: OP) -> RwasmEvm<CTX, INSP, I, OP> {
        RwasmEvm(self.0.with_precompiles(precompiles))
    }

    /// Consumes self and returns the inner Inspector.
    pub fn into_inspector(self) -> INSP {
        self.0.into_inspector()
    }
}

impl<CTX, INSP, I, P> InspectorEvmTr for RwasmEvm<CTX, INSP, I, P>
where
    CTX: ContextTr<Journal: JournalExt> + ContextSetters,
    I: InstructionProvider<Context = CTX, InterpreterTypes = EthInterpreter>,
    P: PrecompileProvider<CTX, Output = InterpreterResult>,
    INSP: Inspector<CTX, I::InterpreterTypes>,
{
    type Inspector = INSP;

    fn inspector(&mut self) -> &mut Self::Inspector {
        &mut self.0.inspector
    }

    fn ctx_inspector(&mut self) -> (&mut Self::Context, &mut Self::Inspector) {
        (&mut self.0.ctx, &mut self.0.inspector)
    }

    fn ctx_inspector_frame(
        &mut self,
    ) -> (&mut Self::Context, &mut Self::Inspector, &mut Self::Frame) {
        (
            &mut self.0.ctx,
            &mut self.0.inspector,
            self.0.frame_stack.get(),
        )
    }

    fn ctx_inspector_frame_instructions(
        &mut self,
    ) -> (
        &mut Self::Context,
        &mut Self::Inspector,
        &mut Self::Frame,
        &mut Self::Instructions,
    ) {
        (
            &mut self.0.ctx,
            &mut self.0.inspector,
            self.0.frame_stack.get(),
            &mut self.0.instruction,
        )
    }

    #[inline]
    fn inspect_frame_init(
        &mut self,
        mut frame_init: <Self::Frame as FrameTr>::FrameInit,
    ) -> Result<FrameInitResult<'_, Self::Frame>, ContextDbError<Self::Context>> {
        let (ctx, inspector) = self.ctx_inspector();
        if let Some(mut output) = frame_start(ctx, inspector, &mut frame_init.frame_input) {
            frame_end(ctx, inspector, &frame_init.frame_input, &mut output);
            return Ok(ItemOrResult::Result(output));
        }

        let frame_input = frame_init.frame_input.clone();
        if let ItemOrResult::Result(mut output) = self.frame_init(frame_init)? {
            let (ctx, inspector) = self.ctx_inspector();
            frame_end(ctx, inspector, &frame_input, &mut output);
            return Ok(ItemOrResult::Result(output));
        }

        // if it is new frame, initialize the interpreter.
        let (ctx, inspector, frame) = self.ctx_inspector_frame();
        let interp = &mut frame.interpreter;
        inspector.initialize_interp(interp, ctx);
        Ok(ItemOrResult::Item(frame))
    }

    #[inline]
    fn inspect_frame_run(
        &mut self,
    ) -> Result<FrameInitOrResult<Self::Frame>, ContextDbError<Self::Context>> {
        let (context, _inspector, frame, _) = self.ctx_inspector_frame_instructions();

        // TODO(dmitry123): "add support of inspector for EVM-compatible syscalls"
        let action = run_rwasm_loop(frame, context)?.into_interpreter_action();
        let mut result = frame.process_next_action(context, action);

        if let Ok(ItemOrResult::Result(frame_result)) = &mut result {
            let (ctx, inspector, frame) = self.ctx_inspector_frame();
            frame_end(ctx, inspector, &frame.input, frame_result);
            frame.set_finished(true);
        };
        result
    }
}

impl<CTX, INSP, I, P> EvmTr for RwasmEvm<CTX, INSP, I, P, RwasmFrame>
where
    CTX: ContextTr,
    I: InstructionProvider<Context = CTX, InterpreterTypes = EthInterpreter>,
    P: PrecompileProvider<CTX, Output = InterpreterResult>,
{
    type Context = CTX;
    type Instructions = I;
    type Precompiles = P;
    type Frame = RwasmFrame;

    fn ctx(&mut self) -> &mut Self::Context {
        &mut self.0.ctx
    }

    fn ctx_ref(&self) -> &Self::Context {
        &self.0.ctx
    }

    fn ctx_instructions(&mut self) -> (&mut Self::Context, &mut Self::Instructions) {
        (&mut self.0.ctx, &mut self.0.instruction)
    }

    fn ctx_precompiles(&mut self) -> (&mut Self::Context, &mut Self::Precompiles) {
        (&mut self.0.ctx, &mut self.0.precompiles)
    }

    fn frame_stack(&mut self) -> &mut FrameStack<Self::Frame> {
        &mut self.0.frame_stack
    }

    fn frame_init(
        &mut self,
        frame_input: <Self::Frame as FrameTr>::FrameInit,
    ) -> Result<
        ItemOrResult<&mut Self::Frame, <Self::Frame as FrameTr>::FrameResult>,
        ContextError<<<Self::Context as ContextTr>::Db as Database>::Error>,
    > {
        let is_first_init = self.0.frame_stack.index().is_none();
        let new_frame = if is_first_init {
            self.0.frame_stack.start_init()
        } else {
            self.0.frame_stack.get_next()
        };
        let ctx = &mut self.0.ctx;
        let precompiles = &mut self.0.precompiles;
        let res = Self::Frame::init_with_context(new_frame, ctx, precompiles, frame_input)?;
        let mut res = res.map_frame(|token| {
            if is_first_init {
                self.0.frame_stack.end_init(token);
            } else {
                self.0.frame_stack.push(token);
            }
            self.0.frame_stack.get()
        });
        match &mut res {
            ItemOrResult::Item(new_frame) => {
                match &mut new_frame.input {
                    FrameInput::Call(inputs) => {
                        // a special hook for runtime upgrade
                        // that is used only for testnet to upgrade genesis without forks
                        if inputs.caller == UPDATE_GENESIS_AUTH
                            && inputs.input.bytes(ctx).starts_with(&UPDATE_GENESIS_PREFIX)
                        {
                            return upgrade_runtime_hook(ctx, inputs);
                        }
                        // TODO(dmitry123): "do we want to disable it for mainnet?"
                        if let Some(precompiled_address) = try_resolve_precompile_account_from_input(
                            inputs.input.bytes(ctx).as_ref(),
                        ) {
                            let account =
                                &ctx.journal_mut().load_account_code(precompiled_address)?;
                            // rewrite bytecode address
                            inputs.bytecode_address = precompiled_address;
                            // rewrite bytecode with code hash
                            new_frame.interpreter.bytecode = ExtBytecode::new_with_hash(
                                account.info.code.clone().unwrap_or_default(),
                                account.info.code_hash,
                            );
                        }
                    }
                    FrameInput::Create(inputs) => {
                        let precompile_runtime =
                            resolve_precompiled_runtime_from_input(inputs.init_code.as_ref());
                        // create a new EIP-7702 account that points to the EVM runtime system precompile
                        let ownable_account_bytecode =
                            OwnableAccountBytecode::new(precompile_runtime, Bytes::new());
                        new_frame.interpreter.input.account_owner = Some(precompile_runtime);
                        let bytecode = Bytecode::OwnableAccount(ownable_account_bytecode);
                        ctx.journal_mut()
                            .set_code(new_frame.interpreter.input.target_address, bytecode);
                        // an original init code we pass as an input inside the runtime
                        // to execute deployment logic
                        let input_bytecode = inputs.init_code.clone();
                        new_frame.interpreter.input.input = CallInput::Bytes(input_bytecode);
                        // we should reload bytecode here since it's an EIP-7702 account
                        let bytecode = ctx.journal_mut().code(precompile_runtime)?;
                        assert!(
                            !bytecode.data.is_empty(),
                            "precompile bytecode is empty, missing account"
                        );
                        // if it's a CREATE or CREATE2 call, then we should
                        // to recalculate init code hash to make sure it matches runtime hash
                        let code_hash = ctx.journal_mut().code_hash(precompile_runtime)?;
                        // write new fields into input
                        new_frame.interpreter.bytecode = ExtBytecode::new_with_hash(
                            Bytecode::new_raw(bytecode.data),
                            code_hash.data,
                        );
                    }
                    _ => unreachable!(),
                }
            }
            _ => {}
        }
        Ok(res)
    }

    fn frame_run(
        &mut self,
    ) -> Result<
        FrameInitOrResult<Self::Frame>,
        ContextError<<<Self::Context as ContextTr>::Db as Database>::Error>,
    > {
        let frame = self.0.frame_stack.get();
        let context = &mut self.0.ctx;
        let action = run_rwasm_loop(frame, context)?.into_interpreter_action();
        frame.process_next_action(context, action).inspect(|i| {
            if i.is_result() {
                frame.set_finished(true);
            }
        })
    }

    fn frame_return_result(
        &mut self,
        result: <Self::Frame as FrameTr>::FrameResult,
    ) -> Result<
        Option<<Self::Frame as FrameTr>::FrameResult>,
        ContextError<<<Self::Context as ContextTr>::Db as Database>::Error>,
    > {
        if self.0.frame_stack.get().is_finished() {
            self.0.frame_stack.pop();
        }
        if self.0.frame_stack.index().is_none() {
            return Ok(Some(result));
        }

        let frame = self.0.frame_stack.get();
        // TODO(dmitry123): "it seems we can't eliminate interpreter (revm is not ready for this yet)"
        frame.interpreter.memory.free_child_context();
        // P.S: we can skip frame's error check here because we don't use Host

        // if call is interrupted then we need to remember the interrupted state;
        // the execution can be continued
        // since the state is updated already
        Self::insert_interrupted_result(frame.interrupted_outcome.as_mut().unwrap(), result);
        Ok(None)
    }
}

impl<CTX, INSP, I, P> RwasmEvm<CTX, INSP, I, P> {
    ///
    pub fn insert_interrupted_result(
        interrupted_outcome: &mut SystemInterruptionOutcome,
        result: FrameResult,
    ) {
        let created_address = if let FrameResult::Create(create_outcome) = &result {
            create_outcome.address.or_else(|| {
                // I don't know why EVM returns empty address and ok status in case of nonce
                // overflow, I think nobody knows...
                let is_nonce_overflow = create_outcome.result.result == InstructionResult::Return
                    && create_outcome.address.is_none();
                if is_nonce_overflow {
                    Some(Address::ZERO)
                } else {
                    None
                }
            })
        } else {
            None
        };
        let mut result = result.into_interpreter_result();
        // for the frame result we take gas from the result field
        // because it stores information about gas consumed before the call as well
        let mut gas = interrupted_outcome.remaining_gas;
        match result.result {
            return_ok!() => {
                let remaining = result.gas.remaining();
                gas.erase_cost(remaining);
                let refunded = result.gas.refunded();
                gas.record_refund(refunded);
                // for CREATE/CREATE2 calls, we need to write the created address into output
                if let Some(created_address) = created_address {
                    result.output = created_address.into_array().into();
                }
            }
            return_revert!() => {
                gas.erase_cost(result.gas.remaining());
            }
            InstructionResult::FatalExternalError => {
                panic!("revm: fatal external error");
            }
            _ => {}
        }
        // we can rewrite here gas since it's adjusted with the consumed value
        result.gas = gas;
        interrupted_outcome.result = Some(result);
    }
}
