//! Contains the `[RwasmEvm]` type and its implementation of the execution EVM traits.

use crate::{
    api::RwasmFrame, executor::run_rwasm_loop, precompiles::RwasmPrecompiles,
    types::SystemInterruptionOutcome, ExecutionResult,
};
use fluentbase_sdk::{resolve_precompiled_runtime_from_input, Address, Bytes};
use revm::{
    bytecode::{ownable_account::OwnableAccountBytecode, Bytecode},
    context::{ContextError, ContextSetters, Evm, FrameStack, JournalTr},
    context_interface::ContextTr,
    handler::{
        evm::FrameTr,
        instructions::{EthInstructions, InstructionProvider},
        EvmTr, FrameInitOrResult, FrameResult, ItemOrResult, PrecompileProvider,
    },
    inspector::{InspectorEvmTr, JournalExt, NoOpInspector},
    interpreter::{
        interpreter::{EthInterpreter, ExtBytecode},
        return_ok, return_revert, CallInput, FrameInput, Gas, InstructionResult, InterpreterResult,
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

impl<CTX, INSP, I, P> InspectorEvmTr<SystemInterruptionOutcome> for RwasmEvm<CTX, INSP, I, P>
where
    CTX: ContextTr<Journal: JournalExt> + ContextSetters,
    I: InstructionProvider<Context = CTX, InterpreterTypes = EthInterpreter>,
    P: PrecompileProvider<CTX, Output = InterpreterResult>,
    INSP: Inspector<CTX, I::InterpreterTypes>,
{
    type Inspector = INSP;

    fn all_inspector(
        &self,
    ) -> (
        &Self::Context,
        &Self::Instructions,
        &Self::Precompiles,
        &FrameStack<Self::Frame>,
        &Self::Inspector,
    ) {
        let ctx = &self.0.ctx;
        let frame = &self.0.frame_stack;
        let instructions = &self.0.instruction;
        let precompiles = &self.0.precompiles;
        let inspector = &self.0.inspector;
        (ctx, instructions, precompiles, frame, inspector)
    }
    fn all_mut_inspector(
        &mut self,
    ) -> (
        &mut Self::Context,
        &mut Self::Instructions,
        &mut Self::Precompiles,
        &mut FrameStack<Self::Frame>,
        &mut Self::Inspector,
    ) {
        let ctx = &mut self.0.ctx;
        let frame = &mut self.0.frame_stack;
        let instructions = &mut self.0.instruction;
        let precompiles = &mut self.0.precompiles;
        let inspector = &mut self.0.inspector;
        (ctx, instructions, precompiles, frame, inspector)
    }

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

    #[inline]
    fn all(
        &self,
    ) -> (
        &Self::Context,
        &Self::Instructions,
        &Self::Precompiles,
        &FrameStack<Self::Frame>,
    ) {
        let ctx = &self.0.ctx;
        let instructions = &self.0.instruction;
        let precompiles = &self.0.precompiles;
        let frame_stack = &self.0.frame_stack;
        (ctx, instructions, precompiles, frame_stack)
    }

    #[inline]
    fn all_mut(
        &mut self,
    ) -> (
        &mut Self::Context,
        &mut Self::Instructions,
        &mut Self::Precompiles,
        &mut FrameStack<Self::Frame>,
    ) {
        let ctx = &mut self.0.ctx;
        let instructions = &mut self.0.instruction;
        let precompiles = &mut self.0.precompiles;
        let frame_stack = &mut self.0.frame_stack;
        (ctx, instructions, precompiles, frame_stack)
    }

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

    #[tracing::instrument(level = "info", skip_all)]
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
                unsafe { self.0.frame_stack.end_init(token) };
            } else {
                unsafe { self.0.frame_stack.push(token) };
            }
            self.0.frame_stack.get()
        });
        match &mut res {
            ItemOrResult::Item(new_frame) => {
                match &mut new_frame.input {
                    #[allow(unused_variables)]
                    FrameInput::Call(inputs) => {
                        let _span = tracing::info_span!("revm.frame_init.call_hook").entered();

                        // ============================================================================
                        // SECURITY: CALLDATA-BASED PRECOMPILE DISPATCH VULNERABILITY
                        // ============================================================================
                        //
                        // The following code is DISABLED for mainnet and restricted to testnet only.
                        //
                        // VULNERABILITY DESCRIPTION:
                        // 1. UPDATE_GENESIS_AUTH: Privileged address that can deploy arbitrary bytecode
                        //    to any address via upgrade_runtime_hook_v1/v2
                        // 2. Calldata-based dispatch: Precompiles invoked by calldata prefix instead of
                        //    destination address (via try_resolve_precompile_account_from_input)
                        //
                        // SECURITY IMPACT:
                        // - If UPDATE_GENESIS_AUTH key is compromised, attacker gains full system control
                        // - Calldata-based dispatch violates Ethereum standard (EIP-1352)
                        // - Any transaction with specific byte prefix unexpectedly triggers precompiles
                        // - Breaks tooling/scripts expecting standard address-based precompile behavior
                        //
                        // AUDITOR RECOMMENDATION:
                        // Remove functionality for mainnet deployment. Use standard address-based
                        // precompile dispatch as specified in EIP-1352.
                        //
                        // CURRENT MITIGATION:
                        // - Restricted to testnet via 'fluent-testnet' feature flag
                        // - Multicall tests temporarily disabled (see e2e/src/multicall.rs)
                        //
                        // TODO: Implement proper EIP-1352 compliant precompile system:
                        //  1. Assign fixed addresses for precompiles (e.g., 0x0000...0100)
                        //  2. Dispatch based on target address, not calldata
                        //  3. Remove try_resolve_precompile_account_from_input entirely
                        //  4. Update all affected tests
                        //
                        // ============================================================================
                        #[cfg(feature = "fluent-testnet")]
                        {
                            use crate::upgrade::{
                                upgrade_runtime_hook_v1, upgrade_runtime_hook_v2,
                            };
                            use fluentbase_sdk::{
                                try_resolve_precompile_account_from_input, UPDATE_GENESIS_AUTH,
                                UPDATE_GENESIS_PREFIX_V1, UPDATE_GENESIS_PREFIX_V2,
                            };
                            // a special hook for runtime upgrade
                            // that is used only for testnet to upgrade genesis without forks
                            if inputs.caller == UPDATE_GENESIS_AUTH {
                                let input = inputs.input.bytes(ctx);
                                if input.starts_with(&UPDATE_GENESIS_PREFIX_V1) {
                                    return upgrade_runtime_hook_v1(ctx, inputs);
                                } else if input.starts_with(&UPDATE_GENESIS_PREFIX_V2) {
                                    return upgrade_runtime_hook_v2(ctx, inputs);
                                }
                            }
                            // calldata-based precompile dispatch for testnet only
                            if let Some(precompiled_address) =
                                try_resolve_precompile_account_from_input(
                                    inputs.input.bytes(ctx).as_ref(),
                                )
                            {
                                let account = &ctx
                                    .journal_mut()
                                    .load_account_with_code(precompiled_address)?;
                                // rewrite bytecode address
                                inputs.bytecode_address = precompiled_address;
                                // rewrite bytecode with code hash
                                new_frame.interpreter.bytecode = ExtBytecode::new_with_hash(
                                    account.info.code.clone().unwrap_or_default(),
                                    account.info.code_hash,
                                );
                            }
                        }
                    }
                    FrameInput::Create(inputs) => {
                        let _span = tracing::info_span!("revm.frame_init.create_hook").entered();
                        let precompile_runtime =
                            resolve_precompiled_runtime_from_input(inputs.init_code().as_ref());
                        // create a new EIP-7702 account that points to the EVM runtime system precompile
                        let ownable_account_bytecode =
                            OwnableAccountBytecode::new(precompile_runtime, Bytes::new());
                        new_frame.interpreter.input.account_owner = Some(precompile_runtime);
                        let bytecode = Bytecode::OwnableAccount(ownable_account_bytecode);
                        ctx.journal_mut()
                            .set_code(new_frame.interpreter.input.target_address, bytecode);
                        // an original init code we pass as an input inside the runtime
                        // to execute deployment logic
                        let input_bytecode = inputs.init_code().clone();
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

    #[tracing::instrument(level = "info", skip_all)]
    fn frame_run(
        &mut self,
    ) -> Result<
        FrameInitOrResult<Self::Frame>,
        ContextError<<<Self::Context as ContextTr>::Db as Database>::Error>,
    > {
        let frame = self.0.frame_stack.get();
        let context = &mut self.0.ctx;
        let action = run_rwasm_loop::<Self::Context, NoOpInspector>(frame, context, None)?
            .into_interpreter_action();
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
        Self::insert_interrupted_result(frame, result);
        Ok(None)
    }
}

impl<CTX, INSP, I, P> RwasmEvm<CTX, INSP, I, P> {
    ///
    fn result_created_address(result: &FrameResult) -> Option<Address> {
        let create_outcome = match &result {
            FrameResult::Create(create_outcome) => create_outcome,
            FrameResult::Call(_) => return None,
        };
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
    }

    /// Apply gas changes to the interpreter gas based on the execution result.
    /// In the result, we store info about total gas consumed for the entire interruption,
    /// including pre-execution checks (like call stipend, etc.).
    pub fn insert_interrupted_result(frame: &mut RwasmFrame, result: FrameResult) {
        let created_address = Self::result_created_address(&result);
        // For the frame result we take gas from the result field,
        // because it stores information about gas consumed before the call as well
        let mut result = result.into_interpreter_result();
        match result.result {
            return_ok!() => {
                let remaining = result.gas.remaining();
                frame.interpreter.gas.erase_cost(remaining);
                let refunded = result.gas.refunded();
                frame.interpreter.gas.record_refund(refunded);
                // for CREATE/CREATE2 calls, we need to write the created address into output
                if let Some(created_address) = created_address {
                    result.output = created_address.into_array().into();
                }
            }
            return_revert!() => {
                frame.interpreter.gas.erase_cost(result.gas.remaining());
            }
            InstructionResult::FatalExternalError => {
                panic!("revm: fatal external error");
            }
            _ => {}
        }
        let interrupted_outcome = frame.interrupted_outcome.as_mut().unwrap();
        // Call how much gas we consumed.
        // For the final gas calculation, we must know that amount of gas we had before the call.
        // It's important because we must have all call related spends to be included.
        let mut total_gas_consumed = Gas::new_spent(
            interrupted_outcome.inputs.gas.remaining() - frame.interpreter.gas.remaining(),
        );
        total_gas_consumed.record_refund(
            interrupted_outcome.inputs.gas.refunded() - frame.interpreter.gas.refunded(),
        );
        let int_result = ExecutionResult {
            result: result.result,
            output: result.output,
            gas: total_gas_consumed,
        };
        interrupted_outcome.result = Some(int_result);
    }
}
