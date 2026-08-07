use crate::{
    metrics::{self, RuntimeModeLabel, RuntimeTimer},
    module_factory::ModuleFactory,
    runtime::{ContractRuntime, ExecutionMode, SystemRuntime},
    RuntimeContext,
};
use fluentbase_types::{
    byteorder::{ByteOrder, LittleEndian},
    import_linker_v1_preview, Address, BytecodeOrHash, ExitCode, HashMap, B256,
    MAX_IN_FLIGHT_MEMORY_BYTES,
};
use rwasm::{ExecutionEngine, ImportLinker, RwasmModule, StrategyDefinition, TrapCode};
use std::{cell::RefCell, mem::take, sync::Arc};

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
        // We don't propagate output into an intermediary state
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
    pub return_data: Vec<u8>,
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
    /// `fuel16_ptr` optionally points to a 16-byte buffer where fuel consumption and refund are written back.
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

    /// Warm up the bytecode
    fn warmup(&mut self, bytecode: RwasmModule, hash: B256, address: Address);

    /// Resets the per-transaction call identifier counter and clears recoverable runtimes.
    ///
    /// Intended to be invoked at the beginning of a new transaction.
    fn reset_call_id_counter(&mut self);

    fn memory_read(
        &mut self,
        call_id: u32,
        offset: usize,
        buffer: &mut [u8],
    ) -> Result<(), TrapCode>;
}

pub struct ThreadLocalExecutor;

thread_local! {
    pub static LOCAL_RUNTIME_EXECUTOR: RefCell<RuntimeFactoryExecutor> = RefCell::new(RuntimeFactoryExecutor::new(import_linker_v1_preview()));
}

impl RuntimeExecutor for ThreadLocalExecutor {
    fn execute(
        &mut self,
        bytecode_or_hash: BytecodeOrHash,
        ctx: RuntimeContext,
    ) -> ExecutionResult {
        LOCAL_RUNTIME_EXECUTOR
            .with_borrow_mut(|runtime_executor| runtime_executor.execute(bytecode_or_hash, ctx))
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
        LOCAL_RUNTIME_EXECUTOR.with_borrow_mut(|runtime_executor| {
            runtime_executor.resume(
                call_id,
                return_data,
                fuel16_ptr,
                fuel_consumed,
                fuel_refunded,
                exit_code,
            )
        })
    }

    fn forget_runtime(&mut self, call_id: u32) {
        LOCAL_RUNTIME_EXECUTOR
            .with_borrow_mut(|runtime_executor| runtime_executor.forget_runtime(call_id))
    }

    fn warmup(&mut self, bytecode: RwasmModule, hash: B256, address: Address) {
        LOCAL_RUNTIME_EXECUTOR
            .with_borrow_mut(|runtime_executor| runtime_executor.warmup(bytecode, hash, address))
    }

    fn reset_call_id_counter(&mut self) {
        LOCAL_RUNTIME_EXECUTOR
            .with_borrow_mut(|runtime_executor| runtime_executor.reset_call_id_counter())
    }

    fn memory_read(
        &mut self,
        call_id: u32,
        offset: usize,
        buffer: &mut [u8],
    ) -> Result<(), TrapCode> {
        LOCAL_RUNTIME_EXECUTOR.with_borrow_mut(|runtime_executor| {
            runtime_executor.memory_read(call_id, offset, buffer)
        })
    }
}

/// Returns a default runtime executor.
pub fn default_runtime_executor() -> impl RuntimeExecutor {
    ThreadLocalExecutor {}
}

pub struct RuntimeFactoryExecutor {
    /// A module factory
    pub module_factory: ModuleFactory,
    /// Suspended runtimes keyed by per-transaction call identifier.
    pub recoverable_runtimes: HashMap<u32, ExecutionMode>,
    /// An import linker
    pub import_linker: Arc<ImportLinker>,
    /// Monotonically increasing counter for assigning call identifiers.
    pub transaction_call_id_counter: u32,
    /// Ceiling on linear memory held simultaneously by all live frames of one transaction.
    ///
    /// Defaults to [`MAX_IN_FLIGHT_MEMORY_BYTES`]; overridable so tests can exercise the limit
    /// without allocating gigabytes.
    pub max_in_flight_memory_bytes: u64,
}

