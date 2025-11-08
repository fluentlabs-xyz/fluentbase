use crate::RuntimeContext;
use rwasm::TrapCode;

mod rwasm_runtime;
pub use rwasm_runtime::*;
mod system_runtime;
pub use system_runtime::*;

pub enum ExecutionMode {
    // TODO(dmitry123): Only this runtime is used for now, other remains for future optimizations.
    Rwasm(RwasmRuntime),
    System(SystemRuntime),
}

impl ExecutionMode {
    pub fn execute(&mut self) -> Result<(), TrapCode> {
        match self {
            ExecutionMode::Rwasm(runtime) => runtime.execute(),
            ExecutionMode::System(runtime) => runtime.execute(false),
        }
    }

    pub fn resume(&mut self, exit_code: i32) -> Result<(), TrapCode> {
        match self {
            ExecutionMode::Rwasm(runtime) => runtime.resume(exit_code),
            ExecutionMode::System(runtime) => runtime.resume(exit_code),
        }
    }

    pub fn try_consume_fuel(&mut self, fuel: u64) -> Result<(), TrapCode> {
        match self {
            ExecutionMode::Rwasm(runtime) => runtime.try_consume_fuel(fuel),
            ExecutionMode::System(runtime) => runtime.try_consume_fuel(fuel),
        }
    }

    pub fn memory_write(&mut self, offset: usize, data: &[u8]) -> Result<(), TrapCode> {
        match self {
            ExecutionMode::Rwasm(runtime) => runtime.memory_write(offset, data),
            ExecutionMode::System(runtime) => runtime.memory_write(offset, data),
        }
    }

    pub fn memory_read(&mut self, offset: usize, buffer: &mut [u8]) -> Result<(), TrapCode> {
        match self {
            ExecutionMode::Rwasm(runtime) => runtime.memory_read(offset, buffer),
            ExecutionMode::System(runtime) => runtime.memory_read(offset, buffer),
        }
    }

    pub fn remaining_fuel(&self) -> Option<u64> {
        match self {
            ExecutionMode::Rwasm(runtime) => runtime.remaining_fuel(),
            ExecutionMode::System(runtime) => runtime.remaining_fuel(),
        }
    }

    pub fn context_mut<R, F: FnOnce(&mut RuntimeContext) -> R>(&mut self, func: F) -> R {
        match self {
            ExecutionMode::Rwasm(runtime) => runtime.context_mut(func),
            ExecutionMode::System(runtime) => runtime.context_mut(func),
        }
    }

    pub fn context<R, F: FnOnce(&RuntimeContext) -> R>(&self, func: F) -> R {
        match self {
            ExecutionMode::Rwasm(runtime) => runtime.context(func),
            ExecutionMode::System(runtime) => runtime.context(func),
        }
    }
}
