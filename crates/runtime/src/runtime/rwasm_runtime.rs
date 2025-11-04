use crate::context::RuntimeContext;
use rwasm::{ExecutionEngine, RwasmModule, RwasmStore, Store, TrapCode, Value};

/// A compiled, executable runtime instance with its store and engine strategy.
pub struct RwasmRuntime {
    /// An engine for executing rWasm apps
    pub engine: ExecutionEngine,
    /// Underlying execution module.
    pub module: RwasmModule,
    /// Engine store carrying linear memory and the RuntimeContext.
    pub store: RwasmStore<RuntimeContext>,
}

impl RwasmRuntime {
    /// Creates a runtime from bytecode or code hash and initializes its store with the provided context.
    pub fn new(
        engine: ExecutionEngine,
        module: RwasmModule,
        store: RwasmStore<RuntimeContext>,
    ) -> Self {
        Self {
            engine,
            module,
            store,
        }
    }

    pub fn execute(&mut self) -> Result<(), TrapCode> {
        self.engine
            .execute(&mut self.store, &self.module, &[], &mut [])
    }

    pub fn resume(&mut self, exit_code: i32) -> Result<(), TrapCode> {
        self.engine
            .resume(&mut self.store, &[Value::I32(exit_code)], &mut [])
    }

    pub fn memory_write(&mut self, offset: usize, data: &[u8]) -> Result<(), TrapCode> {
        self.store.memory_write(offset, data)
    }

    pub fn try_consume_fuel(&mut self, fuel: u64) -> Result<(), TrapCode> {
        self.store.try_consume_fuel(fuel)
    }

    pub fn remaining_fuel(&self) -> Option<u64> {
        self.store.remaining_fuel()
    }

    pub fn context_mut<R, F: FnOnce(&mut RuntimeContext) -> R>(&mut self, func: F) -> R {
        self.store.context_mut(func)
    }

    pub fn context<R, F: FnOnce(&RuntimeContext) -> R>(&self, func: F) -> R {
        self.store.context(func)
    }
}
