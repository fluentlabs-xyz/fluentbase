use rwasm::Caller;
use rwasm::core::Trap;
use crate::RuntimeContext;
use tiny_keccak::keccakf;

pub(crate) const STATE_SIZE: u32 = 25;

// The permutation state is 25 u64's.  Our word size is 32 bits, so it is 50 words.
pub const STATE_NUM_WORDS: u32 = STATE_SIZE * 2;

pub struct SyscallKeccak256Permute;

impl SyscallKeccak256Permute {

    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>, arg1: u32, arg2: u32) -> Result<(), Trap> {

        let state_ptr = arg1;
        if arg2 != 0 {
            panic!("Expected arg2 to be 0, got {arg2}");
        }

        let mut state = Vec::new();

        let state_values = caller.read_memory(state_ptr, STATE_NUM_WORDS)?;

        for values in state_values.chunks_exact(2) {
            let least_sig = values[0];
            let most_sig = values[1];
            state.push(least_sig as u64 + ((most_sig as u64) << 32));
        }
        let mut state = state.try_into().unwrap();
        keccakf(&mut state);

        // Increment the clk by 1 before writing because we read from memory at start_clk.
        let mut values_to_write = Vec::new();
        for i in 0..STATE_SIZE {
            let most_sig = ((state[i as usize] >> 32) & 0xFFFFFFFF) as u32;
            let least_sig = (state[i as usize] & 0xFFFFFFFF) as u32;
            values_to_write.push(least_sig);
            values_to_write.push(most_sig);
        }

        caller.write_memory(state_ptr, values_to_write.into_iter().map(|x| x.to_be_bytes()).flatten().collect::<Vec<_>>().as_slice())?;

        Ok(())
    }
}
