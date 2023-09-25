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
pub(crate) struct OpTableCopyGadget<F: Field> {
    table_index_src: AdviceColumn,
    table_index_dst: AdviceColumn,
    start: AdviceColumn,
    range: AdviceColumn,
    size_src: AdviceColumn,
    size_dst: AdviceColumn,
    out: AdviceColumn,
    _pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpTableCopyGadget<F> {
    const NAME: &'static str = "WASM_TABLE_COPY";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_TABLE_COPY;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let table_index_src = cb.query_cell();
        let table_index_dst = cb.query_cell();
        let start = cb.query_cell();
        let range = cb.query_cell();
        let size_src = cb.query_cell();
        let size_dst = cb.query_cell();
        let out = cb.query_cell();
        cb.require_opcode(Instruction::TableCopy(Default::default()));
        cb.table_size(table_index_src.current(), size_src.current());
        cb.table_size(table_index_dst.current(), size_dst.current());
/*
        cb.table_copy(
            table_index_src.expr(),
            table_index_dst.expr(),
            start.expr(),
            range.expr(),
        );
*/
        cb.stack_pop(start.current());
        cb.stack_pop(range.current());
        cb.stack_push(out.current());
        cb.range_check_1024(start.current());
        cb.range_check_1024(range.current());
        cb.range_check_1024(size_src.current());
        cb.range_check_1024(size_dst.current());
        cb.range_check_1024(size_src.current() - (start.current() + range.current()));
        cb.range_check_1024(size_dst.current() - (start.current() + range.current()));
        cb.copy_lookup(
            CopyTableTag::CopyTable,
            table_index_src.current() * 1024.expr() + start.current(),
            table_index_dst.current() * 1024.expr() + start.current(),
            range.current(),
        );

        Self {
            table_index_src,
            table_index_dst,
            start,
            range,
            size_src,
            size_dst,
            out,
            _pd: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &ExecStep,
    ) -> Result<(), GadgetError> {
        let (table_index_dst, start, range, out) = match trace.instr() {
            Instruction::TableCopy(ti) => (
                ti,
                trace.curr_nth_stack_value(0)?,
                trace.curr_nth_stack_value(1)?,
                trace.next_nth_stack_value(0)?,
            ),
            _ => bail_illegal_opcode!(trace),
        };
        self.table_index_dst
            .assign(region, offset, F::from(table_index_dst.to_u32() as u64));
        self.start.assign(region, offset, F::from(start.to_bits()));
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
    fn table_copy() {
        test_ok(instruction_set! {
            I32Const(0)
            I32Const(1)
            TableInit(0)
            TableGet(1)
            Drop
        });
    }
}
