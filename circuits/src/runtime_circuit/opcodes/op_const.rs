use crate::{
    bail_illegal_opcode,
    exec_step::{ExecStep, GadgetError},
    runtime_circuit::{
        constraint_builder::OpConstraintBuilder,
        execution_state::ExecutionState,
        opcodes::ExecutionGadget,
    },
    util::Field,
};
use fluentbase_rwasm::engine::bytecode::Instruction;
use halo2_proofs::circuit::Region;
use std::marker::PhantomData;

#[derive(Clone, Debug)]
pub(crate) struct OpConstGadget<F: Field> {
    _pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpConstGadget<F> {
    const NAME: &'static str = "WASM_CONST";
    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_CONST;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        cb.stack_push(cb.query_rwasm_value());
        Self {
            _pd: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        _region: &mut Region<'_, F>,
        _offset: usize,
        trace: &ExecStep,
    ) -> Result<(), GadgetError> {
        let value = match trace.instr() {
            Instruction::I32Const(val) | Instruction::I64Const(val) => val,
            _ => bail_illegal_opcode!(trace),
        };
        debug_assert_eq!(trace.next_nth_stack_value(0)?, *value);
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::instruction_set;

    #[test]
    fn test_stack_top_offset() {
        test_ok(instruction_set! {
            I32Const(100)
            Drop
        });
    }

    #[test]
    fn test_stack_depth() {
        test_ok(instruction_set! {
            I32Const(100)
            I32Const(20)
            I32Const(3)
            Drop
            Drop
            Drop
        });
    }
}