impl RuntimeFactoryExecutor {
    pub fn new(import_linker: Arc<ImportLinker>) -> Self {
        Self {
            module_factory: ModuleFactory::new(),
            recoverable_runtimes: HashMap::new(),
            import_linker,
            transaction_call_id_counter: 1,
            max_in_flight_memory_bytes: MAX_IN_FLIGHT_MEMORY_BYTES,
        }
    }

    /// Returns the linear memory held by every frame of this transaction that is currently
    /// suspended awaiting resumption.
    ///
    /// This is derived from `recoverable_runtimes` on demand rather than tracked incrementally,
    /// so it cannot drift out of sync with the frames that are actually alive. The map holds
    /// every ancestor of the frame being created, which is precisely the set whose memory is
    /// resident at the same time.
    pub fn in_flight_memory_bytes(&self) -> u64 {
        self.recoverable_runtimes
            .values()
            .map(|runtime| runtime.frame_memory_size_bytes() as u64)
            .sum()
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

        // Get current call_id before incrementing
        let call_id = self.transaction_call_id_counter;

        // Check if call_id would overflow i32 when cast (positive exit codes are reserved for call_id)
        if call_id > i32::MAX as u32 {
            return ExecutionResult {
                exit_code: ExitCode::UnknownError.into_i32(),
                fuel_consumed: interruption.fuel_consumed,
                fuel_refunded: interruption.fuel_refunded,
                output: vec![],
                return_data: vec![],
            };
        }

        // Increment counter for next call (safe since call_id <= i32::MAX < u32::MAX)
        self.transaction_call_id_counter += 1;
        let prev = self.recoverable_runtimes.insert(call_id, runtime);
        debug_assert!(prev.is_none());
        metrics::set_recoverable_runtimes(self.recoverable_runtimes.len());

        ExecutionResult {
            // We return `call_id` as exit code (it's safe because exit code can't be positive)
            exit_code: call_id as i32,
            // Forward info about consumed and refunded fuel (during the call)
            fuel_consumed: interruption.fuel_consumed,
            fuel_refunded: interruption.fuel_refunded,
            // The output we map into return data
            output: interruption.return_data,
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
        // It's applied for execution runtimes where we don't know the final fuel consumed
        // till it's committed by Wasm runtime.
        // That is why we rewrite fuel here to check how much we've really spent based on the context information.
        if let Some(store_fuel_consumed) = fuel_consumed {
            execution_result.fuel_consumed = store_fuel_consumed;
        }

        // Fill the exit code in the execution result based on the next result:
        // - Ok - execution passed, exit code is 0 (Ok)
        // - InterruptionCalled - we don't know exit code since it's just an interruption
        // - Err - an execution trap code (halts execution)
        match next_result {
            Ok(_) => {
                // Don't write exit code here, because it's managed by host functions
            }
            Err(TrapCode::InterruptionCalled) => {
                // We don't set exit code here,
                // because exit code is used to represent identifier of call id
            }
            Err(err) => {
                execution_result.exit_code = ExitCode::from(err).into_i32();
            }
        }

        // If the next result is interruption
        if next_result == Err(TrapCode::InterruptionCalled) {
            let ExecutionResult {
                fuel_consumed,
                fuel_refunded,
                mut return_data,
                ..
            } = execution_result;
            // A case for normal interruption (not system runtime interruption), where we should
            // serialize the context we remembered inside the `exec.rs `
            // handler to pass into parent runtime.
            //
            // Safety: For system runtimes we don't save this
            match ctx.take_resumable_context_serialized() {
                Ok(Some(resumable_return_data)) => {
                    return_data = resumable_return_data;
                }
                Ok(None) => {}
                Err(exit_code) => {
                    return RuntimeResult::Result(ExecutionResult {
                        exit_code: exit_code.into_i32(),
                        fuel_consumed,
                        fuel_refunded,
                        output: vec![],
                        return_data: vec![],
                    });
                }
            }
            return RuntimeResult::Interruption(ExecutionInterruption {
                fuel_consumed,
                fuel_refunded,
                return_data,
            });
        }

        RuntimeResult::Result(execution_result)
    }
}
impl RuntimeExecutor for RuntimeFactoryExecutor {
    fn execute(
        &mut self,
        bytecode_or_hash: BytecodeOrHash,
        ctx: RuntimeContext,
    ) -> ExecutionResult {
        let timer = RuntimeTimer::start();
        let state = metrics::state_label(ctx.state);
        let system_runtime_params = match &bytecode_or_hash {
            BytecodeOrHash::Bytecode { address, hash, .. } => {
                fluentbase_types::is_execute_using_system_runtime(address)
                    .then_some((*address, *hash))
            }
            BytecodeOrHash::Hash(_) => None,
        };

        // If we have a cached module, then use it, otherwise create a new one and cache
        let module = self.module_factory.get_module_or_init(bytecode_or_hash);

        // If there is no cached store, then construct a new one (slow)
        let fuel_limit_value = ctx.fuel_limit;
        let fuel_limit = Some(fuel_limit_value);

        let mut exec_mode = if let Some((address, code_hash)) = system_runtime_params {
            let consume_fuel = fluentbase_types::is_engine_metered_precompile(&address);
            let runtime = SystemRuntime::new(
                module,
                self.import_linker.clone(),
                code_hash,
                address,
                ctx,
                consume_fuel,
            );
            ExecutionMode::System(runtime)
        } else {
            let engine = ExecutionEngine::acquire_shared();
            // We always execute untrusted contracts with rWasm VM
            let strategy = StrategyDefinition::Rwasm { engine, module };
            let runtime =
                ContractRuntime::new(strategy, self.import_linker.clone(), ctx, fuel_limit);
            // This is an extraordinary case where we fail during resource init inside the entrypoint,
            // but there is nothing we can do here rather than just return the execution error.
            //
            // By default, start sections are not allowed for users, so users can't deploy contracts that
            // cause traps inside the entrypoint.
            //
            // Ideally, it should never happen.
            if let Some(trap_code) = runtime.as_ref().err() {
                metrics::record_initialization_error(RuntimeModeLabel::Contract, state, *trap_code);
                let result = ExecutionResult {
                    exit_code: ExitCode::from(trap_code).into_i32(),
                    fuel_consumed: fuel_limit_value,
                    fuel_refunded: 0,
                    output: vec![],
                    return_data: vec![],
                };
                metrics::record_execution(RuntimeModeLabel::Contract, state, &timer, &result);
                return result;
            }
            ExecutionMode::Contract(runtime.unwrap())
        };
        let mode = runtime_mode_label(&exec_mode);

        // Bound the linear memory held simultaneously by every live frame of this transaction.
        //
        // Each suspended parent keeps its whole store alive in `recoverable_runtimes`, so a deep
        // enough call chain pins `depth * frame_size` bytes of resident memory while paying only
        // the per-frame fuel charge for it. Fuel prices a single allocation; it cannot bound the
        // sum across frames, which is what exhausts the node.
        //
        // The check runs after construction rather than before: the page count a module declares
        // is not a field on the module, it is encoded in the entrypoint bytecode, and decoding it
        // would tie this to rWasm's codegen. Measuring the frame we just built avoids that
        // entirely, and the resulting overshoot is bounded by one frame.
        let in_flight = self.in_flight_memory_bytes() + exec_mode.frame_memory_size_bytes() as u64;
        if in_flight > self.max_in_flight_memory_bytes {
            // Dropping `exec_mode` here releases the frame that pushed us over the limit.
            let result = ExecutionResult {
                exit_code: ExitCode::OutOfMemory.into_i32(),
                fuel_consumed: fuel_limit_value,
                fuel_refunded: 0,
                output: vec![],
                return_data: vec![],
            };
            metrics::record_execution(mode, state, &timer, &result);
            return result;
        }

        // Execute program
        let result = exec_mode.execute();
        let fuel_consumed = exec_mode
            .remaining_fuel()
            .zip(fuel_limit)
            .map(|(remaining_fuel, store_fuel)| store_fuel - remaining_fuel);

        let runtime_result =
            self.handle_execution_result(result, fuel_consumed, exec_mode.context_mut());
        let result = self.try_remember_runtime(runtime_result, exec_mode);
        metrics::record_execution(mode, state, &timer, &result);
        metrics::set_recoverable_runtimes(self.recoverable_runtimes.len());
        result
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
        let timer = RuntimeTimer::start();
        let Some(mut runtime) = self.recoverable_runtimes.remove(&call_id) else {
            unreachable!(
                "runtime: missing recoverable runtime for resume, this should never happen: call_id={}, fuel_consumed={}, exit_code={}",
                call_id, fuel_consumed, exit_code
            )
        };
        metrics::set_recoverable_runtimes(self.recoverable_runtimes.len());
        let (mode, state) = runtime_labels(&runtime);
        let mut fuel_remaining = runtime.remaining_fuel();
        let resume_inner = |runtime: &mut ExecutionMode| {
            // Copy return data into return data
            runtime.context_mut().execution_result.return_data = return_data.to_vec();
            if fuel16_ptr > 0 {
                let mut buffer = [0u8; 16];
                LittleEndian::write_u64(&mut buffer[..8], fuel_consumed);
                LittleEndian::write_i64(&mut buffer[8..], fuel_refunded);
                runtime.memory_write(fuel16_ptr as usize, &buffer)?;
            }
            runtime.resume(exit_code, fuel_consumed)
        };
        let result = resume_inner(&mut runtime);
        // We need to adjust the fuel limit because `fuel_consumed` should not be included into spent.
        if result != Err(TrapCode::OutOfFuel) {
            // Safety: We can safely unwrap here, because `OutOfFuel` check we did in `resume_inner` and the result is ok.
            fuel_remaining = fuel_remaining.map(|v| v.checked_sub(fuel_consumed).unwrap());
        }
        let fuel_consumed = runtime
            .remaining_fuel()
            .and_then(|remaining_fuel| Some(fuel_remaining? - remaining_fuel));
        let runtime_result =
            self.handle_execution_result(result, fuel_consumed, runtime.context_mut());
        let result = self.try_remember_runtime(runtime_result, runtime);
        metrics::record_resume(mode, state, &timer, &result);
        metrics::set_recoverable_runtimes(self.recoverable_runtimes.len());
        result
    }

