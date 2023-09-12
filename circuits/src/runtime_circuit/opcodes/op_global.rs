use crate::{
    constraint_builder::{AdviceColumn, FixedColumn, ToExpr},
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
pub(crate) struct OpGlobalGadget<F: Field> {
    is_get_global: FixedColumn,
    is_set_global: FixedColumn,
    index: AdviceColumn,
    value: AdviceColumn,
    pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpGlobalGadget<F> {
    const NAME: &'static str = "WASM_GLOBAL";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_GLOBAL;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let is_get_global = cb.query_fixed();
        let is_set_global = cb.query_fixed();

        cb.require_equal(
            "op_global: selector",
            is_get_global.expr() + is_set_global.expr(),
            1.expr(),
        );

        let index = cb.query_cell();
        let value = cb.query_cell();

        cb.condition(is_get_global.expr(), |cb| {
            cb.require_opcode(Instruction::GlobalGet(Default::default()));
            cb.global_get(index.expr(), value.expr());
            cb.stack_push(value.expr());
        });

        cb.condition(is_set_global.expr(), |cb| {
            cb.require_opcode(Instruction::GlobalSet(Default::default()));
            cb.stack_pop(value.expr());
            cb.global_set(index.expr(), value.expr());
        });

        Self {
            is_set_global,
            is_get_global,
            index,
            value,
            pd: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &ExecStep,
    ) -> Result<(), GadgetError> {
        match trace.instr() {
            Instruction::GlobalGet(index) => {
                self.is_get_global.assign(region, offset, F::one());
                let value = trace.next_nth_stack_value(0)?;
                self.value.assign(region, offset, value.to_bits());
                self.index.assign(region, offset, index.to_u32() as u64);
            }
            Instruction::GlobalSet(index) => {
                self.is_set_global.assign(region, offset, F::one());
                let value = trace.curr_nth_stack_value(0)?;
                self.value.assign(region, offset, value.to_bits());
                self.index.assign(region, offset, index.to_u32() as u64);
            }
            _ => unreachable!("not supported opcode: {:?}", trace.instr()),
        };
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::instruction_set;

    #[test]
    fn test_global_get() {
        let code = instruction_set! {
            GlobalGet(0)
            GlobalGet(1)
            GlobalGet(2)
            Drop
            Drop
            Drop
        };
        test_ok(code);
    }

    #[test]
    fn test_global_set() {
        let code = instruction_set! {
            I32Const(-16383)
            GlobalSet(0)
            I32Const(16383)
            GlobalSet(1)
            GlobalGet(0)
            GlobalGet(1)
            Drop
            Drop
        };
        test_ok(code);
    }
}
