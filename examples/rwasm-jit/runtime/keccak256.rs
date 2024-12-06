use crate::RuntimeContext;
use fluentbase_sdk::B256;
use rwasm::{core::Trap, Caller};

pub struct SyscallKeccak256;

impl SyscallKeccak256 {
    pub fn fn_handler(
        mut caller: Caller<'_, RuntimeContext>,
        data_offset: u32,
        data_len: u32,
        output_offset: u32,
    ) -> Result<(), Trap> {
        let data = caller.read_memory(data_offset, data_len)?;
        caller.write_memory(output_offset, Self::fn_impl(data).as_slice())?;
        Ok(())
    }

    pub fn fn_impl(data: &[u8]) -> B256 {
        let mut result = [0u8; 32];
        keccak_hash::write_keccak(data, &mut result);
        result.into()
    }
}
