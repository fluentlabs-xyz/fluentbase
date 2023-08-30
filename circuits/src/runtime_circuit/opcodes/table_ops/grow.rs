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
pub(crate) struct TableGrowGadget<F: Field> {
    table_index: AdviceColumn,
    init_val: AdviceColumn,
    grow_val: AdviceColumn,
    res_val: AdviceColumn,
    _pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for TableGrowGadget<F> {
    const NAME: &'static str = "WASM_TABLE_GROW";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_TABLE_GROW;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let table_index = cb.query_rwasm_value();
        let init_val = cb.query_rwasm_value();
        let grow_val = cb.query_rwasm_value();
        let res_val = cb.query_rwasm_value();
        cb.table_grow(table_index.expr(), init_val.expr(), grow_val.expr(), res_val.expr());
        cb.stack_pop(init_val.current());
        cb.stack_pop(grow_val.current());
        cb.stack_push(res_val.current());
        Self {
            table_index,
            init_val,
            grow_val,
            res_val,
            _pd: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &TraceStep,
    ) -> Result<(), GadgetError> {
        let (table_index, init_val, grow_val, res_val) = match trace.instr() {
            Instruction::TableGrow(ti) =>
                ( ti,
                  trace.curr_nth_stack_value(0)?,
                  trace.curr_nth_stack_value(1)?,
                  trace.curr_nth_stack_value(2)?,
                ),
            _ => bail_illegal_opcode!(trace),
        };
        self.table_index.assign(region, offset, F::from(table_index.to_u32() as u64));
        self.init_val.assign(region, offset, F::from(init_val.to_bits()));
        self.grow_val.assign(region, offset, F::from(grow_val.to_bits()));
        self.res_val.assign(region, offset, F::from(res_val.to_bits()));
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok_with_demo_table;
    use fluentbase_rwasm::instruction_set;

    #[test]
    fn table_grow() {
        test_ok_with_demo_table(instruction_set! {
            RefFunc(0)
            I32Const(0)
            TableGrow(0)
            Drop
        });
    }
}
