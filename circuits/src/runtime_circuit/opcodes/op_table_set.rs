use crate::{
    constraint_builder::{AdviceColumn, ToExpr},
    exec_step::MAX_TABLE_SIZE,
    gadgets::lt::LtGadget,
    runtime_circuit::{
        constraint_builder::OpConstraintBuilder,
        execution_state::ExecutionState,
        opcodes::{ExecStep, ExecutionGadget, GadgetError},
    },
    rw_builder::rw_row::RwTableContextTag,
    util::Field,
};
use fluentbase_runtime::ExitCode;
use halo2_proofs::circuit::Region;
use std::{marker::PhantomData, ops::Neg};

#[derive(Clone, Debug)]
pub(crate) struct OpTableSetGadget<F: Field> {
    elem: AdviceColumn,
    value: AdviceColumn,
    size: AdviceColumn,
    lt_gadget: LtGadget<F, 2>,
    _pd: PhantomData<F>,
}

const RANGE_THRESHOLD: usize = MAX_TABLE_SIZE * 2 + 1;

impl<F: Field> ExecutionGadget<F> for OpTableSetGadget<F> {
    const NAME: &'static str = "WASM_TABLE_SET";
    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_TABLE_SET;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let value = cb.query_cell();
        let elem = cb.query_cell();
        let size = cb.query_cell();

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

        // So in case of exit code causing error, pc delta must be different.
        // To solve this problem `configure_state_transition` is defined with disabled constraint.
        cb.condition(1.expr() - lt_gadget.expr(), |cb| {
            cb.next_pc_delta(9.expr());
        });

        cb.condition(lt_gadget.expr(), |cb| {
            cb.exit_code_lookup((ExitCode::TableOutOfBounds as i32 as u64).expr());
        });

        // Value is not important if exit code cause error, but we need to shift (rw_counter and
        // offset). TODO: Change `draft_shift` to something more correct.
        cb.condition(lt_gadget.expr(), |cb| {
            cb.draft_shift(2, 1);
        });
        cb.condition(1.expr() - lt_gadget.expr(), |cb| {
            cb.stack_pop(value.current());
        });

        cb.stack_pop(elem.current());

        // If exit code causing error than nothing is written, but we need to shift.
        cb.condition(lt_gadget.expr(), |cb| {
            cb.draft_shift(1, 0);
        });
        cb.condition(1.expr() - lt_gadget.expr(), |cb| {
            cb.table_elem_lookup(
                1.expr(),
                cb.query_rwasm_value(),
                elem.current(),
                value.current(),
            );
        });

        cb.context_lookup(
            RwTableContextTag::TableSize(cb.query_rwasm_value()),
            0.expr(),
            size.current(),
            None,
        );

        Self {
            elem,
            value,
            size,
            lt_gadget,
            _pd: Default::default(),
        }
    }

    fn configure_state_transition(cb: &mut OpConstraintBuilder<F>) {
        //cb.next_pc_delta(9.expr());
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &ExecStep,
    ) -> Result<(), GadgetError> {
        let table_index = trace.instr().aux_value().unwrap_or_default().as_u32();
        let value = trace.curr_nth_stack_value(0)?;
        let elem = trace.curr_nth_stack_value(1)?;
        let size = trace.read_table_size(table_index);
        self.elem.assign(region, offset, F::from(elem.to_bits()));
        self.value.assign(region, offset, F::from(value.to_bits()));
        self.size.assign(region, offset, F::from(size as u64));
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
    fn table_set_simple() {
        test_ok(instruction_set! {
            RefFunc(0)
            I32Const(2)
            TableGrow(0)
            Drop
            I32Const(1)
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
            I32Const(2)
            RefFunc(0)
            TableSet(0)
        });
    }
}
