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
use fluentbase_runtime::SysFuncIdx;
use halo2_proofs::circuit::Region;
use std::marker::PhantomData;

#[derive(Clone)]
pub struct SysWriteGadget<F: Field> {
    target: AdviceColumn,
    offset: AdviceColumn,
    length: AdviceColumn,
    pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for SysWriteGadget<F> {
    const NAME: &'static str = "WASM_CALL_HOST(_sys_write)";
    const EXECUTION_STATE: ExecutionState =
        ExecutionState::WASM_CALL_HOST(SysFuncIdx::IMPORT_SYS_WRITE);

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let target = cb.query_cell();
        let offset = cb.query_cell();
        let length = cb.query_cell();

        // pop 2 inputs from the stack
        cb.stack_pop(length.current());
        cb.stack_pop(target.current());

        // lookup copy table
        cb.copy_lookup(
            CopyTableTag::Output,
            target.current(),
            offset.current(), // cumulative output offset
            length.current(),
        );

        Self {
            target,
            offset,
            length,
            pd: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        row_offset: usize,
        step: &ExecStep,
    ) -> Result<(), GadgetError> {
        let length = step.curr_nth_stack_value(0)?;
        let target = step.curr_nth_stack_value(1)?;
        self.length.assign(region, row_offset, length.as_u64());
        self.offset
            .assign(region, row_offset, step.output_len as u64 - length.as_u64());
        self.target.assign(region, row_offset, target.as_u64());
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok_with_input;
    use fluentbase_runtime::SysFuncIdx;
    use fluentbase_rwasm::instruction_set;

    #[test]
    fn test_simple_write() {
        let data = vec![0x01, 0x02, 0x03, 0x04, 0x05];
        let bytecode = instruction_set! {
            .add_memory(0, data.as_slice())
            I32Const(0) // target
            I32Const(3) // length
            Call(SysFuncIdx::IMPORT_SYS_WRITE)
        };
        test_ok_with_input(bytecode, vec![]);
    }

    #[test]
    fn test_write_with_offset() {
        let data = vec![0x01, 0x02, 0x03, 0x04, 0x05];
        let bytecode = instruction_set! {
            .add_memory(0, data.as_slice())
            I32Const(1) // target
            I32Const(4) // length
            Call(SysFuncIdx::IMPORT_SYS_WRITE)
        };
        test_ok_with_input(bytecode, vec![]);
    }

    #[test]
    fn test_write_expansion() {
        let data = vec![0x01, 0x02, 0x03, 0x04, 0x05];
        let bytecode = instruction_set! {
            .add_memory(0, data.as_slice())
            // copy first 2 bytes
            I32Const(0) // target
            I32Const(2) // length
            Call(SysFuncIdx::IMPORT_SYS_WRITE)
            // copy rest 3 bytes
            I32Const(2) // target
            I32Const(3) // length
            Call(SysFuncIdx::IMPORT_SYS_WRITE)
        };
        test_ok_with_input(bytecode, vec![]);
    }

    #[test]
    fn test_dirty_overwrite() {
        let data = vec![0x01, 0x02, 0x03, 0x04, 0x05];
        let bytecode = instruction_set! {
            .add_memory(0, data.as_slice())
            I32Const(0) // target
            I32Const(5) // length
            Call(SysFuncIdx::IMPORT_SYS_WRITE)
            I32Const(0) // target
            I32Const(2) // length
            Call(SysFuncIdx::IMPORT_SYS_WRITE)
        };
        test_ok_with_input(bytecode, vec![]);
    }
}
