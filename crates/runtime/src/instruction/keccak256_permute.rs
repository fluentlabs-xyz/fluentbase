use crate::RuntimeContext;
use rwasm::{Caller, TrapCode};
use tiny_keccak::keccakf;

pub(crate) const STATE_SIZE: u32 = 25;

// The permutation state is 25 u64's. Our word size is 32 bits, so it is 50 words.
pub const STATE_NUM_WORDS: u32 = STATE_SIZE * 2;

pub struct SyscallKeccak256Permute;

impl SyscallKeccak256Permute {
    pub fn fn_handler(mut caller: Caller<RuntimeContext>) -> Result<(), TrapCode> {
        let state_ptr: u32 = caller.stack_pop_as();
        let state = caller.memory_read_fixed::<{ STATE_NUM_WORDS as usize }>(state_ptr as usize)?;

        let result = Self::fn_impl(&state);
        caller.memory_write(state_ptr as usize, &result)?;

        Ok(())
    }

    pub fn fn_impl(state: &[u8]) -> Vec<u8> {
        let mut state_result = Vec::new();
        for values in state.chunks_exact(2) {
            let least_sig = values[0];
            let most_sig = values[1];
            state_result.push(least_sig as u64 + ((most_sig as u64) << 32));
        }
        let mut state = state_result.try_into().unwrap();
        keccakf(&mut state);
        let mut values_to_write = Vec::new();
        for i in 0..STATE_SIZE {
            let most_sig = ((state[i as usize] >> 32) & 0xFFFFFFFF) as u32;
            let least_sig = (state[i as usize] & 0xFFFFFFFF) as u32;
            values_to_write.push(least_sig);
            values_to_write.push(most_sig);
        }
        values_to_write
            .into_iter()
            .map(|x| x.to_be_bytes())
            .flatten()
            .collect::<Vec<_>>()
    }
}
