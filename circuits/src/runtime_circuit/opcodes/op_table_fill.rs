use crate::{
    constraint_builder::{AdviceColumn, ToExpr},
    exec_step::MAX_TABLE_SIZE,
    gadgets::lt::LtGadget,
    runtime_circuit::{
        constraint_builder::OpConstraintBuilder,
        execution_state::ExecutionState,
        opcodes::{ExecStep, ExecutionGadget, GadgetError},
    },
    rw_builder::copy_row::CopyTableTag,
    rw_builder::rw_row::RwTableContextTag,
    util::Field,
};
use fluentbase_runtime::ExitCode;
use halo2_proofs::circuit::Region;
use std::marker::PhantomData;

#[derive(Clone, Debug)]
pub(crate) struct OpTableFillGadget<F: Field> {
    start: AdviceColumn,
    value: AdviceColumn,
    range: AdviceColumn,
    size: AdviceColumn,
    lt_gadget: LtGadget<F, 2>,
    _pd: PhantomData<F>,
}

const RANGE_THRESHOLD: usize = MAX_TABLE_SIZE * 2 + 1;

impl<F: Field> ExecutionGadget<F> for OpTableFillGadget<F> {
    const NAME: &'static str = "WASM_TABLE_FILL";

    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_TABLE_FILL;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let start = cb.query_cell();
        let value = cb.query_cell();
        let range = cb.query_cell();
        let size = cb.query_cell();

        cb.range_check_1024(start.current());
        cb.range_check_1024(range.current());
        cb.range_check_1024(size.current());

        let last_point = size.current() - (start.current() + range.current()) - 1.expr();
        // 1024 - 1024 - 1 + 2049 is minimum value.
        // 1 or more is valid + 2048, so less than 2049 in error.
        // less than 2049 is out of valid range, so we checking it.
        // 2049 is RANGE_THRESHOLD now.
        let threshold = last_point + RANGE_THRESHOLD.expr();
        let lt_gadget = cb.lt_gadget(threshold, RANGE_THRESHOLD.expr());

        // So in case of exit code causing error, pc delta must be different.
        // To solve this problem `configure_state_transition` is defined with disabled constraint.
        cb.condition(1.expr() - lt_gadget.expr(), |cb| {
            cb.next_pc_delta(9.expr());
        });
 
        cb.condition(lt_gadget.expr(), |cb| {
            // make sure proper exit code is set
            cb.exit_code_lookup((ExitCode::TableOutOfBounds as i32 as u64).expr());

            // If exit code causing error than nothing is written, but we need to shift.
            cb.draft_shift(1, 0);
        });

        cb.stack_pop(range.current());
        cb.stack_pop(value.current());
        cb.stack_pop(start.current());

        cb.context_lookup(
            RwTableContextTag::TableSize(cb.query_rwasm_value()),
            0.expr(),
            size.current(),
            None,
        );

        cb.condition(1.expr() - lt_gadget.expr(), |cb| {
            cb.copy_lookup(
                CopyTableTag::FillTable,
                    value.current(),
                    cb.query_rwasm_value() * MAX_TABLE_SIZE.expr() + start.current(),
                    range.current(),
            );
        });

        Self {
            start,
            value,
            range,
            size,
            lt_gadget,
            _pd: Default::default(),
        }
    }

    fn configure_state_transition(_cb: &mut OpConstraintBuilder<F>) {
        //cb.next_pc_delta(9.expr());
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &ExecStep,
    ) -> Result<(), GadgetError> {
        let table_index = trace.instr().aux_value().unwrap_or_default().as_u32();
        let start = trace.curr_nth_stack_value(2)?;
        let value = trace.curr_nth_stack_value(1)?;
        let range = trace.curr_nth_stack_value(0)?;
        let size = trace.read_table_size(table_index);
        self.start.assign(region, offset, F::from(start.to_bits()));
        self.value.assign(region, offset, F::from(value.to_bits()));
        self.range.assign(region, offset, F::from(range.to_bits()));
        self.size.assign(region, offset, F::from(size as u64));
        /*
        if start.to_bits() + range.to_bits() < size as u64 {
            let value = trace.next_nth_stack_value(0)?;
            self.value.assign(region, offset, F::from(value.to_bits()));
        }
        */
        self.lt_gadget.assign(
            region,
            offset,
            F::from(
                (size as i64 - (start.to_bits() + range.to_bits()) as i64 - 1 + RANGE_THRESHOLD as i64) as u64,
            ),
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
    fn table_fill_simple() {
        test_ok(instruction_set! {
            RefFunc(0)
            I32Const(4)
            TableGrow(0)
            Drop
            I32Const(0)
            RefFunc(0)
            I32Const(2)
            TableFill(0)
        });
    }

    #[test]
    fn table_fill_after_set() {
        test_ok(instruction_set! {
            RefFunc(0)
            I32Const(4)
            TableGrow(0)
            Drop

            I32Const(0)
            I32Const(2) // TODO: RefFunc(2)
            TableSet(0)

            I32Const(0)
            I32Const(3) // TODO: RefFunc(3)
            I32Const(2)
            TableFill(0)
        });
    }

    #[test]
    fn table_fill_out_of_bounds() {
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

    #[test]
    fn table_fill_after_set_two_times() {
        test_ok(instruction_set! {
            RefFunc(0)
            I32Const(4)
            TableGrow(0)
            Drop

            I32Const(1)
            I32Const(4) // TODO: RefFunc(4)
            TableSet(0)

            I32Const(2)
            I32Const(5) // TODO: RefFunc(5)
            TableSet(0)

            I32Const(1)
            I32Const(3) // TODO: RefFunc(3)
            I32Const(2)
            TableFill(0)

            I32Const(2)
            I32Const(6) // TODO: RefFunc(6)
            TableSet(0)

            I32Const(2)
            I32Const(7) // TODO: RefFunc(7)
            I32Const(2)
            TableFill(0)
        });
    }
}
