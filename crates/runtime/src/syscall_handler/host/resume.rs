/// Syscall entry points for resuming a previously interrupted runtime.
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
    let (call_id, return_data_ptr, return_data_len, exit_code, fuel16_ptr) = (
        params[0].i32().unwrap() as u32,
        params[1].i32().unwrap() as usize,
        params[2].i32().unwrap() as usize,
        params[3].i32().unwrap(),
        params[4].i32().unwrap() as usize,
    );
    let mut return_data = vec![0u8; return_data_len];
    caller.memory_read(return_data_ptr, &mut return_data)?;
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
        &return_data,
        exit_code,
        fuel_consumed,
        fuel_refunded,
        fuel16_ptr as u32,
    );
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
    return_data: &[u8],
    exit_code: i32,
    fuel_consumed: u64,
    fuel_refunded: i64,
    fuel16_ptr: u32,
) -> (u64, i64, i32) {
    // only root can use resume function
    if ctx.call_depth > 0 {
        return (0, 0, ExitCode::RootCallOnly.into_i32());
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
    (
        result.fuel_consumed,
        result.fuel_refunded,
        // We return `call_id` as exit code, it's safe since exit code can't be positive
        result.exit_code,
    )
}