    fn forget_runtime(&mut self, call_id: u32) {
        if let Some(runtime) = self.recoverable_runtimes.remove(&call_id) {
            let (mode, state) = runtime_labels(&runtime);
            metrics::record_forget_runtime(mode, state);
        }
        metrics::set_recoverable_runtimes(self.recoverable_runtimes.len());
    }

    fn warmup(&mut self, bytecode: RwasmModule, hash: B256, address: Address) {
        self.module_factory
            .get_module_or_init(BytecodeOrHash::Bytecode {
                bytecode,
                hash,
                address,
            });
    }

    fn reset_call_id_counter(&mut self) {
        // For each transaction we reset the `call_id` counter (used to track interruptions)
        self.transaction_call_id_counter = 1;
        // Clear recoverable runtimes, because they are no longer valid
        self.recoverable_runtimes.clear();
        metrics::set_recoverable_runtimes(0);
    }

    fn memory_read(
        &mut self,
        call_id: u32,
        offset: usize,
        buffer: &mut [u8],
    ) -> Result<(), TrapCode> {
        let runtime_ref = self.recoverable_runtimes.get_mut(&call_id).expect(
            "runtime: missing recoverable runtime for memory read, this should never happen",
        );
        runtime_ref.memory_read(offset, buffer)
    }
}

fn runtime_mode_label(runtime: &ExecutionMode) -> RuntimeModeLabel {
    match runtime {
        ExecutionMode::Contract(_) => RuntimeModeLabel::Contract,
        ExecutionMode::System(_) => RuntimeModeLabel::System,
    }
}

fn runtime_labels(runtime: &ExecutionMode) -> (RuntimeModeLabel, &'static str) {
    (
        runtime_mode_label(runtime),
        metrics::state_label(runtime.context().state),
    )
}

#[cfg(test)]
mod tests {
    use crate::{
        executor::{ExecutionInterruption, RuntimeExecutor, RuntimeFactoryExecutor, RuntimeResult},
        runtime::{test_contract_module_with_memory, ContractRuntime, ExecutionMode},
        RuntimeContext,
    };
    use fluentbase_types::{
        import_linker_v1_preview, Address, BytecodeOrHash, ExitCode, B256, CALL_STACK_LIMIT,
        MAX_IN_FLIGHT_MEMORY_BYTES,
    };
    use rwasm::{
        ExecutionEngine, RwasmModule, StrategyDefinition, N_BYTES_PER_MEMORY_PAGE,
        N_DEFAULT_MAX_MEMORY_PAGES,
    };

