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
pub(crate) struct ConstGadget<F: Field> {
    value: AdviceColumn,
    _pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for ConstGadget<F> {
    const NAME: &'static str = "WASM_CONST";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_CONST;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let value = cb.query_value();
        cb.stack_push(value.current());
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
        let value = match trace.instr() {
            Instruction::I32Const(val) | Instruction::I64Const(val) => val,
            _ => bail_illegal_opcode!(trace),
        };
        debug_assert_eq!(trace.next_nth_stack_value(0)?, *value);
        self.value.assign(region, offset, F::from(value.to_bits()));
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::instruction_set;

    #[test]
    fn push_gadget_simple() {
        test_ok(instruction_set! {
            I32Const(100)
            Drop
        });
    }
}
