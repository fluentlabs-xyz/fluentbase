#[cfg(feature = "global-executor")]
mod global_executor;
mod local_executor;

use crate::{
    module_factory::ModuleFactory,
    runtime::{ExecutionMode, StrategyRuntime},
    RuntimeContext,
};
use fluentbase_types::{
    byteorder::{ByteOrder, LittleEndian},
    Address, BytecodeOrHash, ExitCode, HashMap, B256,
};
use local_executor::LocalExecutor;
use rwasm::{ExecutionEngine, FuelConfig, ImportLinker, RwasmModule, Strategy, TrapCode};
use std::{
    mem::take,
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
};

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

impl ExecutionResult {
    pub fn take_and_continue(&mut self, is_interrupted: bool) -> Self {
        let mut result = take(self);
        // We don't propagate output into intermediary state
        if is_interrupted {
            self.output = take(&mut result.output);
            self.return_data = take(&mut result.return_data);
        }
        result
    }
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

pub trait RuntimeExecutor {
    /// Executes the entry function of the module determined by the current execution state.
    ///
    /// Returns either a finalized result.
    fn execute(&mut self, bytecode_or_hash: BytecodeOrHash, ctx: RuntimeContext)
        -> ExecutionResult;

    /// Resumes a previously interrupted runtime.
    ///
    /// fuel16_ptr optionally points to a 16-byte buffer where fuel consumption and refund are written back.
    fn resume(
        &mut self,
        call_id: u32,
        return_data: &[u8],
        fuel16_ptr: u32,
        fuel_consumed: u64,
        fuel_refunded: i64,
        exit_code: i32,
    ) -> ExecutionResult;

    /// Drop a runtime we don't need to resume anymore
    fn forget_runtime(&mut self, call_id: u32);

    /// Warmup the bytecode
    fn warmup(&mut self, bytecode: RwasmModule, hash: B256, address: Address);

    #[cfg(feature = "wasmtime")]
    fn warmup_wasmtime(
        &mut self,
        rwasm_module: RwasmModule,
        wasmtime_module: wasmtime::Module,
        code_hash: B256,
    );

    /// Resets the per-transaction call identifier counter and clears recoverable runtimes.
    ///
    /// Intended to be invoked at the beginning of a new transaction.
    fn reset_call_id_counter(&mut self);
}

/// Returns a default runtime executor.
pub fn default_runtime_executor() -> impl RuntimeExecutor {
    LocalExecutor {}
}

pub struct RuntimeFactoryExecutor {
    /// A module factory
    pub module_factory: ModuleFactory,
    /// Suspended runtimes keyed by per-transaction call identifier.
    pub recoverable_runtimes: HashMap<u32, ExecutionMode>,
    /// An import linker
    pub import_linker: Arc<ImportLinker>,
    /// Monotonically increasing counter for assigning call identifiers.
    pub transaction_call_id_counter: AtomicU32,
}

impl RuntimeFactoryExecutor {
    pub fn new(import_linker: Arc<ImportLinker>) -> Self {
        Self {
            module_factory: ModuleFactory::new(),
            recoverable_runtimes: HashMap::new(),
            import_linker,
            transaction_call_id_counter: AtomicU32::new(1),
        }
    }

    /// Saves the current runtime instance for later resumption and returns its call identifier.
    pub fn try_remember_runtime(
        &mut self,
        runtime_result: RuntimeResult,
        runtime: ExecutionMode,
    ) -> ExecutionResult {
        let interruption = match runtime_result {
            RuntimeResult::Result(result) => {
                // Return result (there is no need to do anything else)
                return result;
            }
            RuntimeResult::Interruption(interruption) => interruption,
        };
        // Calculate new `call_id` counter (a runtime recover identifier)
        let call_id = self
            .transaction_call_id_counter
            .fetch_add(1, Ordering::Relaxed);
        // Remember the runtime
        self.recoverable_runtimes.insert(call_id, runtime);
        ExecutionResult {
            // We return `call_id` as exit code (it's safe, because exit code can't be positive)
            exit_code: call_id as i32,
            // Forward info about consumed and refunded fuel (during the call)
            fuel_consumed: interruption.fuel_consumed,
            fuel_refunded: interruption.fuel_refunded,
            // The output we map into return data
            output: interruption.output,
            return_data: vec![],
        }
    }

    /// Consolidates the trap/result of an invocation into a RuntimeResult and updates accounting.
    ///
    /// When fuel_consumed_before_the_call is provided, computes precise fuel usage by diffing the
    /// store's remaining fuel. Returns either a finalized result or an interruption wrapper.
    fn handle_execution_result(
        &mut self,
        next_result: Result<(), TrapCode>,
        fuel_consumed: Option<u64>,
        ctx: &mut RuntimeContext,
    ) -> RuntimeResult {
        let mut execution_result = ctx
            .execution_result
            .take_and_continue(ctx.resumable_context.is_some());
        // There are two counters for fuel: opcode fuel counter; manually charged.
        // It's applied for execution runtimes where we don't know the final fuel consumed,
        // till it's committed by Wasm runtime.
        // That is why we rewrite fuel here to check how much we've really spent based on the context information.
        if let Some(store_fuel_consumed) = fuel_consumed {
            execution_result.fuel_consumed = store_fuel_consumed;
        }
        match next_result {
            Ok(_) => {}
            Err(TrapCode::InterruptionCalled) => {
                return self.handle_resumable_state(execution_result, ctx);
            }
            Err(err) => {
                execution_result.exit_code = ExitCode::from(err).into_i32();
            }
        }
        RuntimeResult::Result(execution_result)
    }