    #[test]
    fn call_id_overflow() {
        let mut executor = RuntimeFactoryExecutor::new(import_linker_v1_preview());

        // Set counter past i32::MAX to trigger overflow on the next allocation.
        executor.transaction_call_id_counter = i32::MAX as u32 + 1;

        let interruption = RuntimeResult::Interruption(ExecutionInterruption {
            fuel_consumed: 100,
            fuel_refunded: 0,
            return_data: vec![1, 2, 3],
        });

        let engine = ExecutionEngine::acquire_shared();
        let module = RwasmModule::default();
        let ctx = RuntimeContext::default();
        let strategy_runtime = ContractRuntime::new(
            StrategyDefinition::Rwasm { module, engine },
            executor.import_linker.clone(),
            ctx,
            None,
        )
        .unwrap();
        let runtime = ExecutionMode::Contract(strategy_runtime);

        let result = executor.try_remember_runtime(interruption, runtime);

        assert_eq!(result.exit_code, ExitCode::UnknownError.into_i32());
        assert_eq!(result.fuel_consumed, 100);
        assert_eq!(result.fuel_refunded, 0);
        assert!(result.output.is_empty());
    }

    /// Initial pages the Rust/Wasm toolchain emits for a contract that allocates nothing of its
    /// own; both `contracts/bn256` and `examples/greeting` compile down to exactly this.
    const TYPICAL_CONTRACT_PAGES: u64 = 17;

