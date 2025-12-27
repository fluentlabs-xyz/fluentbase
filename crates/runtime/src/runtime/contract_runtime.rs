//! Contract execution runtime.
//!
//! This module implements execution of user-deployed contracts
//! in the rWasm environment. It is responsible for:
//! - selecting the correct entrypoint (`main` vs `deploy`),
//! - wiring syscalls via the runtime syscall handler,
//! - driving execution and resumption,
//! - mediating access to linear memory, fuel and runtime context.
//!
//! `ContractRuntime` is intentionally thin: most execution semantics
//! are delegated to `Strategy` and `TypedStore`.

use crate::{syscall_handler::runtime_syscall_handler, RuntimeContext};
use fluentbase_types::{STATE_DEPLOY, STATE_MAIN};
use rwasm::{FuelConfig, ImportLinker, Store, Strategy, TrapCode, TypedStore, Value};
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
    /// Execution strategy used to run the contract code.
    ///
    /// The strategy defines how rWasm bytecode is executed
    /// (e.g. interpreter vs compiled backend).
    strategy: Strategy,

    /// Typed store containing linear memory, globals, fuel state
    /// and the associated `RuntimeContext`.
    store: TypedStore<RuntimeContext>,

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
        strategy: Strategy,
        import_linker: Arc<ImportLinker>,
        ctx: RuntimeContext,
        fuel_config: FuelConfig,
    ) -> Self {
        let entrypoint = match ctx.state {
            STATE_MAIN => "main",
            STATE_DEPLOY => "deploy",
            _ => unreachable!(),
        };

        let store = strategy.create_store(import_linker, ctx, runtime_syscall_handler, fuel_config);

        Self {
            strategy,
            store,
            entrypoint,
        }
    }

    /// Executes the contract entrypoint.
    ///
    /// Starts execution from the resolved entrypoint (`main` or `deploy`)
    /// with no arguments and no direct return values.
    ///
    /// Any trap produced by execution is surfaced as a `TrapCode`.
    pub fn execute(&mut self) -> Result<(), TrapCode> {
        self.strategy
            .execute(&mut self.store, self.entrypoint, &[], &mut [])
    }

    /// Resumes contract execution after an external interruption.
    ///
    /// This is typically called after handling a syscall or delegated
    /// execution. The provided `exit_code` is passed back into the runtime,
    /// and `fuel_consumed` is charged before resuming execution.
    pub fn resume(&mut self, exit_code: i32, fuel_consumed: u64) -> Result<(), TrapCode> {
        self.store.try_consume_fuel(fuel_consumed)?;
        self.strategy
            .resume(&mut self.store, &[Value::I32(exit_code)], &mut [])
    }

    /// Writes data into the contract linear memory.
    ///
    /// Performs bounds checking according to the underlying memory model.
    /// Out-of-bounds writes result in a trap.
    pub fn memory_write(&mut self, offset: usize, data: &[u8]) -> Result<(), TrapCode> {
        self.store.memory_write(offset, data)
    }

    /// Reads data from the contract linear memory.
    ///
    /// Fills `buffer` with bytes starting at `offset`.
    /// Traps if the read exceeds accessible memory.
    pub fn memory_read(&mut self, offset: usize, buffer: &mut [u8]) -> Result<(), TrapCode> {
        self.store.memory_read(offset, buffer)
    }

    /// Returns the remaining execution fuel, if fuel metering is enabled.
    ///
    /// Returns `None` if fuel accounting is disabled for this execution.
    pub fn remaining_fuel(&self) -> Option<u64> {
        self.store.remaining_fuel()
    }

    /// Provides mutable access to the runtime context.
    ///
    /// This is the only supported way to mutate execution-scoped state
    /// such as logs, gas accounting, call depth or environment data.
    pub fn context_mut<R, F: FnOnce(&mut RuntimeContext) -> R>(&mut self, func: F) -> R {
        self.store.context_mut(func)
    }

    /// Provides immutable access to the runtime context.
    ///
    /// Intended for inspection and read-only queries.
    pub fn context<R, F: FnOnce(&RuntimeContext) -> R>(&self, func: F) -> R {
        self.store.context(func)
    }
}
