//! Contract execution runtime.
//!
//! This module implements the execution of user-deployed contracts
//! in the rWasm environment. It is responsible for:
//! - selecting the correct entrypoint (`main` vs. `deploy`),
//! - wiring syscalls via the runtime syscall handler,
//! - driving execution and resumption,
//! - mediating access to linear memory, fuel, and runtime context.
//!
//! `ContractRuntime` is intentionally thin: most execution semantics
//! are delegated to `StrategyDefinition` and `StrategyExecutor`.

#[cfg(feature = "guest-coverage")]
use super::guest_coverage::write_guest_coverage_profile;
use crate::{syscall_handler::runtime_syscall_handler, RuntimeContext};
#[cfg(feature = "guest-coverage")]
use fluentbase_types::STATE_GUEST_COVERAGE;
use fluentbase_types::{STATE_DEPLOY, STATE_MAIN};
use rwasm::{
    ImportLinker, StoreTr, StrategyDefinition, StrategyExecutor, TrapCode, Value,
    N_DEFAULT_MAX_MEMORY_PAGES,
};
use std::sync::Arc;

#[cfg(test)]
pub(crate) fn test_contract_module_with_memory(initial_pages: u32) -> rwasm::RwasmModule {
    let wasm = wat::parse_str(format!(
        r#"
            (module
                (memory (export "memory") {initial_pages})
                (func (export "main"))
                (func (export "deploy"))
            )
        "#
    ))
    .expect("test WAT must be valid");
    let config = rwasm::CompilationConfig::default().with_entrypoint_name("main".into());
    let (module, _) = rwasm::RwasmModule::compile(config, &wasm)
        .expect("rWasm compiler must accept the test contract module");
    module
}

/// Runtime responsible for executing a single contract invocation.
///
/// This runtime encapsulates a concrete execution `Strategy`
/// (interpreter, AOT, JIT, etc.), a typed store holding the
/// `RuntimeContext`, and the resolved entrypoint to invoke.
///
/// A single instance corresponds to one logical contract execution
/// (call or deployment).
pub struct ContractRuntime {
    /// Typed store containing linear memory, globals, fuel state,
    /// and the associated `RuntimeContext`.
    executor: StrategyExecutor<RuntimeContext>,

    /// Name of the entrypoint function to execute.
    ///
    /// Resolved at construction time based on the contract state
    /// (`main` for calls, `deploy` for deployments).
    entrypoint: &'static str,
}

impl ContractRuntime {
    /// Creates a new contract runtime instance.
    ///
    /// This constructor:
    /// - selects the appropriate entrypoint based on `ctx.state`,
    /// - creates a new rWasm store bound to the provided execution strategy,
    /// - wires the runtime syscall handler,
    /// - configures fuel metering.
    ///
    /// # Panics
    ///
    /// Panics if the contract state is neither `STATE_MAIN` nor `STATE_DEPLOY`.
    pub fn new(
        strategy: StrategyDefinition,
        import_linker: Arc<ImportLinker>,
        ctx: RuntimeContext,
        fuel_limit: Option<u64>,
    ) -> Result<Self, TrapCode> {
        let entrypoint = match ctx.state {
            STATE_MAIN => "main",
            STATE_DEPLOY => "deploy",
            _ => unreachable!(),
        };
        let executor = strategy.create_executor(
            import_linker,
            ctx,
            runtime_syscall_handler,
            fuel_limit,
            Some(N_DEFAULT_MAX_MEMORY_PAGES),
        )?;
        Ok(Self {
            executor,
            entrypoint,
        })
    }

    /// Executes the contract entrypoint.
    ///
    /// Starts execution from the resolved entrypoint (`main` or `deploy`)
    /// with no arguments and no direct return values.
    ///
    /// Any trap produced by execution is surfaced as a `TrapCode`.
    pub fn execute(&mut self) -> Result<(), TrapCode> {
        let result = self.executor.execute(self.entrypoint, &[], &mut []);
        #[cfg(feature = "guest-coverage")]
        if result.is_ok() {
            write_guest_coverage_profile(&mut self.executor, Some(STATE_GUEST_COVERAGE));
        }
        result
    }

    /// Resumes contract execution after an external interruption.
    ///
    /// This is typically called after handling a syscall or delegated
    /// execution. The provided `exit_code` is passed back into the runtime,
    /// and `fuel_consumed` is charged before resuming execution.
    pub fn resume(&mut self, exit_code: i32, fuel_consumed: u64) -> Result<(), TrapCode> {
        self.executor.try_consume_fuel(fuel_consumed)?;
        let result = self.executor.resume(&[Value::I32(exit_code)], &mut []);
        #[cfg(feature = "guest-coverage")]
        if result.is_ok() {
            write_guest_coverage_profile(&mut self.executor, Some(STATE_GUEST_COVERAGE));
        }
        result
    }

    /// Writes data into the contract linear memory.
    ///
    /// Performs bounds checking according to the underlying memory model.
    /// Out-of-bounds writes result in a trap.
    pub fn memory_write(&mut self, offset: usize, data: &[u8]) -> Result<(), TrapCode> {
        self.executor.memory_write(offset, data)
    }

    /// Reads data from the contract linear memory.
    ///
    /// Fills `buffer` with bytes starting at `offset`.
    /// Traps if the read exceeds accessible memory.
    pub fn memory_read(&mut self, offset: usize, buffer: &mut [u8]) -> Result<(), TrapCode> {
        self.executor.memory_read(offset, buffer)
    }

