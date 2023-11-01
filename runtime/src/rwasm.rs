use crate::{exported_memory_vec, Runtime, RuntimeContext};
use fluentbase_rwasm::{common::Trap, Caller, rwasm::Compiler};

pub(crate) fn rwasm_compile(
    mut caller: Caller<'_, RuntimeContext>,
    input_offset: i32,
    input_len: i32,
    output_offset: i32,
) -> Result<i32, Trap> {
    let input = exported_memory_vec(&mut caller, input_offset as usize, input_len as usize);

    // translate WASM binary to rWASM
    let import_linker = Runtime::new_linker();
    let mut compiler = Compiler::new_with_linker(input.as_ref(), Some(&import_linker)).unwrap();
    let rwasm_bytecode = compiler.finalize().unwrap();

    caller.write_memory(output_offset as usize, &rwasm_bytecode.as_slice());

    Ok(rwasm_bytecode.len() as i32)
}
