use crate::{
    constraint_builder::{AdviceColumn, ToExpr},
    exec_step::{ExecStep, GadgetError},
    runtime_circuit::{
        constraint_builder::OpConstraintBuilder,
        execution_state::ExecutionState,
        opcodes::ExecutionGadget,
    },
    util::Field,
};
use fluentbase_runtime::SysFuncIdx;
use halo2_proofs::circuit::Region;
use std::marker::PhantomData;

#[derive(Clone)]
pub struct WasiArgsSizesGetGadget<F: Field> {
    argv: AdviceColumn,
    argv_buffer: AdviceColumn,
    length: AdviceColumn,
    pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for WasiArgsSizesGetGadget<F> {
    const NAME: &'static str = "WASM_CALL_HOST(wasi_snapshot_preview1::args_sizes_get)";
    const EXECUTION_STATE: ExecutionState =
        ExecutionState::WASM_CALL_HOST(SysFuncIdx::WASI_ARGS_SIZES_GET);

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let argv = cb.query_cell();
        let argv_buffer = cb.query_cell();
        let length = cb.query_cell();
        // pop argv & argv_buffer offsets
        cb.stack_pop(argv_buffer.current());
        cb.stack_pop(argv.current());
        // copy argv count (its always 1)
        1u32.to_be_bytes().iter().enumerate().for_each(|(i, byte)| {
            cb.mem_write(argv.current() + i.expr(), byte.expr());
        });
        // TODO: "lookup input length equal to argv_buffer value"
        cb.stack_push(0.expr());
        Self {
            argv,
            argv_buffer,
            length,
            pd: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &ExecStep,
    ) -> Result<(), GadgetError> {
        let argv_buffer = trace.curr_nth_stack_value(0)?;
        self.argv_buffer
            .assign(region, offset, argv_buffer.as_u64());
        let argv = trace.curr_nth_stack_value(1)?;
        self.argv.assign(region, offset, argv.as_u64());
        let copied_length = trace.next().unwrap().memory_changes[1].len;
        self.length.assign(region, offset, copied_length as u64);
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_runtime::SysFuncIdx;
    use fluentbase_rwasm::instruction_set;

    #[test]
    fn test_exit() {
        test_ok(instruction_set! {
            I32Const(0)
            I32Const(0)
            Call(SysFuncIdx::WASI_ARGS_SIZES_GET)
            Drop
        });
    }
}
