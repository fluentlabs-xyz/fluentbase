use crate::{
    bail_illegal_opcode,
    constraint_builder::{AdviceColumn, ToExpr},
    runtime_circuit::{
        constraint_builder::OpConstraintBuilder,
        execution_state::ExecutionState,
        opcodes::{ExecStep, ExecutionGadget, GadgetError},
    },
    util::Field,
};
use fluentbase_rwasm::engine::bytecode::Instruction;
use halo2_proofs::circuit::Region;
use std::marker::PhantomData;

#[derive(Clone, Debug)]
pub(crate) struct OpTableSetGadget<F: Field> {
    table_index: AdviceColumn,
    elem_index: AdviceColumn,
    elem_type: AdviceColumn,
    value: AdviceColumn,
    size: AdviceColumn,
    out: AdviceColumn,
    _pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpTableSetGadget<F> {
    const NAME: &'static str = "WASM_TABLE_SET";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_TABLE_SET;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let table_index = cb.query_cell();
        let elem_index = cb.query_cell();
        let elem_type = cb.query_cell();
        let value = cb.query_cell();
        let size = cb.query_cell();
        let out = cb.query_cell();
        cb.require_opcode(Instruction::TableSet(Default::default()));
        cb.stack_pop(elem_type.current());
        cb.stack_pop(elem_index.current());
        cb.stack_pop(value.current());
        //cb.table_size(table_index.expr(), size.expr());
        cb.table_set(table_index.expr(), elem_index.expr(), value.expr());
        cb.stack_push(out.current());
        cb.range_check_1024(elem_index.expr());
        // Minus one is needed here, for example if size of table is zero then zero elem_index will require size to be one.
        //cb.range_check_1024(size.expr() - elem_index.expr() - 1_i32.expr());
        Self {
            table_index,
            elem_index,
            elem_type,
            value,
            size,
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
        let (table_index, elem_type, elem_index, value, out, size) = match trace.instr() {
            Instruction::TableSet(ti) => (
                ti,
                trace.curr_nth_stack_value(0)?,
                trace.curr_nth_stack_value(1)?,
                trace.curr_nth_stack_value(2)?,
                trace.next_nth_stack_value(0)?,
                trace.read_table_size(ti.to_u32()),
            ),
            _ => bail_illegal_opcode!(trace),
        };
        self.table_index
            .assign(region, offset, F::from(table_index.to_u32() as u64));
        self.elem_type
            .assign(region, offset, F::from(elem_type.to_bits()));
        self.elem_index
            .assign(region, offset, F::from(elem_index.to_bits()));
        self.value.assign(region, offset, F::from(value.to_bits()));
        self.out.assign(region, offset, F::from(out.to_bits()));
        println!("DEBUG TABLE SIZE {size}");
        self.size.assign(region, offset, F::from(size as u64));
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::instruction_set;

    #[test]
    fn table_set() {
        test_ok(instruction_set! {
            RefFunc(0)
            I32Const(2)
            TableGrow(0)
            Drop
            I32Const(0)
            I32Const(0)
            RefFunc(0)
            TableSet(0)
            Drop
        });
    }
}