    fn contract_bytecode_with_memory(pages: u32) -> BytecodeOrHash {
        let module = test_contract_module_with_memory(pages);
        BytecodeOrHash::Bytecode {
            hash: B256::with_last_byte(pages as u8),
            bytecode: module,
            address: Address::ZERO,
        }
    }

    fn suspended_frame_with_memory(executor: &RuntimeFactoryExecutor, pages: u32) -> ExecutionMode {
        suspended_frame(executor, test_contract_module_with_memory(pages))
    }

    fn suspended_frame(executor: &RuntimeFactoryExecutor, module: RwasmModule) -> ExecutionMode {
        let runtime = ContractRuntime::new(
            StrategyDefinition::Rwasm {
                module,
                engine: ExecutionEngine::acquire_shared(),
            },
            executor.import_linker.clone(),
            RuntimeContext::default(),
            None,
        )
        .expect("test frame must instantiate");
        ExecutionMode::Contract(runtime)
    }

    #[test]
    fn in_flight_memory_sums_every_suspended_frame() {
        let mut executor = RuntimeFactoryExecutor::new(import_linker_v1_preview());
        assert_eq!(executor.in_flight_memory_bytes(), 0);

        // Frames stay resident while suspended, so the cost of a call chain is the sum over
        // frames, not the size of the largest one.
        for (call_id, pages) in [(1u32, 3u32), (2, 5)] {
            let frame = suspended_frame_with_memory(&executor, pages);
            executor.recoverable_runtimes.insert(call_id, frame);
        }

        assert_eq!(
            executor.in_flight_memory_bytes(),
            (3 + 5) * N_BYTES_PER_MEMORY_PAGE as u64
        );
    }

    #[test]
    fn in_flight_memory_ignores_frames_that_were_forgotten() {
        let mut executor = RuntimeFactoryExecutor::new(import_linker_v1_preview());
        let frame = suspended_frame_with_memory(&executor, 4);
        executor.recoverable_runtimes.insert(7, frame);
        assert_eq!(
            executor.in_flight_memory_bytes(),
            4 * N_BYTES_PER_MEMORY_PAGE as u64
        );

        executor.forget_runtime(7);
        assert_eq!(executor.in_flight_memory_bytes(), 0);
    }

