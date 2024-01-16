use crate::RuntimeContext;
use fluentbase_rwasm::{common::Trap, Caller};

pub struct SysKeccak256;

impl SysKeccak256 {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        data_offset: u32,
        data_len: u32,
        output_offset: u32,
    ) -> Result<(), Trap> {
        let data = caller.read_memory(data_offset, data_len);
        caller.write_memory(output_offset, &Self::fn_impl(data));
        Ok(())
    }

    pub fn fn_impl(data: &[u8]) -> [u8; 32] {
        let mut result = [0u8; 32];
        keccak_hash::write_keccak(data, &mut result);
        result
    }
}
