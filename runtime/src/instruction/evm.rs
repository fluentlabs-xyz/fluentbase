use crate::{
    instruction::{exported_memory_slice, exported_memory_vec},
    runtime::RuntimeContext,
    ExitCode,
    Runtime,
};
use fluentbase_rwasm::{
    common::Trap,
    rwasm::{Compiler, CompilerConfig},
    AsContext,
    AsContextMut,
    Caller,
};

pub(crate) fn sstore<T>(
    mut caller: Caller<'_, RuntimeContext<T>>,
    key_ptr: u32,
    val_ptr: u32,
) -> Result<(), Trap> {
    let key = exported_memory_vec(&mut caller, key_ptr as usize, 32);
    let val = exported_memory_vec(&mut caller, val_ptr as usize, 32);

    // caller.data_mut().take_context(|&t| {
    //     t.
    // });

    Ok(())
}

pub(crate) fn sload<T>(
    mut caller: Caller<'_, RuntimeContext<T>>,
    key_ptr: u32,
    val_ptr: u32,
) -> Result<(), Trap> {
    let key = exported_memory_vec(&mut caller, key_ptr as usize, 32);
    let val = exported_memory_slice(&mut caller, val_ptr as usize, 32);

    Ok(())
}
