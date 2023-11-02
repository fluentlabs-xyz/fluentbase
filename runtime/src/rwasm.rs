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
            if rwasm_bytecode.len() <= output_len as usize {
                caller.write_memory(output_ptr as usize, &rwasm_bytecode.as_slice());
            }

            return Ok(rwasm_bytecode.len() as i32);
        }
        Err(e) => Ok(e.into()),
    }
}

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
    let res = Runtime::run(bytecode.as_slice(), &vec![input.to_vec()]);
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
    if execution_result.data().exit_code < 0 {
        return Ok(execution_result.data().exit_code);
    }
    Ok(output.len() as i32)
}