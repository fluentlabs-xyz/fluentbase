use crate::{
    context::RuntimeContext, factory::CACHING_RUNTIME_FACTORY,
    syscall_handler::runtime_syscall_handler,
};
use fluentbase_types::{
    byteorder::{ByteOrder, LittleEndian},
    Address, BytecodeOrHash, Bytes, ExitCode, B256, STATE_DEPLOY, STATE_MAIN,
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
    /// Return data propagated back to the parent on success paths of nested calls.
    pub return_data: Vec<u8>,
    /// Raw output buffer produced by the callee; for nested calls it is moved into the parent's return_data.
    pub output: Vec<u8>,
}

/// Captures an intentional execution interruption that must be resumed by the root context.
pub struct ExecutionInterruption {
    /// Fuel spent up to the interruption point.
    pub fuel_consumed: u64,
    /// Fuel to refund to the caller at the interruption point.
    pub fuel_refunded: i64,
    /// Encoded interruption payload (e.g., delegated call parameters).
    pub output: Vec<u8>,
    /// Suspended runtime instance to be resumed by the root.
    pub runtime: Runtime,
}

/// Result of running or resuming a runtime.
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

    /// Finalizes the result into (fuel_consumed, fuel_refunded, exit_code) and updates the parent context.
    pub fn finalize(self, ctx: &mut RuntimeContext) -> (u64, i64, i32) {
        match self {
            RuntimeResult::Result(result) => {
                // Move output into parent's return data
                ctx.execution_result.return_data = result.output;
                (result.fuel_consumed, result.fuel_refunded, result.exit_code)
            }
            RuntimeResult::Interruption(interruption) => {
                // Move output into parent's return data
                ctx.execution_result.return_data = interruption.output;
                let call_id = interruption.runtime.remember_runtime(ctx);
                (
                    interruption.fuel_consumed,
                    interruption.fuel_refunded,
                    // We return `call_id` as exit code, it's safe since exit code can't be positive
                    call_id,
                )
            }
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
    #[tracing::instrument(level = "info", skip_all)]
    pub fn new(bytecode_or_hash: BytecodeOrHash, runtime_context: RuntimeContext) -> Self {
        CACHING_RUNTIME_FACTORY.with_borrow_mut(|caching_runtime| {
            let code_hash = bytecode_or_hash.code_hash();

            // If we have a cached module, then use it, otherwise create a new one and cache
            let cached_module = caching_runtime.get_module_or_init(bytecode_or_hash);
            let (strategy, mut store) = cached_module.acquire_shared();

            // If there is cached store then reuse it, but rewrite the context data
            if let Some(store) = store.as_mut() {
                match store {
                    TypedStore::Rwasm(rwasm_store) => {
                        // A special case for rWasm, we need to reset state
                        rwasm_store.reset(false);
                    }
                    _ => {}
                }
                store.context_mut(|context_ref| *context_ref = runtime_context.clone());
            }

            // If there is no cached store, then construct a new one (slow)
            let store = store.unwrap_or_else(|| {
                strategy.create_store(
                    caching_runtime.import_linker.clone(),
                    runtime_context,
                    runtime_syscall_handler,
                )
            });

            Self {
                strategy,
                store,
                code_hash,
            }
        })
    }

    /// Executes the entry function of the module determined by the current execution state.
    ///
    /// Returns either a finalized result or an interruption that must be resumed by the root.
    #[tracing::instrument(level = "info", skip_all)]
    pub fn execute(mut self) -> RuntimeResult {
        let (fuel_limit, disable_fuel) =
            self.store.context(|ctx| (ctx.fuel_limit, ctx.disable_fuel));
        let result = self.execute_inner(Some(fuel_limit));
        let fuel_limit = if disable_fuel { None } else { Some(fuel_limit) };
        self.handle_execution_result(result, fuel_limit)
    }

    fn execute_inner(&mut self, fuel: Option<u64>) -> Result<(), TrapCode> {
        let state = self.store.context(|ctx| ctx.state);
        let func_name = match state {
            STATE_MAIN => "main",
            STATE_DEPLOY => "deploy",
            _ => unreachable!(),
        };
        self.strategy
            .execute(&mut self.store, func_name, &[], &mut [], fuel)
    }

    /// Resumes a previously interrupted runtime.
    ///
    /// fuel16_ptr optionally points to a 16-byte buffer where fuel consumption and refund are written back.
    #[tracing::instrument(level = "info", skip_all, fields(fuel_ptr = fuel16_ptr, exit_code = exit_code))]
    pub fn resume(
        mut self,
        fuel16_ptr: u32,
        fuel_consumed: u64,
        fuel_refunded: i64,
        exit_code: i32,
    ) -> RuntimeResult {
        let disable_fuel = self.store.context(|ctx| ctx.is_fuel_disabled());
        let mut fuel_remaining = self.store.remaining_fuel();
        if disable_fuel {
            fuel_remaining = None;
        }
        let result = self.resume_inner(fuel16_ptr, fuel_consumed, fuel_refunded, exit_code);
        self.handle_execution_result(result, fuel_remaining)
    }

    fn resume_inner(
        &mut self,
        fuel16_ptr: u32,
        fuel_consumed: u64,
        fuel_refunded: i64,
        exit_code: i32,
    ) -> Result<(), TrapCode> {
        if fuel16_ptr > 0 {
            let mut buffer = [0u8; 16];
            LittleEndian::write_u64(&mut buffer[..8], fuel_consumed);
            LittleEndian::write_i64(&mut buffer[8..], fuel_refunded);
            self.store.memory_write(fuel16_ptr as usize, &buffer)?;
        }
        self.strategy
            .resume(&mut self.store, &[Value::I32(exit_code)], &mut [])
    }

    /// Pre-compiles and caches a module for the given code hash and address if not already cached.
    pub fn warmup_strategy(bytecode: Bytes, hash: B256, address: Address) {
        // Ensure the strategy is created and cached ahead of time to avoid JIT cost on first call
        CACHING_RUNTIME_FACTORY.with_borrow_mut(|caching_runtime| {
            caching_runtime.get_module_or_init(BytecodeOrHash::Bytecode {
                address,
                bytecode,
                hash,
            });
        })
    }

    /// Saves the current runtime instance for later resumption and returns its call identifier.
    pub(crate) fn remember_runtime(self, _root_ctx: &mut RuntimeContext) -> i32 {
        // save the current runtime state for future recovery
        CACHING_RUNTIME_FACTORY.with_borrow_mut(|caching_runtime| {
            let call_id = caching_runtime.transaction_call_id_counter;
            caching_runtime.transaction_call_id_counter += 1;
            // root_ctx.call_counter += 1;
            // let call_id = root_ctx.call_counter;
            caching_runtime.recoverable_runtimes.insert(call_id, self);
            call_id as i32
        })
    }

    /// Returns the internal store to the cache associated with this runtime's code hash.
    pub(crate) fn return_store(self) {
        CACHING_RUNTIME_FACTORY.with_borrow_mut(|caching_runtime| {
            if let Some(cached_module) = caching_runtime.cached_modules.get_mut(&self.code_hash) {
                cached_module.return_store(self.store);
            }
        });
    }

    /// Fetches and removes a previously remembered runtime by its call identifier.
    pub(crate) fn recover_runtime(call_id: u32) -> Runtime {
        CACHING_RUNTIME_FACTORY.with_borrow_mut(|caching_runtime| {
            caching_runtime
                .recoverable_runtimes
                .remove(&call_id)
                .expect("runtime: can't resolve runtime by id, it should never happen")
        })
    }

    /// Consolidates the trap/result of an invocation into a RuntimeResult and updates accounting.
    ///
    /// When fuel_consumed_before_the_call is provided, computes precise fuel usage by diffing the
    /// store's remaining fuel. Returns either a finalized result or an interruption wrapper.
    #[tracing::instrument(level = "info", skip_all)]
    fn handle_execution_result(
        mut self,
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
        self.return_store();
        RuntimeResult::Result(execution_result)
    }

    /// Converts an in-flight interruption into a RuntimeResult::Interruption and prepares payload.
    ///
    /// Clears transient buffers, encodes the delegated invocation parameters, and packages the
    /// suspended runtime for recovery by the root context.
    fn handle_resumable_state(mut self, execution_result: ExecutionResult) -> RuntimeResult {
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
            runtime: self,
        })
    }
}

/// Resets the per-transaction call identifier counter and clears recoverable runtimes.
///
/// Intended to be invoked at the beginning of a new transaction.
pub fn reset_call_id_counter() {
    CACHING_RUNTIME_FACTORY.with_borrow_mut(|caching_runtime| {
        caching_runtime.transaction_call_id_counter = 1;
        caching_runtime.recoverable_runtimes.clear();
    });
}
