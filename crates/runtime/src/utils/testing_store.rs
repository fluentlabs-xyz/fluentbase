use crate::RuntimeContext;
use rwasm::{Store, TrapCode};

pub(crate) struct TestingStore {
    pub(crate) ctx: RuntimeContext,
    pub(crate) memory: Vec<u8>,
    pub(crate) fuel_consumed: u64,
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

    fn context_mut<R, F: FnOnce(&mut RuntimeContext) -> R>(&mut self, func: F) -> R {
        func(&mut self.ctx)
    }

    fn context<R, F: FnOnce(&RuntimeContext) -> R>(&self, func: F) -> R {
        func(&self.ctx)
    }

    fn try_consume_fuel(&mut self, delta: u64) -> Result<(), TrapCode> {
        if !self.ctx.disable_fuel && self.fuel_consumed + delta > self.ctx.fuel_limit {
            return Err(TrapCode::OutOfFuel);
        }
        self.fuel_consumed += delta;
        Ok(())
    }

    fn remaining_fuel(&self) -> Option<u64> {
        if !self.ctx.disable_fuel {
            Some(self.ctx.fuel_limit - self.fuel_consumed)
        } else {
            None
        }
    }
}
