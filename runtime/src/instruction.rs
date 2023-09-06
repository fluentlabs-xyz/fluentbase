use crate::{runtime::RuntimeContext, types::EXIT_CODE_EVM_STOP};
use fluentbase_rwasm::{common::Trap, AsContextMut, Caller, Extern, Memory};

fn exported_memory(caller: &mut Caller<'_, RuntimeContext>) -> Memory {
    let memory = caller
        .get_export("memory")
        .unwrap_or_else(|| unreachable!("there is no memory export inside"));
    match memory {
        Extern::Memory(memory) => memory,
        _ => unreachable!("there is no memory export inside"),
    }
}

fn exported_memory_slice<'a>(
    caller: &'a mut Caller<'_, RuntimeContext>,
    offset: usize,
    length: usize,
) -> &'a mut [u8] {
    if length == 0 {
        return &mut [];
    }
    let memory = exported_memory(caller).data_mut(caller.as_context_mut());
    if memory.len() > offset {
        return &mut memory[offset..(offset + length)];
    }
    return &mut [];
}

fn exported_memory_vec(
    caller: &mut Caller<'_, RuntimeContext>,
    offset: usize,
    length: usize,
) -> Vec<u8> {
    if length == 0 {
        return Default::default();
    }
    let memory = exported_memory(caller).data_mut(caller.as_context_mut());
    if memory.len() > offset {
        return Vec::from(&memory[offset..(offset + length)]);
    }
    return Default::default();
}

pub(crate) fn sys_halt(mut caller: Caller<'_, RuntimeContext>, exit_code: u32) -> Result<(), Trap> {
    caller.data_mut().exit_code = exit_code as i32;
    Err(Trap::i32_exit(exit_code as i32))
}

pub(crate) fn sys_read(
    mut caller: Caller<'_, RuntimeContext>,
    target: u32,
    _offset: u32,
    length: u32,
) -> Result<u32, Trap> {
    let memory = exported_memory(&mut caller);
    let input = caller.data().input.clone();
    let memory = memory.data_mut(caller.as_context_mut());
    let length = if length > input.len() as u32 {
        input.len() as u32
    } else {
        length
    };
    memory[(target as usize)..((target + length) as usize)].clone_from_slice(input.as_slice());
    Ok(length)
}

pub(crate) fn evm_stop(mut caller: Caller<'_, RuntimeContext>) -> Result<(), Trap> {
    caller.data_mut().exit_code = EXIT_CODE_EVM_STOP;
    Err(Trap::i32_exit(EXIT_CODE_EVM_STOP))
}

pub(crate) fn evm_return(
    mut caller: Caller<'_, RuntimeContext>,
    offset: u32,
    length: u32,
) -> Result<(), Trap> {
    let memory = exported_memory_vec(&mut caller, offset as usize, length as usize);
    caller.data_mut().return_data(memory.as_slice());
    Ok(())
}
