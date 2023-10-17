use crate::{
    bail_illegal_opcode,
    constraint_builder::{AdviceColumn, ToExpr},
    runtime_circuit::{
        constraint_builder::OpConstraintBuilder,
        execution_state::ExecutionState,
        opcodes::{ExecStep, ExecutionGadget, GadgetError},
    },
    rw_builder::copy_row::CopyTableTag,
    util::Field,
};
use fluentbase_rwasm::engine::bytecode::Instruction;
use halo2_proofs::circuit::Region;
use std::marker::PhantomData;

#[derive(Clone, Debug)]
pub(crate) struct OpTableFillGadget<F: Field> {
    table_index: AdviceColumn,
    start: AdviceColumn,
    value: AdviceColumn,
    range: AdviceColumn,
    size: AdviceColumn,
    _pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpTableFillGadget<F> {
    const NAME: &'static str = "WASM_TABLE_FILL";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_TABLE_FILL;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let table_index = cb.query_cell();
        let start = cb.query_cell();
        let value = cb.query_cell();
        let range = cb.query_cell();
        let size = cb.query_cell();
        cb.require_opcode(Instruction::TableFill(Default::default()));
        //cb.table_size(table_index.expr(), size.expr());
        //cb.table_fill(table_index.expr(), start.expr(), value.expr(), range.expr());

        cb.stack_pop(range.current());
        cb.stack_pop(value.current());
        cb.stack_pop(start.current());

        cb.range_check_1024(start.current());
        cb.range_check_1024(range.current() - 1.expr()); // Range must be non zero value, one or larger.
        cb.range_check_1024(size.current() - (start.current() + range.current()));

        cb.copy_lookup(
            CopyTableTag::FillTable,
            value.current(),
            table_index.current() * 1024 + start.current(),
            range.current(),
        );

        Self {
            table_index,
            start,
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
        trace: &ExecStep,
    ) -> Result<(), GadgetError> {
        let (table_index, start, value, range, size) = match trace.instr() {
            Instruction::TableFill(ti) => (
                ti,
                trace.curr_nth_stack_value(2)?,
                trace.curr_nth_stack_value(1)?,
                trace.curr_nth_stack_value(0)?,
                trace.read_table_size(ti.to_u32()),
            ),
            _ => bail_illegal_opcode!(trace),
        };
        self.table_index
            .assign(region, offset, F::from(table_index.to_u32() as u64));
        self.start.assign(region, offset, F::from(start.to_bits()));
        self.value.assign(region, offset, F::from(value.to_bits()));
        self.range.assign(region, offset, F::from(range.to_bits()));
        self.size.assign(region, offset, F::from(size as u64));
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
            RefFunc(0)
            I32Const(4)
            TableGrow(0)
            Drop
            I32Const(0)
            RefFunc(0)
            I32Const(4)
            TableFill(0)
        });
    }

    #[test]
    fn table_two_times_fill() {
        test_ok(instruction_set! {
            RefFunc(0)
            I32Const(4)
            TableGrow(0)
            Drop
            I32Const(0)
            RefFunc(0)
            I32Const(2)
            TableFill(0)
            I32Const(2)
            RefFunc(0)
            I32Const(2)
            TableFill(0)
        });
    }




}