    #[test]
    fn frame_exceeding_the_in_flight_cap_is_rejected() {
        let mut executor = RuntimeFactoryExecutor::new(import_linker_v1_preview());
        // Room for the four pages already suspended, but not for the frame about to be built.
        executor.max_in_flight_memory_bytes = 5 * N_BYTES_PER_MEMORY_PAGE as u64;
        let frame = suspended_frame_with_memory(&executor, 4);
        executor.recoverable_runtimes.insert(1, frame);

        let result = executor.execute(
            contract_bytecode_with_memory(3),
            RuntimeContext::default().with_fuel_limit(1_000_000),
        );

        assert_eq!(result.exit_code, ExitCode::OutOfMemory.into_i32());
        // The rejected frame must not stay resident.
        assert_eq!(
            executor.in_flight_memory_bytes(),
            4 * N_BYTES_PER_MEMORY_PAGE as u64
        );
    }

    #[test]
    fn frame_within_the_in_flight_cap_executes() {
        let mut executor = RuntimeFactoryExecutor::new(import_linker_v1_preview());
        executor.max_in_flight_memory_bytes = 8 * N_BYTES_PER_MEMORY_PAGE as u64;
        let frame = suspended_frame_with_memory(&executor, 4);
        executor.recoverable_runtimes.insert(1, frame);

        let result = executor.execute(
            contract_bytecode_with_memory(3),
            RuntimeContext::default().with_fuel_limit(1_000_000),
        );

        assert_ne!(result.exit_code, ExitCode::OutOfMemory.into_i32());
    }

    /// Drives a recursive call chain frame by frame, suspending each one the way a nested call
    /// does, and reports how many frames were admitted before the cap refused one.
    ///
    /// The real attack is `depth * frame_size`, so the shape reproduces at any frame size: many
    /// small frames stand in for the few huge ones that would need 64 GiB to run for real. Each
    /// admitted frame is parked in `recoverable_runtimes`, which is exactly the state a suspended
    /// parent leaves behind on the production path.
    fn run_call_chain(executor: &mut RuntimeFactoryExecutor, frame_pages: u32) -> u32 {
        let module = test_contract_module_with_memory(frame_pages);
        let bytecode = BytecodeOrHash::Bytecode {
            hash: B256::with_last_byte(frame_pages as u8),
            bytecode: module.clone(),
            address: Address::ZERO,
        };

        let mut admitted = 0u32;
        for call_id in 1..=CALL_STACK_LIMIT {
            // Generous enough to cover the initial-memory charge of even a maximum-size frame
            // (1023 pages costs 1_047_552 fuel), so fuel never masks the memory cap.
            let result = executor.execute(
                bytecode.clone(),
                RuntimeContext::default().with_fuel_limit(1_000_000_000),
            );
            if result.exit_code == ExitCode::OutOfMemory.into_i32() {
                break;
            }
            assert_eq!(
                result.exit_code, 0,
                "frame {call_id} failed for another reason"
            );
            let frame = suspended_frame(executor, module.clone());
            executor.recoverable_runtimes.insert(call_id, frame);
            admitted += 1;
        }
        admitted
    }

    #[test]
    fn full_depth_chain_of_ordinary_frames_is_admitted() {
        let mut executor = RuntimeFactoryExecutor::new(import_linker_v1_preview());
        // Scaled to the same ratio the production cap has against ordinary contracts: enough
        // headroom for every frame the call stack permits.
        executor.max_in_flight_memory_bytes =
            CALL_STACK_LIMIT as u64 * N_BYTES_PER_MEMORY_PAGE as u64;

        let admitted = run_call_chain(&mut executor, 1);

        // Depth on its own must never trip the cap — only total memory may.
        assert_eq!(admitted, CALL_STACK_LIMIT);
    }

    #[test]
    fn deep_chain_of_memory_heavy_frames_is_cut_off_long_before_full_depth() {
        let mut executor = RuntimeFactoryExecutor::new(import_linker_v1_preview());
        executor.max_in_flight_memory_bytes =
            CALL_STACK_LIMIT as u64 * N_BYTES_PER_MEMORY_PAGE as u64;

        // Same budget, same depth limit, frames eight times fatter: the chain must die at an
        // eighth of the depth rather than running to 1024 and pinning eight times the memory.
        let admitted = run_call_chain(&mut executor, 8);

        assert_eq!(admitted, CALL_STACK_LIMIT / 8);
        assert!(
            executor.in_flight_memory_bytes() <= executor.max_in_flight_memory_bytes,
            "peak memory must never exceed the cap",
        );
    }

