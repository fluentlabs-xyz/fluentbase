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

use crate::{syscall_handler::runtime_syscall_handler, RuntimeContext};
use fluentbase_types::{STATE_DEPLOY, STATE_MAIN};
use rwasm::{
    FuelCosts, ImportLinker, StoreTr, StrategyDefinition, StrategyExecutor, TrapCode, Value,
    N_BYTES_PER_MEMORY_PAGE, N_DEFAULT_MAX_MEMORY_PAGES,
};
use std::sync::Arc;

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
        self.executor.execute(self.entrypoint, &[], &mut [])
    }

    /// Resumes contract execution after an external interruption.
    ///
    /// This is typically called after handling a syscall or delegated
    /// execution. The provided `exit_code` is passed back into the runtime,
    /// and `fuel_consumed` is charged before resuming execution.
    pub fn resume(&mut self, exit_code: i32, fuel_consumed: u64) -> Result<(), TrapCode> {
        self.executor.try_consume_fuel(fuel_consumed)?;
        self.executor.resume(&[Value::I32(exit_code)], &mut [])
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

fn initial_memory_fuel(initial_memory_pages: u32) -> Result<u64, TrapCode> {
    let initial_memory_bytes = initial_memory_pages
        .checked_mul(N_BYTES_PER_MEMORY_PAGE)
        .ok_or(TrapCode::MemoryOutOfBounds)?;
    Ok(u64::from(FuelCosts::fuel_for_bytes(initial_memory_bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_types::import_linker_v1_preview;
    use rwasm::{ExecutionEngine, InstructionSet, RwasmModule, RwasmModuleInner};

    fn module_with_initial_memory(initial_pages: u32) -> RwasmModule {
        let mut code_section = InstructionSet::new();
        code_section.op_stack_check(5);
        code_section.op_i32_const(initial_pages);
        code_section.op_memory_grow();
        code_section.op_drop();
        code_section.op_return();
        let source_pc = code_section.len() as u32;
        RwasmModuleInner {
            code_section,
            data_section: vec![],
            elem_section: vec![],
            hint_section: vec![],
            source_pc,
        }
        .into()
    }

    fn strategy(module: RwasmModule) -> StrategyDefinition {
        StrategyDefinition::Rwasm {
            engine: ExecutionEngine::acquire_shared(),
            module,
        }
    }

    fn compile_wasm_with_initial_memory(initial_pages: u32) -> RwasmModule {
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
        let (module, _) = RwasmModule::compile(config, &wasm)
            .expect("rWasm compiler must accept the maximum valid initial memory");
        module
    }

    #[test]
    fn rwasm_initializer_allocates_maximum_memory_without_a_fuel_charge() {
        let initial_pages = N_DEFAULT_MAX_MEMORY_PAGES - 1;
        let module = compile_wasm_with_initial_memory(initial_pages);
        let fuel_limit = 1_000_000_000u64;

        // This invokes rWasm directly rather than ContractRuntime so the test isolates the
        // compiler-generated initializer. It must fail with OutOfFuel once that initializer
        // emits a proportional ConsumeFuel before MemoryGrow.
        let executor = strategy(module)
            .create_executor(
                import_linker_v1_preview(),
                RuntimeContext::default(),
                runtime_syscall_handler,
                Some(fuel_limit),
                Some(N_DEFAULT_MAX_MEMORY_PAGES),
            )
            .expect("the current unmetered initializer allocates before checking fuel");

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
        let StrategyExecutor::Rwasm { store, .. } = executor;
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
            strategy(module_with_initial_memory(initial_pages)),
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
            strategy(module_with_initial_memory(initial_pages)),
            import_linker_v1_preview(),
            RuntimeContext::default(),
            Some(memory_fuel - 1),
        )
        .err()
        .expect("underfunded initial memory must fail before instantiation");

        assert_eq!(error, TrapCode::OutOfFuel);
    }

    #[test]
    fn rejects_initial_memory_above_frame_budget_before_allocation() {
        let error = ContractRuntime::new(
            strategy(module_with_initial_memory(2)),
            import_linker_v1_preview(),
            RuntimeContext::default(),
            None,
        )
        .err()
        .expect("initial memory above the frame budget must fail before instantiation");

        assert_eq!(error, TrapCode::MemoryOutOfBounds);
    }
}
