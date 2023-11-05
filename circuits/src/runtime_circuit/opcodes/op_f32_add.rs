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
    lhs: AdviceColumn,
    rhs: AdviceColumn,
    out: AdviceColumn,
    lhs_sign: SelectorColumn,
    rhs_sign: SelectorColumn,
    out_sign: SelectorColumn,
    lhs_exp: AdviceColumn,
    rhs_exp: AdviceColumn,
    out_exp: AdviceColumn,
    lhs_limbs: [AdviceColumn; 3],
    rhs_limbs: [AdviceColumn; 3],
    out_limbs: [AdviceColumn; 3],
    _pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpF32AddGadget<F> {
    const NAME: &'static str = "WASM_F32_ADD";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_F32_ADD;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let lhs = cb.query_cell();
        let rhs = cb.query_cell();
        let out = cb.query_cell();

        let lhs_sign = cb.query_selector();
        let rhs_sign = cb.query_selector();
        let out_sign = cb.query_selector();

        let lhs_exp = cb.query_cell();
        let rhs_exp = cb.query_cell();
        let out_exp = cb.query_cell();

        let lhs_limbs = [cb.query_cell(), cb.query_cell(), cb.query_cell()];
        let rhs_limbs = [cb.query_cell(), cb.query_cell(), cb.query_cell()];
        let out_limbs = [cb.query_cell(), cb.query_cell(), cb.query_cell()];

        cb.range_check8(lhs_exp.current());
        cb.range_check8(rhs_exp.current());
        cb.range_check8(out_exp.current());

        for i in 0..2 {
            cb.range_check8(lhs_limbs[i].current());
            cb.range_check8(rhs_limbs[i].current());
            cb.range_check8(out_limbs[i].current());
        }

        cb.range_check7(lhs_limbs[2].current());
        cb.range_check7(rhs_limbs[2].current());
        cb.range_check7(out_limbs[2].current());

        cb.stack_pop(lhs.current());
        cb.stack_pop(rhs.current());
        cb.stack_push(out.current());

        Self {
            lhs,
            rhs,
            out,
            lhs_sign,
            rhs_sign,
            out_sign,
            lhs_exp,
            rhs_exp,
            out_exp,
            lhs_limbs,
            rhs_limbs,
            out_limbs,
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
        self.lhs.assign(region, offset, F::from(lhs.to_bits() as u64));
        self.rhs.assign(region, offset, F::from(rhs.to_bits() as u64));
        self.out.assign(region, offset, F::from(out.to_bits() as u64));
        println!("DEBUG OUT {}", out.to_bits() as u32);
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
            I32Const(0x12800025) // 00010010100000000000000000100101, 8.07797129917e-28
            I32Const(0x0d00001a) // 00001101000000000000000000011010, 3.94431675125e-31
            F32Add
            // Out   0x12801025     00010010100000000001000000100101, 8.08191560369e-28
            Drop
        });
    }

}
