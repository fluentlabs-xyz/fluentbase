use crate::RuntimeContext;
use fluentbase_rwasm::{Caller, RwasmError};
use fluentbase_types::F254;

pub struct SyscallPoseidon;

impl SyscallPoseidon {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>) -> Result<(), RwasmError> {
        let [f32s_ptr, f32s_len, output_ptr] = caller.stack_pop_n();
        let data = caller.memory_read_vec(f32s_ptr.as_usize(), f32s_len.as_usize())?;
        caller.memory_write(output_ptr.as_usize(), Self::fn_impl(&data).as_slice())?;
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
