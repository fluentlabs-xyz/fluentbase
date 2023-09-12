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

const MAX_INPUT_DEGREE: usize = 10;

#[derive(Clone)]
pub struct SysReadGadget<F: Field> {
    target: AdviceColumn,
    offset: AdviceColumn,
    length: AdviceColumn,
    pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for SysReadGadget<F> {
    const NAME: &'static str = "WASM_CALL_HOST(_sys_read)";
    const EXECUTION_STATE: ExecutionState =
        ExecutionState::WASM_CALL_HOST(SysFuncIdx::IMPORT_SYS_READ);

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let target = cb.query_cell();
        let offset = cb.query_cell();
        let length = cb.query_cell();

        // pop 3 inputs from the stack
        cb.stack_pop(length.current());
        cb.stack_pop(offset.current());
        cb.stack_pop(target.current());

        // lookup copy table
        cb.copy_lookup(
            CopyTableTag::ReadInput,
            offset.current(),
            target.current(),
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
        trace: &ExecStep,
    ) -> Result<(), GadgetError> {
        let length = trace.curr_nth_stack_value(0)?;
        let offset = trace.curr_nth_stack_value(1)?;
        let target = trace.curr_nth_stack_value(2)?;
        self.length.assign(region, row_offset, length.as_u64());
        self.offset.assign(region, row_offset, offset.as_u64());
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
    fn test_read_part() {
        let input = vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
        let bytecode = instruction_set! {
            I32Const(0) // target
            I32Const(0) // offset
            I32Const(3) // length
            Call(SysFuncIdx::IMPORT_SYS_READ)
        };
        test_ok_with_input(bytecode, input);
    }

    #[test]
    fn test_read_in_one_row() {
        let input = vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
        let bytecode = instruction_set! {
            I32Const(0) // target
            I32Const(0) // offset
            I32Const(3) // length
            Call(SysFuncIdx::IMPORT_SYS_READ)
            I32Const(3) // target
            I32Const(3) // offset
            I32Const(3) // length
            Call(SysFuncIdx::IMPORT_SYS_READ)
            I32Const(6) // target
            I32Const(6) // offset
            I32Const(3) // length
            Call(SysFuncIdx::IMPORT_SYS_READ)
            I32Const(9) // target
            I32Const(9) // offset
            I32Const(1) // length
            Call(SysFuncIdx::IMPORT_SYS_READ)
        };
        test_ok_with_input(bytecode, input);
    }

    #[test]
    fn test_read_fully() {
        let input = vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
        let bytecode = instruction_set! {
            I32Const(0) // target
            I32Const(0) // offset
            I32Const(10) // length
            Call(SysFuncIdx::IMPORT_SYS_READ)
        };
        test_ok_with_input(bytecode, input);
    }

    #[test]
    fn test_read_one_byte() {
        let input = vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
        let bytecode = instruction_set! {
            I32Const(0) // target
            I32Const(0) // offset
            I32Const(1) // length
            Call(SysFuncIdx::IMPORT_SYS_READ)
        };
        test_ok_with_input(bytecode, input);
    }

    #[test]
    fn test_read_with_offset() {
        let input = vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
        let bytecode = instruction_set! {
            I32Const(0) // target
            I32Const(3) // offset
            I32Const(1) // length
            Call(SysFuncIdx::IMPORT_SYS_READ)
        };
        test_ok_with_input(bytecode, input);
    }
}
