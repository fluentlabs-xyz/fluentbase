use crate::{
    constraint_builder::{AdviceColumn, ToExpr},
    runtime_circuit::{
        constraint_builder::OpConstraintBuilder,
        execution_state::ExecutionState,
        opcodes::{ExecStep, ExecutionGadget, GadgetError},
    },
    rw_builder::rw_row::RwTableContextTag,
    util::Field,
    gadgets::lt::LtGadget,
    exec_step::MAX_TABLE_SIZE,
};
use halo2_proofs::circuit::Region;
use std::marker::PhantomData;
use fluentbase_runtime::ExitCode;
use std::ops::Neg;

#[derive(Clone, Debug)]
pub(crate) struct OpTableSetGadget<F: Field> {
    elem: AdviceColumn,
    value: AdviceColumn,
    size: AdviceColumn,
    exit_code: AdviceColumn,
    lt_gadget: LtGadget<F, 8>,
    _pd: PhantomData<F>,
}

const RANGE_THRESHOLD: usize = MAX_TABLE_SIZE * 2 + 1;

impl<F: Field> ExecutionGadget<F> for OpTableSetGadget<F> {
    const NAME: &'static str = "WASM_TABLE_SET";
    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_TABLE_SET;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let elem = cb.query_cell();
        let value = cb.query_cell();
        let size = cb.query_cell();
        let exit_code = cb.query_cell();
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
        cb.exit_code_lookup(exit_code.current());

        // cb.range_check_1024(elem.expr());
        // Minus one is needed here, for example if size of table is zero then zero elem_index will
        // require size to be one. cb.range_check_1024(size.expr() - elem_index.expr() -
        // 1_i32.expr());

        // Checking that out values is in range, to do `lt` comparsion condition.
        cb.range_check_1024(size.current());
        cb.range_check_1024(elem.current());
        let range = size.current() - elem.current() - 1.expr();
        // 1024 - 1024 - 1 + 2049 is minimum value.
        // 1 or more is valid + 2048, so less than 2049 in error.
        // less than 2049 is out of valid range, so we checking it.
        // 2049 is RANGE_THRESHOLD now.
        let threshold = range + RANGE_THRESHOLD.expr();
        let lt_gadget = cb.lt_gadget(threshold, RANGE_THRESHOLD.expr());
        cb.condition(lt_gadget.expr(), |cb| {
            cb.require_zero("exit code must be valid", exit_code.current() - (ExitCode::TableOutOfBounds as i32).neg().expr());
        });

        Self {
            elem,
            value,
            size,
            exit_code,
            lt_gadget,
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
        self.exit_code.assign(region, offset, F::from(0_u64));
        self.lt_gadget.assign(
            region,
            offset,
            F::from((size as i64 - elem.to_bits() as i64 - 1 + RANGE_THRESHOLD as i64) as u64),
            F::from(RANGE_THRESHOLD as u64),
        );
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

    #[test]
    fn table_set_out_of_bounds() {
        test_ok(instruction_set! {
            RefFunc(0)
            I32Const(2)
            TableGrow(0)
            Drop
            I32Const(3)
            RefFunc(0)
            TableSet(0)
        });
    }

}
