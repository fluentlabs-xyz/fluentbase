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
pub(crate) struct OpTableSetGadget<F: Field> {
    elem: AdviceColumn,
    value: AdviceColumn,
    size: AdviceColumn,
    _pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpTableSetGadget<F> {
    const NAME: &'static str = "WASM_TABLE_SET";
    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_TABLE_SET;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let elem = cb.query_cell();
        let value = cb.query_cell();
        let size = cb.query_cell();
        cb.stack_pop(value.current());
        cb.stack_pop(elem.current());
        cb.table_elem_lookup(
            1.expr(),
            cb.query_rwasm_value(),
            elem.current(),
            value.current(),
        );
        cb.context_lookup(
            RwTableContextTag::TableSize(cb.query_rwasm_value()),
            0.expr(),
            size.current(),
            None,
        );
        // TODO: "table size overflow check"
        // cb.range_check_1024(elem.expr());
        // Minus one is needed here, for example if size of table is zero then zero elem_index will
        // require size to be one. cb.range_check_1024(size.expr() - elem_index.expr() -
        // 1_i32.expr());
        // cb.range_check_1024(size.current() - elem.current() - 1.expr());
        Self {
            elem,
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
        let table_index = trace.instr().aux_value().unwrap_or_default().as_u32();
        let elem = trace.curr_nth_stack_value(0)?;
        let value = trace.curr_nth_stack_value(1)?;
        let size = trace.read_table_size(table_index);
        self.elem.assign(region, offset, F::from(elem.to_bits()));
        self.value.assign(region, offset, F::from(value.to_bits()));
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
            RefFunc(0)
            TableSet(0)
        });
    }
}
