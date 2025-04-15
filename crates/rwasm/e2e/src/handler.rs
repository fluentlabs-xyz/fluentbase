use fluentbase_rwasm::{Caller, RwasmError, SyscallHandler};

pub const ENTRYPOINT_FUNC_IDX: u32 = u32::MAX;

pub const FUNC_PRINT: u32 = 100;
pub const FUNC_PRINT_I32: u32 = 101;
pub const FUNC_PRINT_I64: u32 = 102;
pub const FUNC_PRINT_F32: u32 = 103;
pub const FUNC_PRINT_F64: u32 = 104;
pub const FUNC_PRINT_I32_F32: u32 = 105;
pub const FUNC_PRINT_I64_F64: u32 = 106;

#[derive(Default)]
pub struct TestingContext {
    pub program_counter: u32,
    pub state: u32,
}

pub(crate) fn testing_context_syscall_handler(
    mut caller: Caller<TestingContext>,
    func_idx: u32,
) -> Result<(), RwasmError> {
    match func_idx {
        FUNC_PRINT => {
            println!("print");
            Ok(())
        }
        FUNC_PRINT_I32 => {
            let value = i32::from(caller.stack_pop());
            println!("print: {value}");
            Ok(())
        }
        FUNC_PRINT_I64 => {
            let value = i64::from(caller.stack_pop());
            println!("print: {value}");
            Ok(())
        }
        FUNC_PRINT_F32 => {
            let value = f32::from(caller.stack_pop());
            println!("print: {value}");
            Ok(())
        }
        FUNC_PRINT_F64 => {
            let value = f64::from(caller.stack_pop());
            println!("print: {value}");
            Ok(())
        }
        FUNC_PRINT_I32_F32 => {
            let (v0, v1) = caller.stack_pop2();
            println!("print: {:?} {:?}", i32::from(v0), f32::from(v1));
            Ok(())
        }
        FUNC_PRINT_I64_F64 => {
            let (v0, v1) = caller.stack_pop2();
            println!("print: {:?} {:?}", i64::from(v0), f64::from(v1));
            Ok(())
        }
        u32::MAX => {
            // yeah dirty, but this is how we remember the program counter to reset,
            // since we're 100% sure the function is called using `Call`
            // that we can safely deduct 1 from PC (for `ReturnCall` we need to deduct 2)
            caller.context_mut().program_counter = caller.vm().program_counter() - 1;
            // push state value into the stack
            caller.stack_push(caller.context().state);
            Ok(())
        }
        _ => todo!("not implemented syscall handler"),
    }
}
