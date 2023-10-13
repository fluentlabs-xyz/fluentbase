use crate::{
    constraint_builder::{AdviceColumn, ToExpr},
    exec_step::MAX_TABLE_SIZE,
    runtime_circuit::{
        constraint_builder::OpConstraintBuilder,
        execution_state::ExecutionState,
        opcodes::{ExecStep, ExecutionGadget, GadgetError},
    },
    rw_builder::{copy_row::CopyTableTag, rw_row::RwTableContextTag},
    util::Field,
};
use halo2_proofs::circuit::Region;
use std::marker::PhantomData;

#[derive(Clone, Debug)]
pub(crate) struct OpTableGrowGadget<F: Field> {
    init: AdviceColumn,
    delta: AdviceColumn,
    res: AdviceColumn,
    _pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpTableGrowGadget<F> {
    const NAME: &'static str = "WASM_TABLE_GROW";
    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_TABLE_GROW;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let init = cb.query_cell();
        let delta = cb.query_cell();
        let res = cb.query_cell();
        cb.stack_pop(delta.current());
        cb.stack_pop(init.current());
        cb.stack_push(res.current());
        cb.context_lookup(
            RwTableContextTag::TableSize(cb.query_rwasm_value()),
            1.expr(),
            res.current() + delta.current(),
            Some(res.current()),
        );
        cb.copy_lookup(
            CopyTableTag::FillTable,
            init.current(),
            cb.query_rwasm_value() * MAX_TABLE_SIZE.expr() + res.current(),
            delta.current(),
        );
        Self {
            init,
            delta,
            res,
            _pd: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &ExecStep,
    ) -> Result<(), GadgetError> {
        let delta = trace.curr_nth_stack_value(0)?;
        self.delta.assign(region, offset, F::from(delta.to_bits()));
        let init = trace.curr_nth_stack_value(1)?;
        self.init.assign(region, offset, F::from(init.to_bits()));
        let res = trace.next_nth_stack_value(0)?;
        self.res.assign(region, offset, F::from(res.to_bits()));
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::instruction_set;

    #[test]
    fn table_grow() {
        test_ok(instruction_set! {
            RefFunc(0)
            I32Const(2)
            TableGrow(0)
            Drop
        });
    }

    #[test]
    fn table_grow_for_two_tables() {
        test_ok(instruction_set! {
            RefFunc(0)
            I32Const(1)
            TableGrow(0)
            Drop
            RefFunc(0)
            I32Const(2)
            TableGrow(1)
            Drop
        });
    }

    #[test]
    fn table_grow_in_row() {
        test_ok(instruction_set! {
            RefFunc(0)
            I32Const(2)
            TableGrow(0)
            Drop
            RefFunc(0)
            I32Const(3)
            TableGrow(0)
            Drop
        });
    }
}
