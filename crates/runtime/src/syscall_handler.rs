use crate::{instruction::invoke_runtime_handler, RuntimeContext};
use fluentbase_types::SysFuncIdx;
use rwasm::{TrapCode, TypedCaller, Value};

/// Routes a syscall identified by func_idx to the corresponding runtime instruction handler.
pub(crate) fn runtime_syscall_handler(
    caller: &mut TypedCaller<RuntimeContext>,
    func_idx: u32,
    params: &[Value],
    result: &mut [Value],
) -> Result<(), TrapCode> {
    let sys_func_idx = SysFuncIdx::from_repr(func_idx).ok_or(TrapCode::UnknownExternalFunction)?;
    invoke_runtime_handler(caller, sys_func_idx, params, result)
}
