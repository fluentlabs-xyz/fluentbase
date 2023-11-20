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
pub(crate) struct OpF32MulGadget<F: Field> {
    lhs_exp_ge: SelectorColumn,
    out_exp_ge: SelectorColumn,
    lhs_sign: SelectorColumn,
    rhs_sign: SelectorColumn,
    out_sign: SelectorColumn,
    lhs_exp: F32ExpConfig<F>,
    rhs_exp: F32ExpConfig<F>,
    out_exp: F32ExpConfig<F>,
    lhs_mant: F32MantissaConfig<F>,
    rhs_mant: F32MantissaConfig<F>,
    out_mant: F32MantissaConfig<F>,
    pow2: AdviceColumn,
    lt_gadget: Option<LtGadget<F, 8>>,
    _pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpF32MulGadget<F> {
    const NAME: &'static str = "WASM_F32_MUL";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_F32_MUL;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let lhs_exp_ge = cb.query_selector();
        let out_exp_ge = cb.query_selector();

        let lhs_sign = cb.query_selector();
        let rhs_sign = cb.query_selector();
        let out_sign = cb.query_selector();

        let lhs_exp = cb.query_f32_exp();
        let rhs_exp = cb.query_f32_exp();
        let out_exp = cb.query_f32_exp();

        let lhs_mant = cb.query_f32_mantissa(lhs_exp.is_extra_bit_set().into());
        let rhs_mant = cb.query_f32_mantissa(rhs_exp.is_extra_bit_set().into());
        let out_mant = cb.query_f32_mantissa(out_exp.is_extra_bit_set().into());

        let pow2 = cb.query_cell();

        let lhs_sign_bit = || -> Query<F> { lhs_sign.current().into() };

        cb.range_check8(lhs_exp_ge.current().select(lhs_exp.current() - rhs_exp.current(), 0.expr()));

        cb.stack_pop(
            rhs_sign.current().select(0x80000000_u32.expr(), 0.expr())
                + rhs_exp.current() * 0x800000.expr()
                + rhs_mant.raw_part(rhs_exp.is_extra_bit_set().into())
        );

        cb.stack_pop(
            lhs_sign.current().select(0x80000000_u32.expr(), 0.expr())
                + lhs_exp.current() * 0x800000.expr()
                + lhs_mant.raw_part(lhs_exp.is_extra_bit_set().into())
        );

        cb.stack_push(
            out_sign.current().select(0x80000000_u32.expr(), 0.expr())
                + out_exp.current() * 0x800000.expr()
                + out_mant.raw_part(out_exp.is_extra_bit_set().into())
        );

        cb.condition(lhs_exp.is_inf_or_nan().or(rhs_exp.is_inf_or_nan()).into(), |cb| {
            let out_inf_or_nan: Query<F> = out_exp.is_inf_or_nan().into();
            cb.require_zero("if any argument is inf or nan, than result must be inf of nan",
               1.expr() - out_inf_or_nan,
            );
        });

        // Halo field is used to represent nagative values.
        let lhs_mant_abs = || lhs_mant.absolute();
        let rhs_mant_abs = || rhs_mant.absolute();
        let out_mant_abs = || out_mant.absolute();

        let big_mult = || lhs_mant_abs() * rhs_mant_abs();

        let lt_gadget = cb.lt_gadget(0x800000.expr(), big_mult() - out_mant_abs() * 0x800000.expr());
        cb.require_zero("rest of mantissa must be shifted out of bits", lt_gadget.expr());
        let opt_lt_gadget = Some(lt_gadget);

        cb.require_zero("addition of lhs_exp and rhs_exp must match out_exp",
            ( lhs_exp.current() + rhs_exp.current() - 127.expr() ) - out_exp.current()
        );

        let rhs_sign_q: Query<F> = rhs_sign.current().into();
        let rhs_sign_not_q: Query<F> = rhs_sign.current().not().into();
        let sign_to_check: Query<F> = lhs_sign.current().select(rhs_sign_not_q, rhs_sign_q);
        let out_sign_q: Query<F> = out_sign.current().into();

        cb.require_zero("sign logic must match",
            sign_to_check - out_sign_q
        );

        Self {
            lhs_exp_ge,
            out_exp_ge,
            lhs_sign,
            rhs_sign,
            out_sign,
            lhs_exp,
            rhs_exp,
            out_exp,
            lhs_mant,
            rhs_mant,
            out_mant,
            pow2,
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
        let lhs = trace.curr_nth_stack_value(1)?;
        let rhs = trace.curr_nth_stack_value(0)?;
        let out = trace.next_nth_stack_value(0)?;

        let lhs_raw = lhs.to_bits() as u32;
        let rhs_raw = rhs.to_bits() as u32;
        let out_raw = out.to_bits() as u32;

        let lhs_sign = (lhs_raw as u64 >> 31) == 1;
        let rhs_sign = (rhs_raw as u64 >> 31) == 1;
        let out_sign = (out_raw as u64 >> 31) == 1;

        let lhs_exp = (lhs_raw >> 23) & 0xff;
        let rhs_exp = (rhs_raw >> 23) & 0xff;
        let out_exp = (out_raw >> 23) & 0xff;

        let lhs_exp_ge = lhs_exp >= rhs_exp;
        self.lhs_exp_ge.assign(region, offset, lhs_exp_ge);

        self.lhs_sign.assign(region, offset, lhs_sign);
        self.rhs_sign.assign(region, offset, rhs_sign);
        self.out_sign.assign(region, offset, out_sign);

        self.lhs_exp.assign(region, offset, F::from(lhs_exp as u64));
        self.rhs_exp.assign(region, offset, F::from(rhs_exp as u64));
        self.out_exp.assign(region, offset, F::from(out_exp as u64));

        let (lhs_mant_raw, lhs_mant_abs) = self.lhs_mant.assign_from_raw(region, offset, lhs_raw);
        let (rhs_mant_raw, rhs_mant_abs) = self.rhs_mant.assign_from_raw(region, offset, rhs_raw);
        let (out_mant_raw, out_mant_abs) = self.out_mant.assign_from_raw(region, offset, out_raw);

        let big_mult = lhs_mant_abs as u64 * rhs_mant_abs as u64;

        self.lt_gadget.as_ref().unwrap().assign(
                region,
                offset,
                F::from(0x800000 as u64),
                F::from(big_mult as u64 - out_mant_abs as u64 * 0x800000_u64),
        );

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::instruction_set;

    #[test]
    fn test_f32_mul_squaring() {
        test_ok(instruction_set! {
            I32Const(0x3e020c4a) // 0.127
            I32Const(0x3e020c4a)
            F32Mul
            Drop
        });
    }

    #[test]
    fn test_f32_mul_squaring_with_neg() {
        test_ok(instruction_set! {
            I32Const(0x3e020c4a) // 0.127
            I32Const(0xbe020c4a_u64) // -0.127
            F32Mul
            Drop
        });
    }

    #[test]
    fn test_f32_mul_with_small() {
        test_ok(instruction_set! {
            I32Const(0x3e020c4a) // 0.127
            I32Const(0x03c00000) // 2^-120
            F32Mul
            Drop
        });
    }

}
