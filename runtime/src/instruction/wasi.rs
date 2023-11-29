use crate::{instruction::exported_memory_slice, runtime::RuntimeContext};
use fluentbase_rwasm::Caller;
use fluentbase_rwasm_core::common::Trap;

pub(crate) fn wasi_proc_exit<T>(
    mut caller: Caller<'_, RuntimeContext<T>>,
    exit_code: i32,
) -> Result<(), Trap> {
    caller.data_mut().exit_code = exit_code;
    Err(Trap::i32_exit(exit_code))
}

pub(crate) fn wasi_fd_write<T>(
    _caller: Caller<'_, RuntimeContext<T>>,
    _fd: i32,
    _iovs_ptr: i32,
    _iovs_len: i32,
    _rp0_ptr: i32,
) -> Result<i32, Trap> {
    Ok(wasi::ERRNO_CANCELED.raw() as i32)
}

pub(crate) fn wasi_environ_sizes_get<T>(
    _caller: Caller<'_, RuntimeContext<T>>,
    _rp0_ptr: i32,
    _rp1_ptr: i32,
) -> Result<i32, Trap> {
    Ok(wasi::ERRNO_CANCELED.raw() as i32)
}

pub(crate) fn wasi_environ_get<T>(
    _caller: Caller<'_, RuntimeContext<T>>,
    _environ: i32,
    _environ_buf: i32,
) -> Result<i32, Trap> {
    Ok(wasi::ERRNO_CANCELED.raw() as i32)
}

pub(crate) fn wasi_args_sizes_get<T>(
    mut caller: Caller<'_, RuntimeContext<T>>,
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

pub(crate) fn wasi_args_get<T>(
    mut _caller: Caller<'_, RuntimeContext<T>>,
    _argv_ptrs_ptr: i32,
    _argv_buff_ptr: i32,
) -> Result<i32, Trap> {
    // let argc = caller.data().input_count();
    // let argv = caller.data().input_size();
    // // copy argv ptrs into argc buffer
    // let input = caller.data().input.clone();
    // let argv_buffer = caller.data().argv_buffer();
    // let argv_ptrs = exported_memory_slice(&mut caller, argv_ptrs_ptr as usize, (argc * 4) as
    // usize); let mut ptr_sum = argv_buff_ptr;
    // for (i, it) in input.iter().enumerate() {
    //     let ptr_le = ptr_sum.to_le_bytes();
    //     argv_ptrs[i..].copy_from_slice(&ptr_le);
    //     ptr_sum += it.len() as i32;
    // }
    // // copy argv buffer
    // let argv_buff = exported_memory_slice(&mut caller, argv_buff_ptr as usize, argv as usize);
    // argv_buff.copy_from_slice(argv_buffer.as_slice());
    // return success
    Ok(wasi::ERRNO_SUCCESS.raw() as i32)
}
