//! The `wasmi` interpreter.

pub mod bytecode;
pub mod call_stack;
pub mod code_map;
pub mod exec_context;
pub mod inst_builder;
pub mod value_stack;

pub use self::{
    bytecode::{DropKeep, Target},
    code_map::FuncBody,
    inst_builder::{InstructionIdx, InstructionsBuilder, LabelIdx, Reloc},
};
use self::{
    bytecode::{Instruction, VisitInstruction},
    call_stack::{CallStack, FunctionFrame},
    code_map::{CodeMap, ResolvedFuncBody},
    exec_context::ExecutionContext,
    value_stack::{FromStackEntry, StackEntry, ValueStack},
};
use super::{func::FuncEntityInternal, AsContext, AsContextMut, Func, Signature};
use crate::{
    Trap,
    TrapCode,
    Value,
    ValueType,
    DEFAULT_CALL_STACK_LIMIT,
    DEFAULT_VALUE_STACK_LIMIT,
};
use alloc::{sync::Arc, vec::Vec};
use spin::mutex::Mutex;

/// The outcome of a `wasmi` instruction execution.
///
/// # Note
///
/// This signals to the `wasmi` interpreter what to do after the
/// instruction has been successfully executed.
#[derive(Debug, Copy, Clone)]
pub enum ExecutionOutcome {
    /// Continue with next instruction.
    Continue,
    /// Branch to an instruction at the given position.
    Branch(Target),
    /// Execute function call.
    ExecuteCall(Func),
    /// Return from current function block.
    Return(DropKeep),
}

/// The outcome of a `wasmi` function execution.
#[derive(Debug, Copy, Clone)]
pub enum FunctionExecutionOutcome {
    /// The function has returned.
    Return,
    /// The function called another function.
    NestedCall(Func),
}

/// The `wasmi` interpreter.
///
/// # Note
///
/// - The current `wasmi` engine implements a bytecode interpreter.
/// - This structure is intentionally cheap to copy.
///   Most of its API has a `&self` receiver, so can be shared easily.
#[derive(Debug, Clone)]
pub struct Engine {
    inner: Arc<Mutex<EngineInner>>,
}

/// Configuration for an [`Engine`].
#[derive(Debug)]
pub struct Config {
    /// The internal value stack limit.
    ///
    /// # Note
    ///
    /// Reaching this limit during execution of a Wasm function will
    /// cause a stack overflow trap.
    value_stack_limit: usize,
    /// The internal call stack limit.
    ///
    /// # Note
    ///
    /// Reaching this limit during execution of a Wasm function will
    /// cause a stack overflow trap.
    call_stack_limit: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            value_stack_limit: DEFAULT_VALUE_STACK_LIMIT,
            call_stack_limit: DEFAULT_CALL_STACK_LIMIT,
        }
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new(&Config::default())
    }
}

impl Engine {
    /// Creates a new [`Engine`] with default configuration.
    ///
    /// # Note
    ///
    /// Users should ues [`Engine::default`] to construct a default [`Engine`].
    fn new(config: &Config) -> Self {
        Self {
            inner: Arc::new(Mutex::new(EngineInner::new(config))),
        }
    }

    /// Allocates the instructions of a Wasm function body to the [`Engine`].
    ///
    /// Returns a [`FuncBody`] reference to the allocated function body.
    pub(super) fn alloc_func_body<I>(
        &self,
        len_locals: usize,
        max_stack_height: usize,
        insts: I,
    ) -> FuncBody
    where
        I: IntoIterator<Item = Instruction>,
        I::IntoIter: ExactSizeIterator,
    {
        self.inner
            .lock()
            .alloc_func_body(len_locals, max_stack_height, insts)
    }

    /// Resolves the [`FuncBody`] to the underlying `wasmi` bytecode instructions.
    ///
    /// # Note
    ///
    /// - This API is mainly intended for unit testing purposes and shall not be used
    ///   outside of this context. The function bodies are intended to be data private
    ///   to the `wasmi` interpreter.
    ///
    /// # Panics
    ///
    /// If the [`FuncBody`] is invalid for the [`Engine`].
    #[cfg(test)]
    pub(crate) fn resolve_inst(&self, func_body: FuncBody, index: usize) -> Instruction {
        self.inner
            .lock()
            .code_map
            .resolve(func_body)
            .get(index)
            .clone()
    }

    /// Executes the given [`Func`] using the given arguments `args` and stores the result into `results`.
    ///
    /// # Errors
    ///
    /// - If the given `func` is not a Wasm function, e.g. if it is a host function.
    /// - If the given arguments `args` do not match the expected parameters of `func`.
    /// - If the given `results` do not match the the length of the expected results of `func`.
    /// - When encountering a Wasm trap during the execution of `func`.
    pub(crate) fn execute_func(
        &mut self,
        ctx: impl AsContextMut,
        func: Func,
        args: &[Value],
        results: &mut [Value],
    ) -> Result<(), Trap> {
        self.inner.lock().execute_func(ctx, func, args, results)
    }
}

