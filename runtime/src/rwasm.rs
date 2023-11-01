use crate::{exported_memory_vec, ExitCode, Runtime, RuntimeContext};
use fluentbase_rwasm::{
    common::Trap,
    rwasm::{Compiler, CompilerError},
    Caller,
};

pub(crate) fn rwasm_compile(
    mut caller: Caller<'_, RuntimeContext>,
    input_ptr: u32,
    input_len: u32,
    output_ptr: u32,
    output_len: u32,
) -> Result<i32, Trap> {
    let input = exported_memory_vec(&mut caller, input_ptr as usize, input_len as usize);

    // translate WASM binary to rWASM
    let import_linker = Runtime::new_linker();
    let mut compiler = Compiler::new_with_linker(input.as_ref(), Some(&import_linker)).unwrap();
    let compile_res = compiler.finalize();
    match compile_res {
        Ok(rwasm_bytecode) => {
            if rwasm_bytecode.len() > output_len as usize {
                return Err(ExitCode::RwasmCompileOutputOverflow.into());
            }
            caller.write_memory(output_ptr as usize, &rwasm_bytecode.as_slice());

            return Ok(rwasm_bytecode.len() as i32);
        }
        Err(e) => Ok(e.into()),
    }
}
