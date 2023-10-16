use crate::{
    constraint_builder::ToExpr,
    exec_step::{ExecStep, GadgetError},
    runtime_circuit::{
        constraint_builder::OpConstraintBuilder,
        execution_state::ExecutionState,
        opcodes::ExecutionGadget,
    },
    util::Field,
};
use halo2_proofs::circuit::Region;
use std::marker::PhantomData;

#[derive(Clone)]
pub struct OpReturnGadget<F: Field> {
    pd: PhantomData<F>,
}

impl<F: Field> ExecutionGadget<F> for OpReturnGadget<F> {
    const NAME: &'static str = "WASM_RETURN";
    const EXECUTION_STATE: ExecutionState = ExecutionState::WASM_RETURN;

    fn configure(_cb: &mut OpConstraintBuilder<F>) -> Self {
        // if we're not inside root call then decrease call depth by one
        // cb.condition(cb.call_id(), |cb| {
        //     cb.context_lookup(
        //         RwTableContextTag::CallDepth,
        //         1.expr(),
        //         cb.call_id() - 1,
        //         cb.call_id(),
        //     );
        // });
        Self {
            pd: Default::default(),
        }
    }

    fn configure_state_transition(cb: &mut OpConstraintBuilder<F>) {
        cb.next_pc_delta(0.expr());
    }

    fn assign_exec_step(
        &self,
        _region: &mut Region<'_, F>,
        _offset: usize,
        _trace: &ExecStep,
    ) -> Result<(), GadgetError> {
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::runtime_circuit::testing::test_ok;
    use fluentbase_runtime::SysFuncIdx;
    use fluentbase_rwasm::{engine::DropKeep, instruction_set};

    #[test]
    fn test_root_return() {
        test_ok(instruction_set! {
            Return(DropKeep::default())
        });
    }

    #[test]
    fn test_non_root_return() {
        let bytecode: Vec<u8> = instruction_set! {
            Return(DropKeep::default())
        }
        .into();
        test_ok(instruction_set! {
            .add_memory(0, bytecode.as_slice())
            I32Const(0) // code offset
            I32Const(bytecode.len() as u32) // code len
            I32Const(0) // input offset
            I32Const(0) // input len
            I32Const(0) // output offset
            I32Const(0) // output len
            Call(SysFuncIdx::RWASM_TRANSACT)
            Drop
        });
    }
}
