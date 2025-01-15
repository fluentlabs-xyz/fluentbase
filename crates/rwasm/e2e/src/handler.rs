use fluentbase_rwasm::{Caller, RwasmError, SyscallHandler};

pub const ENTRYPOINT_FUNC_IDX: u32 = u32::MAX;

#[derive(Default)]
pub struct TestingContext {
    pub program_counter: u32,
    pub state: u32,
}

pub struct TestingSyscallHandler {}

impl SyscallHandler<TestingContext> for TestingSyscallHandler {
    fn call_function(mut caller: Caller<TestingContext>, func_idx: u32) -> Result<(), RwasmError> {
        match func_idx {
            u32::MAX => {
                // yeah dirty, but this is how we remember the program counter to reset,
                // since we're 100% sure the function is called using `Call`
                // that we can safely deduct 1 from PC (for `ReturnCall` we need to deduct 2)
                caller.data_mut().program_counter = caller.store().program_counter() - 1;
                // push state value into the stack
                caller.stack_push(caller.data().state);
                Ok(())
            }
            _ => todo!("not implemented syscall handler"),
        }
    }
}
