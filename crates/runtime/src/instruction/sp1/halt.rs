use rwasm::Caller;
use num::{BigUint, One, Zero};
use rwasm::core::Trap;
use sp1_curves::edwards::WORDS_FIELD_ELEMENT;
use sp1_primitives::consts::{bytes_to_words_le, words_to_bytes_le_vec, WORD_SIZE};
use fluentbase_types::ExitCode;
use crate::{RuntimeContext};

pub struct SyscallHalt;

impl SyscallHalt {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>, exit_code: u32, _: u32) -> Result<(), Trap> {
        let mut ctx = caller.data_mut();
        ctx.execution_result.exit_code = exit_code as i32;

        Err(ExitCode::ExecutionHalted.into_trap())
    }
}