    /// The attack at full scale: a recursion that asks for `CALL_STACK_LIMIT` frames of the
    /// largest memory a module may declare — about 64 GiB — must come away with no more than
    /// [`MAX_IN_FLIGHT_MEMORY_BYTES`].
    ///
    /// Unlike the scaled tests above, this one runs the production cap against production frame
    /// sizes, so it really does allocate ~1.5 GiB of resident memory before the cap refuses the
    /// next frame. That is the point — the cap is what stops it becoming 64 GiB — but it makes
    /// the test too memory-hungry for a default `cargo test` run on a constrained machine.
    ///
    /// Run it explicitly with:
    /// `cargo test -p fluentbase-runtime --lib recursion_demanding -- --ignored --nocapture`
    #[test]
    #[ignore = "allocates ~1.5 GiB of resident memory by design"]
    fn recursion_demanding_64_gib_is_capped_at_the_in_flight_limit() {
        let mut executor = RuntimeFactoryExecutor::new(import_linker_v1_preview());
        assert_eq!(
            executor.max_in_flight_memory_bytes, MAX_IN_FLIGHT_MEMORY_BYTES,
            "this test must exercise the production cap",
        );

        let largest_frame_pages = N_DEFAULT_MAX_MEMORY_PAGES - 1;
        let frame_bytes = largest_frame_pages as u64 * N_BYTES_PER_MEMORY_PAGE as u64;
        let demanded = CALL_STACK_LIMIT as u64 * frame_bytes;
        assert!(
            demanded > 60 * 1024 * 1024 * 1024,
            "the chain should be demanding tens of GiB, got {demanded} bytes",
        );

        let admitted = run_call_chain(&mut executor, largest_frame_pages);
        let held = executor.in_flight_memory_bytes();

        // The chain dies at the cap, not at the call-stack limit.
        assert_eq!(admitted, (MAX_IN_FLIGHT_MEMORY_BYTES / frame_bytes) as u32);
        assert!(admitted < CALL_STACK_LIMIT);
        assert!(
            held <= MAX_IN_FLIGHT_MEMORY_BYTES,
            "held {held} bytes, cap is {MAX_IN_FLIGHT_MEMORY_BYTES}",
        );
        // What the attacker actually got is a small fraction of what was asked for.
        assert!(held * 40 < demanded);
    }

    #[test]
    fn cap_admits_full_depth_recursion_of_ordinary_contracts() {
        // Compatibility floor: nothing a normal contract can do today may start failing. The
        // deepest legitimate chain is `CALL_STACK_LIMIT` frames of a default-sized contract.
        let worst_legitimate =
            CALL_STACK_LIMIT as u64 * TYPICAL_CONTRACT_PAGES * N_BYTES_PER_MEMORY_PAGE as u64;

        assert!(
            MAX_IN_FLIGHT_MEMORY_BYTES > worst_legitimate,
            "cap {MAX_IN_FLIGHT_MEMORY_BYTES} would break legitimate depth-{CALL_STACK_LIMIT} \
             recursion needing {worst_legitimate} bytes",
        );
    }

    #[test]
    fn cap_bounds_frames_holding_the_largest_permitted_memory() {
        // Security ceiling: the same depth filled with maximum-memory frames must be cut off
        // far below the ~64 GiB it would otherwise reach.
        let largest_frame =
            (N_DEFAULT_MAX_MEMORY_PAGES - 1) as u64 * N_BYTES_PER_MEMORY_PAGE as u64;
        let affordable_frames = MAX_IN_FLIGHT_MEMORY_BYTES / largest_frame;

        assert!(
            affordable_frames < 32,
            "cap admits {affordable_frames} maximum-memory frames",
        );
        assert!(
            affordable_frames * largest_frame < 2 * 1024 * 1024 * 1024,
            "peak memory must stay under 2 GiB",
        );
    }
}
