use crate::{runtime::RuntimeContext, ExitCode};
use fluentbase_rwasm::{common::Trap, AsContextMut, Caller, Extern, Memory};
use std::mem::size_of;

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
    offset: u32,
    length: u32,
) -> Result<(), Trap> {
    let input = caller.data().input().clone();
    if offset + length > input.len() as u32 {
        return Err(ExitCode::MemoryOutOfBounds.into());
    }
    caller.write_memory(
        target as usize,
        &input.as_slice()[(offset as usize)..(offset as usize + length as usize)],
    );
    Ok(())
}

pub(crate) fn sys_write(
    mut caller: Caller<'_, RuntimeContext>,
    offset: u32,
    length: u32,
) -> Result<(), Trap> {
    // TODO: "add out of memory check"
    let memory = exported_memory_vec(&mut caller, offset as usize, length as usize);
    caller.data_mut().return_data(memory.as_slice());
    Ok(())
}

pub(crate) fn wasi_proc_exit(
    mut caller: Caller<'_, RuntimeContext>,
    exit_code: i32,
) -> Result<(), Trap> {
    caller.data_mut().exit_code = exit_code;
    Err(Trap::i32_exit(exit_code))
}

pub(crate) fn wasi_fd_write(
    _caller: Caller<'_, RuntimeContext>,
    _fd: i32,
    _iovs_ptr: i32,
    _iovs_len: i32,
    _rp0_ptr: i32,
) -> Result<i32, Trap> {
    Ok(wasi::ERRNO_CANCELED.raw() as i32)
}

pub(crate) fn wasi_environ_sizes_get(
    _caller: Caller<'_, RuntimeContext>,
    _rp0_ptr: i32,
    _pr1_ptr: i32,
) -> Result<i32, Trap> {
    Ok(wasi::ERRNO_CANCELED.raw() as i32)
}

pub(crate) fn wasi_environ_get(
    _caller: Caller<'_, RuntimeContext>,
    _environ: i32,
    _environ_buf: i32,
) -> Result<i32, Trap> {
    Ok(wasi::ERRNO_CANCELED.raw() as i32)
}

pub(crate) fn wasi_args_sizes_get(
    mut caller: Caller<'_, RuntimeContext>,
    rp0: i32,
    rp1: i32,
) -> Result<i32, Trap> {
    // first arg is always 1, because we pass only one string
    let rp0_ptr = exported_memory_slice(&mut caller, rp0 as usize, 4);
    rp0_ptr.copy_from_slice(&1u32.to_be_bytes());
    // second arg is length of input
    let input_len = caller.data().input.len() as u32;
    let rp1_ptr = exported_memory_slice(&mut caller, rp1 as usize, 4);
    rp1_ptr.copy_from_slice(&input_len.to_be_bytes());
    // its always success
    Ok(wasi::ERRNO_SUCCESS.raw() as i32)
}

pub(crate) fn wasi_args_get(
    mut caller: Caller<'_, RuntimeContext>,
    argv: i32,
    argv_buffer: i32,
) -> Result<i32, Trap> {
    let input = caller.data().input().clone();
    // copy all input into argv buffer
    caller.write_memory(argv_buffer as usize, &input.as_slice());
    // init argv array (we have only 1 element inside argv)
    caller.write_memory(argv as usize, &argv_buffer.to_be_bytes());
    Ok(wasi::ERRNO_CANCELED.raw() as i32)
}

pub(crate) fn evm_stop(mut caller: Caller<'_, RuntimeContext>) -> Result<(), Trap> {
    caller.data_mut().exit_code = ExitCode::EvmStop as i32;
    Err(ExitCode::EvmStop.into())
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
