use fluentbase_rwasm::{rwasm::Compiler, Caller};
use fluentbase_rwasm_core::common::Trap;

use crate::instruction::exported_memory_slice;
use crate::{exported_memory_vec, ExitCode, Runtime, RuntimeContext};

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
    let compile_res = compiler.finalize(None, true);
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
    state: i32,
    fuel_limit: i32,
) -> Result<i32, Trap> {
    let bytecode = exported_memory_vec(&mut caller, code_offset as usize, code_len as usize);
    let input = exported_memory_vec(&mut caller, input_offset as usize, input_len as usize);
    let output = exported_memory_slice(&mut caller, output_offset as usize, output_len as usize);
    // TODO: "we probably need custom linker here with reduced host calls number"
    // TODO: "make sure there is no panic inside runtime"
    let import_linker = Runtime::new_linker();
    let ctx = RuntimeContext::new(bytecode)
        .with_input(input.to_vec())
        .with_state(state as u32)
        .with_fuel_limit(fuel_limit as u32);
    let result = Runtime::run_with_context(ctx, &import_linker);
    if result.is_err() {
        return Err(ExitCode::TransactError.into());
    }
    let execution_result = result.unwrap();
    let execution_output = execution_result.data().output();
    if execution_output.len() > output.len() {
        return Err(ExitCode::TransactOutputOverflow.into());
    }
    let len = execution_output.len();
    output[0..len].copy_from_slice(execution_output.as_slice());
    Ok(execution_result.data().exit_code())
}
