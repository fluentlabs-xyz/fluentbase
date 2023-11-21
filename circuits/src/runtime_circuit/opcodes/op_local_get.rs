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
pub(crate) struct OpLocalGetGadget<F: Field> {
    value: AdviceColumn,
    _pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpLocalGetGadget<F> {
    const NAME: &'static str = "WASM_LOCAL_GET";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_LOCAL_GET;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let index = cb.query_rwasm_value();
        let value = cb.query_cell();
        cb.stack_lookup(0.expr(), cb.stack_pointer() + index.clone(), value.expr());
        cb.stack_push(value.expr());
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
            Instruction::LocalGet(index) => trace.curr_nth_stack_value(index.to_usize())?,
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
    fn test_get_local() {
        let code = instruction_set! {
            .propagate_locals(2)
            LocalGet(0)
            Drop
            LocalGet(1)
            Drop
            LocalGet(0)
            LocalGet(1)
            Drop
            Drop
        };
        test_ok(code);
    }
}
