use crate::{runtime::RuntimeContext, ExitCode, Runtime};
use fluentbase_rwasm::{common::Trap, AsContextMut, Caller, Extern, Memory};
use std::mem::size_of;
use tiny_keccak::{Hasher, Sha3};
use zktrie::{AccountData, ZkMemoryDb};

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
    argv_len: i32,
    argv_buffer_len: i32,
) -> Result<i32, Trap> {
    // first arg is always 1, because we pass only one string
    let argv_slice = exported_memory_slice(&mut caller, argv_len as usize, 4);
    argv_slice.copy_from_slice(&1u32.to_be_bytes());
    // second arg is length of input
    let input_len = caller.data().input.len() as u32;
    let argv_buffer_slice = exported_memory_slice(&mut caller, argv_buffer_len as usize, 4);
    argv_buffer_slice.copy_from_slice(&input_len.to_be_bytes());
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
    Ok(wasi::ERRNO_SUCCESS.raw() as i32)
}

// global map

// pub(crate) fn zktrie_open(mut caller: Caller<'_, RuntimeContext>) -> Result<i32, Trap> {
//     Ok(0)
// }
//
// pub(crate) fn zktrie_change_nonce(
//     mut caller: Caller<'_, RuntimeContext>,
//     trie_id: i32,
//     key_offset: i32,
//     value_offset: i32,
// ) { let root = exported_memory_vec(&mut caller, root_offset as usize, 32); let key =
//   exported_memory_vec(&mut caller, key_offset as usize, 32); let value = exported_memory_vec(&mut
//   caller, value_offset as usize, 32);
//
//     let mut db = ZkMemoryDb::new();
//
//     /* for some trie node data encoded as bytes `buf` */
//     let hash: zktrie::Hash = root.try_into().unwrap();
//     let mut trie = db.new_trie(&hash).unwrap();
//
//     trie.update_account(key.as_slice(), &AccountData::default())
//         .unwrap();
//
//     let new_root = trie.root();
//
//     // initial_value (prev_trie_root)
//
//     // BeginTx -> zktrie_open_trie
//     // EndTx -> zktrie_commit_trie / zktrie_rollback_trie
//
//     // open_trie
//     // commit_trie
// }

pub(crate) fn rwasm_transact(
    mut caller: Caller<'_, RuntimeContext>,
    code_offset: i32,
    code_len: i32,
    input_offset: i32,
    input_len: i32,
    output_offset: i32,
    output_len: i32,
) -> Result<i32, Trap> {
    let bytecode = exported_memory_vec(&mut caller, code_offset as usize, code_len as usize);
    let input = exported_memory_vec(&mut caller, input_offset as usize, input_len as usize);
    // TODO: "we probably need custom linker here with reduced host calls number"
    // TODO: "make sure there is no panic inside runtime"
    let res = Runtime::run(bytecode.as_slice(), input.as_slice());
    if res.is_err() {
        return Err(ExitCode::TransactError.into());
    }
    let execution_result = res.unwrap();
    // caller
    //     .as_context_mut()
    //     .tracer_mut()
    //     .merge_nested_call(execution_result.tracer());
    // copy output into memory
    let output = execution_result.data().output();
    if output.len() > output_len as usize {
        return Err(ExitCode::TransactOutputOverflow.into());
    }
    caller.write_memory(output_offset as usize, output.as_slice());
    // put exit code on stack
    Ok(execution_result.data().exit_code)
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

pub(crate) fn evm_keccak256(
    mut caller: Caller<'_, RuntimeContext>,
    offset: u32,
    size: u32,
    target: u32,
) -> Result<(), Trap> {
    // Ensure the offset and size are valid
    let input_data = exported_memory_vec(&mut caller, offset as usize, size as usize);
    assert!(offset + size as u32 <= input_data.len() as u32);

    let data_slice = input_data.as_slice();
    let mut hasher = Sha3::v256();

    hasher.update(data_slice);

    let mut result = [0u8; 32];
    hasher.finalize(&mut result);

    // caller.data_mut().return_data(result.as_slice());

    Ok(())
}
