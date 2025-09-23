use crate::RuntimeContext;
use rwasm::{Store, TrapCode, TypedCaller, Value};

pub struct SyscallSha256Extend;

impl SyscallSha256Extend {
    pub fn fn_handler(
        caller: &mut TypedCaller<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let w_ptr: u32 = params[0].i32().unwrap() as u32;

        for i in 16..64 {
            // Read w[i-15].
            let mut w_i_minus_15 = [0u8; 4];
            caller.memory_read((w_ptr + (i - 15) * 4) as usize, &mut w_i_minus_15)?;
            let w_i_minus_15 = u32::from_be_bytes(w_i_minus_15);

            // Compute `s0`.
            let s0 =
                w_i_minus_15.rotate_right(7) ^ w_i_minus_15.rotate_right(18) ^ (w_i_minus_15 >> 3);

            // Read w[i-2].
            let mut w_i_minus_2 = [0u8; 4];
            caller.memory_read((w_ptr + (i - 2) * 4) as usize, &mut w_i_minus_2)?;
            let w_i_minus_2 = u32::from_be_bytes(w_i_minus_2);

            // Compute `s1`.
            let s1 =
                w_i_minus_2.rotate_right(17) ^ w_i_minus_2.rotate_right(19) ^ (w_i_minus_2 >> 10);

            // Read w[i-16].
            let mut w_i_minus_16 = [0u8; 4];
            caller.memory_read((w_ptr + (i - 16) * 4) as usize, &mut w_i_minus_16)?;
            let w_i_minus_16 = u32::from_be_bytes(w_i_minus_16);

            // Read w[i-7].
            let mut w_i_minus_7 = [0u8; 4];
            caller.memory_read((w_ptr + (i - 7) * 4) as usize, &mut w_i_minus_7)?;
            let w_i_minus_7 = u32::from_be_bytes(w_i_minus_7);

            // Compute `w_i`.
            let w_i = s1
                .wrapping_add(w_i_minus_16)
                .wrapping_add(s0)
                .wrapping_add(w_i_minus_7);

            caller.memory_write((w_ptr + i * 4) as usize, &w_i.to_be_bytes())?;
        }

        Ok(())
    }
}
