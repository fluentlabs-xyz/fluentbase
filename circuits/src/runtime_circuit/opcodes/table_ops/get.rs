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
pub(crate) struct OpTableGetGadget<F: Field> {
    table_index: AdviceColumn,
    elem_index: AdviceColumn,
    value: AdviceColumn,
    size: AdviceColumn,
    _pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpTableGetGadget<F> {
    const NAME: &'static str = "WASM_TABLE_GET";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_TABLE_GET;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let table_index = cb.query_cell();
        let elem_index = cb.query_cell();
        let value = cb.query_cell();
        let size = cb.query_cell();
        cb.require_opcode(Instruction::TableGet(Default::default()));
        cb.stack_pop(elem_index.current());
        cb.table_get(table_index.current(), elem_index.current(), value.current());
        cb.stack_push(value.current());
        cb.range_check_1024(elem_index.expr());
        cb.range_check_1024(size.expr() - elem_index.expr());
        cb.context_lookup(
            RwTableContextTag::TableSize(cb.query_rwasm_value()),
            0.expr(),
            size.current(),
            0.expr(),
        );
        Self {
            table_index,
            elem_index,
            value,
            size,
            _pd: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &ExecStep,
    ) -> Result<(), GadgetError> {
        let (table_index, elem_index, value) = match trace.instr() {
            Instruction::TableGet(ti) => (
                ti,
                trace.curr_nth_stack_value(0)?,
                trace.next_nth_stack_value(0)?,
            ),
            _ => bail_illegal_opcode!(trace),
        };
        self.table_index
            .assign(region, offset, F::from(table_index.to_u32() as u64));
        self.elem_index
            .assign(region, offset, F::from(elem_index.to_bits()));
        self.value.assign(region, offset, F::from(value.to_bits()));
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::instruction_set;

    #[test]
    fn table_get() {
        test_ok(instruction_set! {
            RefFunc(0)
            I32Const(2)
            TableGrow(0)
            Drop
            I32Const(0)
            RefFunc(0)
            TableSet(0)
            I32Const(0)
            TableGet(0)
            Drop
        });
    }
}
