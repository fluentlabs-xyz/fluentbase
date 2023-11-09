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
pub(crate) struct OpTableSizeGadget<F: Field> {
    table_index: AdviceColumn,
    value: AdviceColumn,
    _pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpTableSizeGadget<F> {
    const NAME: &'static str = "WASM_TABLE_SIZE";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_TABLE_SIZE;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let table_index = cb.query_cell();
        let value = cb.query_cell();
        cb.require_opcode(Instruction::TableSize(Default::default()));
        //cb.table_size(table_index.expr(), value.expr());
        cb.context_lookup(
            RwTableContextTag::TableSize(table_index.current()),
            0.expr(),
            value.current(),
            None,
        );
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
        trace: &ExecStep,
    ) -> Result<(), GadgetError> {
        let (table_index, value) = match trace.instr() {
            Instruction::TableSize(ti) => (ti, trace.next_nth_stack_value(0)?),
            _ => bail_illegal_opcode!(trace),
        };
        self.table_index
            .assign(region, offset, F::from(table_index.to_u32() as u64));
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
        test_ok(instruction_set! {
            TableSize(0)
            Drop
        });
    }

    #[test]
    fn table_size_with_grow() {
        test_ok(instruction_set! {
            RefFunc(0)
            I32Const(6)
            TableGrow(0)
            Drop
            TableSize(0)
            Drop
        });
    }

    #[test]
    fn table_size_multi() {
        test_ok(instruction_set! {
            RefFunc(0)
            I32Const(6)
            TableGrow(0)
            Drop
            TableSize(0)
            Drop

            RefFunc(0)
            I32Const(3)
            TableGrow(0)
            Drop
            TableSize(0)
            Drop

            RefFunc(0)
            I32Const(2)
            TableGrow(0)
            Drop
            TableSize(0)
            Drop
        });
    }

}
