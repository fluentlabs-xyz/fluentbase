use crate::RuntimeContext;
use rwasm::{StoreTr, TrapCode, Value};

pub fn syscall_enter_leave_unconstrained_handler(
    ctx: &mut impl StoreTr<RuntimeContext>,
    _params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    syscall_enter_leave_unconstrained_impl(ctx.data_mut());
    Ok(())
}

pub fn syscall_enter_leave_unconstrained_impl(_ctx: &mut RuntimeContext) {}
