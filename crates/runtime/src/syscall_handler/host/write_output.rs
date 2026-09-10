use crate::RuntimeContext;
use rwasm::{StoreTr, TrapCode, Value};

pub fn syscall_write_output_handler(
    caller: &mut impl StoreTr<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    let (offset, length) = (params[0].i32().unwrap(), params[1].i32().unwrap());
    // The store validates the range against guest memory before it allocates the buffer.
    let data = caller.memory_read_into_vec(offset as usize, length as usize)?;
    syscall_write_output_owned(caller.data_mut(), data);
    Ok(())
}

pub fn syscall_write_output_impl(ctx: &mut RuntimeContext, data: &[u8]) {
    ctx.execution_result.output.extend_from_slice(data);
}

/// Appends `data` to the frame output, adopting the buffer outright when the output is still
/// empty. System runtimes write their whole envelope in a single call, so that write is copy-free.
pub fn syscall_write_output_owned(ctx: &mut RuntimeContext, data: Vec<u8>) {
    let output = &mut ctx.execution_result.output;
    if output.is_empty() {
        *output = data;
    } else {
        output.extend_from_slice(&data);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owned_write_adopts_then_appends() {
        let mut ctx = RuntimeContext::default();
        syscall_write_output_owned(&mut ctx, vec![1, 2]);
        syscall_write_output_owned(&mut ctx, vec![3]);
        syscall_write_output_impl(&mut ctx, &[4]);
        assert_eq!(ctx.execution_result.output, vec![1, 2, 3, 4]);
    }
}
