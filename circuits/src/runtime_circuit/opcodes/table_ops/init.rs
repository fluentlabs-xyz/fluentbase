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
pub(crate) struct OpTableInitGadget<F: Field> {
    table_index_src: AdviceColumn,
    table_index_dst: AdviceColumn,
    start: AdviceColumn,
    range: AdviceColumn,
    size_src: AdviceColumn,
    size_dst: AdviceColumn,
    out: AdviceColumn,
    _pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpTableInitGadget<F> {
    const NAME: &'static str = "WASM_TABLE_INIT";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_TABLE_INIT;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let table_index_src = cb.query_rwasm_value();
        let table_index_dst = cb.query_rwasm_value();
        let start = cb.query_rwasm_value();
        let range = cb.query_rwasm_value();
        let size_src = cb.query_rwasm_value();
        let size_dst = cb.query_rwasm_value();
        let out = cb.query_rwasm_value();
        cb.table_size(table_index_src.expr(), size_src.expr());
        cb.table_size(table_index_dst.expr(), size_dst.expr());
        cb.table_init(table_index_src.expr(), table_index_dst.expr(), start.expr(), range.expr());
        cb.stack_pop(start.current());
        cb.stack_pop(range.current());
        cb.stack_push(out.current());
        cb.range_check_1024(start.expr());
        cb.range_check_1024(range.expr());
        cb.range_check_1024(size_src.expr());
        cb.range_check_1024(size_dst.expr());
        cb.range_check_1024(size_src.expr() - (start.expr() + range.expr()));
        cb.range_check_1024(size_dst.expr() - (start.expr() + range.expr()));
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
        trace: &TraceStep,
    ) -> Result<(), GadgetError> {
        let (table_index_dst, start, range, out) = match trace.instr() {
            Instruction::TableInit(ti) =>
                ( ti,
                  trace.curr_nth_stack_value(0)?,
                  trace.curr_nth_stack_value(1)?,
                  trace.next_nth_stack_value(0)?,
                ),
            _ => bail_illegal_opcode!(trace),
        };
        self.table_index_dst.assign(region, offset, F::from(table_index_dst.to_u32() as u64));
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
    fn table_init() {
        test_ok(instruction_set! {
            I32Const(0)
            I32Const(1)
            TableInit(0)
            TableGet(1)
            Drop
        });
    }
}
