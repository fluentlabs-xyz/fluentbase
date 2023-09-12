use crate::{
    bail_illegal_opcode,
    constraint_builder::{AdviceColumn, FixedColumn, ToExpr},
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
pub(crate) struct OpLocalGadget<F: Field> {
    is_get_local: FixedColumn,
    is_set_local: FixedColumn,
    is_tee_local: FixedColumn,
    value: AdviceColumn,
    _pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpLocalGadget<F> {
    const NAME: &'static str = "WASM_LOCAL";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_LOCAL;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let is_get_local = cb.query_fixed();
        let is_set_local = cb.query_fixed();
        let is_tee_local = cb.query_fixed();

        let index = cb.query_rwasm_value();
        let value = cb.query_cell();

        cb.require_equal(
            "op_local: selector",
            is_get_local.expr() + is_set_local.expr() + is_tee_local.expr(),
            1.expr(),
        );

        cb.condition(is_get_local.expr(), |cb| {
            cb.require_opcode(Instruction::LocalGet(Default::default()));
            cb.stack_lookup(0.expr(), cb.stack_pointer() + index.clone(), value.expr());
            cb.stack_push(value.expr());
        });

        cb.condition(is_set_local.expr(), |cb| {
            cb.require_opcode(Instruction::LocalSet(Default::default()));
            cb.stack_pop(value.expr());
            cb.stack_lookup(
                1.expr(),
                cb.stack_pointer() + index.clone() - 1.expr(),
                value.expr(),
            );
        });

        cb.condition(is_tee_local.expr(), |cb| {
            cb.require_opcode(Instruction::LocalTee(Default::default()));
            cb.stack_lookup(0.expr(), cb.stack_pointer(), value.expr());
            cb.stack_lookup(
                1.expr(),
                cb.stack_pointer() + index.clone() - 1.expr(),
                value.expr(),
            );
        });

        Self {
            is_set_local,
            is_get_local,
            is_tee_local,
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
        let (selector, value) = match trace.instr() {
            Instruction::LocalGet(index) => (
                &self.is_get_local,
                trace.curr_nth_stack_value(index.to_usize())?,
            ),
            Instruction::LocalSet(_) => (&self.is_set_local, trace.curr_nth_stack_value(0)?),
            Instruction::LocalTee(_) => (&self.is_tee_local, trace.curr_nth_stack_value(0)?),
            _ => bail_illegal_opcode!(trace),
        };
        selector.assign(region, offset, F::one());
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
