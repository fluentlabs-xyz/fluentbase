pub use crate::{crypto::*, ecc::*, mpt::*, rwasm::*};
use crate::{runtime::RuntimeContext, ExitCode, Runtime};
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

pub(crate) fn exported_memory_slice<'a>(
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

pub(crate) fn exported_memory_vec(
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

pub(crate) fn sys_state(caller: Caller<'_, RuntimeContext>) -> Result<u32, Trap> {
    Ok(caller.data().state)
}

pub(crate) fn sys_read(
    mut caller: Caller<'_, RuntimeContext>,
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

pub(crate) fn sys_write(
    mut caller: Caller<'_, RuntimeContext>,
    offset: u32,
    length: u32,
) -> Result<(), Trap> {
    // TODO: "add out of memory check"
    let memory = exported_memory_vec(&mut caller, offset as usize, length as usize);
    caller.data_mut().extend_return_data(memory.as_slice());
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
    _rp1_ptr: i32,
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
    argc_ptr: i32,
    argv_ptr: i32,
) -> Result<i32, Trap> {
    let argc = caller.data().input_count();
    let argv = caller.data().input_size();
    // copy argc into memory
    let argc_slice = exported_memory_slice(&mut caller, argc_ptr as usize, 4);
    argc_slice.copy_from_slice(&argc.to_le_bytes());
    // second arg is length of input
    let argv_slice = exported_memory_slice(&mut caller, argv_ptr as usize, 4);
    argv_slice.copy_from_slice(&argv.to_le_bytes());
    // its always success
    Ok(wasi::ERRNO_SUCCESS.raw() as i32)
}

pub(crate) fn wasi_args_get(
    mut caller: Caller<'_, RuntimeContext>,
    argv_ptrs_ptr: i32,
    argv_buff_ptr: i32,
) -> Result<i32, Trap> {
    let argc = caller.data().input_count();
    let argv = caller.data().input_size();
    // copy argv ptrs into argc buffer
    let input = caller.data().input.clone();
    let argv_buffer = caller.data().argv_buffer();
    let argv_ptrs = exported_memory_slice(&mut caller, argv_ptrs_ptr as usize, (argc * 4) as usize);
    let mut ptr_sum = argv_buff_ptr;
    for (i, it) in input.iter().enumerate() {
        // let ptr_le = ptr_sum.to_le_bytes();
        // argv_ptrs[i..].copy_from_slice(&ptr_le);
        // ptr_sum += it.len() as i32;
    }
    // copy argv buffer
    let argv_buff = exported_memory_slice(&mut caller, argv_buff_ptr as usize, argv as usize);
    argv_buff.copy_from_slice(argv_buffer.as_slice());
    // return success
    Ok(wasi::ERRNO_SUCCESS.raw() as i32)
}
