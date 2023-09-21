use crate::{
    constraint_builder::{AdviceColumn, ToExpr},
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
pub struct WasiArgsGet<F: Field> {
    argv: AdviceColumn,
    argv_buffer: AdviceColumn,
    argv_buffer_limbs: [AdviceColumn; 4],
    length: AdviceColumn,
    pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for WasiArgsGet<F> {
    const NAME: &'static str = "WASM_CALL_HOST(wasi_snapshot_preview1::args_get)";
    const EXECUTION_STATE: ExecutionState =
        ExecutionState::WASM_CALL_HOST(SysFuncIdx::WASI_ARGS_GET);

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let argv = cb.query_cell();
        let argv_buffer = cb.query_cell();
        let argv_buffer_limbs = cb.query_cells();
        let length = cb.query_cell();
        // pop argv & argv_buffer offsets
        cb.stack_pop(argv_buffer.current());
        cb.stack_pop(argv.current());
        // lookup argv_buffer copy
        cb.copy_lookup(
            CopyTableTag::ReadInput,
            0.expr(),
            argv_buffer.current(),
            length.current(),
        );
        // TODO: "add limb checks"
        // lookup argv copy
        (0..4).for_each(|i| {
            cb.mem_write(argv.current() + i.expr(), argv_buffer_limbs[i].current());
        });
        Self {
            argv,
            argv_buffer,
            argv_buffer_limbs,
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
            Call(SysFuncIdx::WASI_ARGS_GET)
            Drop
        });
    }
}