    /// Returns the linear memory currently allocated to this frame, in bytes.
    ///
    /// The store owns this memory for as long as the frame is alive — including while the frame
    /// sits suspended waiting to be resumed — so this is the quantity a caller must sum to bound
    /// the memory held simultaneously across a call chain.
    pub fn memory_size_bytes(&self) -> usize {
        match &self.executor {
            StrategyExecutor::Rwasm { store, .. } => store.memory_size_bytes(),
            #[allow(unreachable_patterns)]
            _ => 0,
        }
    }

    /// Returns the remaining execution fuel if fuel metering is enabled.
    ///
    /// Returns `None` if fuel accounting is disabled for this execution.
    pub fn remaining_fuel(&self) -> Option<u64> {
        self.executor.remaining_fuel()
    }

    /// Provides mutable access to the runtime context.
    ///
    /// This is the only supported way to mutate execution-scoped state
    /// such as logs, gas accounting, call depth, or environment data.
    pub fn context_mut(&mut self) -> &mut RuntimeContext {
        self.executor.data_mut()
    }

    /// Provides immutable access to the runtime context.
    ///
    /// Intended for inspection and read-only queries.
    pub fn context(&self) -> &RuntimeContext {
        self.executor.data()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::test_contract_module_with_memory;
    use fluentbase_types::import_linker_v1_preview;
    use rwasm::{
        ExecutionEngine, FuelCosts, RwasmModule, N_BYTES_PER_MEMORY_PAGE,
        N_DEFAULT_MAX_MEMORY_PAGES,
    };

    fn initial_memory_fuel(initial_memory_pages: u32) -> Result<u64, TrapCode> {
        let initial_memory_bytes = initial_memory_pages
            .checked_mul(N_BYTES_PER_MEMORY_PAGE)
            .ok_or(TrapCode::MemoryOutOfBounds)?;
        Ok(u64::from(FuelCosts::fuel_for_bytes(initial_memory_bytes)))
    }

    fn strategy(module: RwasmModule) -> StrategyDefinition {
        StrategyDefinition::Rwasm {
            engine: ExecutionEngine::acquire_shared(),
            module,
        }
    }

    #[test]
    fn rwasm_initializer_charges_initial_memory_fuel() {
        let initial_pages = N_DEFAULT_MAX_MEMORY_PAGES - 1;
        let module = test_contract_module_with_memory(initial_pages);
        let fuel_limit = 1_000_000_000u64;

        // This invokes rWasm directly rather than ContractRuntime so the test isolates the
        // compiler-generated initializer and its proportional bulk-operation fuel charge.
        let executor = strategy(module)
            .create_executor(
                import_linker_v1_preview(),
                RuntimeContext::default(),
                runtime_syscall_handler,
                Some(fuel_limit),
                Some(N_DEFAULT_MAX_MEMORY_PAGES),
            )
            .expect("sufficient fuel must cover the initial-memory charge");

        // The initializer is a synthesized bytecode segment, so it carries none of the per-block
        // ConsumeFuel that the translator injects into regular functions. The only fuel it burns
        // is the bulk-op charge that `op_memory_grow_checked` emits ahead of MemoryGrow (enabled
        // by `CompilationConfig::default().consume_fuel_for_bulk_ops`), which is exactly
        // `initial_memory_fuel`:
        //
        //   initial_pages * N_BYTES_PER_MEMORY_PAGE / MEMORY_BYTES_PER_FUEL
        //     = 1023 * 65536 / 64 = 1_047_552
        //
        // so the executor is left with 1_000_000_000 - 1_047_552 = 998_952_448.
        let memory_fuel = initial_memory_fuel(initial_pages).unwrap();
        assert_eq!(executor.remaining_fuel(), Some(fuel_limit - memory_fuel));
        #[allow(irrefutable_let_patterns)]
        let StrategyExecutor::Rwasm { store, .. } = executor
        else {
            unreachable!()
        };
        assert_eq!(
            store.memory_size_bytes(),
            initial_pages as usize * N_BYTES_PER_MEMORY_PAGE as usize
        );
    }

    #[test]
    fn meters_initial_memory_before_instantiation() {
        let initial_pages = 1;
        let memory_fuel = initial_memory_fuel(initial_pages).unwrap();
        let runtime = ContractRuntime::new(
            strategy(test_contract_module_with_memory(initial_pages)),
            import_linker_v1_preview(),
            RuntimeContext::default(),
            Some(memory_fuel + 10),
        )
        .unwrap();
        assert_eq!(runtime.remaining_fuel(), Some(10));
    }

    #[test]
    fn rejects_maximum_initial_memory_before_allocation_when_underfunded() {
        let initial_pages = N_DEFAULT_MAX_MEMORY_PAGES - 1;
        let memory_fuel = initial_memory_fuel(initial_pages).unwrap();
        let error = ContractRuntime::new(
            strategy(test_contract_module_with_memory(initial_pages)),
            import_linker_v1_preview(),
            RuntimeContext::default(),
            Some(memory_fuel - 1),
        )
        .err()
        .expect("underfunded initial memory must fail before instantiation");

        assert_eq!(error, TrapCode::OutOfFuel);
    }
}
