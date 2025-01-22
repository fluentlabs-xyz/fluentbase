use crate::RuntimeContext;
use fluentbase_rwasm::{Caller, RwasmError};

pub(crate) struct SyscallSha256Extend;

impl SyscallSha256Extend {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>) -> Result<(), RwasmError> {
        let w_ptr: u32 = caller.stack_pop_as();

        for i in 16..64 {
            // Read w[i-15].
            let w_i_minus_15 = u32::from_be_bytes(
                caller
                    .read_memory(w_ptr + (i - 15) * 4, 4)?
                    .try_into()
                    .unwrap(),
            );

            // Compute `s0`.
            let s0 =
                w_i_minus_15.rotate_right(7) ^ w_i_minus_15.rotate_right(18) ^ (w_i_minus_15 >> 3);

            // Read w[i-2].
            let w_i_minus_2 = u32::from_be_bytes(
                caller
                    .read_memory(w_ptr + (i - 2) * 4, 4)?
                    .try_into()
                    .unwrap(),
            );

            // Compute `s1`.
            let s1 =
                w_i_minus_2.rotate_right(17) ^ w_i_minus_2.rotate_right(19) ^ (w_i_minus_2 >> 10);

            // Read w[i-16].
            let w_i_minus_16 = u32::from_be_bytes(
                caller
                    .read_memory(w_ptr + (i - 16) * 4, 4)?
                    .try_into()
                    .unwrap(),
            );

            // Read w[i-7].
            let w_i_minus_7 = u32::from_be_bytes(
                caller
                    .read_memory(w_ptr + (i - 7) * 4, 4)?
                    .try_into()
                    .unwrap(),
            );

            // Compute `w_i`.
            let w_i = s1
                .wrapping_add(w_i_minus_16)
                .wrapping_add(s0)
                .wrapping_add(w_i_minus_7);

            caller.write_memory(w_ptr + i * 4, &w_i.to_be_bytes())?;
        }

        Ok(())
    }
}
