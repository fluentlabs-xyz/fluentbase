use crate::{
    constraint_builder::{AdviceColumn, ToExpr},
    runtime_circuit::{
        constraint_builder::OpConstraintBuilder,
        execution_state::ExecutionState,
        opcodes::ExecutionGadget,
    },
    trace_step::{GadgetError, TraceStep},
    util::Field,
};
use halo2_proofs::circuit::Region;
use std::marker::PhantomData;

#[derive(Clone, Debug)]
pub(crate) struct OpSelectGadget<F: Field> {
    cond: AdviceColumn,
    cond_inv: AdviceColumn,
    val1: AdviceColumn,
    val2: AdviceColumn,
    res: AdviceColumn,
    vtype: AdviceColumn,
    pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpSelectGadget<F> {
    const NAME: &'static str = "WASM_SELECT";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_SELECT;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let cond = cb.query_cell();
        let cond_inv = cb.query_cell();
        let val1 = cb.query_cell();
        let val2 = cb.query_cell();
        let res = cb.query_cell();
        let vtype = cb.query_cell();

        cb.stack_pop(cond.expr());
        cb.stack_pop(val2.expr());
        cb.stack_pop(val1.expr());
        cb.stack_push(res.expr());

        cb.require_zeros(
            "op_select: cond is zero",
            vec![(1.expr() - cond.expr() * cond_inv.expr()) * (res.expr() - val2.expr())],
        );

        cb.require_zeros(
            "op_select: cond is not zero",
            vec![cond.expr() * (res.expr() - val1.expr())],
        );

        Self {
            cond,
            cond_inv,
            val1,
            val2,
            res,
            vtype,
            pd: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &TraceStep,
    ) -> Result<(), GadgetError> {
        let val1 = trace.curr_nth_stack_value(2)?;
        let val2 = trace.curr_nth_stack_value(1)?;
        let cond = trace.curr_nth_stack_value(0)?;
        let res = trace.next_nth_stack_value(0)?;

        self.cond.assign(region, offset, cond.to_bits());
        self.cond_inv.assign(
            region,
            offset,
            F::from(cond.as_u64()).invert().unwrap_or(F::zero()),
        );
        self.val2.assign(region, offset, val2.to_bits());
        self.val1.assign(region, offset, val1.to_bits());
        self.res.assign(region, offset, res.to_bits());

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::instruction_set;

    #[test]
    fn test_select_i32() {
        test_ok(instruction_set! {
            I32Const(1)
            I32Const(2)
            I32Const(0)
            Select
            Drop
        });
    }

    #[test]
    fn test_select_i64() {
        test_ok(instruction_set! {
            I64Const(1)
            I64Const(2)
            I64Const(0)
            Select
            Drop
        });
    }
}
