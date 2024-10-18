use crate::RuntimeContext;
use fluentbase_types::F254;
use rwasm::{core::Trap, Caller};

pub struct SyscallPoseidon;

impl SyscallPoseidon {
    pub fn fn_handler(
        mut caller: Caller<'_, RuntimeContext>,
        f32s_offset: u32,
        f32s_len: u32,
        output_offset: u32,
    ) -> Result<(), Trap> {
        let data = caller.read_memory(f32s_offset, f32s_len)?;
        caller.write_memory(output_offset, Self::fn_impl(data).as_slice())?;
        Ok(())
    }

    #[cfg(feature = "std")]
    pub fn fn_impl(data: &[u8]) -> F254 {
        use fluentbase_poseidon::poseidon_hash;
        poseidon_hash(data).into()
    }

    #[cfg(not(feature = "std"))]
    pub fn fn_impl(_data: &[u8]) -> F254 {
        unreachable!("poseidon is not supported in `no_std` mode")
    }
}
