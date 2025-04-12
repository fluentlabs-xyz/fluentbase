use crate::{RwasmError, RwasmExecutor};
use rwasm::core::UntypedValue;

pub struct Caller<'a, T> {
    vm: &'a mut RwasmExecutor<T>,
}

impl<'a, T> Caller<'a, T> {
    pub fn new(store: &'a mut RwasmExecutor<T>) -> Self {
        Self { vm: store }
    }

    pub fn stack_push<I: Into<UntypedValue>>(&mut self, value: I) {
        self.vm.sp.push_as(value);
    }

    pub fn stack_pop(&mut self) -> UntypedValue {
        self.vm.sp.pop()
    }

    pub fn stack_pop_as<I: From<UntypedValue>>(&mut self) -> I {
        I::from(self.vm.sp.pop())
    }

    pub fn stack_pop2(&mut self) -> (UntypedValue, UntypedValue) {
        let rhs = self.vm.sp.pop();
        let lhs = self.vm.sp.pop();
        (lhs, rhs)
    }

    pub fn stack_pop2_as<I: From<UntypedValue>>(&mut self) -> (I, I) {
        let (lhs, rhs) = self.stack_pop2();
        (I::from(lhs), I::from(rhs))
    }

    pub fn stack_pop_n<const N: usize>(&mut self) -> [UntypedValue; N] {
        let mut result: [UntypedValue; N] = [UntypedValue::default(); N];
        for i in 0..N {
            result[N - i - 1] = self.vm.sp.pop();
        }
        result
    }

    pub fn memory_read(&self, offset: usize, buffer: &mut [u8]) -> Result<(), RwasmError> {
        self.vm.global_memory.read(offset, buffer)?;
        Ok(())
    }

    pub fn memory_read_fixed<const N: usize>(&self, offset: usize) -> Result<[u8; N], RwasmError> {
        let mut buffer = [0u8; N];
        self.vm.global_memory.read(offset, &mut buffer)?;
        Ok(buffer)
    }

    pub fn memory_read_vec(&self, offset: usize, length: usize) -> Result<Vec<u8>, RwasmError> {
        let mut buffer = vec![0u8; length];
        self.vm.global_memory.read(offset, &mut buffer)?;
        Ok(buffer)
    }

    pub fn memory_write(&mut self, offset: usize, buffer: &[u8]) -> Result<(), RwasmError> {
        self.vm.global_memory.write(offset, buffer)?;
        if let Some(tracer) = self.vm.tracer.as_mut() {
            tracer.memory_change(offset as u32, buffer.len() as u32, buffer);
        }
        Ok(())
    }

    pub fn vm_mut(&mut self) -> &mut RwasmExecutor<T> {
        &mut self.vm
    }

    pub fn vm(&self) -> &RwasmExecutor<T> {
        &self.vm
    }

    pub fn context_mut(&mut self) -> &mut T {
        self.vm.context_mut()
    }

    pub fn context(&self) -> &T {
        self.vm.context()
    }
}
