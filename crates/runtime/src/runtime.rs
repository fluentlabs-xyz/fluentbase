use crate::RuntimeContext;
use rwasm::TrapCode;

mod rwasm_runtime;
pub use rwasm_runtime::*;
mod strategy_runtime;
pub use strategy_runtime::*;
#[cfg(feature = "wasmtime")]
mod wasmtime_runtime;
#[cfg(feature = "wasmtime")]
pub use wasmtime_runtime::*;

pub enum ExecutionMode {
    // TODO(dmitry123): Only this runtime is used for now, other remains for future optimizations.
    Strategy(StrategyRuntime),
    Rwasm(RwasmRuntime),
    #[cfg(feature = "wasmtime")]
    Wasmtime(WasmtimeRuntime),
}

impl ExecutionMode {
    pub fn execute(&mut self) -> Result<(), TrapCode> {
        match self {
            ExecutionMode::Strategy(runtime) => runtime.execute(),
            ExecutionMode::Rwasm(runtime) => runtime.execute(),
            #[cfg(feature = "wasmtime")]
            ExecutionMode::Wasmtime(runtime) => runtime.execute(),
        }
    }

    pub fn resume(&mut self, exit_code: i32) -> Result<(), TrapCode> {
        match self {
            ExecutionMode::Strategy(runtime) => runtime.resume(exit_code),
            ExecutionMode::Rwasm(runtime) => runtime.resume(exit_code),
            #[cfg(feature = "wasmtime")]
            ExecutionMode::Wasmtime(runtime) => runtime.resume(exit_code),
        }
    }

    pub fn try_consume_fuel(&mut self, fuel: u64) -> Result<(), TrapCode> {
        match self {
            ExecutionMode::Strategy(runtime) => runtime.try_consume_fuel(fuel),
            ExecutionMode::Rwasm(runtime) => runtime.try_consume_fuel(fuel),
            #[cfg(feature = "wasmtime")]
            ExecutionMode::Wasmtime(runtime) => runtime.try_consume_fuel(fuel),
        }
    }

    pub fn memory_write(&mut self, offset: usize, data: &[u8]) -> Result<(), TrapCode> {
        match self {
            ExecutionMode::Strategy(runtime) => runtime.memory_write(offset, data),
            ExecutionMode::Rwasm(runtime) => runtime.memory_write(offset, data),
            #[cfg(feature = "wasmtime")]
            ExecutionMode::Wasmtime(runtime) => runtime.memory_write(offset, data),
        }
    }

    pub fn remaining_fuel(&self) -> Option<u64> {
        match self {
            ExecutionMode::Strategy(runtime) => runtime.remaining_fuel(),
            ExecutionMode::Rwasm(runtime) => runtime.remaining_fuel(),
            #[cfg(feature = "wasmtime")]
            ExecutionMode::Wasmtime(runtime) => runtime.remaining_fuel(),
        }
    }

    pub fn context_mut<R, F: FnOnce(&mut RuntimeContext) -> R>(&mut self, func: F) -> R {
        match self {
            ExecutionMode::Strategy(runtime) => runtime.context_mut(func),
            ExecutionMode::Rwasm(runtime) => runtime.context_mut(func),
            #[cfg(feature = "wasmtime")]
            ExecutionMode::Wasmtime(runtime) => runtime.context_mut(func),
        }
    }

    pub fn context<R, F: FnOnce(&RuntimeContext) -> R>(&self, func: F) -> R {
        match self {
            ExecutionMode::Strategy(runtime) => runtime.context(func),
            ExecutionMode::Rwasm(runtime) => runtime.context(func),
            #[cfg(feature = "wasmtime")]
            ExecutionMode::Wasmtime(runtime) => runtime.context(func),
        }
    }
}
