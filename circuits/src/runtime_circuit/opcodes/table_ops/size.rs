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
pub(crate) struct TableSizeGadget<F: Field> {
    table_index: AdviceColumn,
    value: AdviceColumn,
    _pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for TableSizeGadget<F> {
    const NAME: &'static str = "WASM_TABLE_SIZE";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_TABLE_SIZE;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let table_index = cb.query_rwasm_value();
        let value = cb.query_rwasm_value();
        cb.table_size(table_index.expr(), value.expr());
        cb.stack_push(value.current());
        Self {
            table_index,
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
        let (table_index, value) = match trace.instr() {
            Instruction::TableSize(ti) => (ti, trace.curr_nth_stack_value(0)?),
            _ => bail_illegal_opcode!(trace),
        };
        self.table_index.assign(region, offset, F::from(table_index.to_bits()));
        self.value.assign(region, offset, F::from(value.to_bits()));
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::instruction_set;

    #[test]
    fn table_size() {
        test_ok_with_demo_table(instruction_set! {
            TableSize(0)
            Drop
        });
    }
}
