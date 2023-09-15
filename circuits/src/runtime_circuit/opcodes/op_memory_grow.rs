use crate::{
    constraint_builder::{AdviceColumn, ToExpr},
    exec_step::{ExecStep, GadgetError},
    runtime_circuit::{
        constraint_builder::OpConstraintBuilder,
        execution_state::ExecutionState,
        opcodes::ExecutionGadget,
    },
    rw_builder::rw_row::RwTableContextTag,
    util::Field,
};
use halo2_proofs::circuit::Region;
use std::marker::PhantomData;

#[derive(Clone)]
pub struct OpMemoryGrowGadget<F: Field> {
    delta: AdviceColumn,
    memory_size: AdviceColumn,
    result: AdviceColumn,
    marker: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpMemoryGrowGadget<F> {
    const NAME: &'static str = "WASM_MEMORY_GROW";
    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_MEMORY_GROW;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let delta = cb.query_cell();
        let memory_size = cb.query_cell();
        let result = cb.query_cell();

        cb.stack_pop(delta.current());
        cb.stack_push(result.current());

        // TODO: "check memory expansion conditions"

        cb.context_lookup(
            RwTableContextTag::MemorySize,
            1.expr(),
            memory_size.current() + delta.current(),
            // TODO: "add prev lookup"
        );

        Self {
            delta,
            memory_size,
            result,
            marker: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &ExecStep,
    ) -> Result<(), GadgetError> {
        let delta = trace.curr_nth_stack_value(0)?;
        self.delta.assign(region, offset, delta.as_u64());
        let result = trace.next_nth_stack_value(0)?;
        self.result.assign(region, offset, result.as_u64());
        self.memory_size
            .assign(region, offset, trace.curr().memory_size as u64);
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::instruction_set;

    #[test]
    fn test_simple_grow() {
        let code = instruction_set! {
            I32Const(1)
            MemoryGrow
            Drop
        };
        test_ok(code);
    }

    #[test]
    fn test_multiple_grows() {
        let code = instruction_set! {
            I32Const(1)
            MemoryGrow
            Drop
            I32Const(2)
            MemoryGrow
            Drop
            I32Const(3)
            MemoryGrow
            Drop
        };
        test_ok(code);
    }
}
