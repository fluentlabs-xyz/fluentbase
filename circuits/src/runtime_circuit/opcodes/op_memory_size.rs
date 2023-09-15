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
pub struct OpMemorySizeGadget<F: Field> {
    value: AdviceColumn,
    marker: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpMemorySizeGadget<F> {
    const NAME: &'static str = "WASM_MEMORY_SIZE";
    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_MEMORY_SIZE;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let value = cb.query_cell();

        cb.context_lookup(RwTableContextTag::MemorySize, 0.expr(), value.current());
        cb.stack_push(value.current());

        Self {
            value,
            marker: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &ExecStep,
    ) -> Result<(), GadgetError> {
        debug_assert_eq!(
            trace.curr().memory_size,
            trace.next_nth_stack_value(0)?.to_bits() as u32
        );
        self.value
            .assign(region, offset, trace.curr().memory_size as u64);
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::instruction_set;

    #[test]
    fn test_simple_usage() {
        let code = instruction_set! {
            MemorySize
            Drop
        };
        test_ok(code);
    }

    #[test]
    fn test_with_grow() {
        let code = instruction_set! {
            // add 1 page
            I32Const(1)
            MemoryGrow
            Drop
            // size is 1
            MemorySize
            Drop
            // add 2 page2
            I32Const(2)
            MemoryGrow
            Drop
            // size is 3
            MemorySize
            Drop
        };
        test_ok(code);
    }
}
