use crate::{
    bail_illegal_opcode,
    constraint_builder::{AdviceColumn, ToExpr},
    gadgets::{lt::LtGadget, is_zero::IsZeroConfig},
    runtime_circuit::{
        constraint_builder::OpConstraintBuilder,
        execution_state::ExecutionState,
        opcodes::{ExecStep, ExecutionGadget, GadgetError},
        gadgets::f32_exp::F32ExpConfig,
        gadgets::f32_mantissa::F32MantissaConfig,
    },
    rw_builder::rw_row::RwTableContextTag,
    util::Field,
    fixed_table::FixedTableTag,
};
use fluentbase_rwasm::engine::bytecode::Instruction;
use halo2_proofs::circuit::Region;
use std::marker::PhantomData;
use crate::constraint_builder::SelectorColumn;
use crate::constraint_builder::Query;
use std::ops::Not;

#[derive(Clone)]
pub(crate) struct OpF32SqrtGadget<F: Field> {
    arg_sign: SelectorColumn,
    out_sign: SelectorColumn,
    arg_exp: F32ExpConfig<F>,
    out_exp: F32ExpConfig<F>,
    arg_mant: F32MantissaConfig<F>,
    out_mant: F32MantissaConfig<F>,
    lt_gadget: Option<LtGadget<F, 8>>,
    _pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpF32SqrtGadget<F> {
    const NAME: &'static str = "WASM_F32_SQRT";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_F32_SQRT;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let arg_sign = cb.query_selector();
        let out_sign = cb.query_selector();

        let arg_exp = cb.query_f32_exp();
        let out_exp = cb.query_f32_exp();

        let arg_mant = cb.query_f32_mantissa(arg_exp.is_extra_bit_set().into());
        let out_mant = cb.query_f32_mantissa(out_exp.is_extra_bit_set().into());

        let arg_sign_bit = || -> Query<F> { arg_sign.current().into() };

        cb.stack_pop(
            arg_sign.current().select(0x80000000_u32.expr(), 0.expr())
                + arg_exp.current() * 0x800000.expr()
                + arg_mant.raw_part(arg_exp.is_extra_bit_set().into())
        );

        cb.stack_push(
            out_sign.current().select(0x80000000_u32.expr(), 0.expr())
                + out_exp.current() * 0x800000.expr()
                + out_mant.raw_part(out_exp.is_extra_bit_set().into())
        );

        cb.condition(arg_exp.is_inf_or_nan().into(), |cb| {
            let out_inf_or_nan: Query<F> = out_exp.is_inf_or_nan().into();
            cb.require_zero("if argument is inf or nan, than result must be inf of nan",
               1.expr() - out_inf_or_nan,
            );
        });

        let arg_mant_abs = || arg_mant.absolute();
        let out_mant_abs = || out_mant.absolute();

        let big_mult = || out_mant_abs() * out_mant_abs();

        let lt_gadget = cb.lt_gadget(0x1000000.expr(), big_mult() - arg_mant_abs() * 0x1000000.expr() + 0x800000);
        cb.require_zero("rest of mantissa must be shifted out of bits", lt_gadget.expr());
        let opt_lt_gadget = Some(lt_gadget);

        cb.require_zero("increased out_exp and arg_exp must match",
            out_exp.current() - (arg_exp.current() + 1.expr())
        );

        Self {
            arg_sign,
            out_sign,
            arg_exp,
            out_exp,
            arg_mant,
            out_mant,
            lt_gadget: opt_lt_gadget,
            _pd: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &ExecStep,
    ) -> Result<(), GadgetError> {
        let arg = trace.curr_nth_stack_value(0)?;
        let out = trace.next_nth_stack_value(0)?;

        let arg_raw = arg.to_bits() as u32;
        let out_raw = out.to_bits() as u32;

        let arg_sign = (arg_raw as u64 >> 31) == 1;
        let out_sign = (out_raw as u64 >> 31) == 1;

        let arg_exp = (arg_raw >> 23) & 0xff;
        let out_exp = (out_raw >> 23) & 0xff;

        self.arg_sign.assign(region, offset, arg_sign);
        self.out_sign.assign(region, offset, out_sign);

        self.arg_exp.assign(region, offset, F::from(arg_exp as u64));
        self.out_exp.assign(region, offset, F::from(out_exp as u64));

        let (arg_mant_raw, arg_mant_abs) = self.arg_mant.assign_from_raw(region, offset, arg_raw);
        let (out_mant_raw, out_mant_abs) = self.out_mant.assign_from_raw(region, offset, out_raw);

        let big_mult = out_mant_abs as u64 * out_mant_abs as u64;
        println!("DEBUG out_mant_abs {}", out_mant_abs);
        println!("DEBUG bit_mult {}", big_mult);
        println!("DEBUG arg_mant_abs {}", arg_mant_abs);
        println!("DEBUG DIF {}", big_mult as u64 - arg_mant_abs as u64 * 0x1000000_u64 + 0x800000);

        self.lt_gadget.as_ref().unwrap().assign(
                region,
                offset,
                F::from(0x1000000 as u64),
                F::from(big_mult as u64 - arg_mant_abs as u64 * 0x1000000_u64 + 0x800000),
        );

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::instruction_set;

    #[test]
    fn test_f32_sqrt_simple() {
        test_ok(instruction_set! {
            I32Const(0x3e020c4a) // 0.127
            F32Sqrt
            Drop
        });
    }

    // TODO: correct implementation, about side of error.
    #[test]
    fn test_f32_sqrt_more_digits() {
        test_ok(instruction_set! {
            I32Const(0x42fe4106) // 127.127
            F32Sqrt
            Drop
        });
    }

}
