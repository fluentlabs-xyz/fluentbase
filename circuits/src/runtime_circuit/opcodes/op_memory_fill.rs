use crate::{
    constraint_builder::AdviceColumn,
    exec_step::{ExecStep, GadgetError},
    runtime_circuit::{
        constraint_builder::OpConstraintBuilder,
        execution_state::ExecutionState,
        opcodes::ExecutionGadget,
    },
    rw_builder::copy_row::CopyTableTag,
    util::Field,
};
use halo2_proofs::circuit::Region;
use std::marker::PhantomData;

#[derive(Clone)]
pub struct OpMemoryFillGadget<F: Field> {
    dest: AdviceColumn,
    value: AdviceColumn,
    len: AdviceColumn,
    marker: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpMemoryFillGadget<F> {
    const NAME: &'static str = "WASM_MEMORY_FILL";
    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_MEMORY_FILL;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let dest = cb.query_cell();
        let value = cb.query_cell();
        let len = cb.query_cell();

        cb.stack_pop(len.current());
        cb.stack_pop(value.current());
        cb.stack_pop(dest.current());
        cb.copy_lookup(
            CopyTableTag::FillMemory,
            // we use value as from address, read copy circuit for this
            value.current(),
            dest.current(),
            len.current(),
        );

        Self {
            dest,
            value,
            len,
            marker: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &ExecStep,
    ) -> Result<(), GadgetError> {
        let len = trace.curr_nth_stack_value(0)?;
        let value = trace.curr_nth_stack_value(1)?;
        let dest = trace.curr_nth_stack_value(2)?;
        self.len.assign(region, offset, len.as_u64());
        self.value
            .assign(region, offset, value.as_u64() as u8 as u64);
        self.dest.assign(region, offset, dest.as_u64());
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::instruction_set;

    #[test]
    fn test_simple_usage() {
        let default_memory = vec![0x01, 0x02, 0x03, 0x04, 0x05];
        let code = instruction_set! {
            .add_memory(0, default_memory.as_slice())
            I32Const(0)
            I32Const(0xff)
            I32Const(5)
            MemoryFill
            I32Const(0)
            I32Const(0x7f)
            I32Const(5)
            MemoryFill
        };
        test_ok(code);
    }
}
