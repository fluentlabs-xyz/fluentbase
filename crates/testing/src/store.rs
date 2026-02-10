use fluentbase_runtime::RuntimeContext;
use rwasm::{Store, TrapCode};

pub struct TestingStore {
    pub ctx: RuntimeContext,
    pub memory: Vec<u8>,
    pub fuel_consumed: u64,
}

impl Store<RuntimeContext> for TestingStore {
    fn memory_read(&mut self, offset: usize, buffer: &mut [u8]) -> Result<(), TrapCode> {
        let data = self
            .memory
            .get(offset..(offset + buffer.len()))
            .ok_or(TrapCode::MemoryOutOfBounds)?;
        buffer.copy_from_slice(data);
        Ok(())
    }

    fn memory_write(&mut self, offset: usize, buffer: &[u8]) -> Result<(), TrapCode> {
        self.memory
            .get_mut(offset..(offset + buffer.len()))
            .ok_or(TrapCode::MemoryOutOfBounds)?
            .copy_from_slice(buffer);
        Ok(())
    }

    fn data_mut(&mut self) -> &mut RuntimeContext {
        &mut self.ctx
    }

    fn data(&self) -> &RuntimeContext {
        &self.ctx
    }

    fn try_consume_fuel(&mut self, delta: u64) -> Result<(), TrapCode> {
        if self.fuel_consumed + delta > self.ctx.fuel_limit {
            return Err(TrapCode::OutOfFuel);
        }
        self.fuel_consumed += delta;
        Ok(())
    }

    fn remaining_fuel(&self) -> Option<u64> {
        Some(self.ctx.fuel_limit - self.fuel_consumed)
    }
}