    /// Converts an in-flight interruption into a RuntimeResult::Interruption and prepares payload.
    ///
    /// Clears transient buffers, encodes the delegated invocation parameters, and packages the
    /// suspended runtime for recovery by the root context.
    fn handle_resumable_state(
        &mut self,
        execution_result: ExecutionResult,
        ctx: &mut RuntimeContext,
    ) -> RuntimeResult {
        let ExecutionResult {
            fuel_consumed,
            fuel_refunded,
            ..
        } = execution_result;
        // Take resumable context from execution context
        let resumable_context = ctx.resumable_context.take().unwrap();
        if resumable_context.is_root {
            unimplemented!("validate this logic, might not be ok in STF mode");
        }
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

impl RuntimeExecutor for RuntimeFactoryExecutor {
    fn execute(
        &mut self,
        bytecode_or_hash: BytecodeOrHash,
        ctx: RuntimeContext,
    ) -> ExecutionResult {
        #[cfg(feature = "wasmtime")]
        let enable_wasmtime_runtime = match &bytecode_or_hash {
            BytecodeOrHash::Bytecode { address, hash, .. } => {
                fluentbase_types::is_execute_using_aot_compiler(address)
                    .then_some((*address, *hash))
            }
            BytecodeOrHash::Hash(_) => None,
        };

        // If we have a cached module, then use it, otherwise create a new one and cache
        let module = self.module_factory.get_module_or_init(bytecode_or_hash);

        // If there is no cached store, then construct a new one (slow)
        let fuel_remaining = Some(ctx.fuel_limit);
        let fuel_config = FuelConfig::default().with_fuel_limit(ctx.fuel_limit);

        #[cfg(feature = "wasmtime")]
        let strategy = if let Some((address, code_hash)) = enable_wasmtime_runtime {
            let module = self
                .module_factory
                .get_wasmtime_module_or_compile(code_hash, address);
            Strategy::Wasmtime { module }
        } else {
            let engine = ExecutionEngine::acquire_shared();
            Strategy::Rwasm { module, engine }
        };
        #[cfg(not(feature = "wasmtime"))]
        let strategy = {
            let engine = ExecutionEngine::acquire_shared();
            Strategy::Rwasm { module, engine }
        };
        let runtime = StrategyRuntime::new(strategy, self.import_linker.clone(), ctx, fuel_config);
        let mut runtime = ExecutionMode::Strategy(runtime);

        // Execute rWasm program
        let result = runtime.execute();
        let fuel_consumed = runtime
            .remaining_fuel()
            .zip(fuel_remaining)
            .map(|(remaining_fuel, store_fuel)| store_fuel - remaining_fuel);
        let runtime_result =
            runtime.context_mut(|ctx| self.handle_execution_result(result, fuel_consumed, ctx));
        self.try_remember_runtime(runtime_result, runtime)
    }

    fn resume(
        &mut self,
        call_id: u32,
        return_data: &[u8],
        fuel16_ptr: u32,
        fuel_consumed: u64,
        fuel_refunded: i64,
        exit_code: i32,
    ) -> ExecutionResult {
        let mut runtime = self
            .recoverable_runtimes
            .remove(&call_id)
            .expect("runtime: can't resolve runtime by id, it should never happen");
        let mut fuel_remaining = runtime.remaining_fuel();
        let resume_inner = |runtime: &mut ExecutionMode| {
            // Copy return data into return data
            runtime.context_mut(|ctx| {
                ctx.execution_result.return_data = return_data.to_vec();
            });
            // If we have fuel consumed greater than 0 then record it
            if fuel_consumed > 0 {
                runtime.try_consume_fuel(fuel_consumed)?;
            }
            if fuel16_ptr > 0 {
                let mut buffer = [0u8; 16];
                LittleEndian::write_u64(&mut buffer[..8], fuel_consumed);
                LittleEndian::write_i64(&mut buffer[8..], fuel_refunded);
                runtime.memory_write(fuel16_ptr as usize, &buffer)?;
            }
            runtime.resume(exit_code)
        };
        let result = resume_inner(&mut runtime);
        // We need to adjust the fuel limit because `fuel_consumed` should not be included into spent.
        if result != Err(TrapCode::OutOfFuel) {
            // SAFETY: We can safely unwrap here, because `OutOfFuel` check we did in `resume_inner` and the result is ok.
            fuel_remaining = fuel_remaining.map(|v| v.checked_sub(fuel_consumed).unwrap());
        }
        let fuel_consumed = runtime
            .remaining_fuel()
            .and_then(|remaining_fuel| Some(fuel_remaining? - remaining_fuel));
        let runtime_result =
            runtime.context_mut(|ctx| self.handle_execution_result(result, fuel_consumed, ctx));
        let result = self.try_remember_runtime(runtime_result, runtime);
        result
    }

    fn forget_runtime(&mut self, call_id: u32) {
        _ = self.recoverable_runtimes.remove(&call_id);
    }

    fn warmup(&mut self, bytecode: RwasmModule, hash: B256, address: Address) {
        self.module_factory
            .get_module_or_init(BytecodeOrHash::Bytecode {
                bytecode,
                hash,
                address,
            });
        #[cfg(feature = "wasmtime")]
        self.module_factory
            .get_wasmtime_module_or_compile(hash, address);
    }

    #[cfg(feature = "wasmtime")]
    fn warmup_wasmtime(
        &mut self,
        rwasm_module: RwasmModule,
        wasmtime_module: wasmtime::Module,
        hash: B256,
    ) {
        self.module_factory
            .warmup_wasmtime(rwasm_module, wasmtime_module, hash);
    }

    fn reset_call_id_counter(&mut self) {
        self.transaction_call_id_counter.store(1, Ordering::Relaxed);
    }
}
