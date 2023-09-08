use crate::{
    bail_illegal_opcode,
    constraint_builder::{AdviceColumn, ToExpr},
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
pub(crate) struct OpTableFillGadget<F: Field> {
    table_index: AdviceColumn,
    start: AdviceColumn,
    value_type: AdviceColumn,
    value: AdviceColumn,
    range: AdviceColumn,
    size: AdviceColumn,
    out: AdviceColumn,
    _pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpTableFillGadget<F> {
    const NAME: &'static str = "WASM_TABLE_FILL";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_TABLE_FILL;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let table_index = cb.query_cell();
        let start = cb.query_cell();
        let value_type = cb.query_cell();
        let value = cb.query_cell();
        let range = cb.query_cell();
        let size = cb.query_cell();
        let out = cb.query_cell();
        cb.require_opcode(Instruction::TableFill(Default::default()));
        //cb.table_size(table_index.expr(), size.expr());
        //cb.table_fill(table_index.expr(), start.expr(), value.expr(), range.expr());
        cb.stack_pop(start.current());
        cb.stack_pop(value_type.current());
        cb.stack_pop(value.current());
        cb.stack_pop(range.current());
        cb.stack_push(out.current());
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
            out,
            _pd: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &TraceStep,
    ) -> Result<(), GadgetError> {
        let (table_index, start, value_type, value, range, out) = match trace.instr() {
            Instruction::TableFill(ti) =>
                ( ti,
                  trace.curr_nth_stack_value(0)?,
                  trace.curr_nth_stack_value(1)?,
                  trace.curr_nth_stack_value(2)?,
                  trace.curr_nth_stack_value(3)?,
                  trace.next_nth_stack_value(0)?,
                ),
            _ => bail_illegal_opcode!(trace),
        };
        self.table_index.assign(region, offset, F::from(table_index.to_u32() as u64));
        self.start.assign(region, offset, F::from(start.to_bits()));
        self.value_type.assign(region, offset, F::from(value_type.to_bits()));
        self.value.assign(region, offset, F::from(value.to_bits()));
        self.range.assign(region, offset, F::from(range.to_bits()));
        self.out.assign(region, offset, F::from(out.to_bits()));
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::instruction_set;

    #[test]
    fn table_fill() {
        test_ok(instruction_set! {
            I32Const(0)
            I32Const(0)
            RefFunc(0)
            I32Const(2)
            TableFill(0)
            Drop
        });
    }
}
