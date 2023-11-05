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
};
use fluentbase_rwasm::engine::bytecode::Instruction;
use halo2_proofs::circuit::Region;
use std::marker::PhantomData;

#[derive(Clone, Debug)]
pub(crate) struct OpF32AddGadget<F: Field> {
    lhs: AdviceColumn,
    rhs: AdviceColumn,
    out: AdviceColumn,
    _pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpF32AddGadget<F> {
    const NAME: &'static str = "WASM_F32_ADD";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_F32_ADD;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let lhs = cb.query_cell();
        let rhs = cb.query_cell();
        let out = cb.query_cell();
        Self {
            lhs,
            rhs,
            out,
            _pd: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &ExecStep,
    ) -> Result<(), GadgetError> {
        let lhs = trace.curr_nth_stack_value(0)?;
        let rhs = trace.curr_nth_stack_value(1)?;
        let out = trace.next_nth_stack_value(0)?;
        self.lhs.assign(region, offset, F::from(lhs.to_bits() as u64));
        self.rhs.assign(region, offset, F::from(rhs.to_bits() as u64));
        self.out.assign(region, offset, F::from(out.to_bits() as u64));
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::instruction_set;

    #[test]
    fn table_f32_add_simple() {
        test_ok(instruction_set! {
            I32Const(0)
            I32Const(0)
            F32Add
            Drop
        });
    }

}
