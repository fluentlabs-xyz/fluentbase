use crate::context::RuntimeContext;
use fluentbase_types::{
    byteorder::{ByteOrder, LittleEndian},
    ExitCode, B256, STATE_DEPLOY, STATE_MAIN,
};
use rwasm::{Store, Strategy, TrapCode, TypedStore, Value};
use std::{fmt::Debug, mem::take, sync::Arc};

/// Finalized outcome of a single runtime invocation.
///
/// Values are reported in fuel units; gas conversion (if any) is handled by the caller.
#[derive(Default, Clone, Debug)]
pub struct ExecutionResult {
    /// Contract-defined exit status. Negative values map from TrapCode via ExitCode; zero indicates success.
    pub exit_code: i32,
    /// Total fuel consumed by the invocation (excludes refunded fuel).
    pub fuel_consumed: u64,
    /// Fuel refunded to the caller (negative values are not expected).
    pub fuel_refunded: i64,
    /// Raw output buffer produced by the callee; for nested calls it is moved into the parent's return_data.
    pub output: Vec<u8>,
    /// Return data propagated back to the parent on success paths of nested calls.
    pub return_data: Vec<u8>,
}

/// Captures an intentional execution interruption that must be resumed by the root context.
#[derive(Debug, Default, Clone)]
pub struct ExecutionInterruption {
    /// Fuel spent up to the interruption point.
    pub fuel_consumed: u64,
    /// Fuel to refund to the caller at the interruption point.
    pub fuel_refunded: i64,
    /// Encoded interruption payload (e.g., delegated call parameters).
    pub output: Vec<u8>,
}

/// Result of running or resuming a runtime.
#[derive(Clone, Debug)]
pub enum RuntimeResult {
    /// Execution finished; contains the finalized result.
    Result(ExecutionResult),
    /// Execution yielded; contains data necessary to resume later.
    Interruption(ExecutionInterruption),
}

impl RuntimeResult {
    /// Unwraps the successful execution result; panics if this is an interruption.
    pub fn into_execution_result(self) -> ExecutionResult {
        match self {
            RuntimeResult::Result(result) => result,
            _ => unreachable!(),
        }
    }
}

/// A compiled, executable runtime instance with its store and engine strategy.
pub struct Runtime {
    /// Underlying execution strategy (rWasm/Wasmtime).
    pub strategy: Arc<Strategy>,
    /// Engine store carrying linear memory and the RuntimeContext.
    pub store: TypedStore<RuntimeContext>,
    /// Code hash identifying the compiled module within the cache.
    pub code_hash: B256,
}

impl Runtime {
    /// Creates a runtime from bytecode or code hash and initializes its store with the provided context.
    pub fn new(
        strategy: Arc<Strategy>,
        store: TypedStore<RuntimeContext>,
        code_hash: B256,
    ) -> Self {
        Self {
            strategy,
            store,
            code_hash,
        }
    }

    /// Executes the entry function of the module determined by the current execution state.
    ///
    /// Returns either a finalized result or an interruption that must be resumed by the root.
    #[tracing::instrument(level = "info", skip_all)]
    pub fn execute(&mut self) -> RuntimeResult {
        let (fuel_limit, disable_fuel) =
            self.store.context(|ctx| (ctx.fuel_limit, ctx.disable_fuel));
        let result = self.execute_inner();
        let fuel_limit = if disable_fuel { None } else { Some(fuel_limit) };
        self.handle_execution_result(result, fuel_limit)
    }

    fn execute_inner(&mut self) -> Result<(), TrapCode> {
        let state = self.store.context(|ctx| ctx.state);
        let func_name = match state {
            STATE_MAIN => "main",
            STATE_DEPLOY => "deploy",
            _ => unreachable!(),
        };
        self.strategy
            .execute(&mut self.store, func_name, &[], &mut [])
    }

    /// Resumes a previously interrupted runtime.
    ///
    /// fuel16_ptr optionally points to a 16-byte buffer where fuel consumption and refund are written back.
    #[tracing::instrument(level = "info", skip_all, fields(fuel_ptr = fuel16_ptr, exit_code = exit_code))]
    pub fn resume(
        &mut self,
        return_data: Vec<u8>,
        fuel16_ptr: u32,
        fuel_consumed: u64,
        fuel_refunded: i64,
        exit_code: i32,
    ) -> RuntimeResult {
        let disable_fuel = self.store.context(RuntimeContext::is_fuel_disabled);
        let mut fuel_remaining = self.store.remaining_fuel();
        if disable_fuel {
            fuel_remaining = None;
        }
        let result = self.resume_inner(
            return_data,
            fuel16_ptr,
            fuel_consumed,
            fuel_refunded,
            exit_code,
        );
        // We need to adjust the fuel limit because `fuel_consumed` should not be included into spent.
        // We can safely unwrap here, because `OutOfFuel` check we did in `resume_inner`.
        let fuel_remaining = fuel_remaining.map(|v| v.checked_sub(fuel_consumed).unwrap());
        self.handle_execution_result(result, fuel_remaining)
    }

