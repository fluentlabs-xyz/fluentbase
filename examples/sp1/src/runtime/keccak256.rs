use crate::RuntimeContext;
use fluentbase_types::B256;
use rwasm::{core::Trap, Caller};
use tiny_keccak::{Hasher, Keccak};
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
        let mut hasher = Keccak::v256();
        hasher.update(&data);
        let mut output = [0u8; 32];
        hasher.finalize(&mut output);
        output.into()
    }
}
