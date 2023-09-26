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
pub struct WasiEnvironGetGadget<F: Field> {
    environ: AdviceColumn,
    environ_buffer: AdviceColumn,
    pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for WasiEnvironGetGadget<F> {
    const NAME: &'static str = "WASM_CALL_HOST(wasi_snapshot_preview1::environ_get)";
    const EXECUTION_STATE: ExecutionState =
        ExecutionState::WASM_CALL_HOST(SysFuncIdx::WASI_ENVIRON_GET);

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let environ = cb.query_cell();
        let environ_buffer = cb.query_cell();
        cb.stack_pop(environ_buffer.current());
        cb.stack_pop(environ.current());
        // always push error
        cb.stack_push(wasi::ERRNO_CANCELED.raw().expr());
        Self {
            environ,
            environ_buffer,
            pd: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &ExecStep,
    ) -> Result<(), GadgetError> {
        let environ_buffer = trace.curr_nth_stack_value(0)?;
        self.environ_buffer
            .assign(region, offset, environ_buffer.as_u64());
        let environ = trace.curr_nth_stack_value(1)?;
        self.environ.assign(region, offset, environ.as_u64());
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
            Call(SysFuncIdx::WASI_ENVIRON_GET)
            Drop
        });
    }
}
