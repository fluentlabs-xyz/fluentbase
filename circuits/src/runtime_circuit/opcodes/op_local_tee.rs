use crate::{
    bail_illegal_opcode,
    constraint_builder::{AdviceColumn, ToExpr},
    runtime_circuit::{
        constraint_builder::OpConstraintBuilder,
        execution_state::ExecutionState,
        opcodes::{ExecStep, ExecutionGadget, GadgetError},
    },
    util::Field,
};
use fluentbase_rwasm::engine::bytecode::Instruction;
use halo2_proofs::circuit::Region;
use std::marker::PhantomData;

#[derive(Clone)]
pub(crate) struct OpLocalTeeGadget<F: Field> {
    value: AdviceColumn,
    _pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpLocalTeeGadget<F> {
    const NAME: &'static str = "WASM_LOCAL_TEE";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_LOCAL_TEE;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let index = cb.query_rwasm_value();
        let value = cb.query_cell();
        cb.stack_lookup(0.expr(), cb.stack_pointer(), value.expr());
        cb.stack_lookup(
            1.expr(),
            cb.stack_pointer() + index.clone() - 1.expr(),
            value.expr(),
        );
        Self {
            value,
            _pd: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &ExecStep,
    ) -> Result<(), GadgetError> {
        let value = match trace.instr() {
            Instruction::LocalTee(_) => trace.curr_nth_stack_value(0)?,
            _ => bail_illegal_opcode!(trace),
        };
        self.value.assign(region, offset, F::from(value.to_bits()));
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::instruction_set;

    #[test]
    fn test_tee_local() {
        let code = instruction_set! {
            .propagate_locals(3)
            I32Const(100)
            LocalTee(3)
            I32Const(20)
            LocalTee(2)
            I32Const(3)
            LocalTee(2)
            Drop
            Drop
            Drop
        };
        test_ok(code);
    }

    #[test]
    #[ignore]
    fn test_different_locals() {
        let code = instruction_set! {
            .propagate_locals(3)
            LocalGet(0)
            LocalGet(1)
            I32Add
            LocalSet(2)
            I32Const(0)
            LocalTee(2)
            Drop
        };
        test_ok(code);
    }
}
