use crate::{
    bail_illegal_opcode,
    constraint_builder::{AdviceColumn, ToExpr},
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
    lhs_exp_shift: AdviceColumn,
    rhs_exp_shift: AdviceColumn,
    out_exp_shift: AdviceColumn,
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

        let lhs_exp_shift = cb.query_cell();
        let rhs_exp_shift = cb.query_cell();
        let out_exp_shift = cb.query_cell();

        cb.range_check8(lhs_exp_ge.current().select(lhs_exp.current() - rhs_exp.current(), 0.expr()));

        cb.range_check8(lhs_exp.current());
        cb.range_check8(rhs_exp.current());
        cb.range_check8(out_exp.current());

        for i in 0..2 {
            cb.range_check8(lhs_limbs[i].current());
            cb.range_check8(rhs_limbs[i].current());
            cb.range_check8(out_limbs[i].current());
        }

        cb.range_check7(lhs_limbs[2].current() - 0x7f.expr());
        cb.range_check7(rhs_limbs[2].current() - 0x7f.expr());
        cb.range_check7(out_limbs[2].current() - 0x7f.expr());

        cb.stack_pop(
            rhs_sign.current().select(0x80000000_u32.expr(), 0.expr()) + rhs_exp.current() * 0x800000.expr() +
            (rhs_limbs[2].current() - 0x7f.expr()) * 0x10000.expr() + rhs_limbs[1].current() * 0x100.expr() + rhs_limbs[0].current()
        );

        cb.stack_pop(
            lhs_sign.current().select(0x80000000_u32.expr(), 0.expr()) + lhs_exp.current() * 0x800000.expr() +
            (lhs_limbs[2].current() - 0x7f.expr()) * 0x10000.expr() + lhs_limbs[1].current() * 0x100.expr() + lhs_limbs[0].current()
        );

        cb.stack_push(
            out_sign.current().select(0x80000000_u32.expr(), 0.expr()) + out_exp.current() * 0x800000.expr() +
            (out_limbs[2].current() - 0x7f.expr()) * 0x10000.expr() + out_limbs[1].current() * 0x100.expr() + out_limbs[0].current()
        );

        // Halo field is used to represent nagative values.
        let lhs_mant_abs = || lhs_limbs[2].current() * 0x10000.expr() + lhs_limbs[1].current() * 0x100.expr() + lhs_limbs[0].current();
        let lhs_mant = || lhs_sign.current().select(0.expr() - lhs_mant_abs(), lhs_mant_abs());
        let rhs_mant_abs = || rhs_limbs[2].current() * 0x10000.expr() + rhs_limbs[1].current() * 0x100.expr() + rhs_limbs[0].current();
        let rhs_mant = || rhs_sign.current().select(0.expr() - rhs_mant_abs(), rhs_mant_abs());
        let out_mant_abs = || out_limbs[2].current() * 0x10000.expr() + out_limbs[1].current() * 0x100.expr() + out_limbs[0].current();
        let out_mant = || out_sign.current().select(0.expr() - out_mant_abs(), out_mant_abs());

        let first_exp = || lhs_exp_ge.current().select(lhs_exp.current(), rhs_exp.current());
        //let second_exp = || lhs_exp_ge.current().select(lhs_exp.current(), rhs_exp.current());
        let first_mant = || lhs_exp_ge.current().select(lhs_mant(), rhs_mant());

        // Same sign case.
        cb.condition(lhs_sign.current().xnor(rhs_sign.current()).into(), |cb| {
            let growing_exp = || out_exp.current() - first_exp();
            cb.require_boolean("is sign is same, `exp` can grow by zero or one", growing_exp());
            // If `out_exp` is growing, than second mant must satisfy.
            // Second mantissa for check is already truncated.
            let second_mant_for_check = || out_mant() * (growing_exp() + 1.expr()) - first_mant();
        });

/*
        cb.fixed_lookup(
            FixedTableTag::Pow2,
            [
                lhs_exp.current(),
                odd() * is_ctz.expr(),
                terms[i].expr() * is_ctz.expr() + 16.expr() * (1.expr() - is_ctz.expr()),
            ],
        );
*/

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
            lhs_exp_shift,
            rhs_exp_shift,
            out_exp_shift,
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
        let lhs_limbs = [lhs_raw & 0xff, (lhs_raw >> 8) & 0xff, ((lhs_raw >> 16) & 0x7f) | 0x7f];
        let rhs_limbs = [rhs_raw & 0xff, (rhs_raw >> 8) & 0xff, ((rhs_raw >> 16) & 0x7f) | 0x7f];
        let out_limbs = [out_raw & 0xff, (out_raw >> 8) & 0xff, ((out_raw >> 16) & 0x7f) | 0x7f];

        self.lhs_exp_ge.assign(region, offset, lhs_exp >= rhs_exp);

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

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::instruction_set;

    // using https://www.h-schmidt.net/FloatConverter/IEEE754.html.

    #[test]
    fn table_f32_add_simple() {
        test_ok(instruction_set! {
            I32Const(0x12800025) // 0_00100101__0000000_00000000_00100101, 8.07797129917e-28
            I32Const(0x0d00001a) // 0_00011010__0000000_00000000_00011010, 3.94431675125e-31
                                 //            ^                    ^^^^^
                                 //            extra bit            shifted out

            F32Add
            // Out   0x12801025     0_00100101__0000000_00010000_00100101, 8.08191560369e-28
            //                                             ^
            //                                             extra bit in result, shifted by 11
            //                                             exp: 37 - 26 = 11
            Drop
        });
    }

}
