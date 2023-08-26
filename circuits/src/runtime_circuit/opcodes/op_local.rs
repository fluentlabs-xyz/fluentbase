use crate::{
    bail_illegal_opcode,
    constraint_builder::{AdviceColumn, FixedColumn},
    runtime_circuit::{
        constraint_builder::{OpConstraintBuilder, ToExpr},
        execution_state::ExecutionState,
        opcodes::{ExecutionGadget, GadgetError, TraceStep},
    },
    util::Field,
};
use fluentbase_rwasm::engine::bytecode::Instruction;
use halo2_proofs::circuit::Region;
use std::marker::PhantomData;

#[derive(Clone)]
pub(crate) struct LocalGadget<F: Field> {
    is_get_local: FixedColumn,
    is_set_local: FixedColumn,
    is_tee_local: FixedColumn,
    index: AdviceColumn,
    value: AdviceColumn,
    _pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for LocalGadget<F> {
    const NAME: &'static str = "WASM_LOCAL";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_LOCAL;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let is_get_local = cb.query_fixed();
        let is_set_local = cb.query_fixed();
        let is_tee_local = cb.query_fixed();

        let index = cb.query_cell();
        let value = cb.query_cell();

        cb.require_equal(
            "op_local: selector",
            is_get_local.expr() + is_set_local.expr() + is_tee_local.expr(),
            1.expr(),
        );

        cb.condition(is_set_local.expr(), |cb| {
            cb.stack_pop(value.expr());
            cb.stack_lookup(
                1.expr(),
                cb.stack_pointer_offset() + index.expr(),
                value.expr(),
            );
        });

        cb.condition(is_get_local.expr(), |cb| {
            cb.stack_lookup(
                0.expr(),
                cb.stack_pointer_offset() + index.expr(),
                value.expr(),
            );
            cb.stack_push(value.expr());
        });

        cb.condition(is_tee_local.expr(), |cb| {
            cb.stack_pop(value.expr());
            cb.stack_lookup(
                1.expr(),
                cb.stack_pointer_offset() + index.expr() - 1.expr(),
                value.expr(),
            );
            cb.stack_push(value.expr());
        });

        Self {
            is_set_local,
            is_get_local,
            is_tee_local,
            index,
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
        let (selector, index, value) = match trace.instr() {
            Instruction::LocalGet(index) => (
                &self.is_get_local,
                index,
                trace.curr_nth_stack_value(index.to_usize())?,
            ),
            Instruction::LocalSet(index) => {
                (&self.is_set_local, index, trace.curr_nth_stack_value(0)?)
            }
            Instruction::LocalTee(index) => {
                (&self.is_tee_local, index, trace.curr_nth_stack_value(0)?)
            }
            _ => bail_illegal_opcode!(trace),
        };
        selector.assign(region, offset, F::one());
        self.index
            .assign(region, offset, F::from(index.to_usize() as u64));
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
            I32Const(100)
            LocalSet(0)
            I32Const(20)
            LocalSet(1)
            I32Const(100)
            I32Const(20)
            LocalSet(0)
            LocalSet(1)
        };
        test_ok(code);
    }

    #[test]
    fn test_tee_local() {
        let code = instruction_set! {
            .propagate_locals(1)
            I32Const(123)
            LocalTee(0)
            Drop
        };
        test_ok(code);
    }

    #[test]
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
