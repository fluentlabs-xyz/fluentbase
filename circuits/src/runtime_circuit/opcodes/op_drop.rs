use crate::{
    bail_illegal_opcode,
    constraint_builder::AdviceColumn,
    runtime_circuit::{
        constraint_builder::OpConstraintBuilder,
        execution_state::ExecutionState,
        opcodes::{ExecutionGadget, GadgetError, TraceStep},
    },
    util::Field,
};
use fluentbase_rwasm::engine::bytecode::Instruction;
use halo2_proofs::circuit::Region;
use std::marker::PhantomData;

#[derive(Clone, Debug)]
pub(crate) struct DropGadget<F> {
    value: AdviceColumn,
    _pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for DropGadget<F> {
    const NAME: &'static str = "WASM_DROP";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_DROP;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let value = cb.query_rwasm_value();
        cb.stack_pop(value.expr());
        Self {
            value,
            _pd: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &TraceStep,
    ) -> Result<(), GadgetError> {
        match trace.instr() {
            Instruction::Drop => {}
            _ => bail_illegal_opcode!(trace),
        };
        self.value.assign(
            region,
            offset,
            F::from(trace.curr_nth_stack_value(0)?.to_bits()),
        );
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::instruction_set;

    #[test]
    fn test_drop() {
        test_ok(instruction_set! {
            I32Const[1]
            I32Const[2]
            I32Const[3]
            Drop
            Drop
            Drop
        });
    }
}