/// The internal state of the `wasmi` engine.
#[derive(Debug)]
pub struct EngineInner {
    /// Stores the value stack of live values on the Wasm stack.
    value_stack: ValueStack,
    /// Stores the call stack of live function invocations.
    call_stack: CallStack,
    /// Stores all Wasm function bodies that the interpreter is aware of.
    code_map: CodeMap,
    /// Scratch buffer for intermediate results and data.
    scratch: Vec<Value>,
}

impl EngineInner {
    pub fn new(config: &Config) -> Self {
        Self {
            value_stack: ValueStack::new(64, config.value_stack_limit),
            call_stack: CallStack::new(config.call_stack_limit),
            code_map: CodeMap::default(),
            scratch: Vec::new(),
        }
    }

    /// Allocates the instructions of a Wasm function body to the [`Engine`].
    ///
    /// Returns a [`FuncBody`] reference to the allocated function body.
    pub fn alloc_func_body<I>(
        &mut self,
        len_locals: usize,
        max_stack_height: usize,
        insts: I,
    ) -> FuncBody
    where
        I: IntoIterator<Item = Instruction>,
        I::IntoIter: ExactSizeIterator,
    {
        self.code_map.alloc(len_locals, max_stack_height, insts)
    }

    /// Executes the given [`Func`] using the given arguments `args` and stores the result into `results`.
    ///
    /// # Errors
    ///
    /// - If the given arguments `args` do not match the expected parameters of `func`.
    /// - If the given `results` do not match the the length of the expected results of `func`.
    /// - When encountering a Wasm trap during the execution of `func`.
    pub fn execute_func(
        &mut self,
        mut ctx: impl AsContextMut,
        func: Func,
        args: &[Value],
        results: &mut [Value],
    ) -> Result<(), Trap> {
        match func.as_internal(&ctx) {
            FuncEntityInternal::Wasm(wasm_func) => {
                let signature = wasm_func.signature();
                self.execute_wasm_func(&mut ctx, signature, args, results, func)?;
            }
            FuncEntityInternal::Host(host_func) => {
                let signature = host_func.signature();
                Self::check_signature(ctx.as_context(), signature, args, results)?;
                host_func.clone().call(&mut ctx, None, args, results)?;
            }
        }
        Ok(())
    }

    /// Executes the given Wasm [`Func`] using the given arguments `args` and stores the result into `results`.
    ///
    /// # Note
    ///
    /// The caller is required to ensure that the given `func` actually is a Wasm function.
    ///
    /// # Errors
    ///
    /// - If the given arguments `args` do not match the expected parameters of `func`.
    /// - If the given `results` do not match the the length of the expected results of `func`.
    /// - When encountering a Wasm trap during the execution of `func`.
    fn execute_wasm_func(
        &mut self,
        mut ctx: impl AsContextMut,
        signature: Signature,
        args: &[Value],
        results: &mut [Value],
        func: Func,
    ) -> Result<(), Trap> {
        self.value_stack.clear();
        self.call_stack.clear();
        Self::check_signature(ctx.as_context(), signature, args, results)?;
        self.initialize_args(args);
        let frame = FunctionFrame::new(ctx.as_context(), func);
        self.call_stack
            .push(frame)
            .map_err(|_error| TrapCode::StackOverflow)?;
        self.execute_until_done(ctx.as_context_mut())?;
        let result_types = signature.outputs(&ctx);
        self.write_results_back(result_types, results);
        Ok(())
    }

    /// Writes the results of the function execution back into the `results` buffer.
    ///
    /// # Note
    ///
    /// The value stack is empty after this operation.
    ///
    /// # Panics
    ///
    /// - If the `results` buffer length does not match the remaining amount of stack values.
    fn write_results_back(&mut self, result_types: &[ValueType], results: &mut [Value]) {
        assert_eq!(
            self.value_stack.len(),
            results.len(),
            "expected {} values on the stack after function execution but found {}",
            results.len(),
            self.value_stack.len(),
        );
        assert_eq!(results.len(), result_types.len());
        for (result, (value, value_type)) in results
            .iter_mut()
            .zip(self.value_stack.drain().iter().zip(result_types))
        {
            *result = value.with_type(*value_type);
        }
    }

