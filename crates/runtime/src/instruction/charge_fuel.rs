use crate::RuntimeContext;
use rwasm::{Caller, TrapCode, Value};
use std::cell::RefMut;

pub struct SyscallChargeFuel;

impl SyscallChargeFuel {
    pub fn fn_handler(
        caller: &mut dyn Caller<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let fuel_consumed = params[0].i64().unwrap() as u64;
        caller.try_consume_fuel(fuel_consumed)?;
        Ok(())
    }

    pub fn fn_impl(mut ctx: RefMut<RuntimeContext>, fuel_consumed: u64) {
        ctx.try_consume_fuel(fuel_consumed).unwrap();
    }
}
