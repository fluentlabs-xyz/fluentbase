/// Builtin to append bytes to the runtime output buffer.
use crate::{syscall_handler::charge_fuel, RuntimeContext};
use fluentbase_types::OUTPUT_WORD_FUEL_SURCHARGE;
use rwasm::{StoreTr, TrapCode, Value};

/// Reads a slice from linear memory and appends it to ctx.execution_result.output.
pub fn syscall_write_output_handler(
    caller: &mut impl StoreTr<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    let (offset, length) = (
        params[0].i32().unwrap() as u32,
        params[1].i32().unwrap() as u32,
    );

    // The translator already charges the generic linear copy cost. Charge output retention here
    // with u64 arithmetic before allocating or copying so repeated writes cannot grow the host Vec
    // beyond what the invocation's fuel can fund. Keeping this in the host also covers runtimes
    // whose bytecode does not contain translator-injected builtin metering.
    charge_output_fuel(caller, length)?;
    let data = caller.memory_read_into_vec(offset as usize, length as usize)?;
    syscall_write_output_impl(caller.data_mut(), &data);
    Ok(())
}

pub(crate) fn charge_output_fuel(
    caller: &mut impl StoreTr<RuntimeContext>,
    appended_length: u32,
) -> Result<(), TrapCode> {
    let current_length = caller.data().execution_result.output.len() as u64;
    let fuel = output_fuel_surcharge(current_length, appended_length)?;
    charge_fuel(caller, fuel)
}

fn output_fuel_surcharge(current_length: u64, appended_length: u32) -> Result<u64, TrapCode> {
    let new_length = current_length
        .checked_add(u64::from(appended_length))
        .ok_or(TrapCode::IntegerOverflow)?;
    let added_words = new_length.div_ceil(32) - current_length.div_ceil(32);
    added_words
        .checked_mul(u64::from(OUTPUT_WORD_FUEL_SURCHARGE))
        .ok_or(TrapCode::IntegerOverflow)
}

pub fn syscall_write_output_impl(ctx: &mut RuntimeContext, data: &[u8]) {
    ctx.execution_result.output.extend_from_slice(data);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_fuel_surcharge_rounds_to_words() {
        let word_cost = u64::from(OUTPUT_WORD_FUEL_SURCHARGE);

        assert_eq!(output_fuel_surcharge(0, 0), Ok(0));
        assert_eq!(output_fuel_surcharge(0, 1), Ok(word_cost));
        assert_eq!(output_fuel_surcharge(0, 32), Ok(word_cost));
        assert_eq!(output_fuel_surcharge(0, 33), Ok(2 * word_cost));
        assert_eq!(output_fuel_surcharge(1, 31), Ok(0));
        assert_eq!(output_fuel_surcharge(1, 32), Ok(word_cost));
        assert_eq!(
            output_fuel_surcharge(0, u32::MAX),
            Ok(u64::from(u32::MAX).div_ceil(32) * word_cost)
        );
        assert_eq!(
            output_fuel_surcharge(u64::MAX, 1),
            Err(TrapCode::IntegerOverflow)
        );
    }
}
