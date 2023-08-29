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
pub(crate) struct TableFillGadget<F: Field> {
    table_index: AdviceColumn,
    start: AdviceColumn,
    value_type: AdviceColumn,
    value: AdviceColumn,
    range: AdviceColumn,
    size: AdviceColumn,
    _pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for TableFillGadget<F> {
    const NAME: &'static str = "WASM_TABLE_FILL";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_TABLE_FILL;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let table_index = cb.query_rwasm_value();
        let start = cb.query_rwasm_value();
        let value_type = cb.query_rwasm_value();
        let value = cb.query_rwasm_value();
        let range = cb.query_rwasm_value();
        let size = cb.query_rwasm_value();
        cb.table_size(table_index.expr(), size.expr());
        cb.table_fill(table_index.expr(), start.expr(), value.expr(), range.expr(), size.expr());
        cb.stack_pop(start.current());
        cb.stack_pop(value.current());
        cb.stack_pop(range.current());
        cb.range_check_1024(value.expr());
        cb.range_check_1024(range.expr());
        cb.range_check_1024(size.expr() - (value.expr() + range.expr()));
        Self {
            table_index,
            start,
            value_type,
            value,
            range,
            size,
            _pd: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &TraceStep,
    ) -> Result<(), GadgetError> {
        let (table_index, start, value_type, value, range) = match trace.instr() {
            Instruction::TableFill(ti) =>
                ( ti,
                  trace.curr_nth_stack_value(0)?,
                  trace.curr_nth_stack_value(1)?,
                  trace.curr_nth_stack_value(2)?,
                  trace.curr_nth_stack_value(3)?,
                ),
            _ => bail_illegal_opcode!(trace),
        };
        self.table_index.assign(region, offset, F::from(table_index.to_bits()));
        self.start.assign(region, offset, F::from(start.to_bits()));
        self.value_type.assign(region, offset, F::from(value_type.to_bits()));
        self.value.assign(region, offset, F::from(value.to_bits()));
        self.range.assign(region, offset, F::from(range.to_bits()));
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::instruction_set;

    #[test]
    fn table_fill() {
        test_ok_with_demo_table(instruction_set! {
            I32Const(0)
            I32Const(0)
            RefFunc(0)
            I32Const(2)
            TableFill(0)
            Drop
        });
    }
}
