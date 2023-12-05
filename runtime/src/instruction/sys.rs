use crate::{instruction::exported_memory_vec, runtime::RuntimeContext, ExitCode};
use fluentbase_rwasm::Caller;
use fluentbase_rwasm_core::common::Trap;

pub(crate) fn sys_halt<T>(
    mut caller: Caller<RuntimeContext<T>>,
    exit_code: u32,
) -> Result<(), Trap> {
    caller.data_mut().exit_code = exit_code as i32;
    Err(Trap::i32_exit(exit_code as i32))
}

pub(crate) fn sys_state<T>(caller: Caller<RuntimeContext<T>>) -> Result<u32, Trap> {
    Ok(caller.data().state)
}

pub(crate) fn sys_read<T>(
    mut caller: Caller<RuntimeContext<T>>,
    target: u32,
    offset: u32,
    length: u32,
) -> Result<u32, Trap> {
    let input = caller.data().input().clone();
    if offset + length > input.len() as u32 {
        return Err(ExitCode::MemoryOutOfBounds.into());
    }
    caller.write_memory(
        target as usize,
        &input.as_slice()[(offset as usize)..(offset as usize + length as usize)],
    );
    Ok(input.len() as u32)
}

pub(crate) fn sys_write<T>(
    mut caller: Caller<RuntimeContext<T>>,
    offset: u32,
    length: u32,
) -> Result<(), Trap> {
    // TODO: "add out of memory check"
    let memory = exported_memory_vec(&mut caller, offset as usize, length as usize);
    caller.data_mut().extend_return_data(memory.as_slice());
    Ok(())
}
