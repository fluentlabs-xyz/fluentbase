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
pub struct WasiEnvironSizesGet<F: Field> {
    rp0_ptr: AdviceColumn,
    rp1_ptr: AdviceColumn,
    pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for WasiEnvironSizesGet<F> {
    const NAME: &'static str = "WASM_CALL_HOST(wasi_snapshot_preview1::environ_sizes_get)";
    const EXECUTION_STATE: ExecutionState =
        ExecutionState::WASM_CALL_HOST(SysFuncIdx::WASI_ENVIRON_SIZES_GET);

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let rp0_ptr = cb.query_cell();
        let rp1_ptr = cb.query_cell();
        cb.stack_pop(rp1_ptr.current());
        cb.stack_pop(rp0_ptr.current());
        // always push error
        cb.stack_push(wasi::ERRNO_CANCELED.raw().expr());
        Self {
            rp0_ptr,
            rp1_ptr,
            pd: Default::default(),
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &ExecStep,
    ) -> Result<(), GadgetError> {
        let rp1_ptr = trace.curr_nth_stack_value(0)?;
        self.rp1_ptr.assign(region, offset, rp1_ptr.as_u64());
        let rp0_ptr = trace.curr_nth_stack_value(1)?;
        self.rp0_ptr.assign(region, offset, rp0_ptr.as_u64());
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
            Call(SysFuncIdx::WASI_ENVIRON_SIZES_GET)
            Drop
        });
    }
}
