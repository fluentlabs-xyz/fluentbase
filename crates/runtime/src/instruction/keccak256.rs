use crate::RuntimeContext;
use fluentbase_types::{keccak256, B256};
use rwasm_executor::{Caller, RwasmError};

pub struct SyscallKeccak256;

impl SyscallKeccak256 {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>) -> Result<(), RwasmError> {
        let [data_offset, data_len, output_offset] = caller.stack_pop_n();
        let data = caller.memory_read_vec(data_offset.as_usize(), data_len.as_usize())?;
        caller.memory_write(output_offset.as_usize(), Self::fn_impl(&data).as_slice())?;
        Ok(())
    }

    pub fn fn_impl(data: &[u8]) -> B256 {
        keccak256(data)
    }
}
