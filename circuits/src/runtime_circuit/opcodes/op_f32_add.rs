use crate::{
    bail_illegal_opcode,
    constraint_builder::{AdviceColumn, ToExpr},
    gadgets::lt::LtGadget,
    runtime_circuit::{
        constraint_builder::OpConstraintBuilder,
        execution_state::ExecutionState,
        opcodes::{ExecStep, ExecutionGadget, GadgetError},
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

#[derive(Clone, Debug)]
pub(crate) struct OpF32AddGadget<F: Field> {
    lhs_exp_ge: SelectorColumn,
    lhs_sign: SelectorColumn,
    rhs_sign: SelectorColumn,
    out_sign: SelectorColumn,
    lhs_exp: AdviceColumn,
    rhs_exp: AdviceColumn,
    out_exp: AdviceColumn,
    lhs_limbs: [AdviceColumn; 3],
    rhs_limbs: [AdviceColumn; 3],
    out_limbs: [AdviceColumn; 3],
    pow2: AdviceColumn,
    // 24 bits to compare for addition, that is simpler than multiplication.
    lt_gadget: Option<LtGadget<F, 3>>,
    _pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpF32AddGadget<F> {
    const NAME: &'static str = "WASM_F32_ADD";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_F32_ADD;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let lhs_exp_ge = cb.query_selector();

        let lhs_sign = cb.query_selector();
        let rhs_sign = cb.query_selector();
        let out_sign = cb.query_selector();

        let lhs_exp = cb.query_cell();
        let rhs_exp = cb.query_cell();
        let out_exp = cb.query_cell();

        let lhs_limbs = [cb.query_cell(), cb.query_cell(), cb.query_cell()];
        let rhs_limbs = [cb.query_cell(), cb.query_cell(), cb.query_cell()];
        let out_limbs = [cb.query_cell(), cb.query_cell(), cb.query_cell()];

        let pow2 = cb.query_cell();

        let lhs_sign_bit: Query<F> = lhs_sign.current().into();

        cb.range_check8(lhs_exp_ge.current().select(lhs_exp.current() - rhs_exp.current(), 0.expr()));

        cb.range_check8(lhs_exp.current());
        cb.range_check8(rhs_exp.current());
        cb.range_check8(out_exp.current());

        for i in 0..2 {
            cb.range_check8(lhs_limbs[i].current());
            cb.range_check8(rhs_limbs[i].current());
            cb.range_check8(out_limbs[i].current());
        }

        cb.range_check7(lhs_limbs[2].current() - 0x80.expr());
        cb.range_check7(rhs_limbs[2].current() - 0x80.expr());
        cb.range_check7(out_limbs[2].current() - 0x80.expr());

        cb.stack_pop(
            rhs_sign.current().select(0x80000000_u32.expr(), 0.expr()) + rhs_exp.current() * 0x800000.expr() +
            (rhs_limbs[2].current() - 0x80.expr()) * 0x10000.expr() + rhs_limbs[1].current() * 0x100.expr() + rhs_limbs[0].current()
        );

        cb.stack_pop(
            lhs_sign.current().select(0x80000000_u32.expr(), 0.expr()) + lhs_exp.current() * 0x800000.expr() +
            (lhs_limbs[2].current() - 0x80.expr()) * 0x10000.expr() + lhs_limbs[1].current() * 0x100.expr() + lhs_limbs[0].current()
        );

        cb.stack_push(
            out_sign.current().select(0x80000000_u32.expr(), 0.expr()) + out_exp.current() * 0x800000.expr() +
            (out_limbs[2].current() - 0x80.expr()) * 0x10000.expr() + out_limbs[1].current() * 0x100.expr() + out_limbs[0].current()
        );

        // Halo field is used to represent nagative values.
        let lhs_mant_abs = || lhs_limbs[2].current() * 0x10000.expr() + lhs_limbs[1].current() * 0x100.expr() + lhs_limbs[0].current();
        let lhs_mant = || lhs_sign.current().select(0.expr() - lhs_mant_abs(), lhs_mant_abs());
        let rhs_mant_abs = || rhs_limbs[2].current() * 0x10000.expr() + rhs_limbs[1].current() * 0x100.expr() + rhs_limbs[0].current();
        let rhs_mant = || rhs_sign.current().select(0.expr() - rhs_mant_abs(), rhs_mant_abs());
        let out_mant_abs = || out_limbs[2].current() * 0x10000.expr() + out_limbs[1].current() * 0x100.expr() + out_limbs[0].current();
        let out_mant = || out_sign.current().select(0.expr() - out_mant_abs(), out_mant_abs());

        let first_exp = || lhs_exp_ge.current().select(lhs_exp.current(), rhs_exp.current());
        let second_exp = || lhs_exp_ge.current().select(rhs_exp.current(), lhs_exp.current());
        let first_mant = || lhs_exp_ge.current().select(lhs_mant(), rhs_mant());
        let second_mant = || lhs_exp_ge.current().select(rhs_mant(), lhs_mant());

        let mut opt_lt_gadget = None;

        // Same sign case.
        cb.condition(lhs_sign.current().xnor(rhs_sign.current()).into(), |cb| {
            let growing_exp = || out_exp.current() - first_exp();
            cb.require_boolean("is sign is same, `exp` can grow by zero or one", growing_exp());
            // If `out_exp` is growing, than second mant must satisfy.
            // Second mantissa for check is already truncated.
            let second_mant_for_check = || (out_mant() * (growing_exp() + 1.expr()) - first_mant()) * pow2.current();
            cb.fixed_lookup(FixedTableTag::Pow2, [
                first_exp() - second_exp(),
                pow2.current(),
                0.expr(),
            ]);
            let dif = || (second_mant() - second_mant_for_check()) * (1.expr() - lhs_sign_bit * 2.expr());
            let lt_gadget = cb.lt_gadget(pow2.current(), dif());
            cb.require_zero("rest of mantissa must be shifted out of bits", lt_gadget.expr());
            opt_lt_gadget = Some(lt_gadget);
        });

        Self {
            lhs_exp_ge,
            lhs_sign,
            rhs_sign,
            out_sign,
            lhs_exp,
            rhs_exp,
            out_exp,
            lhs_limbs,
            rhs_limbs,
            out_limbs,
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

        let lhs_sign = (lhs_raw as u64 >> 63) == 1;
        let rhs_sign = (rhs_raw as u64 >> 63) == 1;
        let out_sign = (out_raw as u64 >> 63) == 1;

        let lhs_exp = (lhs_raw >> 23) & 0xff;
        let rhs_exp = (rhs_raw >> 23) & 0xff;
        let out_exp = (out_raw >> 23) & 0xff;

        // Here in last limb, extra bit is added, bit number 24 in mantissa that always one.
        // TODO: case with un normalized form.
        let lhs_limbs = [lhs_raw & 0xff, (lhs_raw >> 8) & 0xff, ((lhs_raw >> 16) & 0x7f) | 0x80];
        let rhs_limbs = [rhs_raw & 0xff, (rhs_raw >> 8) & 0xff, ((rhs_raw >> 16) & 0x7f) | 0x80];
        let out_limbs = [out_raw & 0xff, (out_raw >> 8) & 0xff, ((out_raw >> 16) & 0x7f) | 0x80];

        let lhs_exp_ge = lhs_exp >= rhs_exp;
        self.lhs_exp_ge.assign(region, offset, lhs_exp_ge);

        self.lhs_sign.assign(region, offset, lhs_sign);
        self.rhs_sign.assign(region, offset, rhs_sign);
        self.out_sign.assign(region, offset, out_sign);

        self.lhs_exp.assign(region, offset, F::from(lhs_exp as u64));
        self.rhs_exp.assign(region, offset, F::from(rhs_exp as u64));
        self.out_exp.assign(region, offset, F::from(out_exp as u64));

        for i in 0..=2 {
            self.lhs_limbs[i].assign(region, offset, F::from(lhs_limbs[i] as u64));
            self.rhs_limbs[i].assign(region, offset, F::from(rhs_limbs[i] as u64));
            self.out_limbs[i].assign(region, offset, F::from(out_limbs[i] as u64));
        }

        let lhs_mant_abs = (lhs_raw & 0x7fffff) | 0x800000;
        let rhs_mant_abs = (rhs_raw & 0x7fffff) | 0x800000;
        let out_mant_abs = (out_raw & 0x7fffff) | 0x800000;

        let lhs_mant = if lhs_sign { -(lhs_mant_abs as i64) } else { lhs_mant_abs as i64 };
        let rhs_mant = if rhs_sign { -(rhs_mant_abs as i64) } else { rhs_mant_abs as i64 };
        let out_mant = if out_sign { -(out_mant_abs as i64) } else { out_mant_abs as i64 };

        let first_exp = if lhs_exp_ge { lhs_exp } else { rhs_exp };
        let second_exp = if lhs_exp_ge { rhs_exp } else { lhs_exp };

        let first_mant = if lhs_exp_ge { lhs_mant } else { rhs_mant };
        let second_mant = if lhs_exp_ge { rhs_mant } else { lhs_mant };

        if lhs_sign == rhs_sign {
            let growing_exp = out_exp - first_exp;
            assert!(growing_exp >= 0 && growing_exp <= 1);
            let to_shift = first_exp - second_exp;
            let pow2 = 1 << to_shift;
            self.pow2.assign(region, offset, F::from(pow2 as u64));
            let second_mant_for_check = (out_mant * (growing_exp as i64 + 1_i64) - first_mant) * pow2 as i64;
            let dif = (second_mant - second_mant_for_check) * (1_i64 - lhs_sign as i64 * 2_i64);
            println!("POW2 {}", pow2);
            println!("DIF {}", dif);
            self.lt_gadget.as_ref().unwrap().assign(
                region,
                offset,
                F::from(pow2 as u64),
                F::from(dif as u64),
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::instruction_set;

    // using https://www.h-schmidt.net/FloatConverter/IEEE754.html.

    #[test]
    fn test_f32_add_extra_bit_not_shifted_out() {
        test_ok(instruction_set! {
            I32Const(0x12800025) // 0_00100101__0000000_00000000_00100101, 8.07797129917e-28
            I32Const(0x0d00001a) // 0_00011010__0000000_00000000_00011010, 3.94431675125e-31
                                 //            ^                    ^^^^^
                                 //            extra bit            shifted out (number 26)
                                 //                                 so our `dif` is 26
                                 //                                 this is less than 2^11, so it is really shifted out
            F32Add
            // Out   0x12801025     0_00100101__0000000_00010000_00100101, 8.08191560369e-28
            //                                             ^
            //                                             extra bit in result, shifted by 11
            //                                             exp: 37 - 26 = 11
            Drop
        });
    }

    #[test]
    fn test_f32_add_doubling() {
        test_ok(instruction_set! {
            I32Const(0x12800025) // 0_00100101__0000000_00000000_00100101, 8.07797129917e-28
            I32Const(0x12800025)
            F32Add
            // Out   0x13000025     0_00100110__0000000_00000000_00100101, 1.61559425983e-27
            Drop
        });
    }

}
