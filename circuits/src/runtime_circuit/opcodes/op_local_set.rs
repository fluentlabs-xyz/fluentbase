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
pub(crate) struct OpLocalSetGadget<F: Field> {
    value: AdviceColumn,
    _pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpLocalSetGadget<F> {
    const NAME: &'static str = "WASM_LOCAL_SET";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_LOCAL_SET;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let index = cb.query_rwasm_value();
        let value = cb.query_cell();
        cb.stack_pop(value.expr());
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
            Instruction::LocalSet(_) => trace.curr_nth_stack_value(0)?,
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
    fn test_set_local() {
        let code = instruction_set! {
            .propagate_locals(2)
            // 1023: 0  <--+
            // 1022: 0     |
            // 1021: 100 --+
            I32Const(100)
            LocalSet(2) // [1023] = 100
            // 1023: 100
            // 1022: 0  <--+
            // 1021: 20 ---+
            I32Const(20)
            LocalSet(1)
            // 1023: 100 <---+
            // 1022: 20 <-+  |
            // 1021: 101 -|--+
            // 1020: 21 --+
            I32Const(101)
            LocalSet(2)
            I32Const(21)
            LocalSet(1)
        };
        test_ok(code);
    }
}