    fn resume_inner(
        &mut self,
        return_data: Vec<u8>,
        fuel16_ptr: u32,
        fuel_consumed: u64,
        fuel_refunded: i64,
        exit_code: i32,
    ) -> Result<(), TrapCode> {
        let fuel_cost = self.store.context_mut(|ctx| {
            // During the résumé we must clear output, otherwise collision might happen
            ctx.execution_result.output.clear();
            // When fuel is disabled, we only pass consumed fuel amount into the contract back,
            // and it can decide on charging
            let fuel_cost = if !ctx.is_fuel_disabled() && fuel_consumed > 0 {
                Some(fuel_consumed)
            } else {
                None
            };
            // Copy return data into return data
            ctx.execution_result.return_data = return_data;
            fuel_cost
        });
        if let Some(fuel_cost) = fuel_cost {
            // Charge fuel that was spent during the interruption
            // to make sure our fuel calculations are aligned
            self.store.try_consume_fuel(fuel_cost)?;
        }
        if fuel16_ptr > 0 {
            let mut buffer = [0u8; 16];
            LittleEndian::write_u64(&mut buffer[..8], fuel_consumed);
            LittleEndian::write_i64(&mut buffer[8..], fuel_refunded);
            self.store.memory_write(fuel16_ptr as usize, &buffer)?;
        }
        self.strategy
            .resume(&mut self.store, &[Value::I32(exit_code)], &mut [])
    }

    /// Consolidates the trap/result of an invocation into a RuntimeResult and updates accounting.
    ///
    /// When fuel_consumed_before_the_call is provided, computes precise fuel usage by diffing the
    /// store's remaining fuel. Returns either a finalized result or an interruption wrapper.
    #[tracing::instrument(level = "info", skip_all)]
    fn handle_execution_result(
        &mut self,
        next_result: Result<(), TrapCode>,
        fuel_consumed_before_the_call: Option<u64>,
    ) -> RuntimeResult {
        let mut execution_result = self
            .store
            .context_mut(|ctx| take(&mut ctx.execution_result));
        // Once fuel is calculated, we must adjust our fuel limit,
        // because we don't know what gas conversion policy is used,
        // if there is rounding then it can cause miscalculations
        if let Some(fuel_consumed_before_the_call) = fuel_consumed_before_the_call {
            let diff = fuel_consumed_before_the_call - self.store.remaining_fuel().unwrap();
            execution_result.fuel_consumed = diff;
        }
        loop {
            match next_result {
                Ok(_) => break,
                Err(TrapCode::InterruptionCalled) => {
                    return self.handle_resumable_state(execution_result);
                }
                Err(err) => {
                    execution_result.exit_code = ExitCode::from(err).into_i32();
                    break;
                }
            }
        }
        RuntimeResult::Result(execution_result)
    }

    /// Converts an in-flight interruption into a RuntimeResult::Interruption and prepares payload.
    ///
    /// Clears transient buffers, encodes the delegated invocation parameters, and packages the
    /// suspended runtime for recovery by the root context.
    fn handle_resumable_state(&mut self, execution_result: ExecutionResult) -> RuntimeResult {
        let ExecutionResult {
            fuel_consumed,
            fuel_refunded,
            ..
        } = execution_result;

        let resumable_context = self
            .store
            .context_mut(|ctx| ctx.resumable_context.take().unwrap());
        if resumable_context.is_root {
            unimplemented!("validate this logic, might not be ok in STF mode");
        }

        // we disallow nested calls at non-root levels,
        // so we must save the current state
        // to interrupt execution and delegate decision-making
        // to the root execution
        self.store.context_mut(|ctx| {
            let output = &mut ctx.execution_result.output;
            output.clear();
            assert!(output.is_empty(), "runtime: return data must be empty");
        });
        // serialize the delegated execution state,
        // but we don't serialize registers and stack state,
        // instead we remember it inside the internal structure
        // and assign a special identifier for recovery
        let output = resumable_context.params.encode();
        RuntimeResult::Interruption(ExecutionInterruption {
            fuel_consumed,
            fuel_refunded,
            output,
        })
    }
}
