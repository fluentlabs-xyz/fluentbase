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
    ImportLinker, StoreTr, StrategyDefinition, StrategyExecutor, TrapCode, Value,
    N_DEFAULT_MAX_MEMORY_PAGES,
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
