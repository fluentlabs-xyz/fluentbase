/// Syscall entry points for resuming a previously interrupted runtime.
use crate::syscall_handler::syscall_process_exit_code;
use crate::{
    executor::{default_runtime_executor, RuntimeExecutor},
    RuntimeContext,
};
use fluentbase_types::{
    byteorder::{ByteOrder, LittleEndian},
    ExitCode,
};
use rwasm::{StoreTr, TrapCode, Value};

/// Handles the resume syscall. Copies return data, applies fuel, resumes the target, and writes back the exit code.
pub fn syscall_resume_handler(
    caller: &mut impl StoreTr<RuntimeContext>,
    params: &[Value],
    result: &mut [Value],
) -> Result<(), TrapCode> {
    // Reject non-root callers before copying unmetered guest memory.
    if caller.data().call_depth > 0 {
        Err(syscall_process_exit_code(caller, ExitCode::RootCallOnly))
    } else {
        Ok(())
    }?;

    let (call_id, return_data_ptr, return_data_len, exit_code, fuel16_ptr) = (
        params[0].i32().unwrap() as u32,
        params[1].i32().unwrap() as usize,
        params[2].i32().unwrap() as usize,
        params[3].i32().unwrap(),
        params[4].i32().unwrap() as usize,
    );
    let return_data = caller.memory_read_into_vec(return_data_ptr, return_data_len)?;
    let (fuel_consumed, fuel_refunded) = if fuel16_ptr > 0 {
        let mut fuel_buffer = [0u8; 16];
        caller.memory_read(fuel16_ptr, &mut fuel_buffer)?;
        let fuel_consumed = LittleEndian::read_i64(&fuel_buffer[..8]) as u64;
        let fuel_refunded = LittleEndian::read_i64(&fuel_buffer[8..]);
        (fuel_consumed, fuel_refunded)
    } else {
        (0, 0)
    };
    let (fuel_consumed, fuel_refunded, exit_code) = syscall_resume_impl(
        caller.data_mut(),
        call_id,
        return_data,
        exit_code,
        fuel_consumed,
        fuel_refunded,
        fuel16_ptr as u32,
    )
    .map_err(|exit_code| syscall_process_exit_code(caller, exit_code))?;
    if fuel16_ptr > 0 {
        caller.memory_write(fuel16_ptr, &fuel_consumed.to_le_bytes())?;
        caller.memory_write(fuel16_ptr + 8, &fuel_refunded.to_le_bytes())?;
    }
    result[0] = Value::I32(exit_code);
    Ok(())
}

/// Resumes the runtime identified by call_id using the provided return data and fuel accounting.
pub fn syscall_resume_impl(
    ctx: &mut RuntimeContext,
    call_id: u32,
    return_data: Vec<u8>,
    exit_code: i32,
    fuel_consumed: u64,
    fuel_refunded: i64,
    fuel16_ptr: u32,
) -> Result<(u64, i64, i32), ExitCode> {
    // only root can use resume function
    if ctx.call_depth > 0 {
        return Err(ExitCode::RootCallOnly);
    }
    let result = default_runtime_executor().resume(
        call_id,
        return_data,
        fuel16_ptr,
        fuel_consumed,
        fuel_refunded,
        exit_code,
    );
    // Move output into parent's return data
    ctx.execution_result.return_data = result.output;
    Ok((
        result.fuel_consumed,
        result.fuel_refunded,
        // We return `call_id` as exit code, it's safe since exit code can't be positive
        result.exit_code,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::RuntimeFactoryExecutor;
    use fluentbase_types::{import_linker_v1_preview, keccak256, Address, BytecodeOrHash};
    use rwasm::{CompilationConfig, RwasmModule};

    #[test]
    fn resume_checks_call_depth_before_guest_memory() {
        // Exercise the return-data pointer, length, and fuel pointer independently.
        [(-1, 1, 0), (0, -1, 0), (0, 0, -1)]
            .into_iter()
            .for_each(|(return_data_ptr, return_data_len, fuel16_ptr)| {
                let wasm = wat::parse_str(format!(
                    r#"(module
                        (import "fluentbase_v1preview" "_resume"
                            (func $resume (param i32 i32 i32 i32 i32) (result i32)))
                        (memory (export "memory") 1)
                        (func (export "main")
                            i32.const 0
                            i32.const {return_data_ptr}
                            i32.const {return_data_len}
                            i32.const 0
                            i32.const {fuel16_ptr}
                            call $resume
                            drop))"#
                ))
                .unwrap();
                let import_linker = import_linker_v1_preview();
                let config = CompilationConfig::default()
                    .with_entrypoint_name("main".into())
                    .with_import_linker(import_linker.clone());
                let (module, _) = RwasmModule::compile(config, &wasm).unwrap();
                let mut executor = RuntimeFactoryExecutor::new(import_linker);

                [0, 1, 1024].into_iter().for_each(|call_depth| {
                    let result = executor.execute(
                        BytecodeOrHash::Bytecode {
                            bytecode: module.clone(),
                            hash: keccak256(&wasm),
                            address: Address::ZERO,
                        },
                        RuntimeContext::default()
                            .with_fuel_limit(100_000)
                            .with_call_depth(call_depth),
                    );
                    let expected = if call_depth == 0 {
                        ExitCode::MemoryOutOfBounds
                    } else {
                        ExitCode::RootCallOnly
                    };
                    assert_eq!(
                        result.exit_code,
                        expected.into_i32(),
                        "depth={call_depth}, return_data=({return_data_ptr}, {return_data_len}), fuel={fuel16_ptr}"
                    );
                });
            });
    }
}
