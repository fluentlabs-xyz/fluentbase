use crate::RuntimeContext;
use fluentbase_rwasm::{Caller, RwasmError};
use fluentbase_types::B256;

pub struct SyscallKeccak256;

impl SyscallKeccak256 {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>) -> Result<(), RwasmError> {
        let [data_offset, data_len, output_offset] = caller.stack_pop_n();
        let data = caller.memory_read_vec(data_offset.as_usize(), data_len.as_usize())?;
        caller.memory_write(output_offset.as_usize(), Self::fn_impl(&data).as_slice())?;
        Ok(())
    }

    pub fn fn_impl(data: &[u8]) -> B256 {
        let mut result = [0u8; 32];
        keccak_hash::write_keccak(data, &mut result);
        result.into()
    }
}
