use crate::RuntimeContext;
use rwasm::{Store, TrapCode, TypedCaller, Value};

pub struct SyscallChargeFuel;

impl SyscallChargeFuel {
    pub fn fn_handler(
        caller: &mut TypedCaller<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let fuel_consumed = params[0].i64().unwrap() as u64;
        caller.try_consume_fuel(fuel_consumed)?;
        Ok(())
    }

    pub fn fn_impl(_ctx: &mut RuntimeContext, _fuel_consumed: u64) {
        unimplemented!()
    }
}
