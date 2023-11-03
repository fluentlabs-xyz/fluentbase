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
pub struct OpMemoryCopyGadget<F: Field> {
    dest: AdviceColumn,
    source: AdviceColumn,
    len: AdviceColumn,
    marker: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpMemoryCopyGadget<F> {
    const NAME: &'static str = "WASM_MEMORY_COPY";
    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_MEMORY_COPY;

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let dest = cb.query_cell();
        let source = cb.query_cell();
        let len = cb.query_cell();

        cb.stack_pop(len.current());
        cb.stack_pop(source.current());
        cb.stack_pop(dest.current());
        cb.copy_lookup(
            CopyTableTag::CopyMemory,
            source.current(),
            dest.current(),
            len.current(),
        );

        Self {
            dest,
            source,
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
        let source = trace.curr_nth_stack_value(1)?;
        let dest = trace.curr_nth_stack_value(2)?;
        self.len.assign(region, offset, len.as_u64());
        self.source.assign(region, offset, source.as_u64());
        self.dest.assign(region, offset, dest.as_u64());
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_rwasm::instruction_set;

    #[test]
    fn test_simple_copy() {
        let default_memory = vec![0x01, 0x02, 0x03, 0x04, 0x05];
        let code = instruction_set! {
            .add_memory(0, default_memory.as_slice())
            I32Const(5) // dest
            I32Const(0) // source
            I32Const(5) // len
            MemoryCopy
        };
        test_ok(code);
    }

    #[test]
    fn test_simple_copy_another_direction() {
        let default_memory_0 = vec![0x01, 0x02, 0x03, 0x04, 0x05];
        let default_memory_5 = vec![0x0, 0x0, 0x0, 0x0, 0x0];
        let code = instruction_set! {
            .add_memory(0, default_memory_0.as_slice())
            .add_memory(5, default_memory_5.as_slice())
            I32Const(0) // dest
            I32Const(5) // source
            I32Const(5) // len
            MemoryCopy
        };
        test_ok(code);
    }

    // TODO: fix problem with test.
    #[test]
    fn test_copy_with_set_first() {
        let code = instruction_set! {
            .add_memory(0, &[0; 40])
            .add_memory(1, &[0; 40])

            I32Const[0]
            I64Const[1]
            I64Store[0]

            I32Const[1]
            I64Const[2]
            I64Store[0]

            I32Const[2]
            I64Const[3]
            I64Store[0]

            I32Const[3]
            I64Const[4]
            I64Store[0]

            I32Const(40) // dest
            I32Const(0) // source
            I32Const(16) // len
            MemoryCopy
        };
        test_ok(code);
    }

    // TODO: fix problem with test.
    #[test]
    fn test_copy_with_set_second() {
        let code = instruction_set! {
            .add_memory(0, &[0; 40])
            .add_memory(1, &[0; 40])

            I32Const[0]
            I64Const[1]
            I64Store[10]

            I32Const[1]
            I64Const[2]
            I64Store[10]

            I32Const[2]
            I64Const[3]
            I64Store[10]

            I32Const[3]
            I64Const[4]
            I64Store[10]

            I32Const(40) // dest
            I32Const(0) // source
            I32Const(16) // len
            MemoryCopy
        };
        test_ok(code);
    }

}
