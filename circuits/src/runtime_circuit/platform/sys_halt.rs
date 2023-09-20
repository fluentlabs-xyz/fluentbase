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
pub struct SysHaltGadget<F: Field> {
    exit_code: AdviceColumn,
    pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for SysHaltGadget<F> {
    const NAME: &'static str = "WASM_CALL_HOST(_sys_halt)";
    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_CALL_HOST(SysFuncIdx::SYS_HALT);

    fn configure(cb: &mut OpConstraintBuilder<F>) -> Self {
        let exit_code = cb.query_cell();
        cb.stack_pop(exit_code.current());
        cb.exit_code_lookup(exit_code.current());
        Self {
            exit_code,
            pd: Default::default(),
        }
    }

    fn configure_state_transition(cb: &mut OpConstraintBuilder<F>) {
        cb.next_pc_delta(0.expr());
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        trace: &ExecStep,
    ) -> Result<(), GadgetError> {
        let exit_code = trace.curr_nth_stack_value(0)?;
        self.exit_code.assign(region, offset, exit_code.as_u64());
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
            I32Const(7)
            Call(SysFuncIdx::SYS_HALT)
        });
    }
}
