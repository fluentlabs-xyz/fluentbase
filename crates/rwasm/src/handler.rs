use crate::{RwasmError, SyscallHandler};
use rwasm::{
    core::{TrapCode, UntypedValue},
    engine::stack::ValueStackPtr,
    memory::MemoryEntity,
};
use tiny_keccak::Hasher;

#[derive(Default)]
pub struct SimpleCallHandler {
    pub input: Vec<u8>,
    pub state: u32,
    pub output: Vec<u8>,
}

impl SimpleCallHandler {
    fn fn_proc_exit(
        &self,
        sp: &mut ValueStackPtr,
        _global_memory: &mut MemoryEntity,
    ) -> Result<(), RwasmError> {
        let exit_code = sp.pop();
        Err(RwasmError::ExecutionHalted(exit_code.as_i32()))
    }

    fn fn_get_state(
        &self,
        sp: &mut ValueStackPtr,
        _global_memory: &mut MemoryEntity,
    ) -> Result<(), RwasmError> {
        sp.push(UntypedValue::from(self.state));
        Ok(())
    }

    fn fn_read_input(
        &self,
        sp: &mut ValueStackPtr,
        global_memory: &mut MemoryEntity,
    ) -> Result<(), RwasmError> {
        let (target, offset, length) = sp.pop3();
        let input = self
            .input
            .get(offset.as_usize()..(offset.as_usize() + length.as_usize()))
            .ok_or(RwasmError::InputOutOfBounds)?;
        global_memory.write(target.as_usize(), input)?;
        Ok(())
    }

    fn fn_input_size(
        &self,
        sp: &mut ValueStackPtr,
        _global_memory: &mut MemoryEntity,
    ) -> Result<(), RwasmError> {
        sp.push(UntypedValue::from(self.input.len() as i32));
        Ok(())
    }

    fn fn_write_output(
        &mut self,
        sp: &mut ValueStackPtr,
        global_memory: &mut MemoryEntity,
    ) -> Result<(), RwasmError> {
        let (offset, length) = sp.pop2();
        let buffer = global_memory
            .data()
            .get(offset.as_usize()..(offset.as_usize() + length.as_usize()))
            .ok_or(TrapCode::MemoryOutOfBounds)?;
        self.output.extend_from_slice(buffer);
        Ok(())
    }

    fn fn_exec(
        &mut self,
        sp: &mut ValueStackPtr,
        global_memory: &mut MemoryEntity,
    ) -> Result<(), RwasmError> {
        let hash32_ptr = sp.pop();
        let (input_ptr, input_len) = sp.pop2();
        let fuel8_ptr = sp.pop();
        let state = sp.pop();
        // exit code
        sp.push(0.into());
        Ok(())
    }

    fn fn_keccak256(
        &mut self,
        sp: &mut ValueStackPtr,
        global_memory: &mut MemoryEntity,
    ) -> Result<(), RwasmError> {
        let (data_offset, data_len, output32_offset) = sp.pop3();
        let mut buffer = vec![0u8; data_len.as_usize()];
        global_memory.read(data_offset.as_usize(), &mut buffer)?;
        let mut hash = tiny_keccak::Keccak::v256();
        hash.update(&buffer);
        let mut output = [0u8; 32];
        hash.finalize(&mut output);
        global_memory.write(output32_offset.as_usize(), &output)?;
        Ok(())
    }
}

impl SyscallHandler for SimpleCallHandler {
    fn call_function(
        &mut self,
        func_idx: u32,
        sp: &mut ValueStackPtr,
        global_memory: &mut MemoryEntity,
    ) -> Result<(), RwasmError> {
        match func_idx {
            0x001 => self.fn_proc_exit(sp, global_memory),
            0x002 => self.fn_get_state(sp, global_memory),
            0x003 => self.fn_read_input(sp, global_memory),
            0x004 => self.fn_input_size(sp, global_memory),
            0x005 => self.fn_write_output(sp, global_memory),
            0x009 => self.fn_exec(sp, global_memory),
            0x101 => self.fn_keccak256(sp, global_memory),
            _ => unreachable!("rwasm: unknown function ({})", func_idx),
        }
    }
}
