//! Execution mode dispatcher for rWasm-based runtimes.
//!
//! This module defines a thin abstraction over different execution backends
//! (contract vs system runtimes). The goal is to provide a uniform interface
//! for driving execution, memory access, fuel accounting and context handling,
//! while allowing the underlying runtime to differ.
//!
//! From the executor’s point of view, `ExecutionMode` behaves like a tagged
//! union with identical operational semantics.

use crate::RuntimeContext;
use rwasm::TrapCode;

mod contract_runtime;
pub use contract_runtime::ContractRuntime;

mod system_runtime;
pub use system_runtime::SystemRuntime;

/// Represents the active execution mode.
///
/// Execution can happen either in a *contract* context (user-deployed code)
/// or in a *system* context (privileged runtimes, precompiles, delegated VMs).
///
/// Both modes expose the same operational surface:
/// execution control, memory access, fuel accounting, and context access.
/// This enum acts as a dynamic dispatcher between them.
pub enum ExecutionMode {
    /// Contract-level execution runtime.
    ///
    /// Used for regular smart contracts executed in a constrained,
    /// user-controlled environment.
    Contract(ContractRuntime),

    /// System-level execution runtime.
    ///
    /// Used for privileged or delegated runtimes (e.g., EVM/SVM/Wasm system
    /// runtimes) that may have different invariants or capabilities.
    System(SystemRuntime),
}

impl ExecutionMode {
    /// Executes the runtime until it either finishes or traps.
    ///
    /// This is the initial entry point for execution. Any runtime-specific
    /// exit conditions (normal return, trap, host call) are translated
    /// into a `TrapCode` if execution cannot continue.
    pub fn execute(&mut self) -> Result<(), TrapCode> {
        match self {
            ExecutionMode::Contract(runtime) => runtime.execute(),
            ExecutionMode::System(runtime) => runtime.execute(),
        }
    }

    /// Resumes execution after an external interruption.
    ///
    /// Typically used after handling a host call or delegated execution,
    /// passing back the exit code and the amount of fuel consumed by
    /// the external component.
    pub fn resume(&mut self, exit_code: i32, fuel_consumed: u64) -> Result<(), TrapCode> {
        match self {
            ExecutionMode::Contract(runtime) => runtime.resume(exit_code, fuel_consumed),
            ExecutionMode::System(runtime) => runtime.resume(exit_code, fuel_consumed),
        }
    }

    /// Writes data into the runtime linear memory.
    ///
    /// The write is bounds-checked according to the runtime’s memory model.
    /// Any violation (out-of-bounds, unmapped memory, etc.) results in a trap.
    pub fn memory_write(&mut self, offset: usize, data: &[u8]) -> Result<(), TrapCode> {
        match self {
            ExecutionMode::Contract(runtime) => runtime.memory_write(offset, data),
            ExecutionMode::System(runtime) => runtime.memory_write(offset, data),
        }
    }

    /// Reads data from the runtime linear memory.
    ///
    /// The provided buffer is filled with bytes starting at `offset`.
    /// Traps if the read exceeds the accessible memory range.
    pub fn memory_read(&mut self, offset: usize, buffer: &mut [u8]) -> Result<(), TrapCode> {
        match self {
            ExecutionMode::Contract(runtime) => runtime.memory_read(offset, buffer),
            ExecutionMode::System(runtime) => runtime.memory_read(offset, buffer),
        }
    }

    /// Returns the remaining execution fuel, if fuel metering is enabled.
    ///
    /// Some runtimes may choose not to expose fuel accounting; in that case
    /// `None` is returned.
    pub fn remaining_fuel(&self) -> Option<u64> {
        match self {
            ExecutionMode::Contract(runtime) => runtime.remaining_fuel(),
            ExecutionMode::System(runtime) => runtime.remaining_fuel(),
        }
    }

    /// Provides mutable access to the underlying `RuntimeContext`.
    ///
    /// This is the only sanctioned way to mutate execution context shared
    /// between the executor and the runtime (e.g., call depth, logs, gas
    /// accounting, environment data).
    ///
    /// The closure-based API prevents leaking mutable references outside
    /// the execution boundary.
    pub fn context_mut(&mut self) -> &mut RuntimeContext {
        match self {
            ExecutionMode::Contract(runtime) => runtime.context_mut(),
            ExecutionMode::System(runtime) => runtime.context_mut(),
        }
    }

    /// Provides immutable access to the underlying `RuntimeContext`.
    ///
    /// Intended for inspection and read-only queries without allowing
    /// mutation of consensus-critical state.
    pub fn context(&self) -> &RuntimeContext {
        match self {
            ExecutionMode::Contract(runtime) => runtime.context(),
            ExecutionMode::System(runtime) => runtime.context(),
        }
    }
}
