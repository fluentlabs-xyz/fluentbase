use crate::{syscall_handler::runtime_syscall_handler, RuntimeContext};
use fluentbase_types::{STATE_DEPLOY, STATE_MAIN};
use rwasm::{FuelConfig, ImportLinker, Store, Strategy, TrapCode, TypedStore, Value};
use std::sync::Arc;

pub struct RwasmRuntime {
    strategy: Strategy,
    store: TypedStore<RuntimeContext>,
    entrypoint: &'static str,
}

impl RwasmRuntime {
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

    pub fn execute(&mut self) -> Result<(), TrapCode> {
        self.strategy
            .execute(&mut self.store, self.entrypoint, &[], &mut [])
    }

    pub fn resume(&mut self, exit_code: i32) -> Result<(), TrapCode> {
        self.strategy
            .resume(&mut self.store, &[Value::I32(exit_code)], &mut [])
    }

    pub fn memory_write(&mut self, offset: usize, data: &[u8]) -> Result<(), TrapCode> {
        self.store.memory_write(offset, data)
    }

    pub fn memory_read(&mut self, offset: usize, buffer: &mut [u8]) -> Result<(), TrapCode> {
        self.store.memory_read(offset, buffer)
    }

    pub fn try_consume_fuel(&mut self, delta: u64) -> Result<(), TrapCode> {
        self.store.try_consume_fuel(delta)
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