    /// Executes functions until the call stack is empty.
    ///
    /// # Errors
    ///
    /// - If any of the executed instructions yield an error.
    fn execute_until_done(&mut self, mut ctx: impl AsContextMut) -> Result<(), Trap> {
        'outer: loop {
            let mut function_frame = match self.call_stack.pop() {
                Some(frame) => frame,
                None => return Ok(()),
            };
            let result =
                ExecutionContext::new(self, &mut function_frame)?.execute_frame(&mut ctx)?;
            match result {
                FunctionExecutionOutcome::Return => {
                    continue 'outer;
                }
                FunctionExecutionOutcome::NestedCall(func) => {
                    match func.as_internal(ctx.as_context()) {
                        FuncEntityInternal::Wasm(wasm_func) => {
                            let nested_frame = FunctionFrame::new_wasm(func, wasm_func);
                            self.call_stack
                                .push(function_frame)
                                .map_err(|_| TrapCode::StackOverflow)?;
                            self.call_stack
                                .push(nested_frame)
                                .map_err(|_| TrapCode::StackOverflow)?;
                        }
                        FuncEntityInternal::Host(host_func) => {
                            let signature = host_func.signature();
                            let (input_types, output_types) =
                                signature.inputs_outputs(ctx.as_context());
                            Self::prepare_host_function_args(
                                input_types,
                                &mut self.value_stack,
                                &mut self.scratch,
                            );
                            // Note: We push the function context before calling the host function.
                            //       If the VM is not resumable, it does no harm.
                            //       If it is, we then save the context here.
                            let instance = function_frame.instance();
                            self.call_stack
                                .push(function_frame)
                                .map_err(|_| TrapCode::StackOverflow)?;
                            // Prepare scratch buffer to hold both, inputs and outputs of the call.
                            // We are going to split the scratch buffer in the middle right before the call.
                            debug_assert_eq!(self.scratch.len(), input_types.len());
                            let len_inputs = input_types.len();
                            let len_outputs = output_types.len();
                            let zeros = output_types.iter().copied().map(Value::default);
                            self.scratch.extend(zeros);
                            // At this point the scratch buffer holds the host function input arguments
                            // as well as a zero initialized entry per expected host function output value.
                            let (inputs, outputs) = self.scratch.split_at_mut(len_inputs);
                            debug_assert_eq!(inputs.len(), len_inputs);
                            debug_assert_eq!(outputs.len(), len_outputs);
                            // Now we are ready to perform the host function call.
                            // Note: We need to clone the host function due to some borrowing issues.
                            //       This should not be a big deal since host functions usually are cheap to clone.
                            host_func.clone().call(
                                ctx.as_context_mut(),
                                Some(instance),
                                inputs,
                                outputs,
                            )?;
                            // Check if the returned values match their expected types.
                            let output_types = signature.outputs(ctx.as_context());
                            for (required_type, output_value) in
                                output_types.iter().copied().zip(&*outputs)
                            {
                                if required_type != output_value.value_type() {
                                    return Err(TrapCode::UnexpectedSignature).map_err(Into::into);
                                }
                            }
                            // Copy host function output values to the value stack.
                            self.value_stack
                                .extend(outputs.iter().copied().map(|value| value.into()));
                        }
                    }
                }
            }
        }
    }

    /// Prepares the inputs arguments to the host function execution.
    ///
    /// # Note
    ///
    /// This will pop the last `n` values from the `value_stack` where
    /// `n` is the number of required input arguments to the host function
    /// and equal to the number of elements in `input_types`.
    /// The `input_types` slice represents the value types required by the
    /// host function that is about to be called.
    fn prepare_host_function_args(
        input_types: &[ValueType],
        value_stack: &mut ValueStack,
        host_args: &mut Vec<Value>,
    ) {
        let len_args = input_types.len();
        let stack_args = value_stack.pop_as_slice(len_args);
        assert_eq!(len_args, stack_args.len());
        host_args.clear();
        let prepared_args = input_types
            .iter()
            .zip(stack_args)
            .map(|(input_type, host_arg)| host_arg.with_type(*input_type));
        host_args.extend(prepared_args);
    }

    /// Initializes the value stack with the given arguments `args`.
    fn initialize_args(&mut self, args: &[Value]) {
        assert!(
            self.value_stack.is_empty(),
            "encountered non-empty value stack upon function execution initialization",
        );
        for &arg in args {
            self.value_stack.push(arg);
        }
    }

    /// Checks if the `signature` and the given `params` and `results` slices match.
    ///
    /// # Errors
    ///
    /// - If the given `signature` inputs and `params` do not have matching length and value types.
    /// - If the given `signature` outputs and `results` do not have the same lengths.
    fn check_signature(
        ctx: impl AsContext,
        signature: Signature,
        params: &[Value],
        results: &[Value],
    ) -> Result<(), TrapCode> {
        let expected_inputs = signature.inputs(ctx.as_context());
        let expected_outputs = signature.outputs(ctx.as_context());
        let actual_inputs = params.iter().map(|value| value.value_type());
        if expected_inputs.iter().copied().ne(actual_inputs)
            || expected_outputs.len() != results.len()
        {
            return Err(TrapCode::UnexpectedSignature);
        }
        Ok(())
    }
}
